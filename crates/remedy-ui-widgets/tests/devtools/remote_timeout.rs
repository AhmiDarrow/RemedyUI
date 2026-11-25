//! [`set_remote_timeout`] moves how long the Remote page waits. A binary of its own, so no
//! other test sees it changed.

use std::time::Duration;

use remedy_ui_widgets::devtools::{REMOTE_TIMEOUT, remote_timeout, set_remote_timeout};

#[test]
fn the_app_sets_how_long_the_remote_page_waits() {
    assert_eq!(remote_timeout(), REMOTE_TIMEOUT);
    set_remote_timeout(Duration::from_secs(90));
    assert_eq!(remote_timeout(), Duration::from_secs(90));
    set_remote_timeout(REMOTE_TIMEOUT);
}
