//! Effects of a nuclear surface burst as a function of distance (Java: `BlastModel`).
//!
//! - Overpressure: Kinney & Graham's fit for a free-air TNT burst, with an effective charge of 1 kt
//!   of TNT per nuclear kiloton (about half of the yield goes into the blast, and a surface burst
//!   roughly doubles the effective charge through ground reflection). It reproduces Glasstone &
//!   Dolan's surface-burst values: 1 kt gives ~10 psi at ~335 m, 5 psi at ~460 m, 1 psi at ~1.35 km.
//!   Every distance therefore scales with W^(1/3).
//! - Shock front speed: Rankine-Hugoniot for air (gamma = 1.4), U = c0 sqrt(1 + 6/7 dP/P0). Arrival
//!   times are the integral of 1/U.
//! - Fireball: R = 55 m W^0.4 (Glasstone's W^0.4 law for a surface burst).
//! - Crater (dry soil): apparent radius 18 m W^0.3, depth half the radius, lip a quarter of the depth.
//! - Thermal fluence: 25% of the yield radiated, inverse square, light haze.
//! - Initial radiation: inverse square with air attenuation (mean free path ~280 m).

use super::{jmax, jmin};

/// Sea-level atmospheric pressure, psi.
pub const P0_PSI: f64 = 14.696;
/// Speed of sound at sea level, m/s (= blocks per second).
pub const SOUND_SPEED: f64 = 343.0;
/// Windows break around half a psi: the blast stops doing anything to blocks below this.
pub const MIN_EFFECT_PSI: f64 = 0.55;

const TABLE_STEP: f64 = 0.5;

#[derive(Clone, Debug)]
pub struct BlastModel {
    pub yield_kt: f64,
    /// Share of the yield that is fission, 1 for a fission device. The blast, the heat and the cloud
    /// follow the whole yield; the fallout follows the fission.
    pub fission_fraction: f64,
    /// W^(1/3).
    pub cube_root: f64,
    pub crater_radius: f64,
    pub crater_depth: f64,
    pub lip_height: f64,
    pub fireball_radius: f64,
    /// Time for the fireball to reach its full size, seconds.
    pub fireball_time: f64,
    /// Distance where the overpressure falls to [`MIN_EFFECT_PSI`], before any cap.
    pub natural_effect_radius: f64,
    table_radius: f64,
    /// Arrival time of the front every half metre, stored as `f32` like the Java table.
    arrival: Vec<f32>,
}

/// Kinney & Graham, ratio dP/P0.
fn kinney_graham(z: f64) -> f64 {
    let a = z / 4.5;
    let b = z / 0.048;
    let c = z / 0.32;
    let d = z / 1.35;
    808.0 * (1.0 + a * a) / ((1.0 + b * b) * (1.0 + c * c) * (1.0 + d * d)).sqrt()
}

/// Peak side-on overpressure (psi) at `r` for a charge of cube root `cube_root`.
fn overpressure(cube_root: f64, r: f64) -> f64 {
    kinney_graham(jmax(r, 0.05) / (100.0 * cube_root)) * P0_PSI
}

fn shock_speed(cube_root: f64, r: f64) -> f64 {
    SOUND_SPEED * (1.0 + 6.0 / 7.0 * overpressure(cube_root, r) / P0_PSI).sqrt()
}

fn radius_for_psi(cube_root: f64, psi: f64) -> f64 {
    let (mut lo, mut hi) = (0.1f64, 200000.0f64);
    for _ in 0..80 {
        let mid = (lo * hi).sqrt();
        if overpressure(cube_root, mid) > psi {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

impl BlastModel {
    /// A fission burst (all of the yield is fission).
    pub fn new(yield_kt: f64) -> Self {
        Self::with_fission(yield_kt, 1.0)
    }

    pub fn with_fission(yield_kt: f64, fission_fraction: f64) -> Self {
        let fission_fraction = jmax(0.02, jmin(1.0, fission_fraction));
        let yield_kt = jmax(1.0e-4, yield_kt);
        let cube_root = yield_kt.cbrt();
        let crater_radius = 18.0 * yield_kt.powf(0.3);
        let crater_depth = 0.5 * crater_radius;
        let lip_height = 0.25 * crater_depth;
        let fireball_radius = 55.0 * yield_kt.powf(0.4);
        let fireball_time = 0.35 * yield_kt.powf(0.3);
        let natural_effect_radius = radius_for_psi(cube_root, MIN_EFFECT_PSI);

        // Arrival-time table out to a bit beyond the effect radius; past it the front moves at c0.
        let table_radius = natural_effect_radius * 1.25 + 16.0;
        let n = ((table_radius / TABLE_STEP).ceil() as i32 + 1).max(1) as usize;
        let mut arrival = vec![0.0f32; n];
        let mut t = 0.0f64;
        for (i, slot) in arrival.iter_mut().enumerate().skip(1) {
            let r_mid = (i as f64 - 0.5) * TABLE_STEP;
            t += TABLE_STEP / shock_speed(cube_root, r_mid);
            *slot = t as f32;
        }
        Self {
            yield_kt,
            fission_fraction,
            cube_root,
            crater_radius,
            crater_depth,
            lip_height,
            fireball_radius,
            fireball_time,
            natural_effect_radius,
            table_radius,
            arrival,
        }
    }

    /// True for a burst with a fusion stage (megatons, most of it not fission).
    pub fn thermonuclear(&self) -> bool {
        self.fission_fraction < 0.999
    }

    /// Peak side-on overpressure at distance `r` (m), psi.
    pub fn overpressure_psi(&self, r: f64) -> f64 {
        overpressure(self.cube_root, r)
    }

    /// Peak dynamic (wind) pressure behind the front, psi.
    pub fn dynamic_pressure_psi(&self, r: f64) -> f64 {
        let p = self.overpressure_psi(r);
        2.5 * p * p / (7.0 * P0_PSI + p)
    }

    /// Peak wind speed behind the front, m/s (Rankine-Hugoniot particle velocity).
    pub fn wind_speed(&self, r: f64) -> f64 {
        let p = self.overpressure_psi(r) / P0_PSI;
        SOUND_SPEED * 5.0 * p / (7.0 * (1.0 + 6.0 * p / 7.0).sqrt())
    }

    /// Shock front speed at distance `r`, m/s.
    pub fn shock_speed(&self, r: f64) -> f64 {
        shock_speed(self.cube_root, r)
    }

    fn last(&self) -> usize {
        self.arrival.len() - 1
    }

    /// Time for the blast wave (and the sound) to reach distance `r`, seconds.
    pub fn arrival_time(&self, r: f64) -> f64 {
        if r <= 0.0 {
            return 0.0;
        }
        let last = self.last();
        if r >= self.table_radius - TABLE_STEP {
            return self.arrival[last] as f64 + (r - last as f64 * TABLE_STEP) / SOUND_SPEED;
        }
        let f = r / TABLE_STEP;
        let i = f as i32 as usize;
        let k = f - i as f64;
        // Java subtracts the two floats as floats, then widens.
        let (a, b) = (self.arrival[i], self.arrival[i + 1]);
        a as f64 + (b - a) as f64 * k
    }

    /// Radius reached by the blast front after `t` seconds.
    pub fn front_radius(&self, t: f64) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        let last_i = self.last();
        let last = self.arrival[last_i];
        if t >= last as f64 {
            return last_i as f64 * TABLE_STEP + (t - last as f64) * SOUND_SPEED;
        }
        let (mut lo, mut hi) = (0usize, last_i);
        while hi - lo > 1 {
            let mid = (lo + hi) >> 1;
            if self.arrival[mid] as f64 <= t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        // Java subtracts the two floats as floats, then widens.
        let span = (self.arrival[hi] - self.arrival[lo]) as f64;
        let k = if span > 0.0 { (t - self.arrival[lo] as f64) / span } else { 0.0 };
        (lo as f64 + k) * TABLE_STEP
    }

    /// Fireball radius at time `t` (Brode-like t^0.4 growth), metres.
    pub fn fireball_radius_at(&self, t: f64) -> f64 {
        if t >= self.fireball_time {
            return self.fireball_radius;
        }
        self.fireball_radius * (jmax(t, 1.0e-4) / self.fireball_time).powf(0.4)
    }

    /// Thermal fluence at distance `r`, cal/cm^2 (no shadowing: the caller checks line of sight).
    pub fn thermal_fluence(&self, r: f64) -> f64 {
        let rr = jmax(r, 1.0);
        self.yield_kt * 1.989e6 / (rr * rr) * (-rr / 18000.0).exp()
    }

    /// Prompt (gamma + neutron) dose at distance `r`, rem, unshielded.
    pub fn initial_dose_rem(&self, r: f64) -> f64 {
        let km = jmax(r, 30.0) / 1000.0;
        3000.0 * self.yield_kt * (-jmax(r, 30.0) / 280.0).exp() / (km * km)
    }

    /// Distance where the overpressure falls to `psi` (bisection on the monotone fit).
    pub fn radius_for_psi(&self, psi: f64) -> f64 {
        radius_for_psi(self.cube_root, psi)
    }

    /// Distance where the thermal fluence falls to the given value.
    pub fn radius_for_fluence(&self, cal_per_cm2: f64) -> f64 {
        let (mut lo, mut hi) = (1.0f64, 500000.0f64);
        for _ in 0..80 {
            let mid = (lo * hi).sqrt();
            if self.thermal_fluence(mid) > cal_per_cm2 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// Distance where the prompt dose falls to the given value.
    pub fn radius_for_dose(&self, rem: f64) -> f64 {
        let (mut lo, mut hi) = (30.0f64, 100000.0f64);
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if self.initial_dose_rem(mid) > rem {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// Crater floor depth below ground zero at horizontal distance `d` (paraboloid bowl), 0 outside.
    pub fn crater_depth_at(&self, d: f64) -> f64 {
        if d >= self.crater_radius {
            return 0.0;
        }
        let k = d / self.crater_radius;
        self.crater_depth * (1.0 - k * k)
    }

    /// Height of the ejecta lip above the original ground at horizontal distance `d`.
    pub fn lip_height_at(&self, d: f64) -> f64 {
        if d < self.crater_radius {
            return 0.0;
        }
        let k = self.crater_radius / d;
        self.lip_height * k * k * k
    }

    /// Damage to an unprotected creature from the blast, in half-hearts.
    pub fn blast_damage(psi: f64) -> f64 {
        if psi < 0.8 {
            return 0.0;
        }
        2.0 * psi.powf(1.15)
    }
}
