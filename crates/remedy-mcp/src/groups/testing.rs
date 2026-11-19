//! run_tests, budget_check.

use remedy_tools::testing::{BudgetInput, TestInput, budget_check, run_tests};
use remedy_tools::{Group, ToolError};
use serde_json::Value;

use crate::{Context, Reply, ToolDef, ToolGroup, args};

pub(crate) struct Testing;

impl ToolGroup for Testing {
    fn group(&self) -> Group {
        Group::Testing
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::of::<TestInput>(
                "run_tests",
                "Run `cargo test` for an app in the workspace (optionally one package, tests matching a filter, release mode, ignored tests too) and return a structured summary: \
                 counts, every test with its outcome, each failure's output (panic message, golden-image differences), and compiler errors with their file:line:column.",
                false,
            ),
            ToolDef::of::<BudgetInput>(
                "budget_check",
                "Run the RemedyUI engine's budget scenes in release builds (the reference scenes held to frame-time and memory budgets: the 50,000-file explorer, \
                 the 300-page zoom, animation frame budgets, …) and report which met their budget, with their measurements. \
                 Only when asked: it builds the engine in release mode. `list_only` lists the scenes without running them.",
                false,
            ),
        ]
    }

    fn call(&self, cx: &Context<'_>, name: &str, a: Value) -> Result<Reply, ToolError> {
        match name {
            "run_tests" => {
                let input: TestInput = args(a)?;
                let out = run_tests(&input, &cx.config.workspace)?;
                Ok(Reply::new(out.text(), &out))
            }
            "budget_check" => {
                let input: BudgetInput = args(a)?;
                let root = match &cx.config.remedy_root {
                    Some(r) => r.clone(),
                    None => remedy_tools::built_from_checkout().ok_or_else(|| {
                        ToolError::new("no RemedyUI checkout to run the budget scenes in: start the server with --remedy-root")
                    })?,
                };
                let runs = budget_check(&input, &root)?;
                let mut text = String::new();
                for r in &runs {
                    let state = if input.list_only {
                        "scene"
                    } else if r.passed {
                        "met"
                    } else {
                        "MISSED"
                    };
                    text.push_str(&format!(
                        "{state}: {} ({}/tests/{}.rs) {}\n",
                        r.scene.function, r.scene.krate, r.scene.test, r.scene.about
                    ));
                    if !input.list_only {
                        text.push_str(&format!(
                            "  {:.1} s\n  {}\n",
                            r.seconds,
                            r.output.replace('\n', "\n  ")
                        ));
                    }
                }
                Ok(Reply::new(text, &runs))
            }
            other => Err(ToolError::new(format!("no tool `{other}` in the testing group"))),
        }
    }
}
