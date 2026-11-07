#!/usr/bin/env python3
"""Generate original sound banks for the three custom characters.

The sounds are synthesized from noise and sine waves, so they are original
content and can be regenerated at any time. The output is a standard
M.U.G.E.N `ElecbyteSnd` bank containing 16-bit mono WAV subfiles.

Usage:
    python3 tools/gen_sfx.py --content /path/to/mugen/content/chars
"""

from __future__ import annotations

import argparse
import io
import math
import random
import struct
import wave
from array import array
from pathlib import Path

SR = 22_050


def lowpass(alpha: float, previous: float, sample: float) -> float:
    return previous + alpha * (sample - previous)


def whoosh(duration: float, start_hz: float, end_hz: float, bright: float, seed: int) -> list[float]:
    rng = random.Random(seed)
    n = int(SR * duration)
    out = [0.0] * n
    lp = 0.0
    for i in range(n):
        t = i / SR
        p = i / max(1, n - 1)
        attack = min(1.0, t / 0.006)
        envelope = attack * (1.0 - p) ** 2
        hz = start_hz + (end_hz - start_hz) * p
        alpha = 1.0 - math.exp(-2.0 * math.pi * hz / SR)
        lp = lowpass(alpha, lp, rng.uniform(-1.0, 1.0))
        tone = math.sin(2.0 * math.pi * hz * t)
        out[i] = envelope * (lp * 1.2 + tone * bright)
    return out


def impact(duration: float, thump_hz: float, noise_hz: float, click: float, seed: int) -> list[float]:
    rng = random.Random(seed)
    n = int(SR * duration)
    out = [0.0] * n
    lp = 0.0
    for i in range(n):
        t = i / SR
        attack = min(1.0, t / 0.002)
        body = math.sin(2.0 * math.pi * thump_hz * t) * math.exp(-t * 9.0)
        alpha = 1.0 - math.exp(-2.0 * math.pi * noise_hz / SR)
        lp = lowpass(alpha, lp, rng.uniform(-1.0, 1.0))
        grit = lp * math.exp(-t * 28.0)
        tick = math.sin(2.0 * math.pi * 2_600.0 * t) * math.exp(-t * 90.0)
        out[i] = attack * (body * 1.1 + grit * 1.4 + tick * click)
    return out


def guard(duration: float, seed: int) -> list[float]:
    rng = random.Random(seed)
    n = int(SR * duration)
    out = [0.0] * n
    for i in range(n):
        t = i / SR
        envelope = math.exp(-t * 55.0)
        ring = (
            math.sin(2.0 * math.pi * 1_850.0 * t)
            + 0.6 * math.sin(2.0 * math.pi * 2_720.0 * t)
            + 0.35 * math.sin(2.0 * math.pi * 3_410.0 * t)
        )
        grit = rng.uniform(-1.0, 1.0) * math.exp(-t * 120.0)
        out[i] = envelope * (ring * 0.45 + grit * 0.8)
    return out


def fire_special(duration: float, seed: int) -> list[float]:
    rng = random.Random(seed)
    n = int(SR * duration)
    out = [0.0] * n
    lp = 0.0
    for i in range(n):
        t = i / SR
        p = i / max(1, n - 1)
        envelope = min(1.0, t / 0.02) * (1.0 - p) ** 1.4
        cutoff = 250.0 + 2_200.0 * (p ** 0.6)
        alpha = 1.0 - math.exp(-2.0 * math.pi * cutoff / SR)
        lp = lowpass(alpha, lp, rng.uniform(-1.0, 1.0))
        rumble = math.sin(2.0 * math.pi * (70.0 + 90.0 * p) * t)
        crackle = 0.0
        if rng.random() < 0.012:
            crackle = rng.uniform(-1.0, 1.0)
        out[i] = envelope * (lp * 1.5 + rumble * 0.5 + crackle * 0.9)
    return out


def frost_special(duration: float, seed: int) -> list[float]:
    rng = random.Random(seed)
    n = int(SR * duration)
    out = [0.0] * n
    for i in range(n):
        t = i / SR
        p = i / max(1, n - 1)
        envelope = min(1.0, t / 0.03) * (1.0 - p) ** 1.2
        shimmer = (
            math.sin(2.0 * math.pi * 1_150.0 * t)
            + 0.7 * math.sin(2.0 * math.pi * 1_720.0 * t)
            + 0.45 * math.sin(2.0 * math.pi * 2_430.0 * t)
            + 0.3 * math.sin(2.0 * math.pi * 3_210.0 * t)
        )
        air = rng.uniform(-1.0, 1.0) * 0.35
        out[i] = envelope * (shimmer * 0.32 + air)
    return out


def earth_special(duration: float, seed: int) -> list[float]:
    rng = random.Random(seed)
    n = int(SR * duration)
    out = [0.0] * n
    lp = 0.0
    for i in range(n):
        t = i / SR
        p = i / max(1, n - 1)
        envelope = min(1.0, t / 0.01) * (1.0 - p) ** 1.6
        alpha = 1.0 - math.exp(-2.0 * math.pi * 420.0 / SR)
        lp = lowpass(alpha, lp, rng.uniform(-1.0, 1.0))
        rumble = math.sin(2.0 * math.pi * (48.0 + 26.0 * p) * t)
        crunch = 0.0
        if rng.random() < 0.02:
            crunch = rng.uniform(-1.0, 1.0)
        out[i] = envelope * (rumble * 0.9 + lp * 0.9 + crunch * 1.1)
    return out


def super_move(duration: float, seed: int, flavor: str) -> list[float]:
    if flavor == "fire":
        base = fire_special(duration, seed)
    elif flavor == "frost":
        base = frost_special(duration, seed)
    else:
        base = earth_special(duration, seed)
    n = len(base)
    out = [0.0] * n
    for i, sample in enumerate(base):
        p = i / max(1, n - 1)
        sweep = math.sin(2.0 * math.pi * (90.0 + 180.0 * p) * (i / SR))
        out[i] = sample * 1.1 + sweep * 0.35 * (1.0 - p)
    return out


def normalize(samples: list[float], peak: float = 0.8) -> list[float]:
    high = max((abs(s) for s in samples), default=0.0)
    if high <= 0.0:
        return samples
    scale = peak / high
    return [s * scale for s in samples]


def wav_bytes(samples: list[float]) -> bytes:
    pcm = array("h", (int(max(-1.0, min(1.0, s)) * 32_767) for s in samples))
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as writer:
        writer.setnchannels(1)
        writer.setsampwidth(2)
        writer.setframerate(SR)
        writer.writeframes(pcm.tobytes())
    return buffer.getvalue()


def build_bank(entries: list[tuple[int, int, list[float]]]) -> bytes:
    subfiles = []
    for group, index, samples in entries:
        payload = wav_bytes(normalize(samples))
        # 16-byte subfile header: next offset, byte length, group, index,
        # and four reserved bytes.
        subfiles.append(
            bytearray(struct.pack("<IIHHI", 0, len(payload), group, index, 0) + payload)
        )

    offsets = []
    cursor = 512
    for subfile in subfiles:
        offsets.append(cursor)
        cursor += len(subfile)
    for i, subfile in enumerate(subfiles):
        next_offset = offsets[i + 1] if i + 1 < len(subfiles) else 0
        struct.pack_into("<I", subfile, 0, next_offset)

    header = bytearray(512)
    header[:12] = b"ElecbyteSnd\0"
    struct.pack_into("<I", header, 12, 4)
    struct.pack_into("<I", header, 16, len(subfiles))
    struct.pack_into("<I", header, 20, offsets[0])
    return bytes(header) + b"".join(subfiles)


def character_bank(flavor: str, seed: int) -> bytes:
    if flavor == "fire":
        light = whoosh(0.14, 900.0, 300.0, 0.25, seed + 1)
        medium = whoosh(0.20, 700.0, 220.0, 0.35, seed + 2)
        heavy = whoosh(0.28, 520.0, 150.0, 0.45, seed + 3)
        special = fire_special(0.55, seed + 4)
        super_move_sound = super_move(0.95, seed + 5, "fire")
        throw = impact(0.30, 95.0, 1_200.0, 0.25, seed + 6)
    elif flavor == "frost":
        light = whoosh(0.13, 1_500.0, 700.0, 0.35, seed + 1)
        medium = whoosh(0.19, 1_250.0, 520.0, 0.45, seed + 2)
        heavy = whoosh(0.26, 1_000.0, 380.0, 0.55, seed + 3)
        special = frost_special(0.55, seed + 4)
        super_move_sound = super_move(0.95, seed + 5, "frost")
        throw = impact(0.28, 130.0, 2_000.0, 0.35, seed + 6)
    else:
        light = whoosh(0.16, 500.0, 180.0, 0.2, seed + 1)
        medium = whoosh(0.23, 380.0, 120.0, 0.3, seed + 2)
        heavy = whoosh(0.32, 280.0, 80.0, 0.4, seed + 3)
        special = earth_special(0.60, seed + 4)
        super_move_sound = super_move(1.05, seed + 5, "earth")
        throw = impact(0.34, 70.0, 900.0, 0.2, seed + 6)

    return build_bank(
        [
            (0, 0, light),
            (0, 1, medium),
            (0, 2, heavy),
            (3, 0, special),
            (3, 1, super_move_sound),
            (3, 2, throw),
            (5, 0, impact(0.16, 150.0, 2_600.0, 0.5, seed + 10)),
            (5, 1, impact(0.22, 105.0, 1_900.0, 0.4, seed + 11)),
            (5, 2, impact(0.32, 68.0, 1_300.0, 0.3, seed + 12)),
            (6, 0, guard(0.12, seed + 13)),
        ]
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--content",
        default="content/chars",
        help="directory containing kaen/, nival/ and ordan/",
    )
    args = parser.parse_args()
    root = Path(args.content)

    for name, flavor, seed in [
        ("kaen", "fire", 1_000),
        ("nival", "frost", 2_000),
        ("ordan", "earth", 3_000),
    ]:
        path = root / name / f"{name}.snd"
        path.write_bytes(character_bank(flavor, seed))
        print(f"wrote {path} ({path.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
