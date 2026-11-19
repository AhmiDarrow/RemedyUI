//! Icons from premultiplied RGBA: the tray's, and each window's own (`WM_SETICON`, small for
//! the title bar and big for the taskbar and Alt+Tab, at the window's DPI).

use remedy_ui_host::{Unsupported, WindowIcon};
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, HICON, ICON_BIG, ICON_SMALL, ICONINFO, SM_CXICON, SM_CXSMICON,
    SendMessageW, WM_SETICON,
};

/// An icon from premultiplied RGBA: straight-alpha BGRA, as 32-bit icons expect. `what` names
/// the feature in errors.
pub(crate) fn hicon(size: u32, rgba: &[u8], what: &'static str) -> Result<HICON, Unsupported> {
    if rgba.len() != size as usize * size as usize * 4 || size == 0 {
        return Err(Unsupported::new(what, "the icon is not size × size RGBA"));
    }
    let bgra: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| {
            let un = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8
                }
            };
            [un(b), un(g), un(r), a]
        })
        .collect();
    let s = size as i32;
    let mask = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
    // SAFETY: bitmaps from buffers of the right sizes, turned into an icon, then freed.
    unsafe {
        let color = CreateBitmap(s, s, 1, 32, bgra.as_ptr().cast());
        let mono = CreateBitmap(s, s, 1, 1, mask.as_ptr().cast());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mono, hbmColor: color };
        let h = CreateIconIndirect(&raw const info);
        DeleteObject(color);
        DeleteObject(mono);
        if h.is_null() {
            return Err(Unsupported::new(what, std::io::Error::last_os_error().to_string()));
        }
        Ok(h)
    }
}

/// The two icons a window shows, destroyed when dropped (after the window let go of them).
#[derive(Debug)]
pub(crate) struct WindowIcons {
    small: HICON,
    big: HICON,
}

impl Drop for WindowIcons {
    fn drop(&mut self) {
        for h in [self.small, self.big] {
            // SAFETY: icons this module made; no window shows them any more.
            unsafe { DestroyIcon(h) };
        }
    }
}

/// Gives `hwnd` its own icon at the sizes Windows shows for its DPI, or back the app's (the
/// executable's) with `None`. Returns the icons now shown, which must outlive their use.
pub(crate) fn set_window_icon(
    hwnd: isize,
    icon: Option<&WindowIcon>,
) -> Result<Option<WindowIcons>, Unsupported> {
    const WHAT: &str = "window icons";
    let send = |kind: u32, h: HICON| {
        // SAFETY: `hwnd` is a live window of this process (from winit); WM_SETICON takes an
        // icon handle or null and returns the previous one, which stays the caller's.
        unsafe { SendMessageW(hwnd as HWND, WM_SETICON, kind as WPARAM, h as LPARAM) };
    };
    let Some(icon) = icon else {
        send(ICON_SMALL, std::ptr::null_mut());
        send(ICON_BIG, std::ptr::null_mut());
        return Ok(None);
    };
    // SAFETY: plain queries on a live window.
    let (small, big) = unsafe {
        let dpi = GetDpiForWindow(hwnd as HWND).max(96);
        (GetSystemMetricsForDpi(SM_CXSMICON, dpi), GetSystemMetricsForDpi(SM_CXICON, dpi))
    };
    let make = |px: i32| {
        let s = icon.scaled(u32::try_from(px).unwrap_or(16).max(1));
        hicon(s.size(), s.rgba(), WHAT)
    };
    let small = make(small)?;
    let big = match make(big) {
        Ok(h) => h,
        Err(e) => {
            // SAFETY: made above and never shown.
            unsafe { DestroyIcon(small) };
            return Err(e);
        }
    };
    send(ICON_SMALL, small);
    send(ICON_BIG, big);
    Ok(Some(WindowIcons { small, big }))
}
