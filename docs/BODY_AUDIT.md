# Body organ formula audit (fat/armor/egg/color)

Line-by-line check against `Here/SimulationScripts` and spec §5, 2026-11. Only deviations that are
measurable or can change outcomes are listed; tiny floating-boundary differences are documented as
residual.

| organ | game source | Rust | verdict |
|---|---|---|---|
| Fat capacity/inflate | `BibiteFatOrgan.OnGrowth`: `baseCapacity=area·portion`, `maxInflate=0.35·area`, `area=baseCapacity·min(2,1+0.35/portion)` | `FatOrgan::on_growth` same | match |
| Fat threshold/deadband activation | ratio outside `[threshold±deadband]`, activation inverse-lerp, `1−fullness^10` | `FatOrgan::update` same | match |
| Fat amount/energy | `Δ=activation·d2Size·metabolicRate·(reactivity·dt)²·π`, clamp to available/amount; store pays `−Δ·30/eff`, burn gains `−Δ·30·eff` | same (`last_affinity=1−sqrt|a|`, conversion efficiency) | match |
| Fat full/empty guard | `Matter.isFull/isEmpty` = `Mathf.Approximately` (relative ≈1e-6) | exact `amount >= max_amount` / `<=0` | **residual**: only at floating boundary; no observable capture delta, no change |
| Armor thickness/strength | `5·d1Size·(1−sqrt(1−portion/(1+inflate)))`, strength=cohesiveness·thickness, body uses ×2 | same | match |
| Armor mass | `area·massDensity` | same | match |
| Egg layer production | threshold remap `|prod|` 0.15→1, `Δ=metabolicRate·remap·dt/LayTime`, clamp capacity/progress, reabsorb at 0.5, lay while progress≥1 and Want2Lay≥0.25 | `EggLayer::update`, `lay_eggs` count loop | match |
| Egg energy/capacity | `eggEnergy=LayBodyEnergy+upkeep·SizeRatio`, `capacity=floor(wombArea/areaAtBirth)` | same | match |
| Egg hatching | `StartHatch` abort if `hatchTime<=0` or `required>2·energy`; progress/hatch/abort at exact boundaries; free energy shows missing energy | `Egg::start_hatch/tick`, exact boundary fixture | match |
| Egg saved-state organ caps | game reload recomputes organ areas from saved body size | **fixed in `44fd370`**: `spawn_loaded` now refreshes organs/colliders/derived after `set_2d_size`; before fix fat/womb/armor used birth-size caps (fixture fat amount 30.9 clamped to 10.3) | fixed |
| Color | color genes → body color; color killers apply world-side | `record.color = [ColorR,G,B]` | match; color-killer terms world-owned |
| Bite damage | `factor·maxHealth·chunk/realArea`; armor blocks at bite≤armor; ongoing `dt/(5·bitePeriod)` | `resolve_bite_damage` + tests | match |

Synthetic fixtures proving these paths: `crates/bibites-cli/tests/fixtures/body/`; invariants in
`crates/bibites-cli/tests/body_fixtures.rs`; generator `crates/bibites-cli/examples/make_body_fixtures.rs`.
