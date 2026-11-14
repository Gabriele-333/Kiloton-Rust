"""Packs the mod into one registry file: kiloton-<version>.hmod with its content and its native code.

    python tools/pack_hmod.py [OUT_DIR]

The file is a zip (docs/api/mod-files.md in Horizon): `horizon-mod.toml`, `horizon-native.json`, `assets/`, `data/`
and `mixins/kiloton/` (only `.rs`, `.toml`, `data/` folders and the README/LICENSE kind of files). Check it with
`cargo horizon-mod check <file>` before a submission. OUT_DIR defaults to dist/ here.
"""

import json
import sys
import tomllib
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
MOD = HERE / "kiloton"
TEXT_FILES = ("README", "LICENSE", "LICENCE", "COPYING", "NOTICE", ".gitignore")


def native_allowed(rel: Path) -> bool:
    """docs/api/mod-files.md, `Under mixins/<id>/`."""
    parts = rel.parts
    if rel.suffix in (".rs", ".toml"):
        return True
    if parts[0] == "data" or (len(parts) > 2 and parts[1] == "data"):
        return True
    return len(parts) <= 2 and rel.name.startswith(TEXT_FILES)


def main() -> None:
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "dist"
    native = json.loads((MOD / "horizon-native.json").read_text(encoding="utf-8"))
    manifest = tomllib.loads((MOD / "horizon-mod.toml").read_text(encoding="utf-8"))
    mixin = tomllib.loads((MOD / "mixins" / native["id"] / "horizon-mixin.toml").read_text(encoding="utf-8"))
    assert manifest["id"] == native["id"] == mixin["mod"]["id"], "the three manifests name different ids"
    assert manifest["version"] == native["version"] == mixin["mod"]["version"], "the three manifests name different versions"
    assert native["horizon"] == mixin["compatibility"]["horizon_revision"], "horizon-native.json and horizon-mixin.toml pin different commits"
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / f"{native['id']}-{native['version']}.hmod"
    entries: list[tuple[str, Path]] = [("horizon-mod.toml", MOD / "horizon-mod.toml"), ("horizon-native.json", MOD / "horizon-native.json")]
    for top in ("assets", "data"):
        for p in sorted((MOD / top).rglob("*")):
            if p.is_file():
                entries.append((p.relative_to(MOD).as_posix(), p))
    bundle = MOD / "mixins" / native["id"]
    left_out = []
    for p in sorted(bundle.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(bundle)
        if "target" in rel.parts:
            continue
        if native_allowed(rel):
            entries.append((f"mixins/{native['id']}/{rel.as_posix()}", p))
        else:
            left_out.append(rel.as_posix())
    if left_out:
        sys.exit(f"not allowed in a registry bundle (move it under a data/ folder): {left_out}")
    partial = out.with_suffix(".hmod.partial")
    with zipfile.ZipFile(partial, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name, path in entries:
            z.write(path, name)
    partial.replace(out)
    natives = sum(1 for n, _ in entries if not n.startswith(("assets/", "data/", "horizon-mod.toml")))
    print(f"{out} ({out.stat().st_size / 1e6:.1f} MB, {len(entries)} files, {natives} native)")


if __name__ == "__main__":
    main()
