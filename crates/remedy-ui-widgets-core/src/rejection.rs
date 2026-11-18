//! Why the app turned down a change a person made, in words to show them.

/// Why the app turned down a change a person made (an edit, a drop, a form value), in words
/// the widget shows them next to what they changed.
///
/// Validators and edit callbacks return it as their error; `?` and `.into()` make one from a
/// `String` or `&str`:
///
/// ```
/// use remedy_ui_widgets_core::Rejection;
///
/// fn check_name(name: &str) -> Result<(), Rejection> {
///     if name.trim().is_empty() {
///         return Err("A name is needed.".into());
///     }
///     Ok(())
/// }
/// assert_eq!(check_name(" ").map_err(|r| r.to_string()), Err("A name is needed.".to_owned()));
/// assert!(check_name("Ada").is_ok());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Rejection {
    message: String,
}

impl Rejection {
    /// A rejection that tells the person `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Rejection { message: message.into() }
    }

    /// What the person is told.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Rejection {}

impl From<String> for Rejection {
    fn from(message: String) -> Self {
        Rejection { message }
    }
}

impl From<&str> for Rejection {
    fn from(message: &str) -> Self {
        Rejection { message: message.to_owned() }
    }
}
