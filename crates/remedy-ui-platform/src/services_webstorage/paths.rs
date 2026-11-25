//! Where web views keep their localStorage by default, and the origins web-view frameworks
//! serve apps from. These only compute paths: nothing is opened, and an app can pass any
//! other folder to [`crate::services_webstorage::ChromiumStorage::read_dir`] instead.

use std::path::{Path, PathBuf};

/// The origins a Tauri app's page has on Windows: `http://tauri.localhost` (Tauri 2's
/// default), then `https://tauri.localhost` (Tauri 2 with `useHttpsScheme`, and Tauri 1).
pub const TAURI_WINDOWS_ORIGINS: [&str; 2] = ["http://tauri.localhost", "https://tauri.localhost"];

/// The origins a Capacitor app's page has on Android: `https://localhost` (the default,
/// `androidScheme: "https"`), then `http://localhost` (`androidScheme: "http"`, Capacitor 3
/// and earlier).
pub const CAPACITOR_ORIGINS: [&str; 2] = ["https://localhost", "http://localhost"];

/// The store folder below a WebView2 or Android WebView profile folder.
fn below_profile(profile: PathBuf) -> PathBuf {
    profile.join("Default").join("Local Storage").join("leveldb")
}

/// Whether `id` can name one folder: not empty, not `.` or `..`, no separators, and no colon
/// (on Windows `C:` names a drive, and joining it leaves the folder it is joined to).
fn one_folder(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', '\0', ':'])
}

/// The localStorage folder of a Tauri app on Windows, by its bundle identifier
/// (`identifier` in `tauri.conf.json`, such as `com.remedy.craftcalc`):
/// `%LOCALAPPDATA%\<identifier>\EBWebView\Default\Local Storage\leveldb`.
///
/// `None` when `LOCALAPPDATA` is not set (as on other systems) or the identifier cannot name
/// a folder. See [`tauri_windows_dir_in`] to start from another folder.
pub fn tauri_windows_dir(bundle_identifier: &str) -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty())?;
    tauri_windows_dir_in(Path::new(&local), bundle_identifier)
}

/// [`tauri_windows_dir`] below `local_app_data` instead of `%LOCALAPPDATA%`. `None` when the
/// identifier cannot name a folder.
pub fn tauri_windows_dir_in(local_app_data: impl AsRef<Path>, bundle_identifier: &str) -> Option<PathBuf> {
    one_folder(bundle_identifier)
        .then(|| below_profile(local_app_data.as_ref().join(bundle_identifier).join("EBWebView")))
}

/// The localStorage folder of the Android System WebView in an app's data folder (the parent
/// of the app's internal files folder, `/data/user/0/<package>`), as a Capacitor or other
/// web-view app of the same package left it: `<root>/app_webview/Default/Local
/// Storage/leveldb`.
pub fn android_webview_dir(app_data_root: impl AsRef<Path>) -> PathBuf {
    below_profile(app_data_root.as_ref().join("app_webview"))
}

/// [`android_webview_dir`] from the app's internal files folder (Android's
/// `Context.getFilesDir()`, `/data/user/0/<package>/files`), whose parent is the app data
/// folder. `None` when the path has no parent.
pub fn android_webview_dir_from_files_dir(files_dir: impl AsRef<Path>) -> Option<PathBuf> {
    files_dir.as_ref().parent().filter(|p| !p.as_os_str().is_empty()).map(android_webview_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tauri_folder() {
        let dir = tauri_windows_dir_in("C:/Users/u/AppData/Local", "com.remedy.craftcalc");
        let want: PathBuf = [
            "C:/Users/u/AppData/Local",
            "com.remedy.craftcalc",
            "EBWebView",
            "Default",
            "Local Storage",
            "leveldb",
        ]
        .iter()
        .collect();
        assert_eq!(dir, Some(want));
        for bad in ["", ".", "..", "a/b", "a\\b", "D:", "D:x"] {
            assert_eq!(tauri_windows_dir_in("C:/x", bad), None, "{bad:?}");
        }
    }

    #[test]
    fn android_folders() {
        let want: PathBuf =
            ["/data/user/0/com.remedy.app", "app_webview", "Default", "Local Storage", "leveldb"]
                .iter()
                .collect();
        assert_eq!(android_webview_dir("/data/user/0/com.remedy.app"), want);
        assert_eq!(android_webview_dir_from_files_dir("/data/user/0/com.remedy.app/files"), Some(want));
        assert_eq!(android_webview_dir_from_files_dir("files"), None);
    }
}
