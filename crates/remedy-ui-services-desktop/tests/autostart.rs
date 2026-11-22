//! Start-with-the-session entries, written into a temporary folder.

use remedy_ui_services_desktop::autostart::{Entry, desktop_entry, enabled_in, launch_agent, set_in};
#[cfg(windows)]
use remedy_ui_services_desktop::autostart::{enabled_at, set_at};

fn entry() -> Entry {
    Entry {
        name: "Hearth Test".into(),
        id: "com.example.hearth".into(),
        program: std::env::current_exe().unwrap_or_default(),
        args: vec!["--background".into(), "--home".into(), "C:/My Profile".into()],
        run_key: false,
    }
}

#[test]
fn entries_are_written_and_removed() {
    let dir = std::env::temp_dir().join(format!("remedy-autostart-{}", std::process::id()));
    let e = entry();
    assert!(!enabled_in(&dir, &e));
    set_in(&dir, &e, true).unwrap_or_else(|err| unreachable!("{err}"));
    assert!(enabled_in(&dir, &e));
    set_in(&dir, &e, false).unwrap_or_else(|err| unreachable!("{err}"));
    assert!(!enabled_in(&dir, &e));
    // Removing what is not there is fine.
    set_in(&dir, &e, false).unwrap_or_else(|err| unreachable!("{err}"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn launch_agents_and_desktop_entries_quote_their_arguments() {
    let mut e = entry();
    e.program = "/Applications/Hearth.app/Contents/MacOS/Hearth".into();
    e.args.push("a&b".into());
    let plist = launch_agent(&e);
    assert!(plist.contains("<string>com.example.hearth</string>"));
    assert!(plist.contains("<string>--background</string>"));
    assert!(plist.contains("<string>a&amp;b</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
    e.program = "/opt/hearth/hearth".into();
    let desktop = desktop_entry(&e);
    assert!(
        desktop.contains("Exec=/opt/hearth/hearth --background --home \"C:/My Profile\" \"a&b\""),
        "{desktop}"
    );
    assert!(desktop.starts_with("[Desktop Entry]\nType=Application\nName=Hearth Test\n"));
}

/// On Windows the shortcut is a real .lnk that points at the program.
#[cfg(windows)]
#[test]
fn windows_shortcuts_point_at_the_program() {
    let dir = std::env::temp_dir().join(format!("remedy-autostart-lnk-{}", std::process::id()));
    let e = entry();
    set_in(&dir, &e, true).unwrap_or_else(|err| unreachable!("{err}"));
    let lnk = std::fs::read(dir.join("Hearth Test.lnk")).unwrap_or_default();
    // A shell link starts with its header size and the ShellLink class id.
    assert_eq!(lnk.get(..4), Some(&[0x4C, 0, 0, 0][..]));
    let utf16: Vec<u8> = "--background".encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert!(lnk.windows(utf16.len()).any(|w| w == utf16.as_slice()), "the arguments are in the shortcut");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Run key of the test's own under HKCU (never the real one), removed when dropped.
#[cfg(windows)]
struct TestRunKey(String);

#[cfg(windows)]
impl Drop for TestRunKey {
    fn drop(&mut self) {
        let mine = self.0.trim_end_matches(r"\Run");
        let _ = std::process::Command::new("reg").args(["delete", &format!(r"HKCU\{mine}"), "/f"]).output();
    }
}

#[cfg(windows)]
#[test]
fn the_run_key_is_written_adopted_and_never_doubled() {
    let key = TestRunKey(format!(r"Software\RemedyUI-autostart-test-{}\Run", std::process::id()));
    let dir = std::env::temp_dir().join(format!("remedy-autostart-run-{}", std::process::id()));
    let lnk = dir.join("Hearth Test.lnk");
    let ok = |r: Result<(), remedy_ui_services_desktop::DesktopError>| {
        r.unwrap_or_else(|err| unreachable!("{err}"))
    };
    let shortcut = entry();
    let run = entry().windows_run_key(true);
    assert!(!enabled_at(&dir, &key.0, &run));

    // A shortcut first; then the Run key replaces it rather than adding a second start.
    ok(set_at(&dir, &key.0, &shortcut, true));
    assert!(lnk.exists());
    ok(set_at(&dir, &key.0, &run, true));
    assert!(!lnk.exists(), "the shortcut went when the Run value came");
    assert!(enabled_at(&dir, &key.0, &run));

    // An installer's Run value reads as on, and turning it on again adds no shortcut.
    assert!(enabled_at(&dir, &key.0, &shortcut), "the Run value is adopted");
    ok(set_at(&dir, &key.0, &shortcut, true));
    assert!(!lnk.exists(), "never both");

    // Off removes it wherever it is.
    ok(set_at(&dir, &key.0, &shortcut, false));
    assert!(!enabled_at(&dir, &key.0, &run));
    ok(set_at(&dir, &key.0, &run, false));
    let _ = std::fs::remove_dir_all(&dir);
}
