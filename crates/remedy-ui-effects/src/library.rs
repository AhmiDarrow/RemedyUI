//! The built-in effects, written in `.rfx` like any theme's: anything they do, a theme can
//! do too.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::Effect;

/// Every built-in effect: (name, source).
pub const LIBRARY: &[(&str, &str)] =
    &[("fire", include_str!("../effects/fire.rfx")), ("starfield", include_str!("../effects/starfield.rfx"))];

/// The built-in effect `name`, compiled once and shared.
pub fn library(name: &str) -> Option<Arc<Effect>> {
    static COMPILED: OnceLock<HashMap<&'static str, Arc<Effect>>> = OnceLock::new();
    COMPILED
        .get_or_init(|| {
            LIBRARY
                .iter()
                .filter_map(|(n, src)| Effect::compile(src).ok().map(|e| (*n, Arc::new(e))))
                .collect()
        })
        .get(name)
        .cloned()
}

/// The source of the built-in effect `name` (to show, copy or edit).
pub fn library_source(name: &str) -> Option<&'static str> {
    LIBRARY.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}
