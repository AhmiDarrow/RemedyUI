//! The app's badge on Windows: an overlay icon on each window's taskbar button
//! (`ITaskbarList3::SetOverlayIcon`), drawn as a red disc with the count, at the size Windows
//! shows overlays for the window's DPI. Screen readers get the badge's text as its
//! description.

use remedy_ui_host::{Badge, Unsupported};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::core::{HSTRING, PCWSTR};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, SM_CXSMICON};

fn err(e: impl std::fmt::Display) -> Unsupported {
    Unsupported::new("app badges", format!("the taskbar: {e}"))
}

/// Shows `badge` on the taskbar buttons of `hwnds`, or takes it away.
pub(crate) fn set(hwnds: &[isize], badge: Option<Badge>) -> Result<(), Unsupported> {
    super::com_for_winrt();
    // SAFETY: COM is initialized on this thread; the taskbar object is used within the call.
    let taskbar: ITaskbarList3 =
        unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) }.map_err(err)?;
    // SAFETY: as above.
    unsafe { taskbar.HrInit() }.map_err(err)?;
    let mut last = Ok(());
    for &h in hwnds {
        let hwnd = HWND(h as *mut _);
        let Some(b) = badge else {
            // SAFETY: a window of the app; a null icon takes the overlay away.
            if let Err(e) = unsafe { taskbar.SetOverlayIcon(hwnd, HICON::default(), PCWSTR::null()) } {
                last = Err(err(e));
            }
            continue;
        };
        // SAFETY: reading a window's DPI and the system's small-icon size for it.
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd.0)) };
        let size = u32::try_from(size).ok().filter(|s| *s > 0).unwrap_or(16);
        let img = b.image(size);
        let icon = super::icon::hicon(img.size, &img.rgba, "app badges")?;
        let text = HSTRING::from(match b {
            Badge::Count(_) => b.label(),
            _ => "New activity".to_owned(),
        });
        // SAFETY: the taskbar copies the icon, which is destroyed right after.
        let r = unsafe { taskbar.SetOverlayIcon(hwnd, HICON(icon), &text) };
        // SAFETY: the icon made above, no longer used.
        unsafe { DestroyIcon(icon) };
        if let Err(e) = r {
            last = Err(err(e));
        }
    }
    last
}
