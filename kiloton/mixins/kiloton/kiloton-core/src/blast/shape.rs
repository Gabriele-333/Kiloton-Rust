//! What the blast does to one column of the world (Java: `blast.BlastShape`), on a world the game
//! reads and writes for it ([`World`]).
//!
//! A column, top down: inside the crater everything goes and the bowl is lined; elsewhere each
//! block meets the overpressure and the thermal fluence at its distance, shielded by what stands
//! above it and by hills between it and ground zero (the "horizon"), and breaks when they pass
//! its profile's threshold. Trees knocked flat lie down once the sweep is past the ground they fall
//! on; collapsed buildings leave rubble; the flash lights fires; the crater throws out ejecta.

use super::profile::{self, Profile};
use crate::physics::blast::BlastModel;
use std::collections::{HashMap, VecDeque};
use std::f64::consts::PI;

/// What the sweep knows of a block state, beyond its profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateInfo {
    pub profile: Profile,
    /// No axis, or axis y (Java: `upright`).
    pub upright: bool,
    /// No block entity and a full collision cube (Java: `fullBlock`).
    pub full_block: bool,
    /// It holds a fluid (`getFluidState` not empty).
    pub wet: bool,
    /// The source block of its fluid when that fluid is a source (`fs.isSource()`): what stays when
    /// the block itself is blown away (`fs.createLegacyBlock()`).
    pub source: Option<&'static str>,
    /// Ice or frosted ice: a strong flash melts it to water.
    pub melts: bool,
    pub snow_layer: bool,
    pub farmland_or_path: bool,
    pub mycelium_or_podzol: bool,
    pub nylium: bool,
    /// Fire on it is soul fire (`#minecraft:soul_fire_base_blocks`).
    pub soul_base: bool,
    /// Its cracked form, for the rubble of a stone building (Java: `cracked`).
    pub cracked: Option<&'static str>,
    /// One of Kiloton's bombs: never picked as rubble.
    pub nuke: bool,
}

impl StateInfo {
    pub const AIR: StateInfo = StateInfo {
        profile: Profile::AIR,
        upright: true,
        full_block: false,
        wet: false,
        source: None,
        melts: false,
        snow_layer: false,
        farmland_or_path: false,
        mycelium_or_podzol: false,
        nylium: false,
        soul_base: false,
        cracked: None,
        nuke: false,
    };

    fn category(&self) -> u8 {
        self.profile.category
    }
}

/// A block the sweep puts in the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Write {
    Air,
    /// A block by name, in its default state.
    Named(&'static str),
    /// A state as it is (rubble of the building's own blocks).
    State(u32),
    /// A felled trunk: the log state laid along x or z.
    Log { state: u32, along_x: bool },
    /// Fire, or soul fire on soul soil and soul sand.
    Fire { soul: bool },
}

/// The world as the sweep sees it. Reads see the writes made before them.
pub trait World {
    fn loaded(&self, x: i32, z: i32) -> bool;
    /// The y of the column's highest block that isn't air (Java: `getHeight(WORLD_SURFACE)`).
    fn top(&self, x: i32, z: i32) -> i32;
    /// The y of the column's highest block that blocks motion or holds a fluid, leaves left out
    /// (Java: `getHeight(MOTION_BLOCKING_NO_LEAVES)`).
    fn ground(&self, x: i32, z: i32) -> i32;
    fn get(&self, x: i32, y: i32, z: i32) -> u32;
    fn info(&self, state: u32) -> StateInfo;
    /// The default state of a block the sweep writes by name.
    fn named(&self, name: &'static str) -> u32;
    fn set(&mut self, x: i32, y: i32, z: i32, block: Write);
}

/// Above this overpressure trees are shredded and carried off rather than knocked flat (psi).
pub const FALL_MAX_PSI: f64 = 25.0;
/// Longest fallen trunk, in blocks.
pub const MAX_TRUNK: i32 = 9;
/// Collapsed buildings: about a third of their volume stays as rubble below 20 psi, none above 80.
pub const RUBBLE_FRACTION: f64 = 0.3;
/// Pressure behind an obstacle, as a share of what reaches the open.
pub const SHADOW: f64 = 0.6;

/// A tree knocked flat, waiting until the sweep has passed the ground it falls on.
#[derive(Clone, Copy, Debug)]
struct Fall {
    wx: i32,
    wz: i32,
    d: f64,
    ux: f64,
    uz: f64,
    y0: i32,
    len: i32,
    log: u32,
}

/// One burst's sweep.
#[derive(Clone, Debug)]
pub struct Shape {
    pub model: BlastModel,
    pub bx: i32,
    pub by: i32,
    pub bz: i32,
    gy: f64,
    pub seed: i64,
    pub radius: i32,
    pub min_y: i32,
    /// One past the highest block y.
    pub max_y: i32,
    /// Per direction: elevation (tan) of the highest obstacle so far, and how far out it stands.
    pub horizon: Vec<f32>,
    pub horizon_dist: Vec<f32>,
    bins: usize,
    bin_scale: f64,
    psi_table: Vec<f32>,
    heat_table: Vec<f32>,
    pub ground_kind: u8,
    /// Past this distance the effects fade out towards the radius cap instead of stopping dead.
    fade_from: f64,
    pub sea: bool,
    pub block_damage: bool,
    pub fires: bool,
    pub fire_budget: i32,
    pub fires_per_chunk: i32,
    /// Chunks damaged after loading: where the flash would have lit a fire, it has burnt out.
    pub burn_late: bool,
    chunk_fires: HashMap<(i32, i32), i32>,
    falls: VecDeque<Fall>,
    /// The columns just outside the crater whose fluids get a tick, as Java's `edgeFluids` gives
    /// them (x, z, from y down to y): the game side schedules them once the writes are in.
    pub edge_fluids: Vec<(i32, i32, i32, i32)>,
    // Statistics.
    pub columns: u64,
    pub changed: u64,
    pub trees_felled: u64,
    pub rubble_blocks: u64,
    pub fires_placed: u64,
}

impl Shape {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: BlastModel,
        bx: i32,
        by: i32,
        bz: i32,
        seed: i64,
        radius: i32,
        min_y: i32,
        max_y: i32,
        horizon: Option<(Vec<f32>, Vec<f32>)>,
        ground_kind: u8,
    ) -> Self {
        let fade_from =
            if (radius as f64) < model.natural_effect_radius - 1.0 { 0.8 * radius as f64 } else { f64::MAX };
        // 4096 directions: under 7 m apart even 4 km out, enough for the shadow of a hill.
        let b = 64.max(4096.min((2.0 * PI * radius as f64).ceil() as i32)) as usize;
        let (horizon, horizon_dist) = match horizon {
            Some((h, d)) if !h.is_empty() && h.len() == d.len() => (h, d),
            _ => (vec![-1.0e9f32; b], vec![0.0f32; b]),
        };
        let bins = horizon.len();
        let height = max_y - min_y;
        let n = ((radius + height + 8) as f64 * 2.0) as usize + 2;
        let mut psi_table = vec![0.0f32; n];
        let mut heat_table = vec![0.0f32; n];
        for i in 0..n {
            let r = i as f64 * 0.5;
            psi_table[i] = model.overpressure_psi(r.max(0.5)) as f32;
            heat_table[i] = model.thermal_fluence(r.max(0.5)) as f32;
        }
        Self {
            model,
            bx,
            by,
            bz,
            gy: by as f64 + 0.5,
            seed,
            radius,
            min_y,
            max_y,
            horizon,
            horizon_dist,
            bins,
            bin_scale: bins as f64 / (2.0 * PI),
            psi_table,
            heat_table,
            ground_kind,
            fade_from,
            sea: false,
            block_damage: true,
            fires: true,
            fire_budget: 20000,
            fires_per_chunk: 6,
            burn_late: false,
            chunk_fires: HashMap::new(),
            falls: VecDeque::new(),
            edge_fluids: Vec::new(),
            columns: 0,
            changed: 0,
            trees_felled: 0,
            rubble_blocks: 0,
            fires_placed: 0,
        }
    }

    fn psi(&self, r3: f64) -> f32 {
        let i = (r3 * 2.0) as i32 as usize;
        self.psi_table.get(i).copied().unwrap_or(0.0)
    }

    fn heat(&self, r3: f64) -> f32 {
        let i = (r3 * 2.0) as i32 as usize;
        self.heat_table.get(i).copied().unwrap_or(0.0)
    }

    /// Deterministic noise in [0,1) per block, so the same column always breaks the same way.
    pub fn hash(&self, x: i32, y: i32, z: i32) -> f64 {
        let mut h = (self.seed as u64)
            ^ (x as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (y as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ (z as i64 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
        h = (h ^ (h >> 31)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h = (h ^ (h >> 29)).wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 32;
        (h >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    fn bin(&self, dx: i32, dz: i32) -> usize {
        let i = (((dz as f64).atan2(dx as f64) + PI) * self.bin_scale) as i32;
        if i >= self.bins as i32 {
            self.bins - 1
        } else if i < 0 {
            0
        } else {
            i as usize
        }
    }

    fn raise_horizon(&mut self, dx: i32, dz: i32, d: f64, tan: f32) {
        let c = self.bin(dx, dz) as i32;
        let span = (0.5 * self.bin_scale / d).ceil() as i32;
        for k in -span..=span {
            let i = (c + k).rem_euclid(self.bins as i32) as usize;
            if self.horizon[i] < tan || self.shadow_expired(i, d) {
                self.horizon[i] = tan;
                self.horizon_dist[i] = d as f32;
            }
        }
    }

    /// Blast waves bend round obstacles: a shadow only lasts for about ten times the obstacle's
    /// height behind it.
    fn shadow_expired(&self, i: usize, d: f64) -> bool {
        // Java multiplies the two floats as floats.
        let h = (self.horizon[i] * self.horizon_dist[i]) as f64;
        d - self.horizon_dist[i] as f64 > 10.0 * h.max(3.0)
    }

    fn set(&mut self, w: &mut impl World, x: i32, y: i32, z: i32, b: Write) {
        if y < self.min_y || y >= self.max_y {
            return;
        }
        w.set(x, y, z, b);
        self.changed += 1;
    }

    /// Processes one column, (dx, dz) from ground zero. `live` is true during the detonation
    /// itself (horizon updates, fires), false for chunks that load later. False if the chunk is
    /// not loaded.
    pub fn column(&mut self, w: &mut impl World, dx: i32, dz: i32, live: bool) -> bool {
        let (wx, wz) = (self.bx + dx, self.bz + dz);
        if !w.loaded(wx, wz) {
            return false;
        }
        self.columns += 1;
        if !self.block_damage {
            return true;
        }
        let d = ((dx as f64) * (dx as f64) + (dz as f64) * (dz as f64)).sqrt();
        let top = w.top(wx, wz);
        if d < self.model.crater_radius {
            let floor = self.crater(w, wx, wz, d, top);
            // On a slope the ground can fall away below the bowl: what stands there is still inside
            // the fireball and gets the same treatment as the columns around the crater.
            self.blast(w, wx, wz, dx, dz, d, floor - 1, live);
        } else {
            let survivor = self.blast(w, wx, wz, dx, dz, d, top, live);
            if d < 3.6 * self.model.crater_radius {
                self.ejecta(w, wx, wz, d);
            }
            if d < self.model.crater_radius + 3.0 {
                // Java: `edgeFluids`, the water walls left at the crater's edge pour into the hole.
                let floor = (self.by as f64 - 1.0 - self.model.crater_depth).floor() as i32;
                self.edge_fluids.push((wx, wz, top, floor));
            }
            if live && d > 3.0 * self.model.crater_radius && survivor >= self.min_y {
                self.raise_horizon(dx, dz, d, (((survivor + 1) as f64 - self.gy) / d) as f32);
            }
        }
        true
    }

    /// The crater of a column: everything above the bowl goes, the floor is lined. Returns the y of
    /// the crater floor.
    fn crater(&mut self, w: &mut impl World, wx: i32, wz: i32, d: f64, top: i32) -> i32 {
        let depth = self.model.crater_depth_at(d);
        let floor = ((self.by - 1) as f64 - depth + 0.5).floor() as i32;
        let floor = floor.max(self.min_y + 1);
        let mut wet_top = i32::MIN;
        let mut y = top;
        while y > floor {
            let s = w.info(w.get(wx, y, wz));
            if s.category() != profile::AIR && s.category() != profile::INDESTRUCTIBLE {
                if s.wet {
                    wet_top = wet_top.max(y);
                }
                self.set(w, wx, y, wz, Write::Air);
            }
            y -= 1;
        }
        // Line the bowl: fused glass and basalt near the centre, rubble towards the rim.
        let f = w.info(w.get(wx, floor, wz));
        if f.category() != profile::AIR && f.category() != profile::INDESTRUCTIBLE {
            let l = self.lining(wx, floor, wz, d / self.model.crater_radius);
            self.set_named(w, wx, floor, wz, l);
        }
        let f2 = w.info(w.get(wx, floor - 1, wz));
        if f2.category() == profile::TERRAIN && self.hash(wx, floor - 1, wz) < 0.4 {
            let r = self.rubble_block_plain(wx, floor - 1, wz);
            self.set_named(w, wx, floor - 1, wz, r);
        }
        // At sea the bowl fills straight back up to the surface: still water, not a waterfall. Only
        // there: on land a megaton's crater goes down through rivers, lakes and flooded caves, and
        // filling those back left pillars of water a hundred blocks tall.
        if self.sea && wet_top != i32::MIN {
            for y in floor + 1..=wet_top.min(self.by) {
                if w.info(w.get(wx, y, wz)).category() == profile::AIR {
                    self.set(w, wx, y, wz, Write::Named("minecraft:water"));
                }
            }
        }
        floor
    }

    /// Java's `set`: nothing happens where the block already is that one.
    fn set_named(&mut self, w: &mut impl World, x: i32, y: i32, z: i32, name: &'static str) {
        if w.get(x, y, z) != w.named(name) {
            self.set(w, x, y, z, Write::Named(name));
        }
    }

    fn lining(&self, x: i32, y: i32, z: i32, k: f64) -> &'static str {
        let h = self.hash(x, y, z);
        if k < 0.3 && h < 0.12 {
            "minecraft:magma_block"
        } else if h < 0.55 - 0.25 * k {
            "kiloton:trinitite"
        } else if h < 0.7 {
            "minecraft:basalt"
        } else if h < 0.85 {
            "minecraft:gravel"
        } else if y < 0 {
            "minecraft:cobbled_deepslate"
        } else {
            "minecraft:cobblestone"
        }
    }

    /// Java: `rubble(int, int, int)`, under the crater floor.
    fn rubble_block_plain(&self, x: i32, y: i32, z: i32) -> &'static str {
        if self.hash(x, y + 1000, z) < 0.5 {
            "minecraft:gravel"
        } else if y < 0 {
            "minecraft:cobbled_deepslate"
        } else {
            "minecraft:cobblestone"
        }
    }

    /// The blast in one column, from `top` down. Returns the highest surviving solid y, or
    /// `min_y - 1`.
    #[allow(clippy::too_many_arguments)]
    fn blast(&mut self, w: &mut impl World, wx: i32, wz: i32, dx: i32, dz: i32, d: f64, top: i32, live: bool) -> i32 {
        let stop_y = self.min_y.max(self.by - 64);
        let hb = self.bin(dx, dz);
        let hz = if self.shadow_expired(hb, d) { -1.0e9 } else { self.horizon[hb] as f64 };
        // The blast diffracts over small bumps; the fireball is big and high, so only real
        // obstacles hide the ground from its light.
        let tol_blast = 3.0 / d;
        let tol_heat = (2.0 + 0.6 * self.model.fireball_radius) / d;
        let r_vap = self.model.fireball_radius;
        // Near a configured radius cap the effects fade to nothing instead of ending in a hard ring.
        let fade = if d <= self.fade_from {
            1.0
        } else {
            0.0f64.max((self.radius as f64 - d) / (self.radius as f64 - self.fade_from))
        };
        let mut shield = 1.0;
        let mut exposed = true;
        let mut flash_ignites = false;
        let mut survivor = self.min_y - 1;
        let mut ground_y = i32::MIN;
        // Trees knocked flat: how many upright logs came down, and which wood.
        let mut logs_down = 0;
        let mut fall_log: Option<u32> = None;
        // Collapsed buildings: how many blocks came down, one of them for the rubble, the pressure.
        let (mut collapsed, mut collapsed_wood) = (0, 0);
        let mut rubble_src: Option<u32> = None;
        let mut collapse_p = 0.0;
        let mut y = top;
        while y >= stop_y {
            let state = w.get(wx, y, wz);
            let s = w.info(state);
            let c = s.category();
            if c == profile::AIR {
                y -= 1;
                continue;
            }
            if c == profile::INDESTRUCTIBLE {
                survivor = survivor.max(y);
                break;
            }
            let dy = y as f64 + 0.5 - self.gy;
            let r3 = (d * d + dy * dy).sqrt();
            let tan_b = dy / d;
            let shadow = tan_b < hz - tol_blast;
            let dark = tan_b < hz - tol_heat;
            let p = self.psi(r3) as f64 * shield * if shadow { SHADOW } else { 1.0 } * fade;
            let q = if exposed && !dark { self.heat(r3) as f64 * fade } else { 0.0 };

            if c == profile::TERRAIN {
                self.surface(w, wx, y, wz, state, s, r3, p, q);
                survivor = survivor.max(y);
                ground_y = y;
                break;
            }
            if c == profile::FLUID {
                if r3 < r_vap && !self.sea {
                    self.set(w, wx, y, wz, Write::Air);
                    y -= 1;
                    continue;
                }
                break;
            }
            // The flash comes first: an exposed flammable surface may catch fire before the wave.
            if (live || self.burn_late)
                && exposed
                && q > 6.0
                && !flash_ignites
                && s.profile.flammable
                && self.hash(wx, y + 7777, wz) < (0.35f64).min((q - 6.0) / 25.0)
            {
                flash_ignites = true;
            }
            let hot = r3 < r_vap;
            let breaks = if hot {
                true
            } else {
                let jitter = if c == profile::LEAVES {
                    0.45 + 1.1 * self.hash(wx, y, wz)
                } else {
                    0.8 + 0.4 * self.hash(wx, y, wz)
                };
                p >= s.profile.threshold as f64 * jitter
                    || (c == profile::LEAVES && q > 12.0)
                    || (c == profile::PLANT && q > 3.0)
                    || (c == profile::ICE && q > 30.0)
                    || (c == profile::LIGHT && s.snow_layer && q > 2.0)
            };
            if breaks {
                if !hot && c == profile::LOG && p < FALL_MAX_PSI && s.upright {
                    // Blowdown: the tree goes over instead of vanishing. Half of them snap above the
                    // roots and leave a stump; the rest are uprooted.
                    if w.info(w.get(wx, y - 1, wz)).category() == profile::TERRAIN
                        && self.hash(wx, y + 4242, wz) < 0.5
                    {
                        survivor = survivor.max(y);
                        exposed = false;
                        y -= 1;
                        continue;
                    }
                    logs_down += 1;
                    fall_log = Some(state);
                } else if !hot && (c == profile::STRUCTURE || c == profile::WOOD) {
                    collapsed += 1;
                    if c == profile::WOOD {
                        collapsed_wood += 1;
                    }
                    collapse_p = p;
                    // Stairs, slabs and doors do not make a heap: pick the rubble among the solid blocks.
                    if s.full_block && !s.nuke && (rubble_src.is_none() || self.hash(wx, y + 99, wz) < 0.35) {
                        rubble_src = Some(state);
                    }
                }
                let mut after = match s.source {
                    Some(src) if !hot => Write::Named(src),
                    _ => Write::Air,
                };
                if c == profile::ICE && !hot && s.melts && q > 30.0 {
                    after = Write::Named("minecraft:water");
                }
                self.set(w, wx, y, wz, after);
                if profile::blocks_heat(c) {
                    exposed = false;
                }
                y -= 1;
                continue;
            }
            // It holds.
            if c != profile::PLANT && c != profile::LIGHT && c != profile::GLASS {
                survivor = survivor.max(y);
            }
            if profile::blocks_heat(c) {
                exposed = false;
            }
            shield *= profile::shield_factor(c);
            if shield < 0.03 {
                break;
            }
            y -= 1;
        }
        if collapsed >= 2 && ground_y != i32::MIN {
            self.rubble(w, wx, wz, collapsed, collapse_p, rubble_src, 2 * collapsed_wood <= collapsed);
        }
        if logs_down >= 2 && ground_y != i32::MIN {
            if let Some(log) = fall_log {
                self.falls.push_back(Fall {
                    wx,
                    wz,
                    d,
                    ux: dx as f64 / d,
                    uz: dz as f64 / d,
                    y0: ground_y + 1,
                    len: logs_down.min(MAX_TRUNK),
                    log,
                });
                self.trees_felled += 1;
            }
        }
        if flash_ignites && !live {
            self.burnt(w, wx, wz);
        } else if flash_ignites {
            // Whatever still stands where the flash lit the column keeps burning: stripped trunks,
            // surviving canopy, smashed timber, or the bare ground for a moment.
            let nt = w.top(wx, wz);
            let t = w.info(w.get(wx, nt, wz));
            if t.profile.flammable || (t.category() == profile::TERRAIN && self.hash(wx, nt + 31, wz) < 0.3) {
                self.ignite(w, wx, nt + 1, wz);
            }
        }
        survivor
    }

    /// What happens to the ground surface: fused by the fireball, stripped by the wind, scorched by
    /// the flash.
    #[allow(clippy::too_many_arguments)]
    fn surface(&mut self, w: &mut impl World, wx: i32, y: i32, wz: i32, state: u32, s: StateInfo, r3: f64, p: f64, q: f64) {
        let k = s.profile.surface;
        let h = self.hash(wx, y, wz);
        let rf = self.model.fireball_radius;
        let trin = "kiloton:trinitite";
        let to: Option<&'static str> = if r3 < 0.8 * rf {
            match k {
                profile::S_SAND => {
                    if h < 0.45 {
                        Some(trin)
                    } else if h < 0.75 {
                        Some("minecraft:glass")
                    } else {
                        None
                    }
                }
                profile::S_GRASS | profile::S_SOIL => Some(if h < 0.4 {
                    trin
                } else if h < 0.8 {
                    "minecraft:coarse_dirt"
                } else {
                    "minecraft:dirt"
                }),
                profile::S_GRAVEL => (h < 0.4).then_some(trin),
                profile::S_STONE => {
                    if h < 0.25 {
                        Some(trin)
                    } else if h < 0.4 {
                        Some("minecraft:basalt")
                    } else {
                        None
                    }
                }
                _ => (h < 0.3).then_some(trin),
            }
        } else if r3 < rf {
            match k {
                profile::S_SAND => {
                    if h < 0.15 {
                        Some(trin)
                    } else if h < 0.3 {
                        Some("minecraft:glass")
                    } else {
                        None
                    }
                }
                profile::S_GRASS => Some(if h < 0.6 { "minecraft:coarse_dirt" } else { "minecraft:dirt" }),
                profile::S_SOIL => (h < 0.5).then_some("minecraft:coarse_dirt"),
                _ => None,
            }
        } else if k == profile::S_GRASS && (p > 25.0 || q > 3.0 + 9.0 * h) {
            // Stripped by the wind or scorched by the flash.
            let burnt = q > 10.0 + 10.0 * h;
            let mut to = if s.farmland_or_path || !burnt { "minecraft:dirt" } else { "minecraft:coarse_dirt" };
            if s.mycelium_or_podzol {
                to = "minecraft:dirt";
            }
            if s.nylium {
                to = "minecraft:netherrack";
            }
            Some(to)
        } else {
            None
        };
        // Java compares the states: a block already of that kind is left as it is.
        if let Some(to) = to {
            if w.named(to) != state {
                self.set(w, wx, y, wz, Write::Named(to));
            }
        }
    }

    /// Starts a fire. Each chunk takes only a few, so the fires spread over the whole area the
    /// flash reached instead of all going to the first rings around ground zero.
    fn ignite(&mut self, w: &mut impl World, wx: i32, y: i32, wz: i32) {
        if !self.fires || self.fire_budget <= 0 || y >= self.max_y {
            return;
        }
        let key = (wx >> 4, wz >> 4);
        let lit = *self.chunk_fires.get(&key).unwrap_or(&0);
        if lit >= self.fires_per_chunk {
            return;
        }
        if w.info(w.get(wx, y, wz)).category() != profile::AIR {
            return;
        }
        let soul = w.info(w.get(wx, y - 1, wz)).soul_base;
        self.set(w, wx, y, wz, Write::Fire { soul });
        self.chunk_fires.insert(key, lit + 1);
        self.fire_budget -= 1;
        self.fires_placed += 1;
    }

    /// What a fire leaves behind, for a chunk that loads after it would have burnt out: foliage,
    /// timber and trunks lying on the ground are gone, charred trunks still stand, the grass is black.
    fn burnt(&mut self, w: &mut impl World, wx: i32, wz: i32) {
        let mut y = w.top(wx, wz);
        let mut k = 0;
        while k < 32 && y > self.min_y {
            let s = w.info(w.get(wx, y, wz));
            let c = s.category();
            if c == profile::AIR {
                k += 1;
                y -= 1;
                continue;
            }
            if c == profile::LEAVES
                || c == profile::WOOD
                || c == profile::PLANT
                || (c == profile::LOG && !s.upright)
                || (c == profile::LIGHT && s.profile.flammable)
            {
                self.set(w, wx, y, wz, Write::Air);
                k += 1;
                y -= 1;
                continue;
            }
            if c == profile::TERRAIN && s.profile.surface == profile::S_GRASS {
                self.set(w, wx, y, wz, Write::Named("minecraft:coarse_dirt"));
            }
            break;
        }
    }

    /// What is left of a building that came down: a low heap of its own blocks where it stood.
    /// Below 20 psi about a third of the volume stays; faster winds carry more off, none above 80.
    #[allow(clippy::too_many_arguments)]
    fn rubble(&mut self, w: &mut impl World, wx: i32, wz: i32, collapsed: i32, p: f64, src: Option<u32>, stony: bool) {
        let f = if p <= 20.0 { RUBBLE_FRACTION } else { RUBBLE_FRACTION * 0.0f64.max((80.0 - p) / 60.0) };
        let n = 8.min((collapsed as f64 * f + self.hash(wx, 515, wz)).floor() as i32);
        if n <= 0 {
            return;
        }
        let base = w.ground(wx, wz) + 1;
        if w.info(w.get(wx, base - 1, wz)).wet {
            return;
        }
        for k in 0..n {
            let y = base + k;
            if y >= self.max_y {
                break;
            }
            let at = w.info(w.get(wx, y, wz));
            let c = at.category();
            if c != profile::AIR && c != profile::PLANT && c != profile::LIGHT {
                break;
            }
            let Some(b) = self.rubble_block(w, src, stony, wx, y, wz) else { break };
            self.set(w, wx, y, wz, b);
            self.rubble_blocks += 1;
        }
    }

    /// `src` a solid block of the building, or none; `stony` if it was mostly stone or brick.
    fn rubble_block(&self, w: &impl World, src: Option<u32>, stony: bool, x: i32, y: i32, z: i32) -> Option<Write> {
        let h = self.hash(x, y + 2024, z);
        // Smashed timber stays timber; stone and brick break up into cracked blocks, cobble and gravel.
        if let Some(s) = src {
            if w.info(s).category() == profile::WOOD {
                return Some(Write::State(s));
            }
        }
        if src.is_none() && !stony {
            return None;
        }
        if let Some(s) = src {
            if h < 0.5 {
                return Some(match w.info(s).cracked {
                    Some(c) => Write::Named(c),
                    None => Write::State(s),
                });
            }
        }
        Some(Write::Named(if h < 0.8 {
            if y < 0 { "minecraft:cobbled_deepslate" } else { "minecraft:cobblestone" }
        } else {
            "minecraft:gravel"
        }))
    }

    /// Lays down the trees knocked flat by the wave, once the sweep has gone past the ground they
    /// fall on. `up_to` is the distance from ground zero the sweep has finished; `same_chunk_only`
    /// for chunks damaged after loading (only the part of a trunk inside the chunk).
    pub fn flush_falls(&mut self, w: &mut impl World, up_to: f64, same_chunk_only: bool) {
        while let Some(f) = self.falls.front() {
            if f.d + (MAX_TRUNK + 2) as f64 > up_to {
                break;
            }
            let f = self.falls.pop_front().expect("front exists");
            self.lay_trunk(w, f, same_chunk_only);
        }
    }

    /// A trunk lying on the ground, pointing away from ground zero. Blocks only line up with the
    /// axes, so each tree picks x or z with a chance that follows the true direction.
    fn lay_trunk(&mut self, w: &mut impl World, f: Fall, same_chunk_only: bool) {
        let (ax, az) = (f.ux.abs(), f.uz.abs());
        let along_x = self.hash(f.wx, 777, f.wz) * (ax + az) < ax;
        let (sx, sz) = if along_x {
            (if f.ux >= 0.0 { 1 } else { -1 }, 0)
        } else {
            (0, if f.uz >= 0.0 { 1 } else { -1 })
        };
        let (cx0, cz0) = (f.wx >> 4, f.wz >> 4);
        let mut prev_y = f.y0;
        for k in 1..=f.len {
            let (x, z) = (f.wx + sx * k, f.wz + sz * k);
            if same_chunk_only && ((x >> 4) != cx0 || (z >> 4) != cz0) {
                break;
            }
            if !w.loaded(x, z) {
                break;
            }
            let ty = w.ground(x, z) + 1;
            // A trunk bridges small bumps but stops against a wall or at a drop.
            if ty > prev_y + 1 || ty < prev_y - 2 {
                break;
            }
            if w.info(w.get(x, ty - 1, z)).wet {
                break;
            }
            let at = w.info(w.get(x, ty, z));
            let c = at.category();
            if c != profile::AIR && c != profile::PLANT && c != profile::LIGHT {
                break;
            }
            self.set(w, x, ty, z, Write::Log { state: f.log, along_x });
            prev_y = ty;
        }
    }

    /// Debris thrown out of the crater: a raised lip, then a thinning blanket.
    fn ejecta(&mut self, w: &mut impl World, wx: i32, wz: i32, d: f64) {
        let rc = self.model.crater_radius;
        let n = if d < 2.2 * rc {
            (self.model.lip_height_at(d) + (self.hash(wx, 7, wz) - 0.5) * 0.9 + 0.5).floor() as i32
        } else {
            let p = 0.22 * (2.2 * rc / d).powf(3.0);
            if self.hash(wx, 11, wz) < p { 1 } else { 0 }
        };
        if n <= 0 {
            return;
        }
        let ground = w.ground(wx, wz);
        if ground < self.by - 1 - self.model.crater_depth as i32 {
            return;
        }
        // Into the water it sinks: no floating islands of dirt.
        if w.info(w.get(wx, ground, wz)).wet {
            return;
        }
        for k in 1..=n {
            let y = ground + k;
            if y >= self.max_y {
                break;
            }
            if w.info(w.get(wx, y, wz)).category() != profile::AIR {
                continue;
            }
            let b = self.debris(wx, y, wz);
            self.set(w, wx, y, wz, Write::Named(b));
        }
    }

    fn debris(&self, x: i32, y: i32, z: i32) -> &'static str {
        let h = self.hash(x, y + 333, z);
        let trin = "kiloton:trinitite";
        match self.ground_kind {
            profile::S_SAND => {
                if h < 0.55 {
                    "minecraft:sand"
                } else if h < 0.75 {
                    "minecraft:sandstone"
                } else if h < 0.88 {
                    "minecraft:gravel"
                } else {
                    trin
                }
            }
            profile::S_STONE => {
                if h < 0.45 {
                    "minecraft:cobblestone"
                } else if h < 0.75 {
                    "minecraft:gravel"
                } else if h < 0.9 {
                    "minecraft:tuff"
                } else {
                    trin
                }
            }
            _ => {
                if h < 0.32 {
                    "minecraft:dirt"
                } else if h < 0.58 {
                    "minecraft:coarse_dirt"
                } else if h < 0.77 {
                    "minecraft:gravel"
                } else if h < 0.9 {
                    "minecraft:cobblestone"
                } else {
                    trin
                }
            }
        }
    }

    /// Trees still waiting to be laid down.
    pub fn pending_falls(&self) -> usize {
        self.falls.len()
    }
}

/// Every block the sweep writes by name (the game resolves them once).
pub const NAMED: &[&str] = &[
    "minecraft:water",
    "minecraft:lava",
    "kiloton:trinitite",
    "minecraft:magma_block",
    "minecraft:basalt",
    "minecraft:gravel",
    "minecraft:cobblestone",
    "minecraft:cobbled_deepslate",
    "minecraft:glass",
    "minecraft:dirt",
    "minecraft:coarse_dirt",
    "minecraft:netherrack",
    "minecraft:sand",
    "minecraft:sandstone",
    "minecraft:tuff",
    "minecraft:cracked_stone_bricks",
    "minecraft:cracked_deepslate_bricks",
    "minecraft:cracked_deepslate_tiles",
    "minecraft:cracked_polished_blackstone_bricks",
    "minecraft:cracked_nether_bricks",
];

/// Java: `BlastShape.groundKind`: the ground under ground zero, soil if it is no kind of ground.
pub fn ground_kind(surface: u8) -> u8 {
    if surface == profile::S_NONE { profile::S_SOIL } else { surface }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blast::profile::{compute, StateInput};

    /// A flat world of stone with grass on top, oak trees and a timber house, as a test double.
    /// State ids: 0 air, 1 stone, 2 grass block, 3 oak log (y), 4 oak leaves, 5 bedrock, 6 planks,
    /// 7 anything written by name.
    struct Flat {
        writes: HashMap<(i32, i32, i32), Write>,
    }

    fn infos() -> Vec<StateInfo> {
        let mk = |input: StateInput| StateInfo { profile: compute(&input), full_block: true, ..StateInfo::AIR };
        vec![
            StateInfo::AIR,
            mk(StateInput { hardness: 1.5, explosion_resistance: 6.0, terrain: profile::S_STONE, ..Default::default() }),
            mk(StateInput { hardness: 0.6, explosion_resistance: 0.6, terrain: profile::S_GRASS, ..Default::default() }),
            mk(StateInput { hardness: 2.0, explosion_resistance: 2.0, log: true, flammable: true, ..Default::default() }),
            mk(StateInput { hardness: 0.2, explosion_resistance: 0.2, leaves: true, flammable: true, ..Default::default() }),
            mk(StateInput { hardness: -1.0, explosion_resistance: 3_600_000.0, ..Default::default() }),
            mk(StateInput { hardness: 2.0, explosion_resistance: 3.0, timber: true, flammable: true, ..Default::default() }),
            mk(StateInput { hardness: 1.5, explosion_resistance: 6.0, terrain: profile::S_OTHER, ..Default::default() }),
        ]
    }

    impl World for Flat {
        fn loaded(&self, _: i32, _: i32) -> bool {
            true
        }
        fn top(&self, x: i32, z: i32) -> i32 {
            (-64..100).rev().find(|&y| self.get(x, y, z) != 0).unwrap_or(-65)
        }
        fn ground(&self, x: i32, z: i32) -> i32 {
            (-64..100)
                .rev()
                .find(|&y| {
                    let s = self.get(x, y, z);
                    s != 0 && s != 4
                })
                .unwrap_or(-65)
        }
        fn get(&self, x: i32, y: i32, z: i32) -> u32 {
            if let Some(w) = self.writes.get(&(x, y, z)) {
                return match w {
                    Write::Air | Write::Fire { .. } => 0,
                    Write::Named(_) => 7,
                    Write::State(s) | Write::Log { state: s, .. } => *s,
                };
            }
            let tree = x.rem_euclid(7) == 0 && z.rem_euclid(7) == 0;
            let near = |v: i32| matches!(v.rem_euclid(7), 0 | 1 | 6);
            if y == -64 {
                5
            } else if y < 63 {
                1
            } else if y == 63 {
                2
            } else if tree && y < 69 {
                3
            } else if near(x) && near(z) && (67..70).contains(&y) {
                4
            } else if (200..206).contains(&x) && (2..8).contains(&z) && y < 68 {
                6
            } else {
                0
            }
        }
        fn info(&self, state: u32) -> StateInfo {
            infos()[state as usize]
        }
        fn named(&self, _: &'static str) -> u32 {
            7
        }
        fn set(&mut self, x: i32, y: i32, z: i32, block: Write) {
            self.writes.insert((x, y, z), block);
        }
    }

    #[test]
    fn a_forest_goes_down_and_a_house_collapses() {
        let model = BlastModel::new(1.0);
        let radius = model.natural_effect_radius as i32;
        let mut s = Shape::new(model, 0, 64, 0, 7, radius, -64, 320, None, profile::S_SOIL);
        let mut w = Flat { writes: HashMap::new() };
        let mut rings = crate::blast::RingColumns::default();
        for r in 0..=300 {
            rings.clear();
            rings.add_ring(r);
            for &p in &rings.cols {
                s.column(&mut w, crate::blast::ring_columns::dx(p), crate::blast::ring_columns::dz(p), true);
            }
            s.flush_falls(&mut w, (r + 1) as f64, false);
        }
        s.flush_falls(&mut w, f64::MAX, false);
        assert!(s.trees_felled > 50, "trees felled: {}", s.trees_felled);
        let logs_laid = w.writes.values().filter(|b| matches!(b, Write::Log { .. })).count();
        assert!(logs_laid > 50, "trunks laid: {logs_laid}");
        assert!(s.fires_placed > 0, "the flash lit no fire");
        // The timber house at 200 m (about 10 psi for 1 kt) came down and left a heap.
        let standing = (200..206).flat_map(|x| (2..8).map(move |z| (x, z))).filter(|&(x, z)| w.get(x, 67, z) == 6).count();
        assert!(standing < 36, "the house still stands whole");
        assert!(s.rubble_blocks > 0, "no rubble");
        // Bedrock never goes.
        assert_eq!(w.get(0, -64, 0), 5);
    }
}
