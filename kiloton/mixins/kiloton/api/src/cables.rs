//! The energy cables (Java: `CableBlock`, `CableBlockEntity`, `CableNetwork`). Each piece holds as much as its grade
//! moves in a tick; the pieces joined to each other make a network, which once a tick pools them, pulls out of the
//! blocks its extract sides touch, shares the pool evenly among the blocks its other sides touch (up to the grade of
//! each, starting one further along every tick) and puts what is left back into the pieces. Each side remembers how it
//! was set: normal, extract, disconnected (shift-right-click with an empty hand cycles the clicked side).
//!
//! A block entity is out of the world while it ticks, so the pieces only say where they are and how they are set
//! from their tick; the network runs in `Level::energy`, after every block entity ticked. As in Java a
//! network is kept from tick to tick and built again only when a piece comes, goes or changes, or a side starts
//! touching something else. It keeps what each piece holds and tells Horizon only what changes: the pieces next to a
//! block are read every tick (that block may have filled them), and a piece's buffer gives energy too, so the network
//! can take back what it put there (in Java nothing outside the network can take it; in Horizon only Kiloton moves
//! energy). A network's pieces must be in loaded chunks, as in Java.

use crate::bomb::get_int;
use crate::machines::{opposite, step};
use horizon_client::mod_api::{Compound, Host, ModBlockEntity, ModEnergyBuffer, Tag, Text};
use kiloton_core::machine::CableTier;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Mutex;

/// A fast hash for the tables a tick goes through once per piece (positions, ids; the multiply-rotate hash of
/// rustc's `FxHasher`). They hold nothing a player could choose to collide.
#[derive(Default, Clone, Copy)]
struct Fx(u64);

impl Fx {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.add(u64::from_le_bytes(c.try_into().expect("8 bytes")));
        }
        for &b in chunks.remainder() {
            self.add(b as u64);
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    fn write_i32(&mut self, i: i32) {
        self.add(i as u32 as u64);
    }
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

type Map<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;
type Set<K> = HashSet<K, BuildHasherDefault<Fx>>;

/// The block entity type of the four grades (Java: `ModBlockEntities.CABLE`).
pub const ENTITY: &str = "kiloton:cable";
pub const NORMAL: u8 = 0;
pub const EXTRACT: u8 = 1;
pub const DISABLED: u8 = 2;
/// Pieces walked in one network at most (Java: `MAX_PIECES`).
const MAX_PIECES: usize = 32_768;
const SIDES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];
/// The values of a side property.
const SHOWN: [&str; 4] = ["none", "cable", "block", "extract"];
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// `Block.UPDATE_CLIENTS`.
const UPDATE_CLIENTS: i32 = 2;

pub fn tier(block: &str) -> Option<CableTier> {
    CableTier::of_block(block)
}

fn block_name(state: &str) -> &str {
    state.split_once('[').map_or(state, |(b, _)| b)
}

/// Blocks with an energy buffer a cable joins to (Java asks the capability; in Horizon only Kiloton's have one).
fn has_energy(block: &str) -> bool {
    crate::machines::is_machine(block) || block == crate::machines::CREATIVE_SOURCE
}

fn modes_of(nbt: &Compound) -> [u8; 6] {
    let packed = get_int(nbt, "sides").unwrap_or(0);
    std::array::from_fn(|i| {
        let m = ((packed >> (2 * i)) & 3) as u8;
        if m > DISABLED { NORMAL } else { m }
    })
}

fn pack(modes: [u8; 6]) -> i32 {
    modes.iter().enumerate().fold(0, |acc, (i, m)| acc | ((*m as i32 & 3) << (2 * i)))
}

// ------------------------------------------------------------------------------------------------ shared state

#[derive(Clone, Copy, PartialEq, Eq)]
struct Piece {
    tier: CableTier,
    modes: [u8; 6],
}

/// A block a network gives to or takes from: where, the face it is reached by, the grade of the cable that reaches it.
type End = ([i32; 3], u8, i32);

/// A network, kept until one of its pieces comes, goes or changes (Java: `CableNetwork`, rebuilt when `dead`).
#[derive(Default)]
struct Net {
    /// Its pieces, in the order they were walked (the order the pool goes back in).
    pieces: Vec<[i32; 3]>,
    rates: Vec<i32>,
    /// What each piece holds: the network is the only thing that moves a piece's energy, so it keeps the numbers and
    /// asks Horizon only about the pieces next to a block (`edge`), which that block may fill.
    held: Vec<i32>,
    edge: Vec<usize>,
    room: i64,
    inserts: Vec<End>,
    extracts: Vec<End>,
    /// The share's starting point (Java: `next`).
    next: usize,
}

/// A piece as last seen: how it is set (the sides of a neighbour), and what its sides show (`SHOWN`), once read.
#[derive(Clone, Copy)]
struct Seen {
    piece: Piece,
    shown: Option<[u8; 6]>,
}

/// The cables of one dimension.
#[derive(Default)]
struct Dim {
    /// The pieces that ticked since the last run.
    fresh: Vec<([i32; 3], Piece)>,
    known: Map<[i32; 3], Seen>,
    /// Pieces whose sides must be worked out again now (a side was set).
    dirty: Set<[i32; 3]>,
    /// Pieces whose network must be built again, and the networks beside them (Java: `CableNetwork.invalidate`).
    changed: Set<[i32; 3]>,
    nets: Map<u64, Net>,
    member: Map<[i32; 3], u64>,
    next_id: u64,
}

#[derive(Default)]
struct State {
    dims: HashMap<String, Dim, BuildHasherDefault<Fx>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// The cables of `dimension` (no allocation once it has some).
fn dim<R>(dimension: &str, f: impl FnOnce(&mut Dim) -> R) -> R {
    with(|s| {
        if !s.dims.contains_key(dimension) {
            s.dims.insert(dimension.to_owned(), Dim::default());
        }
        f(s.dims.get_mut(dimension).expect("inserted above"))
    })
}

pub fn forget_all() {
    with(|s| *s = State::default());
}

/// Whether a state string shows `prop=value` (without building the pair).
fn shows(state: &str, prop: &str, value: &str) -> bool {
    let Some((_, props)) = state.split_once('[') else { return false };
    props.trim_end_matches(']').split(',').any(|kv| kv.split_once('=') == Some((prop, value)))
}

// ------------------------------------------------------------------------------------------------ the block entity

pub fn block_entity(block: &str) -> Option<(String, Compound)> {
    tier(block).map(|_| (ENTITY.to_owned(), Compound::new()))
}

/// A piece holds what its grade moves in a tick (Java: `PowerBuffer(rate, rate, 0)`; it gives too, see above).
pub fn energy_buffer(block: &str) -> Option<ModEnergyBuffer> {
    let r = tier(block)?.rate();
    Some(ModEnergyBuffer { capacity: r, max_insert: r, max_extract: r, insert_faces: [true; 6], extract_faces: [true; 6] })
}

/// A piece's tick: it says where it is and how it is set, and keeps its sides showing what they touch (Java:
/// `CableBlock.connect`, on every change there; here every half second, or at once after a side was set). Its grade
/// and its sides are kept from tick to tick; its state is read on its first tick and every ten seconds.
pub fn tick(host: &mut dyn Host, pos: [i32; 3], e: &mut ModBlockEntity) {
    let dimension = host.dimension();
    let Some(level) = host.level(dimension) else { return };
    let modes = modes_of(&e.nbt);
    let time = host.level_game_time(level).unwrap_or(0);
    let phase = (time + kiloton_core::machine::pos_as_long(pos)).rem_euclid(200);
    let due = phase % 10 == 0;
    // Most ticks: the grade it had, nothing to redraw; it only says where it is.
    let slow = dim(dimension, |d| {
        let now = !d.dirty.is_empty() && d.dirty.remove(&pos);
        if !now && !due {
            if let Some(seen) = d.known.get_mut(&pos) {
                if seen.shown.is_some() {
                    let piece = Piece { tier: seen.piece.tier, modes };
                    if seen.piece != piece {
                        seen.piece = piece;
                        d.changed.insert(pos);
                    }
                    d.fresh.push((pos, piece));
                    return None;
                }
            }
        }
        let cached = d.known.get(&pos).and_then(|s| s.shown.map(|sh| (s.piece.tier, sh)));
        let near: [Option<bool>; 6] = std::array::from_fn(|i| {
            d.known.get(&step(pos, i as u8)).map(|s| s.piece.modes[opposite(i as u8) as usize] == DISABLED)
        });
        Some((cached, near))
    });
    let Some((cached, near)) = slow else { return };
    // Its own state: on its first tick, then every ten seconds (to follow anything else that set it).
    let (t, mut shown) = match cached {
        Some(c) if phase != 0 => c,
        _ => {
            let Some(state) = host.level_block_state(level, pos) else { return };
            let Some(t) = tier(block_name(&state)) else { return };
            let shown: [u8; 6] = std::array::from_fn(|i| SHOWN.iter().position(|v| shows(&state, SIDES[i], v)).unwrap_or(0) as u8);
            (t, shown)
        }
    };
    let mut moved = false;
    for d in 0..6u8 {
        let want = side(&*host, level, pos, d, modes, near[d as usize]);
        let code = SHOWN.iter().position(|v| *v == want).unwrap_or(0) as u8;
        if shown[d as usize] != code {
            host.set_block_property_with(pos, SIDES[d as usize], want, UPDATE_CLIENTS);
            shown[d as usize] = code;
            moved = true;
        }
    }
    let piece = Piece { tier: t, modes };
    dim(dimension, |d| {
        d.fresh.push((pos, piece));
        // What it touches changed (Java: `neighborChanged` -> `invalidate`), or how it is set: its network is built
        // again.
        let old = d.known.insert(pos, Seen { piece, shown: Some(shown) });
        if old.is_some_and(|o| o.piece != piece) || moved {
            d.changed.insert(pos);
        }
    });
}

/// What a side shows (Java: `CableBlock.side`). `near`: the neighbour is a piece known to the network, and whether
/// its side towards this one is disconnected.
fn side(host: &dyn Host, level: horizon_client::mod_api::LevelRef, pos: [i32; 3], d: u8, modes: [u8; 6], near: Option<bool>) -> &'static str {
    let mode = modes[d as usize];
    if mode == DISABLED {
        return "none";
    }
    if let Some(off) = near {
        return if off { "none" } else { "cable" };
    }
    let q = step(pos, d);
    if !host.level_is_loaded(level, q) {
        return "none";
    }
    let Some(n) = host.level_block_state(level, q) else { return "none" };
    let n = block_name(&n);
    if tier(n).is_some() {
        // A piece that hasn't ticked yet: joined until it says otherwise.
        return "cable";
    }
    if !has_energy(n) {
        return "none";
    }
    if mode == EXTRACT { "extract" } else { "block" }
}

// ------------------------------------------------------------------------------------------------ the network

const MOVE_INSERT: u8 = 1;
const EXTRACT_OP: u8 = 2;
const READ: u8 = 3;
const CAPACITY: u8 = 4;

/// The networks of this level, once a tick after the block entities (`Level::energy`; Java: `CableNetwork.tick`/`run`).
pub fn level_energy(dimension: &str, energy: &mut dyn FnMut(u8, [i32; 3], u8, [i32; 3], u8, i32) -> i32) {
    // Out of the shared state while Horizon is called, back in after.
    let Some(mut d) = with(|s| s.dims.get_mut(dimension).map(std::mem::take)) else { return };
    run_level(&mut d, energy);
    with(|s| {
        if let Some(slot) = s.dims.get_mut(dimension) {
            *slot = d;
        }
    });
}

fn run_level(d: &mut Dim, energy: &mut dyn FnMut(u8, [i32; 3], u8, [i32; 3], u8, i32) -> i32) {
    let pieces = std::mem::take(&mut d.fresh);
    if pieces.is_empty() && d.nets.is_empty() {
        d.known.clear();
        return;
    }
    // The networks that still hold: every piece ticked, none came beside them, none changed.
    let mut dead: Set<u64> = Set::default();
    let mut ticked: Map<u64, usize> = Map::default();
    let mut new_piece = false;
    for (p, _) in &pieces {
        match d.member.get(p) {
            Some(&id) => *ticked.entry(id).or_default() += 1,
            None => {
                new_piece = true;
                for k in 0..6u8 {
                    if let Some(&id) = d.member.get(&step(*p, k)) {
                        dead.insert(id);
                    }
                }
            }
        }
    }
    for (id, net) in &d.nets {
        if ticked.get(id).copied().unwrap_or(0) != net.pieces.len() {
            dead.insert(*id);
        }
    }
    for p in std::mem::take(&mut d.changed) {
        for q in std::iter::once(p).chain((0..6u8).map(|k| step(p, k))) {
            if let Some(&id) = d.member.get(&q) {
                dead.insert(id);
            }
        }
    }
    let rebuilt = new_piece || !dead.is_empty();
    for id in &dead {
        if let Some(net) = d.nets.remove(id) {
            for p in net.pieces {
                d.member.remove(&p);
            }
        }
    }
    if rebuilt {
        let pieces: Map<[i32; 3], Piece> = pieces.iter().copied().collect();
        // Pieces gone or unloaded stop counting as cables for their neighbours.
        d.known.retain(|p, _| pieces.contains_key(p));
        let mut order: Vec<[i32; 3]> = pieces.keys().filter(|p| !d.member.contains_key(*p)).copied().collect();
        order.sort();
        for start in order {
            if d.member.contains_key(&start) {
                continue;
            }
            let net = build(&pieces, &d.known, &d.member, start, energy);
            let id = d.next_id;
            d.next_id += 1;
            for p in &net.pieces {
                d.member.insert(*p, id);
            }
            d.nets.insert(id, net);
        }
    }
    let mut gone = Vec::new();
    for (id, net) in d.nets.iter_mut() {
        if !run(net, energy) {
            gone.push(*id);
        }
    }
    // A piece Horizon no longer has: built again on the next tick.
    for id in gone {
        if let Some(net) = d.nets.remove(&id) {
            for p in net.pieces {
                d.member.remove(&p);
            }
        }
    }
}

/// The pieces joined to `start` and the blocks they touch (Java: `build`).
fn build(
    pieces: &Map<[i32; 3], Piece>,
    known: &Map<[i32; 3], Seen>,
    member: &Map<[i32; 3], u64>,
    start: [i32; 3],
    energy: &mut dyn FnMut(u8, [i32; 3], u8, [i32; 3], u8, i32) -> i32,
) -> Net {
    let mut net = Net::default();
    let mut seen: Set<[i32; 3]> = Set::default();
    seen.insert(start);
    let mut queue = VecDeque::from([start]);
    while let Some(p) = queue.pop_front() {
        if net.pieces.len() >= MAX_PIECES {
            break;
        }
        let piece = pieces[&p];
        let at = net.pieces.len();
        net.pieces.push(p);
        net.rates.push(piece.tier.rate());
        let mut edge = false;
        for d in 0..6u8 {
            let mode = piece.modes[d as usize];
            if mode == DISABLED {
                continue;
            }
            let q = step(p, d);
            if let Some(o) = pieces.get(&q) {
                if o.modes[opposite(d) as usize] != DISABLED && !member.contains_key(&q) && seen.insert(q) {
                    queue.push_back(q);
                }
                continue;
            }
            // A piece that didn't tick this tick (its chunk isn't ticking) is not a block to give to.
            if known.contains_key(&q) || energy(CAPACITY, q, opposite(d), [0; 3], 0, 0) < 0 {
                continue;
            }
            edge = true;
            let end = (q, opposite(d), piece.tier.rate());
            if mode == EXTRACT { net.extracts.push(end) } else { net.inserts.push(end) }
        }
        if edge {
            net.edge.push(at);
        }
    }
    net.held = net.pieces.iter().map(|p| energy(READ, *p, 0, [0; 3], 0, 0).max(0)).collect();
    net.room = net.rates.iter().map(|&r| r as i64).sum();
    net
}

/// One tick of a network (Java: `run`). False when a piece is gone.
fn run(net: &mut Net, energy: &mut dyn FnMut(u8, [i32; 3], u8, [i32; 3], u8, i32) -> i32) -> bool {
    for &i in &net.edge {
        let v = energy(READ, net.pieces[i], 0, [0; 3], 0, 0);
        if v < 0 {
            return false;
        }
        net.held[i] = v;
    }
    let mut pool: i64 = net.held.iter().map(|&v| v as i64).sum();
    for &(q, face, rate) in &net.extracts {
        if pool >= net.room {
            break;
        }
        pool += energy(EXTRACT_OP, q, face, [0; 3], 0, (rate as i64).min(net.room - pool) as i32).max(0) as i64;
    }
    // Shared out evenly: each outlet is offered an equal part of what is left (up to its cable's grade), starting
    // one further along every tick; a second round offers what the full ones turned down to the others.
    let k = net.inserts.len();
    if k > 0 && pool > 0 {
        let first = net.next % k;
        net.next = net.next.wrapping_add(1);
        let mut given = vec![0i64; k];
        for round in 0..2 {
            if pool <= 0 {
                break;
            }
            for i in 0..k {
                if pool <= 0 {
                    break;
                }
                let j = (first + i) % k;
                let (q, face, rate) = net.inserts[j];
                let left = rate as i64 - given[j];
                if left <= 0 {
                    continue;
                }
                let share = if round == 0 { (pool + (k - i) as i64 - 1) / (k - i) as i64 } else { pool };
                let sent = energy(MOVE_INSERT, q, face, [0; 3], 0, left.min(share) as i32).max(0) as i64;
                given[j] += sent;
                pool -= sent;
            }
        }
    }
    // What is left goes back into the pieces in order; Horizon is told only about the ones that change.
    for i in 0..net.pieces.len() {
        let want = (net.rates[i] as i64).min(pool.max(0)) as i32;
        pool -= want as i64;
        let have = net.held[i];
        if want == have {
            continue;
        }
        let p = net.pieces[i];
        let moved = if want > have {
            energy(MOVE_INSERT, p, 0, [0; 3], 0, want - have)
        } else {
            energy(EXTRACT_OP, p, 0, [0; 3], 0, have - want)
        };
        if moved < 0 {
            return false;
        }
        net.held[i] = if want > have { have + moved } else { have - moved };
    }
    true
}

/// The status line's numbers of the network a piece is in: pieces, FE stored, outlets.
fn status(dimension: &str, pos: [i32; 3]) -> (usize, i64, usize) {
    dim(dimension, |d| {
        d.member
            .get(&pos)
            .and_then(|id| d.nets.get(id))
            .map_or((1, 0, 0), |n| (n.pieces.len(), n.held.iter().map(|&v| v as i64).sum(), n.inserts.len() + n.extracts.len()))
    })
}

// ------------------------------------------------------------------------------------------------ players

/// A right click on a piece with an empty hand (Java: `CableBlock.useWithoutItem`): the network's status, or with
/// shift the clicked side's setting cycled. False when the click isn't the cable's (a hand holding something).
pub fn clicked(host: &mut dyn Host, pos: [i32; 3], face: [i32; 3]) -> bool {
    let block = host.block(pos);
    let Some(t) = tier(&block) else { return false };
    let Some(player) = host.acting_player() else { return false };
    let hand_empty = host
        .player_stacks(player.uuid)
        .and_then(|s| s.get(player.selected_slot as usize).cloned())
        .is_none_or(|s| s.is_none());
    if !hand_empty {
        return false;
    }
    let dimension = host.dimension();
    if !player.sneaking {
        let (size, stored, outlets) = status(dimension, pos);
        let name = Text::translate(&format!("block.kiloton.{}", block.trim_start_matches("kiloton:")), &[]);
        host.action_bar(
            player.uuid,
            Text::translate(
                "block.kiloton.cable.status",
                &[
                    name,
                    Text::literal(kiloton_core::machine::grouped(t.rate() as i64)),
                    Text::literal(size.max(1).to_string()),
                    Text::literal(kiloton_core::machine::grouped(stored)),
                    Text::literal(outlets.to_string()),
                ],
            ),
        );
        return true;
    }
    let Some(clicked_face) = (0..6u8).find(|&d| crate::machines::DIRS[d as usize] == face) else { return true };
    // The side the player clicked: the arm or plate the look hits (Java: `sideAt`, from the hit point).
    let shown = host.level(dimension).and_then(|l| host.level_block_state(l, pos)).unwrap_or_default();
    let sides: [&str; 6] = std::array::from_fn(|i| {
        ["none", "cable", "block", "extract"].into_iter().find(|v| shown.contains(&format!("{}={v}", SIDES[i]))).unwrap_or("none")
    });
    let eye = [player.position[0], player.position[1] + player.eye_height as f64, player.position[2]];
    let d = side_at(sides, pos, eye, look(player.yaw, player.pitch)).unwrap_or(clicked_face);
    let q = step(pos, d);
    let to_cable = tier(&host.block(q)).is_some();
    let mut next = NORMAL;
    host.change_mod_block_entity(pos, &mut |e| {
        let mut modes = modes_of(&e.nbt);
        // Normal -> extract -> disconnected -> normal; towards another cable there is nothing to extract.
        next = match modes[d as usize] {
            NORMAL if to_cable => DISABLED,
            NORMAL => EXTRACT,
            EXTRACT => DISABLED,
            _ => NORMAL,
        };
        modes[d as usize] = next;
        e.nbt.insert("sides".into(), Tag::Int(pack(modes)));
    });
    dim(dimension, |s| {
        if let Some(p) = s.known.get_mut(&pos) {
            p.piece.modes[d as usize] = next;
        }
        s.dirty.insert(pos);
        s.changed.insert(pos);
        if to_cable {
            s.dirty.insert(q);
        }
    });
    let key = match next {
        EXTRACT => "extract",
        DISABLED => "disabled",
        _ => "normal",
    };
    host.action_bar(
        player.uuid,
        Text::translate(&format!("block.kiloton.cable.mode.{key}"), &[Text::translate(&format!("dir.kiloton.{}", SIDES[d as usize]), &[])]),
    );
    let c = [pos[0] as f64 + 0.5, pos[1] as f64 + 0.5, pos[2] as f64 + 0.5];
    host.play_sound("minecraft:block.lever.click", c, BLOCKS, 0.35, if next == NORMAL { 0.6 } else { 0.8 });
    true
}

/// `Entity.calculateViewVector(xRot, yRot)`.
fn look(yaw: f32, pitch: f32) -> [f64; 3] {
    let f = pitch as f64 * std::f64::consts::PI / 180.0;
    let g = -(yaw as f64) * std::f64::consts::PI / 180.0;
    [g.sin() * f.cos(), -f.sin(), g.cos() * f.cos()]
}

/// `CableBlock.turn` then `Block.box`: a box modelled pointing north (pixels), turned to face side `d`, in blocks.
fn turned(d: u8, b: [f64; 6]) -> [f64; 6] {
    let t = |x: f64, y: f64, z: f64| match d {
        2 => [x, y, z],
        3 => [16.0 - x, y, 16.0 - z],
        5 => [16.0 - z, y, x],
        4 => [z, y, 16.0 - x],
        1 => [x, 16.0 - z, y],
        _ => [x, z, 16.0 - y],
    };
    let (a, c) = (t(b[0], b[1], b[2]), t(b[3], b[4], b[5]));
    [a[0].min(c[0]), a[1].min(c[1]), a[2].min(c[2]), a[0].max(c[0]), a[1].max(c[1]), a[2].max(c[2])].map(|v| v / 16.0)
}

const ARM: [f64; 6] = [5.0, 5.0, 0.0, 11.0, 11.0, 5.0];
const PLATE: [f64; 6] = [3.0, 3.0, 0.0, 13.0, 13.0, 1.0];
const CORE: [f64; 6] = [5.0 / 16.0, 5.0 / 16.0, 5.0 / 16.0, 11.0 / 16.0, 11.0 / 16.0, 11.0 / 16.0];

/// Where the ray from `o` along `dir` enters box `b`, if it does (slabs).
fn enter(o: [f64; 3], dir: [f64; 3], b: [f64; 6]) -> Option<f64> {
    let (mut lo, mut hi) = (0.0f64, f64::INFINITY);
    for a in 0..3 {
        if dir[a].abs() < 1e-12 {
            if o[a] < b[a] || o[a] > b[a + 3] {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((b[a] - o[a]) / dir[a], (b[a + 3] - o[a]) / dir[a]);
        lo = lo.max(t0.min(t1));
        hi = hi.min(t0.max(t1));
    }
    (lo <= hi).then_some(lo)
}

/// The side whose arm or plate the look from `eye` hits first, outside the core (Java: `CableBlock.sideAt`); None
/// when it hits the core, so the clicked face is the side.
fn side_at(sides: [&str; 6], pos: [i32; 3], eye: [f64; 3], dir: [f64; 3]) -> Option<u8> {
    let o = [eye[0] - pos[0] as f64, eye[1] - pos[1] as f64, eye[2] - pos[2] as f64];
    let mut boxes = vec![CORE];
    for d in 0..6u8 {
        match sides[d as usize] {
            "none" => {}
            "cable" => boxes.push(turned(d, ARM)),
            _ => {
                boxes.push(turned(d, ARM));
                boxes.push(turned(d, PLATE));
            }
        }
    }
    let t = boxes.iter().filter_map(|b| enter(o, dir, *b)).fold(f64::INFINITY, f64::min);
    if !t.is_finite() {
        return None;
    }
    let p = [o[0] + t * dir[0], o[1] + t * dir[1], o[2] + t * dir[2]];
    let inside = |b: [f64; 6], k: f64| (0..3).all(|a| p[a] >= b[a] - k && p[a] <= b[a + 3] + k);
    if inside(CORE, -0.001) {
        return None;
    }
    (0..6u8).find(|&d| sides[d as usize] != "none" && (inside(turned(d, ARM), 0.01) || inside(turned(d, PLATE), 0.01)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arm_looked_at_is_the_side() {
        // A cable running west to east, plate on the west. Looking down at the west arm from above: west (4).
        let sides = ["none", "none", "none", "none", "block", "cable"];
        assert_eq!(side_at(sides, [0, 0, 0], [0.15, 2.0, 0.5], [0.0, -1.0, 0.0]), Some(4));
        assert_eq!(side_at(sides, [0, 0, 0], [0.85, 2.0, 0.5], [0.0, -1.0, 0.0]), Some(5));
        // On the core: the clicked face decides.
        assert_eq!(side_at(sides, [0, 0, 0], [0.5, 2.0, 0.5], [0.0, -1.0, 0.0]), None);
        // Java's view vector: yaw 0 looks south (+z), pitch 90 straight down.
        let v = look(0.0, 0.0);
        assert!((v[2] - 1.0).abs() < 1e-12 && v[0].abs() < 1e-12);
        assert!((look(0.0, 90.0)[1] + 1.0).abs() < 1e-12);
        assert_eq!(turned(4, ARM), [0.0, 0.3125, 0.3125, 0.3125, 0.6875, 0.6875]);
    }

    #[test]
    fn sides_pack_as_java() {
        let modes = [NORMAL, EXTRACT, DISABLED, NORMAL, EXTRACT, DISABLED];
        let mut c = Compound::new();
        c.insert("sides".into(), Tag::Int(pack(modes)));
        assert_eq!(modes_of(&c), modes);
        assert_eq!(pack([EXTRACT, 0, 0, 0, 0, 0]), 1);
        assert_eq!(pack([0, 0, 0, 0, 0, DISABLED]), 2 << 10);
    }
}
