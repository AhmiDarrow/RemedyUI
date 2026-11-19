//! Running Cargo and other programs, with their output kept.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::{Result, ToolError};

/// What a program did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ran {
    /// It exited with success.
    pub ok: bool,
    /// Its exit code, when it had one.
    pub code: Option<i32>,
    /// What it wrote to standard output.
    pub stdout: String,
    /// What it wrote to standard error.
    pub stderr: String,
    /// How long it ran.
    pub took: Duration,
    /// It was stopped for running past its time.
    pub timed_out: bool,
}

/// The Cargo to run: the one running us when Cargo started us, else `cargo` on the path.
pub(crate) fn cargo() -> PathBuf {
    std::env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from)
}

/// Runs `cmd` in `dir`, stopping it after `timeout` when one is given. Only the process this
/// started is ever stopped.
pub(crate) fn run(mut cmd: Command, dir: &Path, timeout: Option<Duration>) -> Result<Ran> {
    cmd.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let shown = format!("{cmd:?}");
    let start = Instant::now();
    let mut child = cmd.spawn().map_err(|e| ToolError::new(format!("could not start {shown}: {e}")))?;
    // Both pipes are drained on their own threads so a chatty program never blocks on a full
    // pipe while we wait.
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(e) => return Err(ToolError::new(format!("waiting for {shown}: {e}"))),
        }
        if timeout.is_some_and(|t| start.elapsed() > t) {
            let _ = child.kill();
            timed_out = true;
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = out.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
    Ok(Ran {
        ok: !timed_out && status.is_some_and(|s| s.success()),
        code: status.and_then(|s| s.code()),
        stdout,
        stderr,
        took: start.elapsed(),
        timed_out,
    })
}

fn drain(mut r: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Compiler errors in Cargo's output, each with its location line (`--> file:line:col`).
pub(crate) fn compile_errors(stderr: &str) -> Vec<String> {
    let lines: Vec<&str> = stderr.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("error")
            && !l.starts_with("error: could not compile")
            && !l.starts_with("error: aborting")
        {
            let at = lines
                .iter()
                .skip(i + 1)
                .take(4)
                .find_map(|n| n.trim_start().strip_prefix("--> "))
                .map(|a| format!(" at {a}"))
                .unwrap_or_default();
            out.push(format!("{l}{at}"));
        }
    }
    out
}

/// The `target` folder Cargo uses for the package or workspace at `dir`.
pub(crate) fn target_dir(dir: &Path) -> Result<PathBuf> {
    let mut cmd = Command::new(cargo());
    cmd.args(["metadata", "--no-deps", "--format-version", "1"]);
    let ran = run(cmd, dir, None)?;
    if !ran.ok {
        return Err(ToolError::new(format!("cargo metadata in {}: {}", dir.display(), ran.stderr.trim())));
    }
    let json: serde_json::Value = serde_json::from_str(&ran.stdout)
        .map_err(|e| ToolError::new(format!("cargo metadata output: {e}")))?;
    json.get("target_directory")
        .and_then(|t| t.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| ToolError::new("cargo metadata named no target directory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_errors_carry_their_location() {
        let stderr = "   Compiling app v0.1.0\nerror[E0425]: cannot find value `x` in this scope\n --> src/lib.rs:3:5\n  |\nerror: could not compile `app`\n";
        assert_eq!(
            compile_errors(stderr),
            vec!["error[E0425]: cannot find value `x` in this scope at src/lib.rs:3:5".to_owned()]
        );
    }

    #[test]
    fn programs_run_and_report() {
        let mut version = Command::new(cargo());
        version.arg("--version");
        let ran = run(version, Path::new("."), Some(Duration::from_secs(60)));
        assert!(ran.is_ok_and(|r| r.ok && r.stdout.starts_with("cargo")));
        assert!(run(Command::new("remedy-tools-no-such-program"), Path::new("."), None).is_err());
    }
}
