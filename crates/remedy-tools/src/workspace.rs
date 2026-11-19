//! Where tools may read and write.

use std::path::{Component, Path, PathBuf};

use crate::{Result, ToolError};

/// The folders tools may work in. Tools acting for an agent get the folders the person
/// named, and refuse paths outside them; the command line, run by the person, works anywhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    roots: Vec<PathBuf>,
    anywhere: bool,
}

/// `p` without Windows' verbatim prefix (`\\?\C:\…` becomes `C:\…`), which Cargo and other
/// tools do not all accept.
pub(crate) fn plain(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.get(1..2) == Some(":") => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}

impl Workspace {
    /// Tools may work in `roots` (each must exist) and nowhere else. Relative paths are taken
    /// from the first.
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Result<Workspace> {
        let mut out = Vec::new();
        for r in roots {
            let c = std::fs::canonicalize(&r)
                .map_err(|e| ToolError::new(format!("workspace folder {}: {e}", r.display())))?;
            if !c.is_dir() {
                return Err(ToolError::new(format!("workspace {} is not a folder", r.display())));
            }
            out.push(plain(&c));
        }
        if out.is_empty() {
            return Err(ToolError::new("a workspace needs at least one folder"));
        }
        Ok(Workspace { roots: out, anywhere: false })
    }

    /// Anywhere: relative paths are taken from the current directory. For tools the person
    /// runs themselves.
    pub fn anywhere() -> Workspace {
        let cwd = std::env::current_dir().map(|d| plain(&d)).unwrap_or_default();
        Workspace { roots: vec![cwd], anywhere: true }
    }

    /// The folders, the first being where relative paths start.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether any path is allowed.
    pub fn is_anywhere(&self) -> bool {
        self.anywhere
    }

    /// The absolute path `path` names (relative ones from the first folder), refused when it
    /// leads outside the workspace. The path need not exist yet.
    pub fn resolve(&self, path: &str) -> Result<PathBuf> {
        let given = Path::new(path.trim());
        let base = self.roots.first().cloned().unwrap_or_default();
        let joined = if given.is_absolute() { given.to_path_buf() } else { base.join(given) };
        // The deepest part that exists is canonicalized (resolving links); the rest must not
        // climb out with `..`.
        let mut existing = joined.clone();
        let mut rest = Vec::new();
        while !existing.exists() {
            let Some(name) = existing.file_name().map(|n| n.to_os_string()) else { break };
            rest.push(name);
            if !existing.pop() {
                break;
            }
        }
        let mut full = plain(&std::fs::canonicalize(&existing).unwrap_or(existing));
        for part in rest.iter().rev() {
            let p = Path::new(part);
            if p.components()
                .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
            {
                return Err(ToolError::new(format!("{path}: `..` cannot be used past a folder that exists")));
            }
            full.push(p);
        }
        if self.anywhere || self.roots.iter().any(|r| full.starts_with(r)) {
            Ok(full)
        } else {
            let roots: Vec<String> = self.roots.iter().map(|r| r.display().to_string()).collect();
            Err(ToolError::new(format!(
                "{} is outside the workspace ({}); the person decides which folders tools may use",
                full.display(),
                roots.join(", ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_workspace() {
        let base = std::env::temp_dir().join(format!("remedy-tools-ws-{}", std::process::id()));
        let _ = std::fs::create_dir_all(base.join("inner"));
        let Ok(ws) = Workspace::new([base.join("inner")]) else { unreachable!("the folder exists") };
        assert!(ws.resolve("app").is_ok_and(|p| p.ends_with("app")));
        assert!(ws.resolve("new/deeper/app").is_ok());
        assert!(ws.resolve("..").is_err(), "the parent is outside");
        assert!(ws.resolve("../inner2").is_err());
        assert!(ws.resolve(&base.display().to_string()).is_err());
        assert!(Workspace::anywhere().resolve(&base.display().to_string()).is_ok());
        assert!(Workspace::new(Vec::new()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
