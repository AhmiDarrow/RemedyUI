//! GPU-resident images: glyphs and nearest-sampled images packed into RGBA atlas pages, and
//! filtered or large images in textures of their own.

use std::collections::HashMap;
use std::sync::Arc;

use remedy_ui_render_cpu::ImageKey;
use vello_gpu::{ImageSource, Pixmap, RectU16, TextureId, TextureRegion};

use crate::renderer::FORMAT;

/// Image texture ids live in the upper half of the id space, tile ids in the lower.
const IMAGE_ID_BASE: u64 = 1 << 63;
/// Transparent pixels between atlas entries, so neighbours never bleed into each other.
const GUTTER: u32 = 1;

struct Shelf {
    y: u32,
    height: u32,
    next_x: u32,
}

struct Page {
    id: TextureId,
    texture: wgpu::Texture,
    shelves: Vec<Shelf>,
    next_y: u32,
    /// Uploads living on the page (images and glyphs); when the last goes, the page starts
    /// over empty.
    used: usize,
}

/// Where an app image's upload lives.
enum Home {
    Page(TextureId),
    Own(TextureId),
}

/// Uploaded images, keyed like the painter's cache.
pub(crate) struct GpuImages {
    page_size: u32,
    pages: Vec<Page>,
    entries: HashMap<ImageKey, ImageSource>,
    own: HashMap<ImageKey, wgpu::Texture>,
    homes: HashMap<u64, Home>,
    views: HashMap<TextureId, wgpu::TextureView>,
    pub(crate) bindings: vello_gpu::TextureBindings,
    next_id: u64,
}

fn texture(device: &wgpu::Device, width: u32, height: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, x: u32, y: u32, pixmap: &Pixmap) {
    let (w, h) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        pixmap.data_as_u8_slice(),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

impl GpuImages {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        // Pages as large as the device allows up to 2048: big enough for thousands of
        // interface glyphs, small enough to allocate quickly.
        let page_size = device.limits().max_texture_dimension_2d.min(2048);
        GpuImages {
            page_size,
            pages: Vec::new(),
            entries: HashMap::new(),
            own: HashMap::new(),
            homes: HashMap::new(),
            views: HashMap::new(),
            bindings: vello_gpu::TextureBindings::new(),
            next_id: IMAGE_ID_BASE,
        }
    }

    fn bind(&mut self, id: TextureId, view: wgpu::TextureView) {
        self.bindings.insert(id, view.clone());
        self.views.insert(id, view);
    }

    /// Frees what was uploaded for app images that are gone.
    pub(crate) fn evict(&mut self, ids: &[u64]) {
        let mut unbound = false;
        for id in ids {
            let key = ImageKey::Image(*id);
            self.entries.remove(&key);
            match self.homes.remove(id) {
                Some(Home::Own(t)) => {
                    self.own.remove(&key);
                    self.views.remove(&t);
                    unbound = true;
                }
                Some(Home::Page(p)) => {
                    if let Some(page) = self.pages.iter_mut().find(|pg| pg.id == p) {
                        page.used = page.used.saturating_sub(1);
                        if page.used == 0 {
                            page.shelves.clear();
                            page.next_y = 0;
                        }
                    }
                }
                None => {}
            }
        }
        if unbound {
            // Bindings cannot be removed one by one: rebuild them from what is left.
            let mut b = vello_gpu::TextureBindings::new();
            for (id, view) in &self.views {
                b.insert(*id, view.clone());
            }
            self.bindings = b;
        }
    }

    /// Binds a texture made elsewhere (a video stream's picture) under `id`.
    pub(crate) fn bind_external(&mut self, id: TextureId, view: wgpu::TextureView) {
        self.bind(id, view);
    }

    /// Unbinds textures made elsewhere.
    pub(crate) fn unbind(&mut self, ids: &[TextureId]) {
        if ids.is_empty() {
            return;
        }
        for id in ids {
            self.views.remove(id);
        }
        let mut b = vello_gpu::TextureBindings::new();
        for (id, view) in &self.views {
            b.insert(*id, view.clone());
        }
        self.bindings = b;
    }

    pub(crate) fn fresh_id(&mut self) -> TextureId {
        let id = TextureId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Drops every upload (memory pressure, or the painter dropped its caches).
    pub(crate) fn clear(&mut self) {
        self.pages.clear();
        self.entries.clear();
        self.own.clear();
        self.homes.clear();
        self.views.clear();
        self.bindings = vello_gpu::TextureBindings::new();
    }

    /// Bytes of GPU memory held.
    pub(crate) fn bytes(&self) -> u64 {
        let page = u64::from(self.page_size) * u64::from(self.page_size) * 4;
        let own: u64 = self.own.values().map(|t| u64::from(t.width()) * u64::from(t.height()) * 4).sum();
        page * self.pages.len() as u64 + own
    }

    pub(crate) fn source(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: ImageKey,
        pixmap: &Arc<Pixmap>,
        filtered: bool,
    ) -> Option<ImageSource> {
        if let Some(s) = self.entries.get(&key) {
            return Some(s.clone());
        }
        let (w, h) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
        if w == 0 || h == 0 {
            return None;
        }
        // Filtered images sample past their edges, so they get their own texture (clamped
        // edges); so do images too large to share a page.
        let source = if filtered || w + 2 * GUTTER > self.page_size / 2 || h + 2 * GUTTER > self.page_size / 2
        {
            if w > device.limits().max_texture_dimension_2d || h > device.limits().max_texture_dimension_2d {
                return None;
            }
            let t = texture(device, w, h, "RemedyUI image");
            upload(queue, &t, 0, 0, pixmap);
            let id = self.fresh_id();
            self.bind(id, t.create_view(&wgpu::TextureViewDescriptor::default()));
            self.own.insert(key, t);
            if let ImageKey::Image(i) = key {
                self.homes.insert(i, Home::Own(id));
            }
            ImageSource::external_texture(id, TextureRegion::Full, true)
        } else {
            let (page, x, y) = self.allocate(device, w + 2 * GUTTER, h + 2 * GUTTER)?;
            let (x, y) = (x + GUTTER, y + GUTTER);
            upload(queue, &page.texture, x, y, pixmap);
            let page_id = page.id;
            if let Some(p) = self.pages.iter_mut().find(|p| p.id == page_id) {
                p.used += 1;
            }
            if let ImageKey::Image(i) = key {
                self.homes.insert(i, Home::Page(page_id));
            }
            let rect = RectU16 {
                x0: u16::try_from(x).ok()?,
                y0: u16::try_from(y).ok()?,
                x1: u16::try_from(x + w).ok()?,
                y1: u16::try_from(y + h).ok()?,
            };
            ImageSource::external_texture(page_id, TextureRegion::Rect(rect), true)
        };
        self.entries.insert(key, source.clone());
        Some(source)
    }

    fn allocate(&mut self, device: &wgpu::Device, w: u32, h: u32) -> Option<(&Page, u32, u32)> {
        let size = self.page_size;
        let fits = |p: &mut Page| -> Option<(u32, u32)> {
            // The best shelf: tall enough, not much taller, with room.
            if let Some(s) = p
                .shelves
                .iter_mut()
                .filter(|s| s.height >= h && s.height <= h + h / 2 + 2 && s.next_x + w <= size)
                .min_by_key(|s| s.height)
            {
                let x = s.next_x;
                s.next_x += w;
                return Some((x, s.y));
            }
            if p.next_y + h <= size {
                let y = p.next_y;
                p.shelves.push(Shelf { y, height: h, next_x: w });
                p.next_y += h;
                return Some((0, y));
            }
            None
        };
        let mut found = None;
        for (i, p) in self.pages.iter_mut().enumerate() {
            if let Some((x, y)) = fits(p) {
                found = Some((i, x, y));
                break;
            }
        }
        if found.is_none() {
            let id = self.fresh_id();
            let t = texture(device, size, size, "RemedyUI atlas page");
            self.bind(id, t.create_view(&wgpu::TextureViewDescriptor::default()));
            let mut p = Page { id, texture: t, shelves: Vec::new(), next_y: 0, used: 0 };
            let at = fits(&mut p)?;
            self.pages.push(p);
            found = Some((self.pages.len() - 1, at.0, at.1));
        }
        let (i, x, y) = found?;
        self.pages.get(i).map(|p| (p, x, y))
    }
}
