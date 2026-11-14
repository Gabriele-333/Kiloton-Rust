"""Kiloton for Horizon: the development loop, from a Horizon source checkout.

    python tools/hz.py stage            mirror the mod into the Horizon workspace
    python tools/hz.py build            stage, then compile the game with the mod (and horizon-ctl)
    python tools/hz.py test             stage, write the Java parity table, run kiloton-core's tests and
                                        the shader check (dev/shader-check, never in the bundle)
    python tools/hz.py run [game args]  start the built game with Kiloton's content (--dev-mod)
    python tools/hz.py launch [args]    the same under horizon-ctl's control (--world NAME --creative ...)
    python tools/hz.py ctl <command>    a horizon-ctl command to that game (command "nuke info 15", screenshot F, quit)
    python tools/hz.py unstage          take the mod back out of the Horizon workspace

The game is built the way Horizon's launcher builds a native mod (launcher/src/native_mods.rs), in the
checkout's own `target/`, so the engine is compiled once and only what changed is redone after an edit:

1. horizon-mixins itself, at the commit `horizon-native.json` pins (`mixins`), built once with
   `cargo install` into `_ext/tools/` (HORIZON_MIXINS names another), applies the bundle to a pristine
   export of the checkout's commit (`_ext/hz-kiloton/`), as the launcher applies it to GitHub's archive:
   injections, support modules and the [[crate]]. The files it changed or added are copied into the
   checkout, only when their bytes differ, so Cargo's timestamps stay meaningful. Its errors are the
   launcher's and the registry reviewers': nothing here imitates it.
2. Then what the launcher adds itself (native_mods/api.rs): the [api] crate in `native-api/kiloton`, a
   generated `horizon-native-game` package that links it and calls its hooks(), and their workspace
   members. Trait injections (`trait_method`) are the launcher's to turn into helpers
   (native_mods/inject.rs): hz.py refuses them, `horizon-launcher native-build` builds those.

`unstage` puts every file back (`git checkout` for Horizon's own, the added ones removed).

`launch` moves the game's window off the desktop and hands the focus back to the window that had it
(tools/ingame/offscreen.ps1;
HZ_ONSCREEN=1 keeps it where it opens), turns off the pause on lost focus and starts the game muted
(master volume 0; HZ_SOUND=1 keeps the sound).

Environment: HORIZON_SRC (default _ext/horizon beside the Kiloton project folder), CARGO (default
%USERPROFILE%\\.cargo\\bin\\cargo.exe), JAVA_HOME (default the JDK 25 install).
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import time
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent  # horizon-26.3/
BUNDLE = HERE / "kiloton"  # the mod: content + mixins/kiloton
MIXINS = BUNDLE / "mixins" / "kiloton"
KILOTON = HERE.parent  # the Kiloton project (common/, shared-26.3/, ...)
RUN = HERE / "run"  # the game folder for development: saves, options.txt, screenshots
HORIZON = Path(os.environ.get("HORIZON_SRC", str(KILOTON.parent / "_ext" / "horizon")))
CARGO = os.environ.get("CARGO", str(Path.home() / ".cargo" / "bin" / "cargo.exe"))
JAVA_HOME = Path(os.environ.get("JAVA_HOME", r"C:\Program Files\Java\jdk-25.0.4.1"))
PROFILE = os.environ.get("HZ_PROFILE", "fast")

MOD_ID = "kiloton"
GAME = "crates/native-game"
API_DIR = f"native-api/{MOD_ID}"
CRATES_DIR = f"native-crates/{MOD_ID}"
# Development-only crates (dev/<name>): staged as workspace members so they build against Horizon's
# own patched crates (naga), never packed into the .hmod.
DEV = HERE / "dev"
DEV_DIR = f"native-crates/{MOD_ID}-dev"
DEV_CRATES = ["shader-check"]


def log(msg: str) -> None:
    print(f"[hz] {msg}", flush=True)


def write_if_changed(path: Path, data: bytes) -> bool:
    if path.exists() and path.read_bytes() == data:
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return True


def mirror(src: Path, dst: Path, rewrite=None) -> int:
    """Copy src's files into dst when their bytes differ; remove files dst has that src lacks."""
    changed = 0
    wanted = set()
    for f in src.rglob("*"):
        if f.is_dir() or "target" in f.relative_to(src).parts or f.name == "Cargo.lock":
            continue
        rel = f.relative_to(src)
        wanted.add(rel)
        data = f.read_bytes()
        if rewrite is not None:
            data = rewrite(rel, data)
        changed += write_if_changed(dst / rel, data)
    if dst.exists():
        for f in sorted(dst.rglob("*"), reverse=True):
            rel = f.relative_to(dst)
            if f.is_file() and rel not in wanted:
                f.unlink()
                changed += 1
            elif f.is_dir() and not any(f.iterdir()):
                f.rmdir()
    return changed


def crates() -> list[tuple[str, str]]:
    manifest = tomllib.loads((MIXINS / "horizon-mixin.toml").read_text(encoding="utf-8"))
    return [(c["id"], c["path"]) for c in manifest.get("crate", [])]


def api_folder() -> str:
    manifest = tomllib.loads((MIXINS / "horizon-mixin.toml").read_text(encoding="utf-8"))
    return manifest["api"]["path"]


def rewrite_api_manifest(rel: Path, data: bytes) -> bytes:
    """Point the [api] crate at the workspace's horizon-client and at the staged [[crate]]s, as the
    launcher does (game features are the generated package's: default-features = false)."""
    if rel.as_posix() != "Cargo.toml":
        return data
    text = data.decode("utf-8")
    text = re.sub(
        r"(?m)^horizon-client\s*=.*$",
        'horizon-client = { path = "../../crates/horizon-client", default-features = false }',
        text,
    )
    for cid, cpath in crates():
        package = tomllib.loads((MIXINS / cpath / "Cargo.toml").read_text(encoding="utf-8"))["package"]["name"]
        text = re.sub(
            rf"(?m)^{re.escape(package)}\s*=.*$",
            f'{package} = {{ path = "../../{CRATES_DIR}/{cid}" }}',
            text,
        )
    return text.encode("utf-8")


def game_manifest() -> str:
    client = tomllib.loads((HORIZON / "crates/horizon-client/Cargo.toml").read_text(encoding="utf-8"))
    build_deps = client.get("build-dependencies", {})
    lines = [
        "# Generated by Kiloton's tools/hz.py, as Horizon's launcher generates it (native_mods/api.rs).",
        "[package]",
        'name = "horizon-native-game"',
        'version = "0.1.0"',
        'edition = "2024"',
        "publish = false",
        'build = "../horizon-client/build.rs"',
        "",
        "[[bin]]",
        'name = "horizon-client"',
        'path = "src/main.rs"',
        "",
        "[dependencies]",
        'horizon-client = { path = "../horizon-client", default-features = false }',
        'horizon-threads = { path = "../horizon-threads" }',
        'horizon-mods = { path = "../horizon-mods" }',
        'horizon-official = { path = "../horizon-official", optional = true }',
        f'mod_0 = {{ path = "../../{API_DIR}", package = "{api_package()}" }}',
        "",
        "[build-dependencies]",
    ]
    for name, spec in build_deps.items():
        if isinstance(spec, str):
            lines.append(f'{name} = "{spec}"')
        else:
            inner = ", ".join(f'{k} = "{v}"' if isinstance(v, str) else f"{k} = {str(v).lower()}" for k, v in spec.items())
            lines.append(f"{name} = {{ {inner} }}")
    lines += [
        "",
        "[target.'cfg(windows)'.build-dependencies]",
        'winresource = "0.1"',
        "",
        "[features]",
        'default = ["horizon-client/default", "official"]',
        'official = ["dep:horizon-official"]',
        "",
    ]
    return "\n".join(lines)


def api_package() -> str:
    return tomllib.loads((MIXINS / api_folder() / "Cargo.toml").read_text(encoding="utf-8"))["package"]["name"]


GAME_MAIN = f"""//! Generated by Kiloton's tools/hz.py, as Horizon's launcher generates it (native_mods/api.rs).
#![forbid(unsafe_code)]

#[global_allocator]
static ALLOCATOR: horizon_threads::GameAllocator = horizon_threads::GAME_ALLOCATOR;

static MOD_API_LEVEL: [u8; horizon_mods::api_level::MARK.len()] = horizon_mods::api_level::MARK;

fn main() {{
    std::hint::black_box(&MOD_API_LEVEL);
    let mods: horizon_client::mod_api::Mods = vec![
        mod_0::hooks(),
    ].into();
    horizon_client::mod_api::name_mods(&["{MOD_ID}"]);
    #[cfg(feature = "official")]
    horizon_official::register();
    horizon_client::entry::main_with(mods);
}}
"""

MEMBERS = [GAME, API_DIR]

# ---- horizon-mixins ----------------------------------------------------------------------------

NATIVE = json.loads((BUNDLE / "horizon-native.json").read_text(encoding="utf-8"))
MIXINS_REV = NATIVE["mixins"]
MIXINS_GIT = "https://github.com/horizon-rust/horizon-mixins"
EXT = HORIZON.parent
TOOL = Path(os.environ.get("HORIZON_MIXINS", str(EXT / "tools" / f"horizon-mixins-{MIXINS_REV[:12]}" / "bin" / "horizon-mixins.exe")))
WORK = EXT / "hz-kiloton"  # pristine/<commit>/, mods/kiloton/, stage/, stage.key
RECORD = HORIZON / ".kiloton-staged.json"  # what the last stage wrote into the checkout
SKIP = {"target", ".git", ".local", "node_modules", ".horizon-mixins"}
BELOW_NORMAL = 0x00004000 if os.name == "nt" else 0


def git(*args: str, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(["git", "-C", str(HORIZON), *args], check=True, **kw)


def horizon_commit() -> str:
    return git("rev-parse", "HEAD", capture_output=True, text=True).stdout.strip()


def mixins_tool() -> Path:
    """horizon-mixins at the pinned commit, built once as the launcher builds it (`cargo install --locked`)."""
    if not TOOL.exists():
        root = TOOL.parent.parent
        log(f"building horizon-mixins {MIXINS_REV[:12]} into {root} (once)")
        cmd = [CARGO, "install", "--locked", "--git", MIXINS_GIT, "--rev", MIXINS_REV, "--root", str(root),
               "horizon-mixins-cli"]
        if subprocess.run(cmd, creationflags=BELOW_NORMAL).returncode != 0 or not TOOL.exists():
            sys.exit("hz.py: horizon-mixins did not build")
    return TOOL


def pristine(commit: str) -> Path:
    """The checkout's commit as committed (`git archive`), stamped with `.horizon-revision` as the launcher's
    download is: horizon-mixins reads the source from here, never from the checkout it writes into."""
    out = WORK / "pristine" / commit
    stamp = out / ".horizon-revision"
    if stamp.exists() and stamp.read_text(encoding="utf-8").strip() == commit:
        return out
    if out.parent.exists():
        shutil.rmtree(out.parent)
    out.mkdir(parents=True)
    log(f"exporting Horizon {commit[:12]} to {out}")
    tar = git("archive", "--format=tar", commit, capture_output=True).stdout
    import io
    import tarfile
    with tarfile.open(fileobj=io.BytesIO(tar)) as t:
        for m in t.getmembers():
            # Links are left out, as the launcher leaves them out of its download: nothing builds from them.
            parts = Path(m.name).parts
            if not (m.isfile() or m.isdir()) or Path(m.name).is_absolute() or ".." in parts:
                continue
            dest = out.joinpath(*parts)
            if m.isdir():
                dest.mkdir(parents=True, exist_ok=True)
            else:
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(t.extractfile(m).read())
    stamp.write_text(commit + "\n", encoding="utf-8")
    return out


def strip_tables(text: str, names: set[str]) -> str:
    """The manifest without the launcher-only tables ([api], [dependencies]), as native_mods/api.rs strips
    its copy before horizon-mixins reads it."""
    out, skip = [], False
    for line in text.splitlines(keepends=True):
        head = line.strip()
        if head.startswith("["):
            skip = not head.startswith("[[") and head.split("]")[0].strip("[ ") in names
        if not skip:
            out.append(line)
    return "".join(out)


def bundle_copy() -> Path:
    """The bundle as horizon-mixins gets it from the launcher: copied, its manifest stripped."""
    manifest = tomllib.loads((MIXINS / "horizon-mixin.toml").read_text(encoding="utf-8"))
    traits = [i["id"] for i in manifest.get("injection", []) if i.get("trait_method")]
    if traits:
        sys.exit(f"hz.py: trait injections {traits} are turned into helpers by the launcher "
                 "(native_mods/inject.rs), not here: build with horizon-launcher native-build")
    dst = WORK / "mods" / MOD_ID
    mirror(MIXINS, dst)
    path = dst / "horizon-mixin.toml"
    text = path.read_text(encoding="utf-8")
    write_if_changed(path, strip_tables(text, {"api", "dependencies"}).encode("utf-8"))
    return dst.parent


def files_under(root: Path) -> dict[str, Path]:
    found = {}
    for dirpath, dirs, names in os.walk(root):
        dirs[:] = [d for d in dirs if d not in SKIP]
        for n in names:
            f = Path(dirpath) / n
            found[f.relative_to(root).as_posix()] = f
    return found


def apply_mixins() -> dict[str, bytes]:
    """horizon-mixins applied to the pristine source: the files it changed or added, with their bytes."""
    import hashlib
    commit = horizon_commit()
    source = pristine(commit)
    mods = bundle_copy()
    tool = mixins_tool()
    key = hashlib.sha256(f"{commit} {MIXINS_REV}".encode())
    for rel, f in sorted(files_under(mods).items()):
        # The [api] crate is the launcher's, not horizon-mixins': editing it needs no new stage.
        if not rel.startswith(f"{MOD_ID}/{api_folder()}/"):
            key.update(rel.encode() + b"\0" + f.read_bytes() + b"\0")
    stage_dir, key_file = WORK / "stage", WORK / "stage.key"
    if not (stage_dir.exists() and key_file.exists() and key_file.read_text() == key.hexdigest()):
        # horizon-mixins replaces a stage atomically only on Linux: elsewhere it publishes a fresh one.
        if stage_dir.exists():
            shutil.rmtree(stage_dir)
        key_file.unlink(missing_ok=True)
        t0 = time.time()
        cmd = [str(tool), "apply", "--workspace", str(source), "--mods", str(mods), "--out", str(stage_dir)]
        # It runs `cargo update`, `cargo metadata` and `rustc -V`: this toolchain's, on the PATH, as the
        # launcher hands it its own (rust_toolchain::Toolchain::command).
        bin_dir = str(Path(CARGO).parent)
        env = dict(os.environ, CARGO=CARGO, PATH=bin_dir + os.pathsep + os.environ.get("PATH", ""))
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, creationflags=BELOW_NORMAL)
        if r.returncode != 0:
            print(r.stdout + r.stderr, file=sys.stderr)
            sys.exit(f"hz.py: horizon-mixins refused the bundle (exit {r.returncode})")
        key_file.write_text(key.hexdigest())
        log(f"horizon-mixins {MIXINS_REV[:12]} applied the bundle in {time.time() - t0:.1f} s")
    changed = {}
    for rel, f in files_under(stage_dir).items():
        data = f.read_bytes()
        original = source / rel
        if not original.exists() or original.read_bytes() != data:
            changed[rel] = data
    return changed


def with_members(text: str) -> str:
    """The workspace manifest with the packages hz.py adds as the launcher adds them (the [api] crate, the
    generated game, the development crates), after whatever horizon-mixins put there."""
    wanted = MEMBERS + [f"{DEV_DIR}/{d}" for d in DEV_CRATES]
    missing = [m for m in wanted if f'"{m}"' not in text]
    if not missing:
        return text
    block = "\n    # Kiloton (tools/hz.py)\n" + "".join(f'    "{m}",\n' for m in missing)
    return re.sub(r"(?m)^(members\s*=\s*\[)", lambda m: m.group(1) + block, text, count=1)


def restore(keep: set[str]) -> int:
    """Put back what the last stage wrote and this one doesn't: Horizon's own files from git, added ones removed."""
    if not RECORD.exists():
        return 0
    old = json.loads(RECORD.read_text(encoding="utf-8"))
    tracked = [r for r in old.get("tracked", []) if r not in keep]
    added = [r for r in old.get("added", []) if r not in keep]
    if tracked:
        git("checkout", "--", *tracked)
    for rel in added:
        (HORIZON / rel).unlink(missing_ok=True)
    return len(tracked) + len(added)


def sync_mixins() -> int:
    """The files horizon-mixins changed, into the checkout (the workspace manifest with hz.py's members)."""
    changed = apply_mixins()
    if "Cargo.toml" not in changed:
        changed["Cargo.toml"] = (WORK / "stage" / "Cargo.toml").read_bytes()
    changed["Cargo.toml"] = with_members(changed["Cargo.toml"].decode("utf-8")).encode("utf-8")
    source = WORK / "pristine" / horizon_commit()
    n = restore(set(changed))
    for rel, data in changed.items():
        n += write_if_changed(HORIZON / rel, data)
    RECORD.write_text(json.dumps({
        "tracked": sorted(r for r in changed if (source / r).exists()),
        "added": sorted(r for r in changed if not (source / r).exists()),
    }, indent=1), encoding="utf-8")
    return n


def stage() -> None:
    t0 = time.time()
    n = sync_mixins()
    # horizon-mixins staged the [[crate]]s in native-crates/kiloton/<id>; the [api] crate is the launcher's part.
    n += mirror(MIXINS / api_folder(), HORIZON / API_DIR, rewrite_api_manifest)
    for d in DEV_CRATES:
        n += mirror(DEV / d, HORIZON / DEV_DIR / d)
    n += write_if_changed(HORIZON / GAME / "Cargo.toml", game_manifest().encode("utf-8"))
    n += write_if_changed(HORIZON / GAME / "src/main.rs", GAME_MAIN.encode("utf-8"))
    log(f"staged into {HORIZON} ({n} files changed, {time.time() - t0:.1f} s)")


def unstage() -> None:
    restore(set())
    RECORD.unlink(missing_ok=True)
    for d in [HORIZON / GAME, HORIZON / API_DIR, HORIZON / CRATES_DIR, HORIZON / DEV_DIR]:
        if d.exists():
            shutil.rmtree(d)
    for d in [HORIZON / "native-api", HORIZON / "native-crates"]:
        if d.exists() and not any(d.iterdir()):
            d.rmdir()
    log("unstaged: the checkout is back to its commit")


def cargo(*args: str, env: dict | None = None) -> None:
    cmd = [CARGO, *args]
    log(" ".join(cmd))
    t0 = time.time()
    full_env = dict(os.environ, **(env or {}))
    # Below normal priority, as the launcher builds: the desktop (and a game) stay smooth.
    flags = 0x00004000 if os.name == "nt" else 0  # BELOW_NORMAL_PRIORITY_CLASS
    r = subprocess.run(cmd, cwd=HORIZON, env=full_env, creationflags=flags)
    log(f"cargo finished in {time.time() - t0:.1f} s (exit {r.returncode})")
    if r.returncode != 0:
        sys.exit(r.returncode)


def build() -> None:
    stage()
    cargo("build", "--profile", PROFILE, "-p", "horizon-native-game", "-p", "horizon-client-lib", "--bins")
    exe = HORIZON / "target" / PROFILE / "horizon-client.exe"
    log(f"game: {exe} ({exe.stat().st_size / 1e6:.0f} MB)")


def parity_table() -> Path:
    out = HERE / "parity" / "out" / "physics.tsv"
    classes = HERE / "parity" / "build" / "classes"
    src = HERE / "parity" / "src" / "ParityTables.java"
    javac = JAVA_HOME / "bin" / "javac.exe"
    java = JAVA_HOME / "bin" / "java.exe"
    common = KILOTON / "common" / "src" / "main" / "java"
    if classes.exists():
        shutil.rmtree(classes)
    subprocess.run(
        [str(javac), "-encoding", "UTF-8", "-d", str(classes), "-sourcepath", str(common), str(src)], check=True
    )
    subprocess.run([str(java), "-cp", str(classes), "ParityTables", str(out)], check=True)
    return out


def test(extra: list[str]) -> None:
    stage()
    table = parity_table()
    cargo(
        "test", "--profile", PROFILE, "-p", "kiloton-core", "-p", "kiloton-shader-check", *extra, "--", "--nocapture",
        env={"KILOTON_PARITY_TABLE": str(table)},
    )


def game_exe() -> Path:
    return Path(os.environ.get("HZ_GAME", str(HORIZON / "target" / PROFILE / "horizon-client.exe")))


def run(extra: list[str]) -> None:
    exe = game_exe()
    RUN.mkdir(exist_ok=True)
    prune_content_cache()
    # The content half (assets/, data/) goes in with --dev-mod once tools/sync_resources.py wrote it.
    args = [str(exe), *game_args(), *extra]
    log(" ".join(args))
    subprocess.run(args, cwd=RUN)


SOCKET = RUN / "control.json"  # horizon-ctl's endpoint for the development game (not the default one)


def game_args() -> list[str]:
    args = ["--resources", str(HORIZON / ".local" / "horizon-client" / "26.3"),
            "--game-dir", str(RUN), "--home", str(RUN / "home")]
    if (BUNDLE / "assets").is_dir():
        args += ["--dev-mod", str(BUNDLE)]
    return args


def ctl(extra: list[str], check: bool = True) -> int:
    exe = HORIZON / "target" / PROFILE / "horizon-ctl.exe"
    r = subprocess.run([str(exe), "--socket", str(SOCKET), *extra])
    if check and r.returncode != 0:
        sys.exit(r.returncode)
    return r.returncode


def prune_content_cache() -> None:
    """Horizon builds the mods' content overlay once per game executable and content (run/.horizon-cache/mod-content,
    130-660 MB each) and never removes the old ones: keep only the newest."""
    root = RUN / ".horizon-cache" / "mod-content"
    if not root.is_dir():
        return
    dirs = sorted((d for d in root.iterdir() if d.is_dir()), key=lambda d: d.stat().st_mtime)
    for d in dirs[:-1]:
        shutil.rmtree(d, ignore_errors=True)
        for marker in root.glob(d.name + ".*"):
            marker.unlink(missing_ok=True)


def mute() -> None:
    """The test game is silent: master volume 0 in options.txt before it starts (HZ_SOUND=1 keeps the sound).
    Horizon still counts every sound it plays (horizon-ctl status: sounds.heard), so the sound checks still work."""
    if os.environ.get("HZ_SOUND") == "1":
        return
    options = RUN / "options.txt"
    lines = options.read_text(encoding="utf-8").splitlines() if options.exists() else []
    out = [l for l in lines if not l.startswith("soundCategory_master:")]
    out.append("soundCategory_master:0.0")
    if out != lines:
        options.write_text("\n".join(out) + "\n", encoding="utf-8")


def launch(extra: list[str]) -> None:
    """Start the built game under horizon-ctl's control (a world: --world NAME [--seed N] [--creative])."""
    RUN.mkdir(exist_ok=True)
    prune_content_cache()
    mute()
    exe = game_exe()
    script = str(HERE / "tools" / "ingame" / "offscreen.ps1")
    ps = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script]
    # The window that has the focus now gets it back once the game's window is out of the way.
    prev = subprocess.run([*ps, "-Foreground"], capture_output=True, text=True).stdout.strip()
    ctl(["launch", "--game", str(exe), "--wait", *game_args(), *extra])
    if os.environ.get("HZ_ONSCREEN") == "1":
        return
    # Off the desktop and without the focus: tests run while Gabriele uses the PC, and his keys
    # and clicks must not land in the game (HZ_ONSCREEN=1 keeps it where it opens).
    out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq horizon-client.exe", "/FO", "CSV", "/NH"],
                         capture_output=True, text=True).stdout
    for line in out.splitlines():
        parts = [p.strip('"') for p in line.split('","')]
        if len(parts) > 1 and parts[1].isdigit():
            subprocess.run([*ps, "-ProcessId", parts[1], *(["-Restore", prev] if prev.isdigit() else [])])
    ctl(["options-set", "pauseOnLostFocus", "false"], check=False)


def main() -> None:
    cmd = sys.argv[1] if len(sys.argv) > 1 else "build"
    rest = sys.argv[2:]
    if cmd == "launch":
        launch(rest)
        return
    if cmd == "ctl":
        sys.exit(ctl(rest, check=False))
    if cmd == "stage":
        stage()
    elif cmd == "unstage":
        unstage()
    elif cmd == "build":
        build()
    elif cmd == "test":
        test(rest)
    elif cmd == "run":
        run(rest)
    else:
        print(__doc__)
        sys.exit(2)


if __name__ == "__main__":
    main()
