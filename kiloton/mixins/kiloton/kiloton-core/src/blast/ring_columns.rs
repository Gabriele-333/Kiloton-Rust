//! The columns of one ring of the sweep (Java: `blast.RingColumns`): those whose distance d from
//! ground zero is r <= d < r + 1. Each is packed in an `i64`, dx in the high half and dz in the low
//! one (a megaton reaches past the 32 km two shorts could hold).
//!
//! Near ground zero a ring is listed whole ([`RingColumns::add_ring`]). Far out a ring is hundreds
//! of thousands of columns and nearly all of them lie in chunks nobody has loaded, so it is listed
//! chunk by chunk, from the loaded ones only ([`RingColumns::add_chunk`]): the two give the same
//! columns.

#[derive(Clone, Debug, Default)]
pub struct RingColumns {
    pub cols: Vec<i64>,
}

pub fn pack(dx: i32, dz: i32) -> i64 {
    ((dx as i64) << 32) | (dz as i64 & 0xFFFF_FFFF)
}

pub fn dx(packed: i64) -> i32 {
    (packed >> 32) as i32
}

pub fn dz(packed: i64) -> i32 {
    packed as i32
}

/// `(int) Math.floor(Math.sqrt(v))` and `ceil` for the non-negative longs the rings use.
fn sqrt_floor(v: i64) -> i32 {
    (v as f64).sqrt().floor() as i32
}

fn sqrt_ceil(v: i64) -> i32 {
    (v as f64).sqrt().ceil() as i32
}

impl RingColumns {
    pub fn clear(&mut self) {
        self.cols.clear();
    }

    pub fn len(&self) -> usize {
        self.cols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cols.is_empty()
    }

    fn add(&mut self, dx: i32, dz: i32) {
        self.cols.push(pack(dx, dz));
    }

    /// Every column of ring `r`.
    pub fn add_ring(&mut self, r: i32) {
        let r2 = r as i64 * r as i64;
        let r12 = (r as i64 + 1) * (r as i64 + 1);
        for dx in (-r - 1)..=(r + 1) {
            let dx2 = dx as i64 * dx as i64;
            let rem = r12 - dx2;
            if rem <= 0 {
                continue;
            }
            let dz_max = sqrt_floor(rem - 1);
            let lo = r2 - dx2;
            let dz_min = if lo <= 0 { 0 } else { sqrt_ceil(lo) };
            for dz in dz_min..=dz_max {
                let d2 = dx2 + dz as i64 * dz as i64;
                if d2 < r2 || d2 >= r12 {
                    continue;
                }
                self.add(dx, dz);
                if dz != 0 {
                    self.add(dx, -dz);
                }
            }
        }
    }

    /// The columns of ring `r` that lie in chunk (cx, cz); (bx, bz) is ground zero.
    pub fn add_chunk(&mut self, bx: i32, bz: i32, cx: i32, cz: i32, r: i32) {
        let x0 = (cx << 4) - bx;
        let z0 = (cz << 4) - bz;
        let z1 = z0 + 15;
        let r2 = r as i64 * r as i64;
        let r12 = (r as i64 + 1) * (r as i64 + 1);
        for dx in x0..=(x0 + 15) {
            let dx2 = dx as i64 * dx as i64;
            let rem = r12 - dx2;
            if rem <= 0 {
                continue;
            }
            let dz_max = sqrt_floor(rem - 1);
            let lo = r2 - dx2;
            let dz_min = if lo <= 0 { 0 } else { sqrt_ceil(lo) };
            // dz from dz_min to dz_max on the positive side, the mirror on the negative one (0 once).
            for dz in dz_min.max(z0)..=dz_max.min(z1) {
                let d2 = dx2 + dz as i64 * dz as i64;
                if d2 >= r2 && d2 < r12 {
                    self.add(dx, dz);
                }
            }
            for dz in (-dz_max).max(z0)..=(-dz_min).min(z1) {
                if dz == 0 {
                    continue;
                }
                let d2 = dx2 + dz as i64 * dz as i64;
                if d2 >= r2 && d2 < r12 {
                    self.add(dx, dz);
                }
            }
        }
    }

    /// Whether ring `r` crosses chunk (cx, cz): the chunk's nearest column is nearer than r + 1 and
    /// its farthest at least r.
    pub fn crosses(bx: i32, bz: i32, cx: i32, cz: i32, r: i32) -> bool {
        let x0 = (cx << 4) - bx;
        let x1 = x0 + 15;
        let z0 = (cz << 4) - bz;
        let z1 = z0 + 15;
        let nx = if x0 > 0 { x0 as f64 } else if x1 < 0 { -x1 as f64 } else { 0.0 };
        let nz = if z0 > 0 { z0 as f64 } else if z1 < 0 { -z1 as f64 } else { 0.0 };
        let fx = x0.abs().max(x1.abs()) as f64;
        let fz = z0.abs().max(z1.abs()) as f64;
        nx * nx + nz * nz < (r as f64 + 1.0) * (r as f64 + 1.0) && fx * fx + fz * fz >= r as f64 * r as f64
    }
}
