//! Properties that take a constant, a signal or a closure.
//!
//! Every builder property that can change accepts `impl IntoProp<T>`: a plain value sets it
//! once and creates no binding; a [`Signal`] or [`Memo`] (of `T`, or of anything that
//! converts into it) and a closure (`Fn() -> R` where `R: Into<T>`) bind it, so it follows
//! the signals read. Making a property reactive never changes the call's shape:
//!
//! ```ignore
//! button.disabled(true);                       // constant
//! button.disabled(busy);                       // a Signal<bool>
//! button.disabled(move || !can_send.get());    // a closure
//! ```

use std::borrow::Cow;
use std::rc::Rc;

use remedy_ui_core::a11y::Semantics;
use remedy_ui_core::reactive::{Memo, Signal};
use remedy_ui_core::style::{Style, StylePatch};
use remedy_ui_text::RichText;

/// A property's source: a value, or a closure re-read when its signals change.
// Exhaustive on purpose: a property is set once or bound, and nothing else.
pub enum Prop<T> {
    /// Set once.
    Value(T),
    /// Bound: read now and again whenever what it reads changes.
    Dynamic(Rc<dyn Fn() -> T>),
}

impl<T> Clone for Prop<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Prop::Value(v) => Prop::Value(v.clone()),
            Prop::Dynamic(f) => Prop::Dynamic(f.clone()),
        }
    }
}

impl<T> std::fmt::Debug for Prop<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Prop::Value(v) => f.debug_tuple("Value").field(v).finish(),
            Prop::Dynamic(_) => f.write_str("Dynamic"),
        }
    }
}

impl<T: 'static> Prop<T> {
    /// The value now (reading a dynamic property's signals, so a binding that calls this
    /// follows them).
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        match self {
            Prop::Value(v) => v.clone(),
            Prop::Dynamic(f) => f(),
        }
    }

    /// True when it is bound to signals.
    pub fn is_dynamic(&self) -> bool {
        matches!(self, Prop::Dynamic(_))
    }

    /// The property mapped through `f`.
    pub fn map<U: 'static>(self, f: impl Fn(T) -> U + 'static) -> Prop<U> {
        match self {
            Prop::Value(v) => Prop::Value(f(v)),
            Prop::Dynamic(g) => Prop::Dynamic(Rc::new(move || f(g()))),
        }
    }
}

/// Anything a builder property accepts: a value, a signal, a memo or a closure.
pub trait IntoProp<T> {
    /// The property.
    fn into_prop(self) -> Prop<T>;
}

impl<T> IntoProp<T> for Prop<T> {
    fn into_prop(self) -> Prop<T> {
        self
    }
}

impl<F, R, T> IntoProp<T> for F
where
    F: Fn() -> R + 'static,
    R: Into<T>,
{
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Rc::new(move || self().into()))
    }
}

impl<S, T> IntoProp<T> for Signal<S>
where
    S: Clone + Into<T> + 'static,
{
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Rc::new(move || self.get().into()))
    }
}

impl<S, T> IntoProp<T> for Memo<S>
where
    S: Clone + PartialEq + Into<T> + 'static,
{
    fn into_prop(self) -> Prop<T> {
        Prop::Dynamic(Rc::new(move || self.get().into()))
    }
}

macro_rules! value_props {
    ($($from:ty => $to:ty),* $(,)?) => {
        $(impl IntoProp<$to> for $from {
            fn into_prop(self) -> Prop<$to> {
                Prop::Value(self.into())
            }
        })*
    };
}

value_props! {
    bool => bool,
    f32 => f32,
    f64 => f64,
    usize => usize,
    u32 => u32,
    i32 => i32,
    String => String,
    &str => String,
    &String => String,
    Cow<'_, str> => String,
    char => String,
    RichText => RichText,
    String => RichText,
    &str => RichText,
    &'static str => &'static str,
    Semantics => Semantics,
    StylePatch => Style,
    Style => Style,
    StylePatch => StylePatch,
    Option<f32> => Option<f32>,
    f32 => Option<f32>,
    Option<String> => Option<String>,
    remedy_ui_core::Color => remedy_ui_core::Color,
}

impl IntoProp<RichText> for &String {
    fn into_prop(self) -> Prop<RichText> {
        Prop::Value(RichText::plain(self.as_str()))
    }
}

impl IntoProp<RichText> for Cow<'_, str> {
    fn into_prop(self) -> Prop<RichText> {
        Prop::Value(RichText::plain(self.as_ref()))
    }
}
