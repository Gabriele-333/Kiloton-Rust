//! What a detonation keeps doing to the world after the blast (Java: `Aftermath`, and the fallout
//! part of `Radiation`): the fallout settles as a film of dust where and when the wind brings it,
//! the black rain leaves soot, and where the dose is lethal the foliage dies and turns red-brown a
//! day later (the Red Forest). Every effect is a function of place and time, so a chunk that loads
//! late is brought up to date at once. The bursts' fallout is kept in `<world>/kiloton/fallout.json`.

use crate::profiles::Tables;
use horizon_client::world::World as GameWorld;
use kiloton_core::blast::profile;
use kiloton_core::physics::fallout;
use kiloton_core::{BlackRain, BlastModel};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const DUST: usize = 0;
pub const RAIN: usize = 1;
pub const DIEBACK: usize = 2;
const CONIFER_LETHAL: f64 = 1000.0;
const BROADLEAF_LETHAL: f64 = 8000.0;
/// Between the lethal dose and the foliage visibly dying, ticks (one in-game day).
const DIEBACK_DELAY: f64 = 24000.0;
/// Chunks in view are checked this often, ticks.
pub const SCAN_PERIOD: i64 = 100;
/// A burst's fallout is forgotten after this many ticks (about 80 in-game days: under a thousandth
/// of its first hour by the 7-10 rule).
const FORGET_AFTER: i64 = 2_000_000;

/// One burst's fallout.
pub struct Source {
    pub dimension: String,
    pub id: u32,
    pub x: f64,
    pub z: f64,
    pub kt: f64,
    pub fission: f64,
    pub seed: i64,
    pub humidity: f64,
    pub sea: bool,
    pub start: i64,
    pub model: BlastModel,
    pub rain: Option<BlackRain>,
    /// The chunks each layer has reached.
    pub done: [HashSet<(i32, i32)>; 3],
    ready: [HashMap<(i32, i32), f64>; 3],
}

impl Source {
    #[allow(clippy::too_many_arguments)]
    pub fn new(dimension: &str, id: u32, x: f64, z: f64, kt: f64, fission: f64, seed: i64, humidity: f64, sea: bool, start: i64) -> Self {
        let model = BlastModel::with_fission(kt, fission);
        let rain = if sea { None } else { BlackRain::of(&model, seed, humidity) };
        Self {
            dimension: dimension.to_owned(),
            id,
            x,
            z,
            kt,
            fission,
            seed,
            humidity,
            sea,
            start,
            model,
            rain,
            done: Default::default(),
            ready: Default::default(),
        }
    }

    fn reach(&self) -> f64 {
        fallout::reach(&self.model).max(self.rain.as_ref().map_or(0.0, |r| r.reach()))
    }

    /// Fallout dose rate (R/h) at a point, before shielding (Java: `Radiation.doseRate`, one record).
    pub fn rate(&self, x: f64, z: f64, now: i64) -> f64 {
        let (dx, dz) = (x - self.x, z - self.z);
        let reach = self.reach();
        if dx * dx + dz * dz > reach * reach {
            return 0.0;
        }
        fallout::dose_rate_with_rain(&self.model, self.seed, self.rain.as_ref(), dx, dz, (now - self.start) as f64)
    }

    /// Game ticks after the burst at which a layer reaches this chunk, or infinity (Java:
    /// `Aftermath.readyDelay`).
    fn ready_delay(&self, layer: usize, cx: i32, cz: i32) -> f64 {
        let m = &self.model;
        let (x0, z0) = (((cx << 4) as f64) - self.x, ((cz << 4) as f64) - self.z);
        let near = ((x0 + 8.0).hypot(z0 + 8.0) - 12.0).max(0.0);
        let px = [x0 + 8.0, x0, x0 + 16.0, x0, x0 + 16.0];
        let pz = [z0 + 8.0, z0, z0, z0 + 16.0, z0 + 16.0];
        match layer {
            DUST => {
                if near > fallout::dust_reach(m) {
                    return f64::INFINITY;
                }
                let (mut cover, mut sec) = (0.0f64, f64::MAX);
                for i in 0..5 {
                    cover = cover.max(fallout::dust_cover(m, self.seed, px[i], pz[i]));
                    sec = sec.min(fallout::dust_seconds(m, self.seed, px[i], pz[i]));
                }
                if cover <= 0.0 { f64::INFINITY } else { sec * 20.0 }
            }
            RAIN => {
                let Some(rain) = &self.rain else { return f64::INFINITY };
                if near > rain.reach() {
                    return f64::INFINITY;
                }
                let fp = (0..5).map(|i| rain.footprint(px[i], pz[i])).fold(0.0f64, f64::max);
                if fp <= 0.05 { f64::INFINITY } else { (rain.start + 60.0) * 20.0 }
            }
            _ => {
                if near > self.reach() {
                    return f64::INFINITY;
                }
                let (mut best, mut bi) = (0.0, 0);
                for i in 0..5 {
                    let d = fallout::total_dose(m, self.seed, self.rain.as_ref(), px[i], pz[i]);
                    if d > best {
                        best = d;
                        bi = i;
                    }
                }
                if best < 0.7 * CONIFER_LETHAL {
                    return f64::INFINITY;
                }
                let h = fallout::hours_to_dose(m, self.seed, self.rain.as_ref(), px[bi], pz[bi], 0.7 * CONIFER_LETHAL);
                h * fallout::TICKS_PER_HOUR + DIEBACK_DELAY
            }
        }
    }
}

fn hash(seed: i64, x: i32, z: i32, salt: i32) -> f64 {
    let mut h = (seed as u64)
        ^ (x as i64 as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ (z as i64 as u64).wrapping_mul(0xC2B2AE3D27D4EB4F)
        ^ (salt as i64 as u64).wrapping_mul(0x165667B19E3779F9);
    h = (h ^ (h >> 31)).wrapping_mul(0xBF58476D1CE4E5B9);
    h = (h ^ (h >> 29)).wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 32;
    (h >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

fn value(seed: i64, x: i32, z: i32, cell: f64, salt: i32) -> f64 {
    let (fx, fz) = (x as f64 / cell, z as f64 / cell);
    let (ix, iz) = (fx.floor() as i32, fz.floor() as i32);
    let (mut tx, mut tz) = (fx - ix as f64, fz - iz as f64);
    tx = tx * tx * (3.0 - 2.0 * tx);
    tz = tz * tz * (3.0 - 2.0 * tz);
    let (a, b) = (hash(seed, ix, iz, salt), hash(seed, ix + 1, iz, salt));
    let (c, d) = (hash(seed, ix, iz + 1, salt), hash(seed, ix + 1, iz + 1, salt));
    (a + (b - a) * tx) + ((c + (d - c) * tx) - (a + (b - a) * tx)) * tz
}

/// Patches a few metres across, so dust and soot lie in drifts (Java: `Aftermath.patches`).
fn patches(seed: i64, x: i32, z: i32, salt: i32) -> f64 {
    0.7 * value(seed, x, z, 6.0, salt) + 0.3 * value(seed, x, z, 2.5, salt + 17)
}

pub const FILM: &str = "kiloton:fallout_dust[soot=false]";
pub const SOOT: &str = "kiloton:fallout_dust[soot=true]";
pub const DEAD_LEAVES: &str = "kiloton:irradiated_leaves";

#[derive(Default)]
pub struct Aftermath {
    pub sources: Vec<Source>,
    /// (source index id, layer, chunk) waiting for their turn.
    queue: Vec<(u32, usize, (i32, i32))>,
    queued: HashSet<(u32, usize, (i32, i32))>,
    /// Blocks to write, by dimension: position and block name.
    pub writes: Vec<(String, [i32; 3], &'static str)>,
    pub dirty: bool,
    pub chunks_done: u64,
    pub blocks_placed: u64,
}

impl Aftermath {
    /// Queues the layers whose time has come in the chunks in view (Java: `Aftermath.check`).
    pub fn scan(&mut self, dimension: &str, world: &GameWorld, now: i64, owes: &dyn Fn(i32, i32) -> bool) {
        self.sources.retain(|s| now - s.start < FORGET_AFTER);
        if self.sources.iter().all(|s| s.dimension != dimension) {
            return;
        }
        let chunks: Vec<(i32, i32)> = world.chunks.keys().copied().collect();
        let (mut soonest, before) = (f64::INFINITY, self.queue.len());
        for s in self.sources.iter_mut().filter(|s| s.dimension == dimension) {
            let reach = s.reach().max(fallout::dust_reach(&s.model)) + 24.0;
            for &(cx, cz) in &chunks {
                let (mx, mz) = (((cx << 4) + 8) as f64 - s.x, ((cz << 4) + 8) as f64 - s.z);
                if mx * mx + mz * mz > reach * reach {
                    continue;
                }
                for layer in 0..3 {
                    if s.done[layer].contains(&(cx, cz)) || self.queued.contains(&(s.id, layer, (cx, cz))) {
                        continue;
                    }
                    let delay = match s.ready[layer].get(&(cx, cz)) {
                        Some(&d) => d,
                        None => {
                            let d = s.ready_delay(layer, cx, cz);
                            s.ready[layer].insert((cx, cz), d);
                            d
                        }
                    };
                    if s.start as f64 + delay > now as f64 {
                        soonest = soonest.min(s.start as f64 + delay - now as f64);
                        continue;
                    }
                    if owes(cx, cz) {
                        continue;
                    }
                    self.queued.insert((s.id, layer, (cx, cz)));
                    self.queue.push((s.id, layer, (cx, cz)));
                }
            }
        }
        if now % 1200 == 0 {
            log::info!(
                "Kiloton: aftermath in {dimension}: {} chunks queued now, next layer due in {:.0} s",
                self.queue.len() - before,
                soonest / 20.0
            );
        }
    }

    /// Works the queue for a few milliseconds of this tick.
    pub fn run(&mut self, dimension: &str, world: &GameWorld, tables: &Tables, budget_ms: f64) {
        let t0 = std::time::Instant::now();
        let mut i = 0;
        while i < self.queue.len() && t0.elapsed().as_secs_f64() * 1000.0 < budget_ms {
            let (id, layer, (cx, cz)) = self.queue[i];
            let Some(si) = self.sources.iter().position(|s| s.id == id && s.dimension == dimension) else {
                i += 1;
                continue;
            };
            self.queue.swap_remove(i);
            self.queued.remove(&(id, layer, (cx, cz)));
            if !world.chunks.contains_key(&(cx, cz)) {
                continue; // unloaded meanwhile: it comes back when it loads
            }
            let before = self.writes.len();
            let s = &self.sources[si];
            match layer {
                DUST => dust(s, world, tables, cx, cz, false, &mut self.writes),
                RAIN => dust(s, world, tables, cx, cz, true, &mut self.writes),
                DIEBACK => dieback(s, world, tables, cx, cz, &mut self.writes),
                _ => {}
            }
            self.blocks_placed += (self.writes.len() - before) as u64;
            self.chunks_done += 1;
            self.sources[si].done[layer].insert((cx, cz));
            self.dirty = true;
        }
    }

    /// Fallout dose rate at a point of a level (R/h, before shielding).
    pub fn rate(&self, dimension: &str, x: f64, z: f64, now: i64) -> f64 {
        self.sources.iter().filter(|s| s.dimension == dimension).map(|s| s.rate(x, z, now)).sum()
    }
}

/// Lays the film of fallout (or, for the black rain, of soot) on the top surface of the chunk.
fn dust(s: &Source, world: &GameWorld, t: &Tables, cx: i32, cz: i32, soot: bool, out: &mut Vec<(String, [i32; 3], &'static str)>) {
    let film = if soot { SOOT } else { FILM };
    for lx in 0..16 {
        for lz in 0..16 {
            let (wx, wz) = ((cx << 4) + lx, (cz << 4) + lz);
            let (dx, dz) = (wx as f64 + 0.5 - s.x, wz as f64 + 0.5 - s.z);
            let chance = if soot {
                s.rain.as_ref().map_or(0.0, |r| 0.6 * r.footprint(dx, dz))
            } else {
                fallout::dust_cover(&s.model, s.seed, dx, dz)
            };
            let v = ((patches(s.seed, wx, wz, if soot { 2 } else { 1 }) - 0.5) * 1.8 + 0.5).clamp(0.0, 1.0);
            if chance <= 0.0 || v >= chance {
                continue;
            }
            let top = world.motion_blocking(wx, wz, |st| t.motion.get(st.0 as usize).copied().unwrap_or(false)) - 1;
            let below = world.block(wx, top, wz).0;
            let bi = t.infos.get(below as usize).copied().unwrap_or(kiloton_core::blast::shape::StateInfo::AIR);
            if bi.wet {
                continue; // it sinks
            }
            if t.dust_states.contains(&below) {
                // Already dusty: the black rain darkens it.
                if soot {
                    out.push((s.dimension.clone(), [wx, top, wz], SOOT));
                }
                continue;
            }
            if bi.profile.category != profile::LEAVES && !bi.full_block {
                continue;
            }
            let at = world.block(wx, top + 1, wz).0;
            let ai = t.infos.get(at as usize).copied().unwrap_or(kiloton_core::blast::shape::StateInfo::AIR);
            let air = t.air.get(at as usize).copied().unwrap_or(false);
            // Short grass and thin snow get buried; anything else keeps the spot.
            if !air && (!t.replaceable.get(at as usize).copied().unwrap_or(false) || ai.wet || t.tall.get(at as usize).copied().unwrap_or(false)) {
                continue;
            }
            out.push((s.dimension.clone(), [wx, top + 1, wz], film));
        }
    }
}

/// Foliage that has taken a lethal dose dies (conifers first: they are ten times more sensitive).
fn dieback(s: &Source, world: &GameWorld, t: &Tables, cx: i32, cz: i32, out: &mut Vec<(String, [i32; 3], &'static str)>) {
    let min_y = world.bottom_y();
    for lx in 0..16 {
        for lz in 0..16 {
            let (wx, wz) = ((cx << 4) + lx, (cz << 4) + lz);
            let dose = fallout::total_dose(&s.model, s.seed, s.rain.as_ref(), wx as f64 + 0.5 - s.x, wz as f64 + 0.5 - s.z);
            let jitter = 0.7 + 0.6 * hash(s.seed, wx, wz, 3);
            if dose < CONIFER_LETHAL * jitter {
                continue;
            }
            let top = world.world_surface(wx, wz, |st| t.air.get(st.0 as usize).copied().unwrap_or(false)) - 1;
            let mut y = top;
            while y > min_y.max(top - 40) {
                let st = world.block(wx, y, wz).0;
                if t.air.get(st as usize).copied().unwrap_or(false) {
                    y -= 1;
                    continue;
                }
                let info = t.infos.get(st as usize).copied().unwrap_or(kiloton_core::blast::shape::StateInfo::AIR);
                if info.profile.category == profile::LEAVES {
                    if !t.dead_leaves.contains(&st) {
                        let lethal = if t.spruce_leaves.contains(&st) { CONIFER_LETHAL } else { BROADLEAF_LETHAL };
                        if dose >= lethal * jitter {
                            out.push((s.dimension.clone(), [wx, y, wz], DEAD_LEAVES));
                        }
                    }
                    y -= 1;
                    continue;
                }
                if info.profile.category == profile::LOG || info.profile.category == profile::PLANT || t.dust_states.contains(&st) {
                    y -= 1;
                    continue;
                }
                break;
            }
        }
    }
}

fn file(world: &Path) -> PathBuf {
    world.join("kiloton").join("fallout.json")
}

pub fn read(world: &Path) -> Vec<Source> {
    let Ok(text) = std::fs::read_to_string(file(world)) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return Vec::new() };
    let mut out = Vec::new();
    for s in v.get("sources").and_then(|s| s.as_array()).into_iter().flatten() {
        let f = |k: &str| s.get(k).and_then(|x| x.as_f64());
        let (Some(dim), Some(id), Some(x), Some(z), Some(kt), Some(fission), Some(seed), Some(hum), Some(start)) = (
            s.get("dimension").and_then(|x| x.as_str()),
            s.get("id").and_then(|x| x.as_u64()),
            f("x"),
            f("z"),
            f("kt"),
            f("fission"),
            s.get("seed").and_then(|x| x.as_i64()),
            f("humidity"),
            s.get("start").and_then(|x| x.as_i64()),
        ) else {
            continue;
        };
        let sea = s.get("sea").and_then(|x| x.as_bool()).unwrap_or(false);
        let mut src = Source::new(dim, id as u32, x, z, kt, fission, seed, hum, sea, start);
        for layer in 0..3 {
            for c in s.get("done").and_then(|d| d.get(layer)).and_then(|d| d.as_array()).into_iter().flatten() {
                if let (Some(cx), Some(cz)) = (c.get(0).and_then(|x| x.as_i64()), c.get(1).and_then(|x| x.as_i64())) {
                    src.done[layer].insert((cx as i32, cz as i32));
                }
            }
        }
        out.push(src);
    }
    out
}

pub fn write(world: &Path, sources: &[Source]) {
    let v = json!({
        "version": 1,
        "sources": sources.iter().map(|s| json!({
            "dimension": s.dimension, "id": s.id, "x": s.x, "z": s.z, "kt": s.kt, "fission": s.fission, "seed": s.seed,
            "humidity": s.humidity, "sea": s.sea, "start": s.start,
            "done": s.done.iter().map(|d| d.iter().map(|&(x, z)| json!([x, z])).collect::<Vec<_>>()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let path = file(world);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}
