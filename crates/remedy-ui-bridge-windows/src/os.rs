//! Win32 calls: the only unsafe code in the Windows bridge. Every function here is a small,
//! safe wrapper that checks each result.

pub(crate) mod badge;
pub(crate) mod dnd;
pub(crate) mod gamepad;
pub(crate) mod icon;
pub(crate) mod instance;
pub(crate) mod print;
pub(crate) mod share;
pub(crate) mod toast;
pub(crate) mod tray;
pub(crate) mod tsf;

use std::ffi::c_void;
use std::time::Duration;

use remedy_ui_core::Color;
use remedy_ui_host::Unsupported;
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::Globalization::{
    GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME,
};
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
    FE_FONTSMOOTHINGCLEARTYPE, FE_FONTSMOOTHINGORIENTATIONBGR, GetCaretBlinkTime, SPI_GETCLIENTAREAANIMATION,
    SPI_GETFONTSMOOTHING, SPI_GETFONTSMOOTHINGORIENTATION, SPI_GETFONTSMOOTHINGTYPE, SPI_GETHIGHCONTRAST,
    SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED, SetWindowDisplayAffinity, SystemParametersInfoW,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

/// Joins the calling thread to a COM apartment for WinRT calls, keeping the one it is in (the
/// UI thread's single-threaded apartment, which WinRT also serves).
pub(crate) fn com_for_winrt() {
    // SAFETY: plain COM initialization for the calling thread; an already-initialized thread
    // returns S_FALSE or RPC_E_CHANGED_MODE and keeps its apartment.
    let _ = unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        )
    };
}

/// Registers `id` as an application user model id for the current user, with the name and
/// icon Windows shows on the app's toasts (`HKCU\Software\Classes\AppUserModelId\<id>`).
pub(crate) fn register_app_id(id: &str, name: &str, icon: Option<&std::path::Path>) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{
        HKEY, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW,
    };
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let path = wide(&format!("Software\\Classes\\AppUserModelId\\{id}"));
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: creating or opening a key under the current user's hive with a NUL-terminated
    // path; the handle is closed below.
    let r = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if r != 0 {
        return Err(format!("the app's notification identity could not be registered (error {r})"));
    }
    let set = |value: &str, data: &str| -> u32 {
        let v = wide(value);
        let d = wide(data);
        // SAFETY: writing a NUL-terminated UTF-16 string value of the stated byte length to
        // the open key.
        unsafe {
            RegSetValueExW(
                key,
                v.as_ptr(),
                0,
                REG_SZ,
                d.as_ptr().cast(),
                u32::try_from(d.len() * 2).unwrap_or(u32::MAX),
            )
        }
    };
    let mut r = set("DisplayName", name);
    if r == 0
        && let Some(icon) = icon
    {
        r = set("IconUri", &icon.to_string_lossy());
    }
    // SAFETY: closing the key opened above.
    unsafe { RegCloseKey(key) };
    if r != 0 {
        return Err(format!("the app's notification name could not be registered (error {r})"));
    }
    Ok(())
}

/// Opens a web or mail link with the user's default app.
pub(crate) fn open_url(url: &str) -> Result<(), Unsupported> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (verb, target) = (wide("open"), wide(url));
    // SAFETY: NUL-terminated wide strings that outlive the call; no window, parameters or
    // directory.
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    if r as isize > 32 {
        Ok(())
    } else {
        Err(Unsupported::new("opening links", format!("the shell refused it (code {})", r as isize)))
    }
}

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

/// The machine's physical memory in bytes.
pub(crate) fn total_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m =
        MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `m` is a MEMORYSTATUSEX with its length set, as the call requires.
    (unsafe { GlobalMemoryStatusEx(&mut m) } != 0).then_some(m.ullTotalPhys).filter(|n| *n > 0)
}

/// The clipboard's HTML ("HTML Format"), if another app copied formatted text.
pub(crate) fn clipboard_html() -> Result<Option<String>, Unsupported> {
    let html_format = format("HTML Format");
    let _open = Open::new()?;
    // SAFETY: the clipboard is open; the handle is owned by the clipboard and only read here.
    let handle = unsafe { GetClipboardData(html_format) };
    if handle.is_null() {
        return Ok(None);
    }
    // SAFETY: "HTML Format" data is a global memory object; lock, copy out within its size,
    // unlock.
    let bytes = unsafe {
        let size = GlobalSize(handle);
        let ptr = GlobalLock(handle).cast::<u8>();
        if ptr.is_null() {
            return Err(last_error("the clipboard"));
        }
        let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
        GlobalUnlock(handle);
        bytes
    };
    Ok(Some(crate::cf_html::unwrap(&bytes)))
}

/// Writes formatted text: Unicode text for every app, and "HTML Format" for apps that paste
/// formatting.
pub(crate) fn set_clipboard_html(text: &str, html: &str) -> Result<(), Unsupported> {
    let units = wide(text);
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    let _open = Open::new()?;
    // SAFETY: the clipboard is open.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(last_error("the clipboard"));
    }
    set_data(u32::from(CF_UNICODETEXT), &bytes)?;
    set_data(format("HTML Format"), &crate::cf_html::wrap(html))
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
    pub(crate) languages: Vec<String>,
    pub(crate) double_click: Duration,
    pub(crate) caret_blink: Option<Duration>,
    pub(crate) subpixel_text: Option<bool>,
    pub(crate) power_saving: bool,
    /// The keyboard's repeat delay and interval, where Windows says.
    pub(crate) key_repeat: Option<(Duration, Duration)>,
}

/// The keyboard's repeat from its Control Panel settings: a delay of 0 to 3 (250 ms to 1 s)
/// and a speed of 0 to 31 (about 2.5 to 30 repeats a second).
pub(crate) fn key_repeat(delay: u32, speed: u32) -> (Duration, Duration) {
    let rate = 2.5 + f64::from(speed.min(31)) * 27.5 / 31.0;
    (Duration::from_millis(250 * (u64::from(delay.min(3)) + 1)), Duration::from_secs_f64(1.0 / rate))
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
    let languages = preferred_ui_languages();

    // SAFETY: no arguments.
    let double_click = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
    // SAFETY: no arguments. INFINITE (u32::MAX) means blinking is off.
    let blink = unsafe { GetCaretBlinkTime() };
    let caret_blink = (blink != u32::MAX && blink != 0).then(|| Duration::from_millis(u64::from(blink)));

    // ClearType: smoothing on, of the ClearType kind, and the panel's subpixel order.
    let spi_u32 = |action| {
        let mut v = 0u32;
        // SAFETY: these SPI_GET actions write one UINT (or BOOL) to the out-pointer.
        (unsafe { SystemParametersInfoW(action, 0, (&raw mut v).cast(), 0) } != 0).then_some(v)
    };
    let subpixel_text = (spi_u32(SPI_GETFONTSMOOTHING).is_some_and(|on| on != 0)
        && spi_u32(SPI_GETFONTSMOOTHINGTYPE) == Some(FE_FONTSMOOTHINGCLEARTYPE))
    .then(|| spi_u32(SPI_GETFONTSMOOTHINGORIENTATION) == Some(FE_FONTSMOOTHINGORIENTATIONBGR));

    let key_repeat =
        spi_u32(SPI_GETKEYBOARDDELAY).zip(spi_u32(SPI_GETKEYBOARDSPEED)).map(|(d, s)| key_repeat(d, s));

    Settings {
        key_repeat,
        power_saving: power_saving(),
        dark,
        accent,
        high_contrast,
        text_scale,
        animations,
        transparency,
        locale,
        languages,
        double_click,
        caret_blink,
        subpixel_text,
    }
}

/// The user's display languages, most preferred first (Settings, Time & language).
fn preferred_ui_languages() -> Vec<String> {
    let (mut count, mut len) = (0u32, 0u32);
    // SAFETY: a null buffer asks for the length, written to `len`.
    if unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &raw mut count, std::ptr::null_mut(), &raw mut len)
    } == 0
    {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` holds the `len` units the first call asked for.
    let ok = unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &raw mut count, buf.as_mut_ptr(), &raw mut len)
    };
    if ok == 0 { Vec::new() } else { multi_string(buf.get(..len as usize).unwrap_or_default()) }
}

/// The strings of a Win32 multi-string: each ends in a NUL, the list in another.
fn multi_string(units: &[u16]) -> Vec<String> {
    units.split(|u| *u == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect()
}

/// On battery power, or battery saver is on.
pub(crate) fn power_saving() -> bool {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: a plain out-parameter of the size Windows expects.
    unsafe {
        let mut s: SYSTEM_POWER_STATUS = std::mem::zeroed();
        // ACLineStatus 0 is battery; SystemStatusFlag 1 is battery saver.
        GetSystemPowerStatus(&mut s) != 0 && (s.ACLineStatus == 0 || s.SystemStatusFlag == 1)
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

/// The mouse pointer, read and moved for tests (`crate::pointer`).
pub(crate) mod pointer {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CURSOR_SHOWING, CURSORINFO, GetCursorInfo, GetCursorPos, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
        SetCursorPos,
    };

    /// Whether Windows shows the pointer now: `GetCursorInfo`'s `CURSOR_SHOWING`, with a cursor
    /// to show (a window may hide it with no cursor rather than the display count).
    pub fn showing() -> bool {
        let mut info = CURSORINFO {
            cbSize: size_of::<CURSORINFO>() as u32,
            flags: 0,
            hCursor: std::ptr::null_mut(),
            ptScreenPos: POINT { x: 0, y: 0 },
        };
        // SAFETY: `info` is a CURSORINFO with its size set, as GetCursorInfo asks.
        let ok = unsafe { GetCursorInfo(&mut info) } != 0;
        ok && info.flags & CURSOR_SHOWING != 0 && !info.hCursor.is_null()
    }

    /// Where the pointer is, in screen pixels.
    pub fn position() -> (i32, i32) {
        let mut at = POINT { x: 0, y: 0 };
        // SAFETY: `at` is a POINT for GetCursorPos to fill.
        let _ = unsafe { GetCursorPos(&mut at) };
        (at.x, at.y)
    }

    /// Moves the pointer to `x`, `y` in screen pixels.
    pub fn move_to(x: i32, y: i32) {
        // SAFETY: plain coordinates.
        let _ = unsafe { SetCursorPos(x, y) };
    }

    /// Clicks the left button where the pointer is (`SendInput`): the window under it comes to
    /// the front, as Windows gives the foreground to whoever the person clicks.
    pub fn click() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput,
        };
        let press = |flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        };
        let inputs = [press(MOUSEEVENTF_LEFTDOWN), press(MOUSEEVENTF_LEFTUP)];
        // SAFETY: `inputs` holds two INPUTs of the size passed.
        let _ = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    }

    /// The primary screen's size in pixels.
    pub fn screen_size() -> (i32, i32) {
        // SAFETY: GetSystemMetrics takes only the metric's index.
        unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
    }

    /// The top-level window under the pointer and the one in front: each one's class, title
    /// and process (marked when it is this one), for a test to say what was under the
    /// pointer when it showed.
    pub fn under() -> String {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GA_ROOT, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowTextW,
            GetWindowThreadProcessId, WindowFromPoint,
        };
        let mut at = POINT { x: 0, y: 0 };
        // SAFETY: `at` is a POINT for GetCursorPos to fill.
        let _ = unsafe { GetCursorPos(&mut at) };
        let describe = |h: windows_sys::Win32::Foundation::HWND| {
            if h.is_null() {
                return "none".to_owned();
            }
            let (mut class, mut title) = ([0u16; 128], [0u16; 128]);
            let mut pid = 0u32;
            // SAFETY: each buffer is passed with its length; `pid` is a u32 to fill.
            let (n, t) = unsafe {
                (GetClassNameW(h, class.as_mut_ptr(), 128), GetWindowTextW(h, title.as_mut_ptr(), 128))
            };
            // SAFETY: as above.
            let _ = unsafe { GetWindowThreadProcessId(h, &mut pid) };
            let s = |b: &[u16], n: i32| {
                String::from_utf16_lossy(b.get(..usize::try_from(n).unwrap_or(0)).unwrap_or_default())
            };
            let mine = if pid == std::process::id() { ", this process" } else { "" };
            format!("{:?} {:?} (process {pid}{mine})", s(&class, n), s(&title, t))
        };
        // SAFETY: plain coordinates and window handles.
        let (point, front) = unsafe { (GetAncestor(WindowFromPoint(at), GA_ROOT), GetForegroundWindow()) };
        format!(
            "under the pointer at ({}, {}): {}; in front: {}",
            at.x,
            at.y,
            describe(point),
            describe(front)
        )
    }
}

/// The pointer hidden over a window. winit hides it (`ShowCursor`) only while it has seen the
/// pointer come in and not leave: a `WM_MOUSELEAVE` it gets around a click, a capture or a
/// full-screen switch leaves the pointer showing over the window. Answering `WM_SETCURSOR`
/// with no cursor while hidden, and clearing the cursor at once when it is over the window,
/// hides it whatever winit believes.
pub(crate) mod cursor {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GA_ROOT, GetAncestor, GetCursorPos, HTCLIENT, SendMessageW, SetCursor, WM_MOUSEMOVE, WM_NCDESTROY,
        WM_SETCURSOR, WindowFromPoint,
    };

    const ID: usize = 0x5245_4d43; // "REMC"

    unsafe extern "system" fn proc(
        h: HWND,
        msg: u32,
        w: WPARAM,
        l: LPARAM,
        _: usize,
        hidden: usize,
    ) -> LRESULT {
        if msg == WM_SETCURSOR && hidden != 0 && (l & 0xFFFF) as u32 == HTCLIENT {
            // SAFETY: a null cursor hides the pointer; called on the window's thread.
            unsafe { SetCursor(std::ptr::null_mut()) };
            return 1;
        }
        if msg == WM_NCDESTROY {
            // SAFETY: the subclass this module set on `h`, removed as the window goes.
            unsafe { RemoveWindowSubclass(h, Some(proc), ID) };
        }
        // SAFETY: passes the message on, as a subclass procedure must.
        unsafe { DefSubclassProc(h, msg, w, l) }
    }

    /// Hides or shows the pointer over `hwnd` (a window of this thread).
    pub fn set_hidden(hwnd: isize, hidden: bool) {
        let h = hwnd as HWND;
        // SAFETY: `h` is a live window of this thread; setting the same subclass again only
        // changes its data (the hidden flag).
        unsafe { SetWindowSubclass(h, Some(proc), ID, usize::from(hidden)) };
        let mut at = POINT { x: 0, y: 0 };
        // SAFETY: `at` is a POINT for GetCursorPos; the rest take handles and plain values.
        let over = unsafe { GetCursorPos(&mut at) != 0 && GetAncestor(WindowFromPoint(at), GA_ROOT) == h };
        if over {
            // Asks the window for its cursor now rather than at the next move: none while
            // hidden, winit's own when shown.
            let l = (WM_MOUSEMOVE << 16 | HTCLIENT) as LPARAM;
            // SAFETY: a WM_SETCURSOR to a window of this thread, its own handle as wParam.
            unsafe { SendMessageW(h, WM_SETCURSOR, h as WPARAM, l) };
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn multi_strings_split_into_languages_in_order() {
        let units: Vec<u16> = "de-DE\0en-US\0fr\0\0".encode_utf16().collect();
        assert_eq!(super::multi_string(&units), ["de-DE", "en-US", "fr"]);
        assert!(super::multi_string(&[0, 0]).is_empty());
        assert!(super::multi_string(&[]).is_empty());
    }

    #[test]
    fn the_user_has_at_least_one_display_language() {
        let list = super::preferred_ui_languages();
        assert!(!list.is_empty());
        assert!(list.iter().all(|l| l.len() >= 2 && !l.contains('\0')), "{list:?}");
    }
}
