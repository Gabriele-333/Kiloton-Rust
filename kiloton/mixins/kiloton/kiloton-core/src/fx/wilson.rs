//! The Wilson cloud: the condensation dome that follows the blast wave out from a burst in moist air
//! (Java: `fx.WilsonCloud`). Drawn by the volume shader (P13) in the same ray march as the cloud.

use crate::BlastModel;

/// Crossroads Baker's yield, the reference of the times below.
const REF_KT: f64 = 23.0;
const T_FORM: f64 = 0.5;
const T_FULL: f64 = 1.4;
const T_PEAK: f64 = 2.2;
const T_GONE: f64 = 4.2;

/// The envelope at `t` seconds: forms, holds briefly, evaporates (0 before and after).
pub fn envelope(yield_kt: f64, t: f64) -> f64 {
    let s = (yield_kt.max(1.0e-3) / REF_KT).cbrt();
    let a = t / s;
    if a <= T_FORM || a >= T_GONE {
        return 0.0;
    }
    if a < T_FULL {
        return smooth((a - T_FORM) / (T_FULL - T_FORM));
    }
    if a < T_PEAK {
        return 1.0;
    }
    1.0 - smooth((a - T_PEAK) / (T_GONE - T_PEAK))
}

/// How readily it forms for a ground-zero humidity (0.15 desert .. 1 sea).
pub fn humidity_factor(humidity: f64) -> f64 {
    let h = (humidity - 0.3) / 0.6;
    0.03 + 0.97 * h.clamp(0.0, 1.0)
}

/// The shell at `t`: radius, half thickness, phase (0..1 forming, 1..2 evaporating), amount; all
/// zero when there is none. `strength` is the client setting (0 off, 1 realistic, 2 exaggerated).
pub fn params(m: &BlastModel, t: f64, humidity: f64, strength: f64) -> [f32; 4] {
    let env = envelope(m.yield_kt, t);
    let s = (m.yield_kt.max(1.0e-3) / REF_KT).cbrt();
    let going = t / s > T_PEAK;
    let amount = strength * humidity_factor(humidity);
    if env <= 0.0 || amount <= 0.001 {
        return [0.0; 4];
    }
    let front = m.front_radius(t);
    [
        (0.87 * front) as f32,
        (0.08 * front) as f32,
        (if going { 2.0 - env } else { env }) as f32,
        amount as f32,
    ]
}

fn smooth(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}
