//! The interceptor battery's launcher as numbers (Java: `SamGeometry`), shared by the server (where
//! an interceptor leaves its rail) and the renderer (where the launcher and its missiles are drawn),
//! so the missile that flies off starts exactly where the one on the rail was.
//!
//! The battery stands on a three by three deck, half a block high. In its middle a turntable carries
//! a yoke, and on the yoke a launcher beam pivots up and down: two rails on each side, an interceptor
//! on each. Lengths in blocks, from the controller block's corner; yaw 0 points south (+z), turning
//! towards east (+x); the launcher's own axes are x across, y up from the beam, z along it.

use super::{jmax, jmin, signum};
use std::f64::consts::PI;

/// Height of the deck top.
pub const DECK: f64 = 0.5;
/// Height of the pivot the beam turns on.
pub const PIVOT_Y: f64 = 1.45;
/// Elevation of the beam when the battery is on (it fires at this angle), degrees.
pub const ELEVATION: f64 = 50.0;
/// Elevation of the beam at rest (safe), degrees.
pub const REST: f64 = 10.0;
/// How fast the turntable trains, degrees per tick (90 per second).
pub const SLEW_DEG_PER_TICK: f64 = 4.5;
/// Ticks for the beam to go from rest to firing elevation, or back.
pub const RAISE_TICKS: i32 = 40;
/// Size of an interceptor against the 3-block ballistic missile whose models it shares.
pub const MISSILE_SCALE: f64 = 0.55;
/// Where the rails sit in the launcher's own axes: across, up from the beam, and the missile's
/// middle along it.
pub const RAIL_X: f64 = 0.6;
pub const RAIL_Y: f64 = 0.28;
pub const MISSILE_Z: f64 = 0.5;

/// Across-axis of rail `slot` (0 upper left, 1 upper right, 2 lower left, 3 lower right).
pub fn rail_x(slot: i32) -> f64 {
    if slot & 1 == 0 { -RAIL_X } else { RAIL_X }
}

pub fn rail_y(slot: i32) -> f64 {
    if slot < 2 { RAIL_Y } else { -RAIL_Y }
}

/// Where the beam points: a unit vector for the given yaw and elevation (radians).
pub fn forward(yaw: f64, elevation: f64) -> [f64; 3] {
    let c = elevation.cos();
    [yaw.sin() * c, elevation.sin(), yaw.cos() * c]
}

/// World position of the middle of the interceptor on rail `slot`, for a battery whose controller
/// block has its corner at (bx, by, bz).
pub fn rail_centre(bx: f64, by: f64, bz: f64, yaw: f64, elevation: f64, slot: i32) -> [f64; 3] {
    local(bx, by, bz, yaw, elevation, rail_x(slot), rail_y(slot), MISSILE_Z)
}

/// A point given in the launcher's own axes, in the world.
#[allow(clippy::too_many_arguments)]
pub fn local(bx: f64, by: f64, bz: f64, yaw: f64, elevation: f64, lx: f64, ly: f64, lz: f64) -> [f64; 3] {
    // Pitch up about the launcher's x axis, then train about the vertical.
    let (ce, se) = (elevation.cos(), elevation.sin());
    let y1 = ly * ce + lz * se;
    let z1 = -ly * se + lz * ce;
    let (cy, sy) = (yaw.cos(), yaw.sin());
    let x2 = lx * cy + z1 * sy;
    let z2 = -lx * sy + z1 * cy;
    [bx + 0.5 + x2, by + PIVOT_Y + y1, bz + 0.5 + z2]
}

/// Yaw (radians) that points the launcher from the battery's middle towards (x, z).
pub fn yaw_towards(bx: f64, bz: f64, x: f64, z: f64) -> f64 {
    (x - (bx + 0.5)).atan2(z - (bz + 0.5))
}

/// An angle brought into (-pi, pi].
pub fn wrap(a: f64) -> f64 {
    let mut a = a % (2.0 * PI);
    if a > PI {
        a -= 2.0 * PI;
    }
    if a <= -PI {
        a += 2.0 * PI;
    }
    a
}

/// Yaw at `tick`, for a turntable that was at `from` at `since` and trains towards `to` the short
/// way round at [`SLEW_DEG_PER_TICK`].
pub fn yaw(from: f64, to: f64, since: f64, tick: f64) -> f64 {
    let d = wrap(to - from);
    let step = SLEW_DEG_PER_TICK.to_radians() * jmax(0.0, tick - since);
    if d.abs() <= step { to } else { from + signum(d) * step }
}

/// How far the beam is raised at `tick` (0 rest, 1 up, moving at a steady rate), for a beam that
/// was at `from` at `since` and moves up (`up`) or down.
pub fn raise(from: f64, up: bool, since: f64, tick: f64) -> f64 {
    let k = from + if up { 1.0 } else { -1.0 } * jmax(0.0, tick - since) / RAISE_TICKS as f64;
    jmax(0.0, jmin(1.0, k))
}

/// The beam's elevation (radians) for a raise fraction, eased at both ends like a hydraulic ram.
pub fn elevation_of(raise: f64) -> f64 {
    let k = jmax(0.0, jmin(1.0, raise));
    (REST + (ELEVATION - REST) * k * k * (3.0 - 2.0 * k)).to_radians()
}
