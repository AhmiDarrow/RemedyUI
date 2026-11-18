//! Start-with-the-session entries, written into a temporary folder.

use remedy_ui_services_desktop::autostart::{Entry, desktop_entry, enabled_in, launch_agent, set_in};

fn entry() -> Entry {
    Entry {
        name: "Hearth Test".into(),
        id: "com.example.hearth".into(),
        program: std::env::current_exe().unwrap_or_default(),
        args: vec!["--background".into(), "--home".into(), "C:/My Profile".into()],
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
