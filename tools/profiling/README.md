# RUSTibites profiling harness (`tools/profiling`)

Repeatable profiling workflows for the headless engine, plus a machine check for
which profilers are actually available. Everything here is no-install and
read-only with respect to `crates/`: instrumentation changes are crate-owner
work, and this directory only runs the existing audits, builds with
`cargo --config` overrides, and wraps OS capture tools.

The theory and interpretation live in [`docs/PROFILING.md`](../../docs/PROFILING.md);
this file is the command reference.

```
tools/profiling/
  profile.ps1        one entry point for all actions
  results/           generated artifacts (checked in; regenerable)
    doctor.txt/.json
    phase_audit.log/.meta.json
    micro_probes_*.log + .meta.json
    bench-phases/    phases.csv/md/raw from `rustibites bench` human output
    symbol-build.json
```

## Quick start

```powershell
cd Z:\RUSTibites

# What works on THIS machine? (wpr/tracerpt/WPA/samply/VS/... + binary symbol status)
powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1

# Tier 2: the ignored perf_* audits in bibites-core (phase tables, sizes, sensing, physics probes).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action phase-audit

# Tier 2b: per-subsystem probes only (body broadphase, sensing breakdown, executor overhead).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action micro-probes

# Tier 1: capture the always-on phase tables from `rustibites bench` (delegates to tools/bench).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action bench-phases `
    -Pops 1000,2000,5000,10000,20000 -Threads 1,8 -Ticks 120

# Release build with debug symbols, without editing root Cargo.toml (~2-4 min LTO build,
# isolated target/profiling dir so target/release is untouched).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action build-symbols

# Elevated shell only: capture a CPU ETW trace around a bench run (WPR needs admin).
powershell -NoProfile -ExecutionPolicy Bypass -File tools\profiling\profile.ps1 -Action etw-capture `
    -Pops 2000 -Ticks 400
```

On PowerShell 7 use `pwsh -File ...`. All actions write to
`tools/profiling/results/` unless `-OutputDir` is given.

## Actions

| Action | What it does | Artifacts |
|---|---|---|
| `doctor` (default) | Probes the machine for `wpr`, `wpa`, `xperf`, `tracerpt`, `samply`, `perf`, `perfview`, `procdump`, `cargo-flamegraph`, VS/WPT installs; records the release binary SHA-256, PDB presence and the `strip=debuginfo` fact; prints what is available now. | `doctor.txt`, `doctor.json` |
| `phase-audit` | `cargo test --release -p bibites-core --lib -- --ignored --nocapture --test-threads=1 perf_` — runs `perf_phase_audit`, `perf_scale_sizes`, `perf_sensing_tick_profile`, `perf_sensing_breakdown`, `perf_probe_clustered_and_spread`, `perf_probe_world_phase_table`, `perf_parallel_executor_overhead`. | `phase_audit.log`, `.meta.json` |
| `micro-probes` | Same mechanism, narrower filters: `perf_probe_` (body), `perf_parallel_executor_overhead` (sim), `perf_sensing_` (sensing). | `micro_probes_body.log`, `micro_probes_executor.log`, `micro_probes_sensing.log` + metadata |
| `bench-phases` | Delegates to `tools/bench/run_phases.ps1`, which parses the `bench` human phase tables into CSV/Markdown. | `bench-phases/phases.csv`, `phases.md`, `phases_raw.txt`, `environment.*` |
| `build-symbols` | `cargo build --release -p bibites-cli --target-dir target/profiling --config profile.release.debug=true --config profile.release.strip=false`. No `Cargo.toml` edits; isolated target dir. | `symbol-build.json` with exe/PDB paths + SHA-256 |
| `etw-capture` | Windows Performance Recorder CPU trace around one `bench` run, then `tracerpt` CSV conversion. Requires an elevated shell; exits 3 with instructions when not elevated. | `.etl`, `.csv`, `etw-capture.json` (or `etw-capture-blocked.txt`) |

Common flags: `-OutputDir`, `-Exe`, `-Build` (only for `bench-phases` binary
resolution), `-MaxLoadPct`/`-LoadWaitSec` (pass-through to the bench harness),
`-TimeoutSec`, `-TestTimeoutSec`, `-Quiet`. `build-symbols` takes `-TargetDir`,
`-Force`; `etw-capture` takes `-WprProfile` (default `CPU`), `-CaptureSeconds`.

Exit codes: `0` success, `3` ETW capture unavailable because the shell is not
elevated (the message is also written to `etw-capture-blocked.txt`), other
non-zero values are cargo/test/capture failures.

## What is available on the reference machine

`profile.ps1 -Action doctor` on the reference machine (2026-10-30) reports:

| Tool | Status | Consequence |
|---|---|---|
| `wpr` | present | ETW CPU capture is possible, but **only from an elevated shell** (kernel session; non-elevated gives `0xc5585011`). |
| `tracerpt` | present | Converts `.etl` to flat CSV/XML for grep-level inspection. |
| `wpa`, `xperf` | missing | No symbols/stacks view for `.etl` here; requires Windows Performance Toolkit installation (approval needed). |
| `samply`, `perf`, `cargo-flamegraph`, `perfview`, `procdump`, `dotnet-trace` | missing | No flamegraph/sampling tool installed. |
| Visual Studio | BuildTools only | No IDE profiling tools (`devenv`/diagnostics hub absent). |

Because no usable sampling profiler is installed in this environment, the
supported workflow is **phase timings + ignored audits + rebuild-with-symbols +
optional WPR capture on an elevated/other machine**. `docs/PROFILING.md` lists
the install options that need Lead approval.

## Notes and limitations

- `cargo test --release` compiles a release test binary on first use (one-time,
  ~1-2 min); subsequent audits are seconds. The script reports the compile+run
  duration in the `.meta.json`.
- `build-symbols` uses a separate `target/profiling` target dir (no rebuild of
  the normal `target/release`, no root manifest edits) but costs an LTO build.
  It records the resulting exe/PDB path and SHA-256.
- The default release CLI is built with `strip = "debuginfo"`, so its PDB is not
  symbol-friendly for profiling; use `build-symbols`.
- These runs are seconds-to-a-minute; do not use them for 100k-tick profiling on
  a shared machine. `bench-phases` accepts the same `-Pops/-Threads/-Ticks`
  bounds as `tools/bench`.
