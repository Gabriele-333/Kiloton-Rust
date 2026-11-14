//! What the sweep knows of every block state (Java: `BlockProfiles` and the state checks of
//! `BlastShape`), built once: the game's block facts come through `Level::read`
//! (hardness, blast resistance, collision, fire odds, axis, fluids, light), the tags through the
//! Host, and [`kiloton_core::blast::profile::compute`] makes the profile.

use horizon_client::mod_api::Host;
use kiloton_core::blast::profile::{self, StateInput};
use kiloton_core::blast::shape::{StateInfo, NAMED};
use std::collections::HashMap;
use std::sync::RwLock;

/// One state as the injection copied it from the game's tables.
#[derive(Clone, Debug)]
pub struct Raw {
    pub name: String,
    pub hardness: f32,
    pub resistance: f32,
    pub collides: bool,
    pub ignite_odds: i32,
    pub axis: u8,
    pub full_collision: bool,
    pub block_entity: bool,
    pub waterlogged: bool,
    /// 0 none, 1 water, 2 lava.
    pub fluid: u8,
    pub fluid_source: bool,
    pub emission: u8,
    pub legacy_solid: bool,
    pub leaves_class: bool,
    pub liquid_class: bool,
    /// `canOcclude`: blocks the flash where the line runs through its shape.
    pub can_occlude: bool,
    /// Its break sound is metal's (the Java edition's `SoundType.METAL` test for shelter walls).
    pub metal: bool,
    /// A door or a trapdoor, and its `open` property (0 none, 1 shut, 2 open).
    pub door: bool,
    pub open: u8,
    /// Its outline as block-local boxes, empty for a whole cube.
    pub boxes: Vec<[f64; 6]>,
    pub replaceable: bool,
    /// Has a lower and an upper half (tall grass, doors).
    pub tall: bool,
}

/// The tables the sweep reads, by state id.
pub struct Tables {
    pub infos: Vec<StateInfo>,
    /// Counts on `MOTION_BLOCKING_NO_LEAVES`: blocks motion or holds a fluid, and isn't leaves.
    pub motion: Vec<bool>,
    /// Air, cave air, void air.
    pub air: Vec<bool>,
    /// `canBeReplaced`, and tall plants and doors (`DOUBLE_BLOCK_HALF`): for the fallout film.
    pub replaceable: Vec<bool>,
    pub tall: Vec<bool>,
    /// The states of the fallout film, of the irradiated leaves and of spruce leaves (the conifers).
    pub dust_states: Vec<u32>,
    pub dead_leaves: Vec<u32>,
    pub spruce_leaves: Vec<u32>,
    /// What the flash, the wave under cover, the prompt radiation and shelters see of each state.
    pub cover: Vec<kiloton_core::blast::cover::CoverInfo>,
    /// The fluid each state holds, for the ticks at the crater's edge: 0 none, 1 water, 2 flowing
    /// water, 3 lava, 4 flowing lava (a waterlogged block holds a water source).
    pub fluid_kind: Vec<u8>,
    /// The default states of the blocks the sweep writes by name.
    pub named: HashMap<&'static str, u32>,
    pub fire: u32,
    pub soul_fire: u32,
    pub air_id: u32,
}

/// The raw states, once the first level tick has copied them.
pub static RAW: RwLock<Option<Vec<Raw>>> = RwLock::new(None);
/// The tables, once the Host has given the tags.
pub static TABLES: RwLock<Option<Tables>> = RwLock::new(None);

fn tag(host: &dyn Host, name: &str, tag: &str) -> bool {
    host.block_name_in_tag(name, tag).unwrap_or(false)
}

/// Java: `BlockProfiles.terrainSurface`. Horizon has no convention (`c:`) tags: vanilla's ore tags,
/// Kiloton's ores and gravel stand in for `c:ores` and `c:gravels`.
fn terrain(host: &dyn Host, n: &str) -> u8 {
    let is = |t: &str| tag(host, n, t);
    let named = |names: &[&str]| names.contains(&n);
    if named(&[
        "minecraft:grass_block",
        "minecraft:podzol",
        "minecraft:mycelium",
        "minecraft:dirt_path",
        "minecraft:farmland",
        "minecraft:moss_block",
    ]) || is("minecraft:nylium")
    {
        return profile::S_GRASS;
    }
    if is("minecraft:dirt") || named(&["minecraft:clay", "minecraft:mud", "minecraft:soul_soil", "minecraft:soul_sand"]) {
        return profile::S_SOIL;
    }
    if is("minecraft:sand") || named(&["minecraft:sandstone", "minecraft:red_sandstone"]) {
        return profile::S_SAND;
    }
    if is("c:gravels") || n == "minecraft:gravel" {
        return profile::S_GRAVEL;
    }
    let ores = is("c:ores")
        || [
            "minecraft:coal_ores",
            "minecraft:iron_ores",
            "minecraft:copper_ores",
            "minecraft:gold_ores",
            "minecraft:redstone_ores",
            "minecraft:lapis_ores",
            "minecraft:diamond_ores",
            "minecraft:emerald_ores",
        ]
        .iter()
        .any(|t| is(t))
        || named(&[
            "minecraft:nether_quartz_ore",
            "minecraft:ancient_debris",
            "kiloton:uranium_ore",
            "kiloton:deepslate_uranium_ore",
            "kiloton:lithium_ore",
            "kiloton:deepslate_lithium_ore",
        ]);
    if is("minecraft:base_stone_overworld")
        || is("minecraft:base_stone_nether")
        || ores
        || is("minecraft:terracotta")
        || named(&[
            "minecraft:calcite",
            "minecraft:dripstone_block",
            "minecraft:end_stone",
            "minecraft:magma_block",
            "minecraft:smooth_basalt",
            "minecraft:obsidian",
        ])
    {
        return profile::S_STONE;
    }
    if named(&["minecraft:packed_mud", "minecraft:suspicious_sand", "minecraft:suspicious_gravel"]) {
        return profile::S_OTHER;
    }
    profile::S_NONE
}

/// Java: `c:glass_blocks` and `c:glass_panes`, which Horizon doesn't have: vanilla's glass by name.
fn glass(host: &dyn Host, n: &str) -> bool {
    tag(host, n, "c:glass_blocks")
        || tag(host, n, "c:glass_panes")
        || (n.starts_with("minecraft:") && (n.ends_with("glass") || n.ends_with("glass_pane")))
}

const TIMBER_TAGS: &[&str] = &[
    "minecraft:planks",
    "minecraft:wool",
    "minecraft:beds",
    "minecraft:wooden_fences",
    "minecraft:wooden_doors",
    "minecraft:wooden_trapdoors",
    "minecraft:wooden_slabs",
    "minecraft:wooden_stairs",
    "minecraft:fence_gates",
];

fn cracked(n: &str) -> Option<&'static str> {
    Some(match n {
        "minecraft:stone_bricks" => "minecraft:cracked_stone_bricks",
        "minecraft:deepslate_bricks" => "minecraft:cracked_deepslate_bricks",
        "minecraft:deepslate_tiles" => "minecraft:cracked_deepslate_tiles",
        "minecraft:polished_blackstone_bricks" => "minecraft:cracked_polished_blackstone_bricks",
        "minecraft:nether_bricks" => "minecraft:cracked_nether_bricks",
        _ => return None,
    })
}

/// Builds the tables from the raw states, asking the Host for each block's tags once.
pub fn build(host: &dyn Host, raw: &[Raw]) -> Tables {
    struct Block {
        leaves: bool,
        log: bool,
        glass: bool,
        ice: bool,
        timber: bool,
        axe: bool,
        terrain: u8,
        soul_base: bool,
    }
    let mut blocks: HashMap<&str, Block> = HashMap::new();
    let mut infos = Vec::with_capacity(raw.len());
    let mut fluid_kind = Vec::with_capacity(raw.len());
    let mut cover = Vec::with_capacity(raw.len());
    let mut motion = Vec::with_capacity(raw.len());
    let mut air = Vec::with_capacity(raw.len());
    for r in raw {
        let n = r.name.as_str();
        let b = blocks.entry(n).or_insert_with(|| Block {
            leaves: tag(host, n, "minecraft:leaves"),
            log: tag(host, n, "minecraft:logs"),
            glass: glass(host, n),
            ice: tag(host, n, "minecraft:ice") || n == "minecraft:snow_block" || n == "minecraft:powder_snow",
            timber: TIMBER_TAGS.iter().any(|t| tag(host, n, t))
                || n == "minecraft:hay_block"
                || n == "minecraft:bookshelf",
            axe: tag(host, n, "minecraft:mineable/axe"),
            terrain: terrain(host, n),
            soul_base: tag(host, n, "minecraft:soul_fire_base_blocks"),
        });
        let is_air = matches!(n, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air");
        let input = StateInput {
            air: is_air,
            hardness: r.hardness,
            explosion_resistance: r.resistance,
            reinforced_deepslate: n == "minecraft:reinforced_deepslate",
            nuke: n == "kiloton:nuclear_bomb" || n == "kiloton:thermonuclear_bomb",
            liquid: r.liquid_class,
            leaves: b.leaves,
            log: b.log,
            glass: b.glass,
            snow_layer: n == "minecraft:snow",
            ice: b.ice,
            no_collision: !r.collides,
            terrain: b.terrain,
            timber: b.timber,
            mineable_axe: b.axe,
            flammable: r.ignite_odds > 0,
            emitter: r.emission > 0,
        };
        let wet = r.fluid != 0 || r.waterlogged;
        fluid_kind.push(match (r.fluid, r.fluid_source || r.waterlogged) {
            (1, true) | (0, true) if wet => 1,
            (1, false) => 2,
            (2, true) => 3,
            (2, false) => 4,
            _ => 0,
        });
        // A waterlogged block holds a water source.
        let source = match (r.fluid, r.fluid_source || r.waterlogged) {
            (1, true) => Some("minecraft:water"),
            (2, true) => Some("minecraft:lava"),
            (0, true) => Some("minecraft:water"),
            _ => None,
        };
        let source = if wet { source } else { None };
        {
            use kiloton_core::blast::cover::{self, CoverInfo};
            let p = profile::compute(&input);
            let is_air = p.category == profile::AIR;
            let seals = if is_air {
                false
            } else if r.fluid != 0 {
                r.fluid_source
            } else if r.door {
                r.open == 1
            } else {
                r.full_collision
            };
            let strength = if p.threshold >= f32::MAX / 2.0 { cover::UNBREAKABLE * 10.0 } else { p.threshold as f64 };
            cover.push(CoverInfo {
                air: is_air,
                seals,
                halvings: cover::halvings(p.category, r.metal),
                strength,
                occludes: r.can_occlude,
                leaves: p.category == profile::LEAVES,
                boxes: r.boxes.clone(),
            });
        }
        infos.push(StateInfo {
            profile: profile::compute(&input),
            upright: r.axis == 0 || r.axis == 2,
            full_block: !r.block_entity && r.full_collision,
            wet,
            source,
            melts: n == "minecraft:ice" || n == "minecraft:frosted_ice",
            snow_layer: n == "minecraft:snow",
            farmland_or_path: n == "minecraft:farmland" || n == "minecraft:dirt_path",
            mycelium_or_podzol: n == "minecraft:mycelium" || n == "minecraft:podzol",
            nylium: n == "minecraft:crimson_nylium" || n == "minecraft:warped_nylium",
            soul_base: b.soul_base,
            cracked: cracked(n),
            nuke: input.nuke,
        });
        // Java's MOTION_BLOCKING_NO_LEAVES: (blocksMotion || fluid) && !LeavesBlock; blocksMotion is
        // isSolid without cobwebs and bamboo saplings.
        let blocks_motion = r.legacy_solid && n != "minecraft:cobweb" && n != "minecraft:bamboo_sapling";
        motion.push((blocks_motion || wet) && !r.leaves_class);
        air.push(is_air);
    }
    let mut named = HashMap::new();
    for &n in NAMED {
        if let Some(id) = host.default_state(n) {
            named.insert(n, id.0);
        }
    }
    let id = |n: &str| host.default_state(n).map_or(0, |s| s.0);
    Tables {
        infos,
        motion,
        air,
        fluid_kind,
        replaceable: raw.iter().map(|r| r.replaceable).collect(),
        tall: raw.iter().map(|r| r.tall).collect(),
        dust_states: raw.iter().enumerate().filter(|(_, r)| r.name == "kiloton:fallout_dust").map(|(i, _)| i as u32).collect(),
        dead_leaves: raw.iter().enumerate().filter(|(_, r)| r.name == "kiloton:irradiated_leaves").map(|(i, _)| i as u32).collect(),
        spruce_leaves: raw.iter().enumerate().filter(|(_, r)| r.name == "minecraft:spruce_leaves").map(|(i, _)| i as u32).collect(),
        cover,
        named,
        fire: id("minecraft:fire"),
        soul_fire: id("minecraft:soul_fire"),
        air_id: id("minecraft:air"),
    }
}
