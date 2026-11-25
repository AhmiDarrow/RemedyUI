//! `cargo remedy new`: an app from a template. The templates live in `remedy-templates`,
//! shared with the developer tools' `new_app`.

use std::path::PathBuf;

use cargo_remedy::templates::{Dependency, title};

use crate::flag_value;

pub fn run(args: &[String]) -> Result<(), String> {
    let name = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("usage: cargo remedy new <name> [--template <t>]")?;
    let template = flag_value(args, "--template").unwrap_or_else(|| "basic".into());
    let dir = flag_value(args, "--path").map_or_else(|| PathBuf::from(name), PathBuf::from);
    let remedy = match flag_value(args, "--remedy-path") {
        Some(p) => {
            Dependency::Path(std::fs::canonicalize(&p).map_err(|e| format!("--remedy-path {p}: {e}"))?)
        }
        None => Dependency::Git(cargo_remedy::templates::REPOSITORY.to_owned()),
    };
    cargo_remedy::templates::create(&dir, name, &template, &remedy).map_err(|e| e.to_string())?;
    println!("Created {} ({template}) in {}", title(name), dir.display());
    println!("  cd {}", dir.display());
    println!("  cargo remedy run        # F12 in a window opens the inspector");
    println!("  cargo test              # the headless test and the preview's golden images");
    println!("  cargo remedy package    # installer and portable zip");
    Ok(())
}
