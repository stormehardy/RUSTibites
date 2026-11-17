# Dense-cluster benchmark scenario inputs

These are the tracked inputs for the dense-pile benchmark mode
([`tools/bench/run_cluster.ps1`](../../run_cluster.ps1)). They are deliberately
tiny and self-contained: no `testdata/`, no game install, no saves checked in.

| File | Purpose | Provenance |
|---|---|---|
| `scenario.info` | Scenario name/version (`ClusterBench`) | written for this benchmark |
| `settings.bb8settings` | Minimal one-zone ecology: no initial plant/meat seeding, no zone groups, one spawn row (`cluster.bb8template`, minimum 1, adult, no gene randomisation) | derived from `crates/bibites-io/tests/fixtures/settings/scenario_settings.bb8settings`, then stripped to one zone / zero fertility / zero biomass / empty zone groups |
| `cluster.bb8template` | Deterministic genome/brain template for the crowd | copy of `crates/bibites-io/tests/fixtures/living.bb8` (tracked io fixture) |

`../../make_cluster_world.py` builds a scenario zip from these files, runs it for
one tick with the benchmark seed (`--templates` pointed at an empty directory so
the user's installed game templates cannot leak in), saves the resulting
1-bibite world, and clones that bibite N times onto a deterministic jittered
lattice inside a `box × box` square. The save is cached under
`target/bench-worlds/cluster/` (gitignored) with metadata including a
`content_sha256`; the cache is invalidated when these files, the N/box/seed or
the engine binary SHA-256 change.

Do not edit generated worlds or add them here. To change the ecology/body plan,
edit these inputs (the cache key changes automatically) and record the new
measurements in [`docs/BENCHMARKS.md`](../../../../docs/BENCHMARKS.md).

The whole-tick numbers from this world use the `cluster.bb8template` genome and a
minimal ecology, so they measure physics/body dominated pile-up. The canonical
physics-only optimisation metric (starter genome, `BibiteStore::step_physics`) is
the `body::perf_probe_clustered_and_spread` audit, which `run_cluster.ps1
-CoreProbe` also records.
