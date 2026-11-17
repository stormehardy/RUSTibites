# Testing-policy migration audit — `Default.zip` as the canonical world

**Status:** audit + docs migration (task-97). **Policy:** [`TESTING_POLICY.md`](TESTING_POLICY.md)
(committed `baff61c`, 2026-10-30): the game's default scenario `testdata/real/Default.zip` is the
canonical test world; `lol.zip`/`Ring.zip`/other saves are historical fixtures only with a documented
reason.

**Canonical reference (policy):** `rustibites run testdata/real/Default.zip --ticks 600 --seed 12345` →
1 → 10 living (1 initial + 9 spawner births), energy mean 156.9 → **115.41**, plant pellets 440 → **447**,
no deaths in the first 600 ticks (scenario `Default`, simTPS 30, simSize 2000). Pellet count is emergent
(±1 with faithful physics/feeding timing; body task-106) — migration-time audit evidence below still
quotes the then-current 448.

## Method

```
Select-String -Path docs,crates,tools,README.md,scripts,.github -Pattern `
  'lol\.zip|lol_round2|lol_50k|feeding_lol|Ring\.zip|ring_parity|feeding_ring|ring_eco|ring-eco|NestEgg\.zip|RadTower\.zip'
```

Result: **205 reference lines across 54 files**. Each was classified as:

| Class | Meaning | Action |
|---|---|---|
| **A — must change** | primary/default test command, docs example, script default, or viewer/bench example | switch to `Default.zip`; label old world as historical |
| **B — historical fixture** | explicit regression/report for a past task (evolved-save parity, Ring spawn bug, 50k evolution, format matrix) | keep; state the fixture reason and label the world next to every number |
| **C — evidence citation** | spec/format evidence extracted from real files (`lol.zip`, `Ring.zip`, `RadTower.zip`, `NestEgg.zip`) | keep; world is already named; no command |
| **D — not a world** | asset/sprite names, CLI arg-parsing token (`"ring.zip"`), unrelated text | no action |

## Audit summary by file

| File | Lines (representative) | Usage | Class → action |
|---|---|---|---|
| `README.md` | 33–36, 44, 74 | primary quickstart commands (`Ring.zip`, `lol.zip`, `ring_parity.zip`, viewer demo) | **A → fmt-dev** switch to Default.zip |
| `docs/USER_GUIDE.md` | 36–39, 52, 103, 118, 129, 166 | primary run/info/check/bench/viewer commands | **A → fmt-dev** |
| `docs/INSTALL.md` | 52 | install scenario list (`Ring.zip`) | **A → fmt-dev** use `Default.zip` in the example/lead list |
| `docs/VIEWER.md` | 11, 34–35, 38, 72, 118, 133 | viewer demo commands + Ring screenshot | **A → fmt-dev/view-dev** Default demo; Ring screenshot labelled historical |
| `crates/bibites-view/src/main.rs` | 1264–1267 | CLI usage examples in `--help` | **A → view-dev** Default primary, historical fixture lines optional |
| `tools/viewer/launch.ps1` | 6, 11 | launcher default input `Ring.zip` | **A → view-dev/fmt-dev** default `Default.zip` |
| `docs/PARITY.md` | 102–103, 483–571, 588, 614, 628 | capture workflow and Ring/lol parity fixtures | **B → parity-dev** keep as historical harness report; add a Default-first note and a Default capture recipe |
| `docs/PARITY_ROUND2.md` | 39 lines (whole Round 2–5 report) | historical lol/Ring parity + feeding references | **B → spec-dev done:** canonical banner + Default baseline added; per-round sections labelled historical |
| `docs/ECOLOGY.md` | 14 lines | historical Ring/lol experiments + dense-founder builder | **B → spec-dev done:** banner, Default first canonical command, historical fixtures labelled |
| `docs/design/FEEDING_METRICS.md` | §8 validation commands | Ring/lol validation | **A/B → spec-dev done:** new §8.2 Default canonical baseline; §8.3 Ring and §8.4 lol are labelled historical-fixture regressions |
| `docs/API.md` | 130, 143, 242–251 | library/CLI examples with Ring.zip | **A → spec-dev done:** examples use Default.zip; Ring cited as historical |
| `docs/design/SOA_STORE.md` | 242 | acceptance gate uses Ring capture | **A → spec-dev done:** Default capture canonical, Ring retained historical |
| `docs/design/CHECKPOINT_STATE.md` | 230, 236 | resume verification on Ring/lol saves | **A/B → spec-dev done:** Default canonical, Ring/lol historical regressions |
| `docs/design/UI_PARITY.md` | 386 | lineage screenshot "use lol.zip-style save" | **B → spec-dev done:** canonical grown Default run; lol opt-in historical |
| `docs/spec/README.md` | — | spec index | **policy pointer added** (specs cite real files as evidence, class C) |
| `docs/spec/01,03,04-*.md` | 01: 375/1711/2061; 03: 98/327/1505; 04: 351/356/702/1381/2173 | format/settings evidence from real files (`simTPS` overrides, section versions) | **C → keep** (world named; no commands). Optional fmt-dev/Lead: add a one-line "historical evidence" note where ambiguous |
| `docs/ARCHITECTURE.md` | 219, 226, 274, 470 | historical measured runs (lol load, Ring scenario round-trip, 50k evolve timing) | **B → Lead/world-dev** keep; label as historical measurements (already names the file) |
| `docs/COMPATIBILITY.md` | 146, 153, 159 | `RadTower.zip` format repro | **B → fmt-dev** documented fixture reason (format edge case); fine as-is |
| `docs/BENCHMARKS.md` | 526 | `bench -Scenario Ring.zip` example | **A → bench-dev** switch to Default.zip |
| `tools/bench/README.md` | 123, 218 | `-Scenario` example uses Ring.zip | **A → bench-dev** Default.zip |
| `tools/bench/run_matrix.ps1` | 45 | usage example Ring.zip | **A → bench-dev** Default.zip |
| `docs/EVOLUTION_RESULTS.md` | 11, 24, 27 | evolution experiment table/commands on `lol.zip` | **B → brain-dev** evolved-save fixture with reason; keep and label, optionally add a Default evolution baseline |
| `tools/evolve/run_experiment.ps1` | 9 | usage example `lol.zip` | **B → brain-dev** historical evolution fixture; add Default example |
| `crates/bibites-cli/src/capture.rs` | 2508, 2610/2618, 2749/2766 | `ring_parity.zip` capture-vs-game test; lol determinism/lifecycle event tests | **B → body-dev** intentional real-fixture regressions (documented reason in comments); recommendation: add a Default-based capture/lifecycle test as the primary and mark Ring/lol tests `#[ignore]`/historical |
| `crates/bibites-cli/src/args.rs` | 444, 456 | arg-parsing test literal `"ring.zip"` | **D → no action** (not a real fixture) |
| `crates/bibites-io/tests/{bundle_i3,robustness,world_i4a}.rs` | bundle 107/147/287; robust 73–75/175; world 72/127/159 | real-file format/robustness matrix (includes `Default.zip` already; uses `lol`/`Ring`/`RadTower`/`NestEgg` for version coverage) | **B → fmt-dev** keep as deliberate fixture matrix; add a comment naming the reason, ensure `Default.zip` is in the primary set (it is) |
| `crates/bibites-io/examples/export_scenario.rs` | 15 | default path `Ring.zip` | **A → fmt-dev** default `testdata/real/Default.zip` |
| `crates/bibites-io/examples/radtower_diag.rs` | 1, 99 | RadTower diagnostic | **B → fmt-dev** documented fixture (keep) |
| `crates/bibites-core/src/world.rs` | 2466, 4974 | comments describing "fresh Ring scenario" cadence | **B → world-dev** keep as historical note or generalise to "fresh scenario"; not a command |
| `crates/bibites-core/src/pellet.rs` | 9 | RadTower settings/evidence comment | **C → keep** |
| `crates/bibites-core/src/settings.rs` | 5 hits (Ring/lol settings comments) | evidence for simTPS/real settings | **C → keep** |
| `tools/parity/capture.py` | 34 | usage docstring Ring.zip | **A → parity-dev** Default example (keep a historical example note) |
| `tools/parity/dump_prefabs.py` | 15, 88 | `--scenario` **default** `Ring.zip` | **A → parity-dev** default `Default.zip` |
| `tools/parity/ecoref.py`, `ecoref_long.py` | 2–6, 186 | Ring-specific ecology reference docs | **B → parity-dev** historical Round-3/4 fixture; label; add Default path when reused |
| `tools/parity/exp_behavior.py` | 45 | `DEFAULT_BASE` = `ring_parity.zip` | **B → parity-dev** historical behavioural experiments; Default base when refreshed |
| `tools/parity/probe.py`, `probe_rotate.py` | 22; 15, 31 | example/default bases | **A/B → parity-dev** Default primary; rotate probe base fixture can stay historical |
| `tools/parity/README.md` | 27–30 | capture walkthrough from Ring.zip | **A → parity-dev** Default walkthrough + historical note |
| `tools/parity/feeding_ref.py` | 17 | usage example path `feeding_lol_600` | **B → parity-dev** historical feeding reference artifact (keep, label) |
| `tools/parity_round2/*` | prepare_lol_base 4; capture_scenario 19/174; analyze_determinism 63 | Round-2/3 reproduction scripts | **B → parity-dev** historical reproduction; `capture_scenario.py` default should become `Default.zip` when reused |
| `tools/parity_round2/prepare_lol_base.py` | 2, 19, 32–33 | builds `lol_round2` fixture | **B → parity-dev** keep (historical fixture builder) |
| `tools/assets/capture_reference.py` | 5, 28 | reference screenshots default source `lol.zip` | **A/B → parity-dev** default `Default.zip`; label lol reference as historical |
| `tools/assets/README.md` | 27 | describes capture_reference lol source | update with the change |
| `tools/experiments/run_ecology.py` | 16, 18 | docstring examples Ring/lol | **B → spec-dev follow-up** docs now lead with Default; script examples can be updated (historical experiments retain their inputs) |
| `tools/experiments/analyze_capture.py` | 9–10 | 50k evolution artifact | **B → spec-dev follow-up** historical artifact (label) |
| `tools/experiments/build_dense_world.py` | 5, 22 | builds from `lol.zip` | **B → spec-dev follow-up** historically-scoped fixture builder; docs/ECOLOGY labels it |
| `tools/experiments/run_round57.ps1` | 3, 5, 20–22 | historical batch (Ring/lol fixtures) | **B → spec-dev follow-up** keep as the record of the pre-policy runs |
| `tools/roundtrip/start_rust_scenario.py` | 11, 98 | default `--original Ring.zip` | **A → fmt-dev** default `Default.zip`; Ring optional historical |
| `docs/ASSETS.md` | 66 | reference frames from `lol.zip` | **B → parity-dev/fmt-dev** label historical; new reference frames from Default |
| `docs/design/CHECKPOINT_STATE.md`, `SOA_STORE.md`, `UI_PARITY.md` | see above | historical verification/screenshot references | **A/B → spec-dev done** |
| `docs/TESTING_POLICY.md` | 16, 21, 45 | policy itself lists historical fixtures | **no change** (policy text) |

Counts: 54 files, **205** reference lines. Approximately **20 files (class A, primary commands/defaults)
or ~35 lines** need an owner change; the rest are intentional historical fixtures, evidence citations,
or non-world tokens.

## Changed in this task (spec-dev, docs only)

* `docs/PARITY_ROUND2.md` — canonical-world banner + Default baseline command/numbers; §1 renamed
  "Historical Round-2 fixtures"; all Round 2–5 sections remain but are explicitly historical.
* `docs/ECOLOGY.md` — policy banner; Default canonical experiment command first; Ring/lol commands and
  the dense founder builder explicitly marked historical fixtures; removed the stale "all runs 15 TPS"
  statement (per-world TPS).
* `docs/design/FEEDING_METRICS.md` — new **§8.2 Default scenario (canonical baseline)** with exact
  commands/acceptance; §8.3 Ring and §8.4 lol renamed/relabelled historical-fixture regressions; §8.5
  statistical protocol says run Default for the canonical envelope; Appendix B commands lead with
  Default; status/§3 updated for world-dev **task-99** conservation accounting approval; §9 ownership
  rows updated (body/world/parity coordination for a gateable conservation identity).
* `docs/API.md` — scenario example, CLI capture/stats/bench/info/check examples use `Default.zip`; Ring/lol
  listed as opt-in historical fixtures.
* `docs/design/SOA_STORE.md` — acceptance gate uses Default capture; Ring retained historical.
* `docs/design/CHECKPOINT_STATE.md` — policy note; P5 verification uses Default (Ring/lol historical).
* `docs/design/UI_PARITY.md` — lineage screenshot grown from Default; lol opt-in.
* `docs/spec/README.md` — policy pointer; real-file citations are evidence, not default commands.

## Intentionally historical (keep; label next to numbers)

* Entire parity rounds 2–5 (`docs/PARITY_ROUND2.md`, `docs/PARITY.md` §10) and feeding references
  `feeding_lol_600` / `feeding_ring_scenario_300` — the record of evolved-save/scenario-start behaviour.
* `testdata/parity/bases/{ring_parity,lol_round2}.zip` and the Round-2/3 support scripts
  (`tools/parity_round2/`) — fixture builders/reproducers.
* `docs/ECOLOGY.md` Ring/lol experiment tables and `tools/experiments/{build_dense_world,run_round57,analyze_capture}` —
  pre-policy ecology record (future runs: Default).
* `crates/bibites-io/tests` real-file matrix — format/version coverage needs varied real files
  (`Default.zip` is included as the primary scenario; `lol`/`Ring`/`RadTower`/`NestEgg` broaden coverage).
* `crates/bibites-cli/src/capture.rs` Ring parity + lol determinism/lifecycle tests — historical
  regression fixtures (recommend a Default-based primary test alongside, see below).
* `docs/spec/*` real-file evidence citations and `docs/ARCHITECTURE.md` measured-run numbers.
* `crates/bibites-core/src/{world,pellet,settings}.rs` comments citing Ring/RadTower values — descriptive,
  not test commands.

## Migration status (2026-10-30, after owner reports)

| Owner | Scope | Status |
|---|---|---|
| spec-dev | docs I own (PARITY_ROUND2, ECOLOGY, design/*, API, spec pointer) | ✅ committed `d79d931` |
| fmt-dev | README/USER_GUIDE/INSTALL/VIEWER, viewer launcher, io example, roundtrip default, io-test comments | ✅ committed (`ab277b5`): Default quickstart `run --ticks 600 --seed 12345` → **1→10 living, 0 corpses, 448 pellets** (440 prepared); viewer demo Default; `docs/images/viewer*.png` regenerated from Default; remaining Ring/lol lines are explicitly labelled historical opt-in |
| body-dev | capture tests; task-90 counters | ✅ Default capture/lifecycle test added (`capture.rs:2499/2537`), Ring parity/lol determinism kept as documented historical regressions; task-90 feeding counters landed (`eb5294d`) |
| view-dev | `crates/bibites-view/src/main.rs` usage examples | ✅ committed: Default primary; Ring/lol lines labelled historical opt-in (`dba6fb6` viewer task-100) |
| brain-dev | `docs/EVOLUTION_RESULTS.md`, `tools/evolve/run_experiment.ps1` | ✅ committed (`d07c7c7`): historical `lol_100k`/`resume_check` labelled; canonical `default_baseline` row added (Default 10k ticks, 2.64 s, ≈126× real-time, pop 1→10→1 smoke); Default command first in reproduction/usage; PS 5.1 stderr fix |
| parity-dev | `docs/PARITY.md`, tools/parity defaults/examples, Default game capture, compare sections | ✅ committed `94694e1` (canonical Default baseline), `77f7613` (feeding/conservation compare sections, self-tests pass), `c2665d1` (caller-path exact hooks + `tools/parity/README.md` Default walkthrough + Appendix-D merge), `f848bfc` (game conservation gate) and `7764d6f` (Rust `counters` gate + refreshed Rust captures; **two-sided PASS** at 600/3000 t). Remaining optional: meat/decay-world capture to close the 0.2464 % game term |
| bench-dev | `docs/BENCHMARKS.md`, `tools/bench/README.md`, `run_matrix.ps1`, world labels | ✅ committed (`21d15b8`, `e298b69`, `e29bb31`, `b51ceda`; frozen-binary provenance in `tools/bench/results/final/binary-provenance.json`): `-Scenario testdata\real\Default.zip` examples; Canonical-scenario section; synthetic worlds kept/allowed; historical Ring/lol quotes labelled; scenario-mode Markdown carries the world; smoke test Default 600 ticks → 1→10; ARCHITECTURE Ring 1,490 tps sentence labelled historical |
| world-dev | `world.rs` comments + task-99 accounting | ✅ committed (`02fb13a`): "fresh Ring scenario" comments generalised; task-99 conservation ledger/reports/residuals + §6.1 f64 rates landed and Default-validated (residual ≈2e-9 relative); task-85 spawn fix green |

Re-audit after owner commits: **all class-A migration items are committed or explicitly labelled
historical**; the two-sided conservation gate is live and passing (`7764d6f`). Only optional follow-ups
remain: meat/decay-world capture to close the game 0.2464 % term, and the task-93 per-capita feeding gap.
`docs/PARITY_ROUND2.md`, `docs/ECOLOGY.md`, `docs/design/*` are done; `docs/PARITY.md`/`tools/parity` are
Default-first with historical fixture labels.

## Per-owner follow-ups

**fmt-dev (README/USER_GUIDE/INSTALL/VIEWER + io examples + roundtrip):**
1. `README.md`: replace the Ring/lol quickstart (`info`/`run`/`bench`/viewer) with Default.zip at
   `--ticks 600 --seed 12345`; keep a labelled "historical fixtures" line.
2. `docs/USER_GUIDE.md`: same for all command blocks (lines 36–39, 103, 118, 129, 166); use `Default.zip`.
3. `docs/INSTALL.md` line 52 and `docs/VIEWER.md` lines 11/34–35/38/72/118/133: Default examples; Ring
   screenshot labelled historical.
4. `crates/bibites-io/examples/export_scenario.rs` line 15: default path → `testdata/real/Default.zip`.
5. `tools/roundtrip/start_rust_scenario.py` line 98: default `--original` → `Default.zip`.
6. Keep `crates/bibites-io/tests` real-file matrix; add a one-line comment stating each non-Default file
   is deliberate version/format coverage (not a default baseline).

**parity-dev (harness + docs/PARITY.md):**
1. `docs/PARITY.md`: add a Default-first note/capture recipe; label §10 Ring fixture historical.
2. `tools/parity/README.md`, `capture.py` docstring, `probe.py`: Default examples.
3. `tools/parity/dump_prefabs.py --scenario` default → `Default.zip`; `probe_rotate.py`/`exp_behavior.py`
   bases: Default primary, Ring historical.
4. `tools/assets/capture_reference.py DEFAULT_SAVE` → `Default.zip`; refresh reference frames; label
   existing lol frames historical.
5. Keep `tools/parity_round2/*` and `feeding_ref.py` historical examples; `capture_scenario.py` default
   `--scenario` → `Default.zip`.

**body-dev (capture tests; task-90):**
1. Add a Default-based capture test (write + read a 600-tick Default capture, lifecycle events) as the
   primary; keep Ring parity/lol determinism tests as documented historical regressions (comments/ignore).
2. `crates/bibites-cli/src/capture.rs` lines 2508/2610/2749: add comments naming the fixture reason; no
   behaviour change.

**view-dev:**
1. `crates/bibites-view/src/main.rs` usage examples (1264–1267): `Default.zip` first; keep one
   historical-fixture line.

**bench-dev:**
1. `docs/BENCHMARKS.md` line 526, `tools/bench/README.md` 123/218, `run_matrix.ps1` 45: `-Scenario
   testdata\real\Default.zip`; synthetic bench worlds remain allowed.

**brain-dev:**
1. `docs/EVOLUTION_RESULTS.md` and `tools/evolve/run_experiment.ps1`: label `lol.zip` as the evolved-save
   historical fixture; add a Default evolution baseline/example.

**world-dev / body-dev:**
1. Task-99 accounting hooks + task-90 counters together make `docs/design/FEEDING_METRICS.md` §3 gateable; use
   Default.zip for the first conservation validation, with Ring post-fix as historical regression.

**Lead:**
1. Decide the canonical Default capture fixture (game + Rust) and whether to refresh the Round-1/2
   fixtures or keep them frozen historical.
2. Recommend closing `docs/TESTING_POLICY.md` migration checklist items once the fmt-dev/parity-dev items
   land; no further spec-dev docs work needed except `tools/experiments` docstrings (historical).

## Recommendation for tests that hard-code non-default worlds

* Keep **format/robustness tests** over the real-file matrix (they need version diversity); state in a
  comment that `Default.zip` is the primary and the others are coverage.
* Convert **integration/parity/lifecycle/threading** tests to Default.zip as the primary, and keep the
  Ring/lol variants only where the fixture is the point (evolved brain, spawn-bug regression, 50k
  evolution) — mark them with a comment and `#[ignore = "historical fixture: <reason>"]` only if the
  runtime cost matters.
* Prefer an env var/CLI arg (`RUSTIBITES_TEST_WORLD`) for optional historical fixtures so CI without
  `testdata/` skips cleanly (existing behaviour already prints `skipping ... testdata missing`); the
  Default path must remain the one documented in docs/commands.
