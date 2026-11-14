//! What one observer experiences, as plain functions of time and distance (Java:
//! `fx.ViewerEffects`): the flash, the glow, the dust after the wave, the shaking and when the
//! ground shock and the blast wave arrive. Light is instantaneous; the blast wave and its sound
//! follow [`BlastModel::arrival_time`] along the straight line from ground zero.

use crate::fx::mushroom::{clamp01, smooth, MushroomCloud};
use crate::BlastModel;

/// Seismic speed of the small tremor that precedes the air blast, m/s.
pub const GROUND_SPEED: f64 = 1800.0;

/// White-out of the screen, 0..1. `dist`: eye to fireball, metres; `look`: cosine between the
/// view and the fireball's direction; `sight`: 1 in direct line of sight, 0 hidden.
pub fn whiteout(cloud: &MushroomCloud, t: f64, dist: f64, look: f64, sight: f64) -> f64 {
    let base = cloud.flash(t);
    if base <= 0.0 {
        return 0.0;
    }
    let range = 1500.0 * cloud.model.cube_root;
    let dist_k = (1.7 / (1.0 + dist / range)).min(1.0);
    let dir_k = 0.4 + 0.6 * clamp01((look + 0.15) / 1.15);
    let los_k = 0.3 + 0.7 * sight;
    clamp01(base * dist_k * dir_k * los_k)
}

/// Strength of the orange glow on the sky and the land, 0..1 (before the night boost).
pub fn glow(cloud: &MushroomCloud, t: f64, dist: f64) -> f64 {
    let h = cloud.heat(t);
    if h < 0.005 {
        return 0.0;
    }
    let range = 2600.0 * cloud.model.yield_kt.powf(0.33);
    clamp01(0.75 * h.powf(0.75) * (1.5 / (1.0 + dist / range)).min(1.0))
}

/// How strongly the ground shock that runs ahead of the air blast is felt, 0..1.
pub fn tremor(m: &BlastModel, horiz: f64) -> f64 {
    (m.radius_for_psi(20.0) / horiz.max(1.0)).powf(1.8).min(1.0)
}

/// When the ground shock arrives, seconds, or -1 if it does not run ahead of the air blast by at
/// least a quarter of a second.
pub fn tremor_time(m: &BlastModel, horiz: f64, dist: f64) -> f64 {
    let tg = horiz / GROUND_SPEED;
    if tg + 0.25 < m.arrival_time(dist) {
        tg
    } else {
        -1.0
    }
}

/// Camera shake amplitude in degrees. `horiz`: horizontal distance from ground zero; `dist`: the
/// straight one; `grounded`: false for someone off the ground (no tremor underfoot).
pub fn shake(m: &BlastModel, t: f64, horiz: f64, dist: f64, grounded: bool) -> f64 {
    let mut amp = 0.0;
    let ta = m.arrival_time(dist);
    let psi = m.overpressure_psi(dist);
    let tg = if grounded { tremor_time(m, horiz, dist) } else { -1.0 };
    if tg >= 0.0 && t >= tg && t < ta + 0.2 {
        amp += 0.6 * tremor(m, horiz) * (-(t - tg) / 1.2).exp() * smooth(tg, tg + 0.15, t);
    }
    if t >= ta {
        let a0 = (1.3 * psi.powf(0.6)).min(9.0);
        let dt = t - ta;
        amp += a0 * (-dt / 0.7).exp() + 0.18 * a0 * (-dt / 7.0).exp();
    }
    amp
}

/// Smooth pseudo-random signal in [-1, 1] for the shake (sum of incommensurate sines).
pub fn noise(t: f64, seed: f64) -> f64 {
    0.5 * (t * 37.1 + seed).sin() + 0.3 * (t * 61.7 + seed * 1.7).sin() + 0.2 * (t * 23.3 + seed * 2.9).sin()
}

/// Angular radius (radians) of a fireball filling the view enough to give the full glare.
const GLARE_FULL: f64 = 0.15;

/// Glare round the fireball on screen, 0..1: the fireball's brightness times the patch of sky it
/// covers, so a far, small fireball dazzles little (Java: `ViewerEffects.glare`, the fix of 10-11).
pub fn glare(cloud: &MushroomCloud, t: f64, dist: f64) -> f64 {
    let base = (cloud.flash(t) * 1.4 + 0.5 * cloud.heat(t).powi(4)).min(1.0);
    let a = cloud.model.fireball_radius / dist.max(1.0);
    base * ((a / GLARE_FULL) * (a / GLARE_FULL)).min(1.0)
}

/// Angular radius (radians) of the after-image: the fireball as the flash burnt it into the eye.
pub fn after_image_radius(cloud: &MushroomCloud, t: f64, dist: f64) -> f64 {
    let t_max = 0.0417 * cloud.model.yield_kt.powf(0.44);
    1.25 * cloud.model.fireball_radius_at(t.max(t_max)) / dist.max(1.0)
}

/// Radius of the dust that hangs in the air after the blast wave has passed.
pub fn dust_radius(m: &BlastModel) -> f64 {
    m.radius_for_psi(2.5)
}

/// Dust haze around the observer, 0..1; `horiz` is the horizontal distance from ground zero.
pub fn dust(m: &BlastModel, t: f64, horiz: f64, life_seconds: f64) -> f64 {
    let r = dust_radius(m);
    if horiz > r {
        return 0.0;
    }
    let ta = m.arrival_time(horiz);
    if t < ta {
        return 0.0;
    }
    let k = 1.0 - horiz / r;
    let settle = 1.0 - 0.75 * smooth(15.0, 0.3 * life_seconds, t - ta);
    clamp01(0.55 * k.sqrt() * smooth(ta, ta + 3.5, t) * settle)
}
