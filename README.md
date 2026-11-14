# Kiloton for Horizon

Kiloton, the nuclear explosion mod for Minecraft, as a native Rust mod for
[Horizon](https://github.com/ludovic111/horizon) (Minecraft Java 26.3 rewritten in Rust).

- **The blast**: pressure, heat and fireball from Glasstone & Dolan, shadowed by hills; craters, ejecta, felled
  trees, fires, fallout and radiation. Chunks loaded later get the damage they are owed.
- **The cloud**: a volumetric mushroom cloud in WGSL, with the flash, the afterimage and the Wilson cloud.
- **The bombs**: fission and thermonuclear devices with their panel, cores, tamper, timer, and arming from the panel or
  by redstone.
- **The industry**: uranium mill, gas centrifuge cascade, breeder reactor, core foundry, exchange column, energy
  cables.

Work in progress: a port of the Java edition (NeoForge, Fabric, Forge). Missiles, air defence, shelters and the hazmat
gear are not ported yet.

## Layout

| Folder | What |
| --- | --- |
| `kiloton/` | The mod: `horizon-mod.toml`, `horizon-native.json`, assets, data, and `mixins/kiloton`: the `api` crate (the hooks), `kiloton-core` (the physics) and `glue` (a horizon-mixins support module and the injection that calls it, for what the hook API doesn't expose yet) |
| `content/` | Generated registry files that Horizon reads |
| `tools/` | Development tools: build, launch, tests. The resource sync and registry generation read the Java edition's sources, which are not in this repository |
| `dev/` | Development-only crates, never packed into the mod: the WGSL check against Horizon's own naga |
| `parity/` | The table generator that checks `kiloton-core` against the Java physics |

The mod builds the way players get it: `horizon-launcher native-build kiloton --trust-native-code`, which applies
the bundle with horizon-mixins at the pinned commit and compiles the game with it.

For development, `HORIZON_SRC=<checkout> python tools/hz.py build` does the same in a Horizon checkout at the commit
pinned in `kiloton/horizon-native.json`: horizon-mixins (installed once with `cargo install` at the pinned revision)
applies the bundle to a clean export of that commit, and the changed files go into the checkout, so the engine is
compiled once and later edits rebuild only what they touch. `python tools/hz.py test` runs the physics and parity
tests and the shader check; `python tools/hz.py unstage` puts the checkout back.

## License

All Rights Reserved. © Gabriele_333, SaturnX Studios. The source is public to be read and reviewed; copying,
redistributing or modifying it needs written permission.
