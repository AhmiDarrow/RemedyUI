//! `cargo remedy`: create, run and package RemedyUI apps.
//!
//! - `cargo remedy new <name> [--template basic|tray|multi-window|document|android] [--path <dir>]
//!   [--remedy-path <RemedyUI checkout>]`
//! - `cargo remedy run [--release] [--watch] [-- <app arguments>]`
//! - `cargo remedy package [--target windows|linux|macos|android]`
//! - `cargo remedy migrate [--dry-run] [--path <dir>]`

mod ico;
mod migrate;
mod new;
mod package;
mod platforms;
mod project;
mod run;
mod tools;

use std::process::ExitCode;

const HELP: &str = "\
cargo remedy: create, run and package RemedyUI apps

USAGE:
    cargo remedy new <name> [--template <basic|tray|multi-window|document|android>] [--path <dir>]
                            [--remedy-path <path to a RemedyUI checkout>]
    cargo remedy run [--release] [--watch] [-- <app arguments>]
    cargo remedy package [--target <windows|linux|macos|android>]
    cargo remedy migrate [--dry-run] [--path <app directory>]

new       Creates an app: a window, the Remedy theme, a test and a preview, and remedy.toml
          for packaging. Templates: basic (one window), tray (lives in the tray, one
          instance), multi-window (a main window opening document windows), document (a
          paged viewer over page images), android (one app for Android and the desktop).
run       Builds and runs the app. Debug builds include devtools: press F12 in a window for
          the inspector. --watch rebuilds and restarts when sources change.
package   Builds a release and packages it. Windows: the icon and version compiled into the
          executable, a per-user installer (Inno Setup), a portable zip, code signing when
          remedy.toml names a certificate, and minisign update signatures when
          REMEDY_UPDATE_KEY points at a minisign secret key. Linux: a .tar.gz with the
          executable, a desktop entry and the icon. Android: a signed APK (ANDROID_HOME, the
          NDK, build-tools; [android] in remedy.toml). macOS: an .app bundle in a zip (needs
          Apple's linker: a Mac or a cross linker).
migrate   Applies the mechanical fixes for RemedyUI's renames to the app's sources (paths
          resolved through each file's `use` items; strings and comments untouched).
          --dry-run prints the changes as a diff instead.
";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Cargo runs `cargo-remedy remedy <args>`.
    if args.first().is_some_and(|a| a == "remedy") {
        args.remove(0);
    }
    let result = match args.first().map(String::as_str) {
        Some("new") => new::run(args.get(1..).unwrap_or_default()),
        Some("run") => run::run(args.get(1..).unwrap_or_default()),
        Some("package") => package::run(args.get(1..).unwrap_or_default()),
        Some("migrate") => migrate::run(args.get(1..).unwrap_or_default()),
        Some("help" | "--help" | "-h") | None => {
            print!("{HELP}\n{}", tools::HELP);
            Ok(())
        }
        Some(cmd) if tools::handles(cmd) => tools::run(cmd, args.get(1..).unwrap_or_default()),
        Some(other) => Err(format!("unknown command `{other}`\n\n{HELP}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The value after `flag` in `args`, if present.
pub(crate) fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let at = args.iter().position(|a| a == flag)?;
    args.get(at + 1).cloned()
}
