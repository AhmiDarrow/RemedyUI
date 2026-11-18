//! `cargo remedy package`: a release build, packaged for Windows (here), or for Linux,
//! Android and macOS ([`crate::platforms`]).
//!
//! 1. The icon (from `remedy.toml`, or a default one) becomes a multi-size `.ico`.
//! 2. With a resource compiler (`llvm-rc`, or `rc` from the Windows SDK), the icon and the
//!    version information are compiled into the executable.
//! 3. With a certificate thumbprint in `remedy.toml` and `signtool`, the executable and the
//!    installer are signed.
//! 4. Inno Setup builds a per-user installer (`<bin>_<version>_x64-setup.exe`): Start menu
//!    entry, optional desktop shortcut, uninstaller, launch at the end.
//! 5. A portable zip holds the executable and the README and license files.
//! 6. With `REMEDY_UPDATE_KEY` (a minisign secret key file), the installer gets a `.sig`
//!    for the updater.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::{Project, cargo, status};

pub fn run(args: &[String]) -> Result<(), String> {
    let default = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "windows"
    };
    let target = crate::flag_value(args, "--target").unwrap_or_else(|| default.into());
    if target != "windows" {
        let root = std::env::current_dir().map_err(|e| e.to_string())?;
        let p = Project::load(&root)?;
        let dist = p.target_dir.join("remedy").join("dist");
        std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
        let mut notes = Vec::new();
        let artifacts = match target.as_str() {
            "linux" => crate::platforms::linux(&p, &dist, &mut notes)?,
            "android" => crate::platforms::android(&p, &dist, &mut notes)?,
            "macos" => crate::platforms::macos(&p, &dist, &mut notes)?,
            other => return Err(format!("no packaging for `{other}`: use windows, linux, macos or android")),
        };
        println!("Packaged {} {} for {target}:", p.name, p.version);
        for a in &artifacts {
            println!("  {}", a.display());
        }
        for n in notes {
            println!("  note: {n}");
        }
        return Ok(());
    }
    if !cfg!(windows) {
        return Err(
            "Windows packages are built on Windows (its resource compiler, signtool and Inno Setup)".into()
        );
    }
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let p = Project::load(&root)?;
    let work = p.target_dir.join("remedy").join("windows");
    let dist = p.target_dir.join("remedy").join("dist");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let mut notes: Vec<String> = Vec::new();

    // 1. Icon.
    let icon_png = match &p.icon {
        Some(path) => {
            let bytes = std::fs::read(path).map_err(|e| format!("icon {}: {e}", path.display()))?;
            crate::ico::read_png(&bytes).map_err(|e| format!("icon {}: {e}", path.display()))?
        }
        None => {
            notes.push("no icon in remedy.toml: used the default icon".into());
            crate::ico::default_icon([61, 139, 95])
        }
    };
    if icon_png.width < 256 || icon_png.height < 256 {
        notes.push(format!(
            "the icon is {}x{}; 256x256 or larger looks sharp everywhere",
            icon_png.width, icon_png.height
        ));
    }
    let ico_path = work.join("app.ico");
    std::fs::write(&ico_path, crate::ico::ico(&icon_png)?).map_err(|e| e.to_string())?;

    // 2. Resources, and the release build.
    let res = match find_rc() {
        Some(rc) => {
            let rc_path = work.join("app.rc");
            std::fs::write(&rc_path, resource_script(&p, &ico_path)).map_err(|e| e.to_string())?;
            let res_path = work.join("app.res");
            let mut cmd = Command::new(&rc);
            if rc.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("llvm-rc")) {
                cmd.arg("/FO").arg(&res_path).arg(&rc_path);
            } else {
                cmd.arg("/nologo").arg("/fo").arg(&res_path).arg(&rc_path);
            }
            status(&mut cmd)?;
            Some(res_path)
        }
        None => {
            notes.push("no resource compiler (llvm-rc or the Windows SDK's rc): the executable has no icon or version info".into());
            None
        }
    };
    let mut build = Command::new(cargo());
    build.current_dir(&p.root).args(["rustc", "--release", "--bin", &p.bin]);
    if let Some(res) = &res {
        build.arg("--").arg("-C").arg(format!("link-arg={}", res.display()));
    }
    status(&mut build)?;
    let exe = p.exe(true);
    let staged = work.join(exe.file_name().ok_or("no executable name")?);
    std::fs::copy(&exe, &staged).map_err(|e| format!("{}: {e}", exe.display()))?;

    // 3. Signing.
    let signtool = p.sign_thumbprint.as_ref().and_then(|_| find_sdk_tool("signtool.exe"));
    let sign = |file: &Path| -> Result<(), String> {
        if let (Some(tool), Some(thumb)) = (&signtool, &p.sign_thumbprint) {
            status(
                Command::new(tool)
                    .args(["sign", "/sha1", thumb, "/fd", "SHA256", "/tr", &p.timestamp_url, "/td", "SHA256"])
                    .arg(file),
            )
        } else {
            Ok(())
        }
    };
    match (&p.sign_thumbprint, &signtool) {
        (Some(_), Some(_)) => sign(&staged)?,
        (Some(_), None) => notes.push("sign_thumbprint is set but signtool was not found: unsigned".into()),
        (None, _) => notes.push("unsigned (set [windows] sign_thumbprint in remedy.toml to sign)".into()),
    }

    // 4. Installer.
    let base = format!("{}_{}_x64", p.bin, p.version);
    let mut artifacts: Vec<PathBuf> = Vec::new();
    match find_iscc() {
        Some(iscc) => {
            let iss = work.join("installer.iss");
            std::fs::write(&iss, inno_script(&p, &staged, &ico_path, &dist, &format!("{base}-setup")))
                .map_err(|e| e.to_string())?;
            status(Command::new(iscc).arg("/Q").arg(&iss))?;
            let setup = dist.join(format!("{base}-setup.exe"));
            sign(&setup)?;
            artifacts.push(setup);
        }
        None => notes.push("Inno Setup (iscc) was not found: no installer, only the portable zip".into()),
    }

    // 5. Portable zip.
    let zip_path = dist.join(format!("{base}-portable.zip"));
    let mut extras: Vec<PathBuf> = Vec::new();
    for name in ["README.md", "LICENSE", "LICENSE-MIT", "LICENSE-APACHE"] {
        let f = p.root.join(name);
        if f.exists() {
            extras.push(f);
        }
    }
    write_zip(&zip_path, &staged, &extras)?;
    artifacts.push(zip_path);

    // 6. Update signatures.
    if let Some(key) = std::env::var_os("REMEDY_UPDATE_KEY") {
        let password = std::env::var("REMEDY_UPDATE_KEY_PASSWORD").ok();
        let sk = minisign::SecretKey::from_file(PathBuf::from(&key), password)
            .map_err(|e| format!("REMEDY_UPDATE_KEY: {e}"))?;
        for a in artifacts.clone() {
            artifacts.push(sign_update(&a, &sk)?);
        }
    }

    println!("Packaged {} {}:", p.name, p.version);
    for a in &artifacts {
        println!("  {}", a.display());
    }
    for n in notes {
        println!("  note: {n}");
    }
    Ok(())
}

/// Writes `<file>.sig`: a minisign signature the updater checks against the app's key.
pub fn sign_update(file: &Path, sk: &minisign::SecretKey) -> Result<PathBuf, String> {
    let data = std::fs::File::open(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let comment = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let sig = minisign::sign(None, sk, data, Some(comment.as_str()), None).map_err(|e| e.to_string())?;
    let sig_path = PathBuf::from(format!("{}.sig", file.display()));
    std::fs::write(&sig_path, sig.to_string()).map_err(|e| e.to_string())?;
    Ok(sig_path)
}

fn version_numbers(v: &str) -> (u16, u16, u16) {
    let core = v.split(['-', '+']).next().unwrap_or("0.0.0");
    let mut it = core.split('.').map(|n| n.parse::<u16>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

fn rc_string(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// The resource script: the icon and the version information.
pub fn resource_script(p: &Project, ico: &Path) -> String {
    let (a, b, c) = version_numbers(&p.version);
    let ico = ico.display().to_string().replace('\\', "/");
    format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "{publisher}"
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{bin}"
      VALUE "OriginalFilename", "{bin}.exe"
      VALUE "ProductName", "{name}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        publisher = rc_string(&p.publisher),
        description = rc_string(if p.description.is_empty() { &p.name } else { &p.description }),
        version = rc_string(&p.version),
        bin = rc_string(&p.bin),
        name = rc_string(&p.name),
    )
}

fn iss_string(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// The Inno Setup script: a per-user install with a Start menu entry and an uninstaller.
pub fn inno_script(p: &Project, exe: &Path, ico: &Path, out: &Path, base: &str) -> String {
    let exe_name = exe.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    format!(
        r#"[Setup]
AppId={id}
AppName={name}
AppVersion={version}
AppPublisher={publisher}
AppComments={description}
DefaultDirName={{localappdata}}\Programs\{name}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir={out}
OutputBaseFilename={base}
SetupIconFile={ico}
UninstallDisplayIcon={{app}}\{exe_name}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "desktopicon"; Description: "{{cm:CreateDesktopIcon}}"; GroupDescription: "{{cm:AdditionalIcons}}"; Flags: unchecked

[Files]
Source: "{exe}"; DestDir: "{{app}}"; Flags: ignoreversion

[Icons]
Name: "{{autoprograms}}\{name}"; Filename: "{{app}}\{exe_name}"
Name: "{{autodesktop}}\{name}"; Filename: "{{app}}\{exe_name}"; Tasks: desktopicon

[Run]
Filename: "{{app}}\{exe_name}"; Description: "{{cm:LaunchProgram,{name_esc}}}"; Flags: nowait postinstall skipifsilent
"#,
        // Inno takes AppId literally; braces would start a constant.
        id = p.id.replace(['{', '}'], ""),
        name = iss_string(&p.name),
        name_esc = p.name.replace(',', "%2c").replace('}', "%7d"),
        version = iss_string(&p.version),
        publisher = iss_string(&p.publisher),
        description = iss_string(&p.description),
        out = out.display(),
        ico = ico.display(),
        exe = exe.display(),
    )
}

fn write_zip(path: &Path, exe: &Path, extras: &[PathBuf]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for f in std::iter::once(exe.to_path_buf()).chain(extras.iter().cloned()) {
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).ok_or("bad file name")?;
        let data = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(&data).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// A tool from the newest Windows SDK (x64).
fn find_sdk_tool(name: &str) -> Option<PathBuf> {
    if let Some(p) = on_path(name) {
        return Some(p);
    }
    let base =
        PathBuf::from(std::env::var_os("ProgramFiles(x86)")?).join("Windows Kits").join("10").join("bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("x64").join(name).is_file())
        .collect();
    versions.sort();
    versions.pop().map(|v| v.join("x64").join(name))
}

fn find_rc() -> Option<PathBuf> {
    on_path("llvm-rc.exe").or_else(|| find_sdk_tool("rc.exe"))
}

fn find_iscc() -> Option<PathBuf> {
    if let Some(p) = on_path("iscc.exe").or_else(|| on_path("ISCC.exe")) {
        return Some(p);
    }
    ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"].iter().find_map(|var| {
        let base = PathBuf::from(std::env::var_os(var)?);
        [
            base.join("Inno Setup 6").join("ISCC.exe"),
            base.join("Programs").join("Inno Setup 6").join("ISCC.exe"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        Project {
            root: PathBuf::from("."),
            bin: "notes".into(),
            version: "1.2.3-beta.1".into(),
            target_dir: PathBuf::from("target"),
            name: "Notes, \"Plus\"".into(),
            id: "app.remedy.notes".into(),
            publisher: "Remedy".into(),
            description: String::new(),
            icon: None,
            sign_thumbprint: None,
            timestamp_url: String::new(),
            lib_name: "notes".into(),
            android_target: "aarch64-linux-android".into(),
            android_min_api: 26,
            android_target_api: 35,
            android_keystore: None,
            android_key_alias: None,
        }
    }

    #[test]
    fn update_signatures_verify_against_the_public_key() {
        let Ok(kp) = minisign::KeyPair::generate_unencrypted_keypair() else { unreachable!("keys") };
        let dir = std::env::temp_dir().join(format!("remedy-sign-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("app_1.0.0_x64-setup.exe");
        let _ = std::fs::write(&file, b"installer bytes");
        let sig_path = sign_update(&file, &kp.sk).unwrap_or_default();
        let text = std::fs::read_to_string(&sig_path).unwrap_or_default();
        let sig = minisign::SignatureBox::from_string(&text);
        let ok = sig.and_then(|s| {
            minisign::verify(&kp.pk, &s, std::io::Cursor::new(b"installer bytes"), true, false, false)
        });
        assert!(ok.is_ok(), "{ok:?}");
        assert!(text.contains("app_1.0.0_x64-setup.exe"), "the file name is the trusted comment");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scripts_quote_names_and_carry_the_version() {
        let p = project();
        let rc = resource_script(&p, Path::new("C:/x/app.ico"));
        assert!(rc.contains("FILEVERSION 1,2,3,0") && rc.contains("\"Notes, \"\"Plus\"\"\""), "{rc}");
        let iss = inno_script(
            &p,
            Path::new("C:/x/notes.exe"),
            Path::new("C:/x/app.ico"),
            Path::new("C:/out"),
            "notes_1.2.3_x64-setup",
        );
        assert!(iss.contains("AppId=app.remedy.notes") && iss.contains("PrivilegesRequired=lowest"), "{iss}");
        assert!(
            iss.contains("{localappdata}") && iss.contains("{cm:LaunchProgram,Notes%2c \"Plus\"}"),
            "{iss}"
        );
        assert_eq!(version_numbers("10.20.30+meta"), (10, 20, 30));
    }
}
