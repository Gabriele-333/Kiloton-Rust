// Kiloton's glue in the game: `horizon_client::game::kiloton`, a horizon-mixins support module
// (`[[module]]` "glue" in horizon-mixin.toml). It sits inside `game`, so it reaches what the Host API
// doesn't give and the [api] crate can't name: the game's block facts and blocks, its level scheduler,
// its explosions and the energy between blocks. The mod installs a `Level` when the game starts
// (api's `hooks()`); the `level` injection calls `level_tick` at the head of each level's tick end,
// before the level_tick_post hook, so the sweep reads here and writes there.

use std::sync::OnceLock;

/// What Kiloton does at the head of a level's tick end, in this order.
pub trait Level: Send + Sync {
    /// The reactors and the creative sources push their power into what they touch, after the block
    /// entities ticked. `energy(op, a, face_a, b, face_b, amount)`: op 0 moves up to `amount` from a
    /// (through face_a) into b (through face_b), 1 inserts into a, 2 extracts from a, 3 reads a's amount
    /// (-1 where no buffer is), 4 its capacity. Faces as `Direction::ALL` (down, up, north, south,
    /// west, east). Java: `Energies.send`, `PowerBuffer.push`.
    fn energy(&self, dimension: &str, energy: &mut dyn FnMut(u8, [i32; 3], u8, [i32; 3], u8, i32) -> i32);
    /// Devices whose pit gave no nuclear yield go off with their lenses only: a conventional blast
    /// (Java: `level.explode(..., 6, TNT)`). `explode(centre, power)` is false when nothing went off.
    fn explode(&self, dimension: &str, explode: &mut dyn FnMut([f64; 3], f32) -> bool);
    /// The game's block facts (hardness, blast resistance, light, fluids) and the level's blocks, with
    /// per state: whether it can collide, how readily it burns, a pillar's axis (0 none, 1 x, 2 y, 3 z)
    /// and whether a door is open (0 not a door, 1 shut, 2 open).
    #[allow(clippy::too_many_arguments)]
    fn read(
        &self,
        dimension: &str,
        facts: &horizon_client::blocks::BlockFacts,
        world: &horizon_client::world::World,
        collides: &dyn Fn(u32) -> bool,
        ignite_odds: &dyn Fn(u32) -> i32,
        axis: &dyn Fn(u32) -> u8,
        open: &dyn Fn(u32) -> u8,
    );
    /// The fluids at the crater's edge get a tick so the water there pours into the hole (Java:
    /// `BlastShape.edgeFluids`): `(position, kind, delay)`, kind 1 water, 2 flowing water, 3 lava,
    /// else flowing lava.
    fn ticks(&self, dimension: &str, schedule_fluids: &mut dyn FnMut(&[([i32; 3], u8, i32)]));
}

static LEVEL: OnceLock<Box<dyn Level>> = OnceLock::new();

/// Called once by the mod's `hooks()`; a second call is ignored.
pub fn install(level: Box<dyn Level>) {
    let _ = LEVEL.set(level);
}

/// The `level` injection's body, in `Game::native_level_tick_post`.
pub(super) fn level_tick(game: &mut super::Game<'_>, dimension: &str) {
    let Some(level) = LEVEL.get() else { return };
    // Mixin code isn't switched off with the mod in Options > Mods: it asks (docs/native-mods.md).
    if !game.native_on("kiloton") {
        return;
    }
    level.energy(dimension, &mut |op: u8, a: [i32; 3], fa: u8, b: [i32; 3], fb: u8, n: i32| {
        let d = |i: u8| horizon_core::geometry::Direction::ALL[(i % 6) as usize];
        match op {
            0 => game.native_move_energy(dimension, a, d(fa), b, d(fb), n),
            1 => game.native_energy_insert(a, d(fa), n, false),
            2 => game.native_energy_extract(dimension, a, d(fa), n, false),
            3 => game.native_energy(dimension, a, d(fa)).map_or(-1, |e| e.amount),
            _ => game.native_energy(dimension, a, d(fa)).map_or(-1, |e| e.capacity),
        }
    });
    level.explode(dimension, &mut |c: [f64; 3], r: f32| {
        game.native_explode(dimension, c, r, false, super::native_explosions::NativeBlast::Tnt)
    });
    {
        let g: &super::Game<'_> = game;
        let id = horizon_world::registry::BlockStateId;
        let property = |s: u32, name: &str| -> Option<String> {
            g.pack.assets.registry.state(id(s)).and_then(|b| b.properties.get(name)).map(|v| v.as_str().to_owned())
        };
        level.read(
            dimension,
            g.res.facts(),
            &g.world,
            &|s: u32| g.res.collision().may_collide(id(s)),
            &|s: u32| g.res.behaviours().data(id(s)).ignite_odds,
            &|s: u32| match property(s, "axis").as_deref() {
                Some("x") => 1,
                Some("y") => 2,
                Some("z") => 3,
                _ => 0,
            },
            &|s: u32| match property(s, "open").as_deref() {
                Some("false") => 1,
                Some("true") => 2,
                _ => 0,
            },
        );
    }
    level.ticks(dimension, &mut |wanted: &[([i32; 3], u8, i32)]| {
        game.with_level(|_, lvl, ticks| {
            for &(p, kind, delay) in wanted {
                let fluid = match kind {
                    1 => "minecraft:water",
                    2 => "minecraft:flowing_water",
                    3 => "minecraft:lava",
                    _ => "minecraft:flowing_lava",
                };
                let _ = lvl.native_schedule_fluid_tick(ticks, p, fluid, delay);
            }
        });
    });
}
