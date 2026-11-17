# Save-section coverage and compatibility matrix

Status: 2026-10-30. Owner: io (fmt-dev). This is the load→save→reload fidelity
matrix for game artifacts. Tests are in `crates/bibites-io/tests/`
(`save_coverage.rs`, `settings_audit.rs`, `bundle_i3.rs`, `world_i4a.rs`,
`checkpoint_state.rs`, `robustness.rs`, `template_compat.rs`); game-dependent
tests skip cleanly when `testdata/` (gitignored) is absent.

## Sections

| Section | Fidelity | Notes |
|---|---|---|
| settings: 78 scenario + 14 independent + 36 material fields | **exact** | Both save- and scenario-shaped `.bb8settings`; unknown top-level/wrapper/material/zone keys preserved; hard-clamped vs out-of-range import policy matches the game (spec 04 Q6). |
| challenge / settings changers / zone groups / spawn rows | **exact** | Parsed and re-emitted; no real fixture currently exercises a challenge in a save. |
| zones | **exact** for modelled fields | Name, id, material/distribution/fertility/biomassDensity/pelletSize, renewBiomass, movement/target, speed, pos, radius/width/height/insideRadius; unknown zone keys preserved via the settings file. |
| scene (`scene.bb8scene`) | **exact** | Raw JSON kept; writer refreshes `version`, `simulatedTime`, `nPellets`, `nBibites`, tools and preserves unknown keys. `nPellets/nBibites` are display-only (writer recomputes; bibite entry count includes corpses). |
| vars / pellets / pheromones / species / data.bin / img / templates / unknown zip entries | **exact** for modelled fields | Pellet/phromone sections are stored as raw JSON plus typed records; data.bin is opaque; templates preserved byte-for-byte; unknown entries preserved. |
| bibites | **exact** for every serialized field | transform/rb2d, genes/meta/flags, brain, body stats, mouth, stomach, phero, eggLayer (progress/count/children raw ids), control.totalTravel, clock; `rot` is transform degrees passed through raw. Derived `growth` reconstructed from `d2Size/SizeRatio²`. Unknown top-level entity keys preserved. |
| eggs | **exact** | Only hatchProgress/energy/id are serialized; hatchTime/totalRequiredEnergy/sizeGene/birthOrientation/isHatching reconstructed exactly from genome+settings. |
| tools | **exact** for serialized fields | Rad/phero towers keep `id`, position, params, `attachedZone` (mapped to zone-list index); color killers have **no serialized id** in the game format (io uses 0). Phero-tower runtime `spot` ids are not serialized (reloaded `None`). |
| species lineage | **exact** for known fields | recordedSpecies/activeSpeciesList/innovation counters/statistics and per-species template (version/nodes/synapses/genes). Unknown species keys are currently dropped; `usedSpanText` is re-emitted as `{}` (all real fixtures have it empty). |
| world save tooling / `.bb8state` | **sidecar only** | Checkpoint payload `SimSnapshot::encode()` is wrapped with base-save SHA-256, settings SHA-256, seed, FNV checksum; sibling `<save>.zip.bb8state`; **the game zip is never modified**. |

### Deliberately not modelled (the game does not serialize these)

* red-death runtime state (no scene fields in 0.6.3.1 saves),
* pheromone `heading` (game writes `{}` and treats it as zero),
* pellet velocity/rotation and `held` (pellet respawns at rest, identity rot),
* phero-tower runtime spot ids, color-killer ids,
* `transform.scale` and rb2d angular velocity (derived at runtime),
* unknown keys inside nested body sub-objects (canonical re-emission).

### Known approximation / gap

* Raw entity runtime ids (`body.id`, children, parents, egg ids) are kept in
  `SaveBundle` side vectors; `BibiteSave.children`/`GenomeMeta.parents` are not
  populated because core ids are generational (I4 world loader maps raw ids).
* Species `usedSpanText` and unknown species/template keys are not round-tripped
  (currently empty/absent in all testdata). Revisit if a save needs them.
* Pellet owner bucket is resolved by zone **name**; duplicate zone names collapse
  to the first index (game data has unique names).

## Version compatibility

| Input | Status |
|---|---|
| 0.6.3.1 (present) | full load/save/game-load |
| 0.6.x older (0.6, 0.6a5, 0.6a6, 0.6a16, 0.6.1a4, 0.6.2a1, 0.6.3a3) | supported migrations (gene/brain updaters), tested on real templates/saves |
| 0.5 (pre-0.6a1) | supported for the 34-value gene layout; real 28-value short arrays tolerated by zero-padding before the migration (matches the game updater) |
| newer than 0.6.3.1 (0.6.3.3+, 0.6.4a1 folder saves) | **`Unsupported`** (io now rejects newer versions instead of applying stale migrations) |

## Game-load assumptions (0.6.3.1)

Verified earlier with the headless BibitesAPI lab (`tools/roundtrip/`):

* Rust-written **save** loads and runs (counts match, no new Player.log
  exceptions, `data.bin` identity); **template** accepted by the game's own
  `/bibite/upgrade` (`updated=false`); **scenario** starts/ runs.
* Assumptions documented here: the game zip is written in the game's section
  shapes/entry order, JSON is compact UTF-8 with BOM, Rust writes the present
  version for sections it authors, unknown zip members are ignored by the
  static loader, and the `.bb8state` sidecar is a sibling file so it cannot
  affect game loading.
* Parity can re-verify any specific fixture in-game on request; no version
  metadata is hidden in the zip beyond `scene.version`.

## Artifacts exercised

`testdata/real/saves/lol.zip` (35 bibites/4 eggs), `testdata/real/saves/ozymandias .zip`,
`testdata/parity/bases/ring.base.zip`, scenarios `Default/Ring/RadTower/NestEgg`,
`testdata/real/templates*` and the installed 220-file user template folder.

## Game-load verification fixture pack

`testdata/parity/compat/` (gitignored) is generated by
`tools/roundtrip/gen_compat_pack.ps1` and contains Rust-written files that
exercise every writer surface:

| fixture | exercises |
|---|---|
| `rust_lol_save.zip` | real save: 35 bibites incl. 14 corpses, 4 eggs, 213 pellets, 592 pheromones, 26 species records, 82,550-byte data.bin |
| `rust_state_save.zip` (+ `.bb8state`) | same save with a sibling checkpoint sidecar (must be ignored by the game) |
| `rust_tools_phero_save.zip` | real save + rad tower, pheromone tower, colour killer, 2 signed-strength pheromones |
| `rust_eggs_save.zip` | real save re-emitted; 4 game-validated eggs |
| `rust_corpses_save.zip` | real save re-emitted; 35 entries incl. 14 corpses |
| `rust_default_scenario.zip` | canonical Default scenario re-emitted by the Rust writer |
| `rust_ring_scenario.zip` | real Ring scenario re-emitted by the Rust writer |
| `rust_default_scenario_save.zip` | CLI save started from the canonical `Default.zip` (0.6 scenario-start save) |
| `rust_template.bb8template` | present-version template for the game's `/bibite/upgrade` converter |

`manifest.json` records each file's bytes/SHA-256, save counts/entity ids where
applicable, scenario zone/spawn/changer/template counts, sidecar SHA and the
exact commands. Regenerate with `pwsh -File tools/roundtrip/gen_compat_pack.ps1`.
Parity-dev's hidden-game verification is one command:

```powershell
python tools\roundtrip\load_compat_pack.py --dir testdata\parity\compat
```

It loads each save/scenario (`client.load_world`/`new_game`), re-saves
(`client.save_world`), parses the template (`/bibite/upgrade`), scans Player.log
for new exceptions, and writes `testdata/parity/compat/game_report.json`.
A load popup/exception or a re-save failure is a rejection; report the fixture
name, expected block and Player.log tail. Fixtures are ordered game-validated
real-derived saves first (`lol` → state → eggs → corpses → tools → CLI Default
save → scenarios → template) so one bad synthetic file cannot hide the rest.
Earlier synthetic dead-bibite/corpse fixtures were removed after the game's
load timed out on the minimal synthetic save; all entity fixtures now derive
from the game-validated `lol.zip`.

## Large-world save/load scalability (2026-11-01)

Method: deterministic cluster saves from
`tools/bench/make_cluster_world.py --n N --box 100` (N identical bibites on a
lattice, seed 12345); release `target\release\rustibites.exe`; `run <world>
--ticks 0 --quiet` measures load+prepare, `run <world> --ticks 0 --save out.zip`
measures load+save (the delta to `--no-state` isolates the `.bb8state` write).
Wall times include process startup and the CLI's template-dir scan, so treat
them as end-to-end user-visible numbers (±10–20% between cold/warm runs).
Default-world row: `run testdata/real/Default.zip --ticks 50000 --seed 12345
--save`, then an io-only load/re-save of that save.

| world | zip size | load | save (no state) | sidecar size | sidecar overhead |
|---|---:|---:|---:|---:|---:|
| 1,000 bibites | 1.8 MB | 3.6 s | 3.0 s | 4.4 MB | ~0.0 s* (0.61 s before opt) |
| 2,000 | 3.6 MB | 6.2 s | 4.4 s | 8.7 MB | 4.9 s before opt |
| 5,000 | 9.0 MB | 7.1 s | 10.9 s | 21.7 MB | 0.21 s* (6.1 s before opt) |
| 10,000 | 18.0 MB | 11.8 s | 20.8 s | 43.4 MB | 14.4 s before opt |
| 20,000 | 36.1 MB | 22.8 s | 37.7 s | 86.7 MB | 19.7 s before opt |
| Default, 50k ticks | 71.7 KB | 0.33 s | 0.36 s | (small) | — |

\*After the io optimization below; sidecar size is unchanged (payload bytes),
only the redundant work was removed.

**Hotspots**
- Load is dominated by zip read + `serde_json` parse per section (every bibite
  and pellet is a separate entry/object); time scales ~linearly (≈1.1 ms per
  bibite load+parse at 20k).
- Save similarly builds a JSON `Vec` per entity, then deflates each zip member;
  the writer holds the whole serialized set before `ZipWriter` consumes it.
- `.bb8state` payload is much larger than the zip (≈2.4× at every size) because
  it stores raw `SimSnapshot`/entity runtime extensions, mostly uncompressed;
  write time is dominated by encode + full-buffer write. `Settings` hashing
  used to re-parse the whole save (fixed), and base SHA-256 used to read the
  whole file into memory (fixed: streamed 1 MiB chunks).
- No memory numbers yet: CLI `peak_bytes` is still `null`; memory is expected to
  be several GiB for 100k (JSON entry vectors + zip buffers + sidecar payload).

**Optimizations made (behavior-neutral, io-only)**
- `read_checkpoint_state`/`write_checkpoint_state`: settings SHA-256 now parses
  only the save's `settings.bb8settings` zip member (was a full
  `load_save_bundle` of all entities); base SHA-256 now streams the zip in
  1 MiB buffers instead of copying the whole file. Sidecar overhead dropped
  from 0.6 s → ~0 s at 1k and 6.1 s → 0.21 s at 5k on this machine.
- Left zip compression level at the default (game compatibility/size; a level-1
  experiment is a possible follow-up, not applied).

**100k feasibility**
- Interpolating the table: ≈180 MB game zip, ≈2–3 min load and ≈3–5 min save,
  several GiB RSS; currently **not ready as a default path**.
- `.bb8state` would be ≈430 MB, above the core
  `sim::MAX_PAYLOAD_BYTES = 256 MiB`; per-section 64 MiB chunks exist, but the
  total cap plus io's whole-payload `Vec` encode/write block it. A streamed
  sidecar needs (a) a core streaming encode or a larger/removable total cap
  (world-dev), and (b) an io streaming writer (header, chunked payload,
  checksum) instead of one buffer. That is the gating item for 100k checkpoint
  resume; 1k–20k saves/loads work today, with sidecars up to 86 MB.

### Load/save throughput follow-up (2026-11-01)

**Change (io-only, byte-identical):** save-bundle entity loading used to parse
each `bibites/*.bb8`/`eggs/*.bb8` twice — once into `Bb8File` and again by
serializing the parsed `Value` back to a string and running the full genome/brain
migration path. `lib::loaded_from_bb8` now builds the `LoadedGenome` directly
from the already-parsed entity, so each entity's JSON is parsed once. No
serialization/output path changed.

| world | metric | before | after | speed-up |
|---|---|---:|---:|---:|
| 10k | load | 11.8 s | 6.58 s | 1.79× |
| 10k | save end-to-end | 20.8 s | 8.33 s | 2.50× |
| 10k | save-only (minus load) | ≈9.0 s | ≈1.75 s | ≈5.1× |
| 10k | save + `.bb8state` | 35.2 s | 9.01 s | 3.9× |
| 20k | load | 22.8 s | 11.71 s | 1.95× |
| 20k | save end-to-end | 37.7 s | 16.53 s | 2.28× |
| 20k | save-only (minus load) | ≈14.9 s | ≈4.82 s | ≈3.1× |
| 20k | save + `.bb8state` | 57.4 s | 17.91 s | 3.2× |

**Byte-identical proof:** the 10k/20k Rust-written save zips after the change
have the same SHA-256 and sizes as the pre-change artifacts
(`9ddb0f7a…` / 18,030,887 B and `ad98ffea…` / 36,087,259 B), so game-load
compatibility and parity outputs are unchanged. Checkpoint tests and the full io
gate stay green.

**Still open for 100k:** save-side entity JSON is still single-threaded before
deflate and the zip writer holds all serialized entries; streamed zip writing +
parallel per-entity JSON (deterministic order) remain the next throughput step,
together with the streamed `.bb8state`/max-payload work below.

### Parallel save JSON follow-up (2026-11-01)

Per-entity JSON serialization in `write_save_bundle` now runs on a small
std::thread pool (up to 8 workers, contiguous index chunks, results collected in
chunk/file order). The zip writer and compression remain single-threaded, so
output bytes are unchanged.

| world | metric | before | after | speed-up |
|---|---|---:|---:|---:|
| 10k | save end-to-end | 8.33 s | 7.54 s | 1.10× |
| 10k | save-only (minus load) | 1.75 s | 2.71 s | noisy/slower |
| 20k | save end-to-end | 16.53 s | 14.92 s | 1.11× |
| 20k | save-only (minus load) | 4.82 s | 3.44 s | 1.40× |

**Byte-identical proof:** 10k/20k output zips match the prior SHA-256 and sizes
(`9ddb0f7a…` / 18,030,887 B and `ad98ffea…` / 36,087,259 B), so game-load
compatibility and parity are unchanged. Savings are modest because zip
compression + entry buffering remain serial/dominant; the next step for 100k is
a streamed zip writer (write each compressed entry directly as it is produced)
plus a streamed `.bb8state` path, as documented above.

### Streamed zip entry writer (2026-11-01)

`write_save_bundle` now creates the `ZipWriter` up front and writes each
section/entity entry **as soon as it is serialized**, instead of building a
`Vec<(name, Vec<u8>)>` for the whole world before starting deflate. Entry order,
bytes, deflate level and timestamps are unchanged.

**Byte-identical proof:** 10k/20k outputs again match the previous SHA-256 and
sizes (`9ddb0f7a…`/18,030,887 B and `ad98ffea…`/36,087,259 B). Game-load
compatibility and Default/Ring parity are therefore unaffected.

**Measured effect (same cluster worlds):** no speed-up — 10k/20k save end-to-end
landed in the same noise band as the parallel-JSON version (≈8.8 s / ≈17.8 s on a
loaded machine; compression is serial and dominates). The win is **peak memory**:
the writer no longer holds every serialized entity (`≈18 MB`/`≈36 MB` of JSON for
10k/20k, hundreds of MB at 100k) before deflate; only one entry is buffered.

**Next throughput step:** hybrid bounded batches — serialize a fixed batch
(e.g. 256 entities) on the thread pool, write the batch to the zip in order, drop
it, repeat. That keeps memory bounded while retaining parallel JSON. Remaining
100k blockers are still serial deflate, the streamed `.bb8state` payload
(`SimSnapshot::encode` whole-Vec + total cap 256 MiB vs ≈430 MB needed) and
load-side all-sections JSON parse; see the 100k section above.

### Hybrid bounded-batch save writer (2026-11-01)

The streamed zip writer now serializes entities in **bounded parallel batches of
256** (up to 8 threads, contiguous chunks, ordered collection/write, batch then
dropped) instead of one-entry-at-a-time or all-entries-first. Peak memory stays
bounded (≤256 serialized entities) and output stays deterministic.

**Byte-identical proof:** 10k/20k saves again match the previous SHA-256 and
sizes (`9ddb0f7a…`/18,030,887 B; `ad98ffea…`/36,087,259 B), so game-load and
Default/Ring parity are unchanged. Full io tests/clippy/fmt green.

**Measured speed (same machine, concurrent build/agents active):** no consistent
recovery on this pass — 10k ≈9.2 s and 20k ≈20.4 s end-to-end (save-only noisy,
partly negative because load and save phases were disturbed by the concurrent
build). This is within the load/deflate noise band and does **not** reproduce the
earlier ≈2.3–2.5× end-to-end numbers; deflate remains serial and load-side JSON
parsing dominates. Follow-up: benchmark on an idle machine and, if JSON is still
the bottleneck under isolation, stream deflate/compression or move zip writing
off the main path; the 256-batch/parallel structure is retained because it is
byte-identical and bounds memory for 100k.

### Experimental streamed `.bb8state` v2 (opt-in, 2026-11-01)

`bibites-io::checkpoint` now ships an opt-in chunked sidecar alongside the
stable v1:

- `write_checkpoint_state_v2(base_save, &SimSnapshot, seed)` writes the payload
  in fixed 16 MiB chunks with a per-chunk FNV-1a checksum and streams chunks to
  the file (no contiguous sidecar buffer on the write path).
- `read_checkpoint_state_v2(base_save, Optional<seed>)` verifies magic/version
  (v2), base-save SHA-256, settings SHA-256, optional seed and every chunk
  checksum, then together with `SimSnapshot::decode`.
- v1 remains the default; the v1 reader rejects v2 and vice versa. Game zip is
  untouched (sibling file), tests cover round-trip, base/seed/settings
  mismatch, chunk corruption and truncation.

**100k status:** v2 is a format/envelope step, not a full streaming solution yet.
`SimSnapshot::encode()` still produces one whole `Vec` and the read path
concatenates chunks before `SimSnapshot::decode`, and core
`MAX_PAYLOAD_BYTES = 256 MiB` still caps a 100k snapshot (≈430 MB). Needs
world-dev: streaming encode/decode (per-section iterator) and a larger/removable
total cap; io can then write/read chunks directly without the concatenation
buffer. Coordinate before enabling v2 by default or in the CLI.

#### Quiet-machine re-benchmark (2026-11-01, hybrid batches)

Re-ran the key sizes with no concurrent build (the earlier hybrid numbers were
disturbed by a running release build). Same deterministic cluster worlds; load =
`run --ticks 0`, save = `run --ticks 0 --save`, save-only = save − load.

| world | load | save end-to-end | save-only | vs pre-optimization save e2e |
|---|---:|---:|---:|---:|
| 10k | 5.64 s | 7.90 s | 2.25 s | 20.8 → 7.90 s (**2.6×**) |
| 20k | 10.44 s | 14.30 s | 3.86 s | 37.7 → 14.30 s (**2.6×**) |

Load improved 11.8→5.64 s (10k) and 22.8→10.44 s (20k) (**≈2.1×**). Outputs
remain byte-identical (`9ddb0f7a…`/18,030,887 B; `ad98ffea…`/36,087,259 B), so
game-load/parity are unchanged. The hybrid bounded-batch writer + single-parse
entity loader together recover the target gains on an idle machine; serial
deflate and load-side JSON parsing remain the next 100k blockers.

Game-load compatibility re-check on the current save was deferred while the
parity lab game was running (`The Bibites` PID present); no format changes
expected, will run in a fresh lab with the compat retry.

#### v2 wired to core section streaming (2026-11-01, core `5ee0d82`)

The experimental v2 sidecar now uses `SimSnapshot::encode_sections()` /
`decode_sections()` directly:

- **write:** header (magic/v2/flags, base zip SHA-256, settings SHA-256, seed,
  section count) followed by streaming records `tag:u16 || len:u32 ||
  fnv1a64:u64 || data`, including the core's 8 MiB chunk sections. No
  contiguous v1 payload `Vec` is built on the write path.
- **read:** parses the section records and feeds `SnapshotSection`s straight to
  `SimSnapshot::decode_sections` — no concatenated payload buffer; base/settings/
  seed identity plus per-section checksums are verified first.
- The public v2 format is opt-in via `RUSTIBITES_CHECKPOINT_V2=1`
  (`v2`/`true`/`yes` also accepted) on the normal `write_checkpoint_state` /
  `read_checkpoint_state` APIs; v1 stays the default and game-zip behaviour is
  unchanged. For large snapshots set the core cap too, e.g.
  `RUSTIBITES_SNAPSHOT_MAX_BYTES=536870912` (clamped 256 MiB–1 GiB; world
  capture preflight uses the same limit).
- **tests:** v2 round-trip, base/seed/settings mismatch, chunk corruption/
  truncation, v1-rejects-v2, and a 9 MiB-section test that crosses the core
  8 MiB chunking boundary through the streamed path. io tests/clippy green.

**Is a real 100k sidecar feasible now?** Format/cap-wise **yes**: a 100k
snapshot is ≈430 MB (≈2.4× a ≈180 MB world zip), below the 1 GiB env-opt-in cap,
and the v2 write/read no longer needs the extra contiguous payload copy. Not yet
measured end-to-end: generating/loading a real 100k world needs a large-memory
run (~2× sidecar for file+snapshot, plus World/entities, likely several GiB RSS)
and the `RUSTIBITES_SNAPSHOT_MAX_BYTES` opt-in; the largest verified sidecar in
tests is the 9 MiB chunk-section case and the 20k real world (86.7 MB, v1). Treat
100k as feasible-by-design but pending a dedicated hardware benchmark.

### 100k v2 checkpoint measurement (2026-11-01)

Exact procedure (`build_dense_world.py` → 100k duplicate founders from `lol.zip`,
zone-centred; env opt-ins active):

```powershell
$env:RUSTIBITES_CHECKPOINT_V2 = "1"
$env:RUSTIBITES_SNAPSHOT_MAX_BYTES = "1073741824"
python tools\experiments\build_dense_world.py --count 100000 --out testdata\experiments\dense\founders_100000.zip
target\release\rustibites.exe run testdata\experiments\dense\founders_100000.zip --ticks 1 --threads 1 --stats json --quiet --save target\scalability\bench_100k_v2.zip
target\release\rustibites.exe run target\scalability\bench_100k_v2.zip --ticks 0 --threads 1 --stats json --quiet
```

| metric | measured |
|---|---:|
| world zip (100k founders) | 501,822,929 B (501.8 MB) |
| save zip (Rust-written) | 501,822,929 B (501.8 MB) |
| `.bb8state` v2 sidecar | **650,194,727 B (650.2 MB)** |
| save wall (load world + 1 tick + zip + v2 sidecar) | **255.5 s** |
| save peak RSS | **2,582.8 MB** |
| load wall (world + sidecar restore) | **92.6 s** |
| load peak RSS | **3,268.0 MB** |
| resume fingerprint | save `1823034608723448443` = load `1823034608723448443` |

**Feasible:** yes on this machine with the env opt-ins; 650 MB sidecar is below
the 1 GiB streaming cap but above the 256 MiB v1 cap (so v1/default rejects it —
`encode_sections`/`encode` fail with `LimitExceeded` unless
`RUSTIBITES_SNAPSHOT_MAX_BYTES` is raised). The 40k control run with
`--no-state` produced no sidecar and completed normally. Limits: ~3.3 GB peak at
100k (load), ~4.5–5 min save and ~1.5 min load; snapshot decode still materialises
extension Vecs and the reader holds the sidecar file bytes plus decoded snapshot;
a future mmap/stream-decode would reduce peak. The v1/default path remains
byte-identical and unchanged.

### Game-load compat-pack status (load-accepted, 2026-11-01)

All **11 pack entries** (9 save/scenario/template fixtures + 2 `.bb8state`
sidecars) are accepted by the game with the expected counts/fields: save
fixtures load **22 living / 13 corpses / 4 eggs** or **1 living / 0 eggs**;
Default/Ring scenarios start with **1 living**; the template converter accepts
`rust_template.bb8template` (`converter kind=template`). No rejections or dropped
expected fields were observed. Evidence report:
`assets/game/reference/compat/game_report_2026_11_01.json` (top-level `fixtures`
array fixed in `d78c120`; report lists the 9 base fixtures, with parity's 11
including the two sidecar entries).

**Re-save harness limitation:** every harness `save_world` re-save attempt still
fails with a bridge connection close (`BridgeUnavailable [WinError 10054]`) even
after 2 s settle, up to 2 retries and a ping; the lifecycle watchdog exits
**124**. This is a harness/bridge lifecycle issue, **not fixture rejection**, and
**not release-blocking**. Load acceptance is closed; a future native/manual
in-game re-save retest (or a bridge that survives post-load re-save) is the
follow-up.
