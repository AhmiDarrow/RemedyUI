//! `cargo remedy new` writes complete apps, refuses bad names and templates that are not here
//! yet. Building the generated apps is checked by `generated_apps_build_and_pass_their_tests`
//! (run with `--ignored`: it compiles the engine three times).

use std::path::PathBuf;
use std::process::Command;

fn cargo_remedy() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cargo-remedy"))
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("remedy-new-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

#[test]
fn every_template_writes_a_complete_app() {
    for t in ["basic", "tray", "multi-window"] {
        let dir = temp(t);
        let out = cargo_remedy()
            .args(["remedy", "new", "my-notes", "--template", t, "--path"])
            .arg(&dir)
            .arg("--remedy-path")
            .arg(workspace())
            .output();
        let Ok(out) = out else { unreachable!("cargo-remedy runs") };
        assert!(out.status.success(), "{t}: {}", String::from_utf8_lossy(&out.stderr));
        for f in [
            "Cargo.toml",
            "src/lib.rs",
            "src/main.rs",
            "tests/app.rs",
            "remedy.toml",
            "assets/icon.png",
            ".gitignore",
            "README.md",
        ] {
            assert!(dir.join(f).is_file(), "{t}: {f}");
        }
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
        assert!(cargo.contains("name = \"my-notes\"") && cargo.contains("crates/remedy-ui\""), "{cargo}");
        assert!(!cargo.contains(r"\\?\") && !cargo.contains("//?/"), "plain paths: {cargo}");
        let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap_or_default();
        assert!(lib.contains("My Notes") && !lib.contains("{{"), "{t}: placeholders filled");
        let test = std::fs::read_to_string(dir.join("tests/app.rs")).unwrap_or_default();
        assert!(test.contains("my_notes::") && !test.contains("{{"), "{t}: {test}");
        let main = std::fs::read_to_string(dir.join("src/main.rs")).unwrap_or_default();
        assert_eq!(main.contains(".single_instance()"), t == "tray", "{t}");
        // A second time into the same place is refused.
        let again =
            cargo_remedy().args(["remedy", "new", "my-notes", "--template", t, "--path"]).arg(&dir).output();
        assert!(again.is_ok_and(|o| !o.status.success()), "{t}: no overwrite");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn bad_names_and_later_templates_are_explained() {
    let run = |args: &[&str]| {
        let o = cargo_remedy().args(args).output();
        let Ok(o) = o else { unreachable!("cargo-remedy runs") };
        (o.status.success(), String::from_utf8_lossy(&o.stderr).to_string())
    };
    let bad = temp("bad").display().to_string();
    let (ok, err) = run(&["remedy", "new", "9lives", "--path", &bad]);
    assert!(!ok && err.contains("not a package name"), "{err}");
    let a = temp("a").display().to_string();
    let (ok, err) = run(&["remedy", "new", "phone", "--template", "android", "--path", &a]);
    assert!(!ok && err.contains("M7"), "{err}");
    let n = temp("n").display().to_string();
    let (ok, err) = run(&["remedy", "new", "x", "--template", "nope", "--path", &n]);
    assert!(!ok && err.contains("basic") && err.contains("tray"), "{err}");
    assert!(cargo_remedy().arg("--help").output().is_ok_and(|o| o.status.success()));
}

#[test]
#[ignore = "builds the engine for three generated apps"]
fn generated_apps_build_and_pass_their_tests() {
    for t in ["basic", "tray", "multi-window"] {
        let dir = temp(&format!("build-{t}"));
        let ok = cargo_remedy()
            .args(["remedy", "new", "gen-app", "--template", t, "--path"])
            .arg(&dir)
            .arg("--remedy-path")
            .arg(workspace())
            .status();
        assert!(ok.is_ok_and(|s| s.success()));
        let test = Command::new(env!("CARGO")).arg("test").current_dir(&dir).status();
        assert!(test.is_ok_and(|s| s.success()), "{t}: cargo test");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
