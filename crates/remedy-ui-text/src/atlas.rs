//! The glyph atlas: rasterized glyphs packed into pages the renderer uploads as textures.
//!
//! Pages are filled shelf by shelf (rows of similar height). There is no fixed number of
//! pages: a full atlas adds one. A glyph larger than a page gets a page of its own size.
//! Pages remember the region changed since the renderer last took it, so uploads cover only
//! new glyphs. Under memory pressure, pages none of whose glyphs were used recently are
//! dropped whole, which keeps packing simple and eviction cheap.

use std::collections::HashMap;

use crate::engine::RunFont;
use crate::raster::{Antialias, GlyphImage, GlyphRasterizer, Hinting, size_64, subpixel_bin};

/// Side of a normal atlas page in pixels. A tuning constant: large enough for hundreds of
/// glyphs at interface sizes, small enough to upload quickly and to drop whole.
pub const PAGE_SIZE: u32 = 1024;

/// What identifies one rasterized glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// The font bytes' id ([`RunFont::id`]).
    pub font: u64,
    /// Face index within the font file.
    pub face: u32,
    /// Glyph id.
    pub glyph: u32,
    /// Size in 1/64 pixel.
    pub size_64: u32,
    /// Horizontal subpixel bin.
    pub bin: u8,
    /// Hinting mode.
    pub hinting: Hinting,
    /// Coverage format.
    pub aa: Antialias,
}

/// Where a glyph is in the atlas, and how to place it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasEntry {
    /// Page index.
    pub page: u32,
    /// Left edge in the page, in pixels.
    pub x: u32,
    /// Top edge in the page.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Offset from the glyph origin to the image's left edge.
    pub left: i32,
    /// Offset from the baseline up to the image's top edge.
    pub top: i32,
}

/// A rectangle of a page, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRect {
    /// Left.
    pub x: u32,
    /// Top.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

impl PageRect {
    fn union(self, o: PageRect) -> PageRect {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.width).max(o.x + o.width);
        let y1 = (self.y + self.height).max(o.y + o.height);
        PageRect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
    }
}

struct Shelf {
    y: u32,
    height: u32,
    next_x: u32,
}

/// One atlas page.
pub struct Page {
    width: u32,
    height: u32,
    data: Vec<u8>,
    shelves: Vec<Shelf>,
    next_y: u32,
    dirty: Option<PageRect>,
    last_used: u64,
    live: bool,
}

impl Page {
    fn new(width: u32, height: u32, bpp: usize) -> Self {
        Page {
            width,
            height,
            data: vec![0; width as usize * height as usize * bpp],
            shelves: Vec::new(),
            next_y: 0,
            dirty: None,
            last_used: 0,
            live: true,
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels, row by row.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Finds room for a `w` by `h` image, with one pixel of gap so filtering never bleeds.
    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (gw, gh) = (w + 1, h + 1);
        // Prefer a shelf that fits without wasting more than half its height.
        let width = self.width;
        if let Some(s) = self
            .shelves
            .iter_mut()
            .filter(|s| s.height >= gh && s.height <= gh + gh / 2 + 2 && s.next_x + gw <= width)
            .min_by_key(|s| s.height)
        {
            let at = (s.next_x, s.y);
            s.next_x += gw;
            return Some(at);
        }
        if self.next_y + gh <= self.height && gw <= self.width {
            let y = self.next_y;
            self.shelves.push(Shelf { y, height: gh, next_x: gw });
            self.next_y += gh;
            return Some((0, y));
        }
        None
    }

    fn blit(&mut self, x: u32, y: u32, img: &GlyphImage, bpp: usize) {
        let row_bytes = img.width as usize * bpp;
        for r in 0..img.height as usize {
            let src = img.data.get(r * row_bytes..(r + 1) * row_bytes).unwrap_or(&[]);
            let start = ((y as usize + r) * self.width as usize + x as usize) * bpp;
            if let Some(dst) = self.data.get_mut(start..start + src.len()) {
                dst.copy_from_slice(src);
            }
        }
        let rect = PageRect { x, y, width: img.width, height: img.height };
        self.dirty = Some(self.dirty.map_or(rect, |d| d.union(rect)));
    }
}

/// Atlas statistics, for the performance overlay and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtlasStats {
    /// Live pages.
    pub pages: usize,
    /// Glyphs stored.
    pub glyphs: usize,
    /// Bytes of pixel data held.
    pub bytes: usize,
}

/// Rasterized glyphs for one coverage format, packed into pages.
pub struct GlyphAtlas {
    aa: Antialias,
    pages: Vec<Page>,
    entries: HashMap<GlyphKey, AtlasEntry>,
    /// Glyphs known to have no image (spaces), so they are not rasterized again.
    empty: HashMap<GlyphKey, ()>,
    raster: GlyphRasterizer,
    frame: u64,
}

impl std::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GlyphAtlas({:?})", self.stats())
    }
}

impl GlyphAtlas {
    /// An empty atlas for one coverage format.
    pub fn new(aa: Antialias) -> Self {
        GlyphAtlas {
            aa,
            pages: Vec::new(),
            entries: HashMap::new(),
            empty: HashMap::new(),
            raster: GlyphRasterizer::new(),
            frame: 0,
        }
    }

    /// The coverage format of this atlas.
    pub fn antialias(&self) -> Antialias {
        self.aa
    }

    /// Starts a new frame; glyphs looked up from now on count as used in it.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// The glyph's place in the atlas, rasterizing and packing it on first use. `x` is the
    /// glyph's horizontal position, used to pick its subpixel bin. Returns `None` for glyphs
    /// with no image (spaces) and fonts that cannot be read.
    pub fn glyph(
        &mut self,
        font: &RunFont,
        glyph: u32,
        size: f32,
        x: f32,
        hinting: Hinting,
    ) -> Option<AtlasEntry> {
        let bin = subpixel_bin(x, hinting);
        let key = GlyphKey {
            font: font.id(),
            face: font.index(),
            glyph,
            size_64: size_64(size),
            bin,
            hinting,
            aa: self.aa,
        };
        if let Some(e) = self.entries.get(&key).copied() {
            if let Some(p) = self.pages.get_mut(e.page as usize) {
                p.last_used = self.frame;
            }
            return Some(e);
        }
        if self.empty.contains_key(&key) {
            return None;
        }
        let Some(img) = self.raster.rasterize(font, glyph, size, bin, hinting, self.aa) else {
            self.empty.insert(key, ());
            return None;
        };
        let e = self.insert(&img)?;
        self.entries.insert(key, e);
        Some(e)
    }

    fn insert(&mut self, img: &GlyphImage) -> Option<AtlasEntry> {
        let bpp = self.aa.bytes_per_pixel();
        let frame = self.frame;
        for (i, p) in self.pages.iter_mut().enumerate() {
            if !p.live {
                continue;
            }
            if let Some((x, y)) = p.alloc(img.width, img.height) {
                p.blit(x, y, img, bpp);
                p.last_used = frame;
                return Some(entry(i, x, y, img));
            }
        }
        // A new page: normal size, or the glyph's own size if it is larger. Reuse a dropped
        // page's slot so page indices stay small.
        let (w, h) = (PAGE_SIZE.max(img.width + 1), PAGE_SIZE.max(img.height + 1));
        let mut page = Page::new(w, h, bpp);
        let (x, y) = page.alloc(img.width, img.height)?;
        page.blit(x, y, img, bpp);
        page.last_used = frame;
        let index = match self.pages.iter().position(|p| !p.live) {
            Some(i) => {
                if let Some(slot) = self.pages.get_mut(i) {
                    *slot = page;
                }
                i
            }
            None => {
                self.pages.push(page);
                self.pages.len() - 1
            }
        };
        Some(entry(index, x, y, img))
    }

    /// The page at `index`, if it is live.
    pub fn page(&self, index: u32) -> Option<&Page> {
        self.pages.get(index as usize).filter(|p| p.live)
    }

    /// The region of each page changed since the last call, for the renderer to upload.
    pub fn take_dirty(&mut self) -> Vec<(u32, PageRect)> {
        self.pages
            .iter_mut()
            .enumerate()
            .filter(|(_, p)| p.live)
            .filter_map(|(i, p)| p.dirty.take().map(|r| (i as u32, r)))
            .collect()
    }

    /// Drops every page none of whose glyphs were used in the last `frames` frames, and the
    /// hinting instances. Called under memory pressure. Returns how many pages were dropped.
    pub fn evict_unused(&mut self, frames: u64) -> usize {
        let cutoff = self.frame.saturating_sub(frames);
        let mut dropped = Vec::new();
        for (i, p) in self.pages.iter_mut().enumerate() {
            if p.live && p.last_used < cutoff {
                p.live = false;
                p.data = Vec::new();
                p.shelves.clear();
                p.dirty = None;
                dropped.push(i as u32);
            }
        }
        if !dropped.is_empty() {
            self.entries.retain(|_, e| !dropped.contains(&e.page));
            self.raster.clear();
        }
        dropped.len()
    }

    /// Current size of the atlas.
    pub fn stats(&self) -> AtlasStats {
        let live: Vec<&Page> = self.pages.iter().filter(|p| p.live).collect();
        AtlasStats {
            pages: live.len(),
            glyphs: self.entries.len(),
            bytes: live.iter().map(|p| p.data.len()).sum(),
        }
    }
}

fn entry(page: usize, x: u32, y: u32, img: &GlyphImage) -> AtlasEntry {
    AtlasEntry { page: page as u32, x, y, width: img.width, height: img.height, left: img.left, top: img.top }
}
