//! The physics of a burst and of the hardware around it (Java: `net.gabriele333.kiloton.physics`).

pub mod ballistics;
pub mod black_rain;
pub mod blast;
pub mod burst_detection;
pub mod fallout;
pub mod fission;
pub mod intercept;
pub mod radar_horizon;
pub mod sam_geometry;
pub mod thermonuclear;
pub mod warhead;

/// Java's `Math.signum`: 0 for 0 (Rust's `signum` gives 1).
#[inline]
pub(crate) fn signum(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

/// Java's `Math.max(a, b)` for doubles: NaN if either is NaN (Rust's `max` ignores a NaN).
#[inline]
pub(crate) fn jmax(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

/// Java's `Math.min(a, b)` for doubles: NaN if either is NaN.
#[inline]
pub(crate) fn jmin(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}
