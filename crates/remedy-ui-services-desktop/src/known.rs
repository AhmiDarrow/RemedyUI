//! The user's usual folders where the system keeps them: Windows' known folders (which may
//! be redirected, to OneDrive for one), the XDG user directories on Linux, the home folders
//! on macOS.

use std::path::{Path, PathBuf};

/// A folder every desktop has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KnownFolder {
    /// The home folder.
    Home,
    /// Documents.
    Documents,
    /// Pictures.
    Pictures,
    /// Downloads.
    Downloads,
    /// The desktop.
    Desktop,
    /// Videos (Movies on macOS).
    Videos,
    /// Music.
    Music,
}

impl KnownFolder {
    /// The user's libraries, in the order file managers list them.
    pub const LIBRARIES: [KnownFolder; 6] = [
        KnownFolder::Documents,
        KnownFolder::Pictures,
        KnownFolder::Downloads,
        KnownFolder::Desktop,
        KnownFolder::Videos,
        KnownFolder::Music,
    ];

    /// Its English name, as file managers show it.
    pub fn name(self) -> &'static str {
        match self {
            KnownFolder::Home => "Home",
            KnownFolder::Documents => "Documents",
            KnownFolder::Pictures => "Pictures",
            KnownFolder::Downloads => "Downloads",
            KnownFolder::Desktop => "Desktop",
            KnownFolder::Videos => "Videos",
            KnownFolder::Music => "Music",
        }
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// Where `k` is, when it exists.
pub fn known_folder(k: KnownFolder) -> Option<PathBuf> {
    platform(k).filter(|p| p.is_dir())
}

#[cfg(windows)]
fn platform(k: KnownFolder) -> Option<PathBuf> {
    crate::os::known_folder(k).or_else(|| (k == KnownFolder::Home).then(home).flatten())
}

#[cfg(target_os = "macos")]
fn platform(k: KnownFolder) -> Option<PathBuf> {
    let h = home()?;
    Some(match k {
        KnownFolder::Home => h,
        KnownFolder::Videos => h.join("Movies"),
        other => h.join(other.name()),
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform(k: KnownFolder) -> Option<PathBuf> {
    let h = home()?;
    if k == KnownFolder::Home {
        return Some(h);
    }
    let config = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| h.join(".config"), PathBuf::from);
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    Some(xdg_user_dir(&text, k, &h).unwrap_or_else(|| h.join(k.name())))
}

/// `k`'s line in an XDG `user-dirs.dirs` file (`XDG_DOCUMENTS_DIR="$HOME/Documents"`).
pub fn xdg_user_dir(text: &str, k: KnownFolder, home: &Path) -> Option<PathBuf> {
    let key = match k {
        KnownFolder::Home => return Some(home.to_path_buf()),
        KnownFolder::Documents => "XDG_DOCUMENTS_DIR",
        KnownFolder::Pictures => "XDG_PICTURES_DIR",
        KnownFolder::Downloads => "XDG_DOWNLOAD_DIR",
        KnownFolder::Desktop => "XDG_DESKTOP_DIR",
        KnownFolder::Videos => "XDG_VIDEOS_DIR",
        KnownFolder::Music => "XDG_MUSIC_DIR",
    };
    text.lines().find_map(|line| {
        let (k2, v) = line.trim().split_once('=')?;
        if k2.trim() != key {
            return None;
        }
        let v = v.trim().trim_matches('"');
        let p = match v.strip_prefix("$HOME") {
            Some(rest) => home.join(rest.trim_start_matches('/')),
            None if v.starts_with('/') => PathBuf::from(v),
            None => return None,
        };
        // A user directory set to the home folder itself means "none".
        (p != home).then_some(p)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{KnownFolder, xdg_user_dir};

    #[test]
    fn xdg_user_dirs_are_read_like_xdg_user_dir_reads_them() {
        let text = "# written by xdg-user-dirs-update\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\nXDG_MUSIC_DIR=\"/srv/music\"\nXDG_VIDEOS_DIR=\"$HOME/\"\n";
        let home = Path::new("/home/sam");
        assert_eq!(xdg_user_dir(text, KnownFolder::Downloads, home), Some(home.join("Téléchargements")));
        assert_eq!(xdg_user_dir(text, KnownFolder::Music, home), Some("/srv/music".into()));
        assert_eq!(xdg_user_dir(text, KnownFolder::Videos, home), None, "the home folder itself means none");
        assert_eq!(xdg_user_dir(text, KnownFolder::Pictures, home), None);
    }

    #[test]
    fn the_libraries_found_here_exist() {
        for k in KnownFolder::LIBRARIES {
            if let Some(p) = super::known_folder(k) {
                assert!(p.is_dir(), "{k:?} at {}", p.display());
            }
        }
        assert!(super::known_folder(KnownFolder::Home).is_some(), "every user has a home folder");
    }
}
