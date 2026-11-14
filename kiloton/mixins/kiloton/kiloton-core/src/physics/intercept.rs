//! Where and when an interceptor meets a missile (Java: `Intercept`). The missile's flight is a
//! known function of time ([`Ballistics`]), so the battery does what a fire-control computer does:
//! it predicts the target's path and picks the earliest point its own missile can reach in time,
//! inside its engagement range and above a least altitude, then fires so as to arrive at the same
//! moment. Nothing is simulated in flight: the meeting is decided at launch.

use super::ballistics::Ballistics;
use super::jmax;

/// Average speed of an interceptor, blocks per second.
pub const SPEED: f64 = 500.0;
/// Least height above the battery for a kill (the warhead must not come down on top of it).
pub const MIN_HEIGHT: f64 = 50.0;
/// The meeting is planned at least this far ahead (tracking, decision, leaving the rail), seconds.
pub const REACTION: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intercept {
    /// The missile's age when they meet (seconds since its launch).
    pub meet_age: f64,
    /// How long the interceptor flies.
    pub flight: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Intercept {
    /// The earliest meeting the battery at (bx, by, bz) can still make, if its interceptor were fired
    /// within `window` seconds from now; none if there is none (out of range, too low, or too late).
    ///
    /// `age` is the missile's age now, `range` the engagement range (straight-line distance from the
    /// battery to the meeting point), `after` the earliest meeting age allowed (to space a second
    /// shot after the first).
    #[allow(clippy::too_many_arguments)]
    pub fn solve(
        path: &Ballistics,
        age: f64,
        bx: f64,
        by: f64,
        bz: f64,
        range: f64,
        window: f64,
        after: f64,
    ) -> Option<Intercept> {
        let end = path.seconds - 1.0;
        let mut t = jmax(age + REACTION, after);
        while t < end {
            let p = path.at(t);
            if p[1] >= by + MIN_HEIGHT {
                let (dx, dy, dz) = (p[0] - bx, p[1] - by, p[2] - bz);
                let d = (dx * dx + dy * dy + dz * dz).sqrt();
                if d <= range {
                    let fly = d / SPEED;
                    // Too late from here: even fired now, it cannot be there by then.
                    if age + fly <= t {
                        // Firing would have to wait longer than the window: decide later.
                        let fire_at = t - fly;
                        if fire_at > age + window {
                            return None;
                        }
                        return Some(Intercept { meet_age: t, flight: t - age, x: p[0], y: p[1], z: p[2] });
                    }
                }
            }
            t += 0.1;
        }
        None
    }
}
