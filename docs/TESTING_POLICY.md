# Testing policy — canonical worlds

**The canonical test world is the game's default scenario: `testdata/real/Default.zip`.**
All validation, parity baselines, ecology experiments, viewer screenshots, docs examples
and performance scenario tests use it unless a task explicitly justifies another fixture.

`testdata/` is local-only (gitignored game data); tests and benchmarks must skip cleanly
when it is absent (CI).

## Allowed worlds

| Purpose | World |
|---|---|
| Default scenario / new game behaviour | **`testdata/real/Default.zip`** (the game's Default scenario: 1 zone, 1 spawn row, 1 bibite start, simSize 2000, simTPS 30) |
| Synthetic throughput benchmarks | generated default-settings world (`World::new(SimSettings::game_defaults(), ScenarioData::with_default_zone_group())` + spawned starter bibites, density-scaled for large N) |
| Historical/regression references | `testdata/real/saves/lol.zip`, `testdata/real/Ring.zip`, `testdata/real/NestEgg.zip`, `RadTower.zip` — only for explicit fixture regressions (documented in the task), never as the default baseline |

## Rules

1. **Default first.** New tests, docs, screenshots and capture commands start from
   `Default.zip`. Do not use `lol.zip` or arbitrary saves as the primary test world.
2. **Historical fixtures are opt-in.** If a task must use a special save/scenario (e.g. an
   evolved-brain regression, a specific zone layout, a tower), say why in the task and keep
   the Default run as the baseline alongside it.
3. **Game-side captures** (`tools/parity`, `tools/roundtrip`) use `Default.zip` for the
   fresh-scenario baseline. Hidden-window only (see `HEADLESS_POLICY.md`), no leftovers.
4. **Docs**: README/user-guide/install/viewer/ecology/parity/benchmark examples should show
   `Default.zip` commands. When quoting historical numbers from `lol`/`Ring`, label the world.
5. **Benchmarks**: `bench` synthetic worlds are fine; `bench --scenario` examples use
   `Default.zip`. Record the world name with every number.
6. **CI**: tests depending on `testdata/` must skip with a clear message when it is absent
   (CI has no game data).

## Reference (as of 2026-10-30)

`rustibites info testdata/real/Default.zip` → scenario `Default`, version 0.6,
simTPS 30, simSize 2000, 1 zone, 1 spawn row, 0 bundled templates.

`rustibites run testdata/real/Default.zip --ticks 600 --seed 12345` →
1 → 10 living (1 initial + 9 virgin-spawn births), energy mean 156.9 → 115.41,
plant pellets 440 → 447, no deaths in the first 600 ticks.

> **Emergent counts:** plant pellet count is not a fixed target. After the body task-106
> contact/physics fixes the same 600-tick run reports **447** pellets (previously 448) and final
> energy mean **115.41**; the count can shift by ±1 when physics/feeding timing changes while the
> population/birth/death outcome stays identical. The body CLI regression was updated to the current
> values; tests should assert the behaviour/envelope, not the exact emergent pellet count.

## Migration checklist (per owner)

- [ ] Replace `lol.zip` / `Ring.zip` primary commands in tests/docs/screenshots with `Default.zip`
- [ ] Keep any historical fixture only with a documented reason
- [ ] Game-side captures refreshed on `Default.zip`
- [ ] Update quotes/tables to name the world next to each number
