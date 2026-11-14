//! The machines of the uranium chain, the exchange column and the creative energy source (Java: `MachineBlock`,
//! `MachineBlockEntity`, `MachineMenu`, `MachineEnergy`, `CreativeSourceBlockEntity`). A machine has five slots, an
//! energy buffer that a slot of coal or a nuclear cell can also fill, and a recipe in progress; the reactor works the
//! other way round: it burns low-enriched fuel elements, makes power, pushes it into the blocks next to it and breeds
//! plutonium from depleted uranium.
//!
//! The buffer is Horizon's (`block_entity_energy`, the int `energy` of the saved compound, as Java's
//! `SimpleEnergyHandler`). A block entity is out of the world while it ticks, so what moves energy between blocks
//! (the reactor's push, the creative source) is queued by the tick and done right after the level's block entities, in
//! `Level::energy` (the game's `kiloton` glue).

use crate::bomb::get_int;
use horizon_client::mod_api::{Compound, Host, ItemStack, ModBlockEntity, ModContainer, ModEnergyBuffer, Tag};
use kiloton_core::config::ServerConfig;
use kiloton_core::machine::{
    self, MachineKind, Recipe, CELL_LIFE, CELL_OUTPUT, CREATIVE_CAPACITY, DEPLETED, FUEL, IN_A, IN_B, LEU, NUCLEAR_CELL,
    OUT_A, OUT_B, REACTOR_FE_PER_TICK, REACTOR_FUEL_TICKS, SLOTS,
};
use std::sync::Mutex;

/// The block entity type of the five machines (Java: `ModBlockEntities.MACHINE`).
pub const ENTITY: &str = "kiloton:machine";
pub const CREATIVE_SOURCE: &str = "kiloton:creative_source";

/// The numbers the screen reads (the menu's data slots, 16 bits each on the wire). 0..8 as Java's `MachineBlockEntity`
/// data; the rest are what Java's screen reads from the server's config and the menu's slots, which a Horizon screen
/// doesn't have.
pub mod data {
    pub const PROGRESS: usize = 0;
    pub const TICKS: usize = 1;
    pub const ENERGY_LO: usize = 2;
    pub const ENERGY_HI: usize = 3;
    pub const BURN: usize = 4;
    pub const BURN_TOTAL: usize = 5;
    pub const CASCADE: usize = 6;
    pub const WORKING: usize = 7;
    /// The recipe of the input slot, its index + 1 (0: nothing that a recipe takes).
    pub const RECIPE: usize = 8;
    /// The tooltip's power while working and energy per run, FE, in two halves each.
    pub const DRAW_LO: usize = 9;
    pub const DRAW_HI: usize = 10;
    pub const RUN_LO: usize = 11;
    pub const RUN_HI: usize = 12;
    pub const COUNT: usize = 13;

    pub fn split(v: i64, out: &mut [i32], lo: usize, hi: usize) {
        let v = v.clamp(0, u32::MAX as i64) as u32;
        out[lo] = (v & 0xFFFF) as i32;
        out[hi] = (v >> 16) as i32;
    }

    pub fn join(d: &[i32], lo: usize, hi: usize) -> u32 {
        let at = |i: usize| (d.get(i).copied().unwrap_or(0) & 0xFFFF) as u32;
        at(lo) | (at(hi) << 16)
    }
}

/// `Block.UPDATE_CLIENTS`, for the `lit` property as Java's `serverTick` sets it.
const UPDATE_CLIENTS: i32 = 2;

pub fn kind(block: &str) -> Option<MachineKind> {
    MachineKind::of_block(block)
}

pub fn is_machine(block: &str) -> bool {
    kind(block).is_some()
}

fn block_name(state: &str) -> &str {
    state.split_once('[').map_or(state, |(b, _)| b)
}

fn config() -> ServerConfig {
    crate::blast::with(|b| b.config.clone().unwrap_or_default())
}

// ------------------------------------------------------------------------------------------------ queued moves

/// What moves energy between blocks this tick, done after the block entities (`Level::energy`).
#[derive(Default)]
struct State {
    /// Reactors with energy to push: dimension, position, the most to push.
    pushes: Vec<(String, [i32; 3], i32)>,
    /// Creative sources: dimension, position.
    sources: Vec<(String, [i32; 3])>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// The world closed: nothing carries over to the next one.
pub fn forget_all() {
    with(|s| *s = State::default());
}

/// Energy operations of `Level::energy` (its `energy` closure's first argument).
pub const OP_MOVE: u8 = 0;
pub const OP_INSERT: u8 = 1;

/// `Direction.values()`: down, up, north, south, west, east (Horizon's `Direction::ALL`, the same order).
pub const DIRS: [[i32; 3]; 6] = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];

pub fn opposite(d: u8) -> u8 {
    d ^ 1
}

pub fn step(p: [i32; 3], d: u8) -> [i32; 3] {
    let s = DIRS[d as usize];
    [p[0] + s[0], p[1] + s[1], p[2] + s[2]]
}

/// After the level's block entities ticked: the reactors push their power into what they touch (Java:
/// `tickReactor`'s push, at most `REACTOR_PUSH`, side after side), the creative sources fill it (`PowerBuffer.push`).
pub fn level_energy(dimension: &str, energy: &mut dyn FnMut(u8, [i32; 3], u8, [i32; 3], u8, i32) -> i32) {
    let (pushes, sources) = with(|s| {
        let (p, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut s.pushes).into_iter().partition(|x| x.0 == dimension);
        s.pushes = rest;
        let (c, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut s.sources).into_iter().partition(|x| x.0 == dimension);
        s.sources = rest;
        (p, c)
    });
    for (_, pos, mut budget) in pushes {
        for d in 0..6u8 {
            if budget <= 0 {
                break;
            }
            budget -= energy(OP_MOVE, pos, d, step(pos, d), opposite(d), budget).max(0);
        }
    }
    for (_, pos) in sources {
        let mut budget = CREATIVE_CAPACITY;
        for d in 0..6u8 {
            if budget <= 0 {
                break;
            }
            budget -= energy(OP_INSERT, step(pos, d), opposite(d), [0; 3], 0, budget).max(0);
        }
    }
}

// ------------------------------------------------------------------------------------------------ the block entity

pub fn block_entity(block: &str) -> Option<(String, Compound)> {
    if is_machine(block) {
        Some((ENTITY.to_owned(), Compound::new()))
    } else if block == CREATIVE_SOURCE {
        Some((CREATIVE_SOURCE.to_owned(), Compound::new()))
    } else {
        None
    }
}

pub fn ticks(block: &str) -> bool {
    is_machine(block) || block == CREATIVE_SOURCE
}

/// The slots each face reaches (Java: `getSlotsForFace`: inputs from the top, fuel and inputs from the sides,
/// outputs from below), and the machine's menu.
pub fn container(block: &str) -> Option<ModContainer> {
    kind(block)?;
    let top = vec![IN_A, IN_B];
    let side = vec![FUEL, IN_A, IN_B];
    Some(ModContainer {
        slots: SLOTS,
        faces: [vec![OUT_A, OUT_B], top, side.clone(), side.clone(), side.clone(), side],
        inputs: vec![IN_A, IN_B, FUEL],
        outputs: vec![OUT_A, OUT_B],
        menu: Some(block.to_owned()),
    })
}

/// The energy buffer (Java: `MachineEnergy`; the creative source's `PowerBuffer`, always full).
pub fn energy_buffer(block: &str) -> Option<ModEnergyBuffer> {
    if let Some(k) = kind(block) {
        return Some(ModEnergyBuffer {
            capacity: k.capacity(),
            max_insert: k.max_receive(),
            max_extract: k.max_extract(),
            insert_faces: [k.max_receive() > 0; 6],
            extract_faces: [k.max_extract() > 0; 6],
        });
    }
    (block == CREATIVE_SOURCE).then(|| ModEnergyBuffer {
        capacity: CREATIVE_CAPACITY,
        max_insert: 0,
        max_extract: CREATIVE_CAPACITY,
        insert_faces: [false; 6],
        extract_faces: [true; 6],
    })
}

/// Whether a hopper or a pipe may put `item` into `slot` (Java: `canPlaceItemThroughFace` = `accepts`).
pub fn hopper_accepts(host: &dyn Host, block: &str, slot: usize, item: &str) -> bool {
    kind(block).is_some_and(|k| k.accepts(slot, item, &|i, t| in_tag(host, i, t)))
}

/// What a slot of a machine's screen takes from a player (Java: `MachineMenu`'s slots: never an output). The hook
/// has no host, so a `c:` tag counts only Kiloton's own members here.
pub fn slot_rules(menu: &str, slot: usize, item: &str) -> Option<bool> {
    let k = kind(menu)?;
    Some(slot != OUT_A && slot != OUT_B && k.accepts(slot, item, &|_, _| false))
}

fn in_tag(host: &dyn Host, item: &str, tag: &str) -> bool {
    host.server_item_in_tag(&ItemStack::new(item, 1), tag).unwrap_or(false)
}

/// The block entity's numbers, as Java keeps them (saved under the same names).
struct Fields {
    energy: i32,
    progress: i32,
    burn: i32,
    burn_total: i32,
    cascade: i32,
}

impl Fields {
    fn read(nbt: &Compound, k: MachineKind) -> Self {
        Fields {
            energy: get_int(nbt, "energy").unwrap_or(0).clamp(0, k.capacity()),
            progress: get_int(nbt, "progress").unwrap_or(0),
            burn: get_int(nbt, "burn").unwrap_or(0),
            burn_total: get_int(nbt, "burn_total").unwrap_or(0),
            // Not saved in Java (recounted every second); kept here so a reload doesn't reset the screen's line.
            cascade: get_int(nbt, "cascade").unwrap_or(10).clamp(10, 40),
        }
    }

    fn write(&self, nbt: &mut Compound) {
        nbt.insert("energy".into(), Tag::Int(self.energy));
        nbt.insert("progress".into(), Tag::Int(self.progress));
        nbt.insert("burn".into(), Tag::Int(self.burn));
        nbt.insert("burn_total".into(), Tag::Int(self.burn_total));
        nbt.insert("cascade".into(), Tag::Int(self.cascade));
    }
}

fn item(items: &[Option<ItemStack>], slot: usize) -> &str {
    items.get(slot).and_then(|s| s.as_ref()).map_or("", |s| &*s.item)
}

fn count(items: &[Option<ItemStack>], slot: usize) -> u32 {
    items.get(slot).and_then(|s| s.as_ref()).map_or(0, |s| s.count)
}

fn shrink(items: &mut [Option<ItemStack>], slot: usize, n: u32) {
    if let Some(s) = items.get_mut(slot) {
        if let Some(st) = s.as_mut() {
            if st.count <= n {
                *s = None;
            } else {
                st.count -= n;
            }
        }
    }
}

/// Java: `fitsIn`: the slot is empty, or holds the same item with room for `n` more.
fn fits_in(host: &dyn Host, items: &[Option<ItemStack>], slot: usize, out: Option<(&str, u32)>) -> bool {
    let Some((id, n)) = out else { return true };
    match items.get(slot).and_then(|s| s.as_ref()) {
        None => true,
        Some(there) => {
            let max = host.max_stack_size(id).unwrap_or(64);
            &*there.item == id && there.extra.is_none() && there.damage == 0 && there.count + n <= max
        }
    }
}

fn fits(host: &dyn Host, items: &[Option<ItemStack>], r: &Recipe) -> bool {
    fits_in(host, items, OUT_A, Some(r.out_a)) && fits_in(host, items, OUT_B, r.out_b)
}

/// Java: `craft` + `put`.
fn craft(items: &mut [Option<ItemStack>], r: &Recipe) {
    shrink(items, IN_A, r.count);
    for (slot, out) in [(OUT_A, Some(r.out_a)), (OUT_B, r.out_b)] {
        let Some((id, n)) = out else { continue };
        match items[slot].as_mut() {
            Some(there) => there.count += n,
            None => items[slot] = Some(ItemStack::new(id, n)),
        }
    }
}

/// A server tick of a machine or a creative source (`native_block_entity_tick`; Java: `MachineBlockEntity.serverTick`,
/// `CreativeSourceBlockEntity.serverTick`).
pub fn tick(host: &mut dyn Host, pos: [i32; 3], e: &mut ModBlockEntity) {
    let dimension = host.dimension();
    if e.id == CREATIVE_SOURCE {
        e.nbt.insert("energy".into(), Tag::Int(CREATIVE_CAPACITY));
        with(|s| s.sources.push((dimension.to_owned(), pos)));
        return;
    }
    let Some(level) = host.level(dimension) else { return };
    let Some(state) = host.level_block_state(level, pos) else { return };
    let Some(k) = kind(block_name(&state)) else { return };
    if e.items.len() < SLOTS {
        e.items.resize(SLOTS, None);
    }
    let cfg = config();
    let mut f = Fields::read(&e.nbt, k);
    let mut items = std::mem::take(&mut e.items);
    let working = if k == MachineKind::Reactor {
        tick_reactor(&*host, &mut items, &mut f, &cfg)
    } else {
        let time = host.level_game_time(level).unwrap_or(0);
        if k == MachineKind::Centrifuge && time.rem_euclid(20) == machine::pos_as_long(pos).rem_euclid(20) {
            let n = (0..6u8)
                .filter(|&d| host.level_block_state(level, step(pos, d)).is_some_and(|s| block_name(&s) == "kiloton:gas_centrifuge"))
                .count() as i32;
            f.cascade = 10 + 5 * n;
        }
        tick_machine(&*host, k, &mut items, &mut f, &cfg)
    };
    if k == MachineKind::Reactor {
        let budget = f.energy.min(machine::REACTOR_PUSH);
        if budget > 0 {
            with(|s| s.pushes.push((dimension.to_owned(), pos, budget)));
        }
    }
    let lit = state.contains("lit=true");
    if lit != working {
        host.set_block_property_with(pos, "lit", if working { "true" } else { "false" }, UPDATE_CLIENTS);
    }
    publish(&*host, k, &items, &f, working, &cfg, e);
    e.items = items;
    f.write(&mut e.nbt);
}

/// Java: `tickMachine`. Whether it works this tick.
fn tick_machine(host: &dyn Host, k: MachineKind, items: &mut [Option<ItemStack>], f: &mut Fields, cfg: &ServerConfig) -> bool {
    let cap = k.capacity();
    // A nuclear cell in the fuel slot: 40 FE a tick, and it ages (Java: `Cells.tick`).
    if item(items, FUEL) == NUCLEAR_CELL {
        if let Some(cell) = items[FUEL].as_mut() {
            cell.damage += 1;
            if cell.damage >= CELL_LIFE {
                items[FUEL] = Some(ItemStack::new(DEPLETED, 1));
            }
        }
        f.energy = cap.min(f.energy.saturating_add(CELL_OUTPUT));
    }
    let value = machine::fuel_value(item(items, FUEL));
    if value > 0 && f.energy as i64 + value as i64 <= cap as i64 {
        shrink(items, FUEL, 1);
        f.energy += value;
    }
    let tags = |i: &str, t: &str| in_tag(host, i, t);
    let recipe = k.recipe_for(item(items, IN_A), &tags).map(|(_, r)| r);
    let can = recipe.is_some_and(|r| count(items, IN_A) >= r.count && fits(host, items, r));
    // A run costs the same energy at any speed: a faster machine draws more per tick.
    let step = machine::step(cfg.machine_speed * f.cascade as f64 / 10.0);
    let cost = match (can, recipe) {
        (true, Some(r)) => k.cost(r, step, cfg.machine_energy),
        _ => 0,
    };
    if can && f.energy as i64 >= cost {
        f.energy -= cost as i32;
        f.progress += step;
        if f.progress >= k.ticks() * 10 {
            if let Some(r) = recipe {
                craft(items, r);
            }
            f.progress = 0;
        }
        true
    } else {
        if !can {
            f.progress = 0;
        }
        false
    }
}

/// Java: `tickReactor` (the push is queued by `tick`). Whether it works this tick.
fn tick_reactor(host: &dyn Host, items: &mut [Option<ItemStack>], f: &mut Fields, cfg: &ServerConfig) -> bool {
    let k = MachineKind::Reactor;
    let tags = |i: &str, t: &str| in_tag(host, i, t);
    let can_breed = |items: &[Option<ItemStack>]| {
        k.recipe_for(item(items, IN_A), &tags).is_some_and(|(_, r)| count(items, IN_A) >= r.count && fits(host, items, r))
    };
    if f.burn <= 0 && item(items, IN_B) == LEU && (f.energy < k.capacity() || can_breed(items)) {
        shrink(items, IN_B, 1);
        f.burn = REACTOR_FUEL_TICKS;
        f.burn_total = REACTOR_FUEL_TICKS;
    }
    let working = f.burn > 0;
    if f.burn > 0 {
        f.burn -= 1;
        f.energy = k.capacity().min(f.energy.saturating_add(REACTOR_FE_PER_TICK));
        if can_breed(items) {
            f.progress += machine::step(cfg.machine_speed);
            if f.progress >= k.ticks() * 10 {
                if let Some((_, r)) = k.recipe_for(item(items, IN_A), &tags) {
                    craft(items, r);
                }
                f.progress = 0;
            }
        } else {
            f.progress = 0;
        }
    }
    working
}

/// The screen's numbers (`data`).
fn publish(host: &dyn Host, k: MachineKind, items: &[Option<ItemStack>], f: &Fields, working: bool, cfg: &ServerConfig, e: &mut ModBlockEntity) {
    let mut v = vec![0; data::COUNT];
    v[data::PROGRESS] = f.progress.clamp(0, 0xFFFF);
    v[data::TICKS] = k.ticks();
    data::split(f.energy as i64, &mut v, data::ENERGY_LO, data::ENERGY_HI);
    v[data::BURN] = f.burn;
    v[data::BURN_TOTAL] = f.burn_total;
    v[data::CASCADE] = f.cascade;
    v[data::WORKING] = working as i32;
    let tags = |i: &str, t: &str| in_tag(host, i, t);
    let found = k.recipe_for(item(items, IN_A), &tags);
    v[data::RECIPE] = found.map_or(0, |(i, _)| i as i32 + 1);
    // The tooltip's numbers: the recipe in the input slot, the first one if it is empty (Java: `drawTips`).
    let r = found.map_or(&k.recipes()[0], |(_, r)| r);
    let speed = cfg.machine_speed * f.cascade as f64 / 10.0;
    data::split(k.draw(r, speed, cfg.machine_energy), &mut v, data::DRAW_LO, data::DRAW_HI);
    data::split(machine::energy_of(r, cfg.machine_energy), &mut v, data::RUN_LO, data::RUN_HI);
    e.data = v;
}

/// The facing a machine is placed with: towards the player (Java: `getHorizontalDirection().getOpposite()`).
pub fn placement_facing(yaw: f32) -> &'static str {
    match crate::bomb::Facing::of_yaw(yaw).name() {
        "north" => "south",
        "south" => "north",
        "west" => "east",
        _ => "west",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_as_java() {
        for d in 0..6u8 {
            let o = opposite(d);
            let (a, b) = (DIRS[d as usize], DIRS[o as usize]);
            assert_eq!([a[0] + b[0], a[1] + b[1], a[2] + b[2]], [0, 0, 0]);
        }
        assert_eq!(placement_facing(0.0), "north");
        let mut v = vec![0; data::COUNT];
        data::split(12_000_000, &mut v, data::ENERGY_LO, data::ENERGY_HI);
        assert_eq!(data::join(&v, data::ENERGY_LO, data::ENERGY_HI), 12_000_000);
    }
}
