//! Parity with the Java edition: every line of the reference table that the Java code wrote
//! (`horizon-26.3/parity`, `ParityTables.java`) is evaluated again here and compared.
//!
//! The table lives outside the crate (it is 4 MB and not part of the mod): the test reads it from
//! `KILOTON_PARITY_TABLE`, which `tools/hz.py test` sets. Without it the test fails and says so.
//!
//! Tolerances: plain arithmetic is bit-identical between Java and Rust (neither contracts to FMA),
//! but `pow`, `exp`, `ln`, `log10`, `cbrt`, `hypot`, `atan2`, `sin` and `cos` come from different
//! libraries and may differ by an ulp. Each table therefore allows a relative error (`rel`) and an
//! absolute one (`abs`) where cancellation turns ulps into absolute noise; the arrival-time table
//! of `BlastModel` stores `f32` like the Java one, so its two columns allow an `f32` ulp. The test
//! prints the largest error seen per table, so a drift shows before it fails.

use kiloton_core::blast::{LateGrid, RingColumns, ring_columns};
use kiloton_core::fx::mushroom::MushroomCloud;
use kiloton_core::fx::sky::SkyLight;
use kiloton_core::fx::{viewer, wilson};
use kiloton_core::physics::{fallout, radar_horizon, sam_geometry};
use kiloton_core::{
    Ballistics, BlackRain, BlastModel, BurstDetection, FissionModel, FissionState, Intercept, ThermonuclearModel,
    ThermonuclearState, Warhead,
};
use std::collections::BTreeMap;

#[derive(Debug)]
enum V {
    Num(f64),
    Text(String),
}

impl V {
    fn show(&self) -> String {
        match self {
            V::Num(x) => format!("{x:?}"),
            V::Text(s) => s.clone(),
        }
    }
}

fn n(x: f64) -> V {
    V::Num(x)
}
fn b(x: bool) -> V {
    V::Text(x.to_string())
}
fn t(x: impl Into<String>) -> V {
    V::Text(x.into())
}
fn i(x: i64) -> V {
    V::Text(x.to_string())
}
fn arr(out: &mut Vec<V>, a: [f64; 3]) {
    out.extend(a.iter().map(|&x| n(x)));
}

fn fstate(s: FissionState) -> &'static str {
    match s {
        FissionState::Empty => "EMPTY",
        FissionState::Subcritical => "SUBCRITICAL",
        FissionState::Fizzle => "FIZZLE",
        FissionState::Nominal => "NOMINAL",
    }
}

fn tstate(s: ThermonuclearState) -> &'static str {
    match s {
        ThermonuclearState::NoPrimary => "NO_PRIMARY",
        ThermonuclearState::NoFuel => "NO_FUEL",
        ThermonuclearState::NoIgnition => "NO_IGNITION",
        ThermonuclearState::Partial => "PARTIAL",
        ThermonuclearState::Nominal => "NOMINAL",
    }
}

struct Args<'a>(&'a [&'a str]);

impl Args<'_> {
    fn f(&self, k: usize) -> f64 {
        self.0[k].parse().unwrap_or_else(|_| panic!("not a number: {}", self.0[k]))
    }
    fn i(&self, k: usize) -> i32 {
        self.0[k].parse().unwrap()
    }
    fn l(&self, k: usize) -> i64 {
        self.0[k].parse().unwrap()
    }
    fn b(&self, k: usize) -> bool {
        self.0[k] == "true"
    }
}

/// The table's relative and absolute tolerance, and the columns (if any) held to the `f32` table.
fn tolerance(name: &str) -> (f64, f64) {
    match name {
        // Bisections and loops whose branches hang on transcendental results.
        "fallout.hours" | "rain.spot" | "intercept" => (1e-9, 1e-12),
        // Positions and velocities: differences of nearly equal numbers.
        "ballistic.at" | "interceptor.at" | "sam.local" | "sam.railCentre" | "sam.forward" => (1e-12, 1e-9),
        // The shader's parameters are f32 on both sides: an f32 ulp apart where the f64 lands on a rounding edge.
        "cloud.volume" | "wilson" | "sky" => (2e-7, 1e-6),
        _ => (1e-12, 1e-300),
    }
}

fn f32_columns(name: &str) -> &'static [usize] {
    match name {
        "blast.at" => &[4],   // arrivalTime
        "blast.time" => &[0], // frontRadius
        _ => &[],
    }
}

fn eval(name: &str, a: &Args, horizon: &mut Option<Vec<i8>>) -> Vec<V> {
    let mut out = Vec::new();
    match name {
        "blast.model" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            out.extend([
                n(m.yield_kt),
                n(m.fission_fraction),
                n(m.cube_root),
                n(m.crater_radius),
                n(m.crater_depth),
                n(m.lip_height),
                n(m.fireball_radius),
                n(m.fireball_time),
                n(m.natural_effect_radius),
                b(m.thermonuclear()),
            ]);
        }
        "blast.at" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            let d = a.f(2);
            out.extend([
                n(m.overpressure_psi(d)),
                n(m.dynamic_pressure_psi(d)),
                n(m.wind_speed(d)),
                n(m.shock_speed(d)),
                n(m.arrival_time(d)),
                n(m.thermal_fluence(d)),
                n(m.initial_dose_rem(d)),
                n(m.crater_depth_at(d)),
                n(m.lip_height_at(d)),
            ]);
        }
        "blast.time" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            out.extend([n(m.front_radius(a.f(2))), n(m.fireball_radius_at(a.f(2)))]);
        }
        "blast.psi" => out.push(n(BlastModel::with_fission(a.f(0), a.f(1)).radius_for_psi(a.f(2)))),
        "blast.fluence" => out.push(n(BlastModel::with_fission(a.f(0), a.f(1)).radius_for_fluence(a.f(2)))),
        "blast.dose" => out.push(n(BlastModel::with_fission(a.f(0), a.f(1)).radius_for_dose(a.f(2)))),
        "blast.damage" => out.push(n(BlastModel::blast_damage(a.f(0)))),
        "fallout.wind" => out.extend([n(fallout::wind_angle(a.l(0))), n(fallout::wind_speed(a.l(0)))]),
        "fallout.decay" => out.push(n(fallout::decay(a.f(0)))),
        "fallout.model" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            out.extend([
                n(fallout::plume_length(&m)),
                n(fallout::reach(&m)),
                n(fallout::surge_radius(&m)),
                n(fallout::dust_reach(&m)),
            ]);
        }
        "fallout.spot" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            let (seed, dx, dz) = (a.l(2), a.f(3), a.f(4));
            out.extend([
                n(fallout::dose_rate_h1(&m, seed, dx, dz)),
                n(fallout::arrival_seconds(seed, dx, dz)),
                n(fallout::surge_h1(&m, dx, dz)),
                n(fallout::surge_seconds(&m, dx, dz)),
                n(fallout::dust_cover(&m, seed, dx, dz)),
                n(fallout::dust_seconds(&m, seed, dx, dz)),
                n(fallout::total_dose(&m, seed, None, dx, dz)),
            ]);
        }
        "fallout.rate" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            out.push(n(fallout::dose_rate(&m, a.l(2), a.f(3), a.f(4), a.f(5))));
        }
        "fallout.until" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            out.push(n(fallout::dose_until(&m, a.l(2), None, a.f(3), a.f(4), a.f(5))));
        }
        "fallout.hours" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            out.push(n(fallout::hours_to_dose(&m, a.l(2), None, a.f(3), a.f(4), a.f(5))));
        }
        "rain.of" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            match BlackRain::of(&m, a.l(2), a.f(3)) {
                None => out.push(t("null")),
                Some(r) => out.extend([
                    t("some"),
                    n(r.start),
                    n(r.end),
                    n(r.centre),
                    n(r.along),
                    n(r.across),
                    n(r.peak_h1),
                    n(r.reach()),
                ]),
            }
        }
        "rain.spot" | "rain.rate" | "rain.strength" => {
            let m = BlastModel::with_fission(a.f(0), a.f(1));
            let seed = a.l(2);
            let r = BlackRain::of(&m, seed, a.f(3)).expect("the Java side had a rain here");
            match name {
                "rain.spot" => {
                    let (dx, dz) = (a.f(4), a.f(5));
                    out.extend([
                        n(r.footprint(dx, dz)),
                        n(r.rainout_h1(dx, dz)),
                        n(fallout::total_dose(&m, seed, Some(&r), dx, dz)),
                        n(fallout::hours_to_dose(&m, seed, Some(&r), dx, dz, 100.0)),
                    ]);
                }
                "rain.rate" => {
                    out.push(n(fallout::dose_rate_with_rain(&m, seed, Some(&r), a.f(4), a.f(5), a.f(6))));
                }
                _ => out.push(n(r.strength(a.f(4)))),
            }
        }
        "fission" => {
            let m = FissionModel::new(a.i(0), a.i(1), a.b(2));
            let w = Warhead::of_fission(&m);
            out.extend([
                i(m.uranium_cores as i64),
                i(m.plutonium_cores as i64),
                n(m.mass_kg),
                n(m.alpha),
                n(m.efficiency),
                n(m.yield_kt),
                t(fstate(m.state)),
                b(m.nuclear()),
                n(w.yield_kt),
                n(w.fission_fraction),
                b(w.thermonuclear),
                b(w.fizzle),
            ]);
        }
        "format.yield" => out.push(t(FissionModel::format(a.f(0)))),
        "format.distance" => out.push(t(FissionModel::distance(a.f(0)))),
        "format.fixed" => out.push(t(kiloton_core::format::fixed(a.f(0), a.i(1) as usize))),
        "warhead.yield" => {
            let w = Warhead::of_yield(a.f(0), a.f(1));
            out.extend([n(w.yield_kt), n(w.fission_fraction), b(w.thermonuclear), b(w.fizzle), b(w.nuclear())]);
        }
        "thermo" => {
            let p = FissionModel::new(a.i(0), a.i(1), a.b(2));
            let m = ThermonuclearModel::new(p, a.i(3), a.b(4), a.b(5));
            let w = Warhead::of_thermonuclear(&m);
            out.extend([
                i(m.charges as i64),
                n(m.fuel_kg),
                n(m.compression),
                n(m.burn),
                n(m.fusion_kt),
                n(m.jacket_kt),
                n(m.yield_kt),
                t(tstate(m.state)),
                b(m.nuclear()),
                b(m.fusion()),
                n(m.fission_fraction()),
                n(m.primary_needed_kt()),
                n(w.yield_kt),
                n(w.fission_fraction),
                b(w.thermonuclear),
                b(w.fizzle),
            ]);
        }
        "ballistic.path" | "ballistic.at" | "interceptor.path" | "interceptor.at" => {
            let path = if name.starts_with("ballistic") {
                Ballistics::new(a.f(0), a.f(1), a.f(2), a.f(3), a.f(4), a.f(5))
            } else {
                Ballistics::interceptor(a.f(0), a.f(1), a.f(2), a.f(3), a.f(4), a.f(5), a.f(6))
            };
            if name.ends_with(".path") {
                out.extend([
                    n(path.range),
                    n(path.apex),
                    n(path.seconds),
                    b(path.interceptor),
                    n(path.burn_seconds()),
                    i(path.ticks() as i64),
                ]);
            } else {
                let tt = a.f(a.0.len() - 1);
                arr(&mut out, path.at(tt));
                arr(&mut out, path.velocity(tt));
                arr(&mut out, path.axis(tt));
                arr(&mut out, path.nozzle(tt));
                out.push(b(path.burning(tt)));
            }
        }
        "ballistics.seconds" => out.push(n(Ballistics::flight_seconds(a.f(0)))),
        "sam.rail" => out.extend([n(sam_geometry::rail_x(a.i(0))), n(sam_geometry::rail_y(a.i(0)))]),
        "sam.wrap" => out.push(n(sam_geometry::wrap(a.f(0)))),
        "sam.forward" => arr(&mut out, sam_geometry::forward(a.f(0), a.f(1))),
        "sam.railCentre" => {
            arr(&mut out, sam_geometry::rail_centre(a.f(0), a.f(1), a.f(2), a.f(3), a.f(4), a.i(5)));
        }
        "sam.local" => arr(
            &mut out,
            sam_geometry::local(a.f(0), a.f(1), a.f(2), a.f(3), a.f(4), a.f(5), a.f(6), a.f(7)),
        ),
        "sam.yaw" => out.push(n(sam_geometry::yaw(a.f(0), a.f(1), a.f(2), a.f(3)))),
        "sam.yawTowards" => out.push(n(sam_geometry::yaw_towards(a.f(0), a.f(1), a.f(2), a.f(3)))),
        "sam.raise" => out.push(n(sam_geometry::raise(a.f(0), a.b(1), a.f(2), a.f(3)))),
        "sam.elevation" => out.push(n(sam_geometry::elevation_of(a.f(0)))),
        "intercept" => {
            let path = Ballistics::new(a.f(0), a.f(1), a.f(2), a.f(3), a.f(4), a.f(5));
            match Intercept::solve(&path, a.f(6), a.f(7), a.f(8), a.f(9), a.f(10), a.f(11), a.f(12)) {
                None => out.push(t("null")),
                Some(m) => out.extend([t("some"), n(m.meet_age), n(m.flight), n(m.x), n(m.y), n(m.z)]),
            }
        }
        "radar.bin" => out.push(i(radar_horizon::bin(a.f(0), a.f(1)) as i64)),
        "radar.angle" => out.push(n(radar_horizon::angle(a.i(0), a.f(1)))),
        "radar.next" => out.push(n(radar_horizon::next(a.f(0)))),
        "radar.encode" => out.push(i(radar_horizon::encode(a.f(0)) as i64)),
        "radar.horizon" => unreachable!("read before evaluating"),
        "radar.above" => {
            let h = horizon.as_deref();
            let (x, y, z) = (a.f(0), a.f(1), a.f(2));
            out.push(b(radar_horizon::above(h, 0.5, 66.35, 0.5, x, y, z)));
            out.push(b(radar_horizon::above(None, 0.5, 66.35, 0.5, x, y, z)));
        }
        "radar.consts" => out.extend([
            n(radar_horizon::REACH),
            n(radar_horizon::START),
            n(radar_horizon::ANTENNA_Y),
        ]),
        "cloud.model" => {
            let c = MushroomCloud::new(BlastModel::new(a.f(0)), a.l(1), a.f(2));
            out.extend([
                n(c.top_height),
                n(c.cap_height),
                n(c.cap_radius),
                n(c.cap_half_height),
                n(c.stem_radius),
                n(c.tau_rise),
                n(c.tau_form),
                n(c.tau_heat),
                n(c.life),
                n(c.megaton),
            ]);
        }
        "cloud.curves" => {
            let c = MushroomCloud::new(BlastModel::new(a.f(0)), a.l(1), a.f(2));
            let t = a.f(3);
            out.extend([n(c.heat(t)), n(c.cap_centre(t)), n(c.fireball_centre(t)), n(c.flash(t))]);
        }
        "cloud.volume" => {
            let c = MushroomCloud::new(BlastModel::new(a.f(0)), a.l(1), a.f(2));
            match c.volume(a.f(3), 150.0, 40.0, -2500.0, &SkyLight::default()) {
                Some(p) => {
                    out.push(V::Text("some".into()));
                    out.extend(p.iter().map(|&x| n(x as f64)));
                }
                None => out.push(V::Text("none".into())),
            }
        }
        "viewer.shake" => {
            let m = BlastModel::new(a.f(0));
            let d = a.f(2);
            out.push(n(viewer::shake(&m, a.f(1), d * 0.9, d, a.b(3))));
        }
        "viewer.light" => {
            let m = BlastModel::new(a.f(0));
            let c = MushroomCloud::new(BlastModel::new(a.f(0)), 7, 0.75);
            let (t, d) = (a.f(1), a.f(2));
            out.extend([
                n(viewer::whiteout(&c, t, d, 0.6, 1.0)),
                n(viewer::whiteout(&c, t, d, -0.4, 0.0)),
                n(viewer::glow(&c, t, d)),
                n(viewer::tremor(&m, d)),
                n(viewer::tremor_time(&m, d * 0.9, d)),
            ]);
        }
        "wilson" => {
            let w = wilson::params(&BlastModel::new(a.f(0)), a.f(1), 0.8, 1.0);
            out.extend(w.iter().map(|&x| n(x as f64)));
        }
        "viewer.noise" => out.extend([n(viewer::noise(a.f(0), 1.3)), n(viewer::noise(a.f(0), 7.7))]),
        "sky" => {
            let mut k = SkyLight::default();
            k.from_sun(a.f(0) as f32, 0.7, a.f(1) as f32);
            out.extend(
                [k.sun_x, k.sun_y, k.sun_z, k.sun_r, k.sun_g, k.sun_b, k.daylight, k.amb_r, k.amb_g, k.amb_b]
                    .iter()
                    .map(|&x| n(x as f64)),
            );
        }
        "burst" => {
            let d = BurstDetection::of(a.f(0), a.f(1), a.l(2));
            out.extend([b(d.optical), n(d.estimate_kt), n(d.error), n(d.delay)]);
        }
        "ring.all" => {
            let mut rc = RingColumns::default();
            rc.add_ring(a.i(0));
            out.extend([i(rc.len() as i64), i(fnv(&rc.cols))]);
        }
        "ring.chunk" => {
            let (bx, bz, cx, cz, r) = (a.i(0), a.i(1), a.i(2), a.i(3), a.i(4));
            let mut rc = RingColumns::default();
            rc.add_chunk(bx, bz, cx, cz, r);
            out.extend([b(RingColumns::crosses(bx, bz, cx, cz, r)), i(rc.len() as i64), i(fnv(&rc.cols))]);
        }
        "ring.pack" => {
            let p = ring_columns::pack(a.i(0), a.i(1));
            out.extend([i(p), i(ring_columns::dx(p) as i64), i(ring_columns::dz(p) as i64)]);
        }
        "grid.new" => {
            let g = LateGrid::for_radius(a.i(0), a.i(1), a.i(2));
            out.extend([
                i(g.cx0 as i64),
                i(g.cz0 as i64),
                i(g.r as i64),
                i(g.count() as i64),
                i(g.bytes() as i64),
            ]);
        }
        "grid.script" | "grid.load" => {
            let (lg, log) = grid_script(a.i(0), a.i(1), a.i(2));
            let bits = lg.bits();
            if name == "grid.script" {
                out.extend([t(log), i(lg.count() as i64), i(fnv(&bits))]);
            } else {
                let side = 2 * lg.r + 1;
                let back = LateGrid::load(lg.cx0, lg.cz0, lg.r, &bits);
                out.extend([i(back.count() as i64), i(back.next(0) as i64), i(back.next(side * side / 2) as i64)]);
            }
        }
        other => panic!("no Rust counterpart for table {other}"),
    }
    out
}

/// FNV-1a over longs, as `ParityTables.fnv`.
fn fnv(a: &[i64]) -> i64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &v in a {
        h ^= v as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h as i64
}

/// `ParityTables.lateGrid`'s fixed script of operations on a grid, with its log.
fn grid_script(bx: i32, bz: i32, radius: i32) -> (LateGrid, String) {
    let mut lg = LateGrid::for_radius(bx, bz, radius);
    let side = (2 * lg.r + 1) as u64;
    let mut log = String::new();
    let mut seed: i64 = 12345 + radius as i64;
    for _ in 0..400 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let s = seed as u64;
        let op = (s >> 33) % 5;
        let cx = lg.cx0 - lg.r - 2 + ((s >> 13) % (side + 4)) as i32;
        let cz = lg.cz0 - lg.r - 2 + ((s >> 41) % (side + 4)) as i32;
        let idx = lg.index(cx, cz);
        match op {
            0 => log.push(if lg.set(idx) { 'S' } else { 's' }),
            1 => log.push(if lg.clear(idx) { 'C' } else { 'c' }),
            2 => log.push(if lg.get(idx) { 'G' } else { 'g' }),
            3 => {
                lg.set_row(cx, cz, cz + ((s >> 7) % 70) as i32);
                log.push('R');
            }
            _ => {
                log.push_str(&lg.next(idx).to_string());
                log.push(';');
            }
        }
        if idx >= 0 {
            log.push(if lg.chunk_x(idx) == cx && lg.chunk_z(idx) == cz { '+' } else { '!' });
        }
    }
    (lg, log)
}

#[test]
fn chunk_lists_make_the_whole_ring() {
    // add_chunk over every chunk the ring crosses gives exactly add_ring's columns.
    for (bx, bz) in [(0, 0), (7, -3), (-1000, 2500)] {
        for r in [0, 1, 5, 16, 63, 200] {
            let mut whole = RingColumns::default();
            whole.add_ring(r);
            let mut parts = RingColumns::default();
            let cr = (r >> 4) + 2;
            for cx in (bx >> 4) - cr..=(bx >> 4) + cr {
                for cz in (bz >> 4) - cr..=(bz >> 4) + cr {
                    let before = parts.len();
                    parts.add_chunk(bx, bz, cx, cz, r);
                    if parts.len() > before {
                        assert!(RingColumns::crosses(bx, bz, cx, cz, r), "ring {r} has columns in a chunk it does not cross");
                    }
                }
            }
            let mut a = whole.cols.clone();
            let mut b = parts.cols.clone();
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a, b, "ring {r} around ({bx}, {bz})");
        }
    }
}

#[derive(Default)]
struct Stats {
    lines: usize,
    worst_rel: f64,
    worst_line: String,
    exact: usize,
    values: usize,
}

#[test]
fn physics_matches_java() {
    let Some(path) = std::env::var_os("KILOTON_PARITY_TABLE") else {
        panic!(
            "KILOTON_PARITY_TABLE is not set: run `python tools/hz.py test` (it writes the Java table and points here)"
        );
    };
    let text = std::fs::read_to_string(&path).expect("reading the parity table");
    let mut stats: BTreeMap<String, Stats> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut horizon: Option<Vec<i8>> = None;
    for (no, line) in text.lines().enumerate() {
        let fields: Vec<&str> = line.split('\t').collect();
        let bar = fields.iter().position(|f| *f == "|").expect("a | in every line");
        let name = fields[0];
        let args = Args(&fields[1..bar]);
        let expected = &fields[bar + 1..];
        if name == "radar.horizon" {
            horizon = Some(expected[0].trim_end_matches(',').split(',').map(|v| v.parse().unwrap()).collect());
            continue;
        }
        let got = eval(name, &args, &mut horizon);
        let (rel, abs) = tolerance(name);
        let st = stats.entry(name.to_string()).or_default();
        st.lines += 1;
        if got.len() != expected.len() {
            failures.push(format!("line {}: {name}: {} values, Java {}", no + 1, got.len(), expected.len()));
            continue;
        }
        for (k, (g, e)) in got.iter().zip(expected).enumerate() {
            st.values += 1;
            let ok = match g {
                V::Num(x) => {
                    let y: f64 = e.parse().unwrap_or(f64::NAN);
                    let (rel, abs) = if f32_columns(name).contains(&k) { (2e-7, 1e-9) } else { (rel, abs) };
                    if x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()) || x == &y {
                        st.exact += 1;
                        true
                    } else {
                        let diff = (x - y).abs();
                        let scale = x.abs().max(y.abs());
                        let r = if scale > 0.0 { diff / scale } else { 0.0 };
                        if r > st.worst_rel {
                            st.worst_rel = r;
                            st.worst_line = format!("line {} col {k}: Rust {x:?}, Java {y:?}", no + 1);
                        }
                        diff <= abs.max(rel * scale)
                    }
                }
                V::Text(s) => {
                    let same = s == e;
                    if same {
                        st.exact += 1;
                    }
                    same
                }
            };
            if !ok {
                failures.push(format!("line {}: {name} col {k}: Rust {}, Java {e}  [{line}]", no + 1, g.show()));
            }
        }
    }
    let mut total = 0;
    for (name, st) in &stats {
        total += st.lines;
        println!(
            "{name:20} {:6} lines, {:7} values, {:7} bit-exact, worst relative error {:.3e} {}",
            st.lines, st.values, st.exact, st.worst_rel, st.worst_line
        );
    }
    println!("{total} lines compared, {} failures", failures.len());
    for f in failures.iter().take(40) {
        println!("FAIL {f}");
    }
    assert!(total > 40000, "the table looks truncated: {total} lines");
    assert!(failures.is_empty(), "{} values differ from the Java edition", failures.len());
}
