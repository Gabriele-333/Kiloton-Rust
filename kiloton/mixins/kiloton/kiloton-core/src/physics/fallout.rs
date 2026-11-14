//! Local fallout of a surface burst (Java: `Fallout`): a plume of contaminated ground downwind of
//! ground zero, plus the activated soil around the crater. Dose rates are in R/h at one hour after
//! the burst ("H+1") and then fall off with the Way-Wigner t^-1.2 law (the "7-10 rule"). Time is in
//! game hours (1000 ticks), so a contaminated area is deadly for a few minutes, dangerous for an
//! in-game day and safe after a couple of in-game weeks.

use super::black_rain::BlackRain;
use super::blast::BlastModel;
use super::{jmax, jmin};
use std::f64::consts::PI;

pub const TICKS_PER_HOUR: f64 = 1000.0;

/// Wind direction at altitude, radians, from the blast seed (shared with the cloud drift).
pub fn wind_angle(seed: i64) -> f64 {
    unit(seed, 1) * PI * 2.0
}

/// Wind speed at altitude, m/s.
pub fn wind_speed(seed: i64) -> f64 {
    4.0 + 6.0 * unit(seed, 2)
}

/// Scale of the H+1 dose rates, R/h: grows slowly with the yield and in proportion to the share of
/// the yield that is fission, the only part that makes fission products.
pub(crate) fn intensity(m: &BlastModel) -> f64 {
    1200.0 * m.yield_kt.powf(0.1) * m.fission_fraction
}

/// Length scale of the plume, metres.
pub fn plume_length(m: &BlastModel) -> f64 {
    900.0 * m.yield_kt.powf(0.45)
}

/// Distance beyond which the dose rate is negligible (used to skip far players).
pub fn reach(m: &BlastModel) -> f64 {
    plume_length(m) * 6.0 + 4.0 * m.crater_radius
}

/// H+1 dose rate (R/h) at an offset from ground zero.
pub fn dose_rate_h1(m: &BlastModel, seed: i64, dx: f64, dz: f64) -> f64 {
    let ang = wind_angle(seed);
    let (wx, wz) = (ang.cos(), ang.sin());
    let s = dx * wx + dz * wz;
    let c = -dx * wz + dz * wx;
    let len = plume_length(m);
    let k = intensity(m);
    let along = if s >= 0.0 { (-s / len).exp() } else { (s / (0.08 * len)).exp() };
    let sigma = 0.1 * len + 0.16 * jmax(s, 0.0) + m.crater_radius;
    let plume = k * along * (-c * c / (2.0 * sigma * sigma)).exp();
    let r2 = dx * dx + dz * dz;
    let hot = 3.0 * jmax(m.crater_radius, 6.0);
    let crater = 2.5 * k * (-r2 / (hot * hot)).exp();
    plume + crater
}

/// Seconds after the burst at which fallout starts to land at this offset.
pub fn arrival_seconds(seed: i64, dx: f64, dz: f64) -> f64 {
    let ang = wind_angle(seed);
    let s = jmax(0.0, dx * ang.cos() + dz * ang.sin());
    45.0 + s / wind_speed(seed)
}

/// Dose rate (R/h) at an offset from ground zero, `ticks` game ticks after the burst.
pub fn dose_rate(m: &BlastModel, seed: i64, dx: f64, dz: f64, ticks: f64) -> f64 {
    let sec = ticks / 20.0;
    let arrive = arrival_seconds(seed, dx, dz);
    // The base surge drops its dust all around ground zero, upwind too, a few seconds to a minute in.
    let surge = surge_h1(m, dx, dz) * decay(ticks) * clamp01((sec - surge_seconds(m, dx, dz)) / 20.0);
    if sec < arrive {
        // Before the plume arrives only the activated crater floor and the surge dust are hot.
        let r2 = dx * dx + dz * dz;
        let hot = 1.5 * jmax(m.crater_radius, 6.0);
        return intensity(m) / 0.6 * (-r2 / (hot * hot)).exp() * decay(ticks) + surge;
    }
    let ramp = jmin(1.0, (sec - arrive) / 30.0);
    dose_rate_h1(m, seed, dx, dz) * decay(ticks) * ramp + surge
}

/// H+1 dose rate (R/h) of the dust the base surge leaves all around ground zero.
pub fn surge_h1(m: &BlastModel, dx: f64, dz: f64) -> f64 {
    let k = intensity(m);
    let r = 0.9 * surge_radius(m);
    0.08 * k * (-(dx * dx + dz * dz) / (r * r)).exp()
}

/// Seconds after the burst at which the surge dust settles here.
pub fn surge_seconds(_m: &BlastModel, dx: f64, dz: f64) -> f64 {
    20.0 + (dx * dx + dz * dz).sqrt() / 25.0
}

fn clamp01(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// Way-Wigner decay relative to H+1, capped at H+0.25 (5.3x) for the first minutes.
pub fn decay(ticks: f64) -> f64 {
    let h = jmax(ticks / TICKS_PER_HOUR, 0.25);
    h.powf(-1.2)
}

/// Dose rate (R/h) including what a black rain washed down. The rain deposits its fallout while it
/// falls, so its share builds up from the start to the end of the rain.
pub fn dose_rate_with_rain(
    m: &BlastModel,
    seed: i64,
    rain: Option<&BlackRain>,
    dx: f64,
    dz: f64,
    ticks: f64,
) -> f64 {
    let mut r = dose_rate(m, seed, dx, dz, ticks);
    if let Some(rain) = rain {
        let sec = ticks / 20.0;
        if sec > rain.start {
            let built = jmin(1.0, (sec - rain.start) / (rain.end - rain.start));
            r += rain.rainout_h1(dx, dz) * decay(ticks) * built;
        }
    }
    r
}

// ---- dust on the ground ------------------------------------------------------------------------

/// Radius of the base surge: the ring of dust a surface burst throws out along the ground.
pub fn surge_radius(m: &BlastModel) -> f64 {
    2.5 * m.fireball_radius
}

/// Chance (0..1) that a column ends up under a visible film of fallout dust: certain inside the
/// base surge, then wherever the fallout is heavy (from 30 R/h at H+1, certain from 1000 R/h).
pub fn dust_cover(m: &BlastModel, seed: i64, dx: f64, dz: f64) -> f64 {
    let d = (dx * dx + dz * dz).sqrt();
    let rs = surge_radius(m);
    let surge = if d <= rs { 1.0 } else { jmax(0.0, 1.0 - (d - rs) / (0.8 * rs)) };
    let h1 = dose_rate_h1(m, seed, dx, dz);
    let plume = if h1 <= 30.0 { 0.0 } else { jmin(1.0, (h1.log10() - 30.0f64.log10()) / 1.5) };
    jmax(surge, plume)
}

/// Seconds after the burst at which the dust settles here: the surge first, then the plume.
pub fn dust_seconds(m: &BlastModel, seed: i64, dx: f64, dz: f64) -> f64 {
    let d = (dx * dx + dz * dz).sqrt();
    let surge = if d <= 1.8 * surge_radius(m) { 20.0 + d / 25.0 } else { f64::MAX };
    jmin(surge, arrival_seconds(seed, dx, dz))
}

/// Farthest point where dust can settle.
pub fn dust_reach(m: &BlastModel) -> f64 {
    let k = intensity(m);
    let plume = if k > 30.0 { plume_length(m) * (k / 30.0).ln() } else { 0.0 };
    jmax(1.8 * surge_radius(m), plume) + 3.0 * jmax(m.crater_radius, 6.0)
}

// ---- dose over time -----------------------------------------------------------------------------

/// The four sources of a spot's dose: H+1 rate and the hour it starts (crater, plume, rain, surge).
fn sources(m: &BlastModel, seed: i64, rain: Option<&BlackRain>, dx: f64, dz: f64) -> [f64; 8] {
    let k = intensity(m);
    let hot = 3.0 * jmax(m.crater_radius, 6.0);
    let crater = 2.5 * k * (-(dx * dx + dz * dz) / (hot * hot)).exp();
    let plume = jmax(0.0, dose_rate_h1(m, seed, dx, dz) - crater);
    let ta = arrival_seconds(seed, dx, dz) * 20.0 / TICKS_PER_HOUR;
    let rain_h1 = rain.map_or(0.0, |r| r.rainout_h1(dx, dz));
    let tr = rain.map_or(1.0, |r| 0.5 * (r.start + r.end) * 20.0 / TICKS_PER_HOUR);
    let ts = surge_seconds(m, dx, dz) * 20.0 / TICKS_PER_HOUR;
    [
        crater,
        0.25,
        plume,
        jmax(ta, 0.25),
        rain_h1,
        jmax(tr, 0.25),
        surge_h1(m, dx, dz),
        jmax(ts, 0.25),
    ]
}

/// Dose (rem) collected by something standing in the open here from the burst until `hours`.
pub fn dose_until(m: &BlastModel, seed: i64, rain: Option<&BlackRain>, dx: f64, dz: f64, hours: f64) -> f64 {
    let s = sources(m, seed, rain, dx, dz);
    let mut d = 0.0;
    for pair in s.chunks_exact(2) {
        let (h1, start) = (pair[0], pair[1]);
        if h1 <= 0.0 || hours <= start {
            continue;
        }
        // Integral of H1 * t^-1.2 from the arrival to now.
        d += 5.0 * h1 * (start.powf(-0.2) - hours.powf(-0.2));
    }
    d
}

/// Dose (rem) collected here over all time.
pub fn total_dose(m: &BlastModel, seed: i64, rain: Option<&BlackRain>, dx: f64, dz: f64) -> f64 {
    dose_until(m, seed, rain, dx, dz, f64::INFINITY)
}

/// Hours after the burst at which the dose here reaches `rem`, or infinity if it never does.
pub fn hours_to_dose(m: &BlastModel, seed: i64, rain: Option<&BlackRain>, dx: f64, dz: f64, rem: f64) -> f64 {
    if total_dose(m, seed, rain, dx, dz) < rem {
        return f64::INFINITY;
    }
    let (mut lo, mut hi) = (0.25f64, 1.0e7f64);
    for _ in 0..60 {
        let mid = (lo * hi).sqrt();
        if dose_until(m, seed, rain, dx, dz, mid) >= rem {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// A uniform number in [0, 1) from a seed and a salt (SplitMix64's finaliser).
pub(crate) fn unit(seed: i64, salt: i32) -> f64 {
    let mut z = (seed as u64).wrapping_add((salt as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}
