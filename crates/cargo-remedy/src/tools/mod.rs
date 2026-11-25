//! The developer-tool commands over `remedy-tools`: everything the `remedy-mcp` server offers
//! agents, for people and scripts. Readable output by default, `--json` for scripts.
//!
//! - `cargo remedy docs <query>` / `docs read <path>` / `docs list`
//! - `cargo remedy widgets [name] [--feature <f>]`
//! - `cargo remedy rui check|compile <file>` / `rui elements`
//! - `cargo remedy rfx check|render <file or built-in>` / `rfx library`
//! - `cargo remedy preview <function> [--png <file>]` / `audit <function>`
//! - `cargo remedy test [filter]` / `budgets [scene]`
//! - `cargo remedy app connect|snapshot|act|command|undo`
//! - `cargo remedy devtools tree|element|semantics|signals|updates|timings`

mod app;
mod docs;
mod preview;
mod rfx;
mod rui;
mod testing;

use std::collections::BTreeMap;

/// The commands this module answers.
const COMMANDS: &[&str] =
    &["docs", "widgets", "rui", "rfx", "preview", "previews", "audit", "test", "budgets", "app", "devtools"];

/// Help for these commands, shown with cargo remedy's own.
pub const HELP: &str = "\
DEVELOPER TOOLS (the same tools remedy-mcp offers agents; add --json for scripts):
    cargo remedy docs <query> [--kind guide|rustdoc] [--limit <n>]    search the docs
    cargo remedy docs read <path> [--heading <h>] [--from <line>] [--lines <n>]
    cargo remedy docs list                                           every document
    cargo remedy widgets [name] [--feature <feature>]                the widget catalog
    cargo remedy rui check <file> [--elements <specs.json>]          check .rui markup
    cargo remedy rui compile <file> [--elements <specs.json>] [--out <file.rs>]
    cargo remedy rui elements                                        the built-in elements
    cargo remedy rfx check <file.rfx|name> [--untrusted]             check a Prism effect: problems
                                                                     under their lines, cost by pass
    cargo remedy rfx render <file.rfx|name> --png <file> [--size <w>x<h>] [--scale <s>]
                     [--time <t>] [--set name=v[,v..]]... [--background #rrggbb|none]
    cargo remedy rfx library                                         the built-in effects
    cargo remedy preview <function> [--png <file>] [--size <w>x<h>] [--scale <s>]
                         [--theme dark|light|<palette>] [--locale <tag>] [--text-scale <s>]
                         [--app <dir>]                               render a preview
    cargo remedy audit <function> [same options]                     accessibility audit
    cargo remedy previews [--app <dir>]                              the app's #[preview] functions
    cargo remedy test [filter] [--release] [--include-ignored] [-p <package>] [--app <dir>]
    cargo remedy budgets [scene] [--list] [--remedy-root <dir>]      the engine's budget scenes
    cargo remedy app connect|snapshot --address <a> --token <t>      a running app (with
    cargo remedy app act <node> <action> [value] --address <a> --token <t> [--wait <s>]
    cargo remedy app command <id> --address <a> --token <t> [--wait <s>]
    cargo remedy app undo --address <a> --token <t>                    the person's grant)
    cargo remedy devtools tree|semantics|updates|timings --address <a> --token <t>
    cargo remedy devtools element <id> | signals [id] --address <a> --token <t>
    (--address/--token default to REMEDY_APP_ADDRESS/REMEDY_APP_TOKEN, and for devtools to
    REMEDY_DEVTOOLS_ADDRESS/REMEDY_DEVTOOLS_TOKEN; --docs-root <dir> reads a checkout's docs)
";

/// Whether `cmd` is one of these commands.
pub fn handles(cmd: &str) -> bool {
    COMMANDS.contains(&cmd)
}

/// Runs command `cmd` with `args`.
pub fn run(cmd: &str, args: &[String]) -> Result<(), String> {
    let a = Args::parse(args)?;
    match cmd {
        "docs" => docs::docs(&a),
        "widgets" => docs::widgets(&a),
        "rui" => rui::rui(&a),
        "rfx" => rfx::rfx(&a),
        "preview" => preview::preview(&a, false),
        "audit" => preview::preview(&a, true),
        "previews" => preview::previews(&a),
        "test" => testing::test(&a),
        "budgets" => testing::budgets(&a),
        "app" => app::app(&a),
        "devtools" => app::devtools(&a),
        other => Err(format!("unknown command `{other}`")),
    }
}

/// Flags that take no value.
const SWITCHES: &[&str] = &["--json", "--release", "--include-ignored", "--list", "--untrusted"];

/// Parsed arguments: positionals and `--flag value` pairs.
#[derive(Debug, Default)]
pub struct Args {
    /// Positional arguments, in order.
    pub positional: Vec<String>,
    flags: BTreeMap<String, String>,
    /// Every `--flag value` in order, for flags given more than once.
    repeated: Vec<(String, String)>,
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, String> {
        let mut a = Args::default();
        let mut i = 0;
        while let Some(arg) = args.get(i) {
            let flag = match arg.as_str() {
                "-p" => "--package".to_owned(),
                s if s.starts_with("--") => s.to_owned(),
                _ => {
                    a.positional.push(arg.clone());
                    i += 1;
                    continue;
                }
            };
            if SWITCHES.contains(&flag.as_str()) {
                a.flags.insert(flag, String::new());
            } else {
                let Some(v) = args.get(i + 1) else { return Err(format!("{arg} needs a value")) };
                a.repeated.push((flag.clone(), v.clone()));
                a.flags.insert(flag, v.clone());
                i += 1;
            }
            i += 1;
        }
        Ok(a)
    }

    /// Every value given for `--flag`, in order.
    pub fn all(&self, flag: &str) -> Vec<String> {
        self.repeated.iter().filter(|(f, _)| f == flag).map(|(_, v)| v.clone()).collect()
    }

    /// The value of `--flag`.
    pub fn get(&self, flag: &str) -> Option<String> {
        self.flags.get(flag).cloned()
    }

    /// Whether the switch `--flag` was given.
    pub fn on(&self, flag: &str) -> bool {
        self.flags.contains_key(flag)
    }

    /// The value of `--flag` as a number.
    pub fn number<T: std::str::FromStr>(&self, flag: &str) -> Result<Option<T>, String> {
        self.get(flag)
            .map(|v| v.parse::<T>().map_err(|_| format!("{flag} takes a number, not `{v}`")))
            .transpose()
    }

    /// Positional `i`.
    pub fn pos(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(String::as_str)
    }

    /// Output `value` as JSON when `--json` was given, else `text`.
    pub fn print(&self, text: &str, value: &impl serde::Serialize) -> Result<(), String> {
        if self.on("--json") {
            let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
            println!("{json}");
        } else {
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
        }
        Ok(())
    }
}

/// A tool error as a command's error.
pub fn err(e: cargo_remedy::devkit::ToolError) -> String {
    e.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_split_into_positionals_flags_and_switches() {
        let args: Vec<String> = ["read", "docs/DESIGN.md", "--heading", "Crates", "--json", "-p", "app"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let a = Args::parse(&args).unwrap_or_default();
        assert_eq!(a.positional, vec!["read".to_owned(), "docs/DESIGN.md".to_owned()]);
        assert_eq!(a.get("--heading").as_deref(), Some("Crates"));
        assert_eq!(a.get("--package").as_deref(), Some("app"));
        assert!(a.on("--json"));
        assert!(Args::parse(&["--heading".to_owned()]).is_err());
        assert!(a.number::<u32>("--heading").is_err());
        assert!(handles("docs") && !handles("new"));
    }
}
