#!/usr/bin/env python3
"""Per-frame Ikemen-GO state capture through its public Lua debug API.

The harness installs ``capture.lua`` as a debug-info provider, patches the
throwaway runtime's ``main.lua`` to load it before the quick-match path, runs a
short match, and reduces the CSV to a small derived summary. Raw traces stay in
a temporary directory and are never committed.

Example:
    IKEMEN_DIR=/path/to/ikemen-runtime \
      python3 tools/oracle/run_frame_capture.py --scenario kfm_attack

The runtime needs a binary that supports SDL's ``offscreen`` driver; see
``docs/oracle/README.md``.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
LOAD_SNIPPET = "pcall(function() dofile('external/mods/zz-oracle.lua') end)\n\n"
QUICK_MATCH = "--initiate quick match only if -loadmotif flag is missing"


def find_binary(root: Path) -> Path:
    override = os.environ.get("IKEMEN_BIN")
    if override:
        return Path(override)
    for name in ("ikemen-local", "Ikemen_GO_Linux", "Ikemen_GO"):
        candidate = root / name
        if candidate.is_file():
            return candidate
    raise SystemExit("no Ikemen-GO binary found; set IKEMEN_BIN")


def patch_main(root: Path) -> bool:
    script = root / "external/script/main.lua"
    text = script.read_text(encoding="utf-8", errors="replace")
    if "external/mods/zz-oracle.lua" in text:
        return False
    if QUICK_MATCH not in text:
        raise SystemExit(f"{script} has no quick-match hook to patch")
    script.write_text(text.replace(QUICK_MATCH, LOAD_SNIPPET + QUICK_MATCH, 1))
    return True


def unpatch_main(root: Path, added: bool) -> None:
    if not added:
        return
    script = root / "external/script/main.lua"
    text = script.read_text(encoding="utf-8", errors="replace")
    script.write_text(text.replace(LOAD_SNIPPET, "", 1))


def prepare_stage(root: Path) -> None:
    stages = root / "stages"
    stages.mkdir(exist_ok=True)
    shutil.copy(ROOT / "tools/oracle/oracle-stage.def", stages / "oracle.def")
    source = ROOT / "content/stages/training/stage1.sff"
    shutil.copy(source, stages / "oracle.sff")



def prepare_oracle_char(root: Path) -> None:
    """Installs the purpose-built oracle character with guard/dizzy data."""
    source = root / "chars/kfm"
    target = root / "chars/oracle"
    if not source.is_dir():
        return
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target)
    for candidate in target.glob("*.def"):
        if candidate.name.lower() == "kfm.def":
            candidate.rename(target / "oracle.def")
    script = target / "oracle.def"
    text = script.read_text(encoding="utf-8", errors="replace")
    if "oracle.cns" not in text:
        text = text.replace("stcommon = common1.cns", "stcommon = common1.cns\nst2     = oracle.cns", 1)
        script.write_text(text, encoding="utf-8")
    shutil.copy(ROOT / "tools/oracle/oracle-char/oracle.cns", target / "oracle.cns")
    cns = target / "kfm.cns"
    if cns.is_file():
        data = cns.read_text(encoding="utf-8", errors="replace")
        if "guardpoints" not in data:
            data = data.replace("volume = 0", "volume = 0\nguardpoints = 20\ndizzypoints = 100", 1)
            cns.write_text(data, encoding="utf-8")

def summarize(path: Path) -> dict:
    with path.open(newline="", encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle))
    if not rows:
        raise SystemExit("capture is empty")
    injection = next(
        (row for row in rows if int(row["p1_state"]) in (200, 400, 440, 7000, 1000, 3000, 800, 810, 2100, 7002, 7003, 7004, 7005, 7007)), None
    )
    if injection is None:
        raise SystemExit("scenario never entered a scripted KFM attack state")
    contact = None
    previous_life = int(injection["p2_life"])
    hits = 0
    guarded = False
    for row in rows[rows.index(injection) :]:
        life = int(row["p2_life"])
        if life < previous_life:
            hits += 1
        if contact is None and (
            life < previous_life
            or int(row["p2_state"]) >= 5000
            or int(row.get("p1_projContact", "-1")) >= 0
        ):
            contact = row
        if int(row.get("p1_projGuarded", "-1")) >= 0:
            guarded = True
        previous_life = life
    if contact is None:
        raise SystemExit("scenario never hit the defender")
    contact_index = rows.index(contact)
    injection_index = rows.index(injection)
    contact_x = float(contact["p2_x"])
    max_x = max(float(row["p2_x"]) for row in rows[contact_index:])
    contact_state = int(contact["p2_state"])
    transition = next(
        (
            row
            for row in rows[contact_index + 1 :]
            if int(row["p2_state"]) >= 5000
            and int(row["p2_state"]) != contact_state
        ),
        None,
    )
    drops = []
    previous = int(injection["p2_life"])
    for row in rows[rows.index(injection) :]:
        life = int(row["p2_life"])
        if life < previous:
            drops.append((row, previous - life))
        previous = life
    guard = next(
        (
            row
            for row in rows
            if int(row["p1_moveGuarded"]) or int(row["p2_moveGuarded"])
        ),
        None,
    )
    guard_info = {}
    if guard:
        guard_index = rows.index(guard)
        before_life = (
            int(rows[guard_index - 1]["p2_life"])
            if guard_index > 0
            else int(guard["p2_life"])
        )
        guard_x = float(guard["p2_x"])
        guard_states = (150, 151, 152, 153, 154, 155)
        guard_end = next(
            (
                index
                for index, row in enumerate(rows[guard_index:])
                if int(row["p2_state"]) not in guard_states
            ),
            len(rows) - guard_index,
        )
        guard_info = {
            "guard_contact_frame": int(guard["frame"]),
            "guard_state": int(guard["p2_state"]),
            "guard_anim": int(guard["p2_anim"]),
            "guard_damage": before_life - int(guard["p2_life"]),
            "guard_push": round(
                max(float(row["p2_x"]) for row in rows[guard_index : guard_index + guard_end])
                - guard_x,
                3,
            ),
            "guard_ticks": guard_end,
        }
    return {
        "injection_frame": int(injection["frame"]),
        "contact_frame": int(contact["frame"]),
        "contact_ticks": int(contact["frame"]) - int(injection["frame"]),
        "damage": int(injection["p2_life"]) - min(int(row["p2_life"]) for row in rows),
        "hits": hits,
        "defender_state": int(contact["p2_state"]),
        "defender_anim": int(contact["p2_anim"]),
        "guarded": guarded,
        "projectile_contact": int(contact.get("p1_projContact", "-1")),
        "projectile_hit": int(contact.get("p1_projHit", "-1")),
        "projectile_guarded": int(contact.get("p1_projGuarded", "-1")),
        "transition_ticks": (
            int(transition["frame"]) - int(contact["frame"]) if transition else None
        ),
        "state_after_contact": int(transition["p2_state"]) if transition else None,
        "push_distance": round(max_x - contact_x, 3),
        **guard_info,
        "juggle_after_first_hit": (
            int(drops[0][0]["p2_juggle"]) if drops and "p2_juggle" in drops[0][0] else None
        ),
        "second_hit_frame": int(drops[1][0]["frame"]) if len(drops) > 1 else None,
        "second_damage": drops[1][1] if len(drops) > 1 else None,
        "second_state": int(drops[1][0]["p2_state"]) if len(drops) > 1 else None,
        "juggle_after_second_hit": (
            int(drops[1][0]["p2_juggle"]) if len(drops) > 1 else None
        ),
        "third_hit_frame": int(drops[2][0]["frame"]) if len(drops) > 2 else None,
        "third_damage": drops[2][1] if len(drops) > 2 else None,
        "third_state": int(drops[2][0]["p2_state"]) if len(drops) > 2 else None,
        "juggle_after_third_hit": (
            int(drops[2][0]["p2_juggle"]) if len(drops) > 2 else None
        ),
        "rows": len(rows),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=os.environ.get("IKEMEN_DIR"))
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--scenario", default="kfm_attack")
    parser.add_argument("--p1", default="kfm")
    parser.add_argument("--p2", default="kfm")
    parser.add_argument("--seconds", type=int, default=10)
    parser.add_argument("--p1-ai", type=int, default=0)
    parser.add_argument("--p2-ai", type=int, default=None)
    parser.add_argument("--timeout", type=int, default=14)
    parser.add_argument("--from-csv", type=Path, help="reduce an existing capture")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    if args.from_csv:
        summary = summarize(args.from_csv)
    else:
        if not args.root:
            parser.error("set IKEMEN_DIR or pass --root")
        root = args.root.resolve()
        binary = args.binary or find_binary(root)
        p2_ai = args.p2_ai if args.p2_ai is not None else 0
        workdir = Path(tempfile.mkdtemp(prefix="manato-frame-"))
        capture = workdir / "frames.csv"
        added = False
        try:
            shutil.copy(ROOT / "tools/oracle/capture.lua", root / "external/mods/zz-oracle.lua")
            prepare_stage(root)
            prepare_oracle_char(root)
            added = patch_main(root)
            env = os.environ.copy()
            env.setdefault("SDL_VIDEODRIVER", "offscreen")
            env["IKEMEN_ORACLE_OUT"] = str(capture)
            env["IKEMEN_ORACLE_SCENARIO"] = args.scenario
            lib_paths = [str(root / "lib")]
            if env.get("LD_LIBRARY_PATH"):
                lib_paths.append(env["LD_LIBRARY_PATH"])
            env["LD_LIBRARY_PATH"] = ":".join(lib_paths)
            command = [
                str(binary),
                "-windowed",
                "-nosound",
                "-nomusic",
                "-nojoy",
                "-debug",
                "-p1",
                args.p1,
                "-p2",
                args.p2,
                "-p1.ai",
                str(args.p1_ai),
                "-p2.ai",
                str(p2_ai),
                "-s",
                "oracle",
                "-rounds",
                "1",
                "-time",
                str(args.seconds),
            ]
            try:
                subprocess.run(
                    command,
                    cwd=root,
                    env=env,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=args.timeout,
                )
            except subprocess.TimeoutExpired:
                # The match loop keeps running in attract mode; the capture
                # already contains everything we need.
                pass
            if not capture.is_file():
                raise SystemExit("oracle produced no per-frame capture")
            summary = summarize(capture)
            if args.keep:
                print(f"raw capture kept at {capture}", file=sys.stderr)
        finally:
            unpatch_main(root, added)
            (root / "external/mods/zz-oracle.lua").unlink(missing_ok=True)

    if args.json:
        print(json.dumps(summary, indent=2, sort_keys=True))
    else:
        print(
            f"contact_ticks={summary['contact_ticks']} damage={summary['damage']} "
            f"hits={summary['hits']} state={summary['defender_state']} "
            f"next_state={summary['state_after_contact']} "
            f"transition={summary['transition_ticks']} "
            f"anim={summary['defender_anim']} push={summary['push_distance']} "
            f"guard_frame={summary.get('guard_contact_frame')} "
            f"guard_state={summary.get('guard_state')} "
            f"guard_damage={summary.get('guard_damage')}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
