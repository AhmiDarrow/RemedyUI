//! The built-in effects, written in `.rfx` like any theme's: anything they do, a theme can
//! do too.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::effects::Effect;

/// Every built-in effect: (name, source).
pub const LIBRARY: &[(&str, &str)] = &[
    ("aurora", include_str!("../../effects/effects/aurora.rfx")),
    ("bokeh", include_str!("../../effects/effects/bokeh.rfx")),
    ("bubbles", include_str!("../../effects/effects/bubbles.rfx")),
    ("caustics", include_str!("../../effects/effects/caustics.rfx")),
    ("clouds", include_str!("../../effects/effects/clouds.rfx")),
    ("code_rain", include_str!("../../effects/effects/code_rain.rfx")),
    ("confetti", include_str!("../../effects/effects/confetti.rfx")),
    ("crt", include_str!("../../effects/effects/crt.rfx")),
    ("crt_screen", include_str!("../../effects/effects/crt_screen.rfx")),
    ("crystal", include_str!("../../effects/effects/crystal.rfx")),
    ("embers", include_str!("../../effects/effects/embers.rfx")),
    ("film_grain", include_str!("../../effects/effects/film_grain.rfx")),
    ("fire", include_str!("../../effects/effects/fire.rfx")),
    ("fireflies", include_str!("../../effects/effects/fireflies.rfx")),
    ("fog", include_str!("../../effects/effects/fog.rfx")),
    ("frost", include_str!("../../effects/effects/frost.rfx")),
    ("galaxy", include_str!("../../effects/effects/galaxy.rfx")),
    ("glow", include_str!("../../effects/effects/glow.rfx")),
    ("heat_shimmer", include_str!("../../effects/effects/heat_shimmer.rfx")),
    ("lava_lamp", include_str!("../../effects/effects/lava_lamp.rfx")),
    ("light_shafts", include_str!("../../effects/effects/light_shafts.rfx")),
    ("lightning", include_str!("../../effects/effects/lightning.rfx")),
    ("mesh_gradient", include_str!("../../effects/effects/mesh_gradient.rfx")),
    ("nebula", include_str!("../../effects/effects/nebula.rfx")),
    ("orb", include_str!("../../effects/effects/orb.rfx")),
    ("paper", include_str!("../../effects/effects/paper.rfx")),
    ("plasma", include_str!("../../effects/effects/plasma.rfx")),
    ("rain", include_str!("../../effects/effects/rain.rfx")),
    ("retro_grid", include_str!("../../effects/effects/retro_grid.rfx")),
    ("ripples", include_str!("../../effects/effects/ripples.rfx")),
    ("shimmer", include_str!("../../effects/effects/shimmer.rfx")),
    ("smoke", include_str!("../../effects/effects/smoke.rfx")),
    ("snow", include_str!("../../effects/effects/snow.rfx")),
    ("sparkles", include_str!("../../effects/effects/sparkles.rfx")),
    ("starfield", include_str!("../../effects/effects/starfield.rfx")),
    ("trails", include_str!("../../effects/effects/trails.rfx")),
    ("volumetric_clouds", include_str!("../../effects/effects/volumetric_clouds.rfx")),
    ("water", include_str!("../../effects/effects/water.rfx")),
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
