//! `cargo remedy test` and `cargo remedy budgets`.

use std::path::PathBuf;

use cargo_remedy::devkit::Workspace;
use cargo_remedy::devkit::testing::{BudgetInput, TestInput, budget_check, run_tests};

use super::{Args, err};

/// Runs the app's tests, summarized; fails when they do.
pub fn test(a: &Args) -> Result<(), String> {
    let mut input = TestInput::new(a.get("--app").unwrap_or_else(|| ".".into()));
    input.filter = a.pos(0).map(str::to_owned);
    input.package = a.get("--package");
    input.release = a.on("--release");
    input.include_ignored = a.on("--include-ignored");
    input.timeout_seconds = a.number("--timeout")?;
    let out = run_tests(&input, &Workspace::anywhere()).map_err(err)?;
    a.print(&out.text(), &out)?;
    if out.success { Ok(()) } else { Err("the tests did not pass".into()) }
}

/// Runs (or lists) the engine's budget scenes; fails when one misses its budget.
pub fn budgets(a: &Args) -> Result<(), String> {
    let root = match a.get("--remedy-root") {
        Some(r) => PathBuf::from(r),
        None => cargo_remedy::devkit::built_from_checkout()
            .or_else(|| std::env::current_dir().ok())
            .ok_or("name the RemedyUI checkout with --remedy-root")?,
    };
    let mut input = BudgetInput::default();
    input.scene = a.pos(0).map(str::to_owned);
    input.list_only = a.on("--list");
    input.timeout_seconds = a.number("--timeout")?;
    let runs = budget_check(&input, &root).map_err(err)?;
    let mut text = String::new();
    for r in &runs {
        if input.list_only {
            text.push_str(&format!(
                "{} ({}/tests/{}.rs)\n  {}\n",
                r.scene.function, r.scene.krate, r.scene.test, r.scene.about
            ));
        } else {
            let state = if r.passed { "met" } else { "MISSED" };
            text.push_str(&format!(
                "{state}: {} ({:.1} s)\n  {}\n",
                r.scene.function,
                r.seconds,
                r.output.replace('\n', "\n  ")
            ));
        }
    }
    a.print(&text, &runs)?;
    if !input.list_only && runs.iter().any(|r| !r.passed) {
        return Err("a scene missed its budget".into());
    }
    Ok(())
}
