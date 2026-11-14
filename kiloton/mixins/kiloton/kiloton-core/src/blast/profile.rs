//! How each block state reacts to the blast (Java: `blast.BlockProfiles`), computed once per state.
//!
//! Thresholds are peak overpressures in psi, from the classic damage criteria: windows go at
//! ~0.5-1 psi, leaves and loose things at 1-3, wood-frame buildings at ~5, brick and stone at
//! ~10-20, reinforced concrete above that. Natural ground ("terrain") is never blown away by
//! pressure: only the crater and the fireball remove it, and it shields everything below it.

pub const AIR: u8 = 0;
pub const INDESTRUCTIBLE: u8 = 1;
pub const TERRAIN: u8 = 2;
pub const FLUID: u8 = 3;
pub const PLANT: u8 = 4;
pub const GLASS: u8 = 5;
pub const LIGHT: u8 = 6;
pub const LEAVES: u8 = 7;
pub const WOOD: u8 = 8;
pub const LOG: u8 = 9;
pub const STRUCTURE: u8 = 10;
pub const ICE: u8 = 11;

/// Surface kinds of terrain, used by the fireball (fusion) and the thermal pulse (scorching).
pub const S_NONE: u8 = 0;
pub const S_GRASS: u8 = 1;
pub const S_SOIL: u8 = 2;
pub const S_SAND: u8 = 3;
pub const S_GRAVEL: u8 = 4;
pub const S_STONE: u8 = 5;
pub const S_OTHER: u8 = 6;

/// What the game knows of a state, as the profile needs it (filled from the game's block facts
/// and the server's tags).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateInput {
    pub air: bool,
    /// Destroy time (-1: unbreakable).
    pub hardness: f32,
    pub explosion_resistance: f32,
    /// Reinforced deepslate, which Java's rule names besides the unbreakable blocks.
    pub reinforced_deepslate: bool,
    /// One of Kiloton's bombs (`NukeBlock`): hard to blow away, but no terrain.
    pub nuke: bool,
    /// Water or lava itself (`LiquidBlock`).
    pub liquid: bool,
    pub leaves: bool,
    pub log: bool,
    /// `c:glass_blocks` or `c:glass_panes`.
    pub glass: bool,
    /// A snow layer (`minecraft:snow`).
    pub snow_layer: bool,
    /// Ice, a snow block or powder snow.
    pub ice: bool,
    /// Its collision shape is always empty.
    pub no_collision: bool,
    /// Its kind of natural ground, `S_NONE` if it isn't any (Java: `terrainSurface`).
    pub terrain: u8,
    /// Planks, wool, beds, wooden fences, doors, trapdoors, slabs and stairs, fence gates, a hay
    /// bale, a bookshelf.
    pub timber: bool,
    /// `#minecraft:mineable/axe`.
    pub mineable_axe: bool,
    pub flammable: bool,
    pub emitter: bool,
}

/// A state's profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    pub category: u8,
    pub surface: u8,
    pub threshold: f32,
    pub flammable: bool,
    pub emitter: bool,
}

impl Profile {
    pub const AIR: Profile = Profile { category: AIR, surface: S_NONE, threshold: 0.0, flammable: false, emitter: false };
}

/// Java: `BlockProfiles.compute`.
pub fn compute(s: &StateInput) -> Profile {
    let mut surface = S_NONE;
    let (category, threshold) = if s.air {
        (AIR, 0.0)
    } else if s.hardness < 0.0 || s.explosion_resistance >= 3_000_000.0 || s.reinforced_deepslate {
        (INDESTRUCTIBLE, f32::MAX)
    } else if s.nuke {
        (STRUCTURE, 60.0)
    } else if s.liquid {
        (FLUID, f32::MAX)
    } else if s.leaves {
        (LEAVES, 2.0)
    } else if s.log {
        // Forests: most trees are blown down around 5 psi; what stands above that is stripped bare.
        (LOG, 5.0)
    } else if s.glass {
        (GLASS, 0.8)
    } else if s.snow_layer {
        (LIGHT, 1.0)
    } else if s.ice {
        (ICE, 6.0)
    } else if s.no_collision {
        if s.hardness <= 0.2 {
            // Grass and flowers bend in a strong wind; flattened from ~3 psi or burnt by the flash.
            (PLANT, 3.0)
        } else {
            (LIGHT, 1.5)
        }
    } else if s.terrain != S_NONE {
        surface = s.terrain;
        (TERRAIN, f32::MAX)
    } else if s.timber || (s.mineable_axe && s.hardness <= 3.0) {
        (WOOD, 4.5)
    } else if s.hardness <= 0.6 && s.explosion_resistance <= 1.0 {
        // Soft odds and ends: carpets, flower pots, cake, scaffolding...
        (LIGHT, 1.5)
    } else {
        let strength = s.explosion_resistance.max(s.hardness * 3.0);
        (STRUCTURE, if strength >= 100.0 { 2000.0 } else { 3.0 + 2.4 * strength.min(25.0) })
    };
    Profile { category, surface, threshold, flammable: s.flammable, emitter: s.emitter }
}

/// How much of the blast a surviving block lets through to the blocks under it.
pub fn shield_factor(category: u8) -> f64 {
    match category {
        GLASS => 0.9,
        LEAVES => 0.85,
        LOG => 0.75,
        WOOD => 0.5,
        ICE => 0.4,
        STRUCTURE => 0.3,
        _ => 1.0,
    }
}

/// Whether a block stops the thermal pulse from reaching what is under it.
pub fn blocks_heat(category: u8) -> bool {
    matches!(category, LEAVES | LOG | WOOD | STRUCTURE | ICE | TERRAIN | INDESTRUCTIBLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_follow_java() {
        let stone = StateInput { hardness: 1.5, explosion_resistance: 6.0, terrain: S_STONE, ..Default::default() };
        assert_eq!(compute(&stone).category, TERRAIN);
        let bricks = StateInput { hardness: 2.0, explosion_resistance: 6.0, ..Default::default() };
        let p = compute(&bricks);
        assert_eq!(p.category, STRUCTURE);
        assert_eq!(p.threshold, 3.0 + 2.4 * 6.0); // max(6, 2 * 3) = 6
        let obsidian_like = StateInput { hardness: 50.0, explosion_resistance: 1200.0, ..Default::default() };
        assert_eq!(compute(&obsidian_like).threshold, 2000.0);
        let grass = StateInput { hardness: 0.0, no_collision: true, ..Default::default() };
        assert_eq!(compute(&grass).category, PLANT);
        let bedrock = StateInput { hardness: -1.0, explosion_resistance: 3_600_000.0, ..Default::default() };
        assert_eq!(compute(&bedrock).category, INDESTRUCTIBLE);
        let planks = StateInput { hardness: 2.0, explosion_resistance: 3.0, timber: true, ..Default::default() };
        assert_eq!(compute(&planks).category, WOOD);
    }
}
