//! What a radar can see past the ground and the buildings around it (Java: `RadarHorizon`): its
//! horizon, as the highest elevation angle of any obstacle in each of [`BINS`] directions, out to
//! [`REACH`] blocks. A target is seen only above that angle, the way a real radar is blind behind a
//! hill or a wall. The server measures the horizon and sends it to clients with the radar.

use std::f64::consts::PI;

/// Directions round the radar (5 degrees each).
pub const BINS: usize = 72;
/// How far out obstacles count, blocks (what a server keeps loaded around a radar, at most).
pub const REACH: f64 = 256.0;
/// First distance looked at: just outside the radar's own base.
pub const START: f64 = 2.5;
/// Height of the antenna dish (Java: `fx.RadarAntenna.HEIGHT`).
pub const ANTENNA_HEIGHT: f64 = 2.7;
/// Height of the antenna's middle above the radar controller's bottom (the dish turns from 2 up).
pub const ANTENNA_Y: f64 = 2.0 + 0.5 * ANTENNA_HEIGHT;

/// The direction bin of a point (dx, dz) away from the radar.
pub fn bin(dx: f64, dz: f64) -> usize {
    let a = dx.atan2(dz);
    let b = ((a + PI) / (2.0 * PI) * BINS as f64).floor() as i32;
    b.rem_euclid(BINS as i32) as usize
}

/// The middle of bin `b` shifted by `k` of a bin (a ray to look along), as an angle for [`bin`].
pub fn angle(b: i32, k: f64) -> f64 {
    -PI + (b as f64 + 0.5 + k) * 2.0 * PI / BINS as f64
}

/// The next distance to look at after `r`: every block close by, sparser far out.
pub fn next(r: f64) -> f64 {
    r + 1.0f64.max(0.05 * r)
}

/// An elevation angle in whole degrees, rounded up, as stored.
pub fn encode(degrees: f64) -> i8 {
    if degrees.is_nan() {
        // Java: Math.max(-90, Math.min(90, NaN)) is NaN, and (byte) NaN is 0.
        return 0;
    }
    degrees.ceil().clamp(-90.0, 90.0) as i8
}

/// Whether the point (px, py, pz) stands above the horizon of a radar whose antenna is at
/// (ax, ay, az). A missing horizon (not measured yet) hides nothing.
pub fn above(horizon: Option<&[i8]>, ax: f64, ay: f64, az: f64, px: f64, py: f64, pz: f64) -> bool {
    let Some(horizon) = horizon.filter(|h| h.len() == BINS) else {
        return true;
    };
    let (dx, dz) = (px - ax, pz - az);
    let el = (py - ay).atan2(dx.hypot(dz).max(1.0e-3)).to_degrees();
    el > horizon[bin(dx, dz)] as f64
}
