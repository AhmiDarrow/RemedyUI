//! `cargo remedy run`: build and run, with devtools in debug builds, and optionally rebuild
//! and restart on every change.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, SystemTime};

use crate::project::{Project, cargo, status};

pub fn run(args: &[String]) -> Result<(), String> {
    let release = args.iter().any(|a| a == "--release");
    let watch = args.iter().any(|a| a == "--watch");
    let app_args: Vec<String> = args.iter().skip_while(|a| *a != "--").skip(1).cloned().collect();
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let project = Project::load(&root)?;
    if !watch {
        let mut cmd = build_cmd(&project, release, "run");
        if !app_args.is_empty() {
            cmd.arg("--").args(&app_args);
        }
        return status(&mut cmd);
    }
    // Watch: build, start, and on every change rebuild and restart.
    let mut child: Option<Child> = None;
    let mut seen = snapshot(&root);
    loop {
        if status(&mut build_cmd(&project, release, "build")).is_ok() {
            if let Some(mut c) = child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            match Command::new(project.exe(release)).args(&app_args).current_dir(&root).spawn() {
                Ok(c) => child = Some(c),
                Err(e) => eprintln!("cannot start {}: {e}", project.exe(release).display()),
            }
            println!("[remedy] running; watching for changes (Ctrl+C to stop)");
        } else {
            println!("[remedy] build failed; fix it and save to try again");
        }
        // Wait for a change that settles (editors save in bursts).
        loop {
            std::thread::sleep(Duration::from_millis(400));
            if let Some(c) = child.as_mut()
                && let Ok(Some(_)) = c.try_wait()
            {
                // The app was closed: stop watching too.
                println!("[remedy] the app exited");
                return Ok(());
            }
            let now = snapshot(&root);
            if now != seen {
                std::thread::sleep(Duration::from_millis(200));
                seen = snapshot(&root);
                println!("[remedy] change seen; rebuilding");
                break;
            }
        }
    }
}

fn build_cmd(project: &Project, release: bool, verb: &str) -> Command {
    let mut cmd = Command::new(cargo());
    cmd.arg(verb).arg("--bin").arg(&project.bin).current_dir(&project.root);
    if release {
        cmd.arg("--release");
    } else {
        // Debug builds carry the inspector (F12).
        cmd.args(["--features", "remedy-ui/devtools"]);
    }
    cmd
}

/// Modification times and sizes of the files a rebuild depends on (size too: two saves in
/// the same timestamp tick on a coarse file system still differ).
fn snapshot(root: &Path) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut out = Vec::new();
    for top in ["src", "assets", "Cargo.toml", "remedy.toml", "build.rs"] {
        let mut stack = vec![root.join(top)];
        while let Some(p) = stack.pop() {
            let Ok(meta) = std::fs::metadata(&p) else { continue };
            if meta.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&p) {
                    stack.extend(rd.flatten().map(|e| e.path()));
                }
            } else if let Ok(m) = meta.modified() {
                out.push((p, m, meta.len()));
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(root: &Path) -> Project {
        Project {
            root: root.to_path_buf(),
            bin: "notes".into(),
            version: "0.1.0".into(),
            target_dir: root.join("target"),
            name: "Notes".into(),
            id: "app.remedy.notes".into(),
            publisher: String::new(),
            description: String::new(),
            icon: None,
            sign_thumbprint: None,
            timestamp_url: String::new(),
        }
    }

    #[test]
    fn edits_to_sources_assets_and_manifests_are_seen_and_builds_are_not() {
        let root = std::env::temp_dir().join(format!("remedy-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["src", "assets", "target/debug"] {
            let _ = std::fs::create_dir_all(root.join(d));
        }
        let _ = std::fs::write(root.join("src/lib.rs"), "fn a() {}");
        let _ = std::fs::write(root.join("Cargo.toml"), "[package]");
        let before = snapshot(&root);
        assert_eq!(before.len(), 2);
        // Build output changing is not a change.
        let _ = std::fs::write(root.join("target/debug/app.exe"), "binary");
        assert_eq!(snapshot(&root), before);
        // A new asset is; so is an edit.
        let _ = std::fs::write(root.join("assets/icon.png"), "png");
        let with_asset = snapshot(&root);
        assert_ne!(with_asset, before);
        let _ = std::fs::write(root.join("src/lib.rs"), "fn a() { let _ = 1; }");
        let edited = snapshot(&root);
        assert_ne!(edited, with_asset, "an edit is seen");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn debug_runs_carry_devtools_and_release_runs_do_not() {
        let p = project(Path::new("."));
        let args = |c: &Command| c.get_args().map(|a| a.to_string_lossy().to_string()).collect::<Vec<_>>();
        let debug = args(&build_cmd(&p, false, "run"));
        assert_eq!(debug, ["run", "--bin", "notes", "--features", "remedy-ui/devtools"]);
        let release = args(&build_cmd(&p, true, "build"));
        assert_eq!(release, ["build", "--bin", "notes", "--release"]);
    }
}
