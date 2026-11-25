//! The notification-area icon: `Shell_NotifyIconW` on a hidden message-only window that
//! receives the icon's clicks, and a native popup menu.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use remedy_ui_core::host::{TrayEvent, TrayIcon, TrayItem, Unsupported};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
    NINF_KEY, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, HICON, HMENU,
    HWND_MESSAGE, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassExW,
    RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, SetMenuDefaultItem, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_NULL, WNDCLASSEXW,
};

use super::wide;

/// The message the shell sends for icon clicks.
const CALLBACK: u32 = WM_APP + 0x51;

struct State {
    hwnd: HWND,
    icon: HICON,
    tray: Option<TrayIcon>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static EVENTS: RefCell<Vec<TrayEvent>> = const { RefCell::new(Vec::new()) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static LAST_ACTIVATE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The select the keyboard sends (Win+B, then Enter or Space on the icon).
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

fn push(e: TrayEvent) {
    EVENTS.with(|v| v.borrow_mut().push(e));
}

/// Activates, once per double-click time: a double-click arrives as two selects, and Enter
/// on the focused icon may repeat.
fn activate(now: Instant) {
    // SAFETY: a plain system query.
    let window = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
    let fresh = LAST_ACTIVATE.with(|l| {
        let fresh = l.get().is_none_or(|at| now.duration_since(at) >= window);
        l.set(Some(now));
        fresh
    });
    if fresh {
        push(TrayEvent::Activate);
    }
}

/// Tray events since the last call.
pub(crate) fn take_events() -> Vec<TrayEvent> {
    EVENTS.with(|v| std::mem::take(&mut *v.borrow_mut()))
}

/// A popup menu of `items`; command ids are the app's ids plus one (0 is "nothing chosen").
/// The caller destroys it, which also destroys its submenus.
pub(crate) fn build_menu(items: &[TrayItem]) -> HMENU {
    // SAFETY: a new menu filled with entries whose strings outlive each call.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return menu;
        }
        for item in items {
            let gray = |enabled: bool| if enabled { 0 } else { MF_GRAYED };
            match item {
                TrayItem::Item { id, label, enabled, default } => {
                    let cmd = (*id as usize) + 1;
                    AppendMenuW(menu, MF_STRING | gray(*enabled), cmd, wide(label).as_ptr());
                    if *default {
                        SetMenuDefaultItem(menu, cmd as u32, 0);
                    }
                }
                TrayItem::Check { id, label, enabled, checked } => {
                    let flags = MF_STRING | gray(*enabled) | if *checked { MF_CHECKED } else { 0 };
                    AppendMenuW(menu, flags, (*id as usize) + 1, wide(label).as_ptr());
                }
                TrayItem::Submenu { label, items } => {
                    let sub = build_menu(items);
                    AppendMenuW(menu, MF_POPUP | gray(!items.is_empty()), sub as usize, wide(label).as_ptr());
                }
                TrayItem::Separator => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                }
            }
        }
        menu
    }
}

/// Shows the menu at `at` (screen pixels: where the shell says, for a click or for
/// Shift+F10 and the menu key).
fn show_menu(hwnd: HWND, at: POINT) {
    let items = STATE
        .with(|s| s.borrow().as_ref().and_then(|s| s.tray.as_ref().map(|t| t.menu.clone())))
        .unwrap_or_default();
    if items.is_empty() {
        return;
    }
    // SAFETY: a menu built, shown modally for `hwnd` (ours, on this thread), and destroyed.
    unsafe {
        let menu = build_menu(&items);
        if menu.is_null() {
            return;
        }
        // The menu closes when focus leaves only if our window is foreground first.
        SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            at.x,
            at.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        if chosen > 0
            && let Ok(id) = u32::try_from(chosen - 1)
        {
            push(TrayEvent::Menu(id));
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == CALLBACK {
        // NOTIFYICON_VERSION_4: the event in the low word, the anchor point in `wparam`.
        let at = POINT { x: i32::from(wparam as u16 as i16), y: i32::from((wparam >> 16) as u16 as i16) };
        match lparam as u32 & 0xFFFF {
            NIN_SELECT | NIN_KEYSELECT => activate(Instant::now()),
            WM_CONTEXTMENU => show_menu(hwnd, at),
            _ => {}
        }
        return 0;
    }
    let restarted = TASKBAR_CREATED.with(Cell::get);
    if restarted != 0 && msg == restarted {
        // Explorer restarted: the icon must be added again.
        let tray = STATE.with(|s| s.borrow().as_ref().and_then(|s| s.tray.clone()));
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.tray = None;
            }
        });
        if let Some(t) = tray {
            let _ = set(Some(&t));
        }
        return 0;
    }
    // SAFETY: the default handling for everything else.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn window() -> Result<HWND, Unsupported> {
    if let Some(h) = STATE.with(|s| s.borrow().as_ref().map(|s| s.hwnd)) {
        return Ok(h);
    }
    let class = wide("RemedyUI.Tray");
    // SAFETY: registering a class whose window procedure is `wndproc`, then creating a
    // message-only window of it on this thread.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&raw const wc);
        TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr())));
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err(Unsupported::new("the tray", std::io::Error::last_os_error().to_string()));
    }
    STATE.with(|s| *s.borrow_mut() = Some(State { hwnd, icon: std::ptr::null_mut(), tray: None }));
    Ok(hwnd)
}

/// Shows, updates or removes the icon.
pub(crate) fn set(tray: Option<&TrayIcon>) -> Result<(), Unsupported> {
    let hwnd = window()?;
    let (had, old_icon) = STATE
        .with(|s| s.borrow().as_ref().map_or((false, std::ptr::null_mut()), |s| (s.tray.is_some(), s.icon)));
    // SAFETY: a zeroed NOTIFYICONDATAW is a valid empty request; fields are set below.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    let Some(t) = tray else {
        if had {
            // SAFETY: removing the icon this window added.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &raw const data) };
        }
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.tray = None;
                // Forgotten with the icon, or the next one shown would destroy it again.
                st.icon = std::ptr::null_mut();
            }
        });
        if !old_icon.is_null() {
            // SAFETY: the icon is no longer shown.
            unsafe { DestroyIcon(old_icon) };
        }
        return Ok(());
    };
    // The size the notification area shows at the system DPI, drawn or scaled from the art.
    // SAFETY: plain system queries.
    let px = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem().max(96)) };
    let img = t.image_at(u32::try_from(px).unwrap_or(16).max(1));
    let h = super::icon::hicon(img.size, &img.rgba, "the tray")?;
    // Version 4 hides the tooltip unless asked (NIF_SHOWTIP).
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = CALLBACK;
    data.hIcon = h;
    for (dst, src) in data.szTip.iter_mut().zip(t.tooltip.encode_utf16().take(127)) {
        *dst = src;
    }
    // SAFETY: adding or modifying our icon with a complete request.
    let ok = unsafe { Shell_NotifyIconW(if had { NIM_MODIFY } else { NIM_ADD }, &raw const data) } != 0;
    if !ok {
        // SAFETY: the new icon was not taken.
        unsafe { DestroyIcon(h) };
        return Err(Unsupported::new("the tray", "the shell refused the icon"));
    }
    if !had {
        // Keyboard selection (NIN_KEYSELECT) and the menu's anchor point come only in version 4.
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: setting the version of the icon just added.
        unsafe { Shell_NotifyIconW(NIM_SETVERSION, &raw const data) };
    }
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.tray = Some(t.clone());
            st.icon = h;
        }
    });
    if !old_icon.is_null() {
        // SAFETY: replaced by the new icon.
        unsafe { DestroyIcon(old_icon) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMenuItemCount, GetMenuItemInfoW, MENUITEMINFOW, MFS_CHECKED, MFS_DEFAULT, MFS_DISABLED,
        MFT_SEPARATOR, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU,
    };

    /// (label, id, state, separator, submenu) of each entry, by position.
    fn entries(menu: HMENU) -> Vec<(String, u32, u32, bool, HMENU)> {
        // SAFETY: reading the entries of a live menu into buffers that outlive each call.
        unsafe {
            (0..GetMenuItemCount(menu) as u32)
                .map(|i| {
                    let mut text = [0u16; 64];
                    let mut mi: MENUITEMINFOW = std::mem::zeroed();
                    mi.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
                    mi.fMask = MIIM_FTYPE | MIIM_ID | MIIM_STATE | MIIM_STRING | MIIM_SUBMENU;
                    mi.dwTypeData = text.as_mut_ptr();
                    mi.cch = text.len() as u32;
                    assert_ne!(GetMenuItemInfoW(menu, i, 1, &raw mut mi), 0);
                    let label = String::from_utf16_lossy(text.get(..mi.cch as usize).unwrap_or_default());
                    (label, mi.wID, mi.fState, mi.fType & MFT_SEPARATOR != 0, mi.hSubMenu)
                })
                .collect()
        }
    }

    #[test]
    fn the_real_menu_has_checks_submenus_and_a_bold_default() {
        let items = vec![
            TrayItem::Item { id: 1, label: "Open".into(), enabled: true, default: true },
            TrayItem::check(2, "Allow TVs", true),
            TrayItem::Check { id: 3, label: "GPU".into(), enabled: false, checked: false },
            TrayItem::Separator,
            TrayItem::submenu(
                "Server",
                vec![TrayItem::item(4, "Restart"), TrayItem::check(5, "Logs", false)],
            ),
        ];
        let menu = build_menu(&items);
        let [open, tvs, gpu, line, server] =
            entries(menu).try_into().unwrap_or_else(|_| unreachable!("five"));
        assert_eq!((open.0.as_str(), open.1), ("Open", 2), "ids are the app's plus one");
        assert_ne!(open.2 & MFS_DEFAULT, 0, "the default is bold");
        assert_ne!(tvs.2 & MFS_CHECKED, 0);
        assert_eq!(gpu.2 & MFS_CHECKED, 0);
        assert_ne!(gpu.2 & MFS_DISABLED, 0, "grayed");
        assert!(line.3, "a separator");
        assert_eq!(server.0, "Server");
        assert!(!server.4.is_null(), "a submenu");
        let sub = entries(server.4);
        assert_eq!(
            sub.iter().map(|e| (e.0.as_str(), e.1)).collect::<Vec<_>>(),
            [("Restart", 5), ("Logs", 6)]
        );
        // SAFETY: destroying the menu built above (and its submenu).
        unsafe { DestroyMenu(menu) };
    }

    #[test]
    fn keyboard_select_activates_and_a_double_click_activates_once() {
        let Ok(hwnd) = window() else { unreachable!("the tray's window") };
        let send = |event: u32| {
            // SAFETY: calling our own window procedure on its thread, as the shell's message would.
            unsafe { wndproc(hwnd, CALLBACK, 0, (event | (1 << 16)) as LPARAM) };
        };
        LAST_ACTIVATE.with(|l| l.set(None));
        take_events();
        send(NIN_KEYSELECT);
        assert_eq!(take_events(), [TrayEvent::Activate], "Enter or Space on the focused icon");
        LAST_ACTIVATE.with(|l| l.set(None));
        send(NIN_SELECT);
        send(NIN_SELECT);
        assert_eq!(take_events(), [TrayEvent::Activate], "a double-click is two selects, one activation");
        // SAFETY: a plain system query.
        let window = Duration::from_millis(u64::from(unsafe { GetDoubleClickTime() }));
        let later = Instant::now() + window;
        activate(later);
        assert_eq!(take_events(), [TrayEvent::Activate], "a click after the double-click time counts again");
    }
}
