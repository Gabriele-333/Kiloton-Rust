//! What a burst detector can tell about a detonation it registers (Java: `BurstDetection`).
//!
//! - Optical (a bhangmeter): it times the fireball's double flash; the time of the second maximum
//!   grows with the yield (Glasstone and Dolan 2.38, 7.86), so with the burst in sight the yield
//!   comes out within about 15%. It needs a line of sight: the burst within [`OPTICAL_RANGE`].
//! - Seismic: farther away only the ground wave is left. mb = 4.45 + 0.75 log10 W, read with an
//!   error of 0.1 to 0.25 units, growing with distance; the P wave runs at about 6 km/s, so a far
//!   station hears the burst a few seconds late.
//!
//! The error of each reading is fixed by the detector and the burst.

pub const OPTICAL_RANGE: f64 = 3000.0;
/// P-wave speed in the crust, blocks (metres) per second.
pub const P_WAVE: f64 = 6000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BurstDetection {
    pub optical: bool,
    pub estimate_kt: f64,
    /// One standard deviation, as a fraction of the estimate (0.15 = plus or minus 15%).
    pub error: f64,
    /// Seconds after the burst at which the detector registers it.
    pub delay: f64,
}

impl BurstDetection {
    /// The reading of a detector `distance` blocks from a burst of `yield_kt`; `salt` fixes the error.
    pub fn of(yield_kt: f64, distance: f64, salt: i64) -> Self {
        let optical = distance <= OPTICAL_RANGE;
        let sigma_log = if optical {
            1.15f64.log10()
        } else {
            (0.1 + 0.15 * (distance / 20000.0).min(1.0)) / 0.75
        };
        let g = gaussian(salt);
        let estimate_kt = yield_kt * 10.0f64.powf(g.clamp(-2.0, 2.0) * sigma_log);
        let error = 10.0f64.powf(sigma_log) - 1.0;
        Self { optical, estimate_kt, error, delay: if optical { 0.0 } else { distance / P_WAVE } }
    }
}

/// A standard normal number from a seed (Box-Muller on two hashed uniforms).
fn gaussian(seed: i64) -> f64 {
    let a = mix(seed as u64);
    let b = mix(seed as u64 ^ 0x9E37_79B9_7F4A_7C15);
    let u = ((a >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
    let v = (b >> 11) as f64 / (1u64 << 53) as f64;
    (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
}

fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
