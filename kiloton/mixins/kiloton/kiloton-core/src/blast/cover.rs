//! What stands between a burst and a creature (Java: `BlastCover`, `Shelter`, `BlastJob.transmission`):
//! the flash's line of sight, the blast wave getting in only through the air, the prompt radiation
//! through walls by their thickness, and closed shelters with their blast rating and protection
//! factor. Pure algorithms over a [`CoverWorld`] and a per-state [`CoverInfo`] table.

use std::collections::{HashMap, HashSet, VecDeque};

/// What the cover algorithms need of one block state.
#[derive(Clone, Debug, Default)]
pub struct CoverInfo {
    pub air: bool,
    /// Closes a shelter: a full solid block, a shut door or trapdoor, a still liquid.
    pub seals: bool,
    /// Halving thicknesses of fallout gamma in one metre (Java: `Shelter.halvings`).
    pub halvings: f64,
    /// Overpressure that breaks it, psi (Java: `Shelter.strength`).
    pub strength: f64,
    /// Blocks light where the line runs through its shape (`canOcclude`).
    pub occludes: bool,
    pub leaves: bool,
    /// Its outline as block-local boxes; empty means the whole cell.
    pub boxes: Vec<[f64; 6]>,
}

pub trait CoverWorld {
    fn loaded(&self, x: i32, z: i32) -> bool;
    fn state(&self, x: i32, y: i32, z: i32) -> u32;
    /// `Level.canSeeSky`: nothing but air above.
    fn sky(&self, x: i32, y: i32, z: i32) -> bool;
    fn min_y(&self) -> i32;
    /// One past the highest block y.
    fn max_y(&self) -> i32;
    /// The y of the column's highest block that isn't air, plus one (`WORLD_SURFACE`).
    fn surface(&self, x: i32, z: i32) -> i32;
}

/// Overpressure kept past a right-angle turn of a passage.
pub const PER_TURN: f64 = 0.9;
/// Length over which the overpressure falls to 1/e along a passage, blocks (an estimate).
pub const DECAY: f64 = 200.0;
/// Part of the prompt radiation that reaches the ground scattered by the air, from the sky above.
pub const SCATTERED: f64 = 0.3;
/// How many times thicker a halving layer is for prompt radiation than for fallout gamma (estimate).
pub const PROMPT_HARDER: f64 = 2.5;
pub const UNBREAKABLE: f64 = 1.0e6;
const PLAYER_CELLS: usize = 24_000;
const PLAYER_STEPS: usize = 256;
const MOB_CELLS: usize = 600;
const MOB_STEPS: usize = 32;
const RAY: usize = 16;
const DIRS: [(i32, i32, i32); 6] = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];

pub struct Cover<'a, W: CoverWorld> {
    pub world: &'a W,
    pub table: &'a [CoverInfo],
}

/// How the wave gets to someone under cover.
#[derive(Clone, Debug)]
pub struct Wave {
    /// No way out within the search: a shelter of `volume` cells of air.
    pub closed: bool,
    pub volume: usize,
    /// The factor its overpressure keeps.
    pub factor: f64,
    /// The opening it comes in by (None beyond the search).
    pub opening: Option<[i32; 3]>,
    /// The way it travels when it gets there (None: from the burst).
    pub direction: Option<[f64; 3]>,
}

#[derive(Clone, Debug)]
pub struct ShelterResult {
    pub sealed: bool,
    pub volume: usize,
    pub rating_psi: f64,
    pub protection: f64,
}

fn key(x: i32, y: i32, z: i32) -> (i32, i32, i32) {
    (x, y, z)
}

impl<'a, W: CoverWorld> Cover<'a, W> {
    fn info(&self, x: i32, y: i32, z: i32) -> &CoverInfo {
        static AIR: std::sync::OnceLock<CoverInfo> = std::sync::OnceLock::new();
        let s = self.world.state(x, y, z) as usize;
        self.table.get(s).unwrap_or_else(|| AIR.get_or_init(|| CoverInfo { air: true, ..Default::default() }))
    }

    fn outside(&self, y: i32) -> bool {
        y < self.world.min_y() || y >= self.world.max_y()
    }

    fn seals(&self, x: i32, y: i32, z: i32) -> bool {
        !self.outside(y) && self.info(x, y, z).seals
    }

    /// How much of the fireball's light reaches `from` (Java: `BlastJob.transmission`): a solid block stops it
    /// where the line runs through its shape, foliage lets half through per block. Unloaded land is air.
    pub fn transmission(&self, from: [f64; 3], to: [f64; 3]) -> f64 {
        let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist < 2.0 {
            return 1.0;
        }
        let u = [d[0] / dist, d[1] / dist, d[2] / dist];
        let mut t = 1.0;
        let near = (dist / 0.5).min(48.0) as i32;
        let steps = near + ((dist - near as f64 * 0.5).max(0.0) / 2.0).min(800.0) as i32;
        for i in 1..steps {
            let along = if i <= near { i as f64 * 0.5 } else { near as f64 * 0.5 + (i - near) as f64 * 2.0 };
            let (px, py, pz) = (from[0] + u[0] * along, from[1] + u[1] * along, from[2] + u[2] * along);
            let (x, y, z) = (px.floor() as i32, py.floor() as i32, pz.floor() as i32);
            if self.outside(y) || !self.world.loaded(x, z) {
                continue;
            }
            let s = self.info(x, y, z);
            if s.air {
                continue;
            }
            if s.leaves {
                t *= 0.5;
                if t < 0.01 {
                    return 0.0;
                }
            } else if s.occludes && inside(s, px - x as f64, py - y as f64, pz - z as f64) {
                return 0.0;
            }
        }
        t
    }

    /// None out in the open (under the sky, or a step from it); otherwise how the wave gets in (Java:
    /// `BlastCover.wave`).
    pub fn wave(&self, eye: [i32; 3], player: bool) -> Option<Wave> {
        let start = if self.seals(eye[0], eye[1], eye[2]) { [eye[0], eye[1] + 1, eye[2]] } else { eye };
        if !self.world.loaded(start[0], start[2]) || self.world.sky(start[0], start[1], start[2]) {
            return None;
        }
        let (max_cells, max_steps) = if player { (PLAYER_CELLS, PLAYER_STEPS) } else { (MOB_CELLS, MOB_STEPS) };
        let s0 = key(start[0], start[1], start[2]);
        let mut parent: HashMap<(i32, i32, i32), (i32, i32, i32)> = HashMap::new();
        parent.insert(s0, s0);
        let mut frontier = vec![s0];
        let mut steps = 0usize;
        while !frontier.is_empty() {
            if steps >= max_steps || parent.len() > max_cells {
                return Some(Wave {
                    closed: false,
                    volume: parent.len(),
                    factor: (-(steps as f64) / DECAY).exp(),
                    opening: None,
                    direction: None,
                });
            }
            let mut next = Vec::new();
            for &c in &frontier {
                for (dx, dy, dz) in DIRS {
                    let n = (c.0 + dx, c.1 + dy, c.2 + dz);
                    if parent.contains_key(&n) || self.outside(n.1) {
                        continue;
                    }
                    let loaded = self.world.loaded(n.0, n.2);
                    if loaded && self.info(n.0, n.1, n.2).seals {
                        continue;
                    }
                    parent.insert(n, c);
                    if !loaded || self.world.sky(n.0, n.1, n.2) {
                        let mut path = vec![n];
                        let mut k = n;
                        while k != s0 {
                            k = parent[&k];
                            path.push(k);
                        }
                        path.reverse();
                        return Some(self.through(&path));
                    }
                    next.push(n);
                }
            }
            frontier = next;
            steps += 1;
        }
        Some(Wave { closed: true, volume: parent.len(), factor: 0.0, opening: None, direction: None })
    }

    /// The passage pulled tight; its turns and length weaken the wave.
    fn through(&self, path: &[(i32, i32, i32)]) -> Wave {
        let centre = |c: (i32, i32, i32)| [c.0 as f64 + 0.5, c.1 as f64 + 0.5, c.2 as f64 + 0.5];
        let mut corners = vec![centre(path[0])];
        let mut anchor = 0;
        for i in 1..path.len() {
            if i - anchor > 1 && !self.clear(centre(path[anchor]), centre(path[i])) {
                anchor = i - 1;
                corners.push(centre(path[anchor]));
            }
        }
        corners.push(centre(path[path.len() - 1]));
        let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let len = |a: [f64; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        let (mut length, mut turns) = (0.0, 0.0);
        for i in 1..corners.len() {
            let a = sub(corners[i], corners[i - 1]);
            length += len(a);
            if i + 1 < corners.len() {
                let b = sub(corners[i + 1], corners[i]);
                let cos = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (len(a) * len(b)).max(1.0e-9);
                turns += cos.clamp(-1.0, 1.0).acos().to_degrees() / 90.0;
            }
        }
        let d = sub(corners[0], corners[1]);
        let l = len(d);
        let direction = if l * l < 1.0e-6 { None } else { Some([d[0] / l, d[1] / l, d[2] / l]) };
        let last = path[path.len() - 1];
        Wave {
            closed: false,
            volume: 0,
            factor: PER_TURN.powf(turns) * (-length / DECAY).exp(),
            opening: Some([last.0, last.1, last.2]),
            direction,
        }
    }

    fn clear(&self, a: [f64; 3], b: [f64; 3]) -> bool {
        let d = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        let samples = (d / 0.25).ceil() as i32;
        for s in 1..samples {
            let t = s as f64 / samples as f64;
            let (x, y, z) = (
                (a[0] + (b[0] - a[0]) * t).floor() as i32,
                (a[1] + (b[1] - a[1]) * t).floor() as i32,
                (a[2] + (b[2] - a[2]) * t).floor() as i32,
            );
            if !self.world.loaded(x, z) || self.seals(x, y, z) {
                return false;
            }
        }
        true
    }

    /// The part of the prompt radiation that reaches `eye` from the fireball (Java: `BlastCover.prompt`).
    pub fn prompt(&self, eye: [f64; 3], fireball: [f64; 3]) -> f64 {
        let direct = 2f64.powf(-self.halvings_along(eye, fireball).min(60.0));
        let scattered = 2f64.powf(-self.halvings_above(eye).min(60.0));
        (1.0 - SCATTERED) * direct + SCATTERED * scattered
    }

    fn halvings_along(&self, from: [f64; 3], to: [f64; 3]) -> f64 {
        let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if dist < 1.0 {
            return 0.0;
        }
        let u = [d[0] / dist, d[1] / dist, d[2] / dist];
        let (mut h, mut along) = (0.0, 0.0);
        let far = ((dist - 24.0) / 1000.0).max(2.0);
        while along < dist {
            let step = if along < 24.0 { 0.5 } else { far };
            along += step;
            let (x, y, z) =
                ((from[0] + u[0] * along).floor() as i32, (from[1] + u[1] * along).floor() as i32, (from[2] + u[2] * along).floor() as i32);
            if self.outside(y) || !self.world.loaded(x, z) {
                continue;
            }
            let s = self.info(x, y, z);
            if s.air {
                continue;
            }
            h += s.halvings / PROMPT_HARDER * step;
            if h > 60.0 {
                break;
            }
        }
        h
    }

    fn halvings_above(&self, eye: [f64; 3]) -> f64 {
        let (x, z) = (eye[0].floor() as i32, eye[2].floor() as i32);
        if !self.world.loaded(x, z) {
            return 0.0;
        }
        let top = self.world.surface(x, z);
        let mut h = 0.0;
        let mut y = eye[1].floor() as i32 + 1;
        while y < top && h <= 60.0 {
            let s = self.info(x, y, z);
            if !s.air {
                h += s.halvings / PROMPT_HARDER;
            }
            y += 1;
        }
        h
    }

    /// Fills the air from `start`; a shelter if it closes within `max` cells (Java: `Shelter.scan`).
    pub fn shelter(&self, start: [i32; 3], max: usize) -> ShelterResult {
        let open = |volume| ShelterResult { sealed: false, volume, rating_psi: 0.0, protection: 1.0 };
        let start = if self.seals(start[0], start[1], start[2]) { [start[0], start[1] + 1, start[2]] } else { start };
        let s0 = key(start[0], start[1], start[2]);
        let mut visited: HashSet<(i32, i32, i32)> = HashSet::new();
        visited.insert(s0);
        let mut queue = VecDeque::from([s0]);
        let mut faces: Vec<((i32, i32, i32), usize)> = Vec::new();
        while let Some(p) = queue.pop_front() {
            if self.outside(p.1) || !self.world.loaded(p.0, p.2) {
                return open(visited.len());
            }
            for (di, (dx, dy, dz)) in DIRS.iter().enumerate() {
                let n = (p.0 + dx, p.1 + dy, p.2 + dz);
                if visited.contains(&n) {
                    continue;
                }
                if self.seals(n.0, n.1, n.2) {
                    faces.push((p, di));
                    continue;
                }
                visited.insert(n);
                if visited.len() > max {
                    return open(visited.len());
                }
                queue.push_back(n);
            }
        }
        if faces.is_empty() {
            return open(visited.len());
        }
        let (mut rating, mut pf) = (f64::MAX, f64::MAX);
        for (c, di) in faces {
            let (dx, dy, dz) = DIRS[di];
            let (mut best, mut halv) = (0.0f64, 0.0f64);
            let mut m = c;
            for _ in 0..RAY {
                m = (m.0 + dx, m.1 + dy, m.2 + dz);
                if !self.seals(m.0, m.1, m.2) {
                    break;
                }
                let s = self.info(m.0, m.1, m.2);
                best = best.max(s.strength);
                halv += s.halvings;
            }
            rating = rating.min(best);
            pf = pf.min(2f64.powf(halv.min(40.0)));
        }
        ShelterResult { sealed: true, volume: visited.len(), rating_psi: rating, protection: pf.max(1.0) }
    }
}

/// Whether a block-local point lies inside the block's own shape.
fn inside(s: &CoverInfo, fx: f64, fy: f64, fz: f64) -> bool {
    if s.boxes.is_empty() {
        return true;
    }
    s.boxes.iter().any(|b| fx >= b[0] && fx < b[3] && fy >= b[1] && fy < b[4] && fz >= b[2] && fz < b[5])
}

/// Fallout halving thicknesses in one metre of a block of this category (Java: `Shelter.halvings`).
pub fn halvings(category: u8, metal: bool) -> f64 {
    use crate::blast::profile::*;
    match category {
        TERRAIN => 12.0,
        STRUCTURE => {
            if metal {
                40.0
            } else {
                14.0
            }
        }
        INDESTRUCTIBLE => 20.0,
        FLUID | ICE => 8.0,
        WOOD | LOG => 4.5,
        GLASS => 3.0,
        LEAVES | PLANT => 0.3,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat world: stone below y 64, air above, with a few blocks set.
    struct Flat(HashMap<(i32, i32, i32), u32>);

    impl CoverWorld for Flat {
        fn loaded(&self, _x: i32, _z: i32) -> bool {
            true
        }
        fn state(&self, x: i32, y: i32, z: i32) -> u32 {
            if let Some(&s) = self.0.get(&(x, y, z)) {
                return s;
            }
            if y < 64 { 1 } else { 0 }
        }
        fn sky(&self, x: i32, y: i32, z: i32) -> bool {
            y >= self.surface(x, z)
        }
        fn min_y(&self) -> i32 {
            -64
        }
        fn max_y(&self) -> i32 {
            320
        }
        fn surface(&self, x: i32, z: i32) -> i32 {
            let mut top = 64;
            for (&(bx, by, bz), &s) in &self.0 {
                if bx == x && bz == z && s != 0 {
                    top = top.max(by + 1);
                }
            }
            top
        }
    }

    fn table() -> Vec<CoverInfo> {
        let air = CoverInfo { air: true, ..Default::default() };
        let stone = CoverInfo { seals: true, halvings: 12.0, strength: UNBREAKABLE * 10.0, occludes: true, ..Default::default() };
        vec![air, stone]
    }

    #[test]
    fn closed_room_is_a_shelter_and_open_ground_is_not() {
        // A 3x3x3 room dug in the stone, 10 blocks down.
        let mut m = HashMap::new();
        for x in 0..3 {
            for y in 50..53 {
                for z in 0..3 {
                    m.insert((x, y, z), 0);
                }
            }
        }
        let w = Flat(m);
        let t = table();
        let c = Cover { world: &w, table: &t };
        let s = c.shelter([1, 51, 1], 2048);
        assert!(s.sealed && s.volume == 27 && s.protection > 1.0e6);
        let wave = c.wave([1, 51, 1], true).unwrap();
        assert!(wave.closed);
        assert!(c.wave([1, 70, 1], true).is_none(), "out in the open");
        assert_eq!(c.transmission([1.5, 51.5, 1.5], [200.5, 120.0, 1.5]), 0.0, "the rock hides the fireball");
        assert_eq!(c.transmission([1.5, 70.5, 1.5], [200.5, 120.0, 1.5]), 1.0);
        assert!(c.prompt([1.5, 51.5, 1.5], [200.5, 120.0, 1.5]) < 1.0e-6);
    }
}
