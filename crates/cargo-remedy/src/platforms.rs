//! `cargo remedy package` for Linux, Android and macOS.
//!
//! - **Linux**: `<bin>-<version>-linux-x86_64.tar.gz` holding the executable, an XDG desktop
//!   entry and the icon in the `hicolor` theme, laid out under a prefix (`bin/`, `share/`), so
//!   unpacking it into `~/.local` installs it for the user.
//! - **Android**: a signed APK with no Gradle: the app's library built for the target with
//!   the NDK's clang, the Android bridge's Java (its `dex/classes.dex`: the activity, the
//!   notification classes and the foreground service), the app's icon, a manifest, `aapt2`,
//!   `zipalign` and `apksigner` (the steps of `crates/remedy-ui-bridge-android/tools/build-apk.sh`,
//!   for any app).
//! - **macOS**: `<name>.app` (Info.plist, the executable, an `.icns` icon) in a zip. It is
//!   built with Apple's linker, so it is packaged on a Mac or with a cross linker set up.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ico::{IconSet, Rgba};
use crate::project::{Project, cargo, status};

/// The icon from `remedy.toml` (`[app] icon` and the sizes `[app] icon_sizes` draws), or the
/// default one.
pub fn icon_set(p: &Project, notes: &mut Vec<String>) -> Result<IconSet, String> {
    let drawings: Vec<Rgba> = p
        .icon
        .iter()
        .chain(&p.icon_sizes)
        .map(|path| crate::ico::read_png_file(path))
        .collect::<Result<_, _>>()?;
    if drawings.is_empty() {
        notes.push("no icon in remedy.toml: used the default icon".into());
        return Ok(IconSet::single(crate::ico::default_icon([61, 139, 95])));
    }
    let set = IconSet::new(drawings, p.icon_pixel_art)?;
    let largest = set.largest().width;
    if largest < 256 {
        notes.push(format!(
            "the icon's largest drawing is {largest}x{largest}; 256x256 or larger looks sharp everywhere"
        ));
    }
    Ok(set)
}

/// The sizes of the `hicolor` icon theme's fixed-size folders.
const HICOLOR_SIZES: [u32; 9] = [16, 22, 24, 32, 48, 64, 128, 256, 512];

/// Android's screen densities: the resource folder suffix and pixels per dp, in quarters
/// (mdpi is 1 dp = 1 px).
const DENSITIES: [(&str, u32); 5] = [("mdpi", 4), ("hdpi", 6), ("xhdpi", 8), ("xxhdpi", 12), ("xxxhdpi", 16)];

/// The launcher icon's resources, as `(path under res/, contents)`: the icon at 48 dp for
/// every density (Android before 8), and an adaptive icon (8 and later) whose foreground is
/// `[android] icon_foreground` (else the icon) filling the 72 dp the launcher shows of the
/// 108 dp layer, on `[android] icon_background` (a color or a gradient), with
/// `[android] icon_monochrome` as its themed (Android 13) layer, which the status bar also
/// shows for notifications. Pictures are placed alike, so the three line up.
pub fn android_icon_resources(
    p: &Project,
    set: &IconSet,
    target_api: u32,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mono = match &p.android_icon_monochrome {
        Some(path) if target_api >= 33 => {
            let shape = crate::ico::read_png_file(path)?;
            // White, with the drawing's alpha as the shape (Android tints it).
            let data =
                shape.data.chunks(4).flat_map(|c| [255, 255, 255, c.get(3).copied().unwrap_or(0)]).collect();
            Some(IconSet::new(
                vec![Rgba { width: shape.width, height: shape.height, data }],
                set.is_pixel_art(),
            )?)
        }
        _ => None,
    };
    let front = match &p.android_icon_foreground {
        Some(path) => IconSet::new(vec![crate::ico::read_png_file(path)?], set.is_pixel_art())?,
        None => set.clone(),
    };
    for (density, quarter) in DENSITIES {
        let px = |dp: u32| dp * quarter / 4;
        out.push((format!("mipmap-{density}/ic_launcher.png"), crate::ico::write_png(&set.at(px(48)))?));
        out.push((
            format!("mipmap-{density}/ic_launcher_foreground.png"),
            crate::ico::write_png(&front.centered(px(72), px(108)))?,
        ));
        if let Some(m) = &mono {
            out.push((
                format!("mipmap-{density}/ic_launcher_monochrome.png"),
                crate::ico::write_png(&m.centered(px(72), px(108)))?,
            ));
        }
    }
    let fill = match p.android_icon_background.as_slice() {
        [top, bottom] => format!(
            "    <gradient android:angle=\"270\" android:startColor=\"{top}\" android:endColor=\"{bottom}\" />\n"
        ),
        [one, ..] => format!("    <solid android:color=\"{one}\" />\n"),
        [] => format!(
            "    <solid android:color=\"{}\" />\n",
            set.corner_color().unwrap_or_else(|| "#ffffff".into())
        ),
    };
    out.push((
        "drawable/ic_launcher_background.xml".into(),
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<shape xmlns:android=\"http://schemas.android.com/apk/res/android\" android:shape=\"rectangle\">\n{fill}</shape>\n"
        )
        .into_bytes(),
    ));
    let monochrome = if mono.is_some() {
        "    <monochrome android:drawable=\"@mipmap/ic_launcher_monochrome\" />\n"
    } else {
        ""
    };
    out.push((
        "mipmap-anydpi-v26/ic_launcher.xml".into(),
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<adaptive-icon xmlns:android=\"http://schemas.android.com/apk/res/android\">\n    <background android:drawable=\"@drawable/ic_launcher_background\" />\n    <foreground android:drawable=\"@mipmap/ic_launcher_foreground\" />\n{monochrome}</adaptive-icon>\n"
        )
        .into_bytes(),
    ));
    Ok(out)
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
    let set = icon_set(p, notes)?;
    let bytes = std::fs::read(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let base = format!("{}-{}-linux-x86_64", p.bin, p.version);
    let mut entries = vec![
        TarEntry { path: format!("{base}/bin/{}", p.bin), data: bytes, executable: true },
        TarEntry {
            path: format!("{base}/share/applications/{}.desktop", p.id),
            data: desktop_entry(p).into_bytes(),
            executable: false,
        },
    ];
    // The icon at 256 px, and at every other hicolor size the artwork was drawn at.
    let mut sizes: Vec<u32> =
        HICOLOR_SIZES.iter().copied().filter(|s| set.drawn_sizes().contains(s)).collect();
    sizes.push(256);
    sizes.sort_unstable();
    sizes.dedup();
    for size in sizes {
        entries.push(TarEntry {
            path: format!("{base}/share/icons/hicolor/{size}x{size}/apps/{}.png", p.id),
            data: crate::ico::write_png(&set.at(size))?,
            executable: false,
        });
    }
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

/// The permissions an Android app's manifest declares: `[android] permissions`, then what
/// its `[android] foreground` work needs (the foreground service permissions, the runtime
/// permission the kind of work uses, and notifications, which show the work), without
/// repeats.
pub fn android_permissions(p: &Project) -> Vec<String> {
    let mut out: Vec<String> = p.android_permissions.clone();
    let mut add = |name: &str| {
        if !out.iter().any(|o| o == name) {
            out.push(name.to_owned());
        }
    };
    if !p.android_foreground.is_empty() {
        add("android.permission.FOREGROUND_SERVICE");
        add("android.permission.POST_NOTIFICATIONS");
    }
    for kind in &p.android_foreground {
        match kind.as_str() {
            "microphone" => {
                add("android.permission.FOREGROUND_SERVICE_MICROPHONE");
                add("android.permission.RECORD_AUDIO");
            }
            "mediaPlayback" => add("android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK"),
            "camera" => {
                add("android.permission.FOREGROUND_SERVICE_CAMERA");
                add("android.permission.CAMERA");
            }
            "dataSync" => add("android.permission.FOREGROUND_SERVICE_DATA_SYNC"),
            _ => {}
        }
    }
    out
}

/// The Android manifest: the bridge's `RemedyActivity` (a `NativeActivity` that also hears
/// notification presses) loading the app's library, the alias notifications open it through,
/// the `SharedFiles` provider that serves the files the app shares to the app the person chose
/// (Android makes providers only from the APK's own code, which is why the APK carries the
/// bridge's dex), the service that keeps keepalive requests running out of sight, and the
/// foreground service when the app does foreground work.
pub fn android_manifest(
    p: &Project,
    lib_name: &str,
    version_code: u32,
    min_api: u32,
    target_api: u32,
) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    let permissions: String = android_permissions(p)
        .iter()
        .map(|n| format!("    <uses-permission android:name=\"{}\" />\n", esc(n)))
        .collect();
    let service = if p.android_foreground.is_empty() {
        String::new()
    } else {
        format!(
            r#"        <!-- Ongoing work that keeps the app running out of sight (App::start_foreground). -->
        <service android:name="dev.remedyui.android.Foreground"
            android:exported="false"
            android:foregroundServiceType="{}" />
"#,
            esc(&p.android_foreground.join("|"))
        )
    };
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="{id}" android:versionCode="{version_code}" android:versionName="{version}">
    <uses-sdk android:minSdkVersion="{min_api}" android:targetSdkVersion="{target_api}" />
{permissions}    <application android:label="{label}" android:icon="@mipmap/ic_launcher" android:hasCode="true">
        <activity android:name="dev.remedyui.android.RemedyActivity"
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
        <!-- Pressing one of the app's notifications opens it through this alias: not exported,
             so only the app's own notifications can say they were pressed. -->
        <activity-alias android:name="dev.remedyui.android.NotificationOpen"
            android:targetActivity="dev.remedyui.android.RemedyActivity"
            android:exported="false" />
        <!-- Serves shared files to the app the person chose, read-only, for that share. -->
        <provider android:name="dev.remedyui.android.SharedFiles"
            android:authorities="{id}.remedy.share"
            android:exported="false"
            android:grantUriPermissions="true" />
        <!-- Decodes images in processes of their own with no permissions (isolated decoding,
             remedy-ui-media-isolated); unused unless the app enables it. -->
        <service android:name="dev.remedyui.android.Decoder"
            android:isolatedProcess="true"
            android:exported="false" />
        <!-- Keeps the process running out of sight while keepalive requests (an app's last
             saves) finish; no notification, no permission. -->
        <service android:name="dev.remedyui.android.Keepalive"
            android:exported="false" />
{service}    </application>
</manifest>
"#,
        id = esc(&p.id),
        version = esc(&p.version),
        label = esc(&p.name),
    )
}

/// The Android bridge's Java half, compiled (`dex/classes.dex` beside the bridge's
/// Cargo.toml): the release of the bridge the app builds with, found with `cargo metadata`.
fn bridge_dex(p: &Project, target: &str) -> Result<PathBuf, String> {
    let out = Command::new(cargo())
        .current_dir(&p.root)
        .args(["metadata", "--format-version", "1", "--filter-platform", target])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    bridge_dex_in(&v)
}

/// The bridge's dex in `cargo metadata` output.
fn bridge_dex_in(metadata: &serde_json::Value) -> Result<PathBuf, String> {
    let manifest = metadata
        .get("packages")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("remedy-ui-bridge-android"))
        .and_then(|p| p.get("manifest_path"))
        .and_then(|m| m.as_str())
        .ok_or("the app does not build with remedy-ui-bridge-android (through remedy-ui) for Android")?;
    let dex = Path::new(manifest).parent().unwrap_or(Path::new(".")).join("dex").join("classes.dex");
    if dex.is_file() { Ok(dex) } else { Err(format!("{} is missing", dex.display())) }
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
    // The bridge's Java, as the app's code.
    let dex = bridge_dex(p, &target)?;
    std::fs::copy(&dex, stage.join("classes.dex")).map_err(|e| format!("{}: {e}", dex.display()))?;
    // The launcher icon, also the source of the notifications' status bar icon.
    let set = icon_set(p, notes)?;
    if p.android_icon_monochrome.is_some() && target_api < 33 {
        notes.push("[android] icon_monochrome needs target_api 33 or later: left out".into());
    }
    for (path, data) in android_icon_resources(p, &set, target_api)? {
        let file = stage.join("res").join(path);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, data).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    let manifest = stage.join("AndroidManifest.xml");
    std::fs::write(&manifest, android_manifest(p, &lib_name, version_code(&p.version), min_api, target_api))
        .map_err(|e| e.to_string())?;
    let build_tools = newest(&sdk.join("build-tools")).ok_or("no build-tools under $ANDROID_HOME")?;
    let jar = sdk.join("platforms").join(format!("android-{target_api}")).join("android.jar");
    if !jar.exists() {
        return Err(format!("{} is missing: install the Android {target_api} platform", jar.display()));
    }
    let compiled = stage.join("res.zip");
    status(
        Command::new(tool(&build_tools, "aapt2"))
            .arg("compile")
            .arg("--dir")
            .arg(stage.join("res"))
            .arg("-o")
            .arg(&compiled),
    )?;
    let base = stage.join("base.apk");
    status(
        Command::new(tool(&build_tools, "aapt2"))
            .arg("link")
            .arg("-o")
            .arg(&base)
            .arg("-I")
            .arg(&jar)
            .arg("--manifest")
            .arg(&manifest)
            .arg(&compiled),
    )?;
    status(
        Command::new(tool(&build_tools, "aapt"))
            .current_dir(&stage)
            .args(["add", "-f", "base.apk", "classes.dex"])
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
{schemes}</dict>
</plist>
"#,
        name = x(&p.name),
        id = x(&p.id),
        version = x(&p.version),
        bin = x(&p.bin),
        schemes = url_types(p),
    )
}

/// `CFBundleURLTypes` for the app's URL schemes, so links in them open the app.
fn url_types(p: &Project) -> String {
    if p.url_schemes.is_empty() {
        return String::new();
    }
    let x = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let list: String = p.url_schemes.iter().map(|s| format!("<string>{}</string>", x(s))).collect();
    format!(
        "  <key>CFBundleURLTypes</key><array><dict><key>CFBundleURLName</key><string>{}</string><key>CFBundleURLSchemes</key><array>{list}</array></dict></array>\n",
        x(&p.id)
    )
}

/// An `.icns` icon of PNG entries, each with its type (`ic08` 256 px, `ic09` 512, `ic10` 1024).
pub fn icns_entries(entries: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let total = 8 + entries.iter().map(|(_, png)| 8 + png.len()).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(total as u32).to_be_bytes());
    for (kind, png) in entries {
        out.extend_from_slice(kind);
        out.extend_from_slice(&((8 + png.len()) as u32).to_be_bytes());
        out.extend_from_slice(png);
    }
    out
}

/// The `.icns` entries for an icon set: 256 px, and 512 and 1024 px when the artwork is that
/// large.
fn mac_icon(set: &IconSet) -> Result<Vec<u8>, String> {
    let largest = set.largest().width;
    let mut entries = vec![(*b"ic08", crate::ico::write_png(&set.at(256))?)];
    for (kind, size) in [(*b"ic09", 512), (*b"ic10", 1024)] {
        if largest >= size {
            entries.push((kind, crate::ico::write_png(&set.at(size))?));
        }
    }
    Ok(icns_entries(&entries))
}

/// Packages for macOS; returns what it wrote.
pub fn macos(p: &Project, dist: &Path, notes: &mut Vec<String>) -> Result<Vec<PathBuf>, String> {
    let target = if cfg!(target_os = "macos") { None } else { Some("aarch64-apple-darwin") };
    if target.is_some() {
        notes.push("built with --target aarch64-apple-darwin: this needs Apple's linker (a Mac, or a cross linker set as CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER)".into());
    }
    let exe = build_bin(p, target)?;
    let mac = mac_icon(&icon_set(p, notes)?)?;
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
    put(&mut zip, format!("{app}/Contents/Resources/app.icns"), &mac, 0o644)?;
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
            icon_sizes: Vec::new(),
            icon_pixel_art: false,
            android_icon_monochrome: None,
            android_icon_foreground: None,
            android_icon_background: Vec::new(),
            sign_thumbprint: None,
            windows_autostart: None,
            timestamp_url: String::new(),
            android_target: "aarch64-linux-android".into(),
            android_min_api: 26,
            android_target_api: 35,
            android_keystore: None,
            android_key_alias: None,
            android_permissions: Vec::new(),
            android_foreground: Vec::new(),
            web_short_name: None,
            web_theme: crate::project::DEFAULT_THEME_COLOR.into(),
            web_icon: None,
            uses_remedy_ui: true,
            url_schemes: Vec::new(),
        }
    }

    #[test]
    fn android_icons_are_adaptive_with_a_themed_layer() {
        let dir = std::env::temp_dir().join(format!("remedy-android-icon-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // A 4 px opaque square on a dark corner, and its one-color shape.
        let pic = Rgba { width: 4, height: 4, data: [10, 20, 30, 255].repeat(16) };
        let shape = Rgba { width: 4, height: 4, data: [0, 0, 0, 200].repeat(16) };
        let mono_path = dir.join("mono.png");
        let _ = std::fs::write(&mono_path, crate::ico::write_png(&shape).unwrap_or_default());
        let set = IconSet::new(vec![pic], true).unwrap_or_else(|e| unreachable!("{e}"));
        let mut p = project();
        p.android_icon_monochrome = Some(mono_path);
        let files = android_icon_resources(&p, &set, 35).unwrap_or_default();
        let get =
            |name: &str| files.iter().find(|(n, _)| n == name).map(|(_, d)| d.clone()).unwrap_or_default();
        let xml = String::from_utf8(get("mipmap-anydpi-v26/ic_launcher.xml")).unwrap_or_default();
        assert!(xml.contains("@mipmap/ic_launcher_foreground") && xml.contains("<monochrome"), "{xml}");
        let colors = String::from_utf8(get("drawable/ic_launcher_background.xml")).unwrap_or_default();
        assert!(colors.contains("#0a141e"), "the icon's corner color: {colors}");
        let legacy = crate::ico::read_png(&get("mipmap-xxxhdpi/ic_launcher.png")).ok();
        assert_eq!(legacy.map(|i| i.width), Some(192), "48 dp at xxxhdpi");
        let fg = crate::ico::read_png(&get("mipmap-mdpi/ic_launcher_foreground.png")).ok();
        assert_eq!(fg.as_ref().map(|i| i.width), Some(108));
        assert_eq!(fg.as_ref().and_then(|i| i.data.get(3).copied()), Some(0), "clear outside the 72 dp");
        let mono = crate::ico::read_png(&get("mipmap-hdpi/ic_launcher_monochrome.png")).ok();
        let middle = (81 * 162 + 81) * 4;
        assert_eq!(
            mono.and_then(|i| i.data.get(middle..middle + 4).map(<[u8]>::to_vec)),
            Some(vec![255, 255, 255, 200])
        );
        // Before API 33 there is no themed layer; a set color wins over the corner.
        p.android_icon_background = vec!["#123456".into(), "#654321".into()];
        let old = android_icon_resources(&p, &set, 32).unwrap_or_default();
        assert!(!old.iter().any(|(n, _)| n.contains("monochrome")));
        let gradient = old.iter().find(|(n, _)| n == "drawable/ic_launcher_background.xml");
        let gradient = gradient.map(|(_, d)| String::from_utf8_lossy(d).to_string()).unwrap_or_default();
        assert!(
            gradient.contains("startColor=\"#123456\"") && gradient.contains("endColor=\"#654321\""),
            "{gradient}"
        );
        let _ = std::fs::remove_dir_all(&dir);
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
        assert!(m.contains(r#"android:hasCode="true""#), "the APK's dex holds the share provider");
        assert!(
            m.contains(r#"android:name="dev.remedyui.android.SharedFiles""#)
                && m.contains(r#"android:authorities="app.remedy.my-notes.remedy.share""#)
                && m.contains(r#"android:exported="false""#)
                && m.contains(r#"android:grantUriPermissions="true""#),
            "{m}"
        );
        // Every class the manifest names is in the bridge's dex the APK carries (a dex file
        // names its classes).
        let bridge =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../remedy-ui-bridge-android/dex/classes.dex");
        let dex = std::fs::read(&bridge).unwrap_or_default();
        assert!(dex.starts_with(b"dex\n"), "{}", bridge.display());
        assert!(
            m.contains(r#"<service android:name="dev.remedyui.android.Decoder""#)
                && m.contains(r#"android:isolatedProcess="true""#),
            "{m}"
        );
        assert!(m.contains(r#"<service android:name="dev.remedyui.android.Keepalive""#), "{m}");
        for class in ["SharedFiles", "RemedyActivity", "Foreground", "Decoder", "Keepalive"] {
            let name = format!("Ldev/remedyui/android/{class};");
            assert!(dex.windows(name.len()).any(|w| w == name.as_bytes()), "{class} is missing from the dex");
        }
        let mut asks = project();
        asks.android_permissions = vec!["android.permission.RECORD_AUDIO".into()];
        let m = android_manifest(&asks, "my_notes", 100, 26, 35);
        assert!(m.contains(r#"<uses-permission android:name="android.permission.RECORD_AUDIO" />"#), "{m}");
        assert_eq!(m.matches("uses-permission").count(), 1);
        assert!(!m.contains("dev.remedyui.android.Foreground"), "no foreground service unless asked for");
        assert_eq!(version_code("0.1.0"), 100);
        let plist = info_plist(&p);
        assert!(plist.contains("<string>My Notes</string>") && plist.contains("<string>my-notes</string>"));
        assert!(!plist.contains("CFBundleURLTypes"), "no schemes unless asked for");
        let mut linked = project();
        linked.url_schemes = vec!["hearth".into()];
        let plist = info_plist(&linked);
        assert!(
            plist.contains("<key>CFBundleURLSchemes</key><array><string>hearth</string></array>"),
            "{plist}"
        );
        let i = icns_entries(&[(*b"ic08", b"PNG".to_vec())]);
        assert_eq!(i.get(..4), Some(&b"icns"[..]));
        assert_eq!(i.len(), 19);
    }

    #[test]
    fn android_apps_run_in_the_bridges_activity_and_declare_their_foreground_work() {
        let p = project();
        let m = android_manifest(&p, "my_notes", 100, 26, 35);
        // The bridge's own code, its activity keeping the native activity's settings.
        assert!(m.contains(r#"android:hasCode="true""#) && !m.contains(r#"hasCode="false""#));
        assert!(m.contains(r#"<activity android:name="dev.remedyui.android.RemedyActivity""#));
        assert!(m.contains(r#"<meta-data android:name="android.app.lib_name" android:value="my_notes" />"#));
        assert!(m.contains("android.intent.category.LAUNCHER"));
        assert!(m.contains("orientation|screenSize|screenLayout|smallestScreenSize|keyboard|keyboardHidden|navigation|uiMode|fontScale|locale|layoutDirection|density"));
        assert!(m.contains(r#"android:windowSoftInputMode="adjustResize""#));
        assert!(m.contains(r#"android:theme="@android:style/Theme.DeviceDefault.NoActionBar""#));
        assert!(m.contains(r#"android:icon="@mipmap/ic_launcher""#));
        // Notifications open it through an alias no other app may start.
        let alias = m.split("<activity-alias").nth(1).unwrap_or_default();
        assert!(alias.contains(r#"android:name="dev.remedyui.android.NotificationOpen""#), "{m}");
        assert!(alias.contains(r#"android:targetActivity="dev.remedyui.android.RemedyActivity""#));
        assert!(alias.contains(r#"android:exported="false""#));

        let mut call = project();
        call.android_permissions =
            vec!["android.permission.RECORD_AUDIO".into(), "android.permission.INTERNET".into()];
        call.android_foreground = vec!["microphone".into(), "mediaPlayback".into()];
        let m = android_manifest(&call, "my_notes", 100, 26, 35);
        assert!(m.contains(r#"<service android:name="dev.remedyui.android.Foreground""#), "{m}");
        assert!(m.contains(r#"android:foregroundServiceType="microphone|mediaPlayback""#));
        assert_eq!(
            android_permissions(&call),
            [
                "android.permission.RECORD_AUDIO",
                "android.permission.INTERNET",
                "android.permission.FOREGROUND_SERVICE",
                "android.permission.POST_NOTIFICATIONS",
                "android.permission.FOREGROUND_SERVICE_MICROPHONE",
                "android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK",
            ],
            "each once, the app's own first"
        );
        assert_eq!(m.matches("<uses-permission").count(), 6);
        let mut sync = project();
        sync.android_foreground = vec!["dataSync".into(), "camera".into()];
        assert_eq!(
            android_permissions(&sync),
            [
                "android.permission.FOREGROUND_SERVICE",
                "android.permission.POST_NOTIFICATIONS",
                "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
                "android.permission.FOREGROUND_SERVICE_CAMERA",
                "android.permission.CAMERA",
            ]
        );
    }

    #[test]
    fn the_bridges_dex_is_found_through_cargo_metadata() {
        let here = env!("CARGO_MANIFEST_DIR");
        let bridge = Path::new(here).join("../remedy-ui-bridge-android/Cargo.toml");
        let meta = serde_json::json!({ "packages": [
            { "name": "remedy-ui", "manifest_path": "/x/remedy-ui/Cargo.toml" },
            { "name": "remedy-ui-bridge-android", "manifest_path": bridge.to_string_lossy() },
        ]});
        let dex = bridge_dex_in(&meta).unwrap_or_default();
        assert!(dex.ends_with("dex/classes.dex") && dex.is_file(), "{}", dex.display());
        let none =
            serde_json::json!({ "packages": [{ "name": "remedy-ui", "manifest_path": "/x/Cargo.toml" }] });
        assert!(bridge_dex_in(&none).is_err_and(|e| e.contains("remedy-ui-bridge-android")));
    }
}
