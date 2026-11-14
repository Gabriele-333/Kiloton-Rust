"""Builds a test scene around a point of the development world, through horizon-ctl: oak trees in a
grid and small houses (planks, stone bricks, bricks, with glass windows) at set distances along +x.

    python tools/ingame/scene.py GX GZ [radius]

The game must be running under `tools/hz.py launch`; the chunks must be loaded (stand nearby).
"""

import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parents[2]
# The Horizon checkout: HORIZON_SRC, or _ext/horizon beside the Kiloton project folder (as tools/hz.py).
CTL = Path(os.environ.get("HORIZON_SRC", str(HERE.parent.parent / "_ext" / "horizon"))) / "target" / "fast" / "horizon-ctl.exe"
SOCKET = HERE / "run" / "control.json"


def ctl(*args: str) -> dict:
    out = subprocess.run([str(CTL), "--socket", str(SOCKET), *args], capture_output=True, text=True).stdout
    try:
        return json.loads(out.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return {}


def ground(x: int, z: int) -> int | None:
    """The y of the highest solid ground block (grass, dirt, sand, stone), or None."""
    r = ctl("blocks", str(x), "40", str(z), str(x), "120", str(z))
    best = None
    for bx, by, bz, name in r.get("blocks", []):
        n = name.split("[")[0]
        if n in ("minecraft:grass_block", "minecraft:dirt", "minecraft:sand", "minecraft:stone", "minecraft:gravel",
                 "minecraft:coarse_dirt", "minecraft:podzol"):
            best = by if best is None else max(best, by)
    return best


def main() -> None:
    gx, gz = int(sys.argv[1]), int(sys.argv[2])
    radius = int(sys.argv[3]) if len(sys.argv) > 3 else 160
    trees = houses = 0
    for dx in range(-radius, radius + 1, 9):
        for dz in range(-radius, radius + 1, 9):
            if dx * dx + dz * dz < 30 * 30 or dx * dx + dz * dz > radius * radius:
                continue
            if 70 <= dx <= 140 and abs(dz) < 12:
                continue  # the street of houses
            x, z = gx + dx, gz + dz
            g = ground(x, z)
            if g is None:
                continue
            r = ctl("command", f"place feature minecraft:oak {x} {g + 1} {z}")
            if any("Placed" in c for c in r.get("chat", [])):
                trees += 1
    for k, (dist, wall) in enumerate([(75, "minecraft:oak_planks"), (95, "minecraft:stone_bricks"),
                                      (115, "minecraft:bricks"), (135, "minecraft:oak_planks")]):
        x0, z0 = gx + dist, gz - 3
        g = ground(x0 + 3, z0 + 3)
        if g is None:
            continue
        y0 = g + 1
        ctl("command", f"fill {x0} {y0} {z0} {x0 + 6} {y0 + 4} {z0 + 6} {wall} hollow")
        ctl("command", f"fill {x0} {y0 + 5} {z0} {x0 + 6} {y0 + 5} {z0 + 6} minecraft:oak_planks")
        ctl("command", f"fill {x0 + 2} {y0 + 2} {z0} {x0 + 4} {y0 + 3} {z0} minecraft:glass_pane")
        ctl("command", f"fill {x0 + 2} {y0 + 2} {z0 + 6} {x0 + 4} {y0 + 3} {z0 + 6} minecraft:glass_pane")
        houses += 1
    print(json.dumps({"trees": trees, "houses": houses}))


if __name__ == "__main__":
    main()
