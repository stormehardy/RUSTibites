# Continuous integration and local gates

RUSTibites has no game or `testdata/` dependency in CI: real-game fixtures are
gitignored (not redistributable), and tests that need them skip when the files
are absent. Everything else (unit/integration tests, clippy, release build) runs
on stock stable Rust.

## What GitHub Actions runs

`.github/workflows/ci.yml` runs the same gate on **windows-latest** and
**ubuntu-latest** for pushes to `main`, pull requests and manual dispatches:

| Step | Command |
|---|---|
| Formatting | `cargo fmt --all -- --check` |
| Lints | `cargo clippy --locked --workspace --all-targets -- -D warnings` |
| Tests | `cargo test --locked --workspace` |
| Release build | `cargo build --locked --release` |

The workflow also installs the Linux libraries that the viewer crate's
`macroquad`/`miniquad` dependency links against (`libx11-dev`, `libxi-dev`,
`libgl1-mesa-dev`, `libasound2-dev`). The viewer's tests render **headlessly**
through the software rasteriser and do not open a window or need a display, so
they run on the ubuntu runner like any other unit test.

Cargo registry/git and `target/` are cached by `actions/cache`, keyed on
`Cargo.lock`. Jobs have a 60-minute timeout (the release profile uses fat LTO,
which is the slow part on Windows). The workflow installs stable Rust with
`rustfmt` and `clippy`; it does not install the game or Unity tooling. `--locked`
makes CI fail if `Cargo.lock` is out of date instead of silently resolving new
dependency versions.

`testdata/` is intentionally absent on CI. Tests that read real saves,
templates or parity captures are written as skip-if-absent (for example the Ring
parity fixture tests and the `bundle_i3` real-file round-trips). If you add a
test that hard-requires `testdata/`, guard it with an explicit early return when
the path does not exist, otherwise CI will fail for contributors without the
game assets.

`.github/workflows/release.yml` is an optional tag workflow (`v*`, plus manual
dispatch). It builds both release binaries (`cargo build --locked --release
--bins`) and calls the repository packaging script
[`tools/release/package.ps1`](../tools/release/package.ps1) with `-SkipBuild
-Version <tag>`, so the portable-folder layout, docs/licenses/assets and
`SHA256SUMS.txt` stay in one place. The workflow uploads the generated
`RUSTibites-<version>-windows-x64.zip` and `SHA256SUMS.txt`; no packaging logic
is duplicated in YAML.

## Reproduce CI locally

Quick three-command gate (kept for compatibility):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\check.ps1
```

Full local gate with PASS/FAIL summary, timings and an optional release bench
smoke:

```powershell
# Windows PowerShell 5.1
powershell -ExecutionPolicy Bypass -File scripts\ci\check.ps1

# PowerShell 7+ (Windows/macOS/Linux)
pwsh -File scripts/ci/check.ps1
```

The script preflights `cargo`, the workspace root (`Cargo.toml`) and the
`rustfmt`/`clippy` toolchain components, so a fresh machine gets
`rustup component add rustfmt clippy` instead of an obscure cargo error. On a
Linux machine, install the viewer's build libraries first (see
[Troubleshooting](#troubleshooting)).

Options:

```text
-Bench                 also run a release `rustibites bench` smoke test
-BenchBibites <N>      bench population (default 2000)
-BenchTicks <N>        bench ticks (default 300)
-SkipTests             fmt + clippy only (useful during large refactors)
```

The script exits `0` only when every selected step passes and prints a summary
such as `RESULT: PASS (3 steps)`. On failure it continues through the remaining
steps (so one run reports all problems) and exits `1`.

The equivalent raw commands are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
# optional benchmark (release):
cargo run --release --bin rustibites -- bench --bibites 2000 --ticks 300 --threads 1
```

Add `--locked` to the cargo commands to reproduce CI exactly (it requires
`Cargo.lock` to be up to date, which is what CI enforces).

## Game-dependent tests

- `testdata/` (real saves, templates, Ring base, parity captures) is gitignored
  and absent on CI. Tests under `crates/bibites-io/tests/` and the Ring parity
  tests in `crates/bibites-cli/src/capture.rs` skip when those files are missing.
- To run them locally, provide the fixtures under `testdata/` as described in
  `README.md` / `docs/PARITY.md`.
- Never commit game assets. CI must stay green without them.

## Troubleshooting

- **Linux viewer link errors** (`cannot find -lX11`, `-lGL`, `-lasound`, missing
  `x11` headers): install the viewer build dependencies before running the gate
  or building:
  ```bash
  sudo apt-get update
  sudo apt-get install -y --no-install-recommends \
      libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev
  ```
  The viewer's tests themselves are headless (software rasteriser), so no X
  server/display is required.
- **Windows long paths**: the workspace path plus cargo target names can exceed
  `MAX_PATH` on older Windows setups. Enable long paths
  (`git config --system core.longpaths true` and the LongPathsEnabled registry
  policy) or build from a short path such as `C:\src\rustibites`.
- **Slow release builds**: `[profile.release]` enables fat LTO and one codegen
  unit; CI caches `target/`, but the first run is slow. Use `-SkipTests`
  locally while iterating, and leave the full gate for pre-push. The cached
  `target/` on a Windows runner is large; if cache uploads get slow, scope the
  cache to `~/.cargo/registry` and let the build cache stay cold.
- **Stale cache / link errors**: run `cargo clean` and retry; CI cache keys
  include `Cargo.lock`, so dependency changes invalidate the cache.
- **`--locked` failures**: `Cargo.lock` is out of date with a manifest change.
  Run `cargo check`/`cargo build` locally without `--locked`, commit the
  updated `Cargo.lock`, and re-run the gate.
- **`cargo run` is ambiguous**: there are multiple binaries
  (`rustibites`, `rustibites-view`). CI and the gate script use
  `--bin rustibites` where the binary matters.
- **`fmt` or `clippy` fail on another agent's in-flight file**: re-run after
  their edit lands; the gate reports the exact file/line.
- **Gate says rustfmt/clippy missing**: `rustup component add rustfmt clippy`.
