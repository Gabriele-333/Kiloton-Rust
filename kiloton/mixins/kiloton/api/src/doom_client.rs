//! The death scene as the doomed player's client shows it (Java: `ClientDoom`, with the camera fixed on
//! 11-11): the camera leaves the body and looks back at it from the side of its shadow, the fireball
//! behind. It only ever moves along a line from the eye that is clear of blocks, so it never ends up in
//! a ceiling or on the other side of one; the height is chosen once a tick and changes smoothly; the
//! scene keeps its own clock (the level's time goes back while the server is busy with the blast).
//! Not here yet: the body glowing white and turning to ash, and the nuclear shadow on the ground.

use horizon_client::mod_api::Host;
use std::collections::HashMap;
use std::sync::Mutex;

/// Seconds after the flash: the camera has pulled out.
const PULL_OUT: f64 = 2.4;
const MARGIN: f64 = 0.2;
const BOX: f64 = 0.15;
const REACH_SPEED: f64 = 2.0;
const LIFT_SPEED: f64 = 1.5;
const LOOK_AHEAD: f64 = 1.0;

struct Scene {
    fireball: [f64; 3],
    origin: [f64; 3],
    clock: i64,
    ticks: i64,
    want_lift: Option<f64>,
    lift: Option<f64>,
    reach: f64,
    last_t: f64,
    lift_tick: i64,
}

static SCENE: Mutex<Option<Scene>> = Mutex::new(None);

/// The server's `kiloton:doom` message, sent to the doomed player only: entity id, fireball, ticks, shadow.
pub fn start(host: &dyn Host, data: &[u8]) {
    if data.len() < 33 {
        return;
    }
    let f = |o: usize| f64::from_le_bytes(data[o..o + 8].try_into().unwrap());
    let ticks = i32::from_le_bytes(data[28..32].try_into().unwrap()) as i64;
    let Some(p) = host.player() else { return };
    let mut g = SCENE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        log::info!("Kiloton: death scene for the local player, {} ticks", ticks);
        *g = Some(Scene {
            fireball: [f(4), f(12), f(20)],
            origin: [p.x, p.y, p.z],
            clock: 0,
            ticks,
            want_lift: None,
            lift: None,
            reach: -1.0,
            last_t: -1.0,
            lift_tick: i64::MIN,
        });
    }
}

/// Each client tick: the scene's own clock; it ends once the player has respawned somewhere else.
pub fn tick(host: &dyn Host) {
    let mut g = SCENE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = g.as_mut() else { return };
    s.clock += 1;
    let t = s.clock as f64 / 20.0;
    let moved = host.player().is_none_or(|p| {
        let d = [p.x - s.origin[0], p.y - s.origin[1], p.z - s.origin[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() > 3.0
    });
    if (t > s.ticks as f64 / 20.0 + 1.0 && moved) || t > 600.0 {
        *g = None;
    }
}

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The blocks' collision boxes, looked up once each per frame.
struct Blocks<'a> {
    host: &'a dyn Host,
    cache: HashMap<[i32; 3], Vec<[f64; 6]>>,
}

impl Blocks<'_> {
    fn solid_at(&mut self, p: [f64; 3]) -> bool {
        let c = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
        let host = self.host;
        let boxes = self.cache.entry(c).or_insert_with(|| host.block_collision(c));
        let l = [p[0] - c[0] as f64, p[1] - c[1] as f64, p[2] - c[2] as f64];
        boxes.iter().any(|b| l[0] >= b[0] && l[0] <= b[3] && l[1] >= b[1] && l[1] <= b[4] && l[2] >= b[2] && l[2] <= b[5])
    }

    /// How far a ray from `from` along `dir` (unit) gets before it touches a block, up to `max`.
    fn ray(&mut self, from: [f64; 3], dir: [f64; 3], max: f64) -> f64 {
        let mut along = 0.0;
        while along < max {
            let p = [from[0] + dir[0] * along, from[1] + dir[1] * along, from[2] + dir[2] * along];
            if self.solid_at(p) {
                return along;
            }
            along += 0.1;
        }
        max
    }

    /// How far the camera's box can go: nine rays, the centre and the corners of the box.
    fn clearance(&mut self, from: [f64; 3], dir: [f64; 3], max: f64) -> f64 {
        let mut free = max;
        for i in 0..9 {
            if free <= 0.0 {
                break;
            }
            let a = if i == 8 {
                from
            } else {
                [
                    from[0] + if i & 1 == 0 { -BOX } else { BOX },
                    from[1] + if i & 2 == 0 { -BOX } else { BOX },
                    from[2] + if i & 4 == 0 { -BOX } else { BOX },
                ]
            };
            free = free.min(self.ray(a, dir, free));
        }
        free
    }

    fn reach_fraction(&mut self, eye: [f64; 3], ground: [f64; 3], lift: f64) -> f64 {
        let path = [ground[0] - eye[0], lift, ground[2] - eye[2]];
        let len = (path[0] * path[0] + path[1] * path[1] + path[2] * path[2]).sqrt();
        let dir = [path[0] / len, path[1] / len, path[2] / len];
        (self.clearance(eye, dir, len + MARGIN) - MARGIN) / len
    }

    fn reach_both(&mut self, eye: [f64; 3], ground: [f64; 3], later: [f64; 3], lift: f64) -> f64 {
        self.reach_fraction(eye, ground, lift).min(self.reach_fraction(eye, later, lift))
    }

    /// The height of the shot above the eye: the designed one if the camera gets all the way there;
    /// otherwise the nearest one that does, keeping the last while it still works.
    fn choose_lift(&mut self, eye: [f64; 3], ground: [f64; 3], later: [f64; 3], design: f64, last: Option<f64>) -> f64 {
        let full = self.reach_both(eye, ground, later, design);
        if full >= 1.0 {
            return design;
        }
        let last_free = last.map_or(-1.0, |l| self.reach_both(eye, ground, later, l));
        if last_free >= 1.0 {
            return last.unwrap();
        }
        let (mut best, mut best_free) = (design, full);
        for n in 1..=24 {
            for sign in [-1.0, 1.0] {
                let lift = design + sign * 0.25 * n as f64;
                if lift < 0.0 {
                    continue;
                }
                let free = self.reach_both(eye, ground, later, lift);
                if free >= 1.0 {
                    return lift;
                }
                if free > best_free + 0.05 {
                    best_free = free;
                    best = lift;
                }
            }
        }
        match last {
            Some(l) if last_free >= 0.0 && last_free >= best_free - 0.05 => l,
            _ => best,
        }
    }
}

/// The camera for this frame while the scene runs: position, yaw, pitch.
pub fn camera(host: &dyn Host, partial: f32) -> Option<([f64; 3], f32, f32)> {
    let mut g = SCENE.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.as_mut()?;
    let t = (s.clock as f64 + partial as f64) / 20.0;
    let o = s.origin;
    let mut f = [s.fireball[0] - o[0], 0.0, s.fireball[2] - o[2]];
    let fl = (f[0] * f[0] + f[2] * f[2]).sqrt();
    f = if fl < 1.0e-3 { [1.0, 0.0, 0.0] } else { [f[0] / fl, 0.0, f[2] / fl] };
    let side = [-f[2], 0.0, f[0]];
    let eye = [o[0], o[1] + 1.62, o[2]];
    let at = |k: f64| {
        let d = (k / 8.0).min(1.0);
        [o[0] + f[0] * (-5.0 - 1.4 * d) + side[0] * (2.0 - 0.4 * d), o[1], o[2] + f[2] * (-5.0 - 1.4 * d) + side[2] * (2.0 - 0.4 * d)]
    };
    let drift = (t / 8.0).min(1.0);
    let ground = at(t);
    let later = at(t + LOOK_AHEAD);
    let design = 2.4 + 0.6 * drift - 1.62;
    let mut blocks = Blocks { host, cache: HashMap::new() };
    if s.clock != s.lift_tick {
        s.lift_tick = s.clock;
        s.want_lift = Some(blocks.choose_lift(eye, ground, later, design, s.want_lift));
    }
    let want = s.want_lift.unwrap_or(design);
    let dt = if s.last_t < 0.0 { 0.0 } else { (t - s.last_t).clamp(0.0, 0.25) };
    s.last_t = t;
    let lift = match s.lift {
        None => want,
        Some(l) => l + (want - l).clamp(-LIFT_SPEED * dt, LIFT_SPEED * dt),
    };
    s.lift = Some(lift);
    let path = [ground[0] - eye[0], lift, ground[2] - eye[2]];
    let len = (path[0] * path[0] + path[1] * path[1] + path[2] * path[2]).sqrt();
    let dir = [path[0] / len, path[1] / len, path[2] / len];
    let free = (blocks.clearance(eye, dir, len + MARGIN) - MARGIN).clamp(0.0, len);
    s.reach = if s.reach < 0.0 || free < s.reach { free } else { free.min(s.reach + REACH_SPEED * dt) };
    let k = smooth(t / PULL_OUT);
    let out = (k * len).min(s.reach);
    let pos = [eye[0] + dir[0] * out, eye[1] + dir[1] * out, eye[2] + dir[2] * out];
    // From looking out towards the fireball to looking back at the body.
    let ahead = [eye[0] + f[0] * 6.0, eye[1], eye[2] + f[2] * 6.0];
    let body = [o[0], o[1] + 0.8, o[2]];
    let look = [ahead[0] + (body[0] - ahead[0]) * k, ahead[1] + (body[1] - ahead[1]) * k, ahead[2] + (body[2] - ahead[2]) * k];
    let d = [look[0] - pos[0], look[1] - pos[1], look[2] - pos[2]];
    let h = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let yaw = (-d[0]).atan2(d[2]).to_degrees() as f32;
    let pitch = -(d[1].atan2(h).to_degrees()) as f32;
    Some((pos, yaw, pitch))
}
