"""Creature test for the blast: cows in the open, in a sealed stone room, behind a wall and at the end of
a bent tunnel, at the same distance from ground zero; then a burst, and what is left of each.

    python tools/ingame/creatures.py GX GY GZ KT DIST

The game must be running under `tools/hz.py launch` (creative player, who is not touched).
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


def cmd(c: str) -> None:
    ctl("command", c)


def main() -> None:
    gx, gy, gz, kt, dist = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]), sys.argv[4], int(sys.argv[5])
    y = gy + 30  # a platform in the air, clear of the terrain
    x0 = gx + dist
    # Platform of stone, 40 x 40, at the test distance.
    cmd(f"fill {x0 - 4} {y - 1} {gz - 20} {x0 + 30} {y - 1} {gz + 20} minecraft:stone")
    cmd(f"fill {x0 - 4} {y} {gz - 20} {x0 + 30} {y + 8} {gz + 20} minecraft:air")
    # 1: in the open.
    cmd(f"summon minecraft:cow {x0} {y} {gz - 12} {{NoAI:1b,CustomName:'open'}}")
    # 2: a sealed room of stone (walls 2 thick).
    cmd(f"fill {x0 + 6} {y} {gz - 3} {x0 + 12} {y + 6} {gz + 3} minecraft:stone")
    cmd(f"fill {x0 + 8} {y + 2} {gz - 1} {x0 + 10} {y + 4} {gz + 1} minecraft:air")
    cmd(f"summon minecraft:cow {x0 + 9} {y + 2} {gz} {{NoAI:1b,CustomName:'sealed'}}")
    # 3: behind a wall (the flash can't see it, the wave still gets round).
    cmd(f"fill {x0 + 2} {y} {gz + 8} {x0 + 2} {y + 4} {gz + 14} minecraft:stone")
    cmd(f"summon minecraft:cow {x0 + 4} {y} {gz + 11} {{NoAI:1b,CustomName:'wall'}}")
    # 4: a room with a bent tunnel out (the wave gets in only through the air).
    cmd(f"fill {x0 + 16} {y} {gz - 3} {x0 + 26} {y + 6} {gz + 3} minecraft:stone")
    cmd(f"fill {x0 + 22} {y + 2} {gz - 1} {x0 + 24} {y + 4} {gz + 1} minecraft:air")
    cmd(f"fill {x0 + 17} {y + 2} {gz} {x0 + 21} {y + 2} {gz} minecraft:air")
    cmd(f"fill {x0 + 17} {y + 2} {gz} {x0 + 17} {y + 2} {gz + 4} minecraft:air")
    cmd(f"summon minecraft:cow {x0 + 23} {y + 2} {gz} {{NoAI:1b,CustomName:'tunnel'}}")
    cmd(f"summon minecraft:item {x0} {y} {gz + 16} {{Item:{{id:'minecraft:apple',count:1}}}}")
    ctl("wait", "20")
    cmd(f"tp @s {x0 + 10} {y + 15} {gz + 30}")
    ctl("wait", "40")
    before = ctl("entities", "64")
    print("before:", [(e.get("type"), e.get("name"), round(e.get("health", -1), 1)) for e in before.get("entities", [])])
    cmd(f"nuke {kt} {gx} {gy + 60} {gz}")
    ctl("wait", "200")
    after = ctl("entities", "64")
    print("after:", [(e.get("type"), e.get("name"), round(e.get("health", -1), 1), [round(v, 1) for v in e.get("position", [])])
                     for e in after.get("entities", [])])


if __name__ == "__main__":
    main()
