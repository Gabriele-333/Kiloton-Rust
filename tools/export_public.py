"""Copies the publishable part of this folder into the public repository's working copy.

    python tools/export_public.py [TARGET]

TARGET defaults to Kiloton-Rust beside the Kiloton project folder. Left out: run/ (the development game folder),
PORT.md (working notes), parity/build and parity/out (generated), caches. Files under the copied folders that are
gone from here are removed from the target; its own root files (README, LICENSE, .gitignore) stay as they are.
"""

import shutil
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parents[1]
DST = Path(sys.argv[1]) if len(sys.argv) > 1 else SRC.parent.parent / "Kiloton-Rust"
TOP = ["kiloton", "content", "parity", "tools", "dev"]
SKIP_DIRS = {"run", "build", "out", "__pycache__", "target", ".horizon-cache"}


def main() -> None:
    DST.mkdir(exist_ok=True)
    wanted = set()
    for top in TOP:
        for p in (SRC / top).rglob("*"):
            rel = p.relative_to(SRC)
            if any(part in SKIP_DIRS for part in rel.parts) or not p.is_file():
                continue
            wanted.add(rel)
            out = DST / rel
            out.parent.mkdir(parents=True, exist_ok=True)
            if not out.exists() or out.read_bytes() != p.read_bytes():
                shutil.copy2(p, out)
    for top in TOP:
        base = DST / top
        if not base.exists():
            continue
        for p in sorted(base.rglob("*"), reverse=True):
            rel = p.relative_to(DST)
            if p.is_file() and rel not in wanted:
                p.unlink()
            elif p.is_dir() and not any(p.iterdir()):
                p.rmdir()
    print(f"{len(wanted)} files in {DST}")


if __name__ == "__main__":
    main()
