//! The bursts on the server (Java: `BlastManager`, `BlastJob`): the sweep of each burst through its
//! level, the chunks it finds unloaded (remembered one bit each, damaged when they load), and what
//! is owed when the world closes before a sweep ends.
//!
//! Each level tick has two halves. At its head (the `level-read` injection) the sweep reads the
//! level's blocks directly and works the columns out with `kiloton_core::blast::shape`, writing into
//! an overlay; right after (the `level_tick_post` hook, with the Host) the overlay goes into the
//! world: one bulk edit (`edit_blocks`, which tells no neighbour, as Java writes the sections), and
//! `set_block` for fires (so they tick) and for states that can't be named in a bulk edit. The next
//! tick reads the columns at the crater's edge back and the `level-ticks` injection gives their
//! fluids a tick, so the water there pours in (Java: `edgeFluids`).

use crate::profiles::Tables;
use horizon_client::mod_api::{Host, NativeBlockEdit, NativeBlockRun, Text};
use horizon_client::world::World as GameWorld;
use kiloton_core::blast::shape::{self, Shape, StateInfo, World, Write};
use kiloton_core::blast::{profile, LateGrid, RingColumns};
use kiloton_core::config::ServerConfig;
use kiloton_core::format::{fixed, nuke_yield};
use kiloton_core::BlastModel;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Server time the bursts may take each tick, ms (Java: `tickBudgetMs`), three times as much (at
/// most 45) while the destruction trails the front.
const TICK_BUDGET_MS: f64 = 12.0;
/// From this ring on the sweep goes by bands of sixteen rings (Java: `BlastJob.FAR_FROM`).
const FAR_FROM: i32 = 256;
/// Most bursts that may owe damage to unloaded chunks (Java: `lateBlastLimit`).
const LATE_BLAST_LIMIT: usize = 32;

/// A burst asked for and not started yet: where it was asked, by whom.
struct Request {
    dimension: String,
    at: [f64; 3],
    kt: f64,
    fission: f64,
    player: u128,
}

/// Everything about a burst that outlives its sweep (Java: `BlastSavedData.Record`).
#[derive(Clone, Debug)]
pub struct Record {
    pub id: u32,
    pub dimension: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub kt: f64,
    pub fission: f64,
    pub seed: i64,
    pub radius: i32,
    pub sea: bool,
    pub ground_kind: u8,
    pub horizon: Vec<f32>,
    pub horizon_dist: Vec<f32>,
    /// Chunks owed whole, and chunks owed in part (256-bit masks of columns).
    pub pending: LateGrid,
    pub partial: HashMap<(i32, i32), [u64; 4]>,
}

impl Record {
    pub fn has_late(&self) -> bool {
        self.pending.count() > 0 || !self.partial.is_empty()
    }

    fn owes(&self, cx: i32, cz: i32) -> bool {
        self.pending.get(self.pending.index(cx, cz)) || self.partial.contains_key(&(cx, cz))
    }
}

/// One running sweep (Java: `BlastJob`).
struct Job {
    record: Record,
    shape: Shape,
    ticks: u64,
    ring: i32,
    cols: RingColumns,
    cursor: usize,
    built_ring: i32,
    band_start: i32,
    live: Vec<(i32, i32)>,
    arrivals: Vec<(i32, i32)>,
    touched: LateGrid,
    sweep_done: bool,
    nanos: u128,
    worst: u128,
    quiet_rings: u64,
    rejoined: u64,
    /// What it has done to the creatures (`entities`).
    ents: crate::entities::EntityWork,
}

impl Job {
    fn age(&self) -> f64 {
        self.ticks as f64 / 20.0
    }

    /// True while the destruction trails the physical blast front.
    fn behind(&self) -> bool {
        !self.arrivals.is_empty()
            || (!self.sweep_done
                && ((self.ring + 24) as f64) < (self.record.radius as f64).min(self.shape.model.front_radius(self.age())))
    }
}

/// The writes of this tick, waiting for the Host.
#[derive(Default)]
struct Overlay {
    blocks: HashMap<(i32, i32, i32), (u32, Write)>,
    /// The highest y written in each column, for the heights.
    tops: HashMap<(i32, i32), i32>,
}

#[derive(Default)]
pub struct Blasts {
    requests: Vec<Request>,
    jobs: Vec<Job>,
    /// Bursts done sweeping that still owe chunks, with those read back from the save.
    pub late: Vec<Record>,
    /// Chunks to damage for a late record: (record id, chunk).
    late_queue: Vec<(u32, (i32, i32))>,
    late_shapes: HashMap<u32, Shape>,
    /// Ticks of each level, for the once-a-second look at the chunks the late records owe.
    late_ticks: HashMap<String, u64>,
    /// Late chunks damaged since the last log line, by burst.
    late_done: HashMap<u32, u32>,
    overlay: Overlay,
    /// Columns just outside a crater whose fluids get a tick once this tick's writes are in
    /// (Java: `edgeFluids`): dimension, x, z, from y down to y.
    edge_scan: Vec<(String, i32, i32, i32, i32)>,
    /// The fluid ticks the `level-ticks` injection schedules: dimension, position, kind (1 water,
    /// 2 flowing water, 3 lava, 4 flowing lava) and delay.
    pub fluid_ticks: Vec<(String, [i32; 3], u8, i32)>,
    /// Bursts started this tick, for the players of their level (Java: `BlastPayload`).
    announce: Vec<Record>,
    /// Bursts whose cloud is still in the sky, with their message, start tick and the cloud's life in
    /// ticks: told to players who come into their level later (Java: `BlastSync`). Not saved.
    clouds: Vec<(String, u32, Vec<u8>, i64, i64)>,
    /// Which player has been told which burst (taken out when the player leaves that level).
    told: HashSet<(u32, u128)>,
    /// The creatures of each level at the last Host half, for the next tick's decisions.
    snaps: HashMap<String, Vec<crate::entities::Snap>>,
    /// What the Host half is to do to them.
    orders: Vec<(String, crate::entities::Order)>,
    /// Fallout doses worked out at the head of the tick: dimension, UUID, entity id, player, rate, ticks.
    fallout_doses: Vec<(String, u128, u32, bool, bool, f64, f64)>,
    pub bodies: crate::entities::Bodies,
    pub aftermath: crate::aftermath::Aftermath,
    /// Each level's game time at its last Host half.
    now: HashMap<String, i64>,
    /// The nuclear shadows in the world, and which player has been told which.
    pub shadows: Vec<crate::entities::Shadow>,
    told_shadows: HashSet<(u32, u128)>,
    /// Lines for players, sent from the Host's half of the tick.
    tells: Vec<(u128, Text)>,
    next_id: u32,
    /// Set when the save must be written again.
    pub dirty: bool,
    pub config: Option<ServerConfig>,
}

pub static BLASTS: Mutex<Option<Blasts>> = Mutex::new(None);

pub fn with<R>(f: impl FnOnce(&mut Blasts) -> R) -> R {
    let mut g = BLASTS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| Blasts { next_id: 1, ..Default::default() }))
}

/// The level as the sweep sees it: the game's blocks under this tick's writes.
struct LevelWorld<'a> {
    world: &'a GameWorld,
    tables: &'a Tables,
    overlay: &'a mut Overlay,
}

impl LevelWorld<'_> {
    fn base(&self, x: i32, z: i32, motion: bool) -> i32 {
        let t = self.tables;
        let h = if motion {
            self.world.motion_blocking(x, z, |s| t.motion.get(s.0 as usize).copied().unwrap_or(false))
        } else {
            self.world.world_surface(x, z, |s| t.air.get(s.0 as usize).copied().unwrap_or(false))
        } - 1;
        match self.overlay.tops.get(&(x, z)) {
            None => h,
            Some(&written) => {
                // A write above the old height can raise it, and writes of air can lower it.
                let mut y = h.max(written);
                let min = self.world.bottom_y();
                while y >= min {
                    let s = self.get(x, y, z) as usize;
                    let counts = if motion {
                        t.motion.get(s).copied().unwrap_or(false)
                    } else {
                        !t.air.get(s).copied().unwrap_or(true)
                    };
                    if counts {
                        return y;
                    }
                    y -= 1;
                }
                min - 1
            }
        }
    }
}

impl World for LevelWorld<'_> {
    fn loaded(&self, x: i32, z: i32) -> bool {
        self.world.loaded(x, z)
    }
    fn top(&self, x: i32, z: i32) -> i32 {
        self.base(x, z, false)
    }
    fn ground(&self, x: i32, z: i32) -> i32 {
        self.base(x, z, true)
    }
    fn get(&self, x: i32, y: i32, z: i32) -> u32 {
        match self.overlay.blocks.get(&(x, y, z)) {
            Some(&(id, _)) => id,
            None => self.world.block(x, y, z).0,
        }
    }
    fn info(&self, state: u32) -> StateInfo {
        self.tables.infos.get(state as usize).copied().unwrap_or(StateInfo::AIR)
    }
    fn named(&self, name: &'static str) -> u32 {
        self.tables.named.get(name).copied().unwrap_or(u32::MAX)
    }
    fn set(&mut self, x: i32, y: i32, z: i32, block: Write) {
        let id = match block {
            Write::Air => self.tables.air_id,
            Write::Named(n) => self.named(n),
            Write::State(s) | Write::Log { state: s, .. } => s,
            Write::Fire { soul } => {
                if soul {
                    self.tables.soul_fire
                } else {
                    self.tables.fire
                }
            }
        };
        self.overlay.blocks.insert((x, y, z), (id, block));
        let top = self.overlay.tops.entry((x, z)).or_insert(y);
        *top = (*top).max(y);
    }
}

fn seed(id: u32, x: i32, y: i32, z: i32) -> i64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut h = t ^ (id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for v in [x, y, z] {
        h = (h ^ v as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
    }
    h as i64
}

impl Blasts {
    fn config(&self) -> ServerConfig {
        self.config.clone().unwrap_or_default()
    }

    pub fn request(&mut self, dimension: &str, at: [f64; 3], kt: f64, fission: f64, player: u128) {
        self.requests.push(Request { dimension: dimension.to_owned(), at, kt, fission, player });
    }

    /// `/nuke stop`: ends every sweep and forgets what was owed (Java: `BlastManager.stopAll`).
    pub fn stop(&mut self, dimension: &str) -> usize {
        let n = self.jobs.iter().filter(|j| j.record.dimension == dimension).count();
        self.jobs.retain(|j| j.record.dimension != dimension);
        self.requests.retain(|r| r.dimension != dimension);
        self.late.retain(|r| r.dimension != dimension);
        self.late_queue.clear();
        self.late_shapes.clear();
        self.dirty = true;
        n
    }

    pub fn status(&self) -> Vec<String> {
        let late: i32 = self.late.iter().map(|r| r.pending.count() + r.partial.len() as i32).sum();
        let mut out = vec![format!("{} running, {} chunks owed for when they load", self.jobs.len(), late)];
        for j in &self.jobs {
            out.push(format!(
                "#{} {}: front {} m, swept {}/{} m, {} blocks, {} trees felled, {} fires, {} rubble, {} chunks for later, {} ms total, worst tick {} ms",
                j.record.id,
                nuke_yield(j.record.kt),
                fixed(j.shape.model.front_radius(j.age()), 0),
                j.ring.min(j.record.radius),
                j.record.radius,
                j.shape.changed,
                j.shape.trees_felled,
                j.shape.fires_placed,
                j.shape.rubble_blocks,
                j.record.pending.count() + j.record.partial.len() as i32,
                fixed(j.nanos as f64 / 1e6, 1),
                fixed(j.worst as f64 / 1e6, 2),
            ));
        }
        out
    }

    /// A chunk came into a level: a burst that owes it damages it (Java: `BlastManager.onChunkLoad`).
    pub fn chunk_loaded(&mut self, dimension: &str, cx: i32, cz: i32) {
        for j in self.jobs.iter_mut().filter(|j| j.record.dimension == dimension) {
            if !j.sweep_done && j.record.pending.get(j.record.pending.index(cx, cz)) {
                j.arrivals.push((cx, cz));
            }
        }
        for r in self.late.iter().filter(|r| r.dimension == dimension) {
            if r.owes(cx, cz) {
                self.late_queue.push((r.id, (cx, cz)));
            }
        }
    }

    /// The head of a level's tick: start what was asked here, sweep, damage late chunks.
    pub fn read(&mut self, dimension: &str, world: &GameWorld, tables: &Tables) {
        let start = Instant::now();
        self.scan_edges(dimension, world, tables);
        let mut i = 0;
        while i < self.requests.len() {
            if self.requests[i].dimension != dimension {
                i += 1;
                continue;
            }
            let r = self.requests.remove(i);
            let id = self.next_id;
            self.next_id += 1;
            let job = self.start(world, tables, id, &r);
            log::info!(
                "Kiloton: burst #{id}, {} kt at {} {} {} in {dimension} (sea: {}, radius {} m, {} chunks loaded)",
                r.kt,
                job.shape.bx,
                job.shape.by,
                job.shape.bz,
                job.shape.sea,
                job.record.radius,
                world.chunks.len()
            );
            self.tells.push((
                r.player,
                Text::translate(
                    "commands.kiloton.detonated",
                    &[
                        Text::literal(nuke_yield(r.kt)),
                        Text::literal(job.shape.bx.to_string()),
                        Text::literal(job.shape.by.to_string()),
                        Text::literal(job.shape.bz.to_string()),
                    ],
                )
                .with_color(0xFFAA00),
            ));
            let mut told = job.record.clone();
            told.pending = LateGrid::empty();
            told.partial.clear();
            self.announce.push(told);
            self.jobs.push(job);
            self.dirty = true;
        }
        // Once a second, the chunks the finished bursts owe that are loaded now: Horizon tells of a chunk before it
        // is ready, `run_late` lets go of one it can't find yet, and that chunk raises no event later (the sweep
        // above does the same for its arrivals).
        let ticks = match self.late_ticks.get_mut(dimension) {
            Some(t) => {
                *t += 1;
                *t
            }
            None => {
                self.late_ticks.insert(dimension.to_owned(), 1);
                1
            }
        };
        if ticks % 20 == 0 && !self.late_done.is_empty() {
            for (id, n) in std::mem::take(&mut self.late_done) {
                let left = self.late.iter().find(|r| r.id == id).map_or(0, |r| r.pending.count() + r.partial.len() as i32);
                log::info!("Kiloton: burst #{id}: {n} chunk(s) damaged as they loaded, {left} still owed");
            }
        }
        if ticks % 20 == 0 && self.late.iter().any(|r| r.dimension == dimension) {
            let queued: HashSet<(u32, (i32, i32))> = self.late_queue.iter().copied().collect();
            for r in self.late.iter().filter(|r| r.dimension == dimension) {
                for &(cx, cz) in world.chunks.keys() {
                    if r.owes(cx, cz) && !queued.contains(&(r.id, (cx, cz))) {
                        self.late_queue.push((r.id, (cx, cz)));
                    }
                }
            }
        }
        let late_here = self.late_queue.iter().any(|&(id, _)| self.late.iter().any(|r| r.id == id && r.dimension == dimension));
        if !self.jobs.iter().any(|j| j.record.dimension == dimension) && !late_here {
            return;
        }
        let behind = self.jobs.iter().any(|j| j.record.dimension == dimension && j.behind());
        let ms = if behind { (TICK_BUDGET_MS * 3.0).min(45.0) } else { TICK_BUDGET_MS };
        let deadline = start + Duration::from_secs_f64(ms / 1000.0);
        let mut finished = Vec::new();
        {
            let mut w = LevelWorld { world, tables, overlay: &mut self.overlay };
            for (k, job) in self.jobs.iter_mut().enumerate() {
                if job.record.dimension != dimension {
                    continue;
                }
                let t0 = Instant::now();
                job.ticks += 1;
                // Once a second, the chunks owed whole that are loaded now: one that was not ready
                // when the sweep met it raises no event later.
                if !job.sweep_done && job.ticks % 20 == 0 {
                    job.arrivals.clear();
                    for &(cx, cz) in world.chunks.keys() {
                        if job.record.pending.get(job.record.pending.index(cx, cz)) {
                            job.arrivals.push((cx, cz));
                        }
                    }
                }
                if !job.sweep_done && !job.arrivals.is_empty() {
                    rejoin(job, &mut w, deadline);
                    let ring = job.ring as f64;
                    job.shape.flush_falls(&mut w, ring, false);
                }
                if !job.sweep_done {
                    sweep(job, &mut w, world, deadline);
                }
                self.edge_scan.extend(job.shape.edge_fluids.drain(..).map(|(x, z, t, f)| (dimension.to_owned(), x, z, t, f)));
                let dt = t0.elapsed().as_nanos();
                job.nanos += dt;
                job.worst = job.worst.max(dt);
                if job.sweep_done {
                    finished.push(k);
                }
            }
        }
        // Bursts done sweeping: what they still owe goes with the records.
        for k in finished.into_iter().rev() {
            let job = self.jobs.remove(k);
            log::info!(
                "Kiloton: burst #{} swept: {} blocks changed in {} columns, {} trees felled, {} rubble blocks, {} fires, {} chunks left for when they load; {} ms of server time over {} ticks (worst {} ms), {} rings through unloaded land, {} chunks done as they loaded",
                job.record.id,
                job.shape.changed,
                job.shape.columns,
                job.shape.trees_felled,
                job.shape.rubble_blocks,
                job.shape.fires_placed,
                job.record.pending.count() + job.record.partial.len() as i32,
                fixed(job.nanos as f64 / 1e6, 1),
                job.ticks,
                fixed(job.worst as f64 / 1e6, 2),
                job.quiet_rings,
                job.rejoined,
            );
            let mut record = job.record;
            record.horizon = job.shape.horizon.clone();
            record.horizon_dist = job.shape.horizon_dist.clone();
            if record.has_late() {
                // Some of the chunks it owes may be loaded already.
                for &(cx, cz) in world.chunks.keys() {
                    if record.owes(cx, cz) {
                        self.late_queue.push((record.id, (cx, cz)));
                    }
                }
                self.late.push(record);
                self.cap_late();
            }
            self.dirty = true;
        }
        self.run_late(dimension, world, tables, deadline);
    }

    /// Java's `edgeFluids` and `scheduleFluid`: each fluid block of the columns at the crater's edge,
    /// from the top down to the bottom of the crater, gets a tick, so the water walls left standing
    /// there pour into the hole. Read here, a tick after the writes, and scheduled by the
    /// `level-ticks` injection right after.
    fn scan_edges(&mut self, dimension: &str, world: &GameWorld, tables: &Tables) {
        if self.edge_scan.is_empty() {
            return;
        }
        let fast_lava = dimension == "minecraft:the_nether";
        let mut keep = Vec::new();
        for e in std::mem::take(&mut self.edge_scan) {
            if e.0 != dimension {
                keep.push(e);
                continue;
            }
            let (_, x, z, top, floor) = e;
            if !world.chunks.contains_key(&(x >> 4, z >> 4)) {
                continue;
            }
            let mut y = top;
            while y >= floor {
                let kind = tables.fluid_kind.get(world.block(x, y, z).0 as usize).copied().unwrap_or(0);
                if kind != 0 {
                    let delay = if kind <= 2 { 5 } else if fast_lava { 10 } else { 30 };
                    self.fluid_ticks.push((dimension.to_owned(), [x, y, z], kind, delay));
                }
                y -= 1;
            }
        }
        self.edge_scan = keep;
    }

    /// The head-of-tick half for creatures and the aftermath: decisions from the last snapshot (lines of
    /// sight, shelters, the wave under cover), the fallout dose rates, and the dust, soot and dying
    /// foliage whose time has come.
    pub fn creatures_read(&mut self, dimension: &str, world: &GameWorld, tables: &Tables) {
        let now = self.now.get(dimension).copied().unwrap_or(0);
        let cfg = self.config();
        if let Some(snaps) = self.snaps.get(dimension) {
            let view = crate::entities::CoverView { world, tables };
            let doomed: HashSet<u128> = self.bodies.doomed.keys().copied().collect();
            let mut out = Vec::new();
            for job in self.jobs.iter_mut().filter(|j| j.record.dimension == dimension) {
                let burst = crate::entities::Burst {
                    id: job.record.id,
                    model: &job.shape.model,
                    centre: [job.record.x as f64 + 0.5, job.record.y as f64, job.record.z as f64 + 0.5],
                    age: job.age(),
                    cap: job.record.radius as f64,
                };
                burst.decide(&mut job.ents, snaps, &view, &cfg, &doomed, &mut out);
            }
            self.orders.extend(out.into_iter().map(|o| (dimension.to_owned(), o)));
            // Fallout: players every half second, creatures every two (Java: `Radiation`).
            if cfg.fallout && now % 10 == 0 && self.aftermath.sources.iter().any(|s| s.dimension == dimension) {
                for s in snaps.iter().filter(|s| s.living) {
                    let Some(uuid) = s.uuid else { continue };
                    let period = if s.player { 10.0 } else if now % 40 == 0 { 40.0 } else { continue };
                    let mut rate = self.aftermath.rate(dimension, s.pos[0], s.pos[2], now);
                    if rate <= 0.0 && !self.bodies.doses.contains_key(&uuid) {
                        continue;
                    }
                    let eye = [s.eye[0].floor() as i32, s.eye[1].floor() as i32, s.eye[2].floor() as i32];
                    let wet = tables.infos.get(world.block(eye[0], eye[1], eye[2]).0 as usize).is_some_and(|i| i.wet);
                    if wet {
                        rate *= 0.1;
                    } else if eye[1] < kiloton_core::blast::cover::CoverWorld::surface(&view, eye[0], eye[2]) {
                        rate *= 0.2;
                    }
                    self.fallout_doses.push((dimension.to_owned(), uuid, s.id, s.player, s.immune, rate, period));
                }
            }
        }
        if cfg.fallout {
            let (late, jobs) = (&self.late, &self.jobs);
            let owes = |cx: i32, cz: i32| {
                late.iter().chain(jobs.iter().map(|j| &j.record)).any(|r| r.dimension == dimension && r.owes(cx, cz))
            };
            if now % crate::aftermath::SCAN_PERIOD == 0 {
                self.aftermath.scan(dimension, world, now, &owes);
            }
            self.aftermath.run(dimension, world, tables, TICK_BUDGET_MS * 0.5);
        }
    }

    /// The Host half for creatures: what was decided, the death scenes, the sickness, the aftermath's
    /// blocks, and a new snapshot when a burst or the fallout needs one.
    fn creatures_apply(&mut self, host: &mut dyn Host, dimension: &str, now: i64) {
        use crate::entities::Order;
        use horizon_client::mod_api::NativeEffect;
        let cfg = self.config();
        let mut effects = Vec::new();
        let mut doom_msgs = Vec::new();
        for (d, o) in std::mem::take(&mut self.orders) {
            if d != dimension {
                self.orders.push((d, o));
                continue;
            }
            match o {
                Order::Hurt { id, amount, kind } => {
                    host.hurt_entity(dimension, id, amount, kind, None);
                }
                Order::Fire { id, ticks } => {
                    if let Some(e) = host.entity(id) {
                        host.entity_set_fire(e, ticks);
                    }
                }
                Order::Push { id, v } => {
                    if let Some(e) = host.entity(id) {
                        host.entity_push(e, v);
                    }
                }
                Order::Remove { id } => {
                    host.remove_entity(dimension, id);
                }
                Order::Dose { uuid, id, player, rem } => {
                    effects.extend(self.bodies.add(dimension, uuid, id, player, false, rem, now));
                }
                Order::Doom { uuid, id, pos, yaw, pitch, fireball, shadow } => {
                    if shadow {
                        let (angle, length) = crate::entities::shadow_of(pos, fireball);
                        let sid = self.shadows.iter().map(|s| s.1).max().unwrap_or(0) + 1;
                        self.shadows.push((dimension.to_owned(), sid, pos, angle, length));
                    }
                    self.bodies.doomed.insert(uuid, (dimension.to_owned(), id, pos, yaw, pitch, now + crate::entities::DOOM_TICKS));
                    let mut b = Vec::with_capacity(33);
                    b.extend_from_slice(&(id as i32).to_le_bytes());
                    for v in fireball {
                        b.extend_from_slice(&v.to_le_bytes());
                    }
                    b.extend_from_slice(&(crate::entities::DOOM_TICKS as i32).to_le_bytes());
                    b.push(shadow as u8);
                    // To the doomed player only: their client runs the scene.
                    doom_msgs.push((uuid, b));
                }
            }
        }
        for (d, uuid, id, player, immune, rate, period) in std::mem::take(&mut self.fallout_doses) {
            if d != dimension {
                self.fallout_doses.push((d, uuid, id, player, immune, rate, period));
                continue;
            }
            let entry = self.bodies.doses.entry(uuid).or_insert_with(|| crate::entities::Dose { amp: -1, ..Default::default() });
            entry.dimension = dimension.to_owned();
            entry.id = id;
            entry.player = player;
            effects.extend(self.bodies.fallout(uuid, rate, period, immune, now));
        }
        for (uuid, effect, ticks, amp) in effects {
            if self.bodies.doses.get(&uuid).is_some_and(|d| d.player) {
                host.add_effect(uuid, &NativeEffect { id: effect.to_owned(), amplifier: amp as u8, duration: ticks, ambient: false, particles: true });
            }
        }
        for (d, id, dmg) in self.bodies.tick_sickness(now) {
            if d == dimension {
                host.hurt_entity(dimension, id, dmg, "kiloton:radiation", None);
            }
        }
        // The death scenes: the client half shows it; here the player is held still, then dies of it.
        let players = host.server_players();
        for (uuid, m) in &doom_msgs {
            let _ = host.net_send_to_player(*uuid, crate::entities::DOOM_CHANNEL, m);
        }
        // The shadows: to each player of the level once (again when they come back to it).
        if !self.shadows.is_empty() && (!doom_msgs.is_empty() || now % 20 == 0) {
            let player_dim: HashMap<u128, &str> = players.iter().map(|p| (p.uuid, p.dimension.as_str())).collect();
            let shadow_dim: HashMap<u32, &str> = self.shadows.iter().map(|s| (s.1, s.0.as_str())).collect();
            self.told_shadows.retain(|(id, p)| matches!((shadow_dim.get(id), player_dim.get(p)), (Some(a), Some(b)) if a == b));
            for s in self.shadows.iter().filter(|s| s.0 == dimension) {
                for p in players.iter().filter(|p| p.dimension == dimension) {
                    if self.told_shadows.insert((s.1, p.uuid)) {
                        let m = crate::entities::shadow_message(s.1, s.2, s.3, s.4);
                        let _ = host.net_send_to_player(p.uuid, crate::entities::SHADOW_CHANNEL, &m);
                    }
                }
            }
        }
        let mut ended = Vec::new();
        for (&uuid, (d, id, pos, yaw, pitch, end)) in &self.bodies.doomed {
            if d != dimension {
                continue;
            }
            let alive = players.iter().any(|p| p.uuid == uuid && p.health > 0.0);
            if now >= *end || !alive {
                ended.push((uuid, *id, alive));
                continue;
            }
            if let Some(e) = host.entity(*id) {
                host.entity_teleport_to(e, *pos, Some([*yaw, *pitch]));
                host.entity_set_velocity(e, [0.0, 0.0, 0.0]);
                host.entity_set_fire(e, 0);
            }
        }
        for &(uuid, _, _) in &ended {
            self.bodies.doomed.remove(&uuid);
        }
        *crate::entities::DOOMED_IDS.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(self.bodies.doomed.values().map(|d| d.1 as i32).collect());
        for (_, id, alive) in ended {
            if alive {
                host.hurt_entity(dimension, id, f32::MAX, "kiloton:nuclear_shadow", None);
            }
        }
        // The aftermath's blocks: one bulk edit, no neighbour told (Java: UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE).
        let writes: Vec<_> = {
            let (here, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.aftermath.writes).into_iter().partition(|w| w.0 == dimension);
            self.aftermath.writes = rest;
            here
        };
        if !writes.is_empty() {
            if now % 100 == 0 || self.aftermath.chunks_done < 50 {
                log::info!(
                    "Kiloton: aftermath: {} blocks this tick ({} chunks and {} blocks so far)",
                    writes.len(),
                    self.aftermath.chunks_done,
                    self.aftermath.blocks_placed
                );
            }
            let mut edit = NativeBlockEdit::default();
            let mut pal: HashMap<&str, u32> = HashMap::new();
            for (_, [x, y, z], name) in writes {
                let next = pal.len() as u32;
                let block = *pal.entry(name).or_insert_with(|| {
                    edit.palette.push(name.to_owned());
                    next
                });
                edit.runs.push(NativeBlockRun { x, z, y, height: 1, surface: None, block, only: None });
            }
            if let Err(e) = host.edit_blocks(dimension, &edit) {
                log::warn!("Kiloton: aftermath edit refused: {e}");
            }
        }
        // A snapshot for the next tick: while a burst still works on creatures, and twice a second for
        // the fallout.
        let busy = self.jobs.iter().any(|j| j.record.dimension == dimension && !j.ents.done);
        let fallout = cfg.fallout && (now + 1) % 10 == 0 && self.aftermath.sources.iter().any(|s| s.dimension == dimension);
        if !(busy || fallout) {
            self.snaps.remove(dimension);
            return;
        }
        let mut snaps = Vec::new();
        let player_ids: HashSet<u32> = players.iter().filter(|p| p.entity_id > 0).map(|p| p.entity_id as u32).collect();
        for p in players.iter().filter(|p| p.dimension == dimension && p.health > 0.0) {
            let id = p.entity_id as u32;
            let eye = host.entity(id).and_then(|e| host.entity_eye_position(e)).unwrap_or([p.position[0], p.position[1] + 1.62, p.position[2]]);
            snaps.push(crate::entities::Snap {
                id,
                kind: "minecraft:player".into(),
                pos: p.position,
                eye,
                uuid: Some(p.uuid),
                player: true,
                immune: p.game_mode == 1 || p.game_mode == 3,
                living: true,
                yaw: p.yaw,
                pitch: p.pitch,
            });
        }
        // Creatures and things near a working burst (or anywhere in the fallout).
        let near: Vec<([f64; 3], f64)> = self
            .jobs
            .iter()
            .filter(|j| j.record.dimension == dimension && !j.ents.done)
            .map(|j| {
                let m = &j.shape.model;
                let reach = m.radius_for_psi(0.8).max(m.radius_for_fluence(3.0)).max(m.radius_for_dose(50.0)).min(j.record.radius as f64 * 2.0);
                ([j.record.x as f64 + 0.5, j.record.y as f64, j.record.z as f64 + 0.5], reach)
            })
            .collect();
        for (id, kind, pos) in host.level_entities(dimension) {
            if player_ids.contains(&id) {
                continue;
            }
            let close = near.iter().any(|(c, r)| (pos[0] - c[0]).powi(2) + (pos[1] - c[1]).powi(2) + (pos[2] - c[2]).powi(2) <= r * r);
            if !close && !fallout {
                continue;
            }
            let Some(e) = host.entity(id) else { continue };
            let living = host.living(e).is_some();
            if !close && !living {
                continue;
            }
            let eye = host.entity_eye_position(e).unwrap_or(pos);
            let uuid = if living { host.entity_uuid(e) } else { None };
            snaps.push(crate::entities::Snap {
                id,
                kind,
                pos,
                eye,
                uuid,
                player: false,
                immune: false,
                living,
                yaw: 0.0,
                pitch: 0.0,
            });
        }
        self.snaps.insert(dimension.to_owned(), snaps);
    }

    /// Keeps new burst ids clear of one read back from the save.
    pub fn bump_id(&mut self, id: u32) {
        self.next_id = self.next_id.max(id + 1);
    }

    fn cap_late(&mut self) {
        let mut owing = self.late.iter().filter(|r| r.has_late()).count();
        let mut i = 0;
        while owing > LATE_BLAST_LIMIT && i < self.late.len() {
            if self.late[i].has_late() {
                self.late[i].pending = LateGrid::empty();
                self.late[i].partial.clear();
                owing -= 1;
            }
            i += 1;
        }
        self.late.retain(|r| r.has_late());
    }

    fn start(&self, world: &GameWorld, tables: &Tables, id: u32, r: &Request) -> Job {
        let config = self.config();
        let (min_y, max_y) = (world.bottom_y(), world.top_y());
        let x = r.at[0].floor() as i32;
        let z = r.at[2].floor() as i32;
        let info = |y: i32| tables.infos.get(world.block(x, y, z).0 as usize).copied().unwrap_or(StateInfo::AIR);
        // Everything is a surface burst: a point in the air comes down to what is under it, the
        // ground, a roof or the surface of the water (Java: `NukeCommand.onGround`). Leaves, plants
        // and light blocks such as snow don't stop it.
        let mut y = (r.at[1].floor() as i32).min(max_y);
        // The Java edition loads ground zero's chunk to find the ground; a mod here can't, so a point
        // in land not loaded goes off where it was asked (read as air it would sink to the bottom).
        let loaded = world.chunks.contains_key(&(x >> 4, z >> 4));
        if !loaded {
            log::info!("Kiloton: ground zero {x} {z} is not loaded: the burst goes off at y {y} as asked");
        }
        while loaded && y > min_y + 1 {
            let below = info(y - 1);
            if below.wet {
                break;
            }
            let c = below.profile.category;
            if c != profile::AIR && c != profile::LEAVES && c != profile::PLANT && c != profile::LIGHT {
                break;
            }
            y -= 1;
        }
        let sea = info(y).wet || info(y - 1).wet;
        let ground = shape::ground_kind(info(y - 1).profile.surface);
        let model = BlastModel::with_fission(r.kt, r.fission);
        let thermo = model.thermonuclear();
        let radius = model.natural_effect_radius.min(config.max_radius(thermo) as f64) as i32;
        let seed = seed(id, x, y, z);
        let mut s = Shape::new(model, x, y, z, seed, radius, min_y, max_y, None, ground);
        s.sea = sea;
        let record = Record {
            id,
            dimension: r.dimension.clone(),
            x,
            y,
            z,
            kt: r.kt,
            fission: r.fission,
            seed,
            radius,
            sea,
            ground_kind: ground,
            horizon: Vec::new(),
            horizon_dist: Vec::new(),
            pending: LateGrid::for_radius(x, z, radius),
            partial: HashMap::new(),
        };
        Job {
            touched: LateGrid::for_radius(x, z, radius),
            record,
            shape: s,
            ticks: 0,
            ring: 0,
            cols: RingColumns::default(),
            cursor: 0,
            built_ring: -1,
            band_start: -1,
            live: Vec::new(),
            arrivals: Vec::new(),
            sweep_done: false,
            nanos: 0,
            worst: 0,
            quiet_rings: 0,
            rejoined: 0,
            ents: Default::default(),
        }
    }

    fn run_late(&mut self, dimension: &str, world: &GameWorld, tables: &Tables, deadline: Instant) {
        let mut guard = self.late_queue.len();
        while guard > 0 && !self.late_queue.is_empty() && Instant::now() < deadline {
            guard -= 1;
            let (id, (cx, cz)) = self.late_queue.remove(0);
            let Some(ri) = self.late.iter().position(|r| r.id == id) else { continue };
            if self.late[ri].dimension != dimension {
                self.late_queue.push((id, (cx, cz)));
                continue;
            }
            let gi = self.late[ri].pending.index(cx, cz);
            let whole = self.late[ri].pending.get(gi);
            let mask = if whole { None } else { self.late[ri].partial.get(&(cx, cz)).copied() };
            if !whole && mask.is_none() {
                continue; // already done (queued twice)
            }
            if !world.chunks.contains_key(&(cx, cz)) {
                continue; // unloaded again: wait for the next load
            }
            let r = &self.late[ri];
            let r2 = r.radius as i64 * r.radius as i64;
            let shape = self.late_shapes.entry(id).or_insert_with(|| late_shape(r, world));
            let mut w = LevelWorld { world, tables, overlay: &mut self.overlay };
            // Nearest columns first, as the wave would have done: trees fall onto ground already hit.
            let mut cols: Vec<(i64, i32)> = Vec::with_capacity(256);
            for bit in 0..256i32 {
                if let Some(m) = mask {
                    if m[(bit >> 6) as usize] & (1u64 << (bit & 63)) == 0 {
                        continue;
                    }
                }
                let dx = ((cx << 4) | (bit & 15)) - shape.bx;
                let dz = ((cz << 4) | (bit >> 4)) - shape.bz;
                let d2 = dx as i64 * dx as i64 + dz as i64 * dz as i64;
                if d2 <= r2 {
                    cols.push((d2, bit));
                }
            }
            cols.sort_unstable();
            for &(_, bit) in &cols {
                let (dx, dz) = (((cx << 4) | (bit & 15)) - shape.bx, ((cz << 4) | (bit >> 4)) - shape.bz);
                shape.column(&mut w, dx, dz, false);
            }
            shape.flush_falls(&mut w, f64::MAX, true);
            self.edge_scan.extend(shape.edge_fluids.drain(..).map(|(x, z, t, f)| (dimension.to_owned(), x, z, t, f)));
            let r = &mut self.late[ri];
            if whole {
                r.pending.clear(gi);
            } else {
                r.partial.remove(&(cx, cz));
            }
            *self.late_done.entry(id).or_default() += 1;
            self.dirty = true;
        }
        self.late.retain(|r| r.has_late());
    }

    /// The Host's half of the tick: what the sweep wrote goes into the world.
    pub fn apply(&mut self, host: &mut dyn Host, dimension: &str, raw_names: &dyn Fn(u32) -> Option<String>) {
        for (uuid, line) in self.tells.drain(..) {
            host.tell(uuid, line);
        }
        let now = host.level(dimension).and_then(|l| host.level_game_time(l)).unwrap_or(0);
        self.now.insert(dimension.to_owned(), now);
        self.creatures_apply(host, dimension, now);
        if !self.announce.is_empty() {
            for r in std::mem::take(&mut self.announce) {
                if r.dimension != dimension {
                    self.announce.push(r);
                    continue;
                }
                // Humidity at ground zero: saturated at sea; elsewhere the Java edition reads the biome's
                // downfall, which the Host doesn't give: an average 0.6.
                let humidity = if r.sea { 1.0 } else { 0.6 };
                let msg = crate::client::encode(
                    r.id,
                    [r.x as f64 + 0.5, r.y as f64, r.z as f64 + 0.5],
                    r.kt,
                    now,
                    r.seed,
                    humidity,
                    r.sea,
                    false,
                    r.fission,
                );
                let life = (kiloton_core::fx::mushroom::life_seconds(r.kt) * 20.0) as i64;
                self.clouds.push((dimension.to_owned(), r.id, msg, now, life));
                self.aftermath.sources.push(crate::aftermath::Source::new(
                    dimension,
                    r.id,
                    r.x as f64 + 0.5,
                    r.z as f64 + 0.5,
                    r.kt,
                    r.fission,
                    r.seed,
                    humidity as f64,
                    r.sea,
                    now,
                ));
                self.aftermath.dirty = true;
            }
        }
        // New bursts at once; the rest once a second, for players who have just come into the level.
        let fresh = self.clouds.iter().any(|c| c.0 == dimension && c.3 == now);
        if !self.clouds.is_empty() && (fresh || now % 20 == 0) {
            let players = host.server_players();
            let here: Vec<u128> = players.iter().filter(|p| p.dimension == dimension).map(|p| p.uuid).collect();
            self.clouds.retain(|c| now - c.3 < c.4 + 100);
            // A player who left a burst's level is told again when they come back (their client forgot
            // it). Checked from whichever level ticks: one with no player in it may not.
            let player_dim: HashMap<u128, &str> = players.iter().map(|p| (p.uuid, p.dimension.as_str())).collect();
            let cloud_dim: HashMap<u32, &str> = self.clouds.iter().map(|c| (c.1, c.0.as_str())).collect();
            self.told.retain(|(id, p)| matches!((cloud_dim.get(id), player_dim.get(p)), (Some(c), Some(d)) if c == d));
            for c in self.clouds.iter().filter(|c| c.0 == dimension) {
                for &p in &here {
                    if self.told.insert((c.1, p)) {
                        if let Err(e) = host.net_send_to_player(p, crate::client::CHANNEL, &c.2) {
                            log::warn!("Kiloton: detonation message not sent: {e}");
                            self.told.remove(&(c.1, p));
                        }
                    }
                }
            }
        }
        if self.overlay.blocks.is_empty() {
            return;
        }
        let blocks = std::mem::take(&mut self.overlay.blocks);
        self.overlay.tops.clear();
        let mut columns: HashMap<(i32, i32), Vec<(i32, String)>> = HashMap::new();
        let mut singles: Vec<([i32; 3], u32, i32)> = Vec::new();
        for ((x, y, z), (id, w)) in blocks {
            let name = match w {
                Write::Air => Some("minecraft:air".to_owned()),
                Write::Named(n) => Some(n.to_owned()),
                Write::Log { state, along_x } => {
                    raw_names(state).map(|n| format!("{n}[axis={}]", if along_x { "x" } else { "z" }))
                }
                Write::State(_) => None,
                Write::Fire { .. } => {
                    // Set one by one, with the neighbours told, so that it ticks and spreads.
                    singles.push(([x, y, z], id, 3));
                    continue;
                }
            };
            match name {
                Some(n) => columns.entry((x, z)).or_default().push((y, n)),
                // A state as it is: no update, no shape change (Java writes the section directly).
                None => singles.push(([x, y, z], id, 2 | 16)),
            }
        }
        let mut edit = NativeBlockEdit::default();
        let mut pal: HashMap<String, u32> = HashMap::new();
        for ((x, z), mut ys) in columns {
            ys.sort_unstable_by_key(|(y, _)| *y);
            let mut i = 0;
            while i < ys.len() {
                let y0 = ys[i].0;
                let mut j = i + 1;
                while j < ys.len() && ys[j].0 == y0 + (j - i) as i32 && ys[j].1 == ys[i].1 {
                    j += 1;
                }
                let next = pal.len() as u32;
                let n = ys[i].1.clone();
                let block = *pal.entry(n.clone()).or_insert_with(|| {
                    edit.palette.push(n);
                    next
                });
                edit.runs.push(NativeBlockRun { x, z, y: y0, height: (j - i) as u32, surface: None, block, only: None });
                i = j;
            }
        }
        if !edit.runs.is_empty() {
            if let Err(e) = host.edit_blocks(dimension, &edit) {
                log::warn!("Kiloton: bulk edit refused: {e}");
            }
        }
        for (pos, id, flags) in singles {
            host.set_block(dimension, pos, horizon_client::mod_api::BlockStateId(id), flags);
        }
    }

    /// What the save holds: the records owing chunks, and for each running sweep a record owing
    /// everything it has not reached yet (Java: `BlastJob.snapshotOwed`, `BlastManager.forSave`;
    /// before the fix of 10-11 a world closed during a megaton's sweep lost the rest of the damage).
    pub fn records_for_save(&self, dimension: &str) -> Vec<Record> {
        let mut out: Vec<Record> = self.late.iter().filter(|r| r.dimension == dimension).cloned().collect();
        for j in self.jobs.iter().filter(|j| j.record.dimension == dimension) {
            let mut r = j.record.clone();
            r.horizon = j.shape.horizon.clone();
            r.horizon_dist = j.shape.horizon_dist.clone();
            if !j.sweep_done {
                owe_rest(j, &mut r);
            }
            if r.has_late() {
                out.push(r);
            }
        }
        out
    }

    /// Forgets a level's bursts (the level closed: its records are in the save).
    pub fn forget(&mut self, dimension: &str) {
        self.jobs.retain(|j| j.record.dimension != dimension);
        self.requests.retain(|r| r.dimension != dimension);
        let ids: Vec<u32> = self.late.iter().filter(|r| r.dimension == dimension).map(|r| r.id).collect();
        self.late.retain(|r| r.dimension != dimension);
        self.late_queue.retain(|(id, _)| !ids.contains(id));
        self.late_shapes.retain(|id, _| !ids.contains(id));
    }

    /// The records of a level read back from its save: their chunks are damaged as they load.
    pub fn restore(&mut self, records: Vec<Record>) {
        for r in records {
            if self.late.iter().any(|l| l.id == r.id && l.dimension == r.dimension) {
                continue;
            }
            self.next_id = self.next_id.max(r.id + 1);
            self.late.push(r);
        }
    }

    /// Queues the loaded chunks of a level that its records owe (after a load, as no event comes).
    pub fn queue_loaded(&mut self, dimension: &str, world: &GameWorld) {
        for r in self.late.iter().filter(|r| r.dimension == dimension) {
            for &(cx, cz) in world.chunks.keys() {
                if r.owes(cx, cz) && !self.late_queue.contains(&(r.id, (cx, cz))) {
                    self.late_queue.push((r.id, (cx, cz)));
                }
            }
        }
    }
}

fn late_shape(r: &Record, world: &GameWorld) -> Shape {
    let model = BlastModel::with_fission(r.kt, r.fission);
    let horizon = if r.horizon.is_empty() { None } else { Some((r.horizon.clone(), r.horizon_dist.clone())) };
    let mut s = Shape::new(model, r.x, r.y, r.z, r.seed, r.radius, world.bottom_y(), world.top_y(), horizon, r.ground_kind);
    s.fires = false;
    s.burn_late = true;
    s.sea = r.sea;
    s
}

/// Java: `BlastJob.rejoin`: the chunks that loaded behind the sweep, whole, nearest columns first.
fn rejoin(job: &mut Job, w: &mut LevelWorld<'_>, deadline: Instant) {
    let mut i = 0;
    while i < job.arrivals.len() {
        if Instant::now() > deadline {
            return;
        }
        let (cx, cz) = job.arrivals[i];
        let gi = job.record.pending.index(cx, cz);
        if !job.record.pending.get(gi) || !w.world.chunks.contains_key(&(cx, cz)) {
            job.arrivals.swap_remove(i);
            continue;
        }
        let x0 = (cx << 4) - job.shape.bx;
        let z0 = (cz << 4) - job.shape.bz;
        let fx = x0.abs().max((x0 + 15).abs()) as f64;
        let fz = z0.abs().max((z0 + 15).abs()) as f64;
        if (fx * fx + fz * fz).sqrt() >= job.ring as f64 {
            i += 1;
            continue;
        }
        let r2 = job.record.radius as i64 * job.record.radius as i64;
        let mut order: Vec<(i64, i32)> = (0..256)
            .filter_map(|bit: i32| {
                let (dx, dz) = (x0 + (bit & 15), z0 + (bit >> 4));
                let d2 = dx as i64 * dx as i64 + dz as i64 * dz as i64;
                (d2 <= r2).then_some((d2, bit))
            })
            .collect();
        order.sort_unstable();
        for &(_, bit) in &order {
            job.shape.column(w, x0 + (bit & 15), z0 + (bit >> 4), true);
        }
        job.record.pending.clear(gi);
        job.touched.set(gi);
        job.rejoined += 1;
        job.arrivals.swap_remove(i);
    }
}

/// Java: `BlastJob.sweep`: ring by ring, never ahead of the front, until the deadline.
fn sweep(job: &mut Job, w: &mut LevelWorld<'_>, world: &GameWorld, deadline: Instant) {
    let front = job.shape.model.front_radius(job.age() + 0.05);
    let mut checks = 0u32;
    while job.ring <= job.record.radius && job.ring as f64 <= front {
        if job.built_ring != job.ring {
            if job.ring >= FAR_FROM {
                // Far out a megaton's rings are hundreds of thousands of columns, nearly all in chunks
                // nobody has loaded: band by band those are left for later whole, and the rings are
                // listed from the loaded ones only. A band with none is crossed in one step.
                let start = job.ring & !15;
                if job.band_start != start {
                    scan_band(job, world, start);
                    job.band_start = start;
                }
                if job.live.is_empty() {
                    let end = (start + 15).min(job.record.radius);
                    if end as f64 > front {
                        return; // the front has not crossed the band yet
                    }
                    while job.ring <= end {
                        job.ring += 1;
                        job.quiet_rings += 1;
                    }
                    let ring = job.ring as f64;
                    job.shape.flush_falls(w, ring, false);
                    checks += 1;
                    if checks & 3 == 0 && Instant::now() > deadline {
                        return;
                    }
                    continue;
                }
                job.cols.clear();
                for k in 0..job.live.len() {
                    let (cx, cz) = job.live[k];
                    if RingColumns::crosses(job.shape.bx, job.shape.bz, cx, cz, job.ring) {
                        job.cols.add_chunk(job.shape.bx, job.shape.bz, cx, cz, job.ring);
                    }
                }
            } else {
                job.cols.clear();
                job.cols.add_ring(job.ring);
            }
            job.cursor = 0;
            job.built_ring = job.ring;
        }
        while job.cursor < job.cols.len() {
            let packed = job.cols.cols[job.cursor];
            job.cursor += 1;
            let (dx, dz) = (kiloton_core::blast::ring_columns::dx(packed), kiloton_core::blast::ring_columns::dz(packed));
            let (wx, wz) = (job.shape.bx + dx, job.shape.bz + dz);
            let gi = job.record.pending.index(wx >> 4, wz >> 4);
            // Most of a big blast lies in chunks nobody has loaded: one bit test skips them.
            if job.record.pending.get(gi) {
                continue;
            }
            if job.shape.column(w, dx, dz, true) {
                job.touched.set(gi);
            } else if job.touched.get(gi) {
                let mask = job.record.partial.entry((wx >> 4, wz >> 4)).or_insert([0; 4]);
                let bit = ((wz & 15) << 4) | (wx & 15);
                mask[(bit >> 6) as usize] |= 1u64 << (bit & 63);
            } else {
                job.record.pending.set(gi);
            }
            checks += 1;
            if checks & 15 == 0 && Instant::now() > deadline {
                return;
            }
        }
        job.ring += 1;
        let ring = job.ring as f64;
        job.shape.flush_falls(w, ring, false);
        if job.ring > job.record.radius {
            break;
        }
    }
    if job.ring > job.record.radius {
        job.shape.flush_falls(w, f64::MAX, false);
        job.sweep_done = true;
    }
}

/// Java: `BlastJob.scanBand`: the chunks rings start ..= start + 15 cross; those not loaded and
/// not begun are left for later now, the others go in `live`.
fn scan_band(job: &mut Job, world: &GameWorld, start: i32) {
    job.live.clear();
    let (bx, bz) = (job.shape.bx, job.shape.bz);
    let inner = start as f64;
    let out = ((start + 16) as f64).min((job.record.radius + 1) as f64);
    let (cx0, cz0) = (bx >> 4, bz >> 4);
    let n = (out / 16.0).ceil() as i32 + 1;
    for i in -n..=n {
        let cx = cx0 + i;
        let x0 = (cx << 4) - bx;
        let x1 = x0 + 15;
        let nx = if x0 > 0 { x0 as f64 } else if x1 < 0 { -x1 as f64 } else { 0.0 };
        let fx = x0.abs().max(x1.abs()) as f64;
        if nx >= out {
            continue;
        }
        let span = ((out * out - nx * nx).sqrt() / 16.0).ceil() as i32 + 1;
        let hole = if fx < inner { ((inner * inner - fx * fx).sqrt() / 16.0).floor() as i32 - 2 } else { 0 };
        let mut j = -span;
        while j <= span {
            if hole > 0 && j > -hole && j < hole {
                j = hole;
                continue;
            }
            let cz = cz0 + j;
            j += 1;
            let z0 = (cz << 4) - bz;
            let z1 = z0 + 15;
            let nz = if z0 > 0 { z0 as f64 } else if z1 < 0 { -z1 as f64 } else { 0.0 };
            let fz = z0.abs().max(z1.abs()) as f64;
            if nx * nx + nz * nz >= out * out || fx * fx + fz * fz < inner * inner {
                continue;
            }
            let gi = job.record.pending.index(cx, cz);
            if gi < 0 || job.record.pending.get(gi) {
                continue;
            }
            if job.touched.get(gi) || world.chunks.contains_key(&(cx, cz)) {
                job.live.push((cx, cz));
            } else {
                job.record.pending.set(gi);
            }
        }
    }
}

/// Java: `snapshotOwed`: every chunk the sweep has not reached is owed whole; one the current ring
/// runs through owes its columns from that ring out.
fn owe_rest(j: &Job, r: &mut Record) {
    let (bx, bz) = (j.shape.bx, j.shape.bz);
    let inner = j.ring as f64;
    let out = (j.record.radius + 1) as f64;
    let whole = inner + 48.0;
    let r2 = j.record.radius as i64 * j.record.radius as i64;
    let ring2 = j.ring as i64 * j.ring as i64;
    let (cx0, cz0) = (bx >> 4, bz >> 4);
    let n = (out / 16.0).ceil() as i32 + 1;
    for i in -n..=n {
        let cx = cx0 + i;
        let x0 = (cx << 4) - bx;
        let x1 = x0 + 15;
        let nx = if x0 > 0 { x0 as f64 } else if x1 < 0 { -x1 as f64 } else { 0.0 };
        let fx = x0.abs().max(x1.abs()) as f64;
        if nx >= out {
            continue;
        }
        let j_out = ((out * out - nx * nx).sqrt() / 16.0).ceil() as i32 + 1;
        let j_in = if nx >= whole { 0 } else { ((whole * whole - nx * nx).sqrt() / 16.0).ceil() as i32 + 2 };
        if j_in == 0 {
            r.pending.set_row(cx, cz0 - j_out, cz0 + j_out);
            continue;
        }
        if j_in <= j_out {
            r.pending.set_row(cx, cz0 + j_in, cz0 + j_out);
            r.pending.set_row(cx, cz0 - j_out, cz0 - j_in);
        }
        let edge = (j_in - 1).min(j_out);
        let hole = if fx < inner { ((inner * inner - fx * fx).sqrt() / 16.0).floor() as i32 - 2 } else { 0 };
        let mut jj = -edge;
        while jj <= edge {
            if hole > 0 && jj > -hole && jj < hole {
                jj = hole;
                continue;
            }
            let cz = cz0 + jj;
            jj += 1;
            let z0 = (cz << 4) - bz;
            let z1 = z0 + 15;
            let nz = if z0 > 0 { z0 as f64 } else if z1 < 0 { -z1 as f64 } else { 0.0 };
            let fz = z0.abs().max(z1.abs()) as f64;
            let (near2, far2) = (nx * nx + nz * nz, fx * fx + fz * fz);
            if near2 >= out * out || far2 < inner * inner {
                continue;
            }
            let gi = r.pending.index(cx, cz);
            if gi < 0 || r.pending.get(gi) {
                continue;
            }
            if near2 >= inner * inner {
                r.pending.set(gi);
                continue;
            }
            let mask = r.partial.entry((cx, cz)).or_insert([0; 4]);
            for bit in 0..256i32 {
                let (dx, dz) = (x0 + (bit & 15), z0 + (bit >> 4));
                let d2 = dx as i64 * dx as i64 + dz as i64 * dz as i64;
                if d2 >= ring2 && d2 <= r2 {
                    mask[(bit >> 6) as usize] |= 1u64 << (bit & 63);
                }
            }
        }
    }
}
