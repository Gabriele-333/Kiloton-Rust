//! The flight of a ballistic missile from its pad to its target, as a function of time (Java:
//! `Ballistics`). Server and clients compute the same path from the same numbers, so nothing is sent
//! while it flies.
//!
//! The missile lifts off slowly and straight up, pitches over while the motor burns, coasts over the
//! top of its arc and comes down steeply. At the scale of a Minecraft world: the top of the arc is
//! 30% of the range (300 m at least, 3 km at most) above the higher end, and the flight lasts 12 s
//! plus a second for every 160 m, which leaves time for a radar to see it coming.

use super::sam_geometry;
use super::{jmax, jmin};

/// Fraction of the flight time during which the motor burns (flame and smoke trail).
pub const BURNOUT: f64 = 0.6;
/// Half the missile's length (it is three blocks tall): the path is its middle.
pub const HALF_LENGTH: f64 = 1.5;

#[derive(Clone, Debug, PartialEq)]
pub struct Ballistics {
    pub x0: f64,
    pub y0: f64,
    pub z0: f64,
    pub x1: f64,
    pub y1: f64,
    pub z1: f64,
    pub range: f64,
    pub apex: f64,
    pub seconds: f64,
    /// An interceptor: it slides off its rail along the launcher, speeds up and curves onto the
    /// meeting point; its motor burns all the way.
    pub interceptor: bool,
    /// Interceptor: the curve's control points (a cubic Bezier from the rail to the meeting point).
    c1: [f64; 3],
    c2: [f64; 3],
}

impl Ballistics {
    pub fn new(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Self {
        let range = (x1 - x0).hypot(z1 - z0);
        Self {
            x0,
            y0,
            z0,
            x1,
            y1,
            z1,
            range,
            apex: jmax(y0, y1) + jmax(300.0, jmin(3000.0, 0.3 * range)),
            seconds: Self::flight_seconds(range),
            interceptor: false,
            c1: [0.0; 3],
            c2: [0.0; 3],
        }
    }

    /// An interceptor's flight from its rail to the meeting point, lasting `seconds`.
    pub fn interceptor(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64, seconds: f64) -> Self {
        // Off the rail along the launcher, then onto the meeting point.
        let (dx, dy, dz) = (x1 - x0, y1 - y0, z1 - z0);
        let chord = (dx * dx + dy * dy + dz * dz).sqrt();
        let d = sam_geometry::forward(dx.atan2(dz), sam_geometry::ELEVATION.to_radians());
        let c1 = [x0 + d[0] * 0.35 * chord, y0 + d[1] * 0.35 * chord, z0 + d[2] * 0.35 * chord];
        let c2 = [c1[0] + (x1 - c1[0]) * 0.6, c1[1] + (y1 - c1[1]) * 0.6, c1[2] + (z1 - c1[2]) * 0.6];
        Self {
            x0,
            y0,
            z0,
            x1,
            y1,
            z1,
            range: (x1 - x0).hypot(z1 - z0),
            apex: jmax(y0, y1),
            seconds: jmax(0.5, seconds),
            interceptor: true,
            c1,
            c2,
        }
    }

    /// Seconds the motor burns.
    pub fn burn_seconds(&self) -> f64 {
        if self.interceptor { self.seconds } else { BURNOUT * self.seconds }
    }

    pub fn flight_seconds(range: f64) -> f64 {
        12.0 + range / 160.0
    }

    pub fn ticks(&self) -> i32 {
        (self.seconds * 20.0).ceil() as i32
    }

    /// Position `t` seconds after launch (clamped to the flight).
    pub fn at(&self, t: f64) -> [f64; 3] {
        let u = jmax(0.0, jmin(1.0, t / self.seconds));
        if self.interceptor {
            // It starts from rest on the rail and speeds up (the curve is run through slowly at first).
            let w = u * u * (2.0 - u);
            let a = 1.0 - w;
            let (b0, b1, b2, b3) = (a * a * a, 3.0 * a * a * w, 3.0 * a * w * w, w * w * w);
            return [
                b0 * self.x0 + b1 * self.c1[0] + b2 * self.c2[0] + b3 * self.x1,
                b0 * self.y0 + b1 * self.c1[1] + b2 * self.c2[1] + b3 * self.y1,
                b0 * self.z0 + b1 * self.c1[2] + b2 * self.c2[2] + b3 * self.z1,
            ];
        }
        let s = u * u;
        let f = s * s * (3.0 - 2.0 * s);
        let base = self.y0 + (self.y1 - self.y0) * f;
        let lift = (self.apex - 0.5 * (self.y0 + self.y1)) * 4.0 * s * (1.0 - s);
        [self.x0 + (self.x1 - self.x0) * f, base + lift, self.z0 + (self.z1 - self.z0) * f]
    }

    /// Velocity in blocks per second at time `t`.
    pub fn velocity(&self, t: f64) -> [f64; 3] {
        let dt = 0.05;
        let a = self.at(jmax(0.0, t - dt));
        let b = self.at(t + dt);
        let k = 1.0 / (jmin(t + dt, self.seconds) - jmax(0.0, t - dt) + 1.0e-9);
        [(b[0] - a[0]) * k, (b[1] - a[1]) * k, (b[2] - a[2]) * k]
    }

    pub fn burning(&self, t: f64) -> bool {
        t >= 0.0 && t < self.burn_seconds()
    }

    /// The missile's axis at time `t`, nose first, as a unit vector: straight up on the pad, then
    /// along the way it moves, so it pitches over as it climbs and comes down nose first.
    pub fn axis(&self, t: f64) -> [f64; 3] {
        if self.interceptor {
            // Along the curve, never zero: on the rail it already points the way the launcher does.
            let u = jmax(0.0, jmin(1.0, t / self.seconds));
            let w = u * u * (2.0 - u);
            let a = 1.0 - w;
            let (k0, k1, k2) = (3.0 * a * a, 6.0 * a * w, 3.0 * w * w);
            let p0 = [self.x0, self.y0, self.z0];
            let p3 = [self.x1, self.y1, self.z1];
            let mut tan = [0.0; 3];
            for i in 0..3 {
                tan[i] = k0 * (self.c1[i] - p0[i]) + k1 * (self.c2[i] - self.c1[i]) + k2 * (p3[i] - self.c2[i]);
            }
            let n = (tan[0] * tan[0] + tan[1] * tan[1] + tan[2] * tan[2]).sqrt();
            if n > 1.0e-9 {
                return [tan[0] / n, tan[1] / n, tan[2] / n];
            }
        }
        if t <= 0.0 {
            return [0.0, 1.0, 0.0];
        }
        let v = self.velocity(jmin(t, self.seconds));
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if n < 1.0e-6 {
            return [0.0, 1.0, 0.0];
        }
        [v[0] / n, v[1] / n, v[2] / n]
    }

    /// Where the motor's nozzle is at time `t`.
    pub fn nozzle(&self, t: f64) -> [f64; 3] {
        let p = self.at(t);
        let a = self.axis(t);
        // An interceptor is drawn smaller than the missile.
        let h = if self.interceptor { sam_geometry::MISSILE_SCALE * HALF_LENGTH } else { HALF_LENGTH };
        [p[0] - a[0] * h, p[1] - a[1] * h, p[2] - a[2] * h]
    }
}
