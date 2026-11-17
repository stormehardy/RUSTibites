"""Generate a deterministic corruption corpus for manual/CI fuzz runs (task-55).

Usage: python tools/fuzz/gen_corpus.py [--out target/fuzz-corpus] [--seed 1234]

Reads game files from testdata/real when present, writes mutated copies to the
output directory. The Rust test suite (`crates/bibites-io/tests/robustness.rs`)
runs the same mutation classes in-process; this script is for curated corpora
and for files too large to commit.
"""
from __future__ import annotations

import argparse
import random
from pathlib import Path


def mutate(data: bytes, rng: random.Random) -> bytes:
    if not data:
        return b"{}"
    kind = rng.randrange(6)
    if kind == 0:
        return data[: rng.randrange(len(data)) or 1]
    if kind == 1:
        out = bytearray(data)
        for _ in range(rng.randint(1, 4)):
            out[rng.randrange(len(out))] ^= 1 << rng.randrange(8)
        return bytes(out)
    if kind == 2:
        out = bytearray(data)
        out.insert(rng.randrange(len(out)), rng.randrange(256))
        return bytes(out)
    if kind == 3:
        out = bytearray(data)
        del out[rng.randrange(len(out))]
        return bytes(out)
    if kind == 4:
        return b"not json at all \x00\xff"
    return b"{" + bytes(data[:32])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="target/fuzz-corpus")
    ap.add_argument("--seed", type=int, default=1234)
    args = ap.parse_args()
    rng = random.Random(args.seed)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    real = Path("testdata/real")
    sources = []
    for pattern in (
        "saves/*.zip",
        "*.zip",
        "templates/*.bb8template",
        "templates_current/*.bb8template",
        "*.bb8",
        "*.bb8settings",
        "speciesData.json",
    ):
        sources.extend(sorted(real.glob(pattern)))
    if not sources:
        print(f"no sources under {real}; nothing to do")
        return 0
    written = 0
    for source in sources:
        data = source.read_bytes()
        for index in range(3):
            target = out / f"{source.stem}_{index}{source.suffix}.bin"
            target.write_bytes(mutate(data, rng))
            written += 1
    print(f"wrote {written} mutated files to {out} from {len(sources)} sources")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
