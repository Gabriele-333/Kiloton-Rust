//! The screen of the machines (Java: `MachineScreen`): the slots of the kind, an energy gauge, the progress arrow (its
//! tooltip lists what the machine makes), and a line about the centrifuge cascade or the reactor's fuel and output.
//! It reads the machine from the menu's data slots (`machines::data`).

use crate::bomb_screen::{in_rect, tr, wrap, Canvas, TIP_WIDTH};
use crate::machines::{self, data};
use horizon_client::mod_api::{Host, HudBatch, NativeScreen};
use kiloton_core::machine::{self, MachineKind, FUEL, IN_A, IN_B, OUT_A, OUT_B, REACTOR_FE_PER_TICK, SLOTS};

const W: i32 = 176;
const H: i32 = 166;
const BG: &str = "kiloton:gui/machine";
const GAUGE_X0: i32 = 8;
const GAUGE_Y0: i32 = 17;
const GAUGE_X1: i32 = 16;
const GAUGE_Y1: i32 = 69;
const ARROW_X: i32 = 82;
const ARROW_Y: i32 = 36;
const TEXT_DIM: u32 = 0xFF8A949E;
const AMBER: u32 = 0xFFE0A526;
const BLUE: u32 = 0xFF5AB4FF;

/// Where each slot sits (`native_screen`): only those the kind uses; the inventory below, as `MachineMenu`'s.
pub fn layout(block: &str) -> Option<NativeScreen> {
    let k = machines::kind(block)?;
    let slots = (0..SLOTS).map(|s| k.slot_pos(s).map(|[x, y]| [x as f32, y as f32])).collect();
    Some(NativeScreen {
        width: W as f32,
        height: H as f32,
        slots,
        inventory_x: 8.0,
        inventory_y: 84.0,
        own_labels: true,
        own_background: true,
    })
}

/// The machine's numbers, decoded.
struct View {
    kind: MachineKind,
    progress: f32,
    energy: u32,
    burn_left: f32,
    cascade: f32,
    working: bool,
    recipe: usize,
    draw: u32,
    run: u32,
}

impl View {
    fn of(k: MachineKind, d: &[i32]) -> Self {
        let at = |i: usize| d.get(i).copied().unwrap_or(0) & 0xFFFF;
        let total = at(data::BURN_TOTAL);
        View {
            kind: k,
            progress: (at(data::PROGRESS) as f32 / (k.ticks() * 10) as f32).min(1.0),
            energy: data::join(d, data::ENERGY_LO, data::ENERGY_HI),
            burn_left: if total <= 0 { 0.0 } else { at(data::BURN) as f32 / total as f32 },
            cascade: at(data::CASCADE).max(10) as f32 / 10.0,
            working: at(data::WORKING) != 0,
            recipe: at(data::RECIPE) as usize,
            draw: data::join(d, data::DRAW_LO, data::DRAW_HI),
            run: data::join(d, data::RUN_LO, data::RUN_HI),
        }
    }
}

/// Java: `MachineScreen.drawBack`.
pub fn draw(host: &dyn Host, block: &str, d: &[i32], batch: &mut HudBatch, origin: [f32; 2]) {
    let Some(k) = machines::kind(block) else { return };
    let v = View::of(k, d);
    let mut c = Canvas { host, batch, ox: origin[0], oy: origin[1] };
    c.blit_size(BG, W, H);
    let title = c.host.screen_title().unwrap_or_default();
    c.fit(&title, 8, 6, 160, AMBER, 1.0);
    c.text(&tr("container.inventory", &[]), 8, 73, TEXT_DIM);
    for s in 0..SLOTS {
        if let Some([x, y]) = k.slot_pos(s) {
            slot(&mut c, x, y, s == OUT_A || s == OUT_B);
        }
    }
    // Energy gauge.
    c.fill(GAUGE_X0 - 1, GAUGE_Y0 - 1, GAUGE_X1 + 1, GAUGE_Y1 + 1, 0xFF101215);
    c.fill(GAUGE_X0, GAUGE_Y0, GAUGE_X1, GAUGE_Y1, 0xFF1E2126);
    let e = v.energy as f32 / k.capacity() as f32;
    let h = (e * (GAUGE_Y1 - GAUGE_Y0) as f32).round() as i32;
    for y in GAUGE_Y1 - h..GAUGE_Y1 {
        let f = (GAUGE_Y1 - y) as f32 / (GAUGE_Y1 - GAUGE_Y0) as f32;
        let (r, g, b) = (255u32, (60.0 + 150.0 * f) as u32, 30u32);
        c.fill(GAUGE_X0, y, GAUGE_X1, y + 1, 0xFF00_0000 | r << 16 | g << 8 | b);
    }
    let mut y = GAUGE_Y0 + 5;
    while y < GAUGE_Y1 {
        c.fill(GAUGE_X1 - 3, y, GAUGE_X1, y + 1, 0x6000_0000);
        y += 6;
    }
    // Fuel flame over the burnable slot.
    if let Some([fx, fy]) = k.slot_pos(FUEL) {
        flame(&mut c, fx + 4, fy - 15, v.energy > 0);
    }
    arrow(&mut c, ARROW_X, ARROW_Y, v.progress, v.working);
    match k {
        MachineKind::Centrifuge if v.cascade > 1.01 => {
            let s = tr("gui.kiloton.machine.cascade", &[&kiloton_core::format::fixed(v.cascade as f64, 1)]);
            c.fit(&s, ARROW_X - 2, 58, 168 - ARROW_X, AMBER, 0.75);
        }
        MachineKind::Reactor => {
            if let (Some([fx, fy]), Some([_, ay])) = (k.slot_pos(IN_B), k.slot_pos(IN_A)) {
                // Fuel element burn-down bar next to its slot.
                c.fill(fx + 19, fy, fx + 22, fy + 16, 0xFF101215);
                let bh = (v.burn_left * 16.0).round() as i32;
                if bh > 0 {
                    c.fill(fx + 19, fy + 16 - bh, fx + 22, fy + 16, BLUE);
                }
                // Labels right up against their slots, between the energy gauge and the slot's frame.
                let room = fx - 3 - (GAUGE_X1 + 2);
                c.fit_right(&tr("gui.kiloton.machine.fuel", &[]), fx - 3, fy + 5, room, TEXT_DIM, 0.75);
                c.fit_right(&tr("gui.kiloton.machine.target", &[]), fx - 3, ay + 5, room, TEXT_DIM, 0.75);
            }
            if v.working {
                let s = tr("gui.kiloton.machine.output", &[&machine::fe(REACTOR_FE_PER_TICK as i64)]);
                c.fit(&s, ARROW_X - 2, 58, 168 - ARROW_X, BLUE, 0.75);
            }
        }
        _ => {}
    }
}

fn slot(c: &mut Canvas<'_>, x: i32, y: i32, output: bool) {
    c.fill(x - 1, y - 1, x + 17, y + 17, 0xFF101215);
    c.fill(x, y, x + 17, y + 17, 0xFF585E66);
    c.fill(x, y, x + 16, y + 16, if output { 0xFF262A30 } else { 0xFF1E2126 });
}

fn flame(c: &mut Canvas<'_>, x: i32, y: i32, lit: bool) {
    const ROWS: [u32; 8] = [0b0010_0000, 0b0011_0000, 0b0111_0000, 0b0111_1000, 0b1111_1000, 0b1111_1100, 0b1111_1100, 0b0111_1000];
    for (j, row) in ROWS.iter().enumerate() {
        let j = j as i32;
        for i in 0..8 {
            if row & (0x80 >> i) != 0 {
                let col = if lit {
                    if j < 3 { 0xFFFFE070 } else if j < 6 { 0xFFFF9A20 } else { 0xFFE04010 }
                } else {
                    0xFF3A3F46
                };
                c.fill(x + i, y + j + 4, x + i + 1, y + j + 5, col);
            }
        }
    }
}

fn arrow(c: &mut Canvas<'_>, x: i32, y: i32, progress: f32, working: bool) {
    let len = 24;
    let filled = (progress * len as f32).round() as i32;
    for i in 0..len {
        let half = if i < 16 { 2 } else { (7 - (i - 16)).max(0) };
        let col = if i < filled { if working { 0xFFFFB020 } else { 0xFFB08020 } } else { 0xFF33383F };
        c.fill(x + i, y + 7 - half, x + i + 1, y + 8 + half, col);
    }
}

/// An item's name as its stack shows it (`getHoverName` of a plain stack).
fn item_name(id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    tr(&format!("item.{ns}.{path}"), &[])
}

/// Java: `MachineScreen.drawTips`. Horizon asks here off a slot holding an item.
pub fn tooltip(host: &dyn Host, block: &str, d: &[i32], at: [f32; 2], lines: &mut Vec<String>) {
    let Some(k) = machines::kind(block) else { return };
    let v = View::of(k, d);
    let (mx, my) = (at[0] as f64, at[1] as f64);
    let mut tip: Vec<String> = Vec::new();
    if in_rect(mx, my, GAUGE_X0 - 1, GAUGE_Y0 - 1, GAUGE_X1 + 1, GAUGE_Y1 + 1) {
        tip.push(format!("{} / {} FE", machine::grouped(v.energy as i64), machine::grouped(k.capacity() as i64)));
        if k == MachineKind::Reactor {
            tip.push(tr("gui.kiloton.machine.reactor_power", &[]));
        } else {
            tip.push(tr("gui.kiloton.machine.uses", &[&machine::fe(v.draw as i64), &machine::fe(v.run as i64)]));
            tip.push(tr("gui.kiloton.machine.power_in", &[]));
        }
    } else if in_rect(mx, my, ARROW_X, ARROW_Y, ARROW_X + 24, ARROW_Y + 16) {
        tip.push(tr("gui.kiloton.machine.recipes", &[]));
        for (i, r) in k.recipes().iter().enumerate() {
            let (out, n) = r.out_a;
            let out = if n > 1 { format!("{n}x {}", item_name(out)) } else { item_name(out) };
            let mut line = format!("{}x {} -> {out}", r.count, tr(r.in_key, &[]));
            if let Some((b, _)) = r.out_b {
                line = format!("{line} + {}", item_name(b));
            }
            // The one the input slot holds now.
            if v.recipe == i + 1 {
                line = format!("> {line}");
            }
            tip.push(line);
        }
        match k {
            MachineKind::Reactor => tip.push(tr("gui.kiloton.machine.reactor_fuel", &[])),
            MachineKind::Centrifuge => tip.push(tr("gui.kiloton.machine.cascade_tip", &[])),
            _ => {}
        }
    } else if v.kind == MachineKind::Reactor && slot_at(k, mx, my) == Some(IN_B) {
        tip.push(tr("gui.kiloton.machine.reactor_fuel", &[]));
    }
    for t in tip {
        lines.extend(wrap(host, &t, TIP_WIDTH));
    }
}

fn slot_at(k: MachineKind, x: f64, y: f64) -> Option<usize> {
    (0..SLOTS).find(|&s| k.slot_pos(s).is_some_and(|[sx, sy]| in_rect(x, y, sx - 1, sy - 1, sx + 17, sy + 17)))
}
