import gzip
import json
import sys
from pathlib import Path

ROOT = Path(r"Z:\RUSTibites")
sys.path.insert(0, str(ROOT / "tools" / "parity"))
import parity_common as pc  # noqa: E402


def load(path: Path):
    cap = pc.open_capture(path)
    return {int(r["tick"]): r for r in cap.records()}


p0 = load(ROOT / "testdata/parity/captures_round2/lol_seed12345/pass-0")
p1 = load(ROOT / "testdata/parity/captures_round2/lol_seed12345/pass-1")
common = sorted(set(p0) & set(p1))
diff = [t for t in common if pc.record_digest(p0[t]) != pc.record_digest(p1[t])]
print("game-vs-game lol: common", len(common), "differing", len(diff), "first", diff[0] if diff else None)


def flat(o, pref=""):
    out = {}
    for k, v in o.items():
        if isinstance(v, dict):
            out.update(flat(v, pref + k + "."))
        elif isinstance(v, list) and v and isinstance(v[0], (int, float)):
            for i, x in enumerate(v):
                out[pref + k + f"[{i}]"] = x
        else:
            out[pref + k] = v
    return out


if diff:
    t = diff[0]
    a, b = flat(p0[t]), flat(p1[t])
    shown = 0
    for k in sorted(set(a) | set(b)):
        if a.get(k) != b.get(k):
            print(f"  tick {t} {k}: left={a.get(k)!r} right={b.get(k)!r}")
            shown += 1
            if shown > 12:
                break
    # magnitude of the first few numeric diffs at the last common tick
    t = common[-1]
    a, b = flat(p0[t]), flat(p1[t])
    ms = []
    for k in sorted(set(a) & set(b)):
        try:
            d = abs(float(b[k]) - float(a[k]))
        except (TypeError, ValueError):
            continue
        if d:
            ms.append((d, k, a[k], b[k]))
    ms.sort(reverse=True)
    print(f"  last common tick {t}: largest numeric diffs:")
    for d, k, av, bv in ms[:8]:
        print(f"    {k}: |d|={d:.6g} ({av!r} vs {bv!r})")

for label, path in (("game", ROOT / "testdata/parity/captures_round2/lol_seed12345/pass-0"),
                    ("rust", ROOT / "testdata/parity/rust_lol_round2")):
    m = json.loads((path / "manifest.json").read_text(encoding="utf-8"))
    print(label, "counts_start", m.get("counts_start"), "counts_end", m.get("counts_end"),
          "settings sim_tps", (m.get("settings") or {}).get("independent", {}).get("simTPS"),
          "virgin", (m.get("settings") or {}).get("independent", {}).get("virginSpawnRate"),
          "base_time", (m.get("sim") or {}).get("base_time"))
