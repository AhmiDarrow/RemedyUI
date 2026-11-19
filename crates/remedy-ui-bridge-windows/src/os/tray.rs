//! The notification-area icon: `Shell_NotifyIconW` on a hidden message-only window that
//! receives the icon's clicks, and a native popup menu.

use std::cell::{Cell, RefCell};

use remedy_ui_host::{TrayEvent, TrayIcon, TrayItem, Unsupported};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, GetCursorPos,
    HICON, HWND_MESSAGE, MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassExW,
    RegisterWindowMessageW, SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSEXW,
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
}

fn push(e: TrayEvent) {
    EVENTS.with(|v| v.borrow_mut().push(e));
}

/// Tray events since the last call.
pub(crate) fn take_events() -> Vec<TrayEvent> {
    EVENTS.with(|v| std::mem::take(&mut *v.borrow_mut()))
}

fn show_menu(hwnd: HWND) {
    let items = STATE
        .with(|s| s.borrow().as_ref().and_then(|s| s.tray.as_ref().map(|t| t.menu.clone())))
        .unwrap_or_default();
    if items.is_empty() {
        return;
    }
    // SAFETY: a menu built, shown modally for `hwnd` (ours, on this thread), and destroyed.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for item in &items {
            match item {
                TrayItem::Item { id, label, enabled } => {
                    let text = wide(label);
                    let flags = MF_STRING | if *enabled { 0 } else { MF_GRAYED };
                    AppendMenuW(menu, flags, (*id as usize) + 1, text.as_ptr());
                }
                TrayItem::Separator => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                }
            }
        }
        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&raw mut at);
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
        match lparam as u32 {
            WM_LBUTTONUP => push(TrayEvent::Activate),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
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
            }
        });
        if !old_icon.is_null() {
            // SAFETY: the icon is no longer shown.
            unsafe { DestroyIcon(old_icon) };
        }
        return Ok(());
    };
    let h = super::icon::hicon(t.icon.size, &t.icon.rgba, "the tray")?;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
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
