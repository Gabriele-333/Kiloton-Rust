//! Kiloton for Horizon: the game side. The launcher links this crate into the game and calls
//! [`hooks`]; each method of `Hooks` is one of Horizon's hook points. The physics and everything
//! else that needs no game types lives in `kiloton-core`.

mod aftermath;
mod blast;
mod bomb;
mod bomb_screen;
mod cables;
mod client;
mod cloud_render;
mod command;
mod doom_client;
mod entities;
mod machine_screen;
mod machines;
mod profiles;
mod save;

use horizon_client::blocks::{BlockFacts, Fluid};
use horizon_client::world::World;
use horizon_client::mod_api::{
    BlockEntityContainerEvent, BlockEntityEnergyEvent, BlockEntityEvent, BlockEntityTickEvent, BlockEntityTicksEvent, BlockStateId, BreakSpeedEvent,
    CameraPoseEvent, CameraShakeEvent, ChunkLoadEvent, ClientTickPostEvent, Compound, ComputeFogColorEvent, ContainerAcceptsEvent,
    DestroyBlockEvent, Flow, Hooks, Host, HudEvent, LevelHost, LevelLoadEvent, LevelTickPostEvent, LevelUnloadEvent, LivingIncomingDamageEvent,
    MenuButtonEvent, MenuSlotRulesEvent, ModContainer, ModContainerViewersEvent, ModEnergyBuffer, NativeScreen, NetFromServerEvent,
    PerformCommandEvent, PlaceBlockEvent, RegisterCommandsEvent, RenderFogEvent, RenderHooks, RightClickBlockV2Event,
    ScreenClickEvent, ScreenDrawEvent, ScreenEvent, ScreenTooltipEvent, ServerStoppingEvent, StateForPlacementEvent,
    WorldSavedEvent,
};
use profiles::{Raw, RAW, TABLES};
use std::collections::HashSet;
use std::sync::Mutex;

pub fn hooks() -> Box<dyn Hooks> {
    horizon_client::game::kiloton::install(Box::new(Kiloton));
    Box::new(Kiloton)
}

struct Kiloton;

/// Levels loaded from the save whose loaded chunks still have to be checked for owed damage.
static TO_QUEUE: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// The levels open now: the save is written for these only (a closed level's records stay as saved).
static OPEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn open(dimension: &str, is_open: bool) {
    let mut g = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let set = g.get_or_insert_with(HashSet::new);
    if is_open {
        if !set.contains(dimension) {
            set.insert(dimension.to_owned());
        }
    } else {
        set.remove(dimension);
    }
}

/// The head of each level's tick end, from the game's `kiloton` glue module (horizon-mixin.toml): energy,
/// lenses-only blasts, the block facts and the level for the sweep, then the fluid ticks, in that order.
impl horizon_client::game::kiloton::Level for Kiloton {
    /// First at the level's tick end: the reactors and the creative sources push their power into
    /// what they touch, after the block entities ticked (machines.rs), then the cables move theirs.
    fn energy(&self, dimension: &str, energy: &mut dyn FnMut(u8, [i32; 3], u8, [i32; 3], u8, i32) -> i32) {
        machines::level_energy(dimension, energy);
        cables::level_energy(dimension, energy);
    }

    /// Right after `energy`: devices that went off with their lenses only.
    fn explode(&self, dimension: &str, explode: &mut dyn FnMut([f64; 3], f32) -> bool) {
        for (centre, power) in bomb::explosions(dimension) {
            if !explode(centre, power) {
                log::warn!("Kiloton: the lenses-only blast at {centre:?} in {dimension} did not go off");
            }
        }
    }

    /// The game's block facts and the level's blocks, for the sweep.
    fn read(
        &self,
        dimension: &str,
        facts: &BlockFacts,
        world: &World,
        collides: &dyn Fn(u32) -> bool,
        ignite_odds: &dyn Fn(u32) -> i32,
        axis: &dyn Fn(u32) -> u8,
        open_state: &dyn Fn(u32) -> u8,
    ) {
        open(dimension, true);
        let fresh = RAW.read().map(|r| r.as_ref().is_none_or(|r| r.len() != facts.states.len())).unwrap_or(true);
        if fresh {
            let raw: Vec<Raw> = facts
                .states
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let id = i as u32;
                    Raw {
                        name: f.name.to_string(),
                        hardness: f.hardness,
                        resistance: f.explosion_resistance,
                        collides: collides(id),
                        ignite_odds: ignite_odds(id),
                        axis: axis(id),
                        full_collision: f.full_collision,
                        block_entity: f.block_entity,
                        waterlogged: f.waterlogged,
                        fluid: match f.fluid {
                            Fluid::None => 0,
                            Fluid::Water => 1,
                            Fluid::Lava => 2,
                        },
                        fluid_source: f.fluid_level == 8 && !f.fluid_falling,
                        emission: f.emission,
                        legacy_solid: f.legacy_solid,
                        leaves_class: f.classes.iter().any(|c| &**c == "LeavesBlock"),
                        liquid_class: f.classes.iter().any(|c| &**c == "LiquidBlock"),
                        can_occlude: f.can_occlude,
                        metal: f.sounds.break_sound.contains("metal"),
                        door: f.classes.iter().any(|c| &**c == "DoorBlock" || &**c == "TrapDoorBlock"),
                        open: open_state(id),
                        replaceable: f.replaceable,
                        tall: f.classes.iter().any(|c| &**c == "DoublePlantBlock" || &**c == "DoorBlock" || &**c == "TallFlowerBlock" || &**c == "TallSeagrassBlock"),
                        boxes: if f.outline.len() == 1 && f.outline[0] == [0.0, 0.0, 0.0, 1.0, 1.0, 1.0] {
                            Vec::new()
                        } else {
                            f.outline.to_vec()
                        },
                    }
                })
                .collect();
            log::info!("Kiloton: block facts of {} states read", raw.len());
            *RAW.write().unwrap_or_else(|e| e.into_inner()) = Some(raw);
            *TABLES.write().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let tables = TABLES.read().unwrap_or_else(|e| e.into_inner());
        let Some(tables) = tables.as_ref() else { return };
        let queue = TO_QUEUE.lock().ok().and_then(|mut q| q.as_mut().map(|q| q.remove(dimension))).unwrap_or(false);
        blast::with(|b| {
            if queue {
                b.queue_loaded(dimension, world);
            }
            b.read(dimension, world, tables);
            b.creatures_read(dimension, world, tables);
        });
    }

    /// Right after `read`: the fluids at the crater's edge get their tick (Java: `edgeFluids`).
    fn ticks(&self, dimension: &str, schedule_fluids: &mut dyn FnMut(&[([i32; 3], u8, i32)])) {
        let wanted: Vec<([i32; 3], u8, i32)> = blast::with(|b| {
            if b.fluid_ticks.is_empty() {
                return Vec::new();
            }
            let (here, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut b.fluid_ticks).into_iter().partition(|t| t.0 == dimension);
            b.fluid_ticks = rest;
            here.into_iter().map(|(_, p, k, d)| (p, k, d)).collect()
        });
        if !wanted.is_empty() {
            log::info!("Kiloton: {} fluid tick(s) at the crater's edge in {dimension}", wanted.len());
            schedule_fluids(&wanted);
        }
    }
}

impl Hooks for Kiloton {
    /// The cloud is drawn in Horizon's frame hooks (`cloud_render`).
    fn render(&self) -> Option<&dyn RenderHooks> {
        Some(&cloud_render::RENDER)
    }

    fn net_from_server(&self, host: &mut dyn Host, event: &mut NetFromServerEvent<'_>) {
        if event.channel == entities::DOOM_CHANNEL {
            doom_client::start(&*host, event.data);
        } else if event.channel == entities::SHADOW_CHANNEL {
            client::shadow(event.data);
        } else {
            client::net_from_server(event.channel, event.data);
        }
    }

    fn client_tick_post(&self, host: &mut dyn Host, _event: &mut ClientTickPostEvent) {
        client::client_tick(host);
        doom_client::tick(&*host);
    }

    /// The death scene's camera, out of the body (Java: `CameraMixin` + `ClientDoom`).
    fn camera_pose(&self, host: &dyn Host, event: &mut CameraPoseEvent<'_>) {
        if let Some((pos, yaw, pitch)) = doom_client::camera(host, event.partial) {
            *event.position = pos;
            event.yaw = yaw;
            event.pitch = pitch;
        }
    }

    /// The blast wave shakes the camera when it reaches it (and the ground shock just before).
    fn camera_shake(&self, _host: &dyn Host, event: &mut CameraShakeEvent<'_>) {
        if let Some([yaw, pitch, roll]) = client::shake_at(event.partial) {
            event.shake.yaw += yaw;
            event.shake.pitch += pitch;
            event.shake.roll += roll;
        }
    }

    /// The fog takes the colour of the flash, the fireball's glow and the dust (Java: `ClientFx.tintFog`).
    fn compute_fog_color(&self, _host: &dyn Host, event: &mut ComputeFogColorEvent<'_>) {
        client::tint_fog(event.rgb);
    }

    /// The dust in the air after the wave pulls the fog in (Java: `ClientEffects.fog`).
    fn render_fog(&self, _host: &dyn Host, event: &mut RenderFogEvent<'_>) {
        if let Some((near, far)) = client::fog(event.fog.render_distance_start, event.fog.render_distance_end) {
            event.fog.render_distance_start = near;
            event.fog.render_distance_end = far;
        }
    }

    /// The flash: the view goes white (Java: `FlashOverlay`'s white-out).
    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent<'_>) {
        let wo = client::whiteout();
        if wo > 0.003 {
            let a = (wo.min(1.0) * 255.0) as u32;
            host.fill(event.batch, [0.0, 0.0, 16384.0, 16384.0], (a << 24) | 0x00FF_FBF2);
        }
    }

    fn register_commands(&self, _host: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        command::register(event.commands);
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        if command::perform(host, event.player, event.command) { Flow::Cancel } else { Flow::Continue }
    }

    /// Each level's tick, right after the head: what the sweep wrote goes into the world.
    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent<'_>) {
        let need = TABLES.read().map(|t| t.is_none()).unwrap_or(false);
        if need {
            let raw = RAW.read().unwrap_or_else(|e| e.into_inner());
            if let Some(raw) = raw.as_ref() {
                let t0 = std::time::Instant::now();
                let tables = profiles::build(host, raw);
                log::info!("Kiloton: block profiles of {} states built in {} ms", raw.len(), t0.elapsed().as_millis());
                *TABLES.write().unwrap_or_else(|e| e.into_inner()) = Some(tables);
            }
        }
        let names = |s: u32| RAW.read().ok().and_then(|r| r.as_ref().and_then(|r| r.get(s as usize).map(|x| x.name.clone())));
        blast::with(|b| b.apply(host, event.dimension, &names));
        bomb::level_tick(host, event.dimension);
    }

    // ---------------------------------------------------------------------------------------- the device (bomb.rs)

    fn block_entity(&self, _host: &mut dyn Host, event: &mut BlockEntityEvent<'_>) -> Option<(String, Compound)> {
        bomb::block_entity(event.block).or_else(|| machines::block_entity(event.block)).or_else(|| cables::block_entity(event.block))
    }

    fn block_entity_energy(&self, _host: &dyn Host, event: &mut BlockEntityEnergyEvent<'_>) -> Option<ModEnergyBuffer> {
        machines::energy_buffer(event.block).or_else(|| cables::energy_buffer(event.block))
    }

    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        if bomb::is_bomb(event.block) || machines::ticks(event.block) || cables::tier(event.block).is_some() {
            event.ticks = true;
        }
    }

    fn block_entity_tick(&self, host: &mut dyn Host, event: &mut BlockEntityTickEvent<'_>) {
        if event.entity.id == bomb::ENTITY {
            bomb::tick(host, event.pos, event.entity);
        } else if event.entity.id == machines::ENTITY || event.entity.id == machines::CREATIVE_SOURCE {
            machines::tick(host, event.pos, event.entity);
        } else if event.entity.id == cables::ENTITY {
            cables::tick(host, event.pos, event.entity);
        }
    }

    fn block_entity_container(&self, _host: &dyn Host, event: &mut BlockEntityContainerEvent<'_>) -> Option<ModContainer> {
        bomb::container(event.block).or_else(|| machines::container(event.block))
    }

    fn container_accepts(&self, host: &dyn Host, event: &mut ContainerAcceptsEvent<'_>) -> Flow {
        let refused = if event.entity.id == bomb::ENTITY {
            !bomb::hopper_accepts(host, event.pos, event.entity, event.slot, &event.stack.item)
        } else if event.entity.id == machines::ENTITY {
            !machines::hopper_accepts(host, &host.block(event.pos), event.slot, &event.stack.item)
        } else {
            false
        };
        if refused { Flow::Cancel } else { Flow::Continue }
    }

    fn menu_slot_rules(&self, event: &mut MenuSlotRulesEvent<'_>) {
        if let Some((may_place, max_stack)) = bomb::slot_rules(event.menu, event.slot, &event.stack.item) {
            event.may_place = may_place;
            event.max_stack = max_stack;
        } else if let Some(may_place) = machines::slot_rules(event.menu, event.slot, &event.stack.item) {
            event.may_place = may_place;
        }
    }

    fn mod_container_viewers(&self, _host: &mut dyn Host, event: &mut ModContainerViewersEvent<'_>) {
        bomb::viewers(event.dimension, event.pos, event.viewers);
    }

    fn menu_button(&self, host: &mut dyn Host, event: &mut MenuButtonEvent<'_>) {
        bomb::button(host, event.pos, event.entity, event.id);
    }

    /// Either half opens the panel: the device moves into the half clicked first.
    fn right_click_block_v2(&self, host: &mut dyn Host, event: &mut RightClickBlockV2Event) -> Flow {
        // The creative source says what it is (Java: `CreativeSourceBlock.useWithoutItem`).
        if host.block(event.pos) == machines::CREATIVE_SOURCE {
            if let Some(p) = host.acting_player() {
                host.action_bar(p.uuid, horizon_client::mod_api::Text::translate("block.kiloton.creative_source.status", &[]));
            }
            return Flow::Continue;
        }
        // A cable with an empty hand: its network's status, or with shift a side's setting (cables.rs).
        if cables::clicked(host, event.pos, event.face) {
            return Flow::Continue;
        }
        if bomb::clicked(host, event.pos) { Flow::Continue } else { Flow::Cancel }
    }

    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        if bomb::is_bomb(&host.block(event.pos)) && !bomb::breaking(host, event.pos) { Flow::Cancel } else { Flow::Continue }
    }

    fn break_speed(&self, host: &dyn Host, event: &mut BreakSpeedEvent) {
        if bomb::armed_state(host, event.state) {
            event.speed = 0.0;
        }
    }

    fn state_for_placement(&self, level: &dyn LevelHost, event: &mut StateForPlacementEvent<'_>) -> Option<Option<BlockStateId>> {
        if machines::is_machine(&event.context.item) {
            return Some(level.state_with_property(event.state, "facing", machines::placement_facing(event.context.yaw)));
        }
        if !bomb::is_bomb(&event.context.item) {
            return None;
        }
        let raw = RAW.read().ok();
        let replaceable = |p: [i32; 3]| {
            let s = level.block_state(p).0 as usize;
            raw.as_ref().and_then(|r| r.as_ref()).and_then(|r| r.get(s)).map_or(s == 0, |x| x.replaceable)
        };
        let at = event.context.pos;
        let facing = bomb::placement_facing(event.context.yaw, replaceable, [at.x, at.y, at.z]);
        Some(facing.and_then(|f| level.state_with_property(event.state, "facing", f.name())))
    }

    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        if bomb::is_bomb(&event.stack.item) {
            bomb::placed(host, event.pos, event.state);
        }
    }

    fn screen(&self, _host: &dyn Host, event: &mut ScreenEvent<'_>) -> Option<NativeScreen> {
        bomb_screen::layout(event.block).or_else(|| machine_screen::layout(event.block))
    }

    fn screen_draw(&self, host: &dyn Host, event: &mut ScreenDrawEvent<'_>) {
        bomb_screen::draw(host, event.block, event.data, event.batch, event.origin);
        machine_screen::draw(host, event.block, event.data, event.batch, event.origin);
    }

    fn screen_click(&self, _host: &dyn Host, event: &mut ScreenClickEvent<'_>) -> Option<i32> {
        bomb_screen::click(event.block, event.data, event.at, event.right)
    }

    fn screen_tooltip(&self, host: &dyn Host, event: &mut ScreenTooltipEvent<'_>) {
        bomb_screen::tooltip(host, event.block, event.data, event.at, event.lines);
        machine_screen::tooltip(host, event.block, event.data, event.at, event.lines);
    }

    fn server_stopping(&self, _host: &mut dyn Host, _event: &mut ServerStoppingEvent) {
        bomb::forget_all();
        machines::forget_all();
        cables::forget_all();
    }

    /// Players held for the death scene take no other damage (Java: `Doom.cancelsDamage`).
    fn living_incoming_damage(&self, event: &mut LivingIncomingDamageEvent<'_>) -> Flow {
        if entities::cancels(event.entity, event.damage_type) { Flow::Cancel } else { Flow::Continue }
    }

    fn chunk_load(&self, _host: &mut dyn Host, event: &mut ChunkLoadEvent<'_>) {
        blast::with(|b| b.chunk_loaded(event.dimension, event.chunk[0], event.chunk[1]));
    }

    /// A level opened: the damage its save owes comes back.
    fn level_load(&self, host: &mut dyn Host, event: &mut LevelLoadEvent<'_>) {
        open(event.dimension, true);
        let Some(world) = host.world_dir() else { return };
        // The server settings, in the file the Java edition uses (written with the defaults if missing).
        let cfg_path = world.join("serverconfig").join("kiloton-server.toml");
        let text = std::fs::read_to_string(&cfg_path).unwrap_or_else(|_| {
            let _ = std::fs::create_dir_all(world.join("serverconfig"));
            let _ = std::fs::write(&cfg_path, kiloton_core::config::ServerConfig::TEMPLATE);
            kiloton_core::config::ServerConfig::TEMPLATE.to_owned()
        });
        let cfg = kiloton_core::config::ServerConfig::parse(&text);
        blast::with(|b| b.config = Some(cfg));
        // The armed devices of this level count down again (their chunks are held as before the save).
        bomb::level_load(host, &world, event.dimension);
        // The nuclear shadows left in this level.
        let shadows: Vec<_> = entities::read_shadows(&world).into_iter().filter(|s| s.0 == event.dimension).collect();
        blast::with(|b| {
            for s in shadows {
                if !b.shadows.iter().any(|o| o.1 == s.1) {
                    b.shadows.push(s);
                }
            }
        });
        // The fallout of earlier bursts in this level.
        let sources: Vec<_> = aftermath::read(&world).into_iter().filter(|s| s.dimension == event.dimension).collect();
        if !sources.is_empty() {
            blast::with(|b| {
                for s in sources {
                    b.bump_id(s.id);
                    if !b.aftermath.sources.iter().any(|o| o.id == s.id && o.dimension == s.dimension) {
                        b.aftermath.sources.push(s);
                    }
                }
            });
        }
        let records: Vec<_> = save::read(&world).into_iter().filter(|r| r.dimension == event.dimension).collect();
        if records.is_empty() {
            return;
        }
        log::info!("Kiloton: {} burst(s) in {} still owe damage to chunks not loaded yet", records.len(), event.dimension);
        blast::with(|b| b.restore(records));
        TO_QUEUE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new).insert(event.dimension.to_owned());
    }

    /// The world was saved: so are the bursts (running sweeps owe what they have not reached).
    fn world_saved(&self, host: &mut dyn Host, _event: &mut WorldSavedEvent) {
        let Some(world) = host.world_dir() else { return };
        let dims: Vec<String> = OPEN.lock().ok().and_then(|g| g.clone()).unwrap_or_default().into_iter().collect();
        blast::with(|b| {
            let records: Vec<_> = dims.iter().flat_map(|d| b.records_for_save(d)).collect();
            save::write(&world, &dims, &records);
            b.dirty = false;
            aftermath::write(&world, &b.aftermath.sources);
            b.aftermath.dirty = false;
            entities::write_shadows(&world, &b.shadows);
        });
        bomb::save(&world);
    }

    /// A level closed: its bursts go in the save and leave memory.
    fn level_unload(&self, host: &mut dyn Host, event: &mut LevelUnloadEvent<'_>) {
        let Some(world) = host.world_dir() else { return };
        blast::with(|b| {
            let records = b.records_for_save(event.dimension);
            save::write(&world, &[event.dimension.to_owned()], &records);
            b.forget(event.dimension);
        });
        open(event.dimension, false);
    }
}
