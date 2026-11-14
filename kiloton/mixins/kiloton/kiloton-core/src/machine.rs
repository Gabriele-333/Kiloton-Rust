//! The machines of the uranium chain and the exchange column (Java: `MachineKind`, `Cells`, `CreativeSourceBlockEntity`,
//! `CableTier`): their slots on screen, what they make, what it costs and how the numbers are written. Items are ids
//! (`kiloton:yellowcake`); an input that Java reads from a `c:` tag also lists Kiloton's own items, for a game where
//! the tag isn't known.
//!
//! ```text
//!   uranium ore -> raw uranium --MILL--> yellowcake (U3O8, 0.7% U-235)
//!   yellowcake --CENTRIFUGE--> LEU 4% --> HEU 20% --> weapons grade 90%   (each pass: 2 in, 1 up + 1 depleted)
//!   depleted uranium + LEU fuel --REACTOR--> plutonium-239                (and power for the others)
//!   weapons grade x4 / plutonium x2 / depleted x4 --FOUNDRY--> uranium core / plutonium core / tamper
//! ```
//!
//! Energy follows one scale, 1 FE = 1 kJ (1 FE/t = 20 kW). A recipe costs a fixed amount of energy whatever the speed:
//! a faster machine draws more per tick and finishes sooner.

use crate::format::fixed;

pub const IN_A: usize = 0;
pub const IN_B: usize = 1;
pub const FUEL: usize = 2;
pub const OUT_A: usize = 3;
pub const OUT_B: usize = 4;
pub const SLOTS: usize = 5;

/// FE per separative work unit: 50 kWh = 180 MJ.
pub const FE_PER_SWU: i32 = 180_000;
/// The reactor: power it makes while a fuel element burns (600 MW, the BN-600 fast breeder), FE a tick.
pub const REACTOR_FE_PER_TICK: i32 = 30_000;
/// How long one fuel element lasts, ticks (it breeds one plutonium in that time).
pub const REACTOR_FUEL_TICKS: i32 = 6_000;
/// The most the reactor pushes out in a tick.
pub const REACTOR_PUSH: i32 = 60_000;

/// A nuclear cell's power while it lives, FE a tick (Java: `Cells.OUTPUT`).
pub const CELL_OUTPUT: i32 = 40;
/// Its life in ticks, two hours (`Cells.LIFE`).
pub const CELL_LIFE: u32 = 144_000;

/// The creative source: always full, gives what its neighbours take (`CreativeSourceBlockEntity.CAPACITY`).
pub const CREATIVE_CAPACITY: i32 = 2_000_000_000;

pub const RAW_URANIUM: &str = "kiloton:raw_uranium";
pub const YELLOWCAKE: &str = "kiloton:yellowcake";
pub const LEU: &str = "kiloton:low_enriched_uranium";
pub const HEU: &str = "kiloton:highly_enriched_uranium";
pub const WEAPONS_GRADE: &str = "kiloton:weapons_grade_uranium";
pub const DEPLETED: &str = "kiloton:depleted_uranium";
pub const PLUTONIUM: &str = "kiloton:plutonium";
pub const URANIUM_CORE: &str = "kiloton:uranium_core";
pub const PLUTONIUM_CORE: &str = "kiloton:plutonium_core";
pub const URANIUM_TAMPER: &str = "kiloton:uranium_tamper";
pub const RAW_LITHIUM: &str = "kiloton:raw_lithium";
pub const LITHIUM: &str = "kiloton:lithium";
pub const LITHIUM_6: &str = "kiloton:lithium_6";
pub const HEAVY_WATER: &str = "kiloton:heavy_water";
pub const NUCLEAR_CELL: &str = "kiloton:nuclear_cell";

/// What a recipe takes: one item, or the items of a tag (with Kiloton's own members for when the tag is unknown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Item(&'static str),
    Tag(&'static str, &'static [&'static str]),
}

impl Input {
    /// Whether `item` is this input; `in_tag(item, tag)` answers for the game's tags.
    pub fn matches(&self, item: &str, in_tag: &dyn Fn(&str, &str) -> bool) -> bool {
        match *self {
            Input::Item(id) => item == id,
            Input::Tag(tag, own) => own.contains(&item) || in_tag(item, tag),
        }
    }
}

/// One recipe (Java: `MachineKind.Recipe`); `energy` is FE for one run at the realistic cost (0 for the reactor,
/// which makes power instead).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub input: Input,
    pub count: u32,
    pub out_a: (&'static str, u32),
    pub out_b: Option<(&'static str, u32)>,
    /// The translation key of what goes in (the arrow's tooltip).
    pub in_key: &'static str,
    pub energy: i32,
}

/// Java: `Math.round(swu * FE_PER_SWU)`.
const fn swu(x10: i64) -> i32 {
    // x10 is the separative work times 10 (32.0 -> 320), so the product is exact: 32.0, 23.3 and 18.5 SWU.
    ((x10 * FE_PER_SWU as i64 + 5) / 10) as i32
}

const URANIUM_ORES: Input = Input::Tag("c:ores/uranium", &["kiloton:uranium_ore", "kiloton:deepslate_uranium_ore"]);
const LITHIUM_ORES: Input = Input::Tag("c:ores/lithium", &["kiloton:lithium_ore", "kiloton:deepslate_lithium_ore"]);

const MILL: [Recipe; 4] = [
    Recipe { input: Input::Tag("c:raw_materials/uranium", &[RAW_URANIUM]), count: 1, out_a: (YELLOWCAKE, 2), out_b: None, in_key: "item.kiloton.raw_uranium", energy: 3_200 },
    Recipe { input: URANIUM_ORES, count: 1, out_a: (YELLOWCAKE, 4), out_b: None, in_key: "block.kiloton.uranium_ore", energy: 3_200 },
    Recipe { input: Input::Tag("c:raw_materials/lithium", &[RAW_LITHIUM]), count: 1, out_a: (LITHIUM, 2), out_b: None, in_key: "item.kiloton.raw_lithium", energy: 3_200 },
    Recipe { input: LITHIUM_ORES, count: 1, out_a: (LITHIUM, 4), out_b: None, in_key: "block.kiloton.lithium_ore", energy: 3_200 },
];
const CENTRIFUGE: [Recipe; 3] = [
    Recipe { input: Input::Item(YELLOWCAKE), count: 2, out_a: (LEU, 1), out_b: Some((DEPLETED, 1)), in_key: "item.kiloton.yellowcake", energy: swu(320) },
    Recipe { input: Input::Item(LEU), count: 2, out_a: (HEU, 1), out_b: Some((DEPLETED, 1)), in_key: "item.kiloton.low_enriched_uranium", energy: swu(233) },
    Recipe { input: Input::Item(HEU), count: 2, out_a: (WEAPONS_GRADE, 1), out_b: Some((DEPLETED, 1)), in_key: "item.kiloton.highly_enriched_uranium", energy: swu(185) },
];
const REACTOR: [Recipe; 1] =
    [Recipe { input: Input::Item(DEPLETED), count: 4, out_a: (PLUTONIUM, 1), out_b: None, in_key: "item.kiloton.depleted_uranium", energy: 0 }];
const FOUNDRY: [Recipe; 3] = [
    Recipe { input: Input::Item(WEAPONS_GRADE), count: 4, out_a: (URANIUM_CORE, 1), out_b: None, in_key: "item.kiloton.weapons_grade_uranium", energy: 9_000 },
    Recipe { input: Input::Item(PLUTONIUM), count: 2, out_a: (PLUTONIUM_CORE, 1), out_b: None, in_key: "item.kiloton.plutonium", energy: 9_000 },
    Recipe { input: Input::Item(DEPLETED), count: 4, out_a: (URANIUM_TAMPER, 1), out_b: None, in_key: "item.kiloton.depleted_uranium", energy: 9_000 },
];
// Natural lithium is 7.5% lithium-6: eight in, one out at weapons enrichment. A bucket of water holds about 150 g of
// heavy water, and separating it costs some 12 MJ a gram: 1.8 million FE.
const EXCHANGE: [Recipe; 2] = [
    Recipe { input: Input::Item(LITHIUM), count: 8, out_a: (LITHIUM_6, 1), out_b: None, in_key: "item.kiloton.lithium", energy: 600_000 },
    Recipe { input: Input::Item("minecraft:water_bucket"), count: 1, out_a: (HEAVY_WATER, 1), out_b: Some(("minecraft:bucket", 1)), in_key: "item.minecraft.water_bucket", energy: 1_800_000 },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MachineKind {
    Mill,
    Centrifuge,
    Reactor,
    Foundry,
    /// Isotope separation by chemical exchange: lithium-6 (COLEX) and heavy water (Girdler sulfide).
    Exchange,
}

impl MachineKind {
    pub const ALL: [MachineKind; 5] = [Self::Mill, Self::Centrifuge, Self::Reactor, Self::Foundry, Self::Exchange];

    /// The block id without the namespace.
    pub fn id(self) -> &'static str {
        match self {
            Self::Mill => "uranium_mill",
            Self::Centrifuge => "gas_centrifuge",
            Self::Reactor => "breeder_reactor",
            Self::Foundry => "core_foundry",
            Self::Exchange => "exchange_column",
        }
    }

    /// The machine of a block (`kiloton:uranium_mill`).
    pub fn of_block(block: &str) -> Option<Self> {
        let path = block.strip_prefix("kiloton:")?;
        Self::ALL.into_iter().find(|k| k.id() == path)
    }

    /// Ticks of one recipe at speed 1.
    pub fn ticks(self) -> i32 {
        match self {
            Self::Mill => 160,
            Self::Centrifuge => 400,
            Self::Reactor => 6_000,
            Self::Foundry => 300,
            Self::Exchange => 600,
        }
    }

    /// The energy buffer, FE.
    pub fn capacity(self) -> i32 {
        match self {
            Self::Mill | Self::Foundry => 200_000,
            Self::Centrifuge => 12_000_000,
            Self::Reactor => 1_000_000,
            Self::Exchange => 4_000_000,
        }
    }

    /// The most it takes in a tick (the reactor takes nothing: it gives).
    pub fn max_receive(self) -> i32 {
        match self {
            Self::Mill | Self::Foundry => 2_000,
            Self::Centrifuge => 100_000,
            Self::Reactor => 0,
            Self::Exchange => 20_000,
        }
    }

    /// The most it gives in a tick (Java: `MachineEnergy`, the reactor's push).
    pub fn max_extract(self) -> i32 {
        if self == Self::Reactor { REACTOR_PUSH } else { 0 }
    }

    /// Item position of a slot on screen (GUI pixels in the 176x166 panel), or None if this machine has no such slot.
    pub fn slot_pos(self, slot: usize) -> Option<[i32; 2]> {
        let t: [Option<[i32; 2]>; SLOTS] = match self {
            Self::Mill | Self::Foundry => [Some([56, 35]), None, Some([26, 53]), Some([116, 35]), None],
            Self::Centrifuge | Self::Exchange => [Some([56, 35]), None, Some([26, 53]), Some([116, 35]), Some([140, 35])],
            Self::Reactor => [Some([56, 24]), Some([56, 48]), None, Some([116, 35]), None],
        };
        t.get(slot).copied().flatten()
    }

    pub fn has_slot(self, slot: usize) -> bool {
        self.slot_pos(slot).is_some()
    }

    pub fn recipes(self) -> &'static [Recipe] {
        match self {
            Self::Mill => &MILL,
            Self::Centrifuge => &CENTRIFUGE,
            Self::Reactor => &REACTOR,
            Self::Foundry => &FOUNDRY,
            Self::Exchange => &EXCHANGE,
        }
    }

    /// The recipe of what is in the input slot, and its index among this machine's.
    pub fn recipe_for(self, item: &str, in_tag: &dyn Fn(&str, &str) -> bool) -> Option<(usize, &'static Recipe)> {
        if item.is_empty() {
            return None;
        }
        self.recipes().iter().enumerate().find(|(_, r)| r.input.matches(item, in_tag))
    }

    /// FE taken in a tick that moves the recipe by `step` tenths of a tick (10 at speed 1). Java: `cost`.
    pub fn cost(self, recipe: &Recipe, step: i32, machine_energy: f64) -> i64 {
        (energy_of(recipe, machine_energy) as f64 * step as f64 / (self.ticks() as f64 * 10.0)).ceil() as i64
    }

    /// Power drawn while the recipe runs at this speed, FE/t. Java: `draw`.
    pub fn draw(self, recipe: &Recipe, speed: f64, machine_energy: f64) -> i64 {
        self.cost(recipe, step(speed), machine_energy)
    }

    /// Whether the item may go into this slot of this machine (by hand, hopper or pipe). Java: `accepts`.
    pub fn accepts(self, slot: usize, item: &str, in_tag: &dyn Fn(&str, &str) -> bool) -> bool {
        if !self.has_slot(slot) {
            return false;
        }
        match slot {
            IN_A => self.recipe_for(item, in_tag).is_some(),
            IN_B => self == Self::Reactor && item == LEU,
            FUEL => fuel_value(item) > 0 || item == NUCLEAR_CELL,
            _ => false,
        }
    }
}

/// Energy of one run of the recipe, FE, with the server's energy setting. Java: `Math.round(energy * MACHINE_ENERGY)`.
pub fn energy_of(recipe: &Recipe, machine_energy: f64) -> i64 {
    java_round(recipe.energy as f64 * machine_energy)
}

/// Tenths of a tick a recipe moves in one tick at this speed: `(int) Math.max(1, Math.round(speed * 10))`.
pub fn step(speed: f64) -> i32 {
    java_round(speed * 10.0).max(1) as i32
}

/// Java's `Math.round(double)`: the closest long, halves up.
pub fn java_round(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    (x + 0.5).floor() as i64
}

/// Fuel a machine burns into energy, FE an item, 0 if it isn't one. Java: `fuelValue`.
pub fn fuel_value(item: &str) -> i32 {
    match item {
        "minecraft:coal" | "minecraft:charcoal" => 16_000,
        "minecraft:coal_block" => 144_000,
        "minecraft:blaze_rod" => 24_000,
        "minecraft:dried_kelp_block" => 40_000,
        _ => 0,
    }
}

/// `MachineKind.fe`: 14,400 -> "14.4k", 5,760,000 -> "5.76M".
pub fn fe(v: i64) -> String {
    if v < 10_000 {
        return grouped(v);
    }
    if v < 1_000_000 {
        return trim(v as f64 / 1_000.0) + "k";
    }
    if v < 1_000_000_000 {
        return trim(v as f64 / 1_000_000.0) + "M";
    }
    trim(v as f64 / 1_000_000_000.0) + "G"
}

fn trim(v: f64) -> String {
    let s = fixed(v, if v < 10.0 { 2 } else if v < 100.0 { 1 } else { 0 });
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s }
}

/// `String.format(Locale.ROOT, "%,d", v)`.
pub fn grouped(v: i64) -> String {
    let digits = v.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if v < 0 { format!("-{out}") } else { out }
}

/// `BlockPos.asLong`: x in 26 bits, z in 26, y in 12.
pub fn pos_as_long(p: [i32; 3]) -> i64 {
    ((p[0] as i64 & 0x3FF_FFFF) << 38) | ((p[2] as i64 & 0x3FF_FFFF) << 12) | (p[1] as i64 & 0xFFF)
}

/// The grades of energy cable (Java: `CableTier`): FE a tick through one connection, and what a piece holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CableTier {
    Copper,
    Gold,
    Hardened,
    Superconducting,
}

impl CableTier {
    pub const ALL: [CableTier; 4] = [Self::Copper, Self::Gold, Self::Hardened, Self::Superconducting];

    pub fn of_block(block: &str) -> Option<Self> {
        Some(match block {
            "kiloton:copper_cable" => Self::Copper,
            "kiloton:gold_cable" => Self::Gold,
            "kiloton:hardened_cable" => Self::Hardened,
            "kiloton:superconducting_cable" => Self::Superconducting,
            _ => return None,
        })
    }

    pub fn rate(self) -> i32 {
        match self {
            Self::Copper => 1_000,
            Self::Gold => 8_000,
            Self::Hardened => 64_000,
            Self::Superconducting => 1_000_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_tags(_: &str, _: &str) -> bool {
        false
    }

    #[test]
    fn energy_as_java() {
        // swu(32.0), swu(23.3), swu(18.5) as Math.round(x * 180000) gives them.
        assert_eq!(CENTRIFUGE.map(|r| r.energy), [5_760_000, 4_194_000, 3_330_000]);
        assert_eq!(java_round(23.3 * 180_000.0), 4_194_000);
        let c = MachineKind::Centrifuge;
        // 5.76 M over 4000 tenths: 1440 a tenth, 14,400 a tick at speed 1; with two neighbours (x2.0) 28,800.
        assert_eq!(c.cost(&CENTRIFUGE[0], 10, 1.0), 14_400);
        assert_eq!(c.draw(&CENTRIFUGE[0], 2.0, 1.0), 28_800);
        assert_eq!(MachineKind::Mill.cost(&MILL[0], 10, 1.0), 20);
        // The ceiling: 3200 * 7 / 1600 = 14.
        assert_eq!(MachineKind::Mill.cost(&MILL[0], 7, 1.0), 14);
        assert_eq!(MachineKind::Mill.cost(&MILL[0], 10, 0.001), 1);
        assert_eq!(step(0.1), 1);
        assert_eq!(step(1.25), 13);
    }

    #[test]
    fn numbers_as_java() {
        assert_eq!(fe(9_999), "9,999");
        assert_eq!(fe(14_400), "14.4k");
        assert_eq!(fe(5_760_000), "5.76M");
        assert_eq!(fe(30_000), "30k");
        assert_eq!(fe(200_000), "200k");
        assert_eq!(fe(1_800_000), "1.8M");
        assert_eq!(fe(12_000_000), "12M");
        assert_eq!(fe(2_000_000_000), "2G");
        assert_eq!(grouped(2_000_000_000), "2,000,000,000");
        assert_eq!(pos_as_long([1, 2, 3]), (1i64 << 38) | (3 << 12) | 2);
        assert_eq!(pos_as_long([-1, -1, -1]).rem_euclid(20), (-1i64).rem_euclid(20));
    }

    #[test]
    fn slots_and_recipes() {
        let m = MachineKind::Mill;
        assert!(m.accepts(IN_A, "kiloton:raw_uranium", &no_tags));
        assert!(m.accepts(IN_A, "kiloton:deepslate_lithium_ore", &no_tags));
        assert!(m.accepts(IN_A, "othermod:uranium_ore", &|i, t| i == "othermod:uranium_ore" && t == "c:ores/uranium"));
        assert!(!m.accepts(IN_B, LEU, &no_tags));
        assert!(m.accepts(FUEL, "minecraft:coal", &no_tags) && m.accepts(FUEL, NUCLEAR_CELL, &no_tags));
        assert!(!m.accepts(OUT_A, YELLOWCAKE, &no_tags));
        let r = MachineKind::Reactor;
        assert!(r.accepts(IN_B, LEU, &no_tags) && !r.accepts(FUEL, "minecraft:coal", &no_tags));
        assert_eq!(r.recipe_for(DEPLETED, &no_tags).map(|(i, x)| (i, x.count)), Some((0, 4)));
        assert_eq!(MachineKind::Foundry.recipe_for(DEPLETED, &no_tags).map(|(i, _)| i), Some(2));
        assert_eq!(MachineKind::of_block("kiloton:gas_centrifuge"), Some(MachineKind::Centrifuge));
        assert_eq!(MachineKind::of_block("kiloton:nuclear_bomb"), None);
        assert_eq!(MachineKind::Exchange.recipes()[1].out_b, Some(("minecraft:bucket", 1)));
    }
}
