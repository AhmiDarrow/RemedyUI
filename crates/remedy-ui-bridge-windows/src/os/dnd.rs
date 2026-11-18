//! OLE drag and drop: a drop target on every window (files, text and links from any app,
//! with positions), and drags out through the shell, which draws the standard drag image.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use remedy_ui_core::Point;
use remedy_ui_host::{DragData, DragEvent, DropEffect, Unsupported};
use windows::Win32::Foundation::{HGLOBAL, HWND, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
    IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    DragQueryFileW, HDROP, ILCreateFromPathW, ILFree, SHCreateDataObject, SHDoDragDrop,
};
use windows::core::{PCWSTR, Ref, implement};

/// Drag events waiting for the event loop, by HWND.
pub(crate) type Queue = Rc<RefCell<Vec<(isize, DragEvent)>>>;

fn effect_bits(e: DropEffect) -> DROPEFFECT {
    match e {
        DropEffect::None => DROPEFFECT_NONE,
        DropEffect::Copy => DROPEFFECT_COPY,
        DropEffect::Move => DROPEFFECT_MOVE,
        DropEffect::Link => DROPEFFECT_LINK,
    }
}

fn effect_of(bits: DROPEFFECT) -> DropEffect {
    if bits.0 & DROPEFFECT_MOVE.0 != 0 {
        DropEffect::Move
    } else if bits.0 & DROPEFFECT_COPY.0 != 0 {
        DropEffect::Copy
    } else if bits.0 & DROPEFFECT_LINK.0 != 0 {
        DropEffect::Link
    } else {
        DropEffect::None
    }
}

fn format(cf: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn url_format() -> u16 {
    let name: Vec<u16> = "UniformResourceLocatorW\0".encode_utf16().collect();
    // SAFETY: a NUL-terminated wide string; the call only reads it.
    let f = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
    u16::try_from(f).unwrap_or(0)
}

/// Text in an HGLOBAL of UTF-16, up to its first NUL.
fn global_text(h: HGLOBAL) -> Option<String> {
    // SAFETY: the handle came from GetData with TYMED_HGLOBAL and is valid until the medium
    // is released; GlobalLock/Unlock bracket every read, and the size bounds it.
    unsafe {
        let p = GlobalLock(h) as *const u16;
        if p.is_null() {
            return None;
        }
        let units = GlobalSize(h) / 2;
        let slice = std::slice::from_raw_parts(p, units);
        let end = slice.iter().position(|c| *c == 0).unwrap_or(slice.len());
        let s = String::from_utf16_lossy(slice.get(..end).unwrap_or_default());
        let _ = GlobalUnlock(h);
        Some(s)
    }
}

/// What a data object from another app carries, in the forms RemedyUI understands.
fn read(data: &IDataObject) -> DragData {
    let mut out = DragData::default();
    // SAFETY: each GetData result is a medium we own and release once read.
    unsafe {
        if let Ok(mut m) = data.GetData(&format(CF_HDROP.0)) {
            let h = HDROP(m.u.hGlobal.0);
            let n = DragQueryFileW(h, u32::MAX, None);
            for i in 0..n {
                let len = DragQueryFileW(h, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let got = DragQueryFileW(h, i, Some(&mut buf)) as usize;
                out.files.push(PathBuf::from(String::from_utf16_lossy(buf.get(..got).unwrap_or_default())));
            }
            ReleaseStgMedium(&mut m);
        }
        if let Ok(mut m) = data.GetData(&format(CF_UNICODETEXT.0)) {
            out.text = global_text(m.u.hGlobal);
            ReleaseStgMedium(&mut m);
        }
        let url = url_format();
        if url != 0
            && let Ok(mut m) = data.GetData(&format(url))
        {
            if let Some(u) = global_text(m.u.hGlobal).filter(|u| !u.is_empty()) {
                out.uris.push(u);
            }
            ReleaseStgMedium(&mut m);
        }
    }
    out
}

#[implement(IDropTarget)]
struct Target {
    hwnd: isize,
    /// Device pixels per logical pixel, kept current by the bridge.
    scale: Rc<std::cell::Cell<f32>>,
    queue: Queue,
    /// What the app said a drop would do, after the last event it saw.
    effect: Rc<std::cell::Cell<DropEffect>>,
}

impl Target {
    fn local(&self, pt: &POINTL) -> Point {
        let mut p = POINT { x: pt.x, y: pt.y };
        // SAFETY: the window is alive while it is registered as a drop target.
        let _ = unsafe { ScreenToClient(HWND(self.hwnd as *mut _), &mut p) };
        let s = self.scale.get().max(0.25);
        Point::new(p.x as f32 / s, p.y as f32 / s)
    }

    fn answer(&self, pdweffect: *mut DROPEFFECT) {
        if pdweffect.is_null() {
            return;
        }
        // SAFETY: OLE passes a valid in/out pointer: the allowed effects in, ours out.
        unsafe {
            let allowed = *pdweffect;
            let ours = effect_bits(self.effect.get());
            *pdweffect = DROPEFFECT(allowed.0 & ours.0);
        }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let d = data.ok().map(read).unwrap_or_default();
        self.queue
            .borrow_mut()
            .push((self.hwnd, DragEvent::Enter { position: Some(self.local(pt)), data: d }));
        // Until the app has seen it, nothing is promised.
        self.effect.set(DropEffect::None);
        self.answer(pdweffect);
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.queue.borrow_mut().push((self.hwnd, DragEvent::Over { position: self.local(pt) }));
        self.answer(pdweffect);
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.queue.borrow_mut().push((self.hwnd, DragEvent::Leave));
        self.effect.set(DropEffect::None);
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let d = data.ok().map(read).unwrap_or_default();
        self.queue
            .borrow_mut()
            .push((self.hwnd, DragEvent::Drop { position: Some(self.local(pt)), data: d }));
        self.answer(pdweffect);
        self.effect.set(DropEffect::None);
        Ok(())
    }
}

/// One window's drop target.
pub(crate) struct Registered {
    hwnd: isize,
    pub(crate) scale: Rc<std::cell::Cell<f32>>,
    pub(crate) effect: Rc<std::cell::Cell<DropEffect>>,
}

impl Drop for Registered {
    fn drop(&mut self) {
        // SAFETY: revokes what `register` registered for this window.
        let _ = unsafe { RevokeDragDrop(HWND(self.hwnd as *mut _)) };
    }
}

/// Makes `hwnd` a drop target whose events go to `queue`.
pub(crate) fn register(hwnd: isize, scale: f32, queue: Queue) -> Result<Registered, Unsupported> {
    // SAFETY: OLE needs a single-threaded apartment on the UI thread; repeated calls only
    // add a reference.
    unsafe { OleInitialize(None) }.map_err(|e| Unsupported::new("drag and drop", e.to_string()))?;
    let scale = Rc::new(std::cell::Cell::new(scale));
    let effect = Rc::new(std::cell::Cell::new(DropEffect::None));
    let target: IDropTarget = Target { hwnd, scale: scale.clone(), queue, effect: effect.clone() }.into();
    // SAFETY: a live window owned by this thread; OLE keeps its own reference to the target.
    unsafe { RegisterDragDrop(HWND(hwnd as *mut _), &target) }
        .map_err(|e| Unsupported::new("drag and drop", e.to_string()))?;
    Ok(Registered { hwnd, scale, effect })
}

fn hglobal(bytes: &[u8]) -> Option<HGLOBAL> {
    // SAFETY: a fresh movable block of exactly the bytes copied in; ownership passes to the
    // data object with `SetData(.., true)`.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Some(h)
    }
}

fn hglobal_text(s: &str) -> Option<HGLOBAL> {
    let bytes: Vec<u8> = s.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_ne_bytes).collect();
    hglobal(&bytes)
}

/// A `DROPFILES` block (CF_HDROP): the header, then wide paths each ending in NUL, then NUL.
fn hdrop_bytes(files: &[PathBuf]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&20u32.to_ne_bytes()); // pFiles: the list follows the 20-byte header
    b.extend_from_slice(&0i32.to_ne_bytes()); // pt.x
    b.extend_from_slice(&0i32.to_ne_bytes()); // pt.y
    b.extend_from_slice(&0i32.to_ne_bytes()); // fNC
    b.extend_from_slice(&1i32.to_ne_bytes()); // fWide
    for f in files {
        for u in f.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)) {
            b.extend_from_slice(&u.to_ne_bytes());
        }
    }
    b.extend_from_slice(&0u16.to_ne_bytes());
    b
}

/// A data object carrying `data` the way other Windows apps read it: files as shell items
/// (Explorer can copy or move them), text, and the first link.
fn data_object(data: &DragData) -> Result<IDataObject, Unsupported> {
    let fail = |e: windows::core::Error| Unsupported::new("dragging out", e.to_string());
    // Files as shell items, so Explorer and other apps can copy or move them.
    let pidls: Vec<*mut ITEMIDLIST> = data
        .files
        .iter()
        .filter_map(|p| {
            let w: Vec<u16> =
                p.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: a NUL-terminated wide path; the result is freed below.
            let id = unsafe { ILCreateFromPathW(PCWSTR(w.as_ptr())) };
            (!id.is_null()).then_some(id)
        })
        .collect();
    let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| p.cast_const()).collect();
    // SAFETY: absolute item ids (no parent folder); the data object copies them.
    let obj: Result<IDataObject, _> = unsafe {
        SHCreateDataObject(None, (!consts.is_empty()).then_some(consts.as_slice()), None::<&IDataObject>)
    };
    for p in pidls {
        // SAFETY: each came from ILCreateFromPathW above.
        unsafe { ILFree(Some(p)) };
    }
    let obj = obj.map_err(fail)?;
    let put_block = |cf: u16, h: Option<HGLOBAL>| {
        if let Some(h) = h {
            let mut m = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, ..Default::default() };
            m.u.hGlobal = h;
            // SAFETY: the data object takes ownership of the block (`true`).
            let _ = unsafe { obj.SetData(&format(cf), &m, true) };
        }
    };
    // The plain file list most apps read, beside the shell's own item list.
    if !data.files.is_empty() {
        put_block(CF_HDROP.0, hglobal(&hdrop_bytes(&data.files)));
    }
    let put = |cf: u16, text: &str| {
        if let Some(h) = hglobal_text(text) {
            let mut m = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, ..Default::default() };
            m.u.hGlobal = h;
            // SAFETY: the data object takes ownership of the block (`true`).
            let _ = unsafe { obj.SetData(&format(cf), &m, true) };
        }
    };
    if let Some(t) = &data.text {
        put(CF_UNICODETEXT.0, t);
    }
    let url = url_format();
    if let Some(u) = data.uris.first()
        && url != 0
    {
        put(url, u);
        if data.text.is_none() {
            put(CF_UNICODETEXT.0, u);
        }
    }
    Ok(obj)
}

/// Drags `data` out of `hwnd` until the user drops or cancels; what the drop did.
pub(crate) fn drag_out(
    hwnd: isize,
    data: &DragData,
    allowed: &[DropEffect],
) -> Result<DropEffect, Unsupported> {
    let fail = |e: windows::core::Error| Unsupported::new("dragging out", e.to_string());
    // SAFETY: OLE on this (UI) thread; repeated calls only add a reference.
    unsafe { OleInitialize(None) }.map_err(fail)?;
    let obj = data_object(data)?;
    let mut bits = 0u32;
    for e in allowed {
        bits |= effect_bits(*e).0;
    }
    // SAFETY: modal: runs the drag with the shell's drag image and default drop source,
    // pumping this thread's messages, and returns when it ends.
    let done = unsafe {
        SHDoDragDrop(
            Some(HWND(hwnd as *mut _)),
            &obj,
            None::<&windows::Win32::System::Ole::IDropSource>,
            DROPEFFECT(bits),
        )
    };
    match done {
        Ok(e) => Ok(effect_of(e)),
        // Cancelled by the user (DRAGDROP_S_CANCEL is a success code, so errors are real).
        Err(e) => Err(fail(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

    fn temp_files() -> (PathBuf, Vec<PathBuf>) {
        let dir = std::env::temp_dir().join(format!("remedy-dnd-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let files: Vec<PathBuf> = ["a.txt", "b c.txt"].iter().map(|n| dir.join(n)).collect();
        for f in &files {
            let _ = std::fs::write(f, b"x");
        }
        (dir, files)
    }

    #[test]
    fn what_we_drag_out_reads_back_as_other_apps_see_it() {
        // SAFETY: OLE for this test thread.
        let _ = unsafe { OleInitialize(None) };
        let (dir, files) = temp_files();
        let data = DragData {
            files: files.clone(),
            text: Some("héllo".into()),
            uris: vec!["https://example.com/".into()],
        };
        let obj = data_object(&data).unwrap_or_else(|e| unreachable!("{e}"));
        let back = read(&obj);
        assert_eq!(back.files, files);
        assert_eq!(back.text.as_deref(), Some("héllo"));
        assert_eq!(back.uris, vec!["https://example.com/".to_owned()]);
        // A link alone also reads as text, for apps that only take text.
        let link = read(
            &data_object(&DragData { uris: vec!["https://remedy.example/".into()], ..DragData::default() })
                .unwrap_or_else(|e| unreachable!("{e}")),
        );
        assert_eq!(link.text.as_deref(), Some("https://remedy.example/"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_drop_target_reports_what_ole_hands_it() {
        // SAFETY: OLE for this test thread.
        let _ = unsafe { OleInitialize(None) };
        let (dir, files) = temp_files();
        let queue: Queue = Rc::default();
        // The desktop window: client and screen coordinates are the same.
        // SAFETY: always valid.
        let desktop = unsafe { GetDesktopWindow() }.0 as isize;
        let effect = Rc::new(std::cell::Cell::new(DropEffect::None));
        let target: IDropTarget = Target {
            hwnd: desktop,
            scale: Rc::new(std::cell::Cell::new(2.0)),
            queue: queue.clone(),
            effect: effect.clone(),
        }
        .into();
        let obj = data_object(&DragData::files(files.clone())).unwrap_or_else(|e| unreachable!("{e}"));
        let mut allowed = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0);
        // SAFETY: called exactly as OLE calls a drop target.
        unsafe {
            target
                .DragEnter(&obj, MODIFIERKEYS_FLAGS(0), POINTL { x: 200, y: 100 }, &mut allowed)
                .unwrap_or_else(|e| unreachable!("{e}"));
        }
        assert_eq!(allowed, DROPEFFECT_NONE, "nothing promised before the app has seen it");
        // The app accepts a copy; the next move says so.
        effect.set(DropEffect::Copy);
        let mut allowed = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0);
        // SAFETY: as above.
        unsafe { target.DragOver(MODIFIERKEYS_FLAGS(0), POINTL { x: 210, y: 120 }, &mut allowed) }
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(allowed, DROPEFFECT_COPY);
        // A move the source does not allow is not offered.
        effect.set(DropEffect::Move);
        let mut copy_only = DROPEFFECT_COPY;
        // SAFETY: as above.
        unsafe { target.DragOver(MODIFIERKEYS_FLAGS(0), POINTL { x: 210, y: 120 }, &mut copy_only) }
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(copy_only, DROPEFFECT_NONE);
        effect.set(DropEffect::Copy);
        let mut allowed = DROPEFFECT_COPY;
        // SAFETY: as above.
        unsafe { target.Drop(&obj, MODIFIERKEYS_FLAGS(0), POINTL { x: 220, y: 140 }, &mut allowed) }
            .unwrap_or_else(|e| unreachable!("{e}"));
        let events: Vec<DragEvent> = queue.borrow().iter().map(|(_, e)| e.clone()).collect();
        assert_eq!(events.len(), 4);
        assert!(
            matches!(events.first(), Some(DragEvent::Enter { position: Some(p), data }) if *p == Point::new(100.0, 50.0) && data.files == files)
        );
        assert!(
            matches!(events.get(1), Some(DragEvent::Over { position }) if *position == Point::new(105.0, 60.0))
        );
        assert!(
            matches!(events.get(3), Some(DragEvent::Drop { position: Some(p), data }) if *p == Point::new(110.0, 70.0) && data.files == files)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
