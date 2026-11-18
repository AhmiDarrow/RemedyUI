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
    let quoted: Vec<String> =
        args.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect();
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
pub(crate) fn known_folder(k: crate::KnownFolder) -> Option<std::path::PathBuf> {
    use crate::KnownFolder as K;
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
