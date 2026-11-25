//! One test that passes and one that fails, for the test summaries.

#[test]
fn adds() {
    assert_eq!(1 + 1, 2);
}

#[test]
fn deliberately_fails() {
    assert_eq!(1 + 1, 3, "this one fails on purpose");
}
