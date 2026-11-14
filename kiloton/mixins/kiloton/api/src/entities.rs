//! What a burst does to players and creatures (Java: `BlastJob.prompt/flash/blastEntities/hitEntity`,
//! `Radiation`, `Doom`): the flash burns what sees the fireball, the prompt radiation goes through
//! walls by their thickness, the blast wave kills inside the fireball, hurts and throws outside it
//! and gets under cover only through the air; the dose makes them sick. A player killed by the
//! flash in the open is held for the death scene, then dies of it.
//!
//! Each tick has two halves, like the sweep: the `level_tick_post` hook (with the Host) takes a
//! snapshot of the creatures and carries out what was decided, and the next tick's head (the
//! `Level::read`, with the level's blocks) decides it: lines of sight, shelters, the way in
//! for the wave.

use crate::profiles::Tables;
use horizon_client::world::World as GameWorld;
use kiloton_core::blast::cover::{Cover, CoverWorld};
use kiloton_core::config::ServerConfig;
use kiloton_core::physics::fallout;
use kiloton_core::BlastModel;
use std::collections::{HashMap, HashSet};

/// Flash fluence that kills for certain in the open, cal/cm2 (Java: `Doom.LETHAL_FLUENCE`).
pub const LETHAL_FLUENCE: f64 = 20.0;
/// How long the death scene lasts before death, ticks (Java: `Doom.TICKS`).
pub const DOOM_TICKS: i64 = 130;
pub const DOOM_CHANNEL: &str = "kiloton:doom";
pub const SHADOW_CHANNEL: &str = "kiloton:shadow";

/// A nuclear shadow left in the world (Java: `NuclearShadowEntity`): dimension, id, feet, direction away
/// from the fireball (radians), length (blocks).
pub type Shadow = (String, u32, [f64; 3], f32, f32);

/// The shadow of a body of 1.8 m under a light this high above the horizon, kept to about twice the body
/// (light at least 24 degrees up) so that it still reads as the figure of a player (Java: `Doom.doom`).
pub fn shadow_of(pos: [f64; 3], fireball: [f64; 3]) -> (f32, f32) {
    let (dx, dz) = (pos[0] - fireball[0], pos[2] - fireball[2]);
    let horiz = (dx * dx + dz * dz).sqrt().max(1.0);
    let elev = (24.0f64).to_radians().max((fireball[1] - (pos[1] + 0.9)).atan2(horiz));
    (dz.atan2(dx) as f32, (1.8 / elev.tan()) as f32)
}

pub fn shadow_message(id: u32, pos: [f64; 3], angle: f32, length: f32) -> Vec<u8> {
    let mut b = Vec::with_capacity(36);
    b.extend_from_slice(&id.to_le_bytes());
    for v in pos {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&angle.to_le_bytes());
    b.extend_from_slice(&length.to_le_bytes());
    b
}

pub fn read_shadows(world: &std::path::Path) -> Vec<Shadow> {
    let Ok(text) = std::fs::read_to_string(world.join("kiloton").join("shadows.json")) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    v.get("shadows")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let f = |k: &str| s.get(k).and_then(|x| x.as_f64());
            Some((
                s.get("dimension")?.as_str()?.to_owned(),
                s.get("id")?.as_u64()? as u32,
                [f("x")?, f("y")?, f("z")?],
                f("angle")? as f32,
                f("length")? as f32,
            ))
        })
        .collect()
}

pub fn write_shadows(world: &std::path::Path, shadows: &[Shadow]) {
    let v = serde_json::json!({ "version": 1, "shadows": shadows.iter().map(|(d, id, p, a, l)| serde_json::json!({
        "dimension": d, "id": id, "x": p[0], "y": p[1], "z": p[2], "angle": a, "length": l
    })).collect::<Vec<_>>() });
    let dir = world.join("kiloton");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("shadows.json");
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, v.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The entity ids of the players held for the death scene: nothing else hurts them meanwhile (Java:
/// `Doom.cancelsDamage`). Apart from the bursts' lock, which is held while they are hurt.
pub static DOOMED_IDS: std::sync::Mutex<Option<HashSet<i32>>> = std::sync::Mutex::new(None);

/// Whether this damage to this entity is to be cancelled.
pub fn cancels(entity: i32, damage_type: &str) -> bool {
    damage_type != "kiloton:nuclear_shadow"
        && DOOMED_IDS.lock().map(|g| g.as_ref().is_some_and(|s| s.contains(&entity))).unwrap_or(false)
}

/// One creature as the Host showed it.
#[derive(Clone, Debug)]
pub struct Snap {
    pub id: u32,
    pub kind: String,
    pub pos: [f64; 3],
    pub eye: [f64; 3],
    pub uuid: Option<u128>,
    pub player: bool,
    /// Creative or spectator: nothing touches them.
    pub immune: bool,
    pub living: bool,
    pub yaw: f32,
    pub pitch: f32,
}

/// What the Host is to do.
#[derive(Clone, Debug)]
pub enum Order {
    Hurt { id: u32, amount: f32, kind: &'static str },
    Fire { id: u32, ticks: i32 },
    Push { id: u32, v: [f64; 3] },
    Remove { id: u32 },
    Dose { uuid: u128, id: u32, player: bool, rem: f64 },
    Doom { uuid: u128, id: u32, pos: [f64; 3], yaw: f32, pitch: f32, fireball: [f64; 3], shadow: bool },
}

/// One burst's work on the creatures.
#[derive(Default)]
pub struct EntityWork {
    pub prompt_done: bool,
    pub done: bool,
    flashed: HashSet<u32>,
    hit: HashSet<u32>,
}

/// The level's blocks as the cover algorithms read them.
pub struct CoverView<'a> {
    pub world: &'a GameWorld,
    pub tables: &'a Tables,
}

impl CoverWorld for CoverView<'_> {
    fn loaded(&self, x: i32, z: i32) -> bool {
        self.world.loaded(x, z)
    }
    fn state(&self, x: i32, y: i32, z: i32) -> u32 {
        self.world.block(x, y, z).0
    }
    fn sky(&self, x: i32, y: i32, z: i32) -> bool {
        y >= self.surface(x, z)
    }
    fn min_y(&self) -> i32 {
        self.world.bottom_y()
    }
    fn max_y(&self) -> i32 {
        self.world.top_y()
    }
    fn surface(&self, x: i32, z: i32) -> i32 {
        let t = self.tables;
        self.world.world_surface(x, z, |s| t.air.get(s.0 as usize).copied().unwrap_or(false))
    }
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn block(p: [f64; 3]) -> [i32; 3] {
    [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]
}

/// Burns from a flash of `q` cal/cm2: a player's short of the lethal fluence never take more than 19
/// points, at or past it the flash kills (Java: `BlastJob.flashDamage`, the change of 10-11).
pub fn flash_damage(player: bool, q: f64) -> f32 {
    let d = (60.0f64).min((q - 2.0) * 1.3);
    (if player { if q >= LETHAL_FLUENCE { 1000.0 } else { d.min(19.0) } } else { d }) as f32
}

pub struct Burst<'a> {
    pub id: u32,
    pub model: &'a BlastModel,
    /// Ground zero (Java: `BlastJob.centre`).
    pub centre: [f64; 3],
    pub age: f64,
    /// The radius cap of the burst.
    pub cap: f64,
}

impl Burst<'_> {
    fn fireball(&self) -> [f64; 3] {
        [self.centre[0], self.centre[1] + (0.5 * self.model.fireball_radius).max(2.0), self.centre[2]]
    }

    fn entity_radius(&self) -> f64 {
        self.model.radius_for_psi(0.8).min(self.cap * 1.5)
    }

    /// The decisions of one tick for this burst, from the last snapshot.
    pub fn decide(&self, w: &mut EntityWork, snaps: &[Snap], view: &CoverView<'_>, cfg: &ServerConfig, doomed: &HashSet<u128>, out: &mut Vec<Order>) {
        if w.done {
            return;
        }
        let cover = Cover { world: view, table: &view.tables.cover };
        let m = self.model;
        if !w.prompt_done {
            w.prompt_done = true;
            if cfg.entity_damage {
                let reach = m.radius_for_fluence(3.0).max(m.radius_for_dose(50.0)).min(self.cap * 2.0);
                for s in snaps.iter().filter(|s| s.living && dist(s.pos, self.centre) <= reach) {
                    self.flash(w, s, &cover, cfg, out);
                }
            }
        }
        let front = m.front_radius(self.age);
        let radius = self.entity_radius();
        if front > radius {
            w.done = true;
        }
        for s in snaps {
            if w.hit.contains(&s.id) {
                continue;
            }
            let d = dist(s.pos, self.centre);
            if d > front || d > radius {
                continue;
            }
            w.hit.insert(s.id);
            self.hit(w, s, d, &cover, cfg, doomed, out);
        }
    }

    /// The flash and the prompt dose for one creature (Java: `BlastJob.flash`; the exosuit is not
    /// ported yet).
    fn flash<W: CoverWorld>(&self, w: &mut EntityWork, s: &Snap, cover: &Cover<'_, W>, cfg: &ServerConfig, out: &mut Vec<Order>) {
        if !w.flashed.insert(s.id) || s.immune {
            return;
        }
        let m = self.model;
        let fb = self.fireball();
        let d = dist(s.pos, self.centre);
        let inside = d < m.fireball_radius;
        // A closed shelter: no light gets in, and the walls cut the prompt radiation. Creatures other
        // than players only count filtered shelters in the Java edition, which aren't ported yet.
        let sealed = !inside && s.player && cover.shelter(block(s.eye), 2048).sealed;
        let burns = !s.player || cfg.flash_hurts_players;
        if s.player && burns && cfg.death_scene {
            let through = if inside { 1.0 } else if sealed { 0.0 } else { cover.transmission(s.eye, fb) };
            let fluence = m.thermal_fluence(d) * through;
            let doomed = inside || fluence >= LETHAL_FLUENCE;
            log::info!(
                "Kiloton: burst #{}: the flash reaches a player at {:.0} m, {:.0} cal/cm2{}: {}",
                self.id,
                d,
                fluence,
                if inside { " inside the fireball" } else if through >= 1.0 { "" } else if through > 0.0 { " through foliage" } else { ", fireball hidden" },
                if doomed { "death scene" } else { "burns only" }
            );
            if doomed {
                if let Some(uuid) = s.uuid {
                    out.push(Order::Doom { uuid, id: s.id, pos: s.pos, yaw: s.yaw, pitch: s.pitch, fireball: fb, shadow: !inside });
                }
                return;
            }
        }
        if inside {
            return; // the blast vaporises it
        }
        let open = m.initial_dose_rem(d);
        let rem = if open >= 10.0 { open * cover.prompt(s.eye, fb) } else { 0.0 };
        let dose = |rem: f64, out: &mut Vec<Order>| {
            if rem >= 10.0 {
                if let Some(uuid) = s.uuid {
                    out.push(Order::Dose { uuid, id: s.id, player: s.player, rem });
                }
            }
        };
        if sealed {
            dose(rem, out);
            return;
        }
        let q = m.thermal_fluence(d) * cover.transmission(s.eye, fb);
        if q >= 3.0 && burns {
            if !s.player || cfg.flash_ignites_players {
                out.push(Order::Fire { id: s.id, ticks: ((q * 0.8).min(20.0) * 20.0) as i32 });
            }
            out.push(Order::Hurt { id: s.id, amount: flash_damage(s.player, q), kind: "kiloton:nuclear_flash" });
            if s.player && !cfg.death_scene {
                log::info!("Kiloton: burst #{}: the flash reaches a player at {:.0} m, {:.1} cal/cm2: {} points", self.id, d, q, flash_damage(true, q));
            }
        }
        dose(rem, out);
    }

    /// The blast wave on one creature or thing (Java: `BlastJob.hitEntity`).
    #[allow(clippy::too_many_arguments)]
    fn hit<W: CoverWorld>(
        &self,
        w: &mut EntityWork,
        s: &Snap,
        d: f64,
        cover: &Cover<'_, W>,
        cfg: &ServerConfig,
        doomed: &HashSet<u128>,
        out: &mut Vec<Order>,
    ) {
        let m = self.model;
        if s.player && cfg.entity_damage && !w.flashed.contains(&s.id) {
            self.flash(w, s, cover, cfg, out);
        }
        if s.uuid.is_some_and(|u| doomed.contains(&u)) || out.iter().any(|o| matches!(o, Order::Doom { id, .. } if *id == s.id)) {
            return;
        }
        let damage = cfg.entity_damage;
        if d < m.fireball_radius && !s.immune {
            if s.living {
                if damage {
                    out.push(Order::Hurt { id: s.id, amount: f32::MAX, kind: "kiloton:nuclear_blast" });
                }
            } else if !s.player {
                out.push(Order::Remove { id: s.id });
            }
            return;
        }
        let mut psi = m.overpressure_psi(d);
        if s.living && !s.immune && s.player && d >= m.crater_radius * 1.2 && d >= m.fireball_radius {
            let r = cover.shelter(block(s.eye), 2048);
            if r.sealed && r.rating_psi > psi {
                return;
            }
        }
        // Away from the open sky the wave gets in only through the air (the fix of 09-11).
        let (mut wind_at, mut weaken, mut along) = (d, 1.0, None);
        if s.living && d >= m.crater_radius * 1.2 {
            let eye = block(s.eye);
            if let Some(wave) = cover.wave(eye, s.player) {
                if wave.closed {
                    let room = cover.shelter(eye, wave.volume + 8);
                    if room.sealed && room.rating_psi > psi {
                        return;
                    }
                } else {
                    if let Some(o) = wave.opening {
                        wind_at = dist([o[0] as f64 + 0.5, o[1] as f64 + 0.5, o[2] as f64 + 0.5], self.centre);
                    }
                    psi = m.overpressure_psi(wind_at) * wave.factor;
                    weaken = wave.factor;
                    along = wave.direction;
                }
            }
        }
        let hanging = matches!(s.kind.as_str(), "minecraft:painting" | "minecraft:item_frame" | "minecraft:glow_item_frame" | "minecraft:leash_knot");
        if hanging && psi > 1.5 {
            out.push(Order::Remove { id: s.id });
            return;
        }
        if s.kind == "minecraft:item" && d < m.radius_for_psi(20.0) {
            out.push(Order::Remove { id: s.id });
            return;
        }
        let push = (m.wind_speed(wind_at) / 20.0 * 0.9).min(3.5) * weaken;
        if s.living && damage && !s.immune {
            let dmg = BlastModel::blast_damage(psi);
            if dmg > 0.5 {
                out.push(Order::Hurt { id: s.id, amount: dmg as f32, kind: "kiloton:nuclear_blast" });
            }
        }
        if push > 0.02 && !s.immune {
            let dir = [s.pos[0] - self.centre[0], s.pos[2] - self.centre[2]];
            let h = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
            let v = match along {
                Some(a) => [a[0], a[1] + 0.1, a[2]],
                None if h < 1.0e-3 => [0.0, 1.0, 0.0],
                None => [dir[0] / h, 0.25, dir[1] / h],
            };
            out.push(Order::Push { id: s.id, v: [v[0] * push, v[1] * push, v[2] * push] });
        }
    }
}

/// A creature's dose and sickness (Java: `Radiation`, `RadiationSicknessEffect`).
#[derive(Clone, Debug, Default)]
pub struct Dose {
    pub dimension: String,
    pub id: u32,
    pub player: bool,
    /// Absorbed dose, repaired at 1 rem a second.
    pub rem: f64,
    /// Everything absorbed since the last death (the dosimeter), players only.
    pub total: f64,
    /// Radiation sickness: its level (0 = I) and the game tick it ends, none when -1.
    pub amp: i32,
    pub until: i64,
}

/// The radiation sickness level for a dose, or None under 100 rem.
pub fn sickness(rem: f64) -> Option<i32> {
    if rem >= 1500.0 {
        Some(4)
    } else if rem >= 800.0 {
        Some(3)
    } else if rem >= 450.0 {
        Some(2)
    } else if rem >= 250.0 {
        Some(1)
    } else if rem >= 100.0 {
        Some(0)
    } else {
        None
    }
}

/// The doses of every creature, by UUID, and the players held for the death scene.
#[derive(Default)]
pub struct Bodies {
    pub doses: HashMap<u128, Dose>,
    /// Player UUID -> (dimension, entity id, where they are held, yaw, pitch, end tick).
    pub doomed: HashMap<u128, (String, u32, [f64; 3], f32, f32, i64)>,
}

impl Bodies {
    /// `Radiation.addDose` (creative players count on the dosimeter only).
    pub fn add(&mut self, dimension: &str, uuid: u128, id: u32, player: bool, immune: bool, rem: f64, now: i64) -> Vec<(u128, &'static str, i32, i32)> {
        let d = self.doses.entry(uuid).or_insert_with(|| Dose { amp: -1, ..Default::default() });
        d.dimension = dimension.to_owned();
        d.id = id;
        d.player = player;
        if player {
            d.total += rem;
        }
        if immune {
            return Vec::new();
        }
        d.rem += rem;
        Self::sicken(d, uuid, now)
    }

    /// `Radiation.applySickness`: the sickness level and its length from the dose; nausea and weakness
    /// as vanilla effects (uuid, effect, ticks, amplifier).
    fn sicken(d: &mut Dose, uuid: u128, now: i64) -> Vec<(u128, &'static str, i32, i32)> {
        let mut effects = Vec::new();
        let Some(amp) = sickness(d.rem) else { return effects };
        let duration = (20.0 * 60.0 * 5.0f64).min(20.0 * (d.rem / 2.0)) as i64;
        let left = d.until - now;
        if d.amp < amp || left < duration / 2 {
            d.amp = amp;
            d.until = now + duration;
        }
        // The Java edition rolls 1 in 40 for nausea at each application.
        if d.rem >= 200.0 && (now as u64).wrapping_mul(0x9E3779B97F4A7C15).rotate_left(17) % 40 == 0 {
            effects.push((uuid, "minecraft:nausea", 160, 0));
        }
        if d.rem >= 250.0 {
            effects.push((uuid, "minecraft:weakness", 200, if amp >= 3 { 1 } else { 0 }));
        }
        effects
    }

    /// The sickness hurts on its own clock: level II every 4 s, III every 2 s, IV and up every second,
    /// harder (`RadiationSicknessEffect`). Returns (dimension, entity id, damage).
    pub fn tick_sickness(&mut self, now: i64) -> Vec<(String, u32, f32)> {
        let mut out = Vec::new();
        for d in self.doses.values_mut() {
            if d.amp < 0 || now >= d.until {
                d.amp = -1;
                continue;
            }
            let interval = if d.amp >= 3 { 20 } else if d.amp == 2 { 40 } else { 80 };
            if d.amp >= 1 && (d.until - now) % interval == 0 {
                out.push((d.dimension.clone(), d.id, if d.amp >= 4 { 3.0 } else if d.amp >= 3 { 2.0 } else { 1.0 }));
            }
        }
        out
    }

    /// The fallout dose of one creature for `period` ticks at this rate (R/h, shielding applied), and the
    /// body's repair (Java: `Radiation.onPlayerTick`, `onLevelTick`).
    pub fn fallout(&mut self, uuid: u128, rate: f64, period: f64, immune: bool, now: i64) -> Vec<(u128, &'static str, i32, i32)> {
        let Some(d) = self.doses.get_mut(&uuid) else {
            if rate <= 0.0 {
                return Vec::new();
            }
            self.doses.insert(uuid, Dose { amp: -1, ..Default::default() });
            return self.fallout(uuid, rate, period, immune, now);
        };
        let got = rate * period / fallout::TICKS_PER_HOUR;
        if d.player {
            d.total += got;
        }
        if immune {
            return Vec::new();
        }
        let repair = if d.player { 0.5 * period / 10.0 } else { 2.0 * period / 40.0 };
        d.rem = (d.rem + got - repair).max(0.0);
        Self::sicken(d, uuid, now)
    }
}
