//! `cargo remedy package`: a release build, packaged for Windows (here), for Linux,
//! Android and macOS ([`crate::platforms`]), or as an installable web page ([`crate::web`]).
//!
//! 1. The icon (from `remedy.toml`, or a default one) becomes a multi-size `.ico`.
//! 2. The icon and the version information are written as a resource file
//!    ([`remedy_ui_build`], no resource compiler needed) and linked into the executable.
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
            "web" => crate::web::package(&p, &crate::web::Options::parse(args), &dist, &mut notes)?,
            other => {
                return Err(format!("no packaging for `{other}`: use windows, linux, macos, android or web"));
            }
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
    let icons = crate::platforms::icon_set(&p, &mut notes)?;
    let ico_path = work.join("app.ico");
    std::fs::write(&ico_path, crate::ico::ico(&icons)?).map_err(|e| e.to_string())?;

    // 2. Resources, and the release build. An app whose build script adds its own
    // (`remedy_ui_build`) stands aside while this build runs.
    let res_path = work.join("app.res");
    std::fs::write(&res_path, resources(&p, std::fs::read(&ico_path).map_err(|e| e.to_string())?)?)
        .map_err(|e| e.to_string())?;
    let mut build = Command::new(cargo());
    build
        .current_dir(&p.root)
        .env(remedy_ui_build::PACKAGE_RESOURCES_ENV, "1")
        .args(["rustc", "--release", "--bin", &p.bin])
        .arg("--")
        .arg("-C")
        .arg(format!("link-arg={}", res_path.display()));
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

/// The executable's resources: the icon and the version information.
pub fn resources(p: &Project, ico: Vec<u8>) -> Result<Vec<u8>, String> {
    remedy_ui_build::WindowsResources::empty(&p.version)
        .icon_bytes(ico)
        .company(&p.publisher)
        .file_description(if p.description.is_empty() { &p.name } else { &p.description })
        .executable(&p.bin)
        .product_name(&p.name)
        .to_res()
}

fn iss_string(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// The unchecked "start when I sign in" task, when `[windows] autostart_args` is set.
fn autostart_task(p: &Project) -> String {
    if p.windows_autostart.is_none() {
        return String::new();
    }
    format!(
        r#"Name: "startup"; Description: "Start {} when I sign in"; Flags: unchecked"#,
        iss_string(&p.name).replace('{', "{{")
    )
}

/// What the task writes: the Run value `autostart::Entry::windows_run_key` reads (removed on
/// uninstall), and no Startup-folder shortcut beside it, so the app starts once.
fn autostart_entries(p: &Project, exe_name: &str) -> String {
    let Some(args) = &p.windows_autostart else { return String::new() };
    let name = iss_string(&p.name).replace('{', "{{");
    let args = iss_string(args).replace('{', "{{");
    format!(
        "[Registry]\nRoot: HKCU; Subkey: \"Software\\Microsoft\\Windows\\CurrentVersion\\Run\"; ValueType: string; ValueName: \"{name}\"; ValueData: \"\"\"{{app}}\\{exe_name}\"\" {args}\"; Flags: uninsdeletevalue; Tasks: startup\n\n[InstallDelete]\nType: files; Name: \"{{userstartup}}\\{name}.lnk\"; Tasks: startup\n\n"
    )
}

/// Keeps an earlier install's desktop shortcut when `[windows.replaces]` removes it.
fn replaces_icons(p: &Project, exe_name: &str) -> String {
    if p.windows_replaces.is_none() {
        return String::new();
    }
    format!(
        "Name: \"{{autodesktop}}\\{}\"; Filename: \"{{app}}\\{exe_name}\"; Check: HadOldDesktopIcon\n",
        iss_string(&p.name)
    )
}

/// The `[Code]` that takes over `[windows.replaces]`. Before anything is installed it waits
/// for the old program to close (its updater starts this setup, then quits), uninstalls a
/// per-machine MSI build with `msiexec`, then removes only the per-user build's named files,
/// its shortcuts and keys. The app's data lives elsewhere and is never touched. Setup stops,
/// with the old install still working, if the program stays open or the MSI uninstall fails.
fn replaces_code(p: &Project) -> String {
    let Some(r) = &p.windows_replaces else { return String::new() };
    const CODE: &str = r#"[Code]
const
  UninstallRoot = 'Software\Microsoft\Windows\CurrentVersion\Uninstall';
  OldUninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\@KEY@';
var
  OldDir: String;
  OldMsi: String;
  OldDesktop: Boolean;

// The product code of a per-machine MSI build named @MSI@, or ''.
function FindOldMsi(): String;
var
  Names: TArrayOfString;
  I: Integer;
  Name: String;
  IsMsi: Cardinal;
begin
  Result := '';
  if (@MSI_WANTED@) and RegGetSubkeyNames(HKLM64, UninstallRoot, Names) then
    for I := 0 to GetArrayLength(Names) - 1 do
      if RegQueryStringValue(HKLM64, UninstallRoot + '\' + Names[I], 'DisplayName', Name) and (Name = '@MSI@')
        and RegQueryDWordValue(HKLM64, UninstallRoot + '\' + Names[I], 'WindowsInstaller', IsMsi) and (IsMsi = 1) then
      begin
        Result := Names[I];
        Exit;
      end;
end;

function InitializeSetup(): Boolean;
begin
  if RegQueryStringValue(HKCU, OldUninstallKey, 'InstallLocation', OldDir) then
    OldDir := RemoveBackslashUnlessRoot(RemoveQuotes(OldDir))
  else
    OldDir := '';
  OldMsi := FindOldMsi();
  OldDesktop := ((OldDir <> '') or (OldMsi <> '')) 
    and (FileExists(ExpandConstant('{userdesktop}') + '\@NAME@.lnk') or FileExists(ExpandConstant('{commondesktop}') + '\@NAME@.lnk'));
  Result := True;
end;

function HadOldDesktopIcon(): Boolean;
begin
  Result := OldDesktop;
end;

// False once the person gives up waiting (or at once when setup is silent).
function KeepWaiting(var Tries: Integer): Boolean;
begin
  Result := True;
  Tries := Tries + 1;
  if Tries < 40 then
    Sleep(250)
  else if WizardSilent() or (SuppressibleMsgBox('Close @NAME@ (in the tray too), then press Retry.', mbError, MB_RETRYCANCEL, IDRETRY) <> IDRETRY) then
    Result := False
  else
    Tries := 0;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Tries: Integer;
  Code: Integer;
begin
  Result := '';
  if (OldDir = '') and (OldMsi = '') then
    Exit;
  Tries := 0;
  while (@MUTEX_WANTED@) and CheckForMutexes('@MUTEX@') do
    if not KeepWaiting(Tries) then
    begin
      Result := '@NAME@ is still running. Close it and run setup again.';
      Exit;
    end;
  if OldMsi <> '' then
  begin
    // 0 removed, 1605 already gone, 3010 removed once Windows restarts.
    if not Exec(ExpandConstant('{sys}\msiexec.exe'), '/x ' + OldMsi + ' /passive /norestart', '', SW_SHOW, ewWaitUntilTerminated, Code)
      or ((Code <> 0) and (Code <> 1605) and (Code <> 3010)) then
    begin
      Result := 'The earlier @NAME@ installed for all users was not removed (Windows Installer code ' + IntToStr(Code) + '). Your data is untouched; run setup again.';
      Exit;
    end;
  end;
  if OldDir = '' then
    Exit;
  Tries := 0;
  while FileExists(OldDir + '\@PROGRAM@') and not DeleteFile(OldDir + '\@PROGRAM@') do
    if not KeepWaiting(Tries) then
    begin
      Result := '@NAME@ is still running. Close it and run setup again.';
      Exit;
    end;
@DELETE_FILES@  // Only if nothing else is left in it.
  RemoveDir(OldDir);
  DeleteFile(ExpandConstant('{userprograms}') + '\@NAME@.lnk');
  DeleteFile(ExpandConstant('{userdesktop}') + '\@NAME@.lnk');
  RegDeleteKeyIncludingSubkeys(HKCU, OldUninstallKey);
@DELETE_KEYS@end;
"#;
    // Pascal strings: a quote is doubled. The names in `r` were checked to hold none.
    // The first file is the program; project.rs refuses a list without one.
    let Some((program, others)) = r.files.split_first() else { return String::new() };
    let files: String = others.iter().map(|f| format!("  DeleteFile(OldDir + '\\{f}');\n")).collect();
    let keys: String =
        r.registry.iter().map(|k| format!("  RegDeleteKeyIncludingSubkeys(HKCU, '{k}');\n")).collect();
    CODE.replace("@KEY@", &r.uninstall_key)
        .replace("@MSI_WANTED@", if r.msi_name.is_some() { "True" } else { "False" })
        .replace("@MSI@", r.msi_name.as_deref().unwrap_or(""))
        .replace("@MUTEX_WANTED@", if r.mutex.is_some() { "True" } else { "False" })
        .replace("@MUTEX@", r.mutex.as_deref().unwrap_or(""))
        .replace("@PROGRAM@", program)
        .replace("@DELETE_FILES@", &files)
        .replace("@DELETE_KEYS@", &keys)
        .replace("@NAME@", &p.name.replace('\'', "''"))
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
UninstallDisplayName={name}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "desktopicon"; Description: "{{cm:CreateDesktopIcon}}"; GroupDescription: "{{cm:AdditionalIcons}}"; Flags: unchecked
{autostart_task}

[Files]
Source: "{exe}"; DestDir: "{{app}}"; Flags: ignoreversion

[Icons]
Name: "{{autoprograms}}\{name}"; Filename: "{{app}}\{exe_name}"
Name: "{{autodesktop}}\{name}"; Filename: "{{app}}\{exe_name}"; Tasks: desktopicon
{replaces_icons}
{autostart_entries}[Run]
Filename: "{{app}}\{exe_name}"; Description: "{{cm:LaunchProgram,{name_esc}}}"; Flags: nowait postinstall skipifsilent

{replaces_code}"#,
        replaces_icons = replaces_icons(p, &exe_name),
        replaces_code = replaces_code(p),
        autostart_task = autostart_task(p),
        autostart_entries = autostart_entries(p, &exe_name),
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
            icon_sizes: Vec::new(),
            icon_pixel_art: false,
            android_icon_monochrome: None,
            android_icon_foreground: None,
            android_icon_background: Vec::new(),
            sign_thumbprint: None,
            windows_autostart: None,
            windows_replaces: None,
            linux_glibc: None,
            timestamp_url: String::new(),
            lib_name: "notes".into(),
            android_target: "aarch64-linux-android".into(),
            android_min_api: 26,
            android_target_api: 35,
            android_keystore: None,
            android_permissions: Vec::new(),
            android_foreground: Vec::new(),
            android_key_alias: None,
            web_short_name: None,
            web_theme: crate::project::DEFAULT_THEME_COLOR.into(),
            web_icon: None,
            uses_remedy_ui: true,
            url_schemes: Vec::new(),
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
        let ico = crate::ico::ico(&crate::ico::IconSet::single(crate::ico::default_icon([1, 2, 3])))
            .unwrap_or_default();
        let res = resources(&p, ico).unwrap_or_default();
        let text: String = res
            .chunks(2)
            .filter_map(|c| c.first().filter(|_| c.get(1) == Some(&0)))
            .map(|b| char::from(*b))
            .collect();
        assert!(text.contains("Notes, \"Plus\"") && text.contains("1.2.3-beta.1"), "{text}");
        let iss = inno_script(
            &p,
            Path::new("C:/x/notes.exe"),
            Path::new("C:/x/app.ico"),
            Path::new("C:/out"),
            "notes_1.2.3_x64-setup",
        );
        assert!(iss.contains("AppId=app.remedy.notes") && iss.contains("PrivilegesRequired=lowest"), "{iss}");
        assert!(
            iss.contains("UninstallDisplayName=Notes, \"\"Plus\"\""),
            "the name as installed apps list it: {iss}"
        );
        assert!(
            iss.contains("{localappdata}") && iss.contains("{cm:LaunchProgram,Notes%2c \"Plus\"}"),
            "{iss}"
        );
        assert!(!iss.contains("startup"), "no sign-in choice unless asked");
        let mut p = p;
        p.windows_autostart = Some("--background".into());
        let iss =
            inno_script(&p, Path::new("C:/x/notes.exe"), Path::new("C:/x/app.ico"), Path::new("C:/out"), "n");
        assert!(
            iss.contains(
                r#"Name: "startup"; Description: "Start Notes, ""Plus"" when I sign in"; Flags: unchecked"#
            ),
            "{iss}"
        );
        assert!(
            iss.contains(r#"ValueName: "Notes, ""Plus"""; ValueData: """{app}\notes.exe"" --background"; Flags: uninsdeletevalue; Tasks: startup"#),
            "{iss}"
        );
        assert!(
            iss.contains(r#"Name: "{userstartup}\Notes, ""Plus"".lnk"; Tasks: startup"#),
            "never a second start: {iss}"
        );
        assert!(!iss.contains("[Code]") && !iss.contains("HadOldDesktopIcon"), "no takeover unless asked");
    }

    #[test]
    fn a_replaced_install_loses_only_its_named_files_and_keys() {
        let mut p = project();
        p.name = "Notes".into();
        p.windows_replaces = Some(crate::project::Replaces {
            uninstall_key: "Notes".into(),
            files: vec!["notes.exe".into(), "uninstall.exe".into()],
            registry: vec![r"Software\maker\Notes".into()],
            mutex: Some("notes-running".into()),
            msi_name: Some("Notes".into()),
        });
        let iss =
            inno_script(&p, Path::new("C:/x/notes.exe"), Path::new("C:/x/app.ico"), Path::new("C:/out"), "n");
        for want in [
            r"OldUninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Notes';",
            r"while FileExists(OldDir + '\notes.exe') and not DeleteFile(OldDir + '\notes.exe') do",
            r"DeleteFile(OldDir + '\uninstall.exe');",
            "RemoveDir(OldDir);",
            "while (True) and CheckForMutexes('notes-running') do",
            "and (Name = 'Notes')",
            "'/x ' + OldMsi + ' /passive /norestart'",
            r"DeleteFile(ExpandConstant('{userprograms}') + '\Notes.lnk');",
            r"RegDeleteKeyIncludingSubkeys(HKCU, 'Software\maker\Notes');",
            r#"Name: "{autodesktop}\Notes"; Filename: "{app}\notes.exe"; Check: HadOldDesktopIcon"#,
        ] {
            assert!(iss.contains(want), "{want}\n{iss}");
        }
        assert!(!iss.contains("DelTree") && !iss.contains("{userappdata}"), "never the data: {iss}");
    }
}
