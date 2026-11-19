//! Tests, summarized: an app's `cargo test`, and the engine's budget scenes (the reference
//! scenes held to frame and memory budgets in release builds).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::process::{cargo, compile_errors, run};
use crate::{Result, ToolError, Workspace};

/// Which tests to run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct TestInput {
    /// The app's (or workspace's) folder, inside the workspace.
    pub app: String,
    /// Only tests whose name contains this.
    #[serde(default)]
    pub filter: Option<String>,
    /// Only this package of a workspace.
    #[serde(default)]
    pub package: Option<String>,
    /// Build in release mode (what budget tests need).
    #[serde(default)]
    pub release: bool,
    /// Run ignored tests too.
    #[serde(default)]
    pub include_ignored: bool,
    /// Stop after this many seconds (no limit when absent).
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

impl TestInput {
    /// Every test of the app in `app`.
    pub fn new(app: impl Into<String>) -> Self {
        TestInput { app: app.into(), ..Default::default() }
    }
}

/// How a test ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Outcome {
    /// It passed.
    Passed,
    /// It failed.
    Failed,
    /// It was ignored.
    Ignored,
}

/// One test.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct TestCase {
    /// The test target it is in (`tests/app.rs`, `unittests src/lib.rs`, `Doc-tests app`).
    pub target: String,
    /// Its name.
    pub name: String,
    /// How it ended.
    pub outcome: Outcome,
    /// What it printed, for failures.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
}

/// A test run, summarized.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct TestSummary {
    /// Everything built and every test passed.
    pub success: bool,
    /// Tests passed.
    pub passed: usize,
    /// Tests failed.
    pub failed: usize,
    /// Tests ignored.
    pub ignored: usize,
    /// Every test that ran or was ignored.
    pub tests: Vec<TestCase>,
    /// Compiler errors, when it did not build, each with its location.
    pub compile_errors: Vec<String>,
    /// It ran out of time.
    pub timed_out: bool,
    /// How long it took, in seconds.
    pub seconds: f64,
    /// The end of Cargo's own output when the run failed for a reason not shown above.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl TestSummary {
    /// A few lines for people: the counts, then each failure with its output.
    pub fn text(&self) -> String {
        let mut out = format!(
            "{}: {} passed, {} failed, {} ignored in {:.1} s\n",
            if self.success { "ok" } else { "FAILED" },
            self.passed,
            self.failed,
            self.ignored,
            self.seconds
        );
        if self.timed_out {
            out.push_str("stopped: it ran out of time\n");
        }
        for e in &self.compile_errors {
            out.push_str(&format!("{e}\n"));
        }
        for t in self.tests.iter().filter(|t| t.outcome == Outcome::Failed) {
            out.push_str(&format!("\n--- {} ({})\n{}\n", t.name, t.target, t.output.trim_end()));
        }
        if !self.note.is_empty() {
            out.push_str(&format!("\n{}\n", self.note));
        }
        out
    }
}

/// Reads libtest's output (`cargo test`'s standard output and error together).
pub fn parse(stdout: &str, stderr: &str) -> (Vec<TestCase>, bool) {
    let mut tests: Vec<TestCase> = Vec::new();
    let mut target = String::new();
    // Cargo names each test binary on standard error ("Running tests/app.rs (…)") in the
    // order their results appear on standard output.
    let mut targets: Vec<String> = stderr
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            t.strip_prefix("Running ")
                .map(|r| r.split(" (").next().unwrap_or(r).to_owned())
                .or_else(|| t.strip_prefix("Doc-tests ").map(|r| format!("Doc-tests {r}")))
        })
        .collect();
    targets.reverse();
    let mut saw_results = false;
    let mut failure: Option<(String, String)> = None;
    let finish = |tests: &mut Vec<TestCase>, f: Option<(String, String)>| {
        if let Some((name, text)) = f
            && let Some(t) = tests.iter_mut().rev().find(|t| t.name == name && t.outcome == Outcome::Failed)
        {
            t.output = text.trim_end().to_owned();
        }
    };
    for line in stdout.lines() {
        if line.starts_with("running ") && (line.ends_with(" tests") || line.ends_with(" test")) {
            target = targets.pop().unwrap_or_default();
            continue;
        }
        if let Some(rest) = line.strip_prefix("---- ")
            && let Some(name) =
                rest.strip_suffix(" stdout ----").or_else(|| rest.strip_suffix(" stderr ----"))
        {
            finish(&mut tests, failure.take());
            failure = Some((name.to_owned(), String::new()));
            continue;
        }
        if let Some((_, text)) = failure.as_mut() {
            if line == "failures:" || line.starts_with("test result:") {
                finish(&mut tests, failure.take());
            } else {
                text.push_str(line);
                text.push('\n');
                continue;
            }
        }
        if line.starts_with("test result:") {
            saw_results = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("test ")
            && let Some((name, result)) = rest.rsplit_once(" ... ")
        {
            let outcome = if result.starts_with("ok") {
                Outcome::Passed
            } else if result.starts_with("FAILED") {
                Outcome::Failed
            } else if result.starts_with("ignored") {
                Outcome::Ignored
            } else {
                continue;
            };
            tests.push(TestCase {
                target: target.clone(),
                name: name.to_owned(),
                outcome,
                output: String::new(),
            });
        }
    }
    finish(&mut tests, failure.take());
    (tests, saw_results)
}

fn summarize(ran: &crate::process::Ran) -> TestSummary {
    let (tests, saw_results) = parse(&ran.stdout, &ran.stderr);
    let count = |o: Outcome| tests.iter().filter(|t| t.outcome == o).count();
    let (passed, failed, ignored) = (count(Outcome::Passed), count(Outcome::Failed), count(Outcome::Ignored));
    let compile_errors = compile_errors(&ran.stderr);
    let note = if !ran.ok && failed == 0 && compile_errors.is_empty() && !ran.timed_out {
        let tail: Vec<&str> = ran.stderr.lines().rev().take(15).collect();
        tail.into_iter().rev().collect::<Vec<_>>().join("\n")
    } else {
        String::new()
    };
    TestSummary {
        success: ran.ok && failed == 0 && (saw_results || compile_errors.is_empty()),
        passed,
        failed,
        ignored,
        tests,
        compile_errors,
        timed_out: ran.timed_out,
        seconds: ran.took.as_secs_f64(),
        note,
    }
}

/// Runs `cargo test` for the app and summarizes it.
pub fn run_tests(input: &TestInput, ws: &Workspace) -> Result<TestSummary> {
    let dir = ws.resolve(&input.app)?;
    if !dir.join("Cargo.toml").is_file() {
        return Err(ToolError::new(format!("{} has no Cargo.toml", dir.display())));
    }
    let mut cmd = Command::new(cargo());
    cmd.args(["test", "--no-fail-fast", "--color", "never"]);
    if input.release {
        cmd.arg("--release");
    }
    if let Some(p) = &input.package {
        cmd.args(["-p", p]);
    }
    cmd.arg("--");
    if let Some(f) = &input.filter {
        cmd.arg(f);
    }
    if input.include_ignored {
        cmd.arg("--include-ignored");
    }
    cmd.args(["--color", "never"]);
    let ran = run(cmd, &dir, input.timeout_seconds.map(Duration::from_secs))?;
    Ok(summarize(&ran))
}

/// One of the engine's budget scenes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Scene {
    /// The crate holding it.
    pub krate: String,
    /// Its test file's name (`zoom_budget`).
    pub test: String,
    /// The test function.
    pub function: String,
    /// What it holds the engine to (its doc comment).
    pub about: String,
}

/// Which budget scenes to run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct BudgetInput {
    /// Only scenes whose crate, test or function contains this; all when absent.
    #[serde(default)]
    pub scene: Option<String>,
    /// Only list the scenes, without running them.
    #[serde(default)]
    pub list_only: bool,
    /// Stop each scene after this many seconds (no limit when absent).
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// A scene's run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct SceneRun {
    /// The scene.
    pub scene: Scene,
    /// It met its budget.
    pub passed: bool,
    /// What it printed (its measurements, or why it failed).
    pub output: String,
    /// How long building and running it took, in seconds.
    pub seconds: f64,
}

/// The engine's budget scenes: tests in `crates/*/tests` named for a budget, or marked as
/// measured in release builds.
pub fn scenes(root: &Path) -> Vec<Scene> {
    let mut out = Vec::new();
    let Ok(crates) = std::fs::read_dir(root.join("crates")) else { return out };
    let mut dirs: Vec<_> = crates.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        let Some(krate) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let Ok(files) = std::fs::read_dir(dir.join("tests")) else { continue };
        let mut files: Vec<_> =
            files.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "rs")).collect();
        files.sort();
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let test = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let mut docs: Vec<&str> = Vec::new();
            let mut attrs: Vec<&str> = Vec::new();
            for line in text.lines() {
                let t = line.trim();
                if let Some(d) = t.strip_prefix("///") {
                    docs.push(d.trim());
                } else if t.starts_with("#[") {
                    attrs.push(t);
                } else if let Some(rest) = t.strip_prefix("fn ") {
                    let function: String =
                        rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    let is_test = attrs.contains(&"#[test]");
                    let budget = function.contains("budget")
                        || attrs.iter().any(|a| a.contains("budget") || a.contains("measured in release"));
                    if is_test && budget {
                        out.push(Scene {
                            krate: krate.clone(),
                            test: test.clone(),
                            function,
                            about: docs.join(" "),
                        });
                    }
                    docs.clear();
                    attrs.clear();
                } else if !t.is_empty() {
                    docs.clear();
                    attrs.clear();
                }
            }
        }
    }
    out
}

/// Runs the budget scenes of the RemedyUI checkout at `root`, each in a release build.
pub fn budget_check(input: &BudgetInput, root: &Path) -> Result<Vec<SceneRun>> {
    if !root.join("crates").join("remedy-ui").join("Cargo.toml").is_file() {
        return Err(ToolError::new(format!("{} is not a RemedyUI checkout", root.display())));
    }
    let wanted: Vec<Scene> = scenes(root)
        .into_iter()
        .filter(|s| {
            input.scene.as_ref().is_none_or(|w| {
                s.krate.contains(w.as_str()) || s.test.contains(w.as_str()) || s.function.contains(w.as_str())
            })
        })
        .collect();
    if wanted.is_empty() {
        return Err(ToolError::new(match &input.scene {
            Some(w) => format!("no budget scene matches `{w}`"),
            None => "the checkout has no budget scenes".to_owned(),
        }));
    }
    if input.list_only {
        return Ok(wanted
            .into_iter()
            .map(|scene| SceneRun { scene, passed: false, output: String::new(), seconds: 0.0 })
            .collect());
    }
    let mut runs = Vec::new();
    for scene in wanted {
        let mut cmd = Command::new(cargo());
        cmd.args(["test", "--release", "--color", "never", "-p", &scene.krate, "--test", &scene.test, "--"])
            .args(["--exact", &scene.function, "--include-ignored", "--nocapture"]);
        let ran = run(cmd, root, input.timeout_seconds.map(Duration::from_secs))?;
        let summary = summarize(&ran);
        let mut output = ran.stdout.trim().to_owned();
        if !summary.compile_errors.is_empty() {
            output = summary.compile_errors.join("\n");
        } else if ran.timed_out {
            output.push_str("\nstopped: it ran out of time");
        }
        runs.push(SceneRun {
            scene,
            passed: summary.success && summary.passed > 0,
            output,
            seconds: summary.seconds,
        });
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STDOUT: &str = "
running 2 tests
test the_app_opens ... ok
test previews_match ... FAILED

failures:

---- previews_match stdout ----
thread 'previews_match' panicked at tests/app.rs:20:5:
12 pixels differ

failures:
    previews_match

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s


running 1 test
test slow ... ignored, needs a GPU

test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s
";
    const STDERR: &str = "   Compiling app v0.1.0\n    Finished `test` profile\n     Running tests/app.rs (target/debug/deps/app-1)\n     Running tests/gpu.rs (target/debug/deps/gpu-2)\n";

    #[test]
    fn libtest_output_becomes_cases_with_failure_output() {
        let (tests, saw) = parse(STDOUT, STDERR);
        assert!(saw);
        assert_eq!(tests.len(), 3);
        let Some(failed) = tests.iter().find(|t| t.outcome == Outcome::Failed) else {
            unreachable!("one failed")
        };
        assert_eq!((failed.name.as_str(), failed.target.as_str()), ("previews_match", "tests/app.rs"));
        assert!(failed.output.contains("12 pixels differ"), "{}", failed.output);
        assert!(
            tests
                .iter()
                .any(|t| t.name == "slow" && t.outcome == Outcome::Ignored && t.target == "tests/gpu.rs")
        );
        let ran = crate::process::Ran {
            ok: false,
            code: Some(101),
            stdout: STDOUT.into(),
            stderr: STDERR.into(),
            took: Duration::from_millis(500),
            timed_out: false,
        };
        let s = summarize(&ran);
        assert!(!s.success && s.passed == 1 && s.failed == 1 && s.ignored == 1);
        assert!(s.text().contains("--- previews_match (tests/app.rs)"));
    }

    #[test]
    fn compile_failures_are_reported_as_such() {
        let ran = crate::process::Ran {
            ok: false,
            code: Some(101),
            stdout: String::new(),
            stderr:
                "error[E0425]: cannot find value `x`\n --> src/lib.rs:3:5\nerror: could not compile `app`\n"
                    .into(),
            took: Duration::ZERO,
            timed_out: false,
        };
        let s = summarize(&ran);
        assert!(!s.success);
        assert_eq!(s.compile_errors.len(), 1);
    }

    #[test]
    fn the_engine_budget_scenes_are_found() {
        let Some(root) = crate::built_from_checkout() else { return };
        let found = scenes(&root);
        assert!(
            found.iter().any(|s| s.krate == "remedy-ui-doc-pages" && s.test == "zoom_budget"),
            "{found:?}"
        );
        assert!(
            found.iter().any(|s| s.function == "the_fifty_thousand_file_explorer_meets_its_frame_budget")
        );
        let listed = budget_check(
            &BudgetInput { scene: Some("zoom".into()), list_only: true, timeout_seconds: None },
            &root,
        );
        assert!(listed.is_ok_and(|l| {
            l.iter().all(|r| r.scene.test.contains("zoom") || r.scene.function.contains("zoom"))
        }));
        assert!(
            budget_check(&BudgetInput { scene: Some("no-such-scene".into()), ..Default::default() }, &root)
                .is_err()
        );
    }
}
