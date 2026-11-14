//! The device (Java: `NukeBlock`, `NukeBlockEntity`, `BombMenu`): two blocks long like a bed, a panel with the pit,
//! the tamper (and on the thermonuclear casing the fuel, the spark plug and the jacket), the timer and the arming
//! code; armed by its panel or by a redstone signal on either half, it counts down with beeps that speed up, warns
//! the players within reach and goes off from the middle of its casing.
//!
//! Horizon opens a block's menu on the block that was clicked, and Java opens the head's panel from either half. So
//! here both halves have a block entity and the device (its load, timer, code and countdown) lives in one of them,
//! the *active* half: the head when it is placed, then whichever half a player last opened. The device moves across
//! in `right_click_block_v2`, before the menu opens.
//!
//! While it is armed nothing can be taken out of it (Java: `removeItem` refuses): its load moves to fifteen slots the
//! panel never shows (`SIZE..SLOTS`) and comes back when it is disarmed.

use horizon_client::mod_api::{
    BlockStateId, Compound, Host, ItemStack, ModBlockEntity, ModContainer, NativeServerPlayer, Tag, Text,
};
use kiloton_core::config::ServerConfig;
use kiloton_core::{BlastModel, FissionModel, FissionState, ThermonuclearModel, Warhead};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

pub const FISSION: &str = "kiloton:nuclear_bomb";
pub const THERMO: &str = "kiloton:thermonuclear_bomb";
/// The block entity type of both casings (Java: `ModBlockEntities.BOMB`).
pub const ENTITY: &str = "kiloton:nuclear_bomb";

pub fn is_bomb(block: &str) -> bool {
    block == FISSION || block == THERMO
}

// Slots, as `NukeBlockEntity`'s.
pub const PIT: usize = 6;
pub const TAMPER: usize = PIT;
pub const PRIMARY: usize = PIT + 1;
pub const FUEL: usize = PRIMARY;
pub const FUEL_SLOTS: usize = 6;
pub const SPARK: usize = FUEL + FUEL_SLOTS;
pub const JACKET: usize = SPARK + 1;
pub const SIZE: usize = JACKET + 1;
/// The container: the panel's slots, then where an armed device keeps its load.
pub const SLOTS: usize = 2 * SIZE;
pub const MIN_TIMER: i32 = 5;
pub const MAX_TIMER: i32 = 600;

// Menu buttons (Java: `NukeBlockEntity.B_*`); `B_COVER` is the safety cover, which only the panel moves.
pub const B_MINUS_10: i32 = 0;
pub const B_PLUS_10: i32 = 1;
pub const B_MINUS_60: i32 = 2;
pub const B_PLUS_60: i32 = 3;
pub const B_ARM: i32 = 4;
pub const B_DISARM: i32 = 5;
pub const B_NEW_CODE: i32 = 6;
pub const B_COVER: i32 = 7;

pub const URANIUM_CORE: &str = "kiloton:uranium_core";
pub const PLUTONIUM_CORE: &str = "kiloton:plutonium_core";
pub const URANIUM_TAMPER: &str = "kiloton:uranium_tamper";
pub const LITHIUM_DEUTERIDE: &str = "kiloton:lithium_deuteride";

/// The numbers the panel reads (the menu's data slots, 16 bits each on the wire).
pub mod data {
    pub const TIMER: usize = 0;
    pub const ARMED: usize = 1;
    pub const REMAINING: usize = 2;
    pub const TOTAL: usize = 3;
    pub const CODE_HI: usize = 4;
    pub const CODE_LO: usize = 5;
    /// Bit i: pit slot i holds a uranium core.
    pub const URANIUM: usize = 6;
    /// Bit i: pit slot i holds a plutonium core.
    pub const PLUTONIUM: usize = 7;
    /// `F_*` below.
    pub const FLAGS: usize = 8;
    /// Bit i: fuel slot i holds a charge.
    pub const FUEL: usize = 9;
    /// The server's yield cap for this casing, an `f32`'s bits in two halves.
    pub const CAP_HI: usize = 10;
    pub const CAP_LO: usize = 11;
    pub const COUNT: usize = 12;
    pub const F_TAMPER: i32 = 1;
    pub const F_JACKET: i32 = 2;
    pub const F_SPARK: i32 = 4;
}

const RED: u32 = 0xFF5555;
const GOLD: u32 = 0xFFAA00;
const GREEN: u32 = 0x55FF55;
/// `SoundSource.BLOCKS`.
const BLOCKS: usize = 4;
/// `Block.UPDATE_ALL | Block.UPDATE_SUPPRESS_DROPS`.
const REMOVE_QUIETLY: i32 = 3 | 32;

pub fn core(item: &str) -> bool {
    item == URANIUM_CORE || item == PLUTONIUM_CORE
}

/// What may sit in a slot of the device (Java: `NukeBlockEntity.accepts`; the slots from `FUEL` on exist in the
/// thermonuclear casing only).
pub fn accepts(slot: usize, item: &str, thermonuclear: bool) -> bool {
    if slot >= PRIMARY && !thermonuclear {
        return false;
    }
    if slot < PIT {
        core(item)
    } else if slot == TAMPER || slot == JACKET {
        item == URANIUM_TAMPER
    } else if (FUEL..SPARK).contains(&slot) {
        item == LITHIUM_DEUTERIDE
    } else {
        slot == SPARK && core(item)
    }
}

/// What a device is loaded with, as the panel shows it (read from the slots on the server, from the data slots on
/// the client).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Load {
    pub uranium: u8,
    pub plutonium: u8,
    pub tamper: bool,
    pub fuel: u8,
    pub spark: bool,
    pub jacket: bool,
}

impl Load {
    /// The load of the slots `slots` (laid out as the panel's).
    pub fn of(slots: &[Option<ItemStack>]) -> Self {
        let item = |i: usize| slots.get(i).and_then(|s| s.as_ref()).map(|s| &*s.item).unwrap_or("");
        let mut l = Load::default();
        for i in 0..PIT {
            match item(i) {
                URANIUM_CORE => l.uranium |= 1 << i,
                PLUTONIUM_CORE => l.plutonium |= 1 << i,
                _ => {}
            }
        }
        l.tamper = item(TAMPER) == URANIUM_TAMPER;
        for i in 0..FUEL_SLOTS {
            if item(FUEL + i) == LITHIUM_DEUTERIDE {
                l.fuel |= 1 << i;
            }
        }
        l.spark = core(item(SPARK));
        l.jacket = item(JACKET) == URANIUM_TAMPER;
        l
    }

    pub fn from_data(d: &[i32]) -> Self {
        let at = |i: usize| d.get(i).copied().unwrap_or(0) & 0xFFFF;
        let flags = at(data::FLAGS);
        Load {
            uranium: at(data::URANIUM) as u8,
            plutonium: at(data::PLUTONIUM) as u8,
            tamper: flags & data::F_TAMPER != 0,
            fuel: at(data::FUEL) as u8,
            spark: flags & data::F_SPARK != 0,
            jacket: flags & data::F_JACKET != 0,
        }
    }

    /// The fission device, or the primary of the thermonuclear one (Java: `NukeBlockEntity.model`).
    pub fn model(&self) -> FissionModel {
        FissionModel::new(self.uranium.count_ones() as i32, self.plutonium.count_ones() as i32, self.tamper)
    }

    /// The two stages (Java: `NukeBlockEntity.thermo`).
    pub fn thermo(&self) -> ThermonuclearModel {
        ThermonuclearModel::new(self.model(), self.fuel.count_ones() as i32, self.spark, self.jacket)
    }

    /// What the device does as it is loaded (Java: `NukeBlockEntity.warhead`).
    pub fn warhead(&self, thermonuclear: bool) -> Warhead {
        if thermonuclear { Warhead::of_thermonuclear(&self.thermo()) } else { Warhead::of_fission(&self.model()) }
    }
}

/// `NukeBlockEntity.clock`: m:ss, rounded up.
pub fn clock(ticks: i32) -> String {
    let s = ((ticks + 19) / 20).max(0);
    format!("{}:{:02}", s / 60, s % 60)
}

/// `NukeBlockEntity.formatCode`.
pub fn format_code(code: i32) -> String {
    format!("{code:06}")
}

// ------------------------------------------------------------------------------------------------ the two halves

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    North,
    South,
    West,
    East,
}

impl Facing {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "north" => Self::North,
            "south" => Self::South,
            "west" => Self::West,
            "east" => Self::East,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::South => "south",
            Self::West => "west",
            Self::East => "east",
        }
    }

    /// `Direction.getClockWise`.
    fn clockwise(self) -> Self {
        match self {
            Self::North => Self::East,
            Self::East => Self::South,
            Self::South => Self::West,
            Self::West => Self::North,
        }
    }

    fn step(self) -> [i32; 2] {
        match self {
            Self::North => [0, -1],
            Self::South => [0, 1],
            Self::West => [-1, 0],
            Self::East => [1, 0],
        }
    }

    /// `Direction.fromYRot` (what `BlockPlaceContext.getHorizontalDirection` gives for a player's yaw).
    pub fn of_yaw(yaw: f32) -> Self {
        match ((yaw as f64 / 90.0 + 0.5).floor() as i64).rem_euclid(4) {
            0 => Self::South,
            1 => Self::West,
            2 => Self::North,
            _ => Self::East,
        }
    }
}

/// One half of a placed device, read from its block state.
#[derive(Clone, Debug, PartialEq)]
pub struct Half {
    pub block: String,
    pub facing: Facing,
    pub head: bool,
    pub armed: bool,
}

impl Half {
    /// From a state as `/setblock` writes it: `kiloton:nuclear_bomb[armed=false,facing=north,part=head]`.
    pub fn parse(state: &str) -> Option<Self> {
        let (block, props) = state.split_once('[').unwrap_or((state, ""));
        if !is_bomb(block) {
            return None;
        }
        let mut facing = Facing::North;
        let mut head = true;
        let mut armed = false;
        for kv in props.trim_end_matches(']').split(',') {
            match kv.split_once('=') {
                Some(("facing", v)) => facing = Facing::parse(v)?,
                Some(("part", v)) => head = v == "head",
                Some(("armed", v)) => armed = v == "true",
                _ => {}
            }
        }
        Some(Half { block: block.to_owned(), facing, head, armed })
    }

    pub fn thermonuclear(&self) -> bool {
        self.block == THERMO
    }

    /// From the head towards the foot (Java: `NukeBlock.footDirection`).
    fn foot_step(&self) -> [i32; 2] {
        self.facing.clockwise().step()
    }

    pub fn partner(&self, pos: [i32; 3]) -> [i32; 3] {
        let [dx, dz] = self.foot_step();
        let s = if self.head { 1 } else { -1 };
        [pos[0] + s * dx, pos[1], pos[2] + s * dz]
    }

    pub fn head_pos(&self, pos: [i32; 3]) -> [i32; 3] {
        if self.head { pos } else { self.partner(pos) }
    }

    /// The middle of the casing, on the ground: where the device goes off (Java: `NukeBlock.centre`).
    pub fn centre(&self, pos: [i32; 3]) -> [f64; 3] {
        let h = self.head_pos(pos);
        let [dx, dz] = self.foot_step();
        [h[0] as f64 + 0.5 + 0.5 * dx as f64, h[1] as f64, h[2] as f64 + 0.5 + 0.5 * dz as f64]
    }
}

fn half_at(host: &dyn Host, dimension: &str, pos: [i32; 3]) -> Option<Half> {
    let level = host.level(dimension)?;
    Half::parse(&host.level_block_state(level, pos)?)
}

// ------------------------------------------------------------------------------------------------ saved state

/// What the active half keeps (Java: `NukeBlockEntity` fields), in its saved compound under the same names.
#[derive(Clone, Debug, PartialEq)]
struct Device {
    timer: i32,
    armed: bool,
    remaining: i32,
    total: i32,
    code: i32,
    failed: i32,
    lockout: i64,
    armed_by: Option<u128>,
    last_user: Option<u128>,
    remote: bool,
    remote_distance: f64,
    ticketed: bool,
    /// A redstone signal reached the device last tick (an arming is the signal's rise).
    powered: bool,
}

impl Device {
    fn new(timer: i32) -> Self {
        Device {
            timer: timer.clamp(MIN_TIMER, MAX_TIMER),
            armed: false,
            remaining: 0,
            total: 0,
            code: new_code(),
            failed: 0,
            lockout: 0,
            armed_by: None,
            last_user: None,
            remote: false,
            remote_distance: 0.0,
            ticketed: false,
            powered: false,
        }
    }

    fn read(nbt: &Compound, default_timer: i32) -> Self {
        let code = get_int(nbt, "code").unwrap_or(-1);
        Device {
            timer: get_int(nbt, "timer").unwrap_or(default_timer).clamp(MIN_TIMER, MAX_TIMER),
            armed: get_bool(nbt, "armed"),
            remaining: get_int(nbt, "remaining").unwrap_or(0),
            total: get_int(nbt, "total").unwrap_or(0),
            code: if code < 0 { new_code() } else { code },
            failed: get_int(nbt, "failed").unwrap_or(0),
            lockout: match get(nbt, "lockout") {
                Some(Tag::Long(v)) => *v,
                _ => 0,
            },
            armed_by: get_string(nbt, "armedBy").and_then(|s| parse_uuid(&s)),
            last_user: get_string(nbt, "lastUser").and_then(|s| parse_uuid(&s)),
            remote: get_bool(nbt, "remote"),
            remote_distance: match get(nbt, "remoteDistance") {
                Some(Tag::Double(bits)) => f64::from_bits(*bits),
                _ => 0.0,
            },
            ticketed: get_bool(nbt, "ticketed"),
            powered: get_bool(nbt, "powered"),
        }
    }

    fn write(&self, nbt: &mut Compound) {
        nbt.insert("timer".into(), Tag::Int(self.timer));
        nbt.insert("armed".into(), Tag::Byte(self.armed as i8));
        nbt.insert("remaining".into(), Tag::Int(self.remaining));
        nbt.insert("total".into(), Tag::Int(self.total));
        nbt.insert("code".into(), Tag::Int(self.code));
        nbt.insert("failed".into(), Tag::Int(self.failed));
        nbt.insert("lockout".into(), Tag::Long(self.lockout));
        nbt.insert("armedBy".into(), Tag::String(self.armed_by.map(uuid_string).unwrap_or_default().as_str().into()));
        nbt.insert("lastUser".into(), Tag::String(self.last_user.map(uuid_string).unwrap_or_default().as_str().into()));
        nbt.insert("remote".into(), Tag::Byte(self.remote as i8));
        nbt.insert("remoteDistance".into(), Tag::Double(self.remote_distance.to_bits()));
        nbt.insert("ticketed".into(), Tag::Byte(self.ticketed as i8));
        nbt.insert("powered".into(), Tag::Byte(self.powered as i8));
    }
}

pub(crate) fn get<'a>(c: &'a Compound, key: &str) -> Option<&'a Tag> {
    c.iter().find(|(k, _)| k.0.iter().copied().eq(key.encode_utf16())).map(|(_, v)| v)
}

pub(crate) fn get_int(c: &Compound, key: &str) -> Option<i32> {
    match get(c, key)? {
        Tag::Int(v) => Some(*v),
        Tag::Short(v) => Some(*v as i32),
        Tag::Byte(v) => Some(*v as i32),
        _ => None,
    }
}

pub(crate) fn get_bool(c: &Compound, key: &str) -> bool {
    matches!(get(c, key), Some(Tag::Byte(v)) if *v != 0)
}

fn get_string(c: &Compound, key: &str) -> Option<String> {
    match get(c, key)? {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// `UUID.toString`.
fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn parse_uuid(s: &str) -> Option<u128> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok()
}

/// A six-digit arming code (Java: `ThreadLocalRandom.nextInt(1_000_000)`).
fn new_code() -> i32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut x = t ^ COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x % 1_000_000) as i32
}

// ------------------------------------------------------------------------------------------------ shared state

#[derive(Default)]
struct State {
    /// Players already warned of each armed device (Java: `NukeBlockEntity.warned`), by dimension and head.
    warned: HashMap<(String, [i32; 3]), HashSet<u128>>,
    /// Armed devices, by dimension and head: their chunk tickets, saved with the world.
    armed: HashSet<(String, [i32; 3])>,
    /// Open menus of mod containers, by dimension and position (`mod_container_viewers`).
    viewers: HashMap<(String, [i32; 3]), u32>,
    /// Positions of active halves that are armed (for the slot rules while such a panel is open).
    armed_halves: HashSet<(String, [i32; 3])>,
    /// Advancements to grant from the server's tick (player, advancement).
    awards: Vec<(u128, &'static str)>,
    /// Conventional blasts for the `level-explode` injection (dimension, centre, power).
    explosions: Vec<(String, [f64; 3], f32)>,
    /// The armed devices of the save, to hold their chunks again when their level opens.
    to_hold: HashMap<String, Vec<[i32; 3]>>,
    dirty: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

fn config() -> ServerConfig {
    crate::blast::with(|b| b.config.clone().unwrap_or_default())
}

fn ticket_owner(head: [i32; 3]) -> String {
    format!("kiloton:bomb/{}/{}/{}", head[0], head[1], head[2])
}

fn chunk_of(p: [i32; 3]) -> [i32; 2] {
    [p[0] >> 4, p[2] >> 4]
}

/// Holds the head's chunk while the device counts down (Java: `BombChunks.keep`).
fn keep(host: &mut dyn Host, dimension: &str, head: [i32; 3], on: bool) {
    let owner = ticket_owner(head);
    if on {
        host.chunk_load_request(&owner, dimension, chunk_of(head));
    } else {
        host.chunk_load_release(&owner, dimension, chunk_of(head));
    }
    with(|s| {
        let key = (dimension.to_owned(), head);
        let changed = if on { s.armed.insert(key) } else { s.armed.remove(&key) };
        s.dirty |= changed;
    });
}

fn award(player: Option<u128>, name: &'static str) {
    if let Some(p) = player {
        with(|s| s.awards.push((p, name)));
    }
}

// ------------------------------------------------------------------------------------------------ the block entity

/// The block entity of either half (`native_block_entity`).
pub fn block_entity(block: &str) -> Option<(String, Compound)> {
    is_bomb(block).then(|| (ENTITY.to_owned(), Compound::new()))
}

/// The container of either half: the panel's slots and the hidden ones. Hoppers reach the panel's slots from every
/// side (Java's device is a plain `Container`).
pub fn container(block: &str) -> Option<ModContainer> {
    if !is_bomb(block) {
        return None;
    }
    let shown: Vec<usize> = (0..if block == THERMO { SIZE } else { PRIMARY }).collect();
    Some(ModContainer {
        slots: SLOTS,
        faces: std::array::from_fn(|_| shown.clone()),
        inputs: shown.clone(),
        outputs: shown,
        menu: Some(block.to_owned()),
    })
}

/// Whether the half `e` holds the device.
fn active(e: &ModBlockEntity) -> bool {
    get_bool(&e.nbt, "device")
}

fn set_active(e: &mut ModBlockEntity, on: bool) {
    e.nbt.insert("device".into(), Tag::Byte(on as i8));
    e.nbt.insert("init".into(), Tag::Byte(1));
}

fn ensure_slots(e: &mut ModBlockEntity) {
    if e.items.len() < SLOTS {
        e.items.resize(SLOTS, None);
    }
}

/// A server tick of either half (`native_block_entity_tick`).
pub fn tick(host: &mut dyn Host, pos: [i32; 3], e: &mut ModBlockEntity) {
    let dimension = host.dimension();
    let Some(level) = host.level(dimension) else { return };
    let Some(here) = host.level_block_state(level, pos).and_then(|s| Half::parse(&s)) else { return };
    ensure_slots(e);
    let partner = here.partner(pos);
    let head = here.head_pos(pos);
    // The other half gone (where its chunk is loaded): this one goes too, with its drops, as Java's `updateShape`
    // and `Block.updateOrDestroy` do. The load in its slots (hidden ones included) drops with it.
    if host.level_is_loaded(level, partner) {
        let whole = host
            .level_block_state(level, partner)
            .and_then(|s| Half::parse(&s))
            .is_some_and(|o| o.block == here.block && o.head != here.head);
        if !whole {
            if active(e) && Device::read(&e.nbt, 30).ticketed {
                keep(host, dimension, head, false);
            }
            with(|s| s.armed_halves.remove(&(dimension.to_owned(), pos)));
            host.destroy_block(pos, None, false);
            return;
        }
    }
    let cfg = config();
    if get(&e.nbt, "init").is_none() {
        // Placed by `place_block` the head is set up there; anything else (a structure, `/setblock`) starts here.
        set_active(e, here.head);
        if here.head {
            Device::new(cfg.timer_seconds).write(&mut e.nbt);
        }
    }
    if !active(e) {
        e.data = vec![0; data::COUNT];
        return;
    }
    let mut d = Device::read(&e.nbt, cfg.timer_seconds);
    // A redstone signal on either half arms a safe device (Java: `neighborChanged` + `hasNeighborSignal`).
    let powered = host.neighbor_signal(dimension, pos) > 0 || host.neighbor_signal(dimension, partner) > 0;
    if powered && !d.powered && !d.armed {
        arm(host, dimension, pos, &here, e, &mut d, None, &cfg);
    }
    d.powered = powered;
    if d.armed {
        countdown(host, dimension, pos, &here, e, &mut d, &cfg);
        if !d.armed && get(&e.nbt, "gone").is_some() {
            return;
        }
    } else if d.ticketed {
        keep(host, dimension, head, false);
        d.ticketed = false;
    }
    with(|s| {
        let key = (dimension.to_owned(), pos);
        if d.armed {
            s.armed_halves.insert(key);
        } else {
            s.armed_halves.remove(&key);
        }
    });
    d.write(&mut e.nbt);
    publish(e, &d, &here, &cfg);
}

/// The panel's numbers (`data`).
fn publish(e: &mut ModBlockEntity, d: &Device, here: &Half, cfg: &ServerConfig) {
    let load = Load::of(if d.armed { &e.items[SIZE..] } else { &e.items[..] });
    let thermo = here.thermonuclear();
    let cap = cfg.clamp_yield(f64::MAX, thermo) as f32;
    let bits = cap.to_bits();
    let mut v = vec![0; data::COUNT];
    v[data::TIMER] = d.timer;
    v[data::ARMED] = d.armed as i32;
    v[data::REMAINING] = d.remaining;
    v[data::TOTAL] = d.total;
    v[data::CODE_HI] = d.code / 1000;
    v[data::CODE_LO] = d.code % 1000;
    v[data::URANIUM] = load.uranium as i32;
    v[data::PLUTONIUM] = load.plutonium as i32;
    v[data::FLAGS] = (load.tamper as i32 * data::F_TAMPER) | (load.jacket as i32 * data::F_JACKET) | (load.spark as i32 * data::F_SPARK);
    v[data::FUEL] = load.fuel as i32;
    v[data::CAP_HI] = (bits >> 16) as i32;
    v[data::CAP_LO] = (bits & 0xFFFF) as i32;
    e.data = v;
}

/// The yield as chat shows it (Java: `NukeBlockEntity.yieldText`), gold.
fn yield_text(w: &Warhead, cfg: &ServerConfig) -> Text {
    let s = if w.nuclear() { FissionModel::format(cfg.clamp_yield(w.yield_kt, w.thermonuclear)) } else { "0 t".to_owned() };
    Text::colored(s, GOLD)
}

/// Starts the countdown (`by` is None for a redstone signal). Java: `NukeBlockEntity.arm`.
#[allow(clippy::too_many_arguments)]
fn arm(
    host: &mut dyn Host,
    dimension: &str,
    pos: [i32; 3],
    here: &Half,
    e: &mut ModBlockEntity,
    d: &mut Device,
    by: Option<&NativeServerPlayer>,
    cfg: &ServerConfig,
) {
    if d.armed {
        return;
    }
    let head = here.head_pos(pos);
    d.armed = true;
    d.total = d.timer * 20;
    d.remaining = d.total;
    with(|s| s.warned.remove(&(dimension.to_owned(), head)));
    d.armed_by = by.map(|p| p.uuid).or(d.last_user);
    d.remote = false;
    keep(host, dimension, head, true);
    d.ticketed = true;
    // The load goes where the panel can't reach it.
    for i in 0..SIZE {
        if e.items[i].is_some() && e.items[SIZE + i].is_none() {
            e.items[SIZE + i] = e.items[i].take();
        }
    }
    set_armed_state(host, pos, here, true);
    let c = here.centre(pos);
    host.play_sound("kiloton:arm", c, BLOCKS, 2.0, 1.0);
    let load = Load::of(&e.items[SIZE..]);
    let w = load.warhead(here.thermonuclear());
    log::info!(
        "Kiloton: device armed at {:?} by {}: {} ({}), {} s",
        head,
        by.map_or("redstone".to_owned(), |p| p.name.clone()),
        FissionModel::format(w.yield_kt),
        if w.thermonuclear { "thermonuclear".to_owned() } else { format!("{:?}", load.model().state).to_uppercase() },
        d.timer
    );
    if let Some(p) = by {
        if cfg.countdown_warning {
            host.action_bar(
                p.uuid,
                Text::translate("kiloton.bomb.armed", &[yield_text(&w, cfg), Text::literal(clock(d.remaining))]).with_color(RED),
            );
        }
    }
    update_warnings(host, dimension, pos, here, &load, d, cfg);
}

/// Java: `NukeBlockEntity.disarm`.
fn disarm(host: &mut dyn Host, dimension: &str, pos: [i32; 3], here: &Half, e: &mut ModBlockEntity, d: &mut Device, by: Option<&NativeServerPlayer>) {
    if !d.armed {
        return;
    }
    if d.remaining <= 100 {
        award(by.map(|p| p.uuid), "last_second");
    }
    d.armed = false;
    keep(host, dimension, here.head_pos(pos), false);
    d.ticketed = false;
    d.remaining = 0;
    d.total = 0;
    // The load comes back to the panel; what a hopper or a player put there meanwhile goes to whoever disarmed.
    for i in 0..SIZE {
        let Some(s) = e.items[SIZE + i].take() else { continue };
        match (e.items[i].take(), by) {
            (None, _) => e.items[i] = Some(s),
            (Some(other), Some(p)) => {
                e.items[i] = Some(s);
                if let Some(rest) = host.give_stack(p.uuid, other) {
                    e.items[SIZE + i] = Some(rest);
                }
            }
            (Some(other), None) => {
                e.items[i] = Some(other);
                e.items[SIZE + i] = Some(s);
            }
        }
    }
    with(|s| s.armed_halves.remove(&(dimension.to_owned(), pos)));
    set_armed_state(host, pos, here, false);
    host.play_sound("minecraft:block.iron_trapdoor.close", here.centre(pos), BLOCKS, 1.0, 0.6);
    if let Some(p) = by {
        host.action_bar(p.uuid, Text::translate("kiloton.bomb.disarmed", &[]).with_color(GREEN));
    }
}

/// The `armed` property of both halves (Java: `setArmedState`; the foot follows by `updateShape` there).
fn set_armed_state(host: &mut dyn Host, pos: [i32; 3], here: &Half, on: bool) {
    let v = if on { "true" } else { "false" };
    host.set_block_property(pos, "armed", v);
    host.set_block_property(here.partner(pos), "armed", v);
}

/// Java: `NukeBlockEntity.countdown`.
fn countdown(host: &mut dyn Host, dimension: &str, pos: [i32; 3], here: &Half, e: &mut ModBlockEntity, d: &mut Device, cfg: &ServerConfig) {
    d.remaining -= 1;
    if d.remaining <= 0 {
        detonate(host, dimension, pos, here, e, d, cfg);
        return;
    }
    let last_ten = d.remaining <= 200;
    if d.remaining % 20 == 0 || (last_ten && d.remaining % 10 == 0) {
        let pitch = if last_ten { 1.6 } else { 1.0 + 0.4 * (1.0 - d.remaining as f32 / d.total.max(1) as f32) };
        host.play_sound("kiloton:beep", here.centre(pos), BLOCKS, if last_ten { 2.5 } else { 1.5 }, pitch);
    }
    if d.remaining % 10 == 0 {
        let load = Load::of(&e.items[SIZE..]);
        update_warnings(host, dimension, pos, here, &load, d, cfg);
    }
}

/// Range of the warning: where the blast still breaks windows (1 psi), at least 256 blocks (Java: `warningRadius`).
fn warning_radius(w: &Warhead, cfg: &ServerConfig) -> f64 {
    if !w.nuclear() {
        return 96.0;
    }
    let r = BlastModel::new(cfg.clamp_yield(w.yield_kt, w.thermonuclear)).radius_for_psi(1.0);
    r.min(cfg.max_radius(w.thermonuclear) as f64).max(256.0)
}

/// One chat line to each player who comes within reach (Java: `updateWarnings`; no bar since 1.4.0).
fn update_warnings(host: &mut dyn Host, dimension: &str, pos: [i32; 3], here: &Half, load: &Load, d: &Device, cfg: &ServerConfig) {
    if !cfg.countdown_warning {
        return;
    }
    let r = warning_radius(&load.warhead(here.thermonuclear()), cfg);
    let c = here.centre(pos);
    let key = (dimension.to_owned(), here.head_pos(pos));
    for p in host.server_players() {
        if p.dimension != dimension {
            continue;
        }
        let dist = ((p.position[0] - c[0]).powi(2) + (p.position[1] - c[1]).powi(2) + (p.position[2] - c[2]).powi(2)).sqrt();
        if dist > r || !with(|s| s.warned.entry(key.clone()).or_default().insert(p.uuid)) {
            continue;
        }
        host.tell(
            p.uuid,
            Text::translate("kiloton.bomb.warning", &[Text::literal(FissionModel::distance(dist)), Text::literal(clock(d.remaining))])
                .with_color(RED),
        );
    }
}

/// Java: `NukeBlockEntity.detonate`.
fn detonate(host: &mut dyn Host, dimension: &str, pos: [i32; 3], here: &Half, e: &mut ModBlockEntity, d: &mut Device, cfg: &ServerConfig) {
    let load = Load::of(&e.items[SIZE..]);
    let m = load.model();
    let thermo = here.thermonuclear();
    let w = load.warhead(thermo);
    let c = here.centre(pos);
    let head = here.head_pos(pos);
    d.armed = false;
    keep(host, dimension, head, false);
    d.ticketed = false;
    with(|s| {
        s.armed_halves.remove(&(dimension.to_owned(), pos));
        s.warned.remove(&(dimension.to_owned(), head));
    });
    for slot in e.items.iter_mut() {
        *slot = None;
    }
    e.nbt.insert("gone".into(), Tag::Byte(1));
    let armer = d.armed_by;
    let kt = cfg.clamp_yield(w.yield_kt, w.thermonuclear);
    if m.nuclear() {
        award(armer, "detonate");
        if m.state == FissionState::Fizzle {
            award(armer, "fizzle");
        }
        if kt >= 100.0 {
            award(armer, "strategic");
        }
        if w.thermonuclear {
            award(armer, "thermonuclear");
        }
        if d.remote && d.remote_distance >= 1000.0 {
            award(armer, "long_distance");
        }
    }
    // Both halves go without drops (the load went off with it).
    if let Some(air) = host.default_state("minecraft:air") {
        host.set_block(dimension, head, air, REMOVE_QUIETLY);
        let foot = if here.head { here.partner(pos) } else { pos };
        host.set_block(dimension, foot, air, REMOVE_QUIETLY);
    }
    if m.nuclear() {
        log::info!(
            "Kiloton: device at {:?}: {} (primary alpha {:.2}, efficiency {:.1}%{})",
            head,
            FissionModel::format(kt),
            m.alpha,
            m.efficiency * 100.0,
            if w.thermonuclear { format!(", thermonuclear, {:.0}% fission", w.fission_fraction * 100.0) } else { String::new() }
        );
        // No player: the chat line a `/nuke` gets back is the command's, not the device's.
        crate::blast::with(|b| b.request(dimension, c, kt, w.fission_fraction, 0));
    } else {
        // Only the explosive lenses go off: a big conventional blast that scatters the pit.
        log::info!("Kiloton: device at {:?}: no nuclear yield ({:?}), the lenses only", head, m.state);
        with(|s| s.explosions.push((dimension.to_owned(), [c[0], c[1] + 0.5, c[2]], 6.0)));
    }
}

// ------------------------------------------------------------------------------------------------ players

/// A panel button, on the server (`native_menu_button`; Java: `BombMenu.clickMenuButton` -> `NukeBlockEntity.button`).
pub fn button(host: &mut dyn Host, pos: [i32; 3], e: &mut ModBlockEntity, id: i32) {
    if e.id != ENTITY || !active(e) {
        return;
    }
    let dimension = host.dimension();
    let Some(here) = half_at(&*host, dimension, pos) else { return };
    ensure_slots(e);
    let cfg = config();
    let by = host.acting_player();
    let mut d = Device::read(&e.nbt, cfg.timer_seconds);
    match id {
        B_MINUS_10 | B_PLUS_10 | B_MINUS_60 | B_PLUS_60 => {
            if d.armed {
                return;
            }
            let step = match id {
                B_MINUS_10 => -10,
                B_PLUS_10 => 10,
                B_MINUS_60 => -60,
                _ => 60,
            };
            d.timer = (d.timer + step).clamp(MIN_TIMER, MAX_TIMER);
        }
        B_ARM => arm(host, dimension, pos, &here, e, &mut d, by.as_ref(), &cfg),
        B_DISARM => disarm(host, dimension, pos, &here, e, &mut d, by.as_ref()),
        B_NEW_CODE => {
            if d.armed {
                return;
            }
            d.code = new_code();
            d.failed = 0;
            d.lockout = 0;
        }
        _ => return,
    }
    with(|s| {
        let key = (dimension.to_owned(), pos);
        if d.armed {
            s.armed_halves.insert(key);
        } else {
            s.armed_halves.remove(&key);
        }
    });
    d.write(&mut e.nbt);
    publish(e, &d, &here, &cfg);
}

/// A right click on either half, before its use (`native_right_click_block_v2`): the device moves into the half
/// clicked, so its menu opens with the panel. False when the click must not go on (someone has the other half's
/// panel open, or the other half holds items it shouldn't).
pub fn clicked(host: &mut dyn Host, pos: [i32; 3]) -> bool {
    let dimension = host.dimension();
    let Some(here) = half_at(&*host, dimension, pos) else { return true };
    let user = host.acting_player().map(|p| p.uuid);
    let Some(this) = host.mod_block_entity(pos) else { return true };
    if this.id != ENTITY {
        return true;
    }
    let partner = here.partner(pos);
    if active(&this) {
        host.change_mod_block_entity(pos, &mut |e| remember_user(e, user));
        return true;
    }
    let Some(other) = host.mod_block_entity(partner) else { return true };
    if other.id != ENTITY || !active(&other) {
        return true;
    }
    let open = with(|s| s.viewers.get(&(dimension.to_owned(), partner)).copied().unwrap_or(0) > 0);
    if open || this.items.iter().any(Option::is_some) {
        return false;
    }
    // Take everything out of the other half and put it here.
    let mut moved: Option<(Compound, Vec<Option<ItemStack>>, Vec<i32>)> = None;
    host.change_mod_block_entity(partner, &mut |e| {
        let nbt = std::mem::take(&mut e.nbt);
        let items = std::mem::replace(&mut e.items, vec![None; SLOTS]);
        let data = std::mem::replace(&mut e.data, vec![0; data::COUNT]);
        set_active(e, false);
        moved = Some((nbt, items, data));
    });
    let Some((nbt, mut items, data)) = moved else { return true };
    items.resize(SLOTS, None);
    let placed = host.change_mod_block_entity(pos, &mut |e| {
        e.nbt = nbt.clone();
        e.items = items.clone();
        e.data = data.clone();
        set_active(e, true);
        remember_user(e, user);
    });
    if !placed {
        // Put it back where it was.
        host.change_mod_block_entity(partner, &mut |e| {
            e.nbt = nbt.clone();
            e.items = items.clone();
            e.data = data.clone();
        });
        return true;
    }
    with(|s| {
        if s.armed_halves.remove(&(dimension.to_owned(), partner)) {
            s.armed_halves.insert((dimension.to_owned(), pos));
        }
    });
    true
}

/// Who last opened the panel (Java: `lastUser`, set by `createMenu`), for a redstone arming's advancements.
fn remember_user(e: &mut ModBlockEntity, user: Option<u128>) {
    if let Some(u) = user {
        e.nbt.insert("lastUser".into(), Tag::String(uuid_string(u).as_str().into()));
    }
}

/// A player breaks a half (`native_destroy_block`). False keeps the block: an armed device can't be broken but in
/// creative (Java: `getDestroyProgress` 0). Otherwise the other half goes too, as Java's `updateShape` makes it go:
/// with its drops in survival, quietly in creative (Java's `playerWillDestroy`), the load dropping where it was.
pub fn breaking(host: &mut dyn Host, pos: [i32; 3]) -> bool {
    let dimension = host.dimension();
    let Some(here) = half_at(&*host, dimension, pos) else { return true };
    let creative = host.acting_player().is_some_and(|p| p.game_mode == 1);
    if here.armed && !creative {
        return false;
    }
    let partner = here.partner(pos);
    let head = here.head_pos(pos);
    for p in [pos, partner] {
        if let Some(e) = host.mod_block_entity(p) {
            if e.id == ENTITY && active(&e) && Device::read(&e.nbt, 30).ticketed {
                keep(host, dimension, head, false);
            }
        }
        with(|s| s.armed_halves.remove(&(dimension.to_owned(), p)));
    }
    let Some(other) = half_at(&*host, dimension, partner) else { return true };
    if other.block != here.block || other.head == here.head {
        return true;
    }
    if creative {
        // Java: the head goes without its item; its pit drops unless the foot was broken (`forget`). Here the load
        // is in whichever half is active: an armed or a breaking head keeps it, the foot's click forgets it.
        if here.head {
            let mut load = Vec::new();
            host.change_mod_block_entity(partner, &mut |e| load = std::mem::take(&mut e.items));
            if load.iter().any(Option::is_some) {
                host.change_mod_block_entity(pos, &mut |e| {
                    e.items.resize(SLOTS, None);
                    for (slot, s) in e.items.iter_mut().zip(load.iter()) {
                        if slot.is_none() {
                            *slot = s.clone();
                        }
                    }
                });
            }
        } else {
            for p in [pos, partner] {
                host.change_mod_block_entity(p, &mut |e| {
                    for s in e.items.iter_mut() {
                        *s = None;
                    }
                });
            }
        }
        if let Some(air) = host.default_state("minecraft:air") {
            host.set_block(dimension, partner, air, REMOVE_QUIETLY);
        }
    } else {
        host.destroy_block(partner, None, false);
    }
    true
}

/// An armed device digs at no speed (Java: `getDestroyProgress` 0 but in creative, where it breaks at once anyway).
pub fn armed_state(host: &dyn Host, state: BlockStateId) -> bool {
    host.block_state_property(state, "part").is_some() && host.block_state_property(state, "armed") == Some("true")
}

/// The state the device is placed as (`native_state_for_placement`): facing the way the player looks, refused when
/// the foot's block can't be replaced (Java: `getStateForPlacement`).
pub fn placement_facing(yaw: f32, foot_replaceable: impl Fn([i32; 3]) -> bool, pos: [i32; 3]) -> Option<Facing> {
    let facing = Facing::of_yaw(yaw);
    let [dx, dz] = facing.clockwise().step();
    foot_replaceable([pos[0] + dx, pos[1], pos[2] + dz]).then_some(facing)
}

/// The device was placed (`native_place_block`): the foot goes beside the head, and the head gets the device
/// (Java: `setPlacedBy`).
pub fn placed(host: &mut dyn Host, pos: [i32; 3], state: BlockStateId) {
    let dimension = host.dimension();
    let Some(here) = half_at(&*host, dimension, pos) else { return };
    if !here.head {
        return;
    }
    let foot = here.partner(pos);
    if host.set_block(dimension, foot, state, 3) {
        host.set_block_property(foot, "part", "foot");
    }
    let timer = config().timer_seconds;
    let user = host.acting_player().map(|p| p.uuid);
    host.change_mod_block_entity(pos, &mut |e| {
        ensure_slots(e);
        set_active(e, true);
        let mut d = Device::new(timer);
        d.last_user = user;
        d.write(&mut e.nbt);
    });
    host.change_mod_block_entity(foot, &mut |e| {
        ensure_slots(e);
        set_active(e, false);
    });
}

/// What hoppers may put into the device (`native_container_accepts`): the panel's rules, one item a slot, nothing
/// while it is armed, nothing into the half that doesn't hold it.
pub fn hopper_accepts(host: &dyn Host, pos: [i32; 3], e: &ModBlockEntity, slot: usize, item: &str) -> bool {
    let thermo = host.block(pos) == THERMO;
    slot < SIZE
        && active(e)
        && !get_bool(&e.nbt, "armed")
        && e.items.get(slot).is_none_or(Option::is_none)
        && accepts(slot, item, thermo)
}

/// What a slot of the panel takes from a player (`native_menu_slot_rules`). The hook knows only the menu type, so
/// while any armed device's panel is open no panel takes anything (an armed panel's slots are empty, and Java
/// refuses them).
pub fn slot_rules(menu: &str, slot: usize, item: &str) -> Option<(bool, u32)> {
    if !is_bomb(menu) {
        return None;
    }
    let armed_open = with(|s| s.armed_halves.iter().any(|k| s.viewers.get(k).copied().unwrap_or(0) > 0));
    Some((slot < SIZE && !armed_open && accepts(slot, item, menu == THERMO), 1))
}

/// How many players have a mod container open (`native_mod_container_viewers`).
pub fn viewers(dimension: &str, pos: [i32; 3], n: u32) {
    with(|s| {
        if n == 0 {
            s.viewers.remove(&(dimension.to_owned(), pos));
        } else {
            s.viewers.insert((dimension.to_owned(), pos), n);
        }
    });
}

/// After a level's tick (`level_tick_post`): the advancements earned (Java: `Awards.award`, a criterion `done`
/// each), and the tickets of this level's devices that went without their tick seeing it (broken by an explosion,
/// swept by a blast).
pub fn level_tick(host: &mut dyn Host, dimension: &str) {
    let awards = with(|s| std::mem::take(&mut s.awards));
    if !awards.is_empty() {
        let players = host.server_players();
        for (uuid, name) in awards {
            if let Some(p) = players.iter().find(|p| p.uuid == uuid) {
                host.run_command(&format!("advancement grant {} only kiloton:{name}", p.name));
            }
        }
    }
    let Some(level) = host.level(dimension) else { return };
    if host.level_game_time(level).unwrap_or(0) % 40 == 0 {
        let held: Vec<(String, [i32; 3])> = with(|s| s.armed.iter().filter(|k| k.0 == dimension).cloned().collect());
        for (dimension, head) in held {
            if !host.level_is_loaded(level, head) {
                continue;
            }
            let still = host.level_block_state(level, head).and_then(|s| Half::parse(&s)).is_some_and(|h| h.armed);
            if !still {
                let owner = ticket_owner(head);
                host.chunk_load_release(&owner, &dimension, chunk_of(head));
                with(|s| {
                    s.armed.remove(&(dimension.clone(), head));
                    s.dirty = true;
                });
            }
        }
    }
}

/// The lenses-only blasts queued for this level (the `level-explode` injection).
pub fn explosions(dimension: &str) -> Vec<([f64; 3], f32)> {
    with(|s| {
        if s.explosions.is_empty() {
            return Vec::new();
        }
        let (here, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut s.explosions).into_iter().partition(|x| x.0 == dimension);
        s.explosions = rest;
        here.into_iter().map(|(_, c, p)| (c, p)).collect()
    })
}

// ------------------------------------------------------------------------------------------------ saving

const FILE: &str = "armed.json";

/// The armed devices' chunks, saved with the world (Java's tickets are saved with the level; Horizon's are not).
pub fn save(world: &Path) {
    let (dirty, list) = with(|s| (s.dirty, s.armed.iter().cloned().collect::<Vec<_>>()));
    if !dirty {
        return;
    }
    let json = serde_json::Value::Array(
        list.iter().map(|(d, p)| serde_json::json!({ "dimension": d, "pos": [p[0], p[1], p[2]] })).collect(),
    );
    let dir = world.join("kiloton");
    let _ = std::fs::create_dir_all(&dir);
    let tmp = dir.join(format!("{FILE}.tmp"));
    if std::fs::write(&tmp, serde_json::to_vec_pretty(&json).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(FILE));
        with(|s| s.dirty = false);
    }
}

/// A level opened: the chunks of its armed devices are held again, so their countdown goes on.
pub fn level_load(host: &mut dyn Host, world: &Path, dimension: &str) {
    let loaded = with(|s| !s.to_hold.is_empty() || s.armed.iter().any(|k| k.0 == dimension));
    if !loaded {
        let text = std::fs::read(world.join("kiloton").join(FILE)).unwrap_or_default();
        let list: Vec<serde_json::Value> = serde_json::from_slice(&text).unwrap_or_default();
        with(|s| {
            for v in list {
                let (Some(d), Some(p)) = (v["dimension"].as_str(), v["pos"].as_array()) else { continue };
                let p: Vec<i32> = p.iter().filter_map(|x| x.as_i64().map(|x| x as i32)).collect();
                if p.len() == 3 {
                    s.to_hold.entry(d.to_owned()).or_default().push([p[0], p[1], p[2]]);
                }
            }
        });
    }
    let here = with(|s| s.to_hold.remove(dimension).unwrap_or_default());
    for head in here {
        keep(host, dimension, head, true);
    }
}

/// The world closed: nothing carries over to the next one.
pub fn forget_all() {
    with(|s| *s = State::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves() {
        let h = Half::parse("kiloton:nuclear_bomb[armed=false,facing=north,part=head]").unwrap();
        assert_eq!(h.partner([0, 64, 0]), [1, 64, 0]);
        assert_eq!(h.centre([0, 64, 0]), [1.0, 64.0, 0.5]);
        let f = Half::parse("kiloton:nuclear_bomb[armed=true,facing=north,part=foot]").unwrap();
        assert_eq!(f.partner([1, 64, 0]), [0, 64, 0]);
        assert_eq!(f.centre([1, 64, 0]), [1.0, 64.0, 0.5]);
        assert!(f.armed && !f.head);
        assert!(Half::parse("minecraft:stone").is_none());
        assert_eq!(Facing::of_yaw(0.0), Facing::South);
        assert_eq!(Facing::of_yaw(90.0), Facing::West);
        assert_eq!(Facing::of_yaw(-90.0), Facing::East);
        assert_eq!(Facing::of_yaw(180.0), Facing::North);
        assert_eq!(Facing::of_yaw(-170.0), Facing::North);
    }

    #[test]
    fn uuids_and_clock() {
        let u = 0x0123_4567_89ab_cdef_0011_2233_4455_6677u128;
        assert_eq!(uuid_string(u), "01234567-89ab-cdef-0011-223344556677");
        assert_eq!(parse_uuid(&uuid_string(u)), Some(u));
        assert_eq!(clock(600), "0:30");
        assert_eq!(clock(1), "0:01");
        assert_eq!(clock(12000), "10:00");
        assert_eq!(format_code(42), "000042");
        assert!((0..1_000_000).contains(&new_code()));
    }

    #[test]
    fn slots() {
        assert!(accepts(0, URANIUM_CORE, false) && accepts(5, PLUTONIUM_CORE, false));
        assert!(accepts(TAMPER, URANIUM_TAMPER, false) && !accepts(TAMPER, URANIUM_CORE, false));
        assert!(!accepts(FUEL, LITHIUM_DEUTERIDE, false) && accepts(FUEL, LITHIUM_DEUTERIDE, true));
        assert!(accepts(SPARK, PLUTONIUM_CORE, true) && accepts(JACKET, URANIUM_TAMPER, true));
        let stack = |s: &str| Some(ItemStack { item: s.into(), count: 1, damage: 0, extra: None });
        let mut slots = vec![None; SIZE];
        slots[0] = stack(PLUTONIUM_CORE);
        slots[1] = stack(PLUTONIUM_CORE);
        slots[2] = stack(URANIUM_CORE);
        slots[TAMPER] = stack(URANIUM_TAMPER);
        slots[FUEL + 3] = stack(LITHIUM_DEUTERIDE);
        slots[SPARK] = stack(URANIUM_CORE);
        let l = Load::of(&slots);
        assert_eq!((l.plutonium, l.uranium, l.tamper, l.fuel, l.spark, l.jacket), (0b11, 0b100, true, 0b1000, true, false));
        let mut e = ModBlockEntity::default();
        let d = Device::new(30);
        let here = Half::parse("kiloton:thermonuclear_bomb[armed=false,facing=east,part=head]").unwrap();
        e.items = slots;
        e.items.resize(SLOTS, None);
        publish(&mut e, &d, &here, &ServerConfig::default());
        assert_eq!(Load::from_data(&e.data), l);
        let bits = ((e.data[data::CAP_HI] as u32 & 0xFFFF) << 16) | (e.data[data::CAP_LO] as u32 & 0xFFFF);
        assert_eq!(f32::from_bits(bits), 20000.0);
        let mut nbt = Compound::new();
        d.write(&mut nbt);
        assert_eq!(Device::read(&nbt, 30), d);
    }
}
