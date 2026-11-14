"""Copy Kiloton's resources from the Java edition into the Horizon mod's content (kiloton/assets, kiloton/data).

    python tools/sync_resources.py           copy what changed, remove what the sources no longer have
    python tools/sync_resources.py --check   only say what would change (exit 1 if anything would)

The Java edition stays the source: texture, model, sound, language, recipe, loot, tag, advancement and
worldgen files are made there (tools/gen_assets.py, gen_sounds.py, gen_data.py) and copied here, never
edited here. The 26.3 build of the Java edition reads common/ and shared-26.3/ together (their paths
never overlap), plus NeoForge's biome modifiers, which Horizon reads as they are (docs/mods.md). The
few files only Horizon reads are kept in horizon-26.3/content/ and copied the same way.

Left out: the GLSL shaders of the Java renderer (assets/kiloton/shaders/: the Horizon port draws with
WGSL) and the loaders' own files (mixin configs, mod metadata).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent  # horizon-26.3/
KILOTON = HERE.parent
OUT = HERE / "kiloton"

SOURCES = [
    KILOTON / "common" / "src" / "main" / "resources",
    KILOTON / "shared-26.3" / "src" / "main" / "resources",
    # Only its data (the biome modifiers that put the ores in the world).
    KILOTON / "neoforge-26.3" / "src" / "main" / "resources",
    # What only Horizon reads (data/kiloton/horizon/: the creative tab), kept with the port.
    HERE / "content",
]
SKIP = [
    re.compile(r"^assets/kiloton/shaders/"),
    # The Java fungus's GLSL include (a mod may not add files to vanilla's namespace except tags and atlases).
    re.compile(r"^assets/minecraft/shaders/"),
]
# A content path: assets/ or data/, then at least two parts of vanilla's resource-location characters.
VALID = re.compile(r"^(assets|data)(/[a-z0-9_.-]+){2,}$")


def collect() -> dict[str, Path]:
    files: dict[str, Path] = {}
    for root in SOURCES:
        for f in root.rglob("*"):
            if not f.is_file():
                continue
            rel = f.relative_to(root).as_posix()
            if not rel.startswith(("assets/", "data/")) or any(p.search(rel) for p in SKIP):
                continue
            if not VALID.match(rel):
                raise SystemExit(f"not a valid content path: {rel} (in {root})")
            if rel in files:
                raise SystemExit(f"{rel} is in both {files[rel]} and {f}")
            files[rel] = f
    return files


def main() -> None:
    check = "--check" in sys.argv
    files = collect()
    added = changed = removed = 0
    for rel, src in sorted(files.items()):
        dst = OUT / rel
        data = src.read_bytes()
        if not dst.exists():
            added += 1
        elif dst.read_bytes() != data:
            changed += 1
        else:
            continue
        if not check:
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_bytes(data)
    for top in ("assets", "data"):
        base = OUT / top
        if not base.exists():
            continue
        for f in sorted(base.rglob("*"), reverse=True):
            rel = f.relative_to(OUT).as_posix()
            if f.is_file() and rel not in files:
                removed += 1
                if not check:
                    f.unlink()
            elif f.is_dir() and not check and not any(f.iterdir()):
                f.rmdir()
    verb = "would change" if check else "synced"
    print(f"{verb}: {len(files)} files, {added} new, {changed} changed, {removed} removed")
    if check and (added or changed or removed):
        sys.exit(1)


if __name__ == "__main__":
    main()
