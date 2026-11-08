#!/usr/bin/env python3
"""Parse an Ikemen-GO ``-log`` match summary.

Ikemen-GO writes the result of ``getGameStats().Matches[matchNo()]`` through
``main.f_printTable`` when launched with ``-log <file>``. The file is a Lua
table dump, not JSON. This module turns it into plain Python data so the
oracle harness can derive assertions without committing the raw trace.

Usage:
    python3 tools/oracle/parse_match_log.py MATCH.log [--json]
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

TOKEN = re.compile(
    r'\s*(?:'
    r'(table:\s*0x[0-9a-fA-F]+\s*\{)'
    r'|(\})'
    r'|(\["(?:[^"\\]|\\.)*"\]\s*=>)'
    r'|(\[[0-9]+\]\s*=>)'
    r'|("(?:[^"\\]|\\.)*")'
    r'|(-?[0-9]+(?:\.[0-9]+)?)'
    r'|(true|false|nil)'
    r')'
)


def tokenize(text: str):
    pos = 0
    while pos < len(text):
        match = TOKEN.match(text, pos)
        if not match:
            if text[pos:].strip() == "":
                break
            raise ValueError(f"unexpected input at offset {pos}: {text[pos:pos+40]!r}")
        table, close, key, index, string, number, literal = match.groups()
        if table:
            yield ("table", None)
        elif close:
            yield ("close", None)
        elif key:
            yield ("key", re.search(r'"(.*)"', key).group(1))
        elif index:
            yield ("index", int(re.search(r"[0-9]+", index).group(0)))
        elif string:
            yield ("value", json.loads(string))
        elif number:
            yield ("value", float(number) if "." in number else int(number))
        elif literal:
            yield ("value", {"true": True, "false": False, "nil": None}[literal])
        pos = match.end()


def parse(text: str):
    tokens = tokenize(text)
    kind, _ = next(tokens)
    if kind != "table":
        raise ValueError("log does not start with a table")

    def parse_table():
        result = {}
        for kind, value in tokens:
            if kind == "close":
                return result
            if kind == "key":
                key = value
            elif kind == "index":
                key = value
            else:
                raise ValueError(f"expected key/index, got {kind}")
            kind, value = next(tokens)
            if kind == "table":
                result[key] = parse_table()
            else:
                result[key] = value
        return result

    return parse_table()


def summarize_round(round_data: dict) -> dict:
    fighters = round_data.get("Fighters", {})
    out = {}
    for side, entries in fighters.items():
        if not isinstance(entries, dict):
            continue
        first_entry = entries.get(1)
        if not isinstance(first_entry, dict):
            continue
        out[str(side)] = {
            "name": first_entry.get("Name"),
            "life": first_entry.get("Life"),
            "life_max": first_entry.get("LifeMax"),
            "win": first_entry.get("Win"),
            "win_ko": first_entry.get("WinKO"),
            "win_time": first_entry.get("WinTime"),
            "win_perfect": first_entry.get("WinPerfect"),
            "win_clutch": first_entry.get("WinClutch"),
        }
    return {
        "index": round_data.get("Index"),
        "timer": round_data.get("Timer"),
        "draw": bool(out) and not any(fighter.get("win") for fighter in out.values()),
        "fighters": out,
    }


def summarize(stats: dict) -> dict:
    """Reduces the verbose dump to the fields the assertions rely on."""
    rounds = stats.get("Rounds", {})
    parsed = []
    if isinstance(rounds, dict):
        for index in sorted(rounds, key=lambda value: int(value)):
            parsed.append(summarize_round(rounds[index]))
    first = parsed[0] if parsed else {"fighters": {}, "timer": 0}
    return {
        "win_side": stats.get("WinSide"),
        "wins": stats.get("Wins"),
        "draws": stats.get("Draws"),
        "match_time": first["timer"] or stats.get("MatchTime"),
        "round_time": first["timer"] or stats.get("RoundTime"),
        "fighters": first["fighters"],
        "rounds": parsed,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("log", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    try:
        summary = summarize(parse(args.log.read_text(encoding="utf-8", errors="replace")))
    except (OSError, ValueError) as error:
        print(f"failed to parse {args.log}: {error}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(summary, indent=2, sort_keys=True))
    else:
        print(f"win_side={summary['win_side']} match_time={summary['match_time']}")
        for side, fighter in summary["fighters"].items():
            print(
                f"p{side}: {fighter['name']} life={fighter['life']}/{fighter['life_max']} "
                f"win={fighter['win']} ko={fighter['win_ko']} time={fighter['win_time']} "
                f"perfect={fighter['win_perfect']} clutch={fighter['win_clutch']}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
