//! `cargo remedy package` for Linux, Android and macOS.
//!
//! - **Linux**: `<bin>-<version>-linux-x86_64.tar.gz` holding the executable, an XDG desktop
//!   entry and the icon in the `hicolor` theme, laid out under a prefix (`bin/`, `share/`), so
//!   unpacking it into `~/.local` installs it for the user.
//! - **Android**: a signed APK with no Gradle: the app's library built for the target with
//!   the NDK's clang, a manifest for `NativeActivity`, `aapt2`, `zipalign` and `apksigner`
//!   (the steps of `crates/remedy-ui-bridge-android/tools/build-apk.sh`, for any app).
//! - **macOS**: `<name>.app` (Info.plist, the executable, an `.icns` icon) in a zip. It is
//!   built with Apple's linker, so it is packaged on a Mac or with a cross linker set up.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ico::Rgba;
use crate::project::{Project, cargo, status};

/// The icon from `remedy.toml`, or the default one.
pub fn icon(p: &Project, notes: &mut Vec<String>) -> Result<Rgba, String> {
    match &p.icon {
        Some(path) => {
            let bytes = std::fs::read(path).map_err(|e| format!("icon {}: {e}", path.display()))?;
            crate::ico::read_png(&bytes).map_err(|e| format!("icon {}: {e}", path.display()))
        }
        None => {
            notes.push("no icon in remedy.toml: used the default icon".into());
            Ok(crate::ico::default_icon([61, 139, 95]))
        }
    }
}

/// Builds the app's executable for `target` (`None`: this machine), returning its path.
fn build_bin(p: &Project, target: Option<&str>) -> Result<PathBuf, String> {
    let mut build = Command::new(cargo());
    build.current_dir(&p.root).args(["build", "--release", "--bin", &p.bin]);
    if let Some(t) = target {
        build.args(["--target", t]);
    }
    status(&mut build)?;
    let dir = match target {
        Some(t) => p.target_dir.join(t).join("release"),
        None => p.target_dir.join("release"),
    };
    let windows_exe = target.is_none() && cfg!(windows);
    Ok(dir.join(if windows_exe { format!("{}.exe", p.bin) } else { p.bin.clone() }))
}

/// A desktop entry (freedesktop.org) that starts the app.
pub fn desktop_entry(p: &Project) -> String {
    let clean = |s: &str| s.replace(['\n', '\r'], " ");
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={}\nIcon={}\nTerminal=false\nCategories=Utility;\nStartupWMClass={}\n",
        clean(&p.name),
        clean(&p.description),
        p.bin,
        p.id,
        p.bin
    )
}

/// One file in a tar archive: its path inside, its bytes and whether it runs.
pub struct TarEntry {
    /// The path inside the archive (forward slashes).
    pub path: String,
    /// The contents.
    pub data: Vec<u8>,
    /// An executable (mode 755) rather than a file (644).
    pub executable: bool,
}

fn octal(field: &mut [u8], value: u64) {
    let s = format!("{value:0width$o}", width = field.len().saturating_sub(1));
    for (d, b) in field.iter_mut().zip(s.bytes()) {
        *d = b;
    }
}

/// A POSIX ustar archive of `entries`, gzip-compressed.
pub fn tar_gz(entries: &[TarEntry]) -> Result<Vec<u8>, String> {
    let mut tar = Vec::new();
    for e in entries {
        if e.path.len() > 100 {
            return Err(format!("{}: the path is too long for a tar header", e.path));
        }
        let mut h = [0u8; 512];
        for (d, b) in h.iter_mut().zip(e.path.bytes()) {
            *d = b;
        }
        let field = |h: &mut [u8; 512], at: usize, len: usize, v: u64| {
            if let Some(f) = h.get_mut(at..at + len) {
                octal(f, v);
            }
        };
        field(&mut h, 100, 8, if e.executable { 0o755 } else { 0o644 });
        field(&mut h, 108, 8, 0);
        field(&mut h, 116, 8, 0);
        field(&mut h, 124, 12, e.data.len() as u64);
        field(&mut h, 136, 12, 0);
        if let Some(t) = h.get_mut(156) {
            *t = b'0';
        }
        if let Some(m) = h.get_mut(257..265) {
            m.copy_from_slice(b"ustar\x0000");
        }
        // The checksum counts its own field as spaces.
        if let Some(c) = h.get_mut(148..156) {
            c.fill(b' ');
        }
        let sum: u64 = h.iter().map(|b| u64::from(*b)).sum();
        if let Some(c) = h.get_mut(148..155) {
            octal(c, sum);
        }
        tar.extend_from_slice(&h);
        tar.extend_from_slice(&e.data);
        let pad = (512 - e.data.len() % 512) % 512;
        tar.extend(std::iter::repeat_n(0u8, pad));
    }
    tar.extend(std::iter::repeat_n(0u8, 1024));
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar).map_err(|e| e.to_string())?;
    gz.finish().map_err(|e| e.to_string())
}

/// Packages for Linux; returns what it wrote.
pub fn linux(p: &Project, dist: &Path, notes: &mut Vec<String>) -> Result<Vec<PathBuf>, String> {
    let target = if cfg!(target_os = "linux") { None } else { Some("x86_64-unknown-linux-gnu") };
    if target.is_some() {
        notes.push("built with --target x86_64-unknown-linux-gnu: this needs a Linux linker for that target (CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER)".into());
    }
    let exe = build_bin(p, target)?;
    let png = crate::ico::write_png(&icon(p, notes)?)?;
    let bytes = std::fs::read(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let base = format!("{}-{}-linux-x86_64", p.bin, p.version);
    let mut entries = vec![
        TarEntry { path: format!("{base}/bin/{}", p.bin), data: bytes, executable: true },
        TarEntry {
            path: format!("{base}/share/applications/{}.desktop", p.id),
            data: desktop_entry(p).into_bytes(),
            executable: false,
        },
        TarEntry {
            path: format!("{base}/share/icons/hicolor/256x256/apps/{}.png", p.id),
            data: png,
            executable: false,
        },
    ];
    for name in ["README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE"] {
        if let Ok(data) = std::fs::read(p.root.join(name)) {
            entries.push(TarEntry {
                path: format!("{base}/share/doc/{}/{name}", p.bin),
                data,
                executable: false,
            });
        }
    }
    let out = dist.join(format!("{base}.tar.gz"));
    std::fs::write(&out, tar_gz(&entries)?).map_err(|e| format!("{}: {e}", out.display()))?;
    notes.push(format!("unpack into ~/.local (or /usr/local) to install: tar -xzf {base}.tar.gz --strip-components=1 -C ~/.local"));
    Ok(vec![out])
}

/// The Android manifest for a `NativeActivity` app.
pub fn android_manifest(
    p: &Project,
    lib_name: &str,
    version_code: u32,
    min_api: u32,
    target_api: u32,
) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    let permissions: String = p
        .android_permissions
        .iter()
        .map(|n| format!("    <uses-permission android:name=\"{}\" />\n", esc(n)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="{id}" android:versionCode="{version_code}" android:versionName="{version}">
    <uses-sdk android:minSdkVersion="{min_api}" android:targetSdkVersion="{target_api}" />
{permissions}    <application android:label="{label}" android:hasCode="false">
        <activity android:name="android.app.NativeActivity"
            android:exported="true"
            android:configChanges="orientation|screenSize|screenLayout|smallestScreenSize|keyboard|keyboardHidden|navigation|uiMode|fontScale|locale|layoutDirection|density"
            android:windowSoftInputMode="adjustResize"
            android:theme="@android:style/Theme.DeviceDefault.NoActionBar">
            <meta-data android:name="android.app.lib_name" android:value="{lib_name}" />
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"#,
        id = esc(&p.id),
        version = esc(&p.version),
        label = esc(&p.name),
    )
}

/// An Android version code from a semantic version: 1.2.3 is 10203.
pub fn version_code(version: &str) -> u32 {
    let mut parts = version.split(['.', '-', '+']).map(|n| n.parse::<u32>().unwrap_or(0));
    let (a, b, c) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    (a * 10_000 + b.min(99) * 100 + c.min(99)).max(1)
}

fn newest(dir: &Path) -> Option<PathBuf> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
    names.sort();
    names.pop()
}

fn tool(dir: &Path, name: &str) -> PathBuf {
    for ext in [".exe", ".bat", ".cmd", ""] {
        let p = dir.join(format!("{name}{ext}"));
        if p.exists() {
            return p;
        }
    }
    dir.join(name)
}

/// Packages for Android; returns what it wrote.
pub fn android(p: &Project, dist: &Path, notes: &mut Vec<String>) -> Result<Vec<PathBuf>, String> {
    let sdk = PathBuf::from(std::env::var_os("ANDROID_HOME").ok_or("set ANDROID_HOME to the Android SDK")?);
    let ndk = match std::env::var_os("ANDROID_NDK_HOME") {
        Some(n) => PathBuf::from(n),
        None => newest(&sdk.join("ndk")).ok_or("no NDK under $ANDROID_HOME/ndk (or set ANDROID_NDK_HOME)")?,
    };
    let target = p.android_target.clone();
    let abi = match target.as_str() {
        "aarch64-linux-android" => "arm64-v8a",
        "x86_64-linux-android" => "x86_64",
        other => {
            return Err(format!(
                "[android] target `{other}`: use aarch64-linux-android or x86_64-linux-android"
            ));
        }
    };
    let (min_api, target_api) = (p.android_min_api, p.android_target_api);
    let prebuilt = newest(&ndk.join("toolchains/llvm/prebuilt")).ok_or("the NDK has no LLVM toolchain")?;
    let bin = prebuilt.join("bin");
    let clang = tool(&bin, &format!("{target}{min_api}-clang"));
    let clangxx = tool(&bin, &format!("{target}{min_api}-clang++"));
    let upper = target.to_uppercase().replace('-', "_");
    let lower = target.replace('-', "_");
    let mut build = Command::new(cargo());
    build
        .current_dir(&p.root)
        .args(["build", "--release", "--lib", "--target", &target])
        .env(format!("CARGO_TARGET_{upper}_LINKER"), &clang)
        .env(format!("CC_{lower}"), &clang)
        .env(format!("CXX_{lower}"), &clangxx)
        .env(format!("AR_{lower}"), tool(&bin, "llvm-ar"));
    status(&mut build)?;
    let lib_name = p.lib_name.clone();
    let so = p.target_dir.join(&target).join("release").join(format!("lib{lib_name}.so"));
    if !so.exists() {
        return Err(format!(
            "{} was not built: the app's [lib] needs crate-type = [\"cdylib\", \"rlib\"] (the android template has it)",
            so.display()
        ));
    }
    let work = p.target_dir.join("remedy").join("android");
    let stage = work.join("stage");
    let _ = std::fs::remove_dir_all(&stage);
    let lib_dir = stage.join("lib").join(abi);
    std::fs::create_dir_all(&lib_dir).map_err(|e| e.to_string())?;
    let staged_so = lib_dir.join(format!("lib{lib_name}.so"));
    std::fs::copy(&so, &staged_so).map_err(|e| format!("{}: {e}", so.display()))?;
    status(Command::new(tool(&bin, "llvm-strip")).arg("--strip-debug").arg(&staged_so))?;
    let manifest = stage.join("AndroidManifest.xml");
    std::fs::write(&manifest, android_manifest(p, &lib_name, version_code(&p.version), min_api, target_api))
        .map_err(|e| e.to_string())?;
    let build_tools = newest(&sdk.join("build-tools")).ok_or("no build-tools under $ANDROID_HOME")?;
    let jar = sdk.join("platforms").join(format!("android-{target_api}")).join("android.jar");
    if !jar.exists() {
        return Err(format!("{} is missing: install the Android {target_api} platform", jar.display()));
    }
    let base = stage.join("base.apk");
    status(
        Command::new(tool(&build_tools, "aapt2"))
            .arg("link")
            .arg("-o")
            .arg(&base)
            .arg("-I")
            .arg(&jar)
            .arg("--manifest")
            .arg(&manifest),
    )?;
    status(
        Command::new(tool(&build_tools, "aapt"))
            .current_dir(&stage)
            .args(["add", "-f", "base.apk"])
            .arg(format!("lib/{abi}/lib{lib_name}.so")),
    )?;
    let aligned = stage.join("aligned.apk");
    status(Command::new(tool(&build_tools, "zipalign")).args(["-f", "-p", "4"]).arg(&base).arg(&aligned))?;
    // The app's own release key when remedy.toml names one, else a debug key of this build's.
    let (keystore, pass, alias) = match &p.android_keystore {
        Some(ks) => {
            let pass = std::env::var("REMEDY_ANDROID_KEYSTORE_PASSWORD")
                .map_err(|_| "set REMEDY_ANDROID_KEYSTORE_PASSWORD for [android] keystore")?;
            (ks.clone(), pass, p.android_key_alias.clone().unwrap_or_else(|| "release".into()))
        }
        None => {
            let ks = work.join("debug.keystore");
            if !ks.exists() {
                status(Command::new("keytool").args(["-genkeypair", "-keystore"]).arg(&ks).args([
                    "-storepass",
                    "android",
                    "-keypass",
                    "android",
                    "-alias",
                    "debug",
                    "-keyalg",
                    "RSA",
                    "-keysize",
                    "2048",
                    "-validity",
                    "10000",
                    "-dname",
                    "CN=RemedyUI debug",
                ]))?;
            }
            notes.push(
                "signed with a debug key (set [android] keystore in remedy.toml to sign for release)".into(),
            );
            (ks, "android".to_owned(), "debug".to_owned())
        }
    };
    let apk = dist.join(format!("{}_{}_{abi}.apk", p.bin, p.version));
    status(
        Command::new(tool(&build_tools, "apksigner"))
            .args(["sign", "--ks"])
            .arg(&keystore)
            .arg("--ks-key-alias")
            .arg(&alias)
            .arg("--ks-pass")
            .arg(format!("pass:{pass}"))
            .arg("--out")
            .arg(&apk)
            .arg(&aligned),
    )?;
    let _ = std::fs::remove_dir_all(&stage);
    Ok(vec![apk])
}

/// An Info.plist for the app bundle.
pub fn info_plist(p: &Project) -> String {
    let x = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>{name}</string>
  <key>CFBundleDisplayName</key><string>{name}</string>
  <key>CFBundleIdentifier</key><string>{id}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleExecutable</key><string>{bin}</string>
  <key>CFBundleIconFile</key><string>app.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict>
</plist>
"#,
        name = x(&p.name),
        id = x(&p.id),
        version = x(&p.version),
        bin = x(&p.bin),
    )
}

/// An `.icns` icon holding `png` (256 px or larger) as its 256 px image.
pub fn icns(png: &[u8]) -> Vec<u8> {
    let entry_len = 8 + png.len();
    let total = 8 + entry_len;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(b"ic08");
    out.extend_from_slice(&(entry_len as u32).to_be_bytes());
    out.extend_from_slice(png);
    out
}

/// Packages for macOS; returns what it wrote.
pub fn macos(p: &Project, dist: &Path, notes: &mut Vec<String>) -> Result<Vec<PathBuf>, String> {
    let target = if cfg!(target_os = "macos") { None } else { Some("aarch64-apple-darwin") };
    if target.is_some() {
        notes.push("built with --target aarch64-apple-darwin: this needs Apple's linker (a Mac, or a cross linker set as CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER)".into());
    }
    let exe = build_bin(p, target)?;
    let png = crate::ico::write_png(&icon(p, notes)?)?;
    let app = format!("{}.app", p.name);
    let file = std::fs::File::create(dist.join(format!("{}_{}_macos.zip", p.bin, p.version)))
        .map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let put = |zip: &mut zip::ZipWriter<std::fs::File>,
               path: String,
               data: &[u8],
               mode: u32|
     -> Result<(), String> {
        zip.start_file(path, opts.unix_permissions(mode)).map_err(|e| e.to_string())?;
        zip.write_all(data).map_err(|e| e.to_string())
    };
    put(&mut zip, format!("{app}/Contents/Info.plist"), info_plist(p).as_bytes(), 0o644)?;
    let bytes = std::fs::read(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    put(&mut zip, format!("{app}/Contents/MacOS/{}", p.bin), &bytes, 0o755)?;
    put(&mut zip, format!("{app}/Contents/Resources/app.icns"), &icns(&png), 0o644)?;
    zip.finish().map_err(|e| e.to_string())?;
    notes.push("unsigned: sign and notarize on a Mac (codesign, notarytool) before handing it out".into());
    Ok(vec![dist.join(format!("{}_{}_macos.zip", p.bin, p.version))])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        Project {
            root: PathBuf::from("."),
            bin: "my-notes".into(),
            lib_name: "my_notes".into(),
            version: "1.2.3".into(),
            target_dir: PathBuf::from("target"),
            name: "My Notes".into(),
            id: "app.remedy.my-notes".into(),
            publisher: "Me".into(),
            description: "Notes & more".into(),
            icon: None,
            sign_thumbprint: None,
            timestamp_url: String::new(),
            android_target: "aarch64-linux-android".into(),
            android_min_api: 26,
            android_target_api: 35,
            android_keystore: None,
            android_key_alias: None,
            android_permissions: Vec::new(),
            web_short_name: None,
            web_theme: crate::project::DEFAULT_THEME_COLOR.into(),
            web_icon: None,
            uses_remedy_ui: true,
        }
    }

    #[test]
    fn linux_archives_are_tar_gz_with_a_desktop_entry() {
        let p = project();
        let entry = desktop_entry(&p);
        assert!(entry.contains("Exec=my-notes") && entry.contains("Icon=app.remedy.my-notes"), "{entry}");
        let Ok(gz) =
            tar_gz(&[TarEntry { path: "a/bin/x".into(), data: b"hello".to_vec(), executable: true }])
        else {
            unreachable!("small archives write")
        };
        let mut tar = Vec::new();
        let read = std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(gz.as_slice()), &mut tar);
        assert!(read.is_ok());
        assert_eq!(tar.get(..7), Some(&b"a/bin/x"[..]));
        assert_eq!(tar.get(257..262), Some(&b"ustar"[..]));
        assert_eq!(tar.get(512..517), Some(&b"hello"[..]));
        assert_eq!(tar.len() % 512, 0);
    }

    #[test]
    fn android_and_macos_metadata_carry_the_app() {
        let p = project();
        let m = android_manifest(&p, "my_notes", version_code(&p.version), 26, 35);
        assert!(m.contains(r#"package="app.remedy.my-notes""#) && m.contains(r#"android:value="my_notes""#));
        assert!(m.contains(r#"android:versionCode="10203""#));
        assert!(!m.contains("uses-permission"), "nothing is declared unasked");
        let mut asks = project();
        asks.android_permissions = vec!["android.permission.RECORD_AUDIO".into()];
        let m = android_manifest(&asks, "my_notes", 100, 26, 35);
        assert!(m.contains(r#"<uses-permission android:name="android.permission.RECORD_AUDIO" />"#), "{m}");
        assert_eq!(version_code("0.1.0"), 100);
        let plist = info_plist(&p);
        assert!(plist.contains("<string>My Notes</string>") && plist.contains("<string>my-notes</string>"));
        let i = icns(b"PNG");
        assert_eq!(i.get(..4), Some(&b"icns"[..]));
        assert_eq!(i.len(), 19);
    }
}
