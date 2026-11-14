//! `/nuke` (Java: `NukeCommand`). `/nuke <kt> [pos]` sets off a surface burst, `info <kt>` prints
//! its effect radii, `status` lists the running bursts and `stop` ends them. `clear` (the clouds)
//! waits for the client side.

use horizon_client::mod_api::{Host, NativeCommand, NativeServerPlayer, Text};
use kiloton_core::config::ServerConfig;
use kiloton_core::format::{fixed, nuke_yield};
use kiloton_core::BlastModel;

/// Operators of this level and above see and run `/nuke` (vanilla's game masters).
const LEVEL: u8 = 2;

const GOLD: u32 = 0xFFAA00;
const GRAY: u32 = 0xAAAAAA;
const WHITE: u32 = 0xFFFFFF;
const RED: u32 = 0xFF5555;

pub fn register(commands: &mut Vec<NativeCommand>) {
    for usage in [
        "nuke <kilotons:number>",
        "nuke <kilotons:number> <pos:block_pos>",
        "nuke info <kilotons:number>",
        "nuke status",
        "nuke stop",
        "nuke clear",
    ] {
        commands.push(NativeCommand::op(usage, LEVEL));
    }
}

/// Runs a `/nuke` line (without its slash); false when the line is not ours.
pub fn perform(host: &mut dyn Host, player: &NativeServerPlayer, line: &str) -> bool {
    let mut words = line.split_whitespace();
    if words.next() != Some("nuke") {
        return false;
    }
    let config = ServerConfig::default();
    let rest: Vec<&str> = words.collect();
    match rest.as_slice() {
        ["info", kt] => match parse_yield(kt) {
            Some(kt) => info(host, kt, &config),
            None => host.feedback(Text::colored(format!("Not a yield: {kt}"), RED)),
        },
        ["status"] => {
            let lines = crate::blast::with(|b| b.status());
            for l in lines {
                host.feedback(Text::colored(l, WHITE));
            }
        }
        ["stop"] => {
            let n = crate::blast::with(|b| b.stop(&player.dimension));
            host.feedback(Text::translate("commands.kiloton.stop", &[Text::literal(n.to_string())]));
        }
        ["clear"] => host.feedback(Text::colored("Kiloton for Horizon: the clouds are not ported yet.", GRAY)),
        [kt, pos @ ..] => match (parse_yield(kt), parse_pos(pos, player.position)) {
            (Some(kt), _) if kt > config.max_command_yield() => host.feedback(Text::colored(
                format!("That is more than the configured maximum ({} kt)", config.max_command_yield()),
                RED,
            )),
            (Some(kt), Some(at)) => {
                // Above the fission limit the burst is a thermonuclear one (half fission, half fusion).
                let fission = if kt > config.max_yield_kt { 0.5 } else { 1.0 };
                crate::blast::with(|b| b.request(&player.dimension, at, kt, fission, player.uuid));
            }
            (None, _) => host.feedback(Text::colored(format!("Not a yield: {kt}"), RED)),
            (_, None) => host.feedback(Text::colored("Not a position: x y z, ~ for yours".to_string(), RED)),
        },
        [] => host.feedback(Text::colored("/nuke <kilotons> [pos] | info <kilotons> | status | stop | clear", GRAY)),
    }
    true
}

/// The command's own range (Java: `DoubleArgumentType.doubleArg(0.001, 100000)`).
fn parse_yield(word: &str) -> Option<f64> {
    word.parse::<f64>().ok().filter(|kt| (0.001..=100000.0).contains(kt))
}

/// `x y z` (`~` and `~n` relative to `here`), or `here` itself when none is given.
fn parse_pos(words: &[&str], here: [f64; 3]) -> Option<[f64; 3]> {
    match words {
        [] => Some(here),
        [x, y, z] => {
            let mut out = [0.0; 3];
            for (k, w) in [x, y, z].into_iter().enumerate() {
                out[k] = match w.strip_prefix('~') {
                    Some("") => here[k],
                    Some(rel) => here[k] + rel.parse::<f64>().ok()?,
                    // A block position names the block: its middle across.
                    None => w.parse::<i32>().ok()? as f64 + if k == 1 { 0.0 } else { 0.5 },
                };
            }
            Some(out)
        }
        _ => None,
    }
}

fn info(host: &mut dyn Host, kt: f64, config: &ServerConfig) {
    let m = BlastModel::new(kt);
    host.feedback(Text::colored(format!("{} surface burst", nuke_yield(kt)), GOLD));
    let f0 = |x: f64| fixed(x, 0);
    let f2 = |x: f64| fixed(x, 2);
    let (r20, r5, r1) = (m.radius_for_psi(20.0), m.radius_for_psi(5.0), m.radius_for_psi(1.0));
    let cap = config.max_radius(m.thermonuclear());
    let lines = [
        ("crater", format!("{} m radius, {} m deep", f0(m.crater_radius), f0(m.crater_depth))),
        ("fireball", format!("{} m radius after {} s", f0(m.fireball_radius), f2(m.fireball_time))),
        ("20 psi (concrete)", format!("{} m, wave there after {} s", f0(r20), f2(m.arrival_time(r20)))),
        ("5 psi (houses)", format!("{} m, after {} s", f0(r5), f2(m.arrival_time(r5)))),
        ("1 psi (windows, leaves)", format!("{} m, after {} s", f0(r1), f2(m.arrival_time(r1)))),
        ("fires (6 cal/cm2)", format!("{} m", f0(m.radius_for_fluence(6.0)))),
        ("500 rem prompt dose", format!("{} m", f0(m.radius_for_dose(500.0)))),
        (
            "blocks touched up to",
            format!("{} m (cap {cap})", f0(m.natural_effect_radius.min(cap as f64))),
        ),
    ];
    for (key, value) in lines {
        host.feedback(Text::colored(format!("  {key}: "), GRAY).append(Text::colored(value, WHITE)));
    }
}
