//! The bursts on the client (Java: `ClientBlasts`, `ClientEffects`, `ClientFx`, `FlashOverlay`):
//! what the server tells each player of a detonation, and what the player sees, hears and feels of
//! it. Light is instant; the ground tremor, the blast wave with its sound and the shaking come when
//! they reach the camera (`kiloton_core::fx::viewer`). The cloud itself is drawn by
//! [`crate::cloud_render`] from the same list.

use horizon_client::mod_api::Host;
use kiloton_core::config::ClientConfig;
use kiloton_core::fx::mushroom::{self, MushroomCloud};
use kiloton_core::fx::sky::{self, SkyLight};
use kiloton_core::fx::viewer;
use kiloton_core::BlastModel;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The mod message channel of a detonation (Java: `BlastPayload`).
pub const CHANNEL: &str = "kiloton:blast";


/// One detonation as a client knows it.
pub struct Blast {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub start_tick: i64,
    pub humidity: f32,
    pub cloud: MushroomCloud,
    pub cloud_off: bool,
    tremor_played: bool,
    blast_played: bool,
    rumble_played: bool,
    debris_played: bool,
    /// 1 when the fireball is in direct line of sight from the camera, 0 when hidden (Java: `sight`).
    pub sight: f64,
    flash_seen: bool,
    /// The after-image burnt into the eye: how strong, where on screen (0..1 across and down) and how
    /// big (a share of the screen's height), fixed where the fireball was when it was seen.
    pub after_strength: f64,
    pub after_pos: [f32; 2],
    pub after_size: f32,
    /// The black rain of this burst, or none (Java: `BlackRain.of` with the server's humidity).
    pub rain: Option<kiloton_core::BlackRain>,
}

impl Blast {
    pub fn model(&self) -> &BlastModel {
        &self.cloud.model
    }
}

/// The detonation message: id, position, yield, start tick, seed, humidity, sea, cloud off, fission
/// fraction (little-endian, 58 bytes).
pub fn encode(id: u32, at: [f64; 3], kt: f64, start_tick: i64, seed: i64, humidity: f32, sea: bool, cloud_off: bool, fission: f64) -> Vec<u8> {
    let mut b = Vec::with_capacity(58);
    b.extend_from_slice(&id.to_le_bytes());
    for v in at {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(kt as f32).to_le_bytes());
    b.extend_from_slice(&start_tick.to_le_bytes());
    b.extend_from_slice(&seed.to_le_bytes());
    b.extend_from_slice(&humidity.to_le_bytes());
    b.push(sea as u8);
    b.push(cloud_off as u8);
    b.extend_from_slice(&(fission as f32).to_le_bytes());
    b
}

fn decode(d: &[u8], scale: f64) -> Option<Blast> {
    if d.len() < 58 {
        return None;
    }
    let f64_at = |o: usize| f64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    let f32_at = |o: usize| f32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let i64_at = |o: usize| i64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    let id = u32::from_le_bytes(d[0..4].try_into().unwrap());
    let (x, y, z) = (f64_at(4), f64_at(12), f64_at(20));
    let kt = f32_at(28) as f64;
    let start_tick = i64_at(32);
    let seed = i64_at(40);
    let humidity = f32_at(48);
    let sea = d[52] != 0;
    let cloud_off = d[53] != 0;
    let fission = f32_at(54) as f64;
    if !(x.is_finite() && y.is_finite() && z.is_finite() && kt.is_finite() && kt > 0.0) {
        return None;
    }
    let model = BlastModel::with_fission(kt, fission);
    let rain = if sea { None } else { kiloton_core::BlackRain::of(&model, seed, humidity as f64) };
    Some(Blast {
        id,
        x,
        y,
        z,
        start_tick,
        // At sea: saturated air (a big white condensation cloud).
        humidity: if sea { 1.0 } else { humidity },
        cloud: MushroomCloud::new(model, seed, scale),
        cloud_off,
        tremor_played: false,
        blast_played: false,
        rumble_played: false,
        debris_played: false,
        sight: 1.0,
        flash_seen: false,
        after_strength: 0.0,
        after_pos: [0.5, 0.5],
        after_size: 0.0,
        rain,
    })
}

#[derive(Default)]
pub struct ClientState {
    pub blasts: Vec<Blast>,
    /// The client level's game time at the last client tick, and when that tick ran.
    pub clock: Option<(i64, Instant)>,
    pub dimension: String,
    pub sky: SkyLight,
    /// The 64^3 noise of the volume (textures/effect/cloud_noise.png), RGBA, 528 x 528.
    pub noise: Option<Arc<(u32, u32, Vec<u8>)>>,
    /// White-out and shake of this frame, worked out at the client tick.
    pub whiteout: f32,
    pub shake: f32,
    pub shake_time: f64,
    /// The client settings (`kiloton-client.toml` in the config folder), read at the first tick.
    pub config: ClientConfig,
    config_read: bool,
    /// Glow of the fireball on sky and land (colour, strength) and dust in the air (Java: `ClientFx`).
    pub glow: [f32; 4],
    pub dust: f32,
    /// Black rain falling where the camera is, 0..1 (Java: `ClientFx.soot`).
    pub soot: f32,
    /// The nuclear shadows of this level: id, feet, direction away from the fireball, length.
    pub shadows: Vec<(u32, [f64; 3], f32, f32)>,
    rng: u64,
}

impl ClientState {
    /// Seconds since `b` went off, at this instant (the tick's game time plus the time since it).
    pub fn age(&self, b: &Blast) -> f64 {
        match self.clock {
            Some((gt, at)) => (gt - b.start_tick) as f64 / 20.0 + at.elapsed().as_secs_f64().min(0.05),
            None => -1.0,
        }
    }
}

pub static CLIENT: Mutex<Option<ClientState>> = Mutex::new(None);

pub fn with<R>(f: impl FnOnce(&mut ClientState) -> R) -> R {
    let mut g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(ClientState::default))
}

/// A message from the server half.
pub fn net_from_server(channel: &str, data: &[u8]) {
    if channel != CHANNEL {
        return;
    }
    let scale = with(|c| c.config.cloud_scale);
    let Some(b) = decode(data, scale) else {
        log::warn!("Kiloton: a detonation message that doesn't read ({} bytes)", data.len());
        return;
    };
    with(|c| {
        if let Some(old) = c.blasts.iter_mut().find(|o| o.id == b.id) {
            // Sent again: the cloud was cleared.
            old.cloud_off |= b.cloud_off;
            return;
        }
        log::info!("Kiloton: detonation #{} heard, {} kt at {:.0} {:.0} {:.0}", b.id, b.model().yield_kt, b.x, b.y, b.z);
        c.blasts.push(b);
    });
}

/// A nuclear shadow the server tells of (`kiloton:shadow`: id, feet, angle, length).
pub fn shadow(d: &[u8]) {
    if d.len() < 36 {
        return;
    }
    let f = |o: usize| f64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    let g = |o: usize| f32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let id = u32::from_le_bytes(d[0..4].try_into().unwrap());
    let s = (id, [f(4), f(12), f(20)], g(28), g(32));
    with(|c| {
        if !c.shadows.iter().any(|o| o.0 == id) {
            log::info!("Kiloton: nuclear shadow #{id} at {:.1} {:.1} {:.1}, {:.1} m long", s.1[0], s.1[1], s.1[2], s.3);
            c.shadows.push(s);
        }
    });
}

/// Unit vector of where the camera looks (Minecraft's yaw and pitch, degrees).
fn forward(yaw: f32, pitch: f32) -> [f64; 3] {
    let (y, p) = ((yaw as f64).to_radians(), (pitch as f64).to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}

/// The end of every client tick: the clock, the sky, the sounds as the wave arrives, the white-out
/// and the shaking.
pub fn client_tick(host: &mut dyn Host) {
    let clock = host.clock();
    let camera = host.camera();
    if !with(|c| c.config_read) {
        let cfg = match host.config_dir() {
            Ok(dir) => {
                let path = dir.join("kiloton-client.toml");
                let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
                    let _ = std::fs::write(&path, ClientConfig::TEMPLATE);
                    ClientConfig::TEMPLATE.to_owned()
                });
                ClientConfig::parse(&text)
            }
            Err(_) => ClientConfig::default(),
        };
        log::info!("Kiloton: client settings {cfg:?}");
        with(|c| {
            c.config = cfg;
            c.config_read = true;
        });
    }
    // Line of sight to each fireball, every quarter second and every tick in the first two seconds
    // (Java: `ClientEffects.sight`): a straight line against the client's blocks, the first kilometre.
    if let (Some(cam), Some(clk)) = (camera.as_ref(), clock.as_ref()) {
        let wanted: Vec<(usize, [f64; 3])> = with(|c| {
            c.blasts
                .iter()
                .enumerate()
                .filter_map(|(i, b)| {
                    let t = (clk.game_time - b.start_tick) as f64 / 20.0;
                    (t >= 0.0 && t <= b.cloud.life && (clk.game_time % 5 == 0 || t < 2.0))
                        .then(|| (i, [b.x, b.y + b.cloud.fireball_centre(t).max(2.0), b.z]))
                })
                .collect()
        });
        if !wanted.is_empty() {
            let eye = cam.position;
            let seen: Vec<(usize, f64)> = wanted.into_iter().map(|(i, fb)| (i, sight(host, eye, fb))).collect();
            with(|c| {
                for (i, s) in seen {
                    if let Some(b) = c.blasts.get_mut(i) {
                        b.sight = s;
                    }
                }
            });
        }
    }
    let noise_missing = with(|c| c.noise.is_none());
    if noise_missing {
        if let Some(t) = host.client_resources().texture("kiloton:effect/cloud_noise") {
            with(|c| c.noise = Some(Arc::new((t.width, t.height, t.rgba))));
        }
    }
    let mut sounds: Vec<(&'static str, [f64; 3], f32, f32)> = Vec::new();
    with(|c| {
        let Some(clock) = clock else {
            c.blasts.clear();
            c.clock = None;
            return;
        };
        // (No camera, as on the death screen, is not another dimension: the bursts stay.)
        let dim = camera.as_ref().map(|k| k.dimension.clone()).unwrap_or_else(|| c.dimension.clone());
        if dim != c.dimension {
            // Another world or dimension: the server tells this one's bursts again.
            c.blasts.clear();
            c.shadows.clear();
            c.dimension = dim;
        }
        c.clock = Some((clock.game_time, Instant::now()));
        let angle = sky::sun_angle(clock.day_time);
        let bright = sky::sky_brightness(angle, 0.0);
        c.sky.from_sun(angle, bright, 0.0);
        c.whiteout = 0.0;
        c.shake = 0.0;
        c.dust = 0.0;
        c.glow = [0.0; 4];
        let Some(cam) = camera else { return };
        let flash_strength = c.config.flash_strength;
        let shake_strength = c.config.shake_strength;
        let day = c.sky.daylight as f64;
        let (mut gr, mut gg, mut gb, mut gw, mut dust) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let proj = cam.projection;
        let view = cam.view;
        let eye = cam.position;
        let look = forward(cam.yaw, cam.pitch);
        let mut whiteout = 0.0f64;
        let (mut shake, mut shake_time) = (0.0f64, 0.0f64);
        for i in 0..c.blasts.len() {
            let t = c.age(&c.blasts[i]);
            let b = &mut c.blasts[i];
            if t < 0.0 || t > b.cloud.life {
                continue;
            }
            let m = b.cloud.model.clone();
            let horiz = ((b.x - eye[0]).powi(2) + (b.z - eye[2]).powi(2)).sqrt();
            let dist = (horiz * horiz + (b.y - eye[1]).powi(2)).sqrt();
            // Where the sounds come from: a little way towards the burst (no fall-off with distance:
            // the loudness is worked out here).
            let dir = [b.x - eye[0], b.y - eye[1], b.z - eye[2]];
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            let at = if len < 1.0 { eye } else { [eye[0] + dir[0] / len * 2.0, eye[1] + dir[1] / len * 2.0, eye[2] + dir[2] / len * 2.0] };
            let psi = m.overpressure_psi(dist);
            let tg = viewer::tremor_time(&m, horiz, dist);
            if !b.tremor_played && (tg < 0.0 || t >= tg) {
                b.tremor_played = true;
                let vol = (0.45 * viewer::tremor(&m, horiz)) as f32;
                if tg >= 0.0 && vol >= 0.03 && t < tg + 2.0 {
                    sounds.push(("kiloton:tremor", at, vol, 1.0));
                }
            }
            let ta = m.arrival_time(dist);
            if !b.blast_played && t >= ta {
                b.blast_played = true;
                if t < ta + 3.0 && dist < 60000.0 {
                    let pitch = (1.05 - 0.04 * (1.0 + dist / 100.0).log10()).clamp(0.6, 1.1) as f32;
                    if psi > 2.5 {
                        sounds.push(("kiloton:blast_near", at, 1.0, pitch));
                    } else if psi > 0.25 {
                        sounds.push(("kiloton:blast_far", at, (0.55 + 0.25 * psi) as f32, pitch));
                    } else {
                        sounds.push(("kiloton:blast_distant", at, (0.2 + psi * 3.0).clamp(0.12, 0.9) as f32, pitch));
                    }
                }
            }
            if !b.debris_played && t >= ta + 1.1 {
                b.debris_played = true;
                if psi > 1.2 && t < ta + 4.0 {
                    sounds.push(("kiloton:debris", at, (0.35 + 0.12 * psi).min(1.0) as f32, 1.0));
                }
            }
            if !b.rumble_played && t >= ta + 0.8 {
                b.rumble_played = true;
                if t < ta + 4.0 && dist < 25000.0 {
                    sounds.push(("kiloton:rumble", at, (0.3 + psi * 0.3).clamp(0.15, 0.9) as f32, 0.95));
                }
            }
            if b.after_strength > 0.0 {
                b.after_strength = (b.after_strength - 0.05 / 6.0).max(0.0);
            }
            if !b.cloud_off {
                let fy = b.y + b.cloud.fireball_centre(t);
                let (dx, dy, dz) = (b.x - eye[0], fy - eye[1], b.z - eye[2]);
                let fd = (dx * dx + dy * dy + dz * dz).sqrt();
                let lk = if fd > 1e-3 { (look[0] * dx + look[1] * dy + look[2] * dz) / fd } else { 1.0 };
                let w = viewer::whiteout(&b.cloud, t, fd, lk, b.sight) * flash_strength;
                whiteout = whiteout.max(w);
                // Looking at it when the flash came: the after-image stays where it was on screen.
                if w > 0.45 && lk > 0.8 && b.sight > 0.5 && !b.flash_seen {
                    b.flash_seen = true;
                    b.after_strength = w.min(1.0) * flash_strength;
                    let aspect = (proj[5] / proj[0].max(1e-6)) as f64;
                    let r = viewer::after_image_radius(&b.cloud, t, fd) * proj[5] as f64 * 0.5;
                    b.after_size = (2.0 * r).clamp(0.03, 0.22 * aspect.min(1.0)) as f32;
                    b.after_pos = project(&view, &proj, [dx, dy, dz]).unwrap_or([0.5, 0.5]);
                }
                let g = viewer::glow(&b.cloud, t, fd) * (0.3 + 0.45 * (1.0 - day));
                if g > 0.001 {
                    let hot = mushroom::blackbody(b.cloud.heat(t));
                    gr += hot[0] as f64 * g;
                    gg += hot[1] as f64 * g;
                    gb += hot[2] as f64 * g;
                    gw += g;
                }
                dust = dust.max(viewer::dust(&m, t, horiz, b.cloud.life));
            }
            let sh = viewer::shake(&m, t, horiz, dist, true) * shake_strength;
            if sh > shake {
                shake = sh;
                shake_time = t;
            }
        }
        c.whiteout = whiteout as f32;
        c.shake = shake as f32;
        if gw > 0.0 {
            c.glow = [(gr / gw) as f32, (gg / gw) as f32, (gb / gw) as f32, gw.min(1.0) as f32];
        }
        // The black rain where the camera is: it darkens the air like the dust.
        let mut soot = 0.0f64;
        for b in &c.blasts {
            if let (Some(rain), Some((gt, _))) = (&b.rain, c.clock) {
                let st = rain.strength((gt - b.start_tick) as f64 / 20.0);
                if st > 0.0 {
                    soot = soot.max(st * rain.footprint(eye[0] - b.x, eye[2] - b.z));
                }
            }
        }
        c.soot = soot as f32;
        c.dust = (dust as f32).max(0.75 * c.soot);
        if !c.config.sky_effects {
            c.glow = [0.0; 4];
            c.dust = 0.0;
        }
        c.shake_time = shake_time;
        let now = c.clock;
        c.blasts.retain(|b| match now {
            Some((gt, _)) => ((gt - b.start_tick) as f64 / 20.0) < b.cloud.life + 5.0,
            None => false,
        });
    });
    for (sound, at, volume, pitch) in sounds {
        host.play_local_sound(sound, at, volume, pitch, 0);
    }
    black_rain(host);
}

/// Black rain where the camera is (Java: `ClientEffects.blackRain`): dark drops all around and the hiss of
/// rain. The drops are falling dust of black concrete (the Java edition has a particle of its own).
fn black_rain(host: &mut dyn Host) {
    let Some(cam) = host.camera() else { return };
    let (soot, mut rng) = with(|c| (c.soot as f64, c.rng));
    if soot < 0.02 {
        return;
    }
    if rng == 0 {
        rng = 0x9E3779B97F4A7C15;
    }
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    let drop = horizon_client::particles::ParticleOptions::block(
        horizon_client::particles::ParticleType::FallingDust,
        "block/black_concrete".to_owned(),
        0xFFFF_FFFF,
    );
    let n = (soot * 70.0).min(70.0) as i32;
    let e = cam.position;
    for _ in 0..n {
        let at = [e[0] + (next() - 0.5) * 30.0, e[1] + 4.0 + next() * 14.0, e[2] + (next() - 0.5) * 30.0];
        host.add_particle(&drop, at, [0.0, -0.4, 0.0]);
    }
    if next() < 0.25 * soot + 0.05 {
        let at = [e[0] + (next() - 0.5) * 8.0, e[1], e[2] + (next() - 0.5) * 8.0];
        host.play_local_sound("minecraft:weather.rain", at, (0.15 + 0.35 * soot) as f32, 0.8, 0);
    }
    let state = next().to_bits();
    with(|c| c.rng = state | 1);
}

/// The camera's shake this frame, degrees: yaw, pitch, roll (`partial`: the frame's part of a tick).
pub fn shake_at(partial: f32) -> Option<[f32; 3]> {
    with(|c| {
        let a = c.shake as f64;
        if a < 0.01 {
            return None;
        }
        let t = c.shake_time + partial as f64 / 20.0;
        Some([
            (a * 0.55 * viewer::noise(t, 1.3)) as f32,
            (a * 0.45 * viewer::noise(t, 4.1)) as f32,
            (a * 0.7 * viewer::noise(t, 7.7)) as f32,
        ])
    })
}

/// The white-out of the view, 0..1.
pub fn whiteout() -> f32 {
    with(|c| c.whiteout)
}

/// Where a camera-relative point lands on the screen (0..1 across and down), None behind the camera.
pub fn project(view: &[f32; 16], proj: &[f32; 16], p: [f64; 3]) -> Option<[f32; 2]> {
    let mul = |m: &[f32; 16], v: [f32; 4]| {
        let mut o = [0f32; 4];
        for (r, out) in o.iter_mut().enumerate() {
            *out = m[r] * v[0] + m[4 + r] * v[1] + m[8 + r] * v[2] + m[12 + r] * v[3];
        }
        o
    };
    let c = mul(proj, mul(view, [p[0] as f32, p[1] as f32, p[2] as f32, 1.0]));
    if c[3] <= 0.01 {
        return None;
    }
    let (nx, ny) = (c[0] / c[3], c[1] / c[3]);
    if nx.abs() > 1.6 || ny.abs() > 1.6 {
        return None;
    }
    Some([nx * 0.5 + 0.5, 1.0 - (ny * 0.5 + 0.5)])
}

/// Whether the line from the eye to the fireball is clear of blocks one sees (glass, liquids and air let
/// it through), the first kilometre of it (Java: `ClientEffects.sight`, a VISUAL clip).
fn sight(host: &dyn Host, eye: [f64; 3], fb: [f64; 3]) -> f64 {
    let d = [fb[0] - eye[0], fb[1] - eye[1], fb[2] - eye[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len < 1.0 {
        return 1.0;
    }
    let u = [d[0] / len, d[1] / len, d[2] / len];
    let max = len.min(1000.0);
    let mut last = [i32::MIN; 3];
    let mut along = 0.3;
    while along < max {
        let p = [(eye[0] + u[0] * along).floor() as i32, (eye[1] + u[1] * along).floor() as i32, (eye[2] + u[2] * along).floor() as i32];
        if p != last {
            last = p;
            let name = host.block(p);
            let n = name.split('[').next().unwrap_or("");
            let clear = n.is_empty()
                || n.ends_with(":air")
                || n.ends_with("cave_air")
                || n.ends_with("void_air")
                || n.contains("glass")
                || n.ends_with(":water")
                || n.ends_with(":lava")
                || n.ends_with(":light")
                || n.ends_with(":barrier");
            if !clear {
                return 0.0;
            }
        }
        along += if along < 64.0 { 0.25 } else { 1.0 };
    }
    1.0
}

/// The fog colour tinted by the flash, the glow and the dust (Java: `ClientFx.tintSky`).
pub fn tint_fog(rgb: &mut [f32; 3]) {
    let (glow, dust, white) = with(|c| (c.glow, c.dust as f64, c.whiteout as f64));
    if glow[3] <= 0.0 && dust <= 0.0 && white <= 0.0 {
        return;
    }
    let (mut r, mut g, mut b) = (rgb[0] as f64, rgb[1] as f64, rgb[2] as f64);
    let k = (glow[3] as f64 * 0.6).min(0.6);
    r += (glow[0] as f64 * 0.95 - r) * k;
    g += (glow[1] as f64 * 0.75 - g) * k;
    b += (glow[2] as f64 * 0.6 - b) * k;
    if dust > 0.0 {
        let l = (r + g + b) / 3.0;
        let d = dust * 0.75;
        r += (l * 1.05 + 0.08 - r) * d;
        g += (l * 0.92 + 0.05 - g) * d;
        b += (l * 0.75 + 0.02 - b) * d;
    }
    r += (1.0 - r) * white;
    g += (1.0 - g) * white;
    b += (0.97 - b) * white;
    *rgb = [r.clamp(0.0, 1.0) as f32, g.clamp(0.0, 1.0) as f32, b.clamp(0.0, 1.0) as f32];
}

/// The dust in the air pulls the fog in (Java: `ClientEffects.fog`): new start and end for a fog that
/// ends at `far`, None to leave it.
pub fn fog(near: f32, far: f32) -> Option<(f32, f32)> {
    let dust = with(|c| c.dust);
    if dust < 0.01 {
        return None;
    }
    let target = (far * 0.35).max(48.0);
    let nf = far + (target - far) * dust;
    Some((near.min(nf * 0.05), nf))
}
