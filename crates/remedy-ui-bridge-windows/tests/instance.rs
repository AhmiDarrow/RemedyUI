//! Single instance on Windows: a second claim hands its arguments to the first over the pipe.
#![cfg(windows)]

use std::sync::mpsc;
use std::time::Duration;

use remedy_ui_bridge_windows::claim_single_instance;

#[test]
fn a_second_launch_hands_its_arguments_to_the_first() {
    let id = format!("remedy-ui-test-{}", std::process::id());
    let (tx, rx) = mpsc::channel();
    let first = claim_single_instance(&id, &[], move |args| {
        let _ = tx.send(args);
    });
    assert_eq!(first, Ok(true), "the first claim runs");
    let second = claim_single_instance(&id, &["--show".to_owned(), "note 3".to_owned()], |_| {});
    assert_eq!(second, Ok(false), "the second hands over and should exit");
    let got = rx.recv_timeout(Duration::from_secs(5));
    assert_eq!(got, Ok(vec!["--show".to_owned(), "note 3".to_owned()]));
}
