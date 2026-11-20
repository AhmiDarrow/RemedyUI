//! Build-script help for RemedyUI apps.
//!
//! [`WindowsResources`] puts the app's icon and version information into its Windows
//! executable, so Explorer, the taskbar, Task Manager and shortcuts show the app's own picture
//! and name. The build script writes the compiled resource file itself (no `rc` or `llvm-rc`
//! is needed) and hands it to the linker:
//!
//! ```no_run
//! // In build.rs's `main`:
//! remedy_ui_build::WindowsResources::new()
//!     .icon("../branding/app.ico")
//!     .product_name("Notes")
//!     .compile()
//!     .unwrap_or_else(|e| println!("cargo::warning={e}"));
//! ```
//!
//! With `remedy-ui-build` under `[build-dependencies]`. On other targets `compile` does
//! nothing. The resource file is linked into the package's executables (not its libraries),
//! for the MSVC toolchain, whose linker takes resource files; on the GNU toolchain it warns
//! and leaves the executable without them. `cargo remedy package` compiles the same resources
//! from `remedy.toml` and sets `REMEDY_PACKAGE_RESOURCES` while it builds, and `compile` then
//! stands aside, so the executable never gets them twice.

use std::path::{Path, PathBuf};

/// Windows resource types.
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
/// MOVEABLE | PURE | DISCARDABLE, as resource compilers mark icons and version information.
const FLAGS: u16 = 0x1030;
/// US English, the language the version strings are tagged with.
const LANGUAGE: u16 = 0x0409;
/// UTF-16, the code page of the version strings.
const CODE_PAGE: u16 = 1200;

/// The environment variable `cargo remedy package` sets while it builds the executable with
/// its own resources.
pub const PACKAGE_RESOURCES_ENV: &str = "REMEDY_PACKAGE_RESOURCES";

/// A Windows executable's icon and version information.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowsResources {
    icon: Option<PathBuf>,
    icon_bytes: Option<Vec<u8>>,
    version: String,
    strings: Vec<(String, String)>,
}

impl WindowsResources {
    /// Version information from the package Cargo is building: `CARGO_PKG_VERSION` as the
    /// file and product version, `CARGO_PKG_DESCRIPTION` (else the package name) as the
    /// description Task Manager shows, the package name as the product, and the executable
    /// name (`CARGO_PKG_NAME`, or the `[[bin]]` name when set with
    /// [`WindowsResources::executable`]).
    pub fn new() -> Self {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        let name = var("CARGO_PKG_NAME");
        let description = var("CARGO_PKG_DESCRIPTION");
        WindowsResources::empty(&var("CARGO_PKG_VERSION"))
            .file_description(if description.is_empty() { &name } else { &description })
            .product_name(&name)
            .executable(&name)
    }

    /// No strings yet, version `version` (`1.2.3`, a pre-release or build suffix is kept in
    /// the text and left out of the numbers).
    pub fn empty(version: &str) -> Self {
        let s = WindowsResources { version: version.to_owned(), ..WindowsResources::default() };
        s.set("FileVersion", version).set("ProductVersion", version)
    }

    /// The `.ico` file to show for the executable (every size it holds, as drawn).
    #[must_use]
    pub fn icon(mut self, path: impl AsRef<Path>) -> Self {
        self.icon = Some(path.as_ref().to_path_buf());
        self
    }

    /// An `.ico` file's contents, instead of a path.
    #[must_use]
    pub fn icon_bytes(mut self, ico: Vec<u8>) -> Self {
        self.icon_bytes = Some(ico);
        self
    }

    /// The name people know the app by (`ProductName`).
    #[must_use]
    pub fn product_name(self, name: &str) -> Self {
        self.set("ProductName", name)
    }

    /// What the executable is (`FileDescription`: Task Manager and the "Open with" list show
    /// it).
    #[must_use]
    pub fn file_description(self, text: &str) -> Self {
        self.set("FileDescription", text)
    }

    /// Who makes it (`CompanyName`).
    #[must_use]
    pub fn company(self, name: &str) -> Self {
        self.set("CompanyName", name)
    }

    /// The copyright line (`LegalCopyright`).
    #[must_use]
    pub fn copyright(self, text: &str) -> Self {
        self.set("LegalCopyright", text)
    }

    /// The executable's name without `.exe` (`InternalName`, and `OriginalFilename` with it).
    #[must_use]
    pub fn executable(self, name: &str) -> Self {
        self.set("InternalName", name).set("OriginalFilename", &format!("{name}.exe"))
    }

    /// Any version string by its key; an empty value removes it.
    #[must_use]
    pub fn set(mut self, key: &str, value: &str) -> Self {
        self.strings.retain(|(k, _)| k != key);
        if !value.is_empty() {
            self.strings.push((key.to_owned(), value.to_owned()));
        }
        self
    }

    /// From a build script: when Cargo builds for Windows with the MSVC toolchain, writes the
    /// resource file to `OUT_DIR` and links it into the package's executables. Elsewhere it
    /// does nothing (with a warning on the GNU toolchain, whose linker takes no resource
    /// files). Also tells Cargo to run the build script again when the icon changes.
    pub fn compile(self) -> Result<(), String> {
        println!("cargo::rerun-if-env-changed={PACKAGE_RESOURCES_ENV}");
        if let Some(icon) = &self.icon {
            println!("cargo::rerun-if-changed={}", icon.display());
        }
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        if var("CARGO_CFG_TARGET_OS") != "windows" || std::env::var_os(PACKAGE_RESOURCES_ENV).is_some() {
            return Ok(());
        }
        if var("CARGO_CFG_TARGET_ENV") != "msvc" {
            println!(
                "cargo::warning=the executable has no icon or version information: the GNU linker takes no resource files"
            );
            return Ok(());
        }
        let out = PathBuf::from(
            std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set: call compile from build.rs")?,
        );
        let res = out.join("remedy-resources.res");
        std::fs::write(&res, self.to_res()?).map_err(|e| format!("{}: {e}", res.display()))?;
        println!("cargo::rustc-link-arg-bins={}", res.display());
        Ok(())
    }

    /// The compiled resource file (`.res`): the icon's images and group, and the version
    /// information.
    pub fn to_res(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        // A `.res` file starts with an empty entry.
        entry(&mut out, 0, 0, 0, &[]);
        let ico = match (&self.icon_bytes, &self.icon) {
            (Some(b), _) => Some(b.clone()),
            (None, Some(p)) => Some(std::fs::read(p).map_err(|e| format!("icon {}: {e}", p.display()))?),
            (None, None) => None,
        };
        if let Some(ico) = ico {
            let images = read_ico(&ico)?;
            let mut group = Vec::new();
            group.extend(0u16.to_le_bytes());
            group.extend(1u16.to_le_bytes());
            group.extend(u16::try_from(images.len()).map_err(|_| "too many icon images")?.to_le_bytes());
            for (n, img) in images.iter().enumerate() {
                let id = u16::try_from(n + 1).map_err(|_| "too many icon images")?;
                entry(&mut out, RT_ICON, id, FLAGS, &img.data);
                group.extend([img.width, img.height, img.colors, 0]);
                group.extend(img.planes.to_le_bytes());
                group.extend(img.bits.to_le_bytes());
                group
                    .extend(u32::try_from(img.data.len()).map_err(|_| "icon image too large")?.to_le_bytes());
                group.extend(id.to_le_bytes());
            }
            entry(&mut out, RT_GROUP_ICON, 1, FLAGS, &group);
        }
        entry(&mut out, RT_VERSION, 1, FLAGS, &self.version_info());
        Ok(out)
    }

    /// `VS_VERSIONINFO`: the fixed numbers, the strings, and their language.
    fn version_info(&self) -> Vec<u8> {
        let [a, b, c, d] = version_numbers(&self.version);
        let mut fixed = Vec::with_capacity(52);
        for v in [
            0xFEEF_04BD,                         // signature
            0x0001_0000,                         // structure version
            (u32::from(a) << 16) | u32::from(b), // file version
            (u32::from(c) << 16) | u32::from(d),
            (u32::from(a) << 16) | u32::from(b), // product version
            (u32::from(c) << 16) | u32::from(d),
            0x3F,        // flags mask
            0,           // flags
            0x0004_0004, // VOS_NT_WINDOWS32
            1,           // VFT_APP
            0,           // subtype
            0,           // date
            0,
        ] {
            fixed.extend(u32::to_le_bytes(v));
        }
        let strings: Vec<Vec<u8>> = self
            .strings
            .iter()
            .map(|(k, v)| {
                let value = utf16z(v);
                node(k, &value, (value.len() / 2) as u16, 1, &[])
            })
            .collect();
        let table = node(&format!("{LANGUAGE:04x}{CODE_PAGE:04x}"), &[], 0, 1, &strings);
        let string_info = node("StringFileInfo", &[], 0, 1, &[table]);
        let mut translation = Vec::new();
        translation.extend(LANGUAGE.to_le_bytes());
        translation.extend(CODE_PAGE.to_le_bytes());
        let var = node("Translation", &translation, 4, 0, &[]);
        let var_info = node("VarFileInfo", &[], 0, 1, &[var]);
        node("VS_VERSION_INFO", &fixed, 52, 0, &[string_info, var_info])
    }
}

/// The four version numbers of `1.2.3[-pre][+build]` (missing ones are 0; each at most 65535).
fn version_numbers(v: &str) -> [u16; 4] {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let mut out = [0u16; 4];
    for (slot, part) in out.iter_mut().zip(core.split('.')) {
        *slot = part.trim().parse().unwrap_or(0);
    }
    out
}

/// UTF-16 with a terminating zero, as bytes.
fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// Pads to a multiple of four bytes.
fn pad4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

/// One node of the version information tree: length, value length (`value_len`, in bytes for
/// binary values and in UTF-16 units for text), type (1 text, 0 binary), key, value, children,
/// each part starting on a four-byte boundary.
fn node(key: &str, value: &[u8], value_len: u16, kind: u16, children: &[Vec<u8>]) -> Vec<u8> {
    let mut v = vec![0, 0];
    v.extend(value_len.to_le_bytes());
    v.extend(kind.to_le_bytes());
    v.extend(utf16z(key));
    pad4(&mut v);
    v.extend(value);
    for child in children {
        pad4(&mut v);
        v.extend(child);
    }
    let len = u16::try_from(v.len()).unwrap_or(u16::MAX).to_le_bytes();
    if let Some(head) = v.get_mut(..2) {
        head.copy_from_slice(&len);
    }
    v
}

/// One `.res` entry: its header (numbered type and name) and its data, padded to four bytes.
fn entry(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    out.extend(u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend(32u32.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(kind.to_le_bytes());
    out.extend([0xFF, 0xFF]);
    out.extend(name.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // data version
    out.extend(flags.to_le_bytes());
    out.extend((if kind == 0 { 0 } else { LANGUAGE }).to_le_bytes());
    out.extend(0u32.to_le_bytes()); // version
    out.extend(0u32.to_le_bytes()); // characteristics
    out.extend(data);
    pad4(out);
}

/// One image of an `.ico` file.
struct IcoImage {
    width: u8,
    height: u8,
    colors: u8,
    planes: u16,
    bits: u16,
    data: Vec<u8>,
}

/// The images of an `.ico` file (PNG or bitmap, as stored).
fn read_ico(ico: &[u8]) -> Result<Vec<IcoImage>, String> {
    let u16_at = |at: usize| ico.get(at..at + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
    let u32_at = |at: usize| ico.get(at..at + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    if u16_at(0) != Some(0) || u16_at(2) != Some(1) {
        return Err("the icon is not an .ico file".into());
    }
    let count = usize::from(u16_at(4).unwrap_or(0));
    if count == 0 {
        return Err("the .ico file holds no images".into());
    }
    let mut images = Vec::with_capacity(count);
    for i in 0..count {
        let at = 6 + 16 * i;
        let head = ico.get(at..at + 16).ok_or("the .ico file is cut short")?;
        let (len, offset) = (u32_at(at + 8).unwrap_or(0) as usize, u32_at(at + 12).unwrap_or(0) as usize);
        let data =
            ico.get(offset..offset.saturating_add(len)).ok_or("an .ico image lies outside the file")?;
        let (planes, bits) = (u16_at(at + 4).unwrap_or(0), u16_at(at + 6).unwrap_or(0));
        images.push(IcoImage {
            width: head.first().copied().unwrap_or(0),
            height: head.get(1).copied().unwrap_or(0),
            colors: head.get(2).copied().unwrap_or(0),
            // Some writers leave these 0 for PNG images; Windows picks images by them.
            planes: planes.max(1),
            bits: if bits == 0 { 32 } else { bits },
            data: data.to_vec(),
        });
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `.ico` holding two images (opaque bytes standing in for PNGs).
    fn sample_ico() -> Vec<u8> {
        let imgs: [(u8, &[u8]); 2] = [(16, b"sixteen!"), (0, b"two-five-six")];
        let mut v = vec![0, 0, 1, 0, 2, 0];
        let mut offset = 6 + 16 * 2;
        for (size, data) in imgs {
            v.extend([size, size, 0, 0, 0, 0, 0, 0]);
            v.extend((data.len() as u32).to_le_bytes());
            v.extend((offset as u32).to_le_bytes());
            offset += data.len();
        }
        for (_, data) in imgs {
            v.extend(data);
        }
        v
    }

    /// The `.res` entries: (type, name, data).
    fn entries(res: &[u8]) -> Vec<(u16, u16, Vec<u8>)> {
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(head) = res.get(at..at + 32) {
            let size = u32::from_le_bytes(head.get(..4).and_then(|b| b.try_into().ok()).unwrap_or_default())
                as usize;
            let word = |i: usize| {
                u16::from_le_bytes(head.get(i..i + 2).and_then(|b| b.try_into().ok()).unwrap_or_default())
            };
            out.push((word(10), word(14), res.get(at + 32..at + 32 + size).unwrap_or_default().to_vec()));
            at += 32 + size.next_multiple_of(4);
        }
        out
    }

    #[test]
    fn resources_hold_the_icon_group_and_the_version() {
        let res = WindowsResources::empty("1.2.3-beta.1")
            .icon_bytes(sample_ico())
            .product_name("Notes")
            .file_description("Notes, the app")
            .executable("notes")
            .to_res()
            .unwrap_or_default();
        let e = entries(&res);
        let kinds: Vec<(u16, u16)> = e.iter().map(|(k, n, _)| (*k, *n)).collect();
        assert_eq!(kinds, [(0, 0), (RT_ICON, 1), (RT_ICON, 2), (RT_GROUP_ICON, 1), (RT_VERSION, 1)]);
        assert_eq!(e.get(1).map(|x| x.2.as_slice()), Some(&b"sixteen!"[..]), "images as stored");
        // The group: 2 entries; the second is 256 px (0), 32-bit, 12 bytes, image 2.
        let group = e.get(3).map(|x| x.2.clone()).unwrap_or_default();
        assert_eq!(group.get(..6), Some(&[0, 0, 1, 0, 2, 0][..]));
        assert_eq!(group.get(20..34), Some(&[0, 0, 0, 0, 1, 0, 32, 0, 12, 0, 0, 0, 2, 0][..]));
        let version = e.get(4).map(|x| x.2.clone()).unwrap_or_default();
        let len = version.get(..2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes);
        assert_eq!(len.map(usize::from), Some(version.len()), "the length covers it all");
        let units: Vec<u16> = version
            .chunks(2)
            .map(|c| u16::from_le_bytes([c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(0)]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        for s in [
            "VS_VERSION_INFO",
            "040904b0",
            "ProductName",
            "Notes, the app",
            "notes.exe",
            "1.2.3-beta.1",
            "Translation",
        ] {
            assert!(text.contains(s), "{s} in the version information");
        }
        // The fixed numbers: 1.2.3.0, after the 40-byte header with its key.
        let ms = u32::from_le_bytes(version.get(48..52).and_then(|b| b.try_into().ok()).unwrap_or_default());
        assert_eq!(ms, 0x0001_0002);
    }

    #[test]
    fn bad_icons_and_versions_are_handled() {
        assert!(WindowsResources::empty("1").icon_bytes(b"nope".to_vec()).to_res().is_err());
        assert!(WindowsResources::empty("1").icon_bytes(vec![0, 0, 1, 0, 0, 0]).to_res().is_err());
        assert_eq!(version_numbers("2.10"), [2, 10, 0, 0]);
        assert_eq!(version_numbers("1.2.3.4+build"), [1, 2, 3, 4]);
        assert_eq!(version_numbers("x"), [0, 0, 0, 0]);
        let none = WindowsResources::empty("1.0.0").to_res().unwrap_or_default();
        assert_eq!(
            entries(&none).iter().map(|(k, _, _)| *k).collect::<Vec<_>>(),
            [0, RT_VERSION],
            "no icon, no icon entries"
        );
    }
}
