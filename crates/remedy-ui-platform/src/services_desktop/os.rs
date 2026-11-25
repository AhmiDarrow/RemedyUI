//! Platform calls: the only unsafe code in the desktop services.

use std::path::Path;

/// Writes a Windows shortcut (`.lnk`) at `file` that runs `program` with `args`, starting in
/// the program's folder.
#[cfg(windows)]
pub(crate) fn shortcut(file: &Path, program: &Path, args: &[String]) -> Result<(), String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface};
    let quoted: Vec<String> = args.iter().map(|a| quote_arg(a)).collect();
    let dir = program.parent().map(Path::to_path_buf).unwrap_or_default();
    // SAFETY: COM on this thread (a second initialization only adds a reference); every string
    // is an HSTRING that outlives its call.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(&HSTRING::from(program.as_os_str())).map_err(|e| e.to_string())?;
        link.SetArguments(&HSTRING::from(quoted.join(" "))).map_err(|e| e.to_string())?;
        link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str())).map_err(|e| e.to_string())?;
        let persist: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        persist.Save(&HSTRING::from(file.as_os_str()), true).map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
pub(crate) fn shortcut(_file: &Path, _program: &Path, _args: &[String]) -> Result<(), String> {
    Err("Shortcuts are a Windows file".into())
}

/// Opens the file manager at `path`'s folder with `path` selected.
#[cfg(windows)]
pub(crate) fn reveal(path: &Path) -> Result<(), String> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName};
    use windows::core::HSTRING;
    let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    // SAFETY: COM on this thread (a second initialization only adds a reference); the path is
    // an HSTRING that outlives the call; the item list Windows allocates is freed here.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        SHParseDisplayName(&HSTRING::from(path.as_os_str()), None, &mut pidl, 0, None)
            .map_err(|e| e.to_string())?;
        let r = SHOpenFolderAndSelectItems(pidl, None, 0).map_err(|e| e.to_string());
        CoTaskMemFree(Some(pidl.cast_const().cast()));
        r
    }
}

/// Where Windows keeps a known folder (following redirection).
#[cfg(windows)]
pub(crate) fn known_folder(k: crate::services_desktop::KnownFolder) -> Option<std::path::PathBuf> {
    use crate::services_desktop::KnownFolder as K;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures,
        FOLDERID_Profile, FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    };
    let id = match k {
        K::Home => FOLDERID_Profile,
        K::Documents => FOLDERID_Documents,
        K::Pictures => FOLDERID_Pictures,
        K::Downloads => FOLDERID_Downloads,
        K::Desktop => FOLDERID_Desktop,
        K::Videos => FOLDERID_Videos,
        K::Music => FOLDERID_Music,
    };
    // SAFETY: the returned string is read once and freed with CoTaskMemFree, as documented.
    unsafe {
        let p = SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0.cast_const().cast()));
        s.map(std::path::PathBuf::from)
    }
}

#[cfg(target_os = "android")]
pub(crate) mod android_media;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(windows)]
pub(crate) mod media;
#[cfg(windows)]
pub(crate) mod power;

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Quotes a command-line argument the way `CommandLineToArgvW` reads it back.
#[cfg(windows)]
fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        return a.to_owned();
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in a.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

/// Sets a string value (`name` `None` for the key's default) under `HKCU\<path>`, creating
/// the key.
#[cfg(windows)]
fn set_value(path: &str, name: Option<&str>, data: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegSetValueExW,
    };
    let p = wide(path);
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: creating or opening a key under the current user's hive with a NUL-terminated
    // path; the handle is closed below.
    let r = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            p.as_ptr(),
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
        return Err(format!("the registry key {path} could not be written (error {r})"));
    }
    let n = name.map(wide);
    let d = wide(data);
    let bytes = u32::try_from(d.len() * 2).unwrap_or(u32::MAX);
    // SAFETY: a NUL-terminated UTF-16 string of the stated byte length, written to the key
    // opened above (a null name is the key's default value).
    let r = unsafe {
        RegSetValueExW(
            key,
            n.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
            0,
            REG_SZ,
            d.as_ptr().cast(),
            bytes,
        )
    };
    // SAFETY: the key opened above, closed once.
    unsafe { RegCloseKey(key) };
    if r == 0 {
        Ok(())
    } else {
        Err(format!("the registry value under {path} could not be written (error {r})"))
    }
}

/// Makes `scheme` a URL protocol of the current user that runs `program` with `args` and the
/// URL (`HKCU\Software\Classes\<scheme>`).
#[cfg(windows)]
pub(crate) fn register_scheme(
    scheme: &str,
    name: &str,
    program: &Path,
    args: &[String],
) -> Result<(), String> {
    let base = format!("Software\\Classes\\{scheme}");
    let mut command = quote_arg(&program.to_string_lossy());
    for a in args {
        command.push(' ');
        command.push_str(&quote_arg(a));
    }
    command.push_str(" \"%1\"");
    set_value(&base, None, &format!("URL:{name}"))?;
    set_value(&base, Some("URL Protocol"), "")?;
    set_value(
        &format!("{base}\\DefaultIcon"),
        None,
        &format!("{},0", quote_arg(&program.to_string_lossy())),
    )?;
    set_value(&format!("{base}\\shell\\open\\command"), None, &command)
}

/// Removes the current user's URL protocol `scheme`.
#[cfg(windows)]
pub(crate) fn unregister_scheme(scheme: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW};
    let p = wide(&format!("Software\\Classes\\{scheme}"));
    // SAFETY: deleting a key under the current user's hive by a NUL-terminated path.
    let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, p.as_ptr()) };
    if r == 0 || r == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("the URL scheme could not be removed (error {r})"))
    }
}

/// The command the current user's URL protocol `scheme` runs, if there is one.
#[cfg(windows)]
pub(crate) fn scheme_command(scheme: &str) -> Option<String> {
    get_string(&format!("Software\\Classes\\{scheme}\\shell\\open\\command"), None)
}

/// A string value (`name` `None` for the key's default) under `HKCU\<path>`, if there is one.
#[cfg(windows)]
fn get_string(path: &str, name: Option<&str>) -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
    };
    let p = wide(path);
    let n = name.map(wide);
    let mut buf = vec![0u16; 2048];
    // A longer value is read again with the size Windows asks for; it can grow between two
    // reads, so a few times at most.
    for _ in 0..4 {
        let mut bytes = u32::try_from(buf.len() * 2).unwrap_or(u32::MAX);
        // SAFETY: reading a string value into a buffer of `bytes` bytes, which Windows
        // NUL-terminates; the names are NUL-terminated (a null name is the default value).
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                p.as_ptr(),
                n.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if r == ERROR_MORE_DATA {
            buf.resize(usize::try_from(bytes).ok()?.div_ceil(2) + 1, 0);
            continue;
        }
        if r != 0 {
            return None;
        }
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        return Some(String::from_utf16_lossy(buf.get(..len)?));
    }
    None
}

/// The `name` value under `HKCU\<key>` (a Run key), if there is one.
#[cfg(windows)]
pub(crate) fn run_value(key: &str, name: &str) -> Option<String> {
    get_string(key, Some(name))
}

/// Writes (`Some`) or removes (`None`) the `name` value under `HKCU\<key>`: `program` with
/// `args`, quoted as Windows reads a command line back.
#[cfg(windows)]
pub(crate) fn set_run_value(
    key: &str,
    name: &str,
    command: Option<(&Path, &[String])>,
) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyValueW};
    if let Some((program, args)) = command {
        let mut line = quote_arg(&program.to_string_lossy());
        for a in args {
            line.push(' ');
            line.push_str(&quote_arg(a));
        }
        return set_value(key, Some(name), &line);
    }
    let (k, n) = (wide(key), wide(name));
    // SAFETY: deleting one value by NUL-terminated key path and name.
    let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr()) };
    if r == 0 || r == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("the Run value {name} could not be removed (error {r})"))
    }
}

#[cfg(not(windows))]
pub(crate) fn run_value(_key: &str, _name: &str) -> Option<String> {
    None
}

#[cfg(not(windows))]
pub(crate) fn set_run_value(
    _key: &str,
    _name: &str,
    _command: Option<(&Path, &[String])>,
) -> Result<(), String> {
    Err("the Run key is Windows'".into())
}
/// Replaces the jump list's Tasks with shortcuts to `program` (`ICustomDestinationList`).
#[cfg(windows)]
pub(crate) fn set_jump_tasks(
    program: &Path,
    tasks: &[crate::services_desktop::jump::Task],
) -> Result<(), String> {
    use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{
        DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
    };
    use windows::core::{HSTRING, Interface};
    let e = |e: windows::core::Error| e.to_string();
    let dir = program.parent().map(Path::to_path_buf).unwrap_or_default();
    // SAFETY: COM on this thread (a second initialization only adds a reference); every string
    // is an HSTRING or PROPVARIANT that outlives its call.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        if tasks.is_empty() {
            return list.DeleteList(None).map_err(e);
        }
        let mut slots = 0u32;
        let _removed: IObjectArray = list.BeginList(&mut slots).map_err(e)?;
        let items: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        for t in tasks {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(e)?;
            link.SetPath(&HSTRING::from(program.as_os_str())).map_err(e)?;
            let args: Vec<String> = t.args.iter().map(|a| quote_arg(a)).collect();
            link.SetArguments(&HSTRING::from(args.join(" "))).map_err(e)?;
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str())).map_err(e)?;
            if !t.description.is_empty() {
                link.SetDescription(&HSTRING::from(t.description.as_str())).map_err(e)?;
            }
            let icon = t.icon.as_deref().unwrap_or(program);
            link.SetIconLocation(&HSTRING::from(icon.as_os_str()), 0).map_err(e)?;
            let store: IPropertyStore = link.cast().map_err(e)?;
            let title = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::from(t.label.as_str());
            store.SetValue(&PKEY_Title, &title).map_err(e)?;
            store.Commit().map_err(e)?;
            items.AddObject(&link).map_err(e)?;
        }
        let array: IObjectArray = items.cast().map_err(e)?;
        list.AddUserTasks(&array).map_err(e)?;
        list.CommitList().map_err(e)
    }
}

/// Adds `path` to the user's recent documents (and the app's jump list Recent category).
#[cfg(windows)]
pub(crate) fn add_recent(path: &Path) {
    use windows::Win32::UI::Shell::{SHARD_PATHW, SHAddToRecentDocs};
    use windows::core::HSTRING;
    let p = HSTRING::from(path.as_os_str());
    // SAFETY: SHARD_PATHW takes a NUL-terminated UTF-16 path, which `p` holds for the call.
    unsafe { SHAddToRecentDocs(SHARD_PATHW.0 as u32, Some(p.as_ptr().cast())) };
}

/// Clears the app's recent and frequent destinations (`IApplicationDestinations`).
#[cfg(windows)]
pub(crate) fn clear_recent() -> Result<(), String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::{ApplicationDestinations, IApplicationDestinations};
    // SAFETY: COM on this thread (a second initialization only adds a reference).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let d: IApplicationDestinations =
            CoCreateInstance(&ApplicationDestinations, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
        d.RemoveAllDestinations().map_err(|e| e.to_string())
    }
}
