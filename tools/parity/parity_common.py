#!/usr/bin/env python3
"""parity_common.py - the wire contract between the game ground-truth capture and RUSTibites.

Everything a capture file means is defined here (and in docs/PARITY.md). `capture.py` writes
this schema, `compare.py` reads it, and the Rust side (`bibites-cli`) must emit the same schema.

Schema version: ``rustibites.parity.capture.v1``

Digests are computed over raw little-endian f32 bit patterns (and i32 for integer fields), never
over decimal formatting, so Python/C#/Rust hash exactly the same values. A digest is the first
16 hex characters of SHA-256 over the documented byte sequence.

GENE ORDER: the 35 genes are always in the game's BibiteGenes.Genes enum order (the order the
bridge reports in /series ``geneNames``). The capture manifest repeats the order in ``gene_names``.

Record types
------------
ticks.jsonl.gz   one JSON object per sampled tick (schema ``rustibites.parity.tick.v1``)
events.jsonl.gz  one JSON object per birth/death event (schema ``rustibites.parity.event.v1``)
manifest.json    capture identity + inputs + provenance (schema ``rustibites.parity.capture.v1``)
"""
from __future__ import annotations

import gzip
import hashlib
import json
import struct
from pathlib import Path

CAPTURE_SCHEMA = "rustibites.parity.capture.v1"
TICK_SCHEMA = "rustibites.parity.tick.v1"
EVENT_SCHEMA = "rustibites.parity.event.v1"
COMPARE_SCHEMA = "rustibites.parity.compare.v1"

# The game's BibiteGenes.Genes enum order (what Bridge /series reports as geneNames).
GENE_NAMES = [
    "LayTime", "BroodTime", "HatchTime", "SizeRatio", "SpeedRatio", "ColorR", "ColorG", "ColorB",
    "MutationAmountSigma", "AverageMutationNumber", "BrainMutationSigma", "BrainAverageMutation",
    "ViewAngle", "ViewRadius", "ClockSpeed", "PheroSense", "Diet",
    "HerdSeparationWeight", "HerdAlignmentWeight", "HerdCohesionWeight", "HerdVelocityWeight",
    "HerdSeparationDistance", "GrowthScale", "GrowthMaturityFactor", "GrowthMaturityExponent",
    "EyeOffset", "StomachWAG", "WombWAG", "FatWAG", "ArmorWAG", "ThroatWAG", "MouthMusclesWAG",
    "MoveMusclesWAG", "FatStorageThreshold", "FatStorageDeadband",
]

# Sampler groups the game emits (bridge >= 0.7.0). capture.py always asks for all of them.
SAMPLE_FIELDS = ["counts", "pellets", "vitals", "generation", "species", "brain", "genes",
                 "lifecycle", "tags", "behavior", "body"]

DIGEST_CHARS = 16


# -------------------------------------------------------------------------------------------------
# canonical hashing
# -------------------------------------------------------------------------------------------------
def f32_bits(value) -> bytes:
    """Little-endian IEEE-754 f32 bits of a JSON number (exact: JSON doubles round-trip f32)."""
    return struct.pack("<f", float(value))


def i32_bits(value) -> bytes:
    return struct.pack("<i", int(value))


def u8_bits(value) -> bytes:
    return bytes([1 if value else 0])


def f32_value(value) -> float:
    """Round-trip a JSON number through IEEE-754 f32 and return the widened double.

    Every simulation scalar in the capture schema is a game `float` (f32) widened to a JSON double.
    The game serialises .NET floats with shortest-round-trip formatting (e.g. 154.960068), which Python
    parses to a double that is NOT the exact widened f32 (154.96006774902344); RUSTibites widens f32 to
    f64 exactly. Quantising both sides to f32 bits before comparison makes bit-equality meaningful for
    declared-f32 fields without loosening any tolerance (see docs/PARITY.md 6/6.3).
    """
    return struct.unpack("<f", struct.pack("<f", float(value)))[0]


def sha16(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()[:DIGEST_CHARS]


def genes_digest(genes_by_name: dict, gene_names: list[str] | None = None) -> str:
    names = gene_names or GENE_NAMES
    h = hashlib.sha256()
    for name in names:
        h.update(f32_bits(genes_by_name[name]))
    return h.hexdigest()[:DIGEST_CHARS]


def _nodes(data: dict) -> list[dict]:
    brain = data.get("brain") or {}
    nodes = brain.get("Nodes") or []
    return sorted(nodes, key=lambda n: int(n.get("Index", 0)))


def _synapses(data: dict) -> list[dict]:
    brain = data.get("brain") or {}
    return list(brain.get("Synapses") or [])


def brain_digest(data: dict) -> str:
    """Topology + full runtime state (Value/LastInput/LastOutput) of every node, plus every synapse."""
    h = hashlib.sha256()
    nodes = _nodes(data)
    h.update(i32_bits(len(nodes)))
    for n in nodes:
        h.update(i32_bits(n.get("Type", 0)))
        h.update(i32_bits(n.get("Index", 0)))
        h.update(i32_bits(n.get("Inov", 0)))
        for key in ("baseActivation", "Value", "LastInput", "LastOutput"):
            h.update(f32_bits(n.get(key, 0.0) or 0.0))
    syn = _synapses(data)
    h.update(i32_bits(len(syn)))
    for s in syn:
        h.update(i32_bits(s.get("Inov", 0)))
        h.update(i32_bits(s.get("NodeIn", 0)))
        h.update(i32_bits(s.get("NodeOut", 0)))
        h.update(f32_bits(s.get("Weight", 0.0) or 0.0))
        h.update(u8_bits(s.get("En", False)))
    return h.hexdigest()[:DIGEST_CHARS]


def sense_digest(data: dict) -> str:
    """The 33 input node values (the game's sensor readings as the brain sees them)."""
    h = hashlib.sha256()
    inputs = [n for n in _nodes(data) if 0 <= int(n.get("Index", -1)) < 33]
    h.update(i32_bits(len(inputs)))
    for n in inputs:
        h.update(i32_bits(n.get("Index", 0)))
        h.update(f32_bits(n.get("Value", 0.0) or 0.0))
    return h.hexdigest()[:DIGEST_CHARS]


def output_digest(data: dict) -> str:
    """The 15 output nodes' Value/LastInput/LastOutput - the brain's motor output (brain IO digest)."""
    h = hashlib.sha256()
    outputs = [n for n in _nodes(data) if 33 <= int(n.get("Index", -1)) < 48]
    h.update(i32_bits(len(outputs)))
    for n in outputs:
        h.update(i32_bits(n.get("Index", 0)))
        for key in ("Value", "LastInput", "LastOutput"):
            h.update(f32_bits(n.get(key, 0.0) or 0.0))
    return h.hexdigest()[:DIGEST_CHARS]


def settings_hash(settings: dict) -> str:
    """SHA-256 over the bridge /settings JSON, encoded bit-exactly.

    For each scope in ("scenario", "independent"), keys sorted:  key "=" tag payload ";" then "}"
    tags: b + 1 byte (bool), i + i32 LE (int), f + f32 LE (float).
    Rust must use its setting types from docs/spec/04-formats-settings.md to pick the tag; when
    in doubt compare byte values with compare.py --strict-settings instead of the hash.
    """
    h = hashlib.sha256()
    for scope in ("scenario", "independent"):
        h.update(scope.encode("utf-8"))
        h.update(b"{")
        values = (settings or {}).get(scope) or {}
        for key in sorted(values):
            v = values[key]
            h.update(key.encode("utf-8"))
            h.update(b"=")
            if isinstance(v, bool):
                h.update(b"b")
                h.update(u8_bits(v))
            elif isinstance(v, int):
                h.update(b"i")
                h.update(i32_bits(v))
            else:
                h.update(b"f")
                h.update(f32_bits(v))
            h.update(b";")
        h.update(b"}")
    return h.hexdigest()


def canonical_json(obj) -> str:
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False, default=str)


def record_digest(record: dict) -> str:
    """Digest of one tick record's AGGREGATE telemetry.

    Entity snapshots are deliberately excluded: two repetitions of the same run may take their paused
    snapshot on slightly different ticks (Unity finishes already-queued fixed steps before a pause
    lands), so entities are compared separately on the ticks both captures actually have.
    Used by capture.py --repeat and compare.py --digest."""
    r = dict(record)
    for k in ("wall", "capture", "pass", "entities"):
        r.pop(k, None)
    return sha16(canonical_json(r).encode("utf-8"))


# -------------------------------------------------------------------------------------------------
# record builders
# -------------------------------------------------------------------------------------------------
def _get(d: dict, *keys, default=None):
    cur = d
    for k in keys:
        if not isinstance(cur, dict):
            return default
        cur = cur.get(k)
        if cur is None:
            return default
    return cur


def entity_record(data: dict, kind: str, gene_names: list[str] | None = None) -> dict | None:
    """One per-entity snapshot entry from the game's save-format entity JSON (/entities/json)."""
    if not data:
        return None
    genes_block = data.get("genes") or {}
    genes = genes_block.get("genes") or {}
    rb = data.get("rb2d") or {}
    tr = data.get("transform") or {}
    pos = tr.get("position") or [0.0, 0.0]
    body = data.get("body") or {}
    clock = data.get("clock") or {}
    brain = data.get("brain") or {}
    nodes = len(brain.get("Nodes") or [])
    synapses = len(brain.get("Synapses") or [])
    rec = {
        "kind": kind,
        "id": int(body["id"]) if body.get("id") is not None else None,
        "species": int(genes_block["speciesID"]) if genes_block.get("speciesID") is not None else None,
        "generation": int(genes_block.get("gen") or 0),
        "tag": genes_block.get("tag") or "",
        "x": float(rb.get("px", pos[0])),
        "y": float(rb.get("py", pos[1])),
        "vx": float(rb.get("vx") or 0.0),
        "vy": float(rb.get("vy") or 0.0),
        "rot": float(tr.get("rotation") or 0.0),
        "nodes": nodes,
        "synapses": synapses,
        "brain_digest": brain_digest(data),
        "sense_digest": sense_digest(data),
        "output_digest": output_digest(data),
    }
    if kind == "bibite":
        rec["energy"] = float(body.get("energy") or 0.0)
        rec["health"] = float(body.get("health") or 0.0)
        rec["age_s"] = float(clock.get("timeAlive") or 0.0)
        rec["size"] = float(body.get("d2Size") or 0.0)
    else:
        egg = data.get("egg") or {}
        rec["energy"] = float(egg.get("energy") or 0.0)
        rec["age_s"] = None
        rec["size"] = None
    if genes:
        try:
            rec["genes_digest"] = genes_digest(genes, gene_names)
            rec["genes"] = [float(genes[name]) for name in (gene_names or GENE_NAMES)]
        except KeyError:
            rec["genes_digest"] = None
    else:
        rec["genes_digest"] = None
    return rec


def tick_record_from_row(row: dict, *, source: str, tick: int, sim_tps: int, brain_tps: int,
                         settings_hash_value: str) -> dict:
    """Convert one bridge /series sampler row into the schema's tick record."""
    rec: dict = {
        "schema": TICK_SCHEMA,
        "source": source,
        "tick": int(tick),
        "t": float(row.get("t", 0.0)),
        "sim_tps": int(sim_tps),
        "brain_tps": int(brain_tps),
        "settings_hash": settings_hash_value,
    }
    if "bibites" in row:
        rec["counts"] = {"bibites": int(row.get("bibites", 0)), "eggs": int(row.get("eggs", 0)),
                         "corpses": int(row.get("corpses", 0))}
    if "plantN" in row:
        rec["pellets"] = {"plant_n": int(row.get("plantN", 0)), "plant_biomass": float(row.get("plantBiomass", 0.0)),
                          "meat_n": int(row.get("meatN", 0)), "meat_biomass": float(row.get("meatBiomass", 0.0)),
                          "free_biomass": float(row.get("freeBiomass", 0.0)),
                          "bibite_biomass": float(row.get("bibiteBiomass", 0.0))}
    if "energyMean" in row:
        rec["vitals"] = {"energy_mean": float(row.get("energyMean", 0.0)),
                         "health_mean": float(row.get("healthMean", 0.0)),
                         "age_mean": float(row.get("ageMean", 0.0)), "age_max": float(row.get("ageMax", 0.0))}
    if "genMax" in row:
        rec["generation"] = {"gen_max": int(row.get("genMax", 0)), "gen_mean": float(row.get("genMean", 0.0))}
    if "nSpecies" in row:
        rec["species"] = {"n": int(row.get("nSpecies", 0)),
                          "counts": {str(k): int(v) for k, v in (row.get("species") or {}).items()}}
    if "nodesMean" in row:
        rec["brain"] = {"nodes_mean": float(row.get("nodesMean", 0.0)), "nodes_max": int(row.get("nodesMax", 0)),
                        "syn_mean": float(row.get("synMean", 0.0)), "syn_max": int(row.get("synMax", 0))}
    if row.get("geneMean") is not None:
        rec["genes"] = {"mean": [float(x) for x in row["geneMean"]], "std": [float(x) for x in (row.get("geneStd") or [])]}
    if "births" in row:
        rec["lifecycle"] = {"births": int(row.get("births", 0)), "deaths": int(row.get("deaths", 0))}
    if row.get("tags") is not None:
        rec["tags"] = row["tags"]
    if row.get("outputs") is not None:
        rec["behavior"] = {"speed_mean": float(row.get("speedMean", 0.0)), "speed_max": float(row.get("speedMax", 0.0)),
                           "moving_frac": float(row.get("movingFrac", 0.0)),
                           "travel_rate_mean": float(row.get("travelRateMean", 0.0)),
                           "bite_rate_mean": float(row.get("biteRateMean", 0.0)),
                           "damage_dealt_rate_mean": float(row.get("damageDealtRateMean", 0.0)),
                           "egg_rate_mean": float(row.get("eggRateMean", 0.0)),
                           "outputs": {str(k): float(v) for k, v in row["outputs"].items()}}
    if row.get("sizeMean") is not None:
        rec["body"] = {"size_mean": float(row.get("sizeMean", 0.0)), "area_mean": float(row.get("areaMean", 0.0)),
                       "mass_mean": float(row.get("massMean", 0.0)),
                       "max_energy_mean": float(row.get("maxEnergyMean", 0.0)),
                       "max_health_mean": float(row.get("maxHealthMean", 0.0)),
                       "metabolism_mean": float(row.get("metabolismMean", 0.0))}
    metrics = {k[len("metric."):]: float(v) for k, v in row.items() if k.startswith("metric.")}
    if metrics:
        rec["metrics"] = metrics
    return rec


def event_record(event: dict, *, tick: int) -> dict:
    rec = {"schema": EVENT_SCHEMA, "tick": int(tick)}
    for key, value in event.items():
        if key in ("seq",):
            continue
        if key == "gen":
            rec["generation"] = value
        else:
            rec[key] = value
    rec.setdefault("b", None)
    rec.pop("b", None)
    return rec


# -------------------------------------------------------------------------------------------------
# capture files
# -------------------------------------------------------------------------------------------------
class Capture:
    def __init__(self, path: Path, manifest: dict, ticks_path: Path | None, events_path: Path | None):
        self.path = path
        self.manifest = manifest
        self.ticks_path = ticks_path
        self.events_path = events_path

    @property
    def source(self) -> str:
        return self.manifest.get("source", "unknown")

    @property
    def sim_tps(self) -> int:
        return int(_get(self.manifest, "sim", "sim_tps") or 30)

    @property
    def gene_names(self) -> list[str]:
        return list(self.manifest.get("gene_names") or GENE_NAMES)

    def records(self):
        yield from _read_jsonl_gz(self.ticks_path)

    def events(self):
        yield from _read_jsonl_gz(self.events_path)


def _read_jsonl_gz(path: Path | None):
    """Reads a capture stream from `.jsonl` (plain) or `.jsonl.gz`.

    RUSTibites writes plain UTF-8 JSONL (the harness treats gzip as optional),
    while game captures are gzipped; dispatch on the file suffix so both work.
    """
    if path is None or not path.exists():
        return
    if path.name.endswith(".gz"):
        import gzip as _gzip

        with _gzip.open(path, "rt", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if line:
                    yield json.loads(line)
    else:
        with open(path, "r", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if line:
                    yield json.loads(line)


def open_capture(path: str | Path) -> Capture:
    """Load a capture directory (manifest.json + ticks.jsonl.gz) or a bare .jsonl[.gz] file."""
    p = Path(path)
    if p.is_dir():
        manifest = {}
        mp = p / "manifest.json"
        if mp.exists():
            manifest = json.loads(mp.read_text(encoding="utf-8"))
        ticks = _first_existing(p / "ticks.jsonl.gz", p / "ticks.jsonl")
        events = _first_existing(p / "events.jsonl.gz", p / "events.jsonl")
        return Capture(p, manifest, ticks, events)
    if p.suffix == ".gz" or p.suffix == ".jsonl":
        manifest = {}
        mp = p.with_suffix(".manifest.json")
        if mp.exists():
            manifest = json.loads(mp.read_text(encoding="utf-8"))
        return Capture(p.parent, manifest, p, None)
    raise FileNotFoundError(f"not a capture: {p}")


def _first_existing(*paths: Path) -> Path | None:
    for p in paths:
        if p.exists():
            return p
    return None


def write_jsonl_gz(path: Path, records: list[dict]) -> dict:
    path.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(path, "wt", encoding="utf-8", compresslevel=6) as fh:
        for rec in records:
            fh.write(json.dumps(rec, separators=(",", ":"), ensure_ascii=False, default=str))
            fh.write("\n")
    raw = path.read_bytes()
    return {"name": path.name, "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw), "records": len(records)}


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()
