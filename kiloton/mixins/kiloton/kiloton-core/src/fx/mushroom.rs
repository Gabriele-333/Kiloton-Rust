//! The mushroom cloud's shape, light and timing as functions of the time since the burst (Java:
//! `fx.MushroomCloud`), the parts the volume renderer and the viewer effects need: the curves and
//! [`MushroomCloud::volume`], which packs the cloud at one instant for the ray-marching shader. The
//! sprite path of the Java edition (`frame`, `glow`) is not here: Horizon draws the cloud as a volume
//! only.

use crate::fx::sky::SkyLight;
use crate::BlastModel;
use std::f64::consts::PI;

const LOBES: usize = 11;

/// Number of floats [`MushroomCloud::volume`] writes: 12 vec4 for the shader (P0..P11).
pub const VOLUME_PARAMS: usize = 48;

#[derive(Clone, Debug)]
pub struct MushroomCloud {
    pub model: BlastModel,
    pub scale: f64,
    /// Final cloud top above ground, blocks.
    pub top_height: f64,
    pub cap_height: f64,
    pub cap_radius: f64,
    pub cap_half_height: f64,
    pub stem_radius: f64,
    pub tau_rise: f64,
    pub tau_form: f64,
    pub tau_heat: f64,
    pub life: f64,
    omega0: f64,
    tau_spin: f64,
    skirt_max: f64,
    /// How much of a megaton cloud this is, 0 below 200 kt, 1 from 3 Mt up.
    pub megaton: f64,
    lobe_x: [f64; LOBES],
}

/// How long the cloud of a burst of this yield stays in the sky, in seconds.
pub fn life_seconds(yield_kt: f64) -> f64 {
    810.0 * yield_kt.powf(0.1)
}

pub fn megaton(yield_kt: f64) -> f64 {
    smooth(2.3, 3.5, yield_kt.max(1.0e-3).log10())
}

impl MushroomCloud {
    pub fn new(model: BlastModel, seed: i64, visual_scale: f64) -> Self {
        let w = model.yield_kt;
        let top_height = (2900.0 * w.powf(0.27) * visual_scale).max(model.fireball_radius * 4.0);
        let cap_height = 0.79 * top_height;
        let mt = megaton(w);
        let cap_radius = ((0.30 + 0.22 * mt) * top_height).max(model.fireball_radius * 1.6);
        let cap_half_height = ((0.135 - 0.03 * mt) * top_height).max(model.fireball_radius * 0.9);
        let stem_radius = ((0.065 + 0.03 * mt) * top_height).max(model.fireball_radius * 0.55);
        let tau_rise = 55.0 * w.powf(0.1);
        let tau_form = 24.0 * w.powf(0.1);
        let tau_heat = 0.8 * w.powf(0.44);
        let life = life_seconds(w);
        let omega0 = 2.0 * PI / (7.0 * w.powf(0.12));
        let tau_spin = 1.4 * tau_rise;
        let skirt_max = (0.55 * model.radius_for_psi(8.0)).min(2.2 * cap_radius);
        let mut rng = Rng(seed);
        let mut lobe_x = [0.0; LOBES];
        for (k, lx) in lobe_x.iter_mut().enumerate() {
            let y = 0.95 - 1.25 * (k as f64 + 0.5) / LOBES as f64;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = k as f64 * 2.399963 + rng.next() * 0.6;
            *lx = r * a.cos();
        }
        Self {
            model,
            scale: visual_scale,
            top_height,
            cap_height,
            cap_radius,
            cap_half_height,
            stem_radius,
            tau_rise,
            tau_form,
            tau_heat,
            life,
            omega0,
            tau_spin,
            skirt_max,
            megaton: mt,
            lobe_x,
        }
    }

    pub fn alive(&self, t: f64) -> bool {
        t < self.life
    }

    /// 1 at detonation, decays as the fireball cools.
    pub fn heat(&self, t: f64) -> f64 {
        (-t.max(0.0) / self.tau_heat).exp()
    }

    fn rise(&self, t: f64) -> f64 {
        let breakaway = 1.0 - (-t / (1.2 * self.model.fireball_time.max(0.3))).exp();
        (1.0 - (-t / self.tau_rise).exp()) * breakaway
    }

    /// Height of the centre of the cap (the rising fireball) above ground.
    pub fn cap_centre(&self, t: f64) -> f64 {
        let h0 = 0.65 * self.model.fireball_radius;
        h0 + (self.cap_height - h0) * self.rise(t.max(0.0))
    }

    /// Centre of the luminous fireball: a hemisphere on the ground that lifts off.
    pub fn fireball_centre(&self, t: f64) -> f64 {
        let lift = smooth(0.0, 1.6 * self.model.yield_kt.powf(0.2), t);
        self.cap_centre(t) * lift
    }

    fn cap_radius_at(&self, t: f64) -> f64 {
        let rf = self.model.fireball_radius;
        let grow = 1.0 - (-t.max(0.0) / (1.1 * self.tau_rise)).exp();
        let spread = (0.55 + 0.9 * self.megaton) * smooth(2.0 * self.tau_rise, 0.55 * self.life, t);
        (rf + (self.cap_radius - rf) * grow) * (1.0 + spread)
    }

    fn cap_half_at(&self, t: f64) -> f64 {
        let rf = self.model.fireball_radius;
        let grow = 1.0 - (-t.max(0.0) / (1.1 * self.tau_rise)).exp();
        let flatten = 1.0 - (0.32 + 0.18 * self.megaton) * smooth(2.0 * self.tau_rise, 0.55 * self.life, t);
        (0.85 * rf + (self.cap_half_height - 0.85 * rf) * grow) * flatten
    }

    fn stem_radius_at(&self, t: f64) -> f64 {
        self.stem_radius * (0.35 + 0.65 * (1.0 - (-t.max(0.0) / self.tau_rise).exp()))
    }

    /// Fraction of a full white-out at the burst itself (the eye's view, not physical power).
    pub fn flash(&self, t: f64) -> f64 {
        if t < 0.0 {
            return 0.0;
        }
        let tau = 0.45 * self.model.yield_kt.powf(0.2);
        let rise = (t / 0.025).min(1.0);
        rise * (-t / tau).exp()
    }

    /// The cloud at time t for the volumetric renderer, packed for the shader (see
    /// `kiloton_cloud.wgsl` for what each slot is). `cx, cy, cz` is the camera relative to ground
    /// zero. None if there is nothing to draw.
    pub fn volume(&self, t: f64, cx: f64, cy: f64, cz: f64, sky: &SkyLight) -> Option<[f32; VOLUME_PARAMS]> {
        if t < 0.0 || t >= self.life {
            return None;
        }
        let m = &self.model;
        let heat = self.heat(t);
        let (hc, rc, chh, rs) = (self.cap_centre(t), self.cap_radius_at(t), self.cap_half_at(t), self.stem_radius_at(t));
        let stem_top = (hc - 0.55 * chh).max(0.0);
        let spin = self.omega0 * self.tau_spin * (1.0 - (-t / self.tau_spin).exp()) + 0.12 * self.omega0 * t;
        let life_fade = 1.0 - smooth(0.62 * self.life, self.life, t);
        let thin = 1.0 - 0.45 * smooth(0.35 * self.life, self.life, t);
        let whiten = smooth(6.0 * m.yield_kt.powf(0.1), 45.0 * m.yield_kt.powf(0.1), t);
        let hot = blackbody(heat);
        let cap_on = smooth(0.5 * m.fireball_time, 3.0 * m.fireball_time + 0.6, t);
        let grow = smooth(1.0, 6.0, t / self.tau_heat);
        let fire_rad = lerp(m.fireball_radius_at(t), 0.42 * rc, grow);
        let fire_on = smooth(0.0, 0.015, t) * (1.0 - smooth(2.2, 5.5, t / self.tau_heat));
        let stem_formed = smooth(1.5, 9.0 * m.yield_kt.powf(0.1), t)
            * (1.0 - 0.55 * smooth(3.0 * self.tau_rise, 0.75 * self.life, t));
        let front = m.front_radius(t);
        let skirt_r = (front * 0.93).min(self.skirt_max * (1.0 + 0.18 * (1.0 - (-t / 80.0).exp())));
        let skirt_on = smooth(0.0, 0.3, t) * smooth(1.5, 6.0, skirt_r) * (1.0 - smooth(0.25 * self.life, 0.7 * self.life, t));
        let skirt_in = smooth(5.0, 75.0 * m.yield_kt.powf(0.08), t);
        let skirt_h = (0.38 * skirt_r).max(5.0).min(0.4 * self.skirt_max) * 0.45;
        let stem_speed = stem_top / (30.0 * m.yield_kt.powf(0.1));
        let v: [f64; VOLUME_PARAMS] = [
            cx, cy, cz, t,
            hc, rc, chh, rs,
            stem_top, spin, heat, life_fade * thin,
            sky.sun_x as f64, sky.sun_y as f64, sky.sun_z as f64, sky.daylight as f64,
            sky.sun_r as f64, sky.sun_g as f64, sky.sun_b as f64, stem_formed,
            sky.amb_r as f64, sky.amb_g as f64, sky.amb_b as f64, sky.visibility as f64,
            sky.haze_r as f64, sky.haze_g as f64, sky.haze_b as f64, 0.35 + 0.65 * sky.daylight as f64,
            lerp(0.50, 0.90, whiten), lerp(0.30, 0.88, whiten), lerp(0.22, 0.86, whiten), cap_on,
            hot[0] as f64, hot[1] as f64, hot[2] as f64, heat.powf(0.6),
            self.fireball_centre(t), fire_rad, fire_on, m.fireball_radius,
            skirt_r, skirt_h, skirt_on, skirt_in,
            (self.lobe_x[0] * 977.0) % 61.0, 1.0, stem_speed, 1.0,
        ];
        let mut out = [0f32; VOLUME_PARAMS];
        for (o, x) in out.iter_mut().zip(v) {
            *o = x as f32;
        }
        Some(out)
    }
}

/// Colour of glowing air as it cools: white, yellow, orange, red, dull ember.
pub fn blackbody(heat: f64) -> [f32; 3] {
    let h = clamp01(heat);
    let (r, g, b);
    if h > 0.75 {
        let k = (h - 0.75) / 0.25;
        r = 1.0;
        g = lerp(0.78, 0.97, k);
        b = lerp(0.45, 0.9, k);
    } else if h > 0.4 {
        let k = (h - 0.4) / 0.35;
        r = 1.0;
        g = lerp(0.45, 0.78, k);
        b = lerp(0.12, 0.45, k);
    } else if h > 0.12 {
        let k = (h - 0.12) / 0.28;
        r = lerp(0.75, 1.0, k);
        g = lerp(0.18, 0.45, k);
        b = lerp(0.05, 0.12, k);
    } else {
        let k = h / 0.12;
        r = lerp(0.2, 0.75, k);
        g = lerp(0.03, 0.18, k);
        b = lerp(0.01, 0.05, k);
    }
    [r as f32, g as f32, b as f32]
}

pub fn smooth(e0: f64, e1: f64, x: f64) -> f64 {
    if x <= e0 {
        return 0.0;
    }
    if x >= e1 {
        return 1.0;
    }
    let k = (x - e0) / (e1 - e0);
    k * k * (3.0 - 2.0 * k)
}

pub(crate) fn lerp(a: f64, b: f64, k: f64) -> f64 {
    a + (b - a) * k
}

pub(crate) fn clamp01(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/// SplitMix64, as the Java edition's `MushroomCloud.Rng`.
struct Rng(i64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15u64 as i64);
        let mut z = self.0 as u64;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        (z >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_grows_and_fades() {
        let c = MushroomCloud::new(BlastModel::new(115.0), 42, 0.75);
        let sky = SkyLight::default();
        let early = c.volume(2.0, 0.0, 10.0, -3000.0, &sky).unwrap();
        let late = c.volume(120.0, 0.0, 10.0, -3000.0, &sky).unwrap();
        assert!(late[4] > early[4], "the head rises");
        assert!(early[10] > late[10], "the fireball cools");
        assert!(c.volume(c.life + 1.0, 0.0, 0.0, 0.0, &sky).is_none());
        assert!(c.flash(0.05) > 0.5 && c.flash(5.0) < 0.1);
    }
}
