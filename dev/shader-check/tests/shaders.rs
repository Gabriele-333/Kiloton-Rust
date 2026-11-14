//! The WGSL the client draws with (api/data/*.wgsl) parses and validates with naga, the compiler
//! wgpu uses, so a mistake shows here and not as a failed pipeline in the game.

use std::path::{Path, PathBuf};

fn shader_dir() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The source tree (horizon-26.3/dev/shader-check) or the copy hz.py stages into Horizon
    // (native-crates/kiloton-dev/shader-check, beside native-api/kiloton).
    for c in [
        here.join("../../kiloton/mixins/kiloton/api/data"),
        here.join("../../../native-api/kiloton/data"),
    ] {
        if c.join("kiloton_cloud.wgsl").exists() {
            return c;
        }
    }
    panic!("api/data/kiloton_cloud.wgsl not found from {}", here.display());
}

fn validate(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(source)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
}

#[test]
fn cloud_shaders_validate() {
    let dir = shader_dir();
    let volume = std::fs::read_to_string(dir.join("kiloton_cloud.wgsl")).unwrap();
    validate("kiloton_cloud.wgsl", &volume);
    let composite = std::fs::read_to_string(dir.join("kiloton_cloud_composite.wgsl")).unwrap();
    validate("kiloton_cloud_composite.wgsl", &composite);
    validate("kiloton_cloud_composite.wgsl (linear)", &composite.replacen("const LINEAR: bool = false;", "const LINEAR: bool = true;", 1));
    let flash = std::fs::read_to_string(dir.join("kiloton_flash.wgsl")).unwrap();
    validate("kiloton_flash.wgsl", &flash);
    let shadow = std::fs::read_to_string(dir.join("kiloton_shadow.wgsl")).unwrap();
    validate("kiloton_shadow.wgsl", &shadow);
}
