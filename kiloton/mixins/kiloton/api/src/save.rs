//! The bursts' records in the world's save (Java: `BlastSavedData`, `data/kiloton_blasts.dat`):
//! `<world>/kiloton/blasts.json`, one record per burst that still owes damage to chunks nobody has
//! loaded, running sweeps included (they owe what they have not reached yet).

use crate::blast::Record;
use kiloton_core::blast::LateGrid;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn file(world: &Path) -> PathBuf {
    world.join("kiloton").join("blasts.json")
}

fn to_json(r: &Record) -> Value {
    let partial: Vec<Value> = r.partial.iter().map(|(&(cx, cz), m)| json!([cx, cz, m[0], m[1], m[2], m[3]])).collect();
    json!({
        "id": r.id,
        "dimension": r.dimension,
        "x": r.x, "y": r.y, "z": r.z,
        "kt": r.kt,
        "fission": r.fission,
        "seed": r.seed,
        "radius": r.radius,
        "sea": r.sea,
        "ground_kind": r.ground_kind,
        "horizon": r.horizon,
        "horizon_dist": r.horizon_dist,
        "pending": { "cx0": r.pending.cx0, "cz0": r.pending.cz0, "r": r.pending.r, "bits": r.pending.bits() },
        "partial": partial,
    })
}

fn from_json(v: &Value) -> Option<Record> {
    let i = |k: &str| v.get(k).and_then(Value::as_i64);
    let f = |k: &str| v.get(k).and_then(Value::as_f64);
    let floats = |k: &str| -> Vec<f32> {
        v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect()).unwrap_or_default()
    };
    let p = v.get("pending")?;
    let bits: Vec<i64> = p.get("bits")?.as_array()?.iter().filter_map(Value::as_i64).collect();
    let pending = LateGrid::load(
        p.get("cx0")?.as_i64()? as i32,
        p.get("cz0")?.as_i64()? as i32,
        p.get("r")?.as_i64()? as i32,
        &bits,
    );
    let mut partial = HashMap::new();
    for e in v.get("partial").and_then(Value::as_array).into_iter().flatten() {
        let a = e.as_array()?;
        let n = |k: usize| a.get(k).and_then(Value::as_u64).unwrap_or(0);
        partial.insert((a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32), [n(2), n(3), n(4), n(5)]);
    }
    Some(Record {
        id: i("id")? as u32,
        dimension: v.get("dimension")?.as_str()?.to_owned(),
        x: i("x")? as i32,
        y: i("y")? as i32,
        z: i("z")? as i32,
        kt: f("kt")?,
        fission: f("fission")?,
        seed: i("seed")?,
        radius: i("radius")? as i32,
        sea: v.get("sea").and_then(Value::as_bool).unwrap_or(false),
        ground_kind: i("ground_kind").unwrap_or(2) as u8,
        horizon: floats("horizon"),
        horizon_dist: floats("horizon_dist"),
        pending,
        partial,
    })
}

/// Every record in the save.
pub fn read(world: &Path) -> Vec<Record> {
    let Ok(text) = std::fs::read_to_string(file(world)) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        log::warn!("Kiloton: {} doesn't read; the damage it owed is lost", file(world).display());
        return Vec::new();
    };
    v.get("records").and_then(Value::as_array).into_iter().flatten().filter_map(from_json).collect()
}

/// Writes the records of `dimensions` (replacing theirs in the save, keeping the other levels').
pub fn write(world: &Path, dimensions: &[String], records: &[Record]) {
    let mut all: Vec<Record> = read(world).into_iter().filter(|r| !dimensions.contains(&r.dimension)).collect();
    all.extend(records.iter().cloned());
    let path = file(world);
    if all.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let v = json!({ "version": 1, "records": all.iter().map(to_json).collect::<Vec<_>>() });
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Written beside and moved over, so a crash mid-write leaves the old save whole.
    let tmp = path.with_extension("json.tmp");
    match std::fs::write(&tmp, v.to_string()).and_then(|_| std::fs::rename(&tmp, &path)) {
        Ok(()) => {}
        Err(e) => log::warn!("Kiloton: could not write {}: {e}", path.display()),
    }
}
