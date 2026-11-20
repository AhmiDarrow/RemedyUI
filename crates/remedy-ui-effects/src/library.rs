//! The built-in effects, written in `.rfx` like any theme's: anything they do, a theme can
//! do too.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::Effect;

/// Every built-in effect: (name, source).
pub const LIBRARY: &[(&str, &str)] = &[
    ("aurora", include_str!("../effects/aurora.rfx")),
    ("bokeh", include_str!("../effects/bokeh.rfx")),
    ("bubbles", include_str!("../effects/bubbles.rfx")),
    ("caustics", include_str!("../effects/caustics.rfx")),
    ("clouds", include_str!("../effects/clouds.rfx")),
    ("code_rain", include_str!("../effects/code_rain.rfx")),
    ("confetti", include_str!("../effects/confetti.rfx")),
    ("crt", include_str!("../effects/crt.rfx")),
    ("crt_screen", include_str!("../effects/crt_screen.rfx")),
    ("crystal", include_str!("../effects/crystal.rfx")),
    ("embers", include_str!("../effects/embers.rfx")),
    ("film_grain", include_str!("../effects/film_grain.rfx")),
    ("fire", include_str!("../effects/fire.rfx")),
    ("fireflies", include_str!("../effects/fireflies.rfx")),
    ("fog", include_str!("../effects/fog.rfx")),
    ("frost", include_str!("../effects/frost.rfx")),
    ("galaxy", include_str!("../effects/galaxy.rfx")),
    ("glow", include_str!("../effects/glow.rfx")),
    ("heat_shimmer", include_str!("../effects/heat_shimmer.rfx")),
    ("lava_lamp", include_str!("../effects/lava_lamp.rfx")),
    ("light_shafts", include_str!("../effects/light_shafts.rfx")),
    ("lightning", include_str!("../effects/lightning.rfx")),
    ("mesh_gradient", include_str!("../effects/mesh_gradient.rfx")),
    ("nebula", include_str!("../effects/nebula.rfx")),
    ("orb", include_str!("../effects/orb.rfx")),
    ("paper", include_str!("../effects/paper.rfx")),
    ("plasma", include_str!("../effects/plasma.rfx")),
    ("rain", include_str!("../effects/rain.rfx")),
    ("retro_grid", include_str!("../effects/retro_grid.rfx")),
    ("ripples", include_str!("../effects/ripples.rfx")),
    ("shimmer", include_str!("../effects/shimmer.rfx")),
    ("smoke", include_str!("../effects/smoke.rfx")),
    ("snow", include_str!("../effects/snow.rfx")),
    ("sparkles", include_str!("../effects/sparkles.rfx")),
    ("starfield", include_str!("../effects/starfield.rfx")),
    ("trails", include_str!("../effects/trails.rfx")),
    ("volumetric_clouds", include_str!("../effects/volumetric_clouds.rfx")),
    ("water", include_str!("../effects/water.rfx")),
];

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
