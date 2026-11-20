//! Every `rfx` code block in the language reference (docs/effects/rfx.md) compiles: whole
//! effects as they are, fragments (functions and passes) inside an effect.

use remedy_ui_effects::Effect;

#[test]
fn the_reference_examples_compile() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/effects/rfx.md");
    let doc = std::fs::read_to_string(path).unwrap_or_default();
    assert!(!doc.is_empty(), "{path} is readable");
    let mut blocks = 0;
    let mut rest = doc.as_str();
    while let Some(start) = rest.find("```rfx\n") {
        let body = &rest[start + 7..];
        let Some(end) = body.find("```") else { break };
        let code = &body[..end];
        rest = &body[end + 3..];
        blocks += 1;
        let first = code.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("//")).unwrap_or("");
        let src = if first.starts_with("effect") {
            code.to_owned()
        } else if first.starts_with("particles") {
            format!("effect doc {{ pass flame {{ out = 1.0; }}\n{code}\n}}")
        } else {
            format!("effect doc {{\n{code}\n}}")
        };
        if let Err(e) = Effect::compile(&src) {
            unreachable!("a reference example does not compile: {e}\n{src}");
        }
    }
    assert!(blocks >= 3, "found {blocks} examples");
}
