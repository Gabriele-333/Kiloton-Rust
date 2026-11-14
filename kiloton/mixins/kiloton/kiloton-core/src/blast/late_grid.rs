//! One bit per chunk on the square of chunks around ground zero (Java: `blast.LateGrid`). A blast
//! that reaches kilometres can touch tens of thousands of chunks that are not loaded; remembering
//! each one costs a single bit here (a 4 km blast fits in 32 KB), so nothing has to be left out.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LateGrid {
    /// Chunk coordinates of ground zero.
    pub cx0: i32,
    pub cz0: i32,
    /// Half side of the square, in chunks (-1: the empty grid).
    pub r: i32,
    side: i32,
    bits: Vec<i64>,
    count: i32,
}

impl LateGrid {
    pub fn new(cx0: i32, cz0: i32, r: i32) -> Self {
        let side = if r < 0 { 0 } else { 2 * r + 1 };
        let words = ((side as i64 * side as i64 + 63) >> 6) as usize;
        Self { cx0, cz0, r, side, bits: vec![0; words], count: 0 }
    }

    pub fn empty() -> Self {
        Self::new(0, 0, -1)
    }

    /// A grid that covers every chunk a blast of this block radius can touch.
    pub fn for_radius(bx: i32, bz: i32, radius: i32) -> Self {
        Self::new(bx >> 4, bz >> 4, (radius >> 4) + 2)
    }

    pub fn load(cx0: i32, cz0: i32, r: i32, saved: &[i64]) -> Self {
        if r < 0 {
            return Self::empty();
        }
        let mut g = Self::new(cx0, cz0, r);
        let n = saved.len().min(g.bits.len());
        g.bits[..n].copy_from_slice(&saved[..n]);
        g.count = g.bits.iter().map(|w| w.count_ones() as i32).sum();
        g
    }

    /// Bit index of a chunk, or -1 outside the grid.
    pub fn index(&self, cx: i32, cz: i32) -> i32 {
        let i = cx - self.cx0 + self.r;
        let j = cz - self.cz0 + self.r;
        if i < 0 || j < 0 || i >= self.side || j >= self.side {
            return -1;
        }
        i * self.side + j
    }

    pub fn chunk_x(&self, index: i32) -> i32 {
        index / self.side - self.r + self.cx0
    }

    pub fn chunk_z(&self, index: i32) -> i32 {
        index % self.side - self.r + self.cz0
    }

    pub fn get(&self, index: i32) -> bool {
        index >= 0 && (self.bits[(index >> 6) as usize] & (1i64 << (index & 63))) != 0
    }

    /// True if the bit was not set before.
    pub fn set(&mut self, index: i32) -> bool {
        if index < 0 {
            return false;
        }
        let m = 1i64 << (index & 63);
        let w = &mut self.bits[(index >> 6) as usize];
        if *w & m != 0 {
            return false;
        }
        *w |= m;
        self.count += 1;
        true
    }

    /// True if the bit was set.
    pub fn clear(&mut self, index: i32) -> bool {
        if index < 0 {
            return false;
        }
        let m = 1i64 << (index & 63);
        let w = &mut self.bits[(index >> 6) as usize];
        if *w & m == 0 {
            return false;
        }
        *w &= !m;
        self.count -= 1;
        true
    }

    /// Sets the chunks cz_from ..= cz_to of the row cx, as far as they lie on the grid: whole words
    /// at a time.
    pub fn set_row(&mut self, cx: i32, cz_from: i32, cz_to: i32) {
        let i = cx - self.cx0 + self.r;
        if i < 0 || i >= self.side {
            return;
        }
        let a = (cz_from - self.cz0 + self.r).max(0);
        let b = (cz_to - self.cz0 + self.r).min(self.side - 1);
        if a > b {
            return;
        }
        let from = i as i64 * self.side as i64 + a as i64;
        let to = i as i64 * self.side as i64 + b as i64;
        let (w0, w1) = ((from >> 6) as usize, (to >> 6) as usize);
        for w in w0..=w1 {
            let mut m: i64 = -1;
            if w == w0 {
                m &= -1i64 << (from & 63);
            }
            if w == w1 {
                m &= ((-1i64) as u64 >> (63 - (to & 63))) as i64;
            }
            let old = self.bits[w];
            self.bits[w] = old | m;
            self.count += (m & !old).count_ones() as i32;
        }
    }

    pub fn count(&self) -> i32 {
        self.count
    }

    /// Next set bit at or after `from`, or -1.
    pub fn next(&self, from: i32) -> i32 {
        let from = from.max(0);
        let mut w = (from >> 6) as usize;
        if w >= self.bits.len() {
            return -1;
        }
        let mut word = self.bits[w] & (-1i64 << (from & 63));
        loop {
            if word != 0 {
                return ((w as i32) << 6) + word.trailing_zeros() as i32;
            }
            w += 1;
            if w >= self.bits.len() {
                return -1;
            }
            word = self.bits[w];
        }
    }

    pub fn bits(&self) -> Vec<i64> {
        self.bits.clone()
    }

    /// Bytes the grid takes in memory and on disk.
    pub fn bytes(&self) -> usize {
        self.bits.len() * 8
    }
}
