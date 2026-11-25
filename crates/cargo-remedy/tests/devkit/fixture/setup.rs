// Shared by remedy-mcp's and cargo-remedy's tests (`include!`d): the fixture app, set up in
// the target folder's test space so its build is reused between runs and between both test
// suites (which Cargo runs one after the other).

/// `p` as a plain string with `/`, without Windows' verbatim prefix (which the MSVC linker
/// does not take everywhere).
fn plain_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s).replace('\\', "/")
}

/// The workspace folder holding `fixture-app`, a small app depending on `checkout`'s facade
/// by path (sources from `crates/cargo-remedy/tests/devkit/fixture`).
fn fixture_workspace(checkout: &std::path::Path) -> std::path::PathBuf {
    let ws = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("remedy-fixture");
    let app = ws.join("fixture-app");
    let fail = |what: &str, e: std::io::Error| -> ! { unreachable!("{what}: {e}") };
    for d in [app.join("src"), app.join("tests")] {
        std::fs::create_dir_all(&d).unwrap_or_else(|e| fail("fixture folder", e));
    }
    let facade = plain_path(&checkout.join("crates").join("remedy-ui"));
    let manifest = format!(
        "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nremedy-ui = {{ path = \"{facade}\", default-features = false, features = [\"widgets\"] }}\n\n[workspace]\n"
    );
    let source = checkout.join("crates").join("cargo-remedy").join("tests").join("devkit").join("fixture");
    let copy = |from: std::path::PathBuf, to: std::path::PathBuf| {
        let text = std::fs::read_to_string(&from).unwrap_or_else(|e| fail("fixture source", e));
        if std::fs::read_to_string(&to).ok() != Some(text.clone()) {
            std::fs::write(&to, text).unwrap_or_else(|e| fail("fixture file", e));
        }
    };
    if std::fs::read_to_string(app.join("Cargo.toml")).ok().as_deref() != Some(manifest.as_str()) {
        std::fs::write(app.join("Cargo.toml"), &manifest).unwrap_or_else(|e| fail("fixture manifest", e));
    }
    copy(source.join("src").join("lib.rs"), app.join("src").join("lib.rs"));
    copy(source.join("tests").join("t.rs"), app.join("tests").join("t.rs"));
    if !app.join("Cargo.lock").is_file() {
        std::fs::copy(checkout.join("Cargo.lock"), app.join("Cargo.lock")).unwrap_or_else(|e| fail("fixture lock", e));
    }
    let ws = std::fs::canonicalize(&ws).unwrap_or_else(|e| fail("fixture workspace", e));
    std::path::PathBuf::from(plain_path(&ws))
}
