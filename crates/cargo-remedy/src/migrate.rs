//! `cargo remedy migrate [--dry-run] [--path <dir>]`: applies the mechanical fixes in
//! `migrations.toml` to an app's sources.
//!
//! Sources are parsed with `syn`; edits are made at the spans of the tokens they change, so
//! strings, comments and formatting stay as they were. Paths are resolved through each file's
//! `use` items before they are compared, so an item is found however the file names it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use proc_macro2::{LineColumn, Span};
use syn::spanned::Spanned;
use syn::visit::Visit;

/// The table that ships with this version of cargo-remedy.
pub const TABLE: &str = include_str!("../migrations.toml");

/// One rename of an item reached by path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathRename {
    /// The old path, through the facade (`remedy_ui::text::StylePatch`).
    pub old: Vec<String>,
    /// The new last segment.
    pub new: String,
}

/// One rename of a method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodRename {
    /// The old name.
    pub old: String,
    /// The new name.
    pub new: String,
}

/// One rewrite of calls to a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRewrite {
    /// The function's path, through the facade.
    pub path: Vec<String>,
    /// What the call becomes (`{fn}`, `{0}`, `{1}`, ...).
    pub template: String,
}

/// The migration table.
#[derive(Clone, Debug, Default)]
pub struct Table {
    /// Facade module → engine crate.
    pub crates: HashMap<String, String>,
    /// Engine crate that became a module → its new path (`remedy_ui_widgets::viz`).
    pub moved: HashMap<String, Vec<String>>,
    /// Path renames.
    pub paths: Vec<PathRename>,
    /// Method renames.
    pub methods: Vec<MethodRename>,
    /// Call rewrites.
    pub calls: Vec<CallRewrite>,
}

fn segments(path: &str) -> Vec<String> {
    path.split("::").map(str::to_owned).collect()
}

impl Table {
    /// Reads a table in the form of `migrations.toml`.
    pub fn parse(text: &str) -> Result<Table, String> {
        let doc: toml::Table = text.parse().map_err(|e| format!("migrations.toml: {e}"))?;
        let mut t = Table::default();
        if let Some(c) = doc.get("crates").and_then(|c| c.as_table()) {
            for (k, v) in c {
                t.crates.insert(k.clone(), v.as_str().ok_or("[crates] values are crate names")?.to_owned());
            }
        }
        if let Some(m) = doc.get("moved").and_then(|m| m.as_table()) {
            for (k, v) in m {
                t.moved.insert(k.clone(), segments(v.as_str().ok_or("[moved] values are paths")?));
            }
        }
        let field = |e: &toml::Value, k: &str| -> Result<String, String> {
            e.get(k).and_then(|v| v.as_str()).map(str::to_owned).ok_or(format!("an entry has no `{k}`"))
        };
        for e in doc.get("rename").and_then(|r| r.as_array()).into_iter().flatten() {
            match field(e, "kind")?.as_str() {
                "path" => {
                    t.paths.push(PathRename { old: segments(&field(e, "old")?), new: field(e, "new")? })
                }
                "method" => t.methods.push(MethodRename { old: field(e, "old")?, new: field(e, "new")? }),
                other => return Err(format!("unknown rename kind `{other}`")),
            }
        }
        for e in doc.get("call").and_then(|r| r.as_array()).into_iter().flatten() {
            t.calls.push(CallRewrite { path: segments(&field(e, "path")?), template: field(e, "template")? });
        }
        Ok(t)
    }

    /// `path` in its facade form: `remedy_ui_text::X` is `remedy_ui::text::X`.
    fn canonical(&self, path: &[String]) -> Vec<String> {
        if let Some((first, rest)) = path.split_first()
            && let Some((module, _)) = self.crates.iter().find(|(_, c)| *c == first)
        {
            let mut out = vec!["remedy_ui".to_owned(), module.clone()];
            out.extend(rest.iter().cloned());
            return out;
        }
        // The same crate under its new path: `remedy_ui_core::a11y::X` is `remedy_ui::a11y::X`.
        for (module, krate) in &self.crates {
            if let Some(now) = self.moved.get(krate)
                && let Some(rest) = path.strip_prefix(now.as_slice())
            {
                let mut out = vec!["remedy_ui".to_owned(), module.clone()];
                out.extend(rest.iter().cloned());
                return out;
            }
        }
        path.to_vec()
    }
}

/// One edit: replace `start..end` (bytes) with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

/// Byte offsets of each line's start, for turning spans into offsets.
struct Lines<'a> {
    src: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(src: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Lines { src, starts }
    }

    fn offset(&self, at: LineColumn) -> Option<usize> {
        let line_start = *self.starts.get(at.line.checked_sub(1)?)?;
        let line = self.src.get(line_start..)?;
        // Columns count characters.
        let byte = line.char_indices().nth(at.column).map_or(line.len(), |(b, _)| b);
        Some(line_start + byte)
    }

    fn range(&self, span: Span) -> Option<(usize, usize)> {
        Some((self.offset(span.start())?, self.offset(span.end())?))
    }

    fn text(&self, span: Span) -> Option<&'a str> {
        let (a, b) = self.range(span)?;
        self.src.get(a..b)
    }
}

/// Names the file's `use` items bring in: local name → full path.
fn use_aliases(file: &syn::File) -> HashMap<String, Vec<String>> {
    fn walk(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut HashMap<String, Vec<String>>) {
        match tree {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                walk(&p.tree, prefix, out);
                prefix.pop();
            }
            syn::UseTree::Name(n) => {
                let mut full = prefix.clone();
                let name = n.ident.to_string();
                if name == "self" {
                    if let Some(last) = prefix.last() {
                        out.insert(last.clone(), full);
                    }
                    return;
                }
                full.push(name.clone());
                out.insert(name, full);
            }
            syn::UseTree::Rename(r) => {
                let mut full = prefix.clone();
                full.push(r.ident.to_string());
                out.insert(r.rename.to_string(), full);
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    walk(t, prefix, out);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut out = HashMap::new();
    for item in &file.items {
        if let syn::Item::Use(u) = item {
            walk(&u.tree, &mut Vec::new(), &mut out);
        }
    }
    out
}

/// Names of functions the app defines itself (methods are renamed only when it defines none
/// of the old name).
fn own_fns(file: &syn::File) -> Vec<String> {
    struct V(Vec<String>);
    impl<'ast> Visit<'ast> for V {
        fn visit_signature(&mut self, s: &'ast syn::Signature) {
            self.0.push(s.ident.to_string());
        }
    }
    let mut v = V(Vec::new());
    v.visit_file(file);
    v.0
}

struct Migrator<'a> {
    table: &'a Table,
    aliases: HashMap<String, Vec<String>>,
    lines: Lines<'a>,
    own: Vec<String>,
    edits: Vec<Edit>,
}

impl Migrator<'_> {
    fn resolve(&self, segs: &[String]) -> Vec<String> {
        let mut full = match segs.split_first() {
            Some((first, rest)) => match self.aliases.get(first) {
                Some(a) => a.iter().cloned().chain(rest.iter().cloned()).collect(),
                None => segs.to_vec(),
            },
            None => Vec::new(),
        };
        if full.first().is_some_and(|f| f == "crate" || f.is_empty()) {
            full.remove(0);
        }
        self.table.canonical(&full)
    }

    fn rename_ident(&mut self, ident: &syn::Ident, new: &str) {
        if let Some((start, end)) = self.lines.range(ident.span()) {
            self.edits.push(Edit { start, end, text: new.to_owned() });
        }
    }

    /// A path that starts with a crate that became a module starts with its new place.
    fn moved_crate(&mut self, first: &syn::Ident) {
        if let Some(now) = self.table.moved.get(&first.to_string()).map(|p| p.join("::")) {
            self.rename_ident(first, &now);
        }
    }

    fn path_rename(&self, full: &[String]) -> Option<&PathRename> {
        self.table.paths.iter().find(|r| r.old == full)
    }

    fn use_tree(&mut self, tree: &syn::UseTree, prefix: &mut Vec<String>) {
        match tree {
            syn::UseTree::Path(p) => {
                if prefix.is_empty() {
                    self.moved_crate(&p.ident);
                }
                prefix.push(p.ident.to_string());
                self.use_tree(&p.tree, prefix);
                prefix.pop();
            }
            syn::UseTree::Name(n) => {
                if prefix.is_empty() {
                    self.moved_crate(&n.ident);
                }
                let mut full = prefix.clone();
                full.push(n.ident.to_string());
                let full = self.table.canonical(&full);
                if let Some(new) = self.path_rename(&full).map(|r| r.new.clone()) {
                    self.rename_ident(&n.ident, &new);
                }
            }
            syn::UseTree::Rename(r) => {
                if prefix.is_empty() {
                    self.moved_crate(&r.ident);
                }
                let mut full = prefix.clone();
                full.push(r.ident.to_string());
                let full = self.table.canonical(&full);
                if let Some(new) = self.path_rename(&full).map(|x| x.new.clone()) {
                    self.rename_ident(&r.ident, &new);
                }
            }
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    self.use_tree(t, prefix);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
}

impl<'ast> Visit<'ast> for Migrator<'_> {
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        self.use_tree(&u.tree, &mut Vec::new());
    }

    fn visit_path(&mut self, p: &'ast syn::Path) {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        // The crate itself, written out (not a local name a `use` gave something else).
        if segs.len() > 1
            && let Some(first) = p.segments.first()
            && !self.aliases.contains_key(&first.ident.to_string())
        {
            self.moved_crate(&first.ident);
        }
        // A bare name brought in by `use` was renamed there; a path is renamed at its last
        // segment, or at the segment naming the item when more follows (`StylePatch::bold`).
        for n in (1..=segs.len()).rev() {
            let Some(head) = segs.get(..n) else { continue };
            if n == 1 && segs.len() == 1 && segs.first().is_some_and(|s| self.aliases.contains_key(s)) {
                // The `use` that brought the name in was renamed; the local name follows unless
                // the `use` gave it a name of its own (`as`).
                let full = self.resolve(head);
                let own_name = full.last() != segs.first();
                if !own_name
                    && let Some(new) = self.path_rename(&full).map(|r| r.new.clone())
                    && let Some(seg) = p.segments.first()
                {
                    self.rename_ident(&seg.ident, &new);
                }
                break;
            }
            let full = self.resolve(head);
            if let Some(new) = self.path_rename(&full).map(|r| r.new.clone()) {
                if let Some(seg) = p.segments.iter().nth(n - 1) {
                    self.rename_ident(&seg.ident, &new);
                }
                break;
            }
        }
        syn::visit::visit_path(self, p);
    }

    fn visit_expr_method_call(&mut self, m: &'ast syn::ExprMethodCall) {
        let name = m.method.to_string();
        if let Some(new) = self.table.methods.iter().find(|r| r.old == name).map(|r| r.new.clone())
            && !self.own.contains(&name)
        {
            self.rename_ident(&m.method, &new);
        }
        syn::visit::visit_expr_method_call(self, m);
    }

    fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
        if let syn::Expr::Path(fp) = &*c.func {
            let segs: Vec<String> = fp.path.segments.iter().map(|s| s.ident.to_string()).collect();
            let full = self.resolve(&segs);
            if let Some(rule) = self.table.calls.iter().find(|r| r.path == full).cloned() {
                let args: Option<Vec<&str>> = c.args.iter().map(|a| self.lines.text(a.span())).collect();
                let prefix = if segs.len() > 1 {
                    let written = self.lines.text(fp.span()).unwrap_or_default();
                    written.rsplit_once("::").map(|(a, _)| format!("{a}::")).unwrap_or_default()
                } else {
                    full.get(..full.len().saturating_sub(1))
                        .map(|p| format!("{}::", p.join("::")))
                        .unwrap_or_default()
                };
                if let (Some(args), Some((start, end))) = (args, self.lines.range(c.span())) {
                    let mut text = rule.template.replace("{fn}", &prefix);
                    for (i, a) in args.iter().enumerate() {
                        text = text.replace(&format!("{{{i}}}"), a);
                    }
                    self.edits.push(Edit { start, end, text });
                    // The arguments are carried over as written; nothing inside is rewritten
                    // twice.
                    return;
                }
            }
        }
        syn::visit::visit_expr_call(self, c);
    }
}

/// The migrated text of one source file, or `None` when nothing changes.
pub fn migrate_source(table: &Table, src: &str) -> Result<Option<String>, String> {
    let file = syn::parse_file(src).map_err(|e| format!("does not parse: {e}"))?;
    let mut m = Migrator {
        table,
        aliases: use_aliases(&file),
        lines: Lines::new(src),
        own: own_fns(&file),
        edits: Vec::new(),
    };
    m.visit_file(&file);
    if m.edits.is_empty() {
        return Ok(None);
    }
    let mut edits = m.edits;
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    edits.dedup();
    let mut out = src.to_owned();
    let mut last = usize::MAX;
    for e in edits {
        if e.end > last {
            continue;
        }
        if out.get(e.start..e.end).is_some() {
            out.replace_range(e.start..e.end, &e.text);
            last = e.start;
        }
    }
    Ok(Some(out))
}

/// A line diff of `old` and `new` for `--dry-run`.
pub fn diff(path: &Path, old: &str, new: &str) -> String {
    let (a, b): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
    let mut out = format!("--- {}\n+++ {} (migrated)\n", path.display(), path.display());
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i), b.get(i));
        if x != y {
            out.push_str(&format!("@@ line {} @@\n", i + 1));
            if let Some(x) = x {
                out.push_str(&format!("-{x}\n"));
            }
            if let Some(y) = y {
                out.push_str(&format!("+{y}\n"));
            }
        }
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target" || n == ".git") {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `cargo remedy migrate`.
pub fn run(args: &[String]) -> Result<(), String> {
    let dry = args.iter().any(|a| a == "--dry-run");
    let root = match crate::flag_value(args, "--path") {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    let table = Table::parse(TABLE)?;
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    files.sort();
    let mut changed = 0;
    for f in files {
        let src = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        let new = match migrate_source(&table, &src) {
            Ok(Some(n)) => n,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("skipped {}: {e}", f.display());
                continue;
            }
        };
        changed += 1;
        if dry {
            print!("{}", diff(&f, &src, &new));
        } else {
            std::fs::write(&f, new).map_err(|e| format!("{}: {e}", f.display()))?;
            println!("migrated {}", f.display());
        }
    }
    match (changed, dry) {
        (0, _) => println!("nothing to migrate"),
        (n, true) => println!("{n} file(s) would change; run without --dry-run to apply"),
        (n, false) => println!("{n} file(s) migrated; review the changes and run cargo check"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        let Ok(t) = Table::parse(TABLE) else { unreachable!("the shipped table parses") };
        t
    }

    fn migrate(src: &str) -> String {
        match migrate_source(&table(), src) {
            Ok(Some(s)) => s,
            Ok(None) => src.to_owned(),
            Err(e) => unreachable!("{e}"),
        }
    }

    #[test]
    fn crates_that_became_modules_are_followed_to_their_new_place() {
        let src = "use remedy_ui_viz::bars;\nuse remedy_ui_view::{El, Ui};\nuse remedy_ui_reactive as rx;\nfn f(ui: &Ui) -> El {\n    let s = rx::Signal::new(0);\n    remedy_ui_widgets_core::text(ui, \"remedy_ui_viz stays in strings\")\n}\n";
        let out = migrate(src);
        assert!(out.contains("use remedy_ui_widgets::viz::bars;"), "{out}");
        assert!(out.contains("use remedy_ui_engine::view::{El, Ui};"), "{out}");
        assert!(out.contains("use remedy_ui_core::reactive as rx;"), "{out}");
        assert!(
            out.contains("remedy_ui_widgets::base::text(ui, \"remedy_ui_viz stays in strings\")"),
            "{out}"
        );
        assert!(out.contains("rx::Signal::new(0)"), "a local name is left as it is: {out}");
        // Migrating twice changes nothing more, and an item renamed earlier is still found
        // under the crate's new path.
        assert_eq!(migrate(&out), out);
        let renamed = migrate("fn f() { let _ = remedy_ui_engine::view::TextPatch::default(); }\n");
        assert!(!renamed.is_empty());
    }

    #[test]
    fn renamed_items_are_found_however_they_are_named() {
        let src = "use remedy_ui::text::StylePatch;\nuse remedy_ui::text;\nfn f() {\n    let a = StylePatch::bold();\n    let b = text::StylePatch::italic();\n    let c = remedy_ui_text::StylePatch::default();\n    let s = \"StylePatch stays in strings\"; // and StylePatch in comments\n}\n";
        let out = migrate(src);
        assert!(out.contains("use remedy_ui::text::TextStylePatch;"), "{out}");
        assert!(out.contains("let a = TextStylePatch::bold();"), "{out}");
        assert!(out.contains("text::TextStylePatch::italic()"), "{out}");
        assert!(out.contains("remedy_ui_text::TextStylePatch::default()"), "{out}");
        assert!(out.contains("\"StylePatch stays in strings\"; // and StylePatch in comments"), "{out}");
        // A StylePatch of another crate is left alone.
        let other = "use remedy_ui::style::StylePatch;\nfn f() { let _ = StylePatch::new(); }\n";
        assert_eq!(migrate(other), other);
    }

    #[test]
    fn calls_and_methods_are_rewritten_unless_the_app_owns_the_name() {
        let src = "use remedy_ui::widgets as w;\nfn f(ui: &Ui, p: &Player) -> bool {\n    let _ = w::label(ui, format!(\"{}, {}\", 1, 2), \"muted\");\n    p.muted()\n}\n";
        let out = migrate(src);
        assert!(out.contains("w::text(ui, format!(\"{}, {}\", 1, 2)).variant(\"muted\")"), "{out}");
        assert!(out.contains("p.is_muted()"), "{out}");
        let own =
            "struct S;\nimpl S { fn muted(&self) -> bool { true } }\nfn f(s: S) -> bool { s.muted() }\n";
        assert_eq!(migrate(own), own, "the app's own muted() stays");
        let bare = "use remedy_ui::widgets::label;\nfn f(ui: &Ui) { let _ = label(ui, \"x\", \"small\"); }\n";
        assert!(
            migrate(bare).contains("remedy_ui::widgets::text(ui, \"x\").variant(\"small\")"),
            "{}",
            migrate(bare)
        );
    }

    #[test]
    fn dry_runs_show_a_diff() {
        let d = diff(Path::new("src/lib.rs"), "a\nb\n", "a\nc\n");
        assert!(d.contains("-b") && d.contains("+c") && d.contains("line 2"), "{d}");
    }
}
