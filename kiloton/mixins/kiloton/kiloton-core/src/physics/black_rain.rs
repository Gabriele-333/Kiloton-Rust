//! "Black rain" (Java: `BlackRain`): the sticky, radioactive rain that fell over Hiroshima and
//! Nagasaki. The firestorm lifts soot and fallout into a pyrocumulus cloud that drifts downwind and
//! rains out. At Hiroshima (15 kt) it began 20-30 minutes after the burst, lasted hours and covered
//! an oval of about 19 x 11 km downwind; the rain washed the fallout down and left the ground far
//! more contaminated than dry fallout would have.
//!
//! It only happens over wet enough land (the biome's humidity, from 0.15 desert to 1 rainforest)
//! and with a chance that grows with it. The wait is shortened for the game: it starts 1.5-3
//! minutes after the burst and lasts 5-8 minutes.

use super::blast::BlastModel;
use super::fallout::{unit, wind_angle};
use super::{jmax, jmin};

#[derive(Clone, Debug, PartialEq)]
pub struct BlackRain {
    /// Real seconds after the burst.
    pub start: f64,
    pub end: f64,
    /// Downwind distance of the centre of the rain oval (m).
    pub centre: f64,
    /// Half axes of the oval along and across the wind (m).
    pub along: f64,
    pub across: f64,
    /// H+1 dose rate (R/h) the rain-out leaves at the middle of the oval.
    pub peak_h1: f64,
    wx: f64,
    wz: f64,
}

impl BlackRain {
    fn new(m: &BlastModel, seed: i64) -> Self {
        let ang = wind_angle(seed);
        let along = 3000.0 * m.cube_root;
        let start = 90.0 + 90.0 * unit(seed, 11);
        Self {
            wx: ang.cos(),
            wz: ang.sin(),
            along,
            across: 0.6 * along,
            centre: 0.6 * along,
            start,
            end: start + 300.0 + 180.0 * unit(seed, 12),
            peak_h1: 300.0 * m.yield_kt.powf(0.1) * m.fission_fraction,
        }
    }

    /// The black rain of this burst, or none (too dry, too small, or not this time).
    pub fn of(m: &BlastModel, seed: i64, humidity: f64) -> Option<Self> {
        if m.yield_kt < 0.5 {
            return None;
        }
        let chance = jmax(0.0, jmin(0.95, (humidity - 0.3) * 1.6));
        if unit(seed, 10) < chance { Some(Self::new(m, seed)) } else { None }
    }

    /// How hard it rains here, 0..1, ignoring time (a soft-edged oval downwind of ground zero).
    pub fn footprint(&self, dx: f64, dz: f64) -> f64 {
        let s = dx * self.wx + dz * self.wz - self.centre;
        let c = -dx * self.wz + dz * self.wx;
        let q = (s * s) / (self.along * self.along) + (c * c) / (self.across * self.across);
        if q >= 1.0 {
            return 0.0;
        }
        jmin(1.0, 1.4 * (1.0 - q))
    }

    /// How hard it rains overall at this time, 0..1: a minute to build up, a slow end.
    pub fn strength(&self, seconds: f64) -> f64 {
        if seconds <= self.start || seconds >= self.end {
            return 0.0;
        }
        let up = jmin(1.0, (seconds - self.start) / 60.0);
        let down = jmin(1.0, (self.end - seconds) / 120.0);
        up * down
    }

    /// Farthest a point of the rain can be from ground zero.
    pub fn reach(&self) -> f64 {
        self.centre + self.along
    }

    /// H+1 dose rate (R/h) from the fallout the rain brought down here.
    pub fn rainout_h1(&self, dx: f64, dz: f64) -> f64 {
        self.peak_h1 * self.footprint(dx, dz)
    }
}
