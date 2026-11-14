//! The device's panel on the client (Java: `BombScreen`, `BombMenu`'s layout, `Canvas`). Left: the implosion
//! assembly in section (or the two stages of the thermonuclear casing) and the inventory. Right: a phosphor readout
//! with the yield and what it does, the timer with its buttons, and the arming switch under a safety cover: click the
//! cover to lift it, then press the button. The arming code sits in the plate's corner; clicking it draws a new one.
//!
//! The panel reads the device from the menu's data slots (`bomb::data`), so it draws the same thing while the
//! device is armed and its load is out of reach.

use crate::bomb::{self, Load, B_ARM, B_COVER, B_DISARM, B_MINUS_10, B_MINUS_60, B_NEW_CODE, B_PLUS_10, B_PLUS_60};
use horizon_client::mod_api::{Host, HudBatch, NativeScreen};
use kiloton_core::{BlastModel, FissionModel, FissionState, ThermonuclearState, Warhead};
use std::sync::atomic::{AtomicBool, Ordering};

pub const W: i32 = 330;
pub const H: i32 = 208;
const BG: &str = "kiloton:gui/nuclear_bomb";
const BG_H: &str = "kiloton:gui/thermonuclear_bomb";
// Item positions of the slots (Java: `BombMenu`).
const PIT: [[i32; 2]; 6] = [[83, 41], [102, 52], [102, 74], [83, 85], [64, 74], [64, 52]];
const TAMPER: [i32; 2] = [15, 29];
const INV: [i32; 2] = [10, 126];
const H_PIT: [[i32; 2]; 6] = [[28, 41], [46, 41], [28, 59], [46, 59], [28, 77], [46, 77]];
const H_TAMPER: [i32; 2] = [10, 102];
const H_FUEL: [[i32; 2]; 6] = [[96, 41], [114, 41], [132, 41], [96, 77], [114, 77], [132, 77]];
const H_SPARK: [i32; 2] = [114, 59];
const H_JACKET: [i32; 2] = [152, 59];
const H_PX: f64 = 45.0;
const H_PY: f64 = 67.0;
const CX: f64 = 91.0;
const CY: f64 = 71.0;
// Readout
const CRT_X: i32 = 184;
const CRT_Y: i32 = 21;
// Timer display and buttons
const TIMER_X0: i32 = 220;
const TIMER_Y0: i32 = 124;
const TIMER_X1: i32 = 286;
const TIMER_Y1: i32 = 147;
const TIMER_BUTTONS: [[i32; 2]; 4] = [[185, 124], [288, 124], [185, 137], [288, 137]];
const TIMER_IDS: [i32; 4] = [B_MINUS_10, B_PLUS_10, B_MINUS_60, B_PLUS_60];
const TIMER_LABELS: [&str; 4] = ["-10s", "+10s", "-1m", "+1m"];
// Arming well, the button in it, the info box, the disarm button
const WELL_X0: i32 = 187;
const WELL_Y0: i32 = 158;
const WELL_X1: i32 = 227;
const WELL_Y1: i32 = 202;
const BTN_X: f64 = 207.0;
const BTN_Y: f64 = 180.0;
const INFO_X: i32 = 237;
const INFO_Y: i32 = 161;
const INFO_W: i32 = 83;
const DIS_X0: i32 = 237;
const DIS_Y0: i32 = 187;
const DIS_X1: i32 = 319;
const DIS_Y1: i32 = 199;
// Arming code, bottom right of the plate
const CODE_X0: i32 = 130;
const CODE_Y0: i32 = 101;
const CODE_X1: i32 = 173;
const CODE_Y1: i32 = 120;

const GREEN: u32 = 0xFF7CFF8A;
const GREEN_DIM: u32 = 0xFF3C8A4A;
const AMBER: u32 = 0xFFFFB020;
const RED: u32 = 0xFFFF4A3A;
const GREY: u32 = 0xFF8A949E;
const STENCIL: u32 = 0xFFE0A526;
const BLUEPRINT: u32 = 0xFF6F97C0;

/// The safety cover (the panel's own: Java keeps it in the screen).
static COVER_OPEN: AtomicBool = AtomicBool::new(false);

fn thermo(block: &str) -> bool {
    block == bomb::THERMO
}

/// Where each container slot sits (`native_screen`): the panel's slots, never the hidden ones.
pub fn layout(block: &str) -> Option<NativeScreen> {
    if !bomb::is_bomb(block) {
        return None;
    }
    COVER_OPEN.store(false, Ordering::Relaxed);
    let pos = |p: [i32; 2]| Some([p[0] as f32, p[1] as f32]);
    let mut slots = vec![None; bomb::SLOTS];
    if thermo(block) {
        for (i, p) in H_PIT.iter().enumerate() {
            slots[i] = pos(*p);
        }
        slots[bomb::TAMPER] = pos(H_TAMPER);
        for (i, p) in H_FUEL.iter().enumerate() {
            slots[bomb::FUEL + i] = pos(*p);
        }
        slots[bomb::SPARK] = pos(H_SPARK);
        slots[bomb::JACKET] = pos(H_JACKET);
    } else {
        for (i, p) in PIT.iter().enumerate() {
            slots[i] = pos(*p);
        }
        slots[bomb::TAMPER] = pos(TAMPER);
    }
    Some(NativeScreen {
        width: W as f32,
        height: H as f32,
        slots,
        inventory_y: INV[1] as f32,
        inventory_x: INV[0] as f32,
        own_labels: true,
        own_background: true,
    })
}

fn slot_pos(block: &str, slot: usize) -> Option<[i32; 2]> {
    let t = thermo(block);
    match slot {
        s if s < bomb::PIT => Some(if t { H_PIT[s] } else { PIT[s] }),
        s if s == bomb::TAMPER => Some(if t { H_TAMPER } else { TAMPER }),
        s if t && (bomb::FUEL..bomb::SPARK).contains(&s) => Some(H_FUEL[s - bomb::FUEL]),
        s if t && s == bomb::SPARK => Some(H_SPARK),
        s if t && s == bomb::JACKET => Some(H_JACKET),
        _ => None,
    }
}

/// The device's slot under the cursor, if any.
fn slot_at(block: &str, x: f64, y: f64) -> Option<usize> {
    (0..bomb::SIZE).find(|&s| slot_pos(block, s).is_some_and(|[sx, sy]| in_rect(x, y, sx - 1, sy - 1, sx + 17, sy + 17)))
}

// ------------------------------------------------------------------------------------------------ what it shows

/// The panel's numbers, decoded.
struct View {
    thermo: bool,
    load: Load,
    timer: i32,
    armed: bool,
    remaining: i32,
    code: i32,
    cap: f64,
}

impl View {
    fn of(block: &str, d: &[i32]) -> Self {
        let at = |i: usize| d.get(i).copied().unwrap_or(0);
        let half = |i: usize| (at(i) & 0xFFFF) as u32;
        let cap = f32::from_bits((half(bomb::data::CAP_HI) << 16) | half(bomb::data::CAP_LO)) as f64;
        View {
            thermo: thermo(block),
            load: Load::from_data(d),
            timer: at(bomb::data::TIMER),
            armed: at(bomb::data::ARMED) != 0,
            remaining: at(bomb::data::REMAINING),
            code: at(bomb::data::CODE_HI) * 1000 + at(bomb::data::CODE_LO),
            cap: if cap.is_finite() && cap > 0.0 { cap } else { 150.0 },
        }
    }

    fn warhead(&self) -> Warhead {
        self.load.warhead(self.thermo)
    }

    /// The yield it goes off with, 0 if only its lenses do (Java: `yieldKt`).
    fn yield_kt(&self) -> f64 {
        let w = self.warhead();
        if w.nuclear() { w.yield_kt.min(self.cap).max(0.001) } else { 0.0 }
    }

    fn capped(&self) -> bool {
        let w = self.warhead();
        w.nuclear() && w.yield_kt.min(self.cap).max(0.001) < w.yield_kt
    }
}

pub(crate) fn tr(key: &str, args: &[&str]) -> String {
    let args: Vec<&dyn std::fmt::Display> = args.iter().map(|a| a as &dyn std::fmt::Display).collect();
    horizon_client::lang::tr(key, &args)
}

fn millis() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

fn blink(period_ms: u128) -> bool {
    (millis() / period_ms) % 2 == 0
}

pub(crate) fn in_rect(x: f64, y: f64, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    x >= x0 as f64 && x < x1 as f64 && y >= y0 as f64 && y < y1 as f64
}

// ------------------------------------------------------------------------------------------------ drawing

/// Java's `Canvas` over Horizon's GUI batch: colours ARGB, rectangles [x0, x1) x [y0, y1), positions from the panel's
/// top left.
pub(crate) struct Canvas<'a> {
    pub(crate) host: &'a dyn Host,
    pub(crate) batch: &'a mut HudBatch,
    pub(crate) ox: f32,
    pub(crate) oy: f32,
}

/// How narrow `fit` squeezes a line before it shrinks it as a whole (Java: `Canvas.SQUEEZE`).
const SQUEEZE: f32 = 0.8;
/// Tooltips break into lines this wide (Java: `Canvas.TIP_WIDTH`).
pub(crate) const TIP_WIDTH: f32 = 200.0;

impl Canvas<'_> {
    pub(crate) fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, argb: u32) {
        if x1 <= x0 || y1 <= y0 || argb >> 24 == 0 {
            return;
        }
        self.host.fill(self.batch, [self.ox + x0 as f32, self.oy + y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32], argb);
    }

    pub(crate) fn width(&self, s: &str) -> i32 {
        self.host.text_width(s).ceil() as i32
    }

    pub(crate) fn text(&mut self, s: &str, x: i32, y: i32, argb: u32) {
        self.text_f(s, x as f32, y as f32, argb);
    }

    fn text_f(&mut self, s: &str, x: f32, y: f32, argb: u32) {
        self.host.text(self.batch, s, [self.ox + x, self.oy + y], argb & 0x00FF_FFFF, false);
    }

    /// Draws `s` at (x, y) scaled by (sx, sy) about that point (Java: `push; translate; scale; text; pop`).
    fn text_scaled(&mut self, s: &str, x: f32, y: f32, sx: f32, sy: f32, argb: u32) {
        let n = self.batch.vertices.len();
        self.text_f(s, x, y, argb);
        let (ax, ay) = (self.ox + x, self.oy + y);
        for v in &mut self.batch.vertices[n..] {
            v.pos[0] = ax + (v.pos[0] - ax) * sx;
            v.pos[1] = ay + (v.pos[1] - ay) * sy;
        }
    }

    /// Text at a scale (0.75 for the small stencil labels).
    pub(crate) fn small(&mut self, s: &str, x: i32, y: i32, argb: u32, scale: f32) {
        self.text_scaled(s, x as f32, y as f32, scale, scale, argb);
    }

    /// Java: `Canvas.fit`: as it is when it fits; otherwise narrowed down to `SQUEEZE` of its width, and if even
    /// that is too wide made smaller as a whole, on the same middle line. The width it takes.
    pub(crate) fn fit(&mut self, s: &str, x: i32, y: i32, max_w: i32, argb: u32, scale: f32) -> i32 {
        let w = self.host.text_width(s) * scale;
        if w <= max_w as f32 || w <= 0.0 {
            if scale == 1.0 {
                self.text(s, x, y, argb);
            } else {
                self.small(s, x, y, argb, scale);
            }
            return w.ceil() as i32;
        }
        let k = max_w.max(1) as f32 / w;
        let sy = scale * (k / SQUEEZE).min(1.0);
        self.text_scaled(s, x as f32, y as f32 + 4.0 * (scale - sy), scale * k, sy, argb);
        max_w
    }

    fn fit_width(&self, s: &str, max_w: i32, scale: f32) -> i32 {
        max_w.min((self.host.text_width(s) * scale).ceil() as i32)
    }

    pub(crate) fn fit_right(&mut self, s: &str, right: i32, y: i32, max_w: i32, argb: u32, scale: f32) {
        let w = self.fit_width(s, max_w, scale);
        self.fit(s, right - w, y, max_w, argb, scale);
    }

    pub(crate) fn fit_centre(&mut self, s: &str, cx: i32, y: i32, max_w: i32, argb: u32, scale: f32) {
        let w = self.fit_width(s, max_w, scale);
        self.fit(s, cx - w / 2, y, max_w, argb, scale);
    }

    fn blit(&mut self, sprite: &str) {
        self.blit_size(sprite, W, H);
    }

    /// The whole texture `sprite` (w x h GUI pixels) at the panel's top left.
    pub(crate) fn blit_size(&mut self, sprite: &str, w: i32, h: i32) {
        let (w, h) = (w as f32, h as f32);
        self.host.sprite(self.batch, sprite, [0.0, 0.0, w, h], [self.ox, self.oy, w, h]);
    }

    fn item(&mut self, item: &str, at: [i32; 2]) {
        self.host.item(self.batch, item, [self.ox + at[0] as f32, self.oy + at[1] as f32], 16.0);
    }

    /// Filled circle by rows (pixel centres). Java: `BombScreen.disc`.
    fn disc(&mut self, cx: f64, cy: f64, r: f64, argb: u32) {
        let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
        for y in y0..=y1 {
            let dy = y as f64 + 0.5 - cy;
            if dy.abs() > r {
                continue;
            }
            let half = (r * r - dy * dy).sqrt();
            let (xa, xb) = (java_round(cx - half), java_round(cx + half));
            if xb > xa {
                self.fill(xa, y, xb, y + 1, argb);
            }
        }
    }

    /// Ring between two radii, by rows. Java: `BombScreen.ring`.
    fn ring(&mut self, cx: f64, cy: f64, r0: f64, r1: f64, argb: u32) {
        let (y0, y1) = ((cy - r1).floor() as i32, (cy + r1).ceil() as i32);
        for y in y0..=y1 {
            let dy = y as f64 + 0.5 - cy;
            if dy.abs() > r1 {
                continue;
            }
            let outer = (r1 * r1 - dy * dy).sqrt();
            let (xa, xd) = (java_round(cx - outer), java_round(cx + outer));
            if dy.abs() < r0 {
                let inner = (r0 * r0 - dy * dy).sqrt();
                let (xb, xc) = (java_round(cx - inner), java_round(cx + inner));
                if xb > xa {
                    self.fill(xa, y, xb, y + 1, argb);
                }
                if xd > xc {
                    self.fill(xc, y, xd, y + 1, argb);
                }
            } else if xd > xa {
                self.fill(xa, y, xd, y + 1, argb);
            }
        }
    }

    /// Draws digits (and . : -) right-aligned at `right`; unlit segments in the dim colour show the 8s behind.
    #[allow(clippy::too_many_arguments)]
    fn seven_seg_right(&mut self, s: &str, right: i32, top: i32, w: i32, h: i32, t: i32, on: u32, off: u32) {
        let gap = (t - 1).max(1) + 1;
        let mut x = right - seven_seg_width(s, w, gap);
        for ch in s.chars() {
            if ch == '.' {
                self.fill(x, top + h - t, x + t, top + h, on);
                x += (w / 4).max(2) + gap;
                continue;
            }
            if ch == ':' {
                self.fill(x, top + h / 3 - 1, x + t, top + h / 3 - 1 + t, on);
                self.fill(x, top + 2 * h / 3, x + t, top + 2 * h / 3 + t, on);
                x += (w / 4).max(2) + gap;
                continue;
            }
            let mask = if ch == '-' { 0b0000001 } else { ch.to_digit(10).map_or(0, |d| SEGMENTS[d as usize]) };
            let mid = top + (h - t) / 2;
            let seg = [
                [x + t, top, x + w - t, top + t],
                [x + w - t, top + t, x + w, mid],
                [x + w - t, mid + t, x + w, top + h - t],
                [x + t, top + h - t, x + w - t, top + h],
                [x, mid + t, x + t, top + h - t],
                [x, top + t, x + t, mid],
                [x + t, mid, x + w - t, mid + t],
            ];
            for (i, r) in seg.iter().enumerate() {
                let lit = mask & (1 << (6 - i)) != 0;
                self.fill(r[0], r[1], r[2], r[3], if lit { on } else { off });
            }
            x += w + gap;
        }
    }
}

/// `Math.round` for the circles: half up.
fn java_round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

// Seven-segment digits: segments a b c d e f g.
const SEGMENTS: [u32; 10] = [0b1111110, 0b0110000, 0b1101101, 0b1111001, 0b0110011, 0b1011011, 0b1011111, 0b1110000, 0b1111111, 0b1111011];

fn seven_seg_width(s: &str, w: i32, gap: i32) -> i32 {
    let total: i32 = s.chars().map(|ch| if ch == '.' || ch == ':' { (w / 4).max(2) + gap } else { w + gap }).sum();
    total - gap
}

/// Draw the panel (`native_screen_draw`; Java: `BombScreen.drawBack`).
pub fn draw(host: &dyn Host, block: &str, data: &[i32], batch: &mut HudBatch, origin: [f32; 2]) {
    if !bomb::is_bomb(block) {
        return;
    }
    let v = View::of(block, data);
    let cursor = host.cursor();
    let (mx, my) = ((cursor[0] - origin[0]) as f64, (cursor[1] - origin[1]) as f64);
    let mut c = Canvas { host, batch, ox: origin[0], oy: origin[1] };
    c.blit(if v.thermo { BG_H } else { BG });
    let m = v.load.model();
    header(&mut c, &v);
    if v.thermo {
        schematic_h(&mut c, &v, &m);
    } else {
        schematic(&mut c, &v, &m);
    }
    code(&mut c, &v, mx, my);
    if v.thermo {
        readout_h(&mut c, &v);
    } else {
        readout(&mut c, &v, &m);
    }
    timer(&mut c, &v, mx, my);
    arming(&mut c, &v, &m, mx, my);
    if v.armed {
        // The load is out of the panel's slots while armed (bomb.rs): it is drawn where it sits.
        load_icons(&mut c, block, &v.load);
    }
}

fn load_icons(c: &mut Canvas<'_>, block: &str, l: &Load) {
    for i in 0..bomb::PIT {
        let item = if l.uranium & (1 << i) != 0 {
            bomb::URANIUM_CORE
        } else if l.plutonium & (1 << i) != 0 {
            bomb::PLUTONIUM_CORE
        } else {
            continue;
        };
        if let Some(p) = slot_pos(block, i) {
            c.item(item, p);
        }
    }
    let mut put = |slot: usize, on: bool, item: &str| {
        if on {
            if let Some(p) = slot_pos(block, slot) {
                c.item(item, p);
            }
        }
    };
    put(bomb::TAMPER, l.tamper, bomb::URANIUM_TAMPER);
    if thermo(block) {
        for i in 0..bomb::FUEL_SLOTS {
            put(bomb::FUEL + i, l.fuel & (1 << i) != 0, bomb::LITHIUM_DEUTERIDE);
        }
        // The spark plug's own core isn't in the data slots: either core reads the same on the panel.
        put(bomb::SPARK, l.spark, bomb::PLUTONIUM_CORE);
        put(bomb::JACKET, l.jacket, bomb::URANIUM_TAMPER);
    }
}

fn header(c: &mut Canvas<'_>, v: &View) {
    let title = c.host.screen_title().unwrap_or_default().to_uppercase();
    c.text(&title, 8, 6, STENCIL);
    let s = tr(if v.armed { "gui.kiloton.bomb.status_armed" } else { "gui.kiloton.bomb.status_safe" }, &[]);
    let lamp = if v.armed { if blink(250) { 0xFFFF3020 } else { 0xFF601008 } } else { 0xFF40E060 };
    let w = c.width(&s);
    c.text(&s, 250 - w, 6, if v.armed { RED } else { GREEN });
    c.disc(255.0, 10.0, 3.6, 0xFF101010);
    c.disc(255.0, 10.0, 2.6, lamp);
}

fn pulse(period: f64, base: f64) -> u32 {
    let t = millis() as f64;
    (base + base * (t / period).sin()) as i32 as u32
}

fn schematic(c: &mut Canvas<'_>, v: &View, m: &FissionModel) {
    // Tamper shell (r 34..39): depleted uranium grey if loaded, a faint outline if not.
    if m.tamper {
        c.ring(CX, CY, 33.5, 39.5, 0xFF5E625C);
        c.ring(CX, CY, 37.5, 39.5, 0xFF7A7F78);
    } else {
        c.ring(CX, CY, 36.0, 37.2, 0xFF2E5274);
    }
    // Lenses glow while the countdown runs.
    if v.armed {
        let a = pulse(120.0, 60.0);
        c.ring(CX, CY, 40.5, 50.5, (a << 24) | 0xFF2010);
    }
    // Initiator in the middle.
    let core = if v.armed {
        if blink(200) { 0xFFFF4030 } else { 0xFF802018 }
    } else if m.nuclear() {
        0xFFB8C4D0
    } else {
        0xFF6A7A8A
    };
    c.disc(CX, CY, 3.2, 0xFF0A1018);
    c.disc(CX, CY, 2.2, core);
    // Labels stay in the plate's corners, clear of the lens ring (two lines each, for longer languages).
    c.fit(&tr("gui.kiloton.bomb.tamper", &[]), 13, 21, 56, BLUEPRINT, 0.75);
    c.fit(&tr("gui.kiloton.bomb.assembly_1", &[]), 13, 104, 38, BLUEPRINT, 0.75);
    c.fit(&tr("gui.kiloton.bomb.assembly_2", &[]), 13, 111, 38, BLUEPRINT, 0.75);
    c.fit_right(&tr("gui.kiloton.bomb.lenses_1", &[]), 170, 25, 55, 0xFFC09048, 0.75);
    c.fit_right(&tr("gui.kiloton.bomb.lenses_2", &[]), 170, 32, 38, 0xFFC09048, 0.75);
}

/// The two-stage plate: the tamper and the jacket when loaded, the spark plug down the axis, and, armed, the lenses
/// and the radiation channel pulsing.
fn schematic_h(c: &mut Canvas<'_>, v: &View, m: &FissionModel) {
    let t = v.load.thermo();
    if m.tamper {
        c.ring(H_PX, H_PY, 29.5, 32.5, 0xFF7A7F78);
    } else {
        c.ring(H_PX, H_PY, 30.5, 31.5, 0xFF2E5274);
    }
    if v.armed {
        let a = pulse(120.0, 60.0);
        c.ring(H_PX, H_PY, 33.0, 38.0, (a << 24) | 0xFF2010);
    }
    // Secondary: the pusher round the fuel is uranium when the jacket is in, a thin outline when not.
    let (x0, y0, x1, y1) = (91, 36, 172, 99);
    let (wall, th) = if t.jacket { (0xFF7A7F78, 3) } else { (0xFF2E5274, 1) };
    c.fill(x0, y0, x1, y0 + th, wall);
    c.fill(x0, y1 - th, x1, y1, wall);
    c.fill(x0, y0, x0 + th, y1, wall);
    c.fill(x1 - th, y0, x1, y1, wall);
    // The spark plug: a rod of plutonium or uranium down the axis.
    let rod = if t.spark_plug {
        if v.armed && blink(200) { 0xFFFF6040 } else { 0xFFC8B070 }
    } else {
        0xFF2E5274
    };
    c.fill(95, 66, 113, 69, rod);
    c.fill(131, 66, 150, 69, rod);
    // Armed: X-rays down the channel between the stages.
    if v.armed {
        let a = pulse(90.0, 70.0);
        c.fill(84, 40, 90, 95, (a << 24) | 0x60C0FF);
    }
    c.small(&tr("gui.kiloton.hbomb.primary_label", &[]), 9, 23, BLUEPRINT, 0.75);
    let sec = tr("gui.kiloton.hbomb.secondary_label", &[]);
    let sw = (c.width(&sec) as f32 * 0.75).ceil() as i32;
    c.small(&sec, 172 - sw, 26, BLUEPRINT, 0.75);
    c.small(&tr("gui.kiloton.bomb.tamper", &[]), 30, 108, BLUEPRINT, 0.75);
    let fuel = tr("gui.kiloton.hbomb.fuel_label", &[&format!("{:.0}", t.fuel_kg)]);
    c.small(&fuel, 92, 101, if t.charges > 0 { 0xFF6CCBFF } else { BLUEPRINT }, 0.75);
}

/// The big yield figure at the top of the readout.
fn big_yield(c: &mut Canvas<'_>, nuclear: bool, kt: f64, capped: bool) {
    let x = CRT_X + 4;
    c.small(&tr("gui.kiloton.bomb.yield", &[]), x, CRT_Y + 4, GREEN_DIM, 0.75);
    let (value, unit) = if !nuclear {
        ("---".to_owned(), "kt")
    } else if kt < 1.0 {
        (format!("{:.0}", kt * 1000.0), "t")
    } else if kt < 100.0 {
        (format!("{kt:.1}"), "kt")
    } else if kt < 1000.0 {
        (format!("{kt:.0}"), "kt")
    } else {
        (format!("{:.2}", kt / 1000.0), "Mt")
    };
    let colour = if nuclear { GREEN } else { GREEN_DIM };
    let right = CRT_X + 116;
    c.seven_seg_right(&value, right, CRT_Y + 13, 11, 21, 3, colour, 0x1E40FF60);
    c.text(unit, right + 3, CRT_Y + 26, colour);
    if capped {
        c.fit(&tr("gui.kiloton.bomb.capped", &[]), x, CRT_Y + 13, 50, AMBER, 0.75);
    }
}

/// The readout of the thermonuclear device: the yield, how the secondary lights, the split, the radii.
fn readout_h(c: &mut Canvas<'_>, v: &View) {
    let t = v.load.thermo();
    let w = v.warhead();
    let (x, right) = (CRT_X + 4, CRT_X + 135);
    let kt = v.yield_kt();
    big_yield(c, w.nuclear(), kt, v.capped());
    let (state, sc) = match t.state {
        ThermonuclearState::Nominal => (tr("gui.kiloton.hbomb.state.nominal", &[]), GREEN),
        ThermonuclearState::Partial => (
            tr(if t.spark_plug { "gui.kiloton.hbomb.state.weak_primary" } else { "gui.kiloton.hbomb.state.no_spark" }, &[]),
            AMBER,
        ),
        ThermonuclearState::NoIgnition => (tr("gui.kiloton.hbomb.state.no_ignition", &[]), RED),
        ThermonuclearState::NoFuel => (tr("gui.kiloton.hbomb.state.no_fuel", &[]), AMBER),
        ThermonuclearState::NoPrimary => {
            let some = t.primary.mass_kg > 0.0;
            (tr(if some { "gui.kiloton.bomb.state.subcritical" } else { "gui.kiloton.bomb.state.empty" }, &[]), if some { RED } else { GREY })
        }
    };
    c.fit(&state, x, CRT_Y + 38, right - x, sc, 1.0);
    // Compression bar: the primary's yield against what this much fuel needs (the line).
    let (bx0, bx1, by) = (x, right, CRT_Y + 49);
    c.fill(bx0, by, bx1, by + 5, 0xFF0E2614);
    let need = t.primary_needed_kt();
    let fw = ((t.primary.yield_kt / (need * 1.5)).min(1.0) * (bx1 - bx0) as f64).round() as i32;
    if fw > 0 {
        c.fill(bx0, by, bx0 + fw, by + 5, sc);
    }
    let mark = bx0 + ((bx1 - bx0) as f64 / 1.5).round() as i32;
    c.fill(mark, by - 2, mark + 1, by + 7, 0xFFE8FFE8);
    let ly = CRT_Y + 57;
    let pri = tr("gui.kiloton.hbomb.primary", &[&if t.primary.nuclear() { FissionModel::format(t.primary.yield_kt) } else { "0 t".to_owned() }]);
    let fusion = if w.thermonuclear { (1.0 - w.fission_fraction) * 100.0 } else { 0.0 };
    let fus = tr("gui.kiloton.hbomb.fusion", &[&format!("{fusion:.0}")]);
    let fus_w = c.width(&fus);
    c.fit(&pri, x, ly, right - fus_w - 4 - x, GREEN, 1.0);
    c.text(&fus, right - fus_w, ly, if w.thermonuclear { 0xFF6CCBFF } else { GREEN_DIM });
    radii(c, x, ly, kt);
}

/// Fireball, houses flat, fires (Java: the three `line`s of both readouts).
fn radii(c: &mut Canvas<'_>, x: i32, ly: i32, kt: f64) {
    let b = (kt > 0.0).then(|| BlastModel::new(kt));
    let d = |f: &dyn Fn(&BlastModel) -> f64| b.as_ref().map_or("-".to_owned(), |b| FissionModel::distance(f(b)));
    line(c, x, ly + 9, "gui.kiloton.bomb.fireball", &d(&|b| b.fireball_radius));
    line(c, x, ly + 18, "gui.kiloton.bomb.houses", &d(&|b| b.radius_for_psi(5.0)));
    line(c, x, ly + 27, "gui.kiloton.bomb.fires", &d(&|b| b.radius_for_fluence(6.0)));
}

fn code(c: &mut Canvas<'_>, v: &View, mx: f64, my: f64) {
    if !v.armed && in_rect(mx, my, CODE_X0, CODE_Y0, CODE_X1, CODE_Y1) {
        c.fill(CODE_X0, CODE_Y0, CODE_X1, CODE_Y1, 0x30FFFFFF);
    }
    c.fit_right(&tr("gui.kiloton.bomb.code", &[]), 170, 104, 38, BLUEPRINT, 0.75);
    let code = bomb::format_code(v.code);
    let w = c.width(&code);
    c.text(&code, 171 - w, 111, STENCIL);
}

fn readout(c: &mut Canvas<'_>, v: &View, m: &FissionModel) {
    let x = CRT_X + 4;
    let kt = v.yield_kt();
    big_yield(c, m.nuclear(), kt, v.capped());
    let (state, sc) = match m.state {
        FissionState::Nominal => (tr("gui.kiloton.bomb.state.nominal", &[]), GREEN),
        FissionState::Fizzle => (tr("gui.kiloton.bomb.state.fizzle", &[]), AMBER),
        FissionState::Subcritical => (tr("gui.kiloton.bomb.state.subcritical", &[]), RED),
        FissionState::Empty => (tr("gui.kiloton.bomb.state.empty", &[]), GREY),
    };
    let k = format!("k {:.2}", m.alpha);
    let kw = c.width(&k);
    c.fit(&state, x, CRT_Y + 38, CRT_X + 135 - kw - 4 - x, sc, 1.0);
    c.text(&k, CRT_X + 135 - kw, CRT_Y + 38, if m.mass_kg > 0.0 { sc } else { GREY });
    // Criticality bar: 0..4 critical masses, the line at 1.
    let (bx0, bx1, by) = (x, CRT_X + 135, CRT_Y + 49);
    c.fill(bx0, by, bx1, by + 5, 0xFF0E2614);
    let fw = ((m.alpha / 4.0).min(1.0) * (bx1 - bx0) as f64).round() as i32;
    if fw > 0 {
        c.fill(bx0, by, bx0 + fw, by + 5, sc);
    }
    let one = bx0 + (bx1 - bx0) / 4;
    c.fill(one, by - 2, one + 1, by + 7, 0xFFE8FFE8);
    let ly = CRT_Y + 57;
    let pit = tr("gui.kiloton.bomb.pit", &[&format!("{:.1}", m.mass_kg)]);
    let eff = tr("gui.kiloton.bomb.eff", &[&format!("{:.1}", m.efficiency * 100.0)]);
    let ew = c.width(&eff);
    c.fit(&pit, x, ly, CRT_X + 135 - ew - 4 - x, GREEN, 1.0);
    c.text(&eff, CRT_X + 135 - ew, ly, GREEN);
    radii(c, x, ly, kt);
}

/// A label on the left and its value on the right; the label gives way when the two would touch.
fn line(c: &mut Canvas<'_>, x: i32, y: i32, key: &str, value: &str) {
    let vw = c.width(value);
    c.fit(&tr(key, &[]), x, y, CRT_X + 135 - vw - 4 - x, GREEN_DIM, 1.0);
    c.text(value, CRT_X + 135 - vw, y, GREEN);
}

fn timer(c: &mut Canvas<'_>, v: &View, mx: f64, my: f64) {
    let ticks = if v.armed { v.remaining } else { v.timer * 20 };
    let t = bomb::clock(ticks);
    let colour = if v.armed {
        if v.remaining <= 200 && blink(150) { 0xFFFFE0D0 } else { 0xFFFF3B28 }
    } else {
        0xFFFF6A40
    };
    let digits_w = seven_seg_width(&t, 8, 2);
    let x = (TIMER_X0 + TIMER_X1) / 2 + digits_w / 2;
    c.seven_seg_right(&t, x, TIMER_Y0 + 4, 8, 15, 2, colour, 0xFF2A0806);
    for i in 0..4 {
        let [bx, by] = TIMER_BUTTONS[i];
        if !v.armed && in_rect(mx, my, bx, by, bx + 31, by + 10) {
            c.fill(bx + 1, by + 1, bx + 30, by + 9, 0x40FFFFFF);
        }
        let s = TIMER_LABELS[i];
        let w = c.width(s);
        c.text(s, bx + 16 - w / 2, by + 1, if v.armed { 0xFF5A6068 } else { 0xFFE8ECF0 });
    }
}

fn arming(c: &mut Canvas<'_>, v: &View, m: &FissionModel, mx: f64, my: f64) {
    if v.armed {
        COVER_OPEN.store(true, Ordering::Relaxed);
    }
    let cover_open = COVER_OPEN.load(Ordering::Relaxed);
    // The button: a red mushroom in a black collar.
    c.disc(BTN_X, BTN_Y, 15.5, 0xFF2A2A2E);
    c.disc(BTN_X, BTN_Y, 13.5, 0xFF0C0C0E);
    let hot = v.armed && blink(300);
    c.disc(BTN_X, BTN_Y + 1.0, 11.5, if v.armed { 0xFF701008 } else { 0xFF6E120C });
    c.disc(BTN_X, BTN_Y, 11.0, if hot { 0xFFFF4A30 } else if v.armed { 0xFFD02818 } else { 0xFFB8231A });
    c.disc(BTN_X - 3.0, BTN_Y - 4.0, 4.0, if hot { 0xFFFFB0A0 } else { 0xFFE06050 });
    c.disc(BTN_X - 4.0, BTN_Y - 5.0, 1.6, 0xFFFFD8D0);
    if cover_open {
        // The cover stands up behind the well, seen edge-on.
        for x in WELL_X0..WELL_X1 {
            c.fill(x, 149, x + 1, 155, if (x / 3) % 2 == 0 { 0xFFE0B020 } else { 0xFF1A1814 });
        }
        c.fill(WELL_X0 - 1, 155, WELL_X1 + 1, 157, 0xFF404448);
    } else {
        // Translucent red plastic over the button, striped frame, hinge on top.
        c.fill(WELL_X0 + 3, WELL_Y0 + 3, WELL_X1 - 3, WELL_Y1 - 3, 0x90C02010);
        for i in WELL_X0..WELL_X1 {
            let col = if ((i - WELL_X0) / 3) % 2 == 0 { 0xFFE0B020 } else { 0xFF1A1814 };
            c.fill(i, WELL_Y0, i + 1, WELL_Y0 + 3, col);
            c.fill(i, WELL_Y1 - 3, i + 1, WELL_Y1, col);
        }
        for j in WELL_Y0..WELL_Y1 {
            let col = if ((j - WELL_Y0) / 3) % 2 == 0 { 0xFFE0B020 } else { 0xFF1A1814 };
            c.fill(WELL_X0, j, WELL_X0 + 3, j + 1, col);
            c.fill(WELL_X1 - 3, j, WELL_X1, j + 1, col);
        }
        c.fill(WELL_X0 + 6, WELL_Y0 + 6, WELL_X0 + 14, WELL_Y0 + 8, 0x60FFFFFF);
        c.fill(WELL_X0 - 1, WELL_Y0 - 2, WELL_X1 + 1, WELL_Y0, 0xFF404448);
    }

    let (x, y) = (INFO_X, INFO_Y);
    if v.armed {
        let t = bomb::clock(v.remaining);
        if blink(400) {
            c.fit(&tr("gui.kiloton.bomb.armed", &[]), x, y, 318 - seven_seg_width(&t, 7, 2) - 4 - x, RED, 1.0);
        }
        c.seven_seg_right(&t, 318, y, 7, 11, 2, 0xFFFF3B28, 0xFF2A0806);
        let w = v.warhead();
        let yield_s = if w.nuclear() { FissionModel::format(v.yield_kt()) } else { "0 t".to_owned() };
        c.fit(&tr("gui.kiloton.bomb.detonation", &[&yield_s]), x, y + 15, INFO_W, AMBER, 0.75);
        let hover = in_rect(mx, my, DIS_X0, DIS_Y0, DIS_X1, DIS_Y1);
        c.fill(DIS_X0, DIS_Y0, DIS_X1, DIS_Y1, if hover { 0xFF3C8A4A } else { 0xFF24502C });
        c.fill(DIS_X0, DIS_Y0, DIS_X1, DIS_Y0 + 1, 0xFF6CCB7C);
        c.fill(DIS_X0, DIS_Y1 - 1, DIS_X1, DIS_Y1, 0xFF10241A);
        c.fit_centre(&tr("gui.kiloton.bomb.disarm", &[]), (DIS_X0 + DIS_X1) / 2, DIS_Y0 + 2, DIS_X1 - DIS_X0 - 4, 0xFFE8FFE8, 1.0);
    } else {
        c.fit(
            &tr(if cover_open { "gui.kiloton.bomb.cover_up" } else { "gui.kiloton.bomb.status_safe" }, &[]),
            x,
            y,
            INFO_W,
            if cover_open { AMBER } else { GREEN },
            1.0,
        );
        c.fit(&tr(if cover_open { "gui.kiloton.bomb.press_to_arm" } else { "gui.kiloton.bomb.lift_cover" }, &[]), x, y + 11, INFO_W, GREY, 0.75);
        let (note, nc) = match m.state {
            FissionState::Empty => (tr("gui.kiloton.bomb.note.empty", &[]), AMBER),
            FissionState::Subcritical => (tr("gui.kiloton.bomb.note.subcritical", &[]), AMBER),
            _ => (tr("gui.kiloton.bomb.note.timer", &[&bomb::clock(v.timer * 20)]), GREY),
        };
        c.fit(&note, x, y + 20, INFO_W, nc, 0.75);
        let kt = v.yield_kt();
        if kt > 0.0 {
            let r = BlastModel::new(kt).radius_for_psi(1.0);
            c.fit(&tr("gui.kiloton.bomb.evacuate", &[&FissionModel::distance(r)]), x, y + 29, INFO_W, RED, 0.75);
        }
    }
}

// ------------------------------------------------------------------------------------------------ tooltips and clicks

/// The panel's tooltips (`native_screen_tooltip`; Java: `BombScreen.drawTips`), broken at `TIP_WIDTH`.
pub fn tooltip(host: &dyn Host, block: &str, data: &[i32], at: [f32; 2], lines: &mut Vec<String>) {
    if !bomb::is_bomb(block) {
        return;
    }
    let v = View::of(block, data);
    let (mx, my) = (at[0] as f64, at[1] as f64);
    let m = v.load.model();
    let cover_open = COVER_OPEN.load(Ordering::Relaxed);
    // Horizon asks here only off a slot with an item: an empty slot gets its hint.
    let slot = slot_at(block, mx, my);
    let tip = if in_rect(mx, my, TIMER_X0, TIMER_Y0, TIMER_X1, TIMER_Y1) {
        tr("gui.kiloton.bomb.tip.timer", &[])
    } else if in_rect(mx, my, WELL_X0, WELL_Y0, WELL_X1, WELL_Y1) {
        tr(
            if v.armed {
                "gui.kiloton.bomb.tip.armed"
            } else if cover_open {
                "gui.kiloton.bomb.tip.button"
            } else {
                "gui.kiloton.bomb.tip.cover"
            },
            &[],
        )
    } else if in_rect(mx, my, CODE_X0, CODE_Y0, CODE_X1, CODE_Y1) {
        tr("gui.kiloton.bomb.tip.code", &[])
    } else if v.thermo && in_rect(mx, my, CRT_X + 4, CRT_Y + 45, CRT_X + 135, CRT_Y + 56) {
        let t = v.load.thermo();
        tr(
            "gui.kiloton.hbomb.tip.compression",
            &[&FissionModel::format(t.primary.yield_kt), &FissionModel::format(t.primary_needed_kt()), &format!("{:.0}", t.burn * 100.0)],
        )
    } else if in_rect(mx, my, CRT_X + 4, CRT_Y + 45, CRT_X + 135, CRT_Y + 56) {
        tr("gui.kiloton.bomb.tip.k", &[&format!("{:.2}", m.alpha)])
    } else if v.armed {
        return;
    } else {
        match slot {
            Some(s) if s == bomb::TAMPER => tr("gui.kiloton.bomb.tip.tamper", &[]),
            Some(s) if v.thermo && s == bomb::SPARK => tr("gui.kiloton.hbomb.tip.spark", &[]),
            Some(s) if v.thermo && s == bomb::JACKET => tr("gui.kiloton.hbomb.tip.jacket", &[]),
            Some(s) if v.thermo && (bomb::FUEL..bomb::SPARK).contains(&s) => tr("gui.kiloton.hbomb.tip.fuel", &[]),
            Some(s) if s < bomb::PIT => tr("gui.kiloton.bomb.tip.pit", &[]),
            _ => return,
        }
    };
    lines.extend(wrap(host, &tip, TIP_WIDTH));
}

/// A line broken at its spaces into lines of at most `max_w` GUI pixels.
pub(crate) fn wrap(host: &dyn Host, s: &str, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split(' ').filter(|w| !w.is_empty()) {
        let next = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if !line.is_empty() && host.text_width(&next) > max_w {
            out.push(std::mem::replace(&mut line, word.to_owned()));
        } else {
            line = next;
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

/// A press on the panel (`native_screen_click`; Java: `BombScreen.click`): the button it sends to the server, or
/// None to let the click through. The safety cover is the panel's own; its press sends `B_COVER`, which the server
/// ignores, so the press makes its sound and goes no further.
pub fn click(block: &str, data: &[i32], at: [f32; 2], right: bool) -> Option<i32> {
    if !bomb::is_bomb(block) || right {
        return None;
    }
    let v = View::of(block, data);
    let (mx, my) = (at[0] as f64, at[1] as f64);
    let cover_open = COVER_OPEN.load(Ordering::Relaxed);
    if !v.armed {
        for i in 0..4 {
            let [bx, by] = TIMER_BUTTONS[i];
            if in_rect(mx, my, bx, by, bx + 31, by + 10) {
                return Some(TIMER_IDS[i]);
            }
        }
        if in_rect(mx, my, WELL_X0, WELL_Y0, WELL_X1, WELL_Y1) {
            if !cover_open {
                COVER_OPEN.store(true, Ordering::Relaxed);
                return Some(B_COVER);
            }
            if (mx - BTN_X).hypot(my - BTN_Y) <= 12.0 {
                return Some(B_ARM);
            }
            COVER_OPEN.store(false, Ordering::Relaxed);
            return Some(B_COVER);
        }
        if in_rect(mx, my, CODE_X0, CODE_Y0, CODE_X1, CODE_Y1) {
            return Some(B_NEW_CODE);
        }
        if cover_open && in_rect(mx, my, WELL_X0, 147, WELL_X1, 157) {
            COVER_OPEN.store(false, Ordering::Relaxed);
            return Some(B_COVER);
        }
    } else if in_rect(mx, my, DIS_X0, DIS_Y0, DIS_X1, DIS_Y1) {
        COVER_OPEN.store(false, Ordering::Relaxed);
        return Some(B_DISARM);
    }
    None
}
