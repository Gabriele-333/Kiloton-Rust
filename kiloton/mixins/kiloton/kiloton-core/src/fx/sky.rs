//! Lighting of the scene for one frame (Java: `fx.SkyLight`), filled by the client from the level.

#[derive(Clone, Copy, Debug)]
pub struct SkyLight {
    /// Unit vector towards the sun (or the moon at night, with a dim colour).
    pub sun_x: f32,
    pub sun_y: f32,
    pub sun_z: f32,
    pub sun_r: f32,
    pub sun_g: f32,
    pub sun_b: f32,
    /// 0 = night, 1 = full day.
    pub daylight: f32,
    pub amb_r: f32,
    pub amb_g: f32,
    pub amb_b: f32,
    /// Colour the distance haze tends to (the fog / horizon colour).
    pub haze_r: f32,
    pub haze_g: f32,
    pub haze_b: f32,
    /// Distance at which 63% of the colour is haze, metres.
    pub visibility: f32,
    /// 0..1, relative humidity at ground zero: drives the Wilson cloud.
    pub humidity: f32,
}

impl Default for SkyLight {
    fn default() -> Self {
        Self {
            sun_x: 0.0,
            sun_y: 1.0,
            sun_z: 0.0,
            sun_r: 1.0,
            sun_g: 0.97,
            sun_b: 0.9,
            daylight: 1.0,
            amb_r: 0.55,
            amb_g: 0.6,
            amb_b: 0.7,
            haze_r: 0.7,
            haze_g: 0.8,
            haze_b: 1.0,
            visibility: 9000.0,
            humidity: 0.6,
        }
    }
}

impl SkyLight {
    /// Everything from the sun angle (radians, Minecraft's getSunAngle: 0 = noon), the level's sky
    /// brightness (0.2 at night .. 1 at noon) and the rain (0..1).
    pub fn from_sun(&mut self, sun_angle: f32, sky_brightness: f32, rain: f32) {
        let sx = -(sun_angle as f64).sin() as f32;
        let sy = (sun_angle as f64).cos() as f32;
        let mut day = clamp01((sy + 0.12) / 0.35);
        day *= 1.0 - 0.55 * rain;
        if sy >= -0.05 {
            self.sun_x = sx;
            self.sun_y = sy;
        } else {
            // Moonlight comes from the opposite side.
            self.sun_x = -sx;
            self.sun_y = -sy;
        }
        self.sun_z = 0.18;
        let len = ((self.sun_x * self.sun_x + self.sun_y * self.sun_y + self.sun_z * self.sun_z) as f64).sqrt() as f32;
        self.sun_x /= len;
        self.sun_y /= len;
        self.sun_z /= len;
        self.daylight = day;
        let low = clamp01(1.0 - sy.abs() * 2.2);
        if sy >= -0.05 {
            let k = 0.25 + 0.75 * day;
            self.sun_r = lerp(1.0, 1.0, low) * k;
            self.sun_g = lerp(0.97, 0.62, low) * k;
            self.sun_b = lerp(0.9, 0.35, low) * k;
        } else {
            self.sun_r = 0.10;
            self.sun_g = 0.12;
            self.sun_b = 0.18;
        }
        let amb = 0.07 + 0.48 * day * sky_brightness;
        self.amb_r = amb * 0.92;
        self.amb_g = amb * 0.97;
        self.amb_b = amb * (1.05 + 0.1 * (1.0 - day));
    }
}

/// Minecraft's sun angle for a time of day (`DimensionType.timeOfDay` of the overworld, then
/// `getSunAngle`): 0 at noon, in radians.
pub fn sun_angle(day_time: i64) -> f32 {
    let d = (day_time.rem_euclid(24000)) as f64 / 24000.0 - 0.25;
    let f = d - d.floor();
    let e = 0.5 - (f * std::f64::consts::PI).cos() / 2.0;
    let time_of_day = (f * 2.0 + e) / 3.0;
    (time_of_day * std::f64::consts::PI * 2.0) as f32
}

/// The overworld's sky brightness for a sun angle (`Level.getSkyDarken` turned into 0.2..1).
pub fn sky_brightness(sun_angle: f32, rain: f32) -> f32 {
    let c = (sun_angle as f64).cos() as f32;
    let k = clamp01(c * 2.0 + 0.5);
    (0.2 + 0.8 * k) * (1.0 - 0.3 * rain)
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}
