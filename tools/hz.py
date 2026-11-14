"""Kiloton for Horizon: the development loop, from a Horizon source checkout.

    python tools/hz.py stage            mirror the mod into the Horizon workspace
    python tools/hz.py build            stage, then compile the game with the mod (and horizon-ctl)
    python tools/hz.py test             stage, write the Java parity table, run kiloton-core's tests
    python tools/hz.py run [game args]  start the built game with Kiloton's content (--dev-mod)
    python tools/hz.py launch [args]    the same under horizon-ctl's control (--world NAME --creative ...)
    python tools/hz.py ctl <command>    a horizon-ctl command to that game (command "nuke info 15", screenshot F, quit)
    python tools/hz.py unstage          take the mod back out of the Horizon workspace

The game is built the way Horizon's launcher builds a native trait mod (launcher/src/native_mods/api.rs:
a generated `horizon-native-game` package that links the mod's [api] crate and calls its hooks()),
in the checkout's own `target/`, so the engine is compiled once and only the mod's crates and the
final link are redone after an edit. Unlike the launcher it does not run horizon-mixins: its
[[crate]] is staged directly and only the [api] crate depends on it (the launcher also makes
horizon-client depend on it, which recompiles the engine when the crate changes), and the bundle's
trait injections (HEAD only) are written into their target functions by hz.py itself, marked, as
horizon-mixins would put them there; `unstage` takes them out. Files are copied only when their
bytes change, so Cargo's timestamps stay meaningful.

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
static ALLOCATOR: horizon_threads::Allocator = horizon_threads::ALLOCATOR;

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

# ---- trait injections ---------------------------------------------------------------------------
# The launcher turns each `[[injection]]` with a `trait_method` into a mixin helper and an extra
# `Hooks` method (launcher/src/native_mods/inject.rs, horizon-mod-api-gen's injection.rs), then
# horizon-mixins puts the helper at the head of the target. Without horizon-mixins here, hz.py
# writes the same helper into the target function itself (marked, and taken out by `unstage`) and
# hands the declarations to the game's build script through HORIZON_MOD_API_INJECTIONS, as the
# launcher does. Only what Kiloton uses is covered: HEAD injections without `cancellable`.

INJECT_MARK = "// Kiloton (tools/hz.py): trait injection"
INJECTIONS_JSON = HORIZON / ".horizon-mod-api-injections.json"


def camel(name: str) -> str:
    return "".join(w[:1].upper() + w[1:] for w in name.split("_"))


def event_type(ty: str) -> str:
    """A binding's type as an event field: its references take the event's lifetime."""
    return re.sub(r"&(?!\s*')", "&'a ", ty, count=1)


def trait_injections() -> list[dict]:
    manifest = tomllib.loads((MIXINS / "horizon-mixin.toml").read_text(encoding="utf-8"))
    catalog = json.loads((HORIZON / "native-hooks.json").read_text(encoding="utf-8"))
    targets = {h["name"]: h["target"] for h in catalog["hooks"]}
    out = []
    for inj in manifest.get("injection", []):
        method = inj.get("trait_method")
        if not method:
            continue
        if inj.get("kind") != "HEAD" or inj.get("cancellable"):
            raise SystemExit(f"hz.py handles HEAD trait injections only: {inj.get('id')}")
        target = targets[inj["hook"]] if "hook" in inj else inj["target"]
        name = f"inject_{MOD_ID.replace('-', '_')}_{method}"
        event = camel(name) + "Event"
        binds = inj.get("bindings", [])
        params = ", ".join(f"{b['name']}: {b['ty']}" for b in binds)
        names = ", ".join(b["name"] for b in binds)
        values = ", ".join(b["value"] for b in binds)
        fields = ", ".join(f"pub {b['name']}: {event_type(b['ty'])}" for b in binds)
        has_refs = any("&" in b["ty"] for b in binds)
        out.append({
            "id": inj["id"],
            "target": target,
            "decl": {
                "name": name,
                "event": event,
                "declaration": f"fn {name}(&self, _: &mut {event}{'<' + chr(39) + '_>' if has_refs else ''}) {{}}",
                "event_declaration": (
                    f"#[doc = \" What [`Hooks::{name}`] is told. Fields may be added: the struct is `#[non_exhaustive]`.\"]"
                    f" #[non_exhaustive] pub struct {event}{'<' + chr(39) + 'a>' if has_refs else ''} {{ {fields} }}"
                ),
            },
            "helper": (
                f"fn horizon_trait_dispatch({params}) {{ "
                f"let Some(owner) = crate::mod_api::mods().get(0) else {{ return; }}; "
                f"let mut event = crate::mod_api::{event} {{ {names} }}; "
                f"let _flow = owner.{name}(&mut event); }}"
            ),
            "call": f"horizon_trait_dispatch({values});",
        })
    return out


def target_file(target: str) -> tuple[Path, str]:
    """`horizon_client::game::native_hooks::Game<'_>::native_x` -> its source file and function name."""
    parts = target.split("::")
    func = parts[-1]
    mods = [p for p in parts[1:-1] if not p.startswith("Game")]
    base = HORIZON / "crates" / "horizon-client" / "src"
    path = base.joinpath(*mods).with_suffix(".rs")
    if not path.exists():
        path = base.joinpath(*mods, "mod.rs")
    return path, func


def patch_targets(injections: list[dict], restore_only: bool = False) -> int:
    """Put each injection's helper at the head of its target (taking out any earlier one first)."""
    record = HORIZON / ".kiloton-injected"
    files = {target_file(i["target"])[0] for i in injections}
    if record.exists():
        files |= {Path(line) for line in record.read_text(encoding="utf-8").splitlines() if line}
    block_re = re.compile(r"[ \t]*\{ " + re.escape(INJECT_MARK) + r"[^\n]*\n.*?\n[ \t]*\}\n", re.S)
    changed = 0
    for path in files:
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        new = block_re.sub("", text)
        if not restore_only:
            # Each block goes in right after the brace, so the last one put in runs first: in
            # reverse, so that they run in the order of their ids, as horizon-mixins orders them.
            for inj in sorted(injections, key=lambda i: i["id"], reverse=True):
                target, func = target_file(inj["target"])
                if target != path:
                    continue
                m = re.search(r"fn " + re.escape(func) + r"\b[^{]*\{\n", new)
                if not m:
                    raise SystemExit(f"hz.py: {func} not found in {path}")
                block = (
                    f"        {{ {INJECT_MARK} {inj['id']} (as horizon-mixins applies it)\n"
                    f"            {inj['helper']}\n"
                    f"            {inj['call']}\n"
                    f"        }}\n"
                )
                new = new[: m.end()] + block + new[m.end():]
        if new != text:
            # Python writes the platform's line ends, as the checkout has them.
            path.write_text(new, encoding="utf-8")
            changed += 1
    if restore_only:
        record.unlink(missing_ok=True)
    else:
        record.write_text("\n".join(str(f) for f in sorted({target_file(i["target"])[0] for i in injections})), encoding="utf-8")
    return changed


def stage_injections() -> int:
    injections = trait_injections()
    n = patch_targets(injections)
    data = json.dumps([i["decl"] for i in injections]).encode("utf-8")
    n += write_if_changed(INJECTIONS_JSON, data)
    return n


def workspace_members(add: bool) -> bool:
    """Add (or remove) the mod's packages in the workspace's members list, keeping the file's text."""
    root = HORIZON / "Cargo.toml"
    text = root.read_text(encoding="utf-8")
    wanted = MEMBERS + [f"{CRATES_DIR}/{cid}" for cid, _ in crates()]
    marker = "    # Kiloton (tools/hz.py)\n"
    end = "    # end of Kiloton (tools/hz.py)\n"
    block = marker + "".join(f'    "{m}",\n' for m in wanted) + end
    # Only between the two markers: Horizon's own members right after the block are its own.
    new = re.sub(re.escape(marker) + r".*?" + re.escape(end), "", text, flags=re.S)
    if add:
        new = new.replace("members = [\n", "members = [\n" + block, 1)
    if new != text:
        root.write_text(new, encoding="utf-8")
        return True
    return False


def stage() -> None:
    t0 = time.time()
    n = 0
    for cid, cpath in crates():
        n += mirror(MIXINS / cpath, HORIZON / CRATES_DIR / cid)
    n += mirror(MIXINS / api_folder(), HORIZON / API_DIR, rewrite_api_manifest)
    n += write_if_changed(HORIZON / GAME / "Cargo.toml", game_manifest().encode("utf-8"))
    n += write_if_changed(HORIZON / GAME / "src/main.rs", GAME_MAIN.encode("utf-8"))
    n += workspace_members(True)
    n += stage_injections()
    log(f"staged into {HORIZON} ({n} files changed, {time.time() - t0:.1f} s)")


def unstage() -> None:
    workspace_members(False)
    patch_targets([], restore_only=True)
    INJECTIONS_JSON.unlink(missing_ok=True)
    for d in [HORIZON / GAME, HORIZON / API_DIR, HORIZON / CRATES_DIR]:
        if d.exists():
            shutil.rmtree(d)
    for d in [HORIZON / "native-api", HORIZON / "native-crates"]:
        if d.exists() and not any(d.iterdir()):
            d.rmdir()
    log("unstaged; Cargo.lock keeps the mod's entries until the next build without them")


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
    cargo("build", "--profile", PROFILE, "-p", "horizon-native-game", "-p", "horizon-client-lib", "--bins",
          env={"HORIZON_MOD_API_INJECTIONS": str(INJECTIONS_JSON)})
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
        "test", "--profile", PROFILE, "-p", "kiloton-core", *extra, "--", "--nocapture",
        env={"KILOTON_PARITY_TABLE": str(table)},
    )


def run(extra: list[str]) -> None:
    exe = HORIZON / "target" / PROFILE / "horizon-client.exe"
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
    exe = HORIZON / "target" / PROFILE / "horizon-client.exe"
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
