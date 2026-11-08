#!/usr/bin/env python3
"""Run the Ikemen-GO black-box oracle for round-end classification.

Prerequisites
-------------
* ``IKEMEN_DIR`` points at an Ikemen-GO runtime directory containing ``data/``,
  ``font/``, ``external/``, ``chars/``, ``stages/`` and a Linux binary.
* A binary that supports SDL's ``offscreen`` video driver. The official
  release ships a bundled SDL2 without it, so build the checked-out source:

      cd /path/to/ikemen-go
      CGO_ENABLED=1 go build -trimpath -o "$IKEMEN_DIR/ikemen-local" ./src

  ``libxmp`` headers must be available (``brew install libxmp`` on this host).
* Set ``IKEMEN_BIN`` to override the binary path.

The harness never writes into this repository. Raw ``-log`` dumps go to a
temporary directory and only the derived summary is printed, matching the
clean-room rule that raw traces are not committed.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import parse_match_log  # noqa: E402


SCENARIOS = {
    # name: (p1 life, p2 life, round seconds, rounds)
    "ko_full": (1000, 0, 5, 1),
    "ko_clutch_10": (100, 0, 5, 1),
    "ko_clutch_over": (101, 0, 5, 1),
    "time_over_full": (1000, 500, 1, 1),
    "time_over_draw": (500, 500, 1, 1),
}


def find_binary(root: Path) -> Path:
    override = os.environ.get("IKEMEN_BIN")
    if override:
        return Path(override)
    for name in ("ikemen-local", "Ikemen_GO_Linux", "Ikemen_GO"):
        candidate = root / name
        if candidate.is_file():
            return candidate
    raise SystemExit(
        "no Ikemen-GO binary found; set IKEMEN_BIN or build ikemen-local in IKEMEN_DIR"
    )


def run_case(
    root: Path,
    binary: Path,
    p1: str,
    p2: str,
    p1_life: int,
    p2_life: int,
    seconds: int,
    rounds: int,
    workdir: Path,
    keep: bool,
) -> dict:
    log = workdir / f"{p1_life}-{p2_life}-{seconds}.log"
    command = [
        str(binary),
        "-windowed",
        "-nosound",
        "-nomusic",
        "-nojoy",
        "-p1",
        p1,
        "-p2",
        p2,
        "-p1.ai",
        "0",
        "-p2.ai",
        "0",
        "-p1.life",
        str(p1_life),
        "-p2.life",
        str(p2_life),
        "-rounds",
        str(rounds),
        "-time",
        str(seconds),
        "-log",
        str(log),
    ]
    env = os.environ.copy()
    env["SDL_VIDEODRIVER"] = "offscreen"
    lib_paths = [str(root / "lib")]
    if env.get("LD_LIBRARY_PATH"):
        lib_paths.append(env["LD_LIBRARY_PATH"])
    env["LD_LIBRARY_PATH"] = ":".join(lib_paths)
    result = subprocess.run(
        command,
        cwd=root,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=120,
    )
    if not log.is_file():
        raise SystemExit(
            f"oracle produced no log (exit {result.returncode})\n{result.stdout[-2000:]}"
        )
    summary = parse_match_log.summarize(
        parse_match_log.parse(log.read_text(encoding="utf-8", errors="replace"))
    )
    if not keep:
        log.unlink(missing_ok=True)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=os.environ.get("IKEMEN_DIR"))
    parser.add_argument("--p1", default="kfm")
    parser.add_argument("--p2", default="kfm")
    parser.add_argument("--scenario", choices=sorted(SCENARIOS), action="append")
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--json", action="store_true")
    parser.add_argument(
        "--from-logs",
        type=Path,
        help="parse existing <scenario>.log files instead of running the engine",
    )
    args = parser.parse_args()
    if args.from_logs:
        results = {}
        for name in args.scenario or sorted(SCENARIOS):
            log = args.from_logs / f"{name}.log"
            results[name] = parse_match_log.summarize(
                parse_match_log.parse(log.read_text(encoding="utf-8", errors="replace"))
            )
        if args.json:
            print(json.dumps(results, indent=2, sort_keys=True))
        else:
            for name, summary in results.items():
                winner = summary["fighters"].get("1", {})
                print(
                    f"{name}: win_side={summary['win_side']} timer={summary['round_time']} "
                    f"ko={winner.get('win_ko')} time={winner.get('win_time')} "
                    f"perfect={winner.get('win_perfect')} clutch={winner.get('win_clutch')} "
                    f"life={winner.get('life')}/{winner.get('life_max')}"
                )
        return 0
    if not args.root:
        parser.error("set IKEMEN_DIR or pass --root")
    root = args.root.resolve()
    binary = find_binary(root)
    names = args.scenario or sorted(SCENARIOS)
    workdir = Path(tempfile.mkdtemp(prefix="manato-oracle-"))
    results = {}
    for name in names:
        p1_life, p2_life, seconds, rounds = SCENARIOS[name]
        results[name] = run_case(
            root,
            binary,
            args.p1,
            args.p2,
            p1_life,
            p2_life,
            seconds,
            rounds,
            workdir,
            args.keep,
        )
    if args.json:
        print(json.dumps(results, indent=2, sort_keys=True))
    else:
        for name, summary in results.items():
            winner = summary["fighters"].get("1", {})
            print(
                f"{name}: win_side={summary['win_side']} timer={summary['round_time']} "
                f"ko={winner.get('win_ko')} time={winner.get('win_time')} "
                f"perfect={winner.get('win_perfect')} clutch={winner.get('win_clutch')} "
                f"life={winner.get('life')}/{winner.get('life_max')}"
            )
    if args.keep:
        print(f"raw logs kept in {workdir}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
