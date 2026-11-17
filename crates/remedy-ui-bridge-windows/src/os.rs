//! Win32 calls: the only unsafe code in the Windows bridge. Every function here is a small,
//! safe wrapper that checks each result.

pub(crate) mod instance;
pub(crate) mod tray;
pub(crate) mod tsf;

use std::ffi::c_void;
use std::time::Duration;

use remedy_ui_core::Color;
use remedy_ui_host::Unsupported;
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
use windows_sys::Win32::Graphics::Dwm::DwmGetColorizationColor;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCaretBlinkTime, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SetWindowDisplayAffinity,
    SystemParametersInfoW, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error(what: &'static str) -> Unsupported {
    Unsupported::new(what, std::io::Error::last_os_error().to_string())
}

/// The clipboard, opened for the duration of one operation.
struct Open;

impl Open {
    fn new() -> Result<Open, Unsupported> {
        // The clipboard is a shared resource another process may hold briefly; retry for a
        // short moment before reporting it busy.
        for attempt in 0..10 {
            // SAFETY: a null owner is allowed; success is paired with CloseClipboard in Drop.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Open);
            }
            std::thread::sleep(Duration::from_millis(5 * (attempt + 1)));
        }
        Err(last_error("the clipboard"))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: only reached after a successful OpenClipboard.
        unsafe { CloseClipboard() };
    }
}

/// The clipboard's Unicode text, if it holds any.
pub(crate) fn clipboard_text() -> Result<Option<String>, Unsupported> {
    let _open = Open::new()?;
    // SAFETY: the clipboard is open; the handle is owned by the clipboard and only read here.
    let handle = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT)) };
    if handle.is_null() {
        return Ok(None);
    }
    // SAFETY: CF_UNICODETEXT data is a global memory object; lock, copy out within its size,
    // unlock.
    unsafe {
        let size = GlobalSize(handle);
        let ptr = GlobalLock(handle).cast::<u16>();
        if ptr.is_null() {
            return Err(last_error("the clipboard"));
        }
        let units = std::slice::from_raw_parts(ptr, size / 2);
        let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
        let text = String::from_utf16_lossy(units.get(..end).unwrap_or_default());
        GlobalUnlock(handle);
        Ok(Some(text))
    }
}

fn global_copy(bytes: &[u8]) -> Result<*mut c_void, Unsupported> {
    // SAFETY: allocate, lock, copy exactly `bytes.len()` bytes, unlock. On failure the block is
    // freed; on success ownership passes to the clipboard through SetClipboardData.
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if mem.is_null() {
            return Err(last_error("the clipboard"));
        }
        let ptr = GlobalLock(mem).cast::<u8>();
        if ptr.is_null() {
            GlobalFree(mem);
            return Err(last_error("the clipboard"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        GlobalUnlock(mem);
        Ok(mem)
    }
}

fn set_data(format: u32, bytes: &[u8]) -> Result<(), Unsupported> {
    let mem = global_copy(bytes)?;
    // SAFETY: the clipboard is open and emptied by us; on success it owns `mem`.
    if unsafe { SetClipboardData(format, mem) }.is_null() {
        // SAFETY: the clipboard did not take it, so it is still ours to free.
        unsafe { GlobalFree(mem) };
        return Err(last_error("the clipboard"));
    }
    Ok(())
}

fn format(name: &str) -> u32 {
    let w = wide(name);
    // SAFETY: a valid NUL-terminated UTF-16 string.
    unsafe { RegisterClipboardFormatW(w.as_ptr()) }
}

/// Writes text. Sensitive text is marked so clipboard monitors, clipboard history (Win+V) and
/// cloud clipboard sync skip it.
pub(crate) fn set_clipboard_text(text: &str, sensitive: bool) -> Result<(), Unsupported> {
    let units = wide(text);
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    let _open = Open::new()?;
    // SAFETY: the clipboard is open.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(last_error("the clipboard"));
    }
    set_data(u32::from(CF_UNICODETEXT), &bytes)?;
    if sensitive {
        let zero = 0u32.to_le_bytes();
        set_data(format("ExcludeClipboardContentFromMonitorProcessing"), &zero)?;
        set_data(format("CanIncludeInClipboardHistory"), &zero)?;
        set_data(format("CanUploadToCloudClipboard"), &zero)?;
    }
    Ok(())
}

/// Empties the clipboard.
pub(crate) fn clear_clipboard() -> Result<(), Unsupported> {
    let _open = Open::new()?;
    // SAFETY: the clipboard is open.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(last_error("the clipboard"));
    }
    Ok(())
}

/// Excludes a window from screenshots, recording and screen sharing (Windows 10 2004+), or
/// includes it again.
pub(crate) fn set_capture_protected(hwnd: isize, on: bool) -> Result<(), Unsupported> {
    let affinity = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    // SAFETY: `hwnd` is a live top-level window owned by this process (from winit).
    if unsafe { SetWindowDisplayAffinity(hwnd as HWND, affinity) } == 0 {
        return Err(last_error("capture protection"));
    }
    Ok(())
}

fn reg_dword(key: &str, value: &str) -> Option<u32> {
    let (k, v) = (wide(key), wide(value));
    let mut data = 0u32;
    let mut size = 4u32;
    // SAFETY: valid strings; `data` holds the 4 bytes a REG_DWORD needs, `size` says so.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut data).cast(),
            &raw mut size,
        )
    };
    (status == 0).then_some(data)
}

/// System settings as Windows reports them.
pub(crate) struct Settings {
    pub(crate) dark: Option<bool>,
    pub(crate) accent: Option<Color>,
    pub(crate) high_contrast: bool,
    pub(crate) text_scale: Option<f32>,
    pub(crate) animations: Option<bool>,
    pub(crate) transparency: Option<bool>,
    pub(crate) locale: Option<String>,
    pub(crate) double_click: Duration,
    pub(crate) caret_blink: Option<Duration>,
}

pub(crate) fn settings() -> Settings {
    const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    let dark = reg_dword(PERSONALIZE, "AppsUseLightTheme").map(|v| v == 0);
    let transparency = reg_dword(PERSONALIZE, "EnableTransparency").map(|v| v != 0);
    let text_scale =
        reg_dword(r"Software\Microsoft\Accessibility", "TextScaleFactor").map(|v| v as f32 / 100.0);

    let mut argb = 0u32;
    let mut opaque = 0;
    // SAFETY: two valid out-pointers.
    let accent = (unsafe { DwmGetColorizationColor(&raw mut argb, &raw mut opaque) } >= 0).then(|| {
        let [b, g, r, _] = argb.to_le_bytes();
        Color::rgb8(r, g, b)
    });

    let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    // SAFETY: `hc` is a correctly sized HIGHCONTRASTW, as SPI_GETHIGHCONTRAST requires.
    let ok = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, (&raw mut hc).cast(), 0) } != 0;
    let high_contrast = ok && hc.dwFlags & HCF_HIGHCONTRASTON != 0;

    let mut anim = 0i32;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL.
    let animations =
        (unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut anim).cast(), 0) } != 0)
            .then_some(anim != 0);

    let mut buf = [0u16; 85];
    // SAFETY: the buffer holds LOCALE_NAME_MAX_LENGTH (85) units.
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), 85) };
    let locale = (n > 1).then(|| String::from_utf16_lossy(buf.get(..(n as usize - 1)).unwrap_or_default()));

    // SAFETY: no arguments.
    let double_click = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
    // SAFETY: no arguments. INFINITE (u32::MAX) means blinking is off.
    let blink = unsafe { GetCaretBlinkTime() };
    let caret_blink = (blink != u32::MAX && blink != 0).then(|| Duration::from_millis(u64::from(blink)));

    Settings {
        dark,
        accent,
        high_contrast,
        text_scale,
        animations,
        transparency,
        locale,
        double_click,
        caret_blink,
    }
}

/// Seconds east of UTC at `unix_seconds` in the user's time zone, with that instant's
/// daylight-saving rule (Windows' dynamic time zone data, so historical rules apply).
pub(crate) fn utc_offset(unix_seconds: i64) -> Option<i32> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, FileTimeToSystemTime, GetDynamicTimeZoneInformation,
        SystemTimeToFileTime, SystemTimeToTzSpecificLocalTimeEx,
    };
    // 100 ns ticks since 1601-01-01.
    const EPOCH_DIFF: i64 = 11_644_473_600;
    let ticks = u64::try_from(unix_seconds.checked_add(EPOCH_DIFF)?.checked_mul(10_000_000)?).ok()?;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    // SAFETY: plain out-parameters, all initialized before use, with the sizes Windows expects.
    unsafe {
        let mut zone: DYNAMIC_TIME_ZONE_INFORMATION = std::mem::zeroed();
        if GetDynamicTimeZoneInformation(&mut zone) == u32::MAX {
            return None;
        }
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&ft, &mut utc) == 0 {
            return None;
        }
        let mut local: SYSTEMTIME = std::mem::zeroed();
        if SystemTimeToTzSpecificLocalTimeEx(&zone, &utc, &mut local) == 0 {
            return None;
        }
        let mut lft: FILETIME = std::mem::zeroed();
        if SystemTimeToFileTime(&local, &mut lft) == 0 {
            return None;
        }
        let local_ticks = (u64::from(lft.dwHighDateTime) << 32) | u64::from(lft.dwLowDateTime);
        // SYSTEMTIME keeps milliseconds; the difference is whole seconds of offset.
        let diff = (local_ticks as i64 - ticks as i64) / 10_000_000;
        i32::try_from(diff).ok()
    }
}
