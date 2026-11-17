# Zone fill colour — formula and fitted ramp

**Status:** design / derivation (docs-only, task-113). **Owner:** spec-dev. **Consumers:** view-dev
(renderer), bench-dev (scenario screenshots/analysis), parity-dev (extra reference captures if wanted).
**Sources:** `Here/SimulationScripts/Zone.cs` (`UpdateMaterial`, visibility), `Here/SettingScripts/ZoneSettings.cs`
(`fertility`, `biomassDensity`, `distribution` ranges/defaults), `Here/UIScripts/UserSettings.cs` +
`UserSettingsManager.cs` (`FertilityColor`, `ShowFertility` global), material/shader dump
(`assets/game/materials/PelletSpawnPointMaterial.json`, prefab dump `zoneMaterial =
PelletSpawnPointMaterial`, shader `Shader Graphs/PelletSpawnPoint`), captured reference frames
(`assets/game/reference/{default,default_save,population}.png`), and a 24-scenario hidden-window
colour sweep run for this task (`target/zone-samples/`, scripts under `target/zone_*.py`).

**Confidence tags:** `[C]` code-exact; `[F]` fitted from captures; `[P]` proposed follow-up; `[?]` open.

---

## 0. TL;DR

The zone is a single `SpriteRenderer` using the transparent `PelletSpawnPointMaterial` shader. Its fill is
**not** a function of current pellet biomass or time. It is:

```
opacity   = (f(fertility) + f(zone_biomassDensity)) / 2          [C]  Zone.UpdateMaterial()
f(v)      = 1 - 0.9 * (clamp(invLog(v, 0.001, 100), 0, 1) - 1)^2
invLog(v) = (log10(v) - log10(0.001)) / (log10(100) - log10(0.001))
k         = 0.5  if distribution in {Flat, FlatRing, Rect}       [F]
            1.0  otherwise (Ring, CentricGradual, ExteriorGradual)
base_srgb = user FertilityColor (measured here: (231.1, 154.5, 0)) [?/F]
fill      = base_srgb * k * opacity                              [F]
```

* **Golden vs brown is the distribution, not biomass/fertility/material.** Flat/FlatRing/Rect zones render
  at half brightness (brown); Ring/CentricGradual/ExteriorGradual render golden. Default's zone omits
  `distribution`, whose default is **CentricGradual** (`ZoneSettings.cs`) → golden. `lol.zip`'s zone is
  `Flat` → brown `(107,72,0)`.
* **Fertility and zone biomass density only scale opacity.** A 17-point sweep reproduces the observed fill
  to ≤0.5/255 per channel.
* **Material (Plant vs Meat) does not change the fill colour** (verified at identical settings: both
  `(98,66,0)`); it only changes pellet sprites/outlines elsewhere.
* **Current biomass (`pelletBiomass/maxBiomass`) and time do not affect the fill.** The game's
  `Zone.UpdateMaterial()` never reads `pelletBiomass`; `default` (t≈0.25 s) and `default_save` (t≈20 s,
  11 bibites) reference frames have identical zone-fill colours (same dominant `(196,131,0)` etc.; only
  entities differ, ~17 k of 2.07 M pixels). **This corrects the earlier hypothesis** — the "darker/browner
  at t=20 s" came from the viewer's own biomass-dependent alpha ramp, not from the game.
* The zone sprite is only drawn while `UserSettings.ShowFertility` is true (`Zone.ToggleVisibility`;
  default true). With it off there is no fill.

Expected values: Default newgame/save (fertility 0.7827918, zone biomass density 1, CentricGradual) →
`(196,131,0)` (task-clue measurement `(194,130,0)`, 1–2/255 AA/pattern difference). `lol.zip` zone
(fertility 100, density 1, Flat) → `(107,72,0)` (measured exactly).

---

## 1. Code path

`Zone.cs`:

* `public SpriteRenderer sr; public Material zoneMaterial;` (lines 24/26); at init the zone does
  `mat = Object.Instantiate<Material>(zoneMaterial); sr.material = mat;` (152–153) — **per-zone material
  clone**, no per-frame colour assignment.
* `UpdateMaterial()` (291–305, copied verbatim in the shader dump):
  * `_Ring`, `_Rect`, `_insideRadius` set from `settings`;
  * `_Opacity` = average of two log-scaled fertility/biomass-density terms (formula above);
  * `_PelletSpawnCoefficient` = 0 for `Flat`/`FlatRing`/`Rect`, −1 for `ExteriorGradual`, +1 otherwise
    (`Ring`/`CentricGradual`).
* `_Opacity`/`_PelletSpawnCoefficient` are **settings-derived constants**; subscriptions call
  `UpdateMaterial` when settings change (`UpdateBiomass` 279–283, `UpdateFertility` 285–289), not when
  pellets are eaten/spawned. `RecountBiomass` (435+) updates `pelletBiomass` only.
* `ToggleVisibility` (`Zone.cs` ~473): `sr.enabled = UserSettings.ShowFertility.val`.

Material/shader dump (`assets/game/materials/PelletSpawnPointMaterial.json`, prefab dump):
`shader = "Shader Graphs/PelletSpawnPoint"`, floats `_Opacity=0.5`, `_PelletSpawnCoefficient=1.1378`,
`_Rect/_Ring=0`, `_insideRadius=0.818`; no serialized colours. The runtime shader's uniform list includes
**`_fertilityColor`** (global set from `UserSettings.FertilityColor` by `UserSettingsManager`). The shader
is a Shader Graph; its exact HLSL was not recovered (compiled DXBC only), so the palette mapping below is
fitted from captures.

Input defaults/ranges (`ZoneSettings.cs`, `ScenarioIndependentSettings.cs`):

| input | default | range | use |
|---|---|---|---|
| `fertility` | 1.0 | 0.001 … 100 | opacity term |
| zone `biomassDensity` | 1.0 | 0.001 … 100 | opacity term |
| `distribution` | **CentricGradual** | enum | `_PelletSpawnCoefficient` / palette factor |
| `insideRadius` | 0.9 | 0…1 | shape/pattern only |
| `material` | Plant | Plant/Meat/None | no effect on fill colour (verified) |
| `UserSettings.ShowFertility` | true | bool | sprite enabled |
| `UserSettings.FertilityColor` | code default `(11,128,0)` | colour picker | likely the base fill colour `[?]` |

---

## 2. Formula

Given a zone's effective settings (use serialized values; missing fields take the defaults above):

```text
LOG_MIN = log10(0.001) = -3
LOG_MAX = log10(100)   =  2

inv_log(v) = clamp((log10(clamp(v, 0.001, 100)) - LOG_MIN) / (LOG_MAX - LOG_MIN), 0, 1)
f(v)       = 1 - 0.9 * (inv_log(v) - 1)^2                  # 0.1 at min, 1.0 at max

opacity    = (f(fertility) + f(zone_biomass_density)) / 2  # 0.1 … 1.0

coefficient =  0  for distribution in {Flat, FlatRing, Rect}
             = -1  for distribution == ExteriorGradual
             =  1  otherwise   # Ring, CentricGradual
palette_k   = 0.5 if coefficient == 0 else 1.0             # fitted (0/±1 are the only enum values)

base_srgb   = user_fertility_color                          # measured (231.1,154.5,0) on this profile
alpha       = opacity * shader_shape_pattern                # _Opacity is the shader alpha
appearance_over_black = round(base_srgb * palette_k * alpha) # measured reference values
```

`_Opacity` is **straight alpha**, not an RGB multiplier. Evidence: the serialized
`PelletSpawnPointMaterial` / `Shader Graphs/PelletSpawnPoint` pass is Transparent queue, ZWrite off,
Blend `SrcAlpha` / `OneMinusSrcAlpha` (srcBlend=5, dstBlend=10, Add; UnityPy shader-state dump in
`target/zone_shader_state.txt`, shader PathID 80 in `sharedassets1.assets`); the zone SpriteRenderer
colour is white/opaque in `assets/game/prefabs.json`. The over-black measurement is therefore
`base*opacity`, but over a non-black background the game composites
`base*alpha + background*(1-alpha)`. The reference centre pixel in
`assets/game/reference/default/population.png` is `(211,147,16)`; with Default alpha 0.8481 and the
102-gray grid under the zone that is `(231.1,154.5,0)*0.8481 + 102*0.1519 = (211.5,146.5,15.5)`,
whereas a premultiplied RGB shader would give ~181. Over black the two are identical, which is why an
alpha-255 presentation still matched the black-background reference frames. Draw straight-alpha
`RGB = base_srgb * palette_k`, `A = opacity` so grid/background show through as in the game. This is
**sRGB/display space** (fit directly to screenshot pixels). Do not linearise for the fit.

### Validation data (measured, hidden-window game screenshots)

Sweep at distribution `Flat` (coefficient 0), camera `fit=true` 960×540, `base_brown = (115.5,77.7,0)`:

| variant | fertility | zone density | opacity | observed fill | predicted (`base_brown*opacity`) | err |
|---|---|---|---|---|---|---|
| d0.001 | 0.7827918 | 0.001 | 0.4701 | (54,36,0) | (54.3,36.5,0) | −0.3,−0.5 |
| d0.01 | 0.7827918 | 0.01 | 0.6321 | (73,49,0) | (73.0,49.1,0) | 0.0,−0.1 |
| d0.1 | 0.7827918 | 0.1 | 0.7581 | (88,59,0) | (87.6,58.9,0) | +0.4,+0.1 |
| d1 | 0.7827918 | 1 | 0.8481 | (98,66,0) | (97.9,65.9,0) | +0.1,+0.1 |
| d3 | 0.7827918 | 3 | 0.8784 | (101,68,0) | (101.4,68.2,0) | −0.4,−0.2 |
| d10 | 0.7827918 | 10 | 0.9021 | (104,70,0) | (104.2,70.1,0) | −0.2,−0.1 |
| d30 | 0.7827918 | 30 | 0.9152 | (106,71,0) | (105.7,71.1,0) | +0.3,−0.1 |
| d100 | 0.7827918 | 100 | 0.9201 | (106,71,0) | (106.3,71.5,0) | −0.3,−0.5 |
| f0.001 | 0.001 | 1 | 0.4780 | (55,37,0) | (55.2,37.1,0) | −0.2,−0.1 |
| f0.01 | 0.01 | 1 | 0.6400 | (74,50,0) | (73.9,49.7,0) | +0.1,+0.3 |
| f0.1 | 0.1 | 1 | 0.7660 | (89,60,0) | (88.5,59.5,0) | +0.5,+0.5 |
| f1 | 1 | 1 | 0.8560 | (99,66,0) | (98.9,66.5,0) | +0.1,−0.5 |
| f3 | 3 | 1 | 0.8863 | (102,69,0) | (102.4,68.8,0) | −0.4,+0.2 |
| f10 | 10 | 1 | 0.9100 | (105,71,0) | (105.1,70.7,0) | −0.1,+0.3 |
| f30 | 30 | 1 | 0.9231 | (107,72,0) | (106.6,71.7,0) | +0.4,+0.3 |
| f100 | 100 | 1 | 0.9280 | (107,72,0) | (107.2,72.1,0) | −0.2,−0.1 |

`FlatRing` and `Rect` at the same settings also measured `(98,66,0)` (same coefficient 0). Meat material at
the same settings measured `(98,66,0)` — identical. Golden samples: `Ring` (fert 0.7828, density 1,
opacity 0.8481) → `(196,131,0)`; `CentricGradual`/`ExteriorGradual` full-world fit frame gave a lower
dominant `(188,126,0)` (pattern/edge sampling; the canonical Default `population` frame is `(196,131,0)`).
Reference `default/population.png` and `default_save/population.png`: dominant `(196,131,0)`,
`(195,131,0)`, `(194,130,0)`; same across newgame and t=20 s. `lol.zip` reference `population.png`:
dominant `(107,72,0)` (Flat + fertility 100/density 1 → opacity 0.928, base brown → 107.2,72.1).

The fitted golden base from the reference frames is `196/0.8481 = 231.1`, `131/0.8481 = 154.5`, i.e.
**base_srgb ≈ (231.1, 154.5, 0)**, and brown = exactly that × 0.5. The task's measured `(194,130,0)` is the
same formula within 1–2/255 (AA/dither/pattern). This base is **not** the code default FertilityColor
`(11,128,0)` — either the current lab profile carries a custom golden FertilityColor, or the shader derives
the fill from a fixed gradient. Marked `[?]`; extra capture in §5 will settle it.

---

## 3. Viewer pseudocode

```rust
// Per zone, computed once per settings change (not per frame / not per biomass change).
fn zone_fill(zone: &Zone, user: &UserSettings) -> Option<Rgba> {
    if !user.show_fertility { return None; }                  // sr.enabled = ShowFertility

    let fertility = zone.fertility.unwrap_or(1.0);            // ZoneSettings defaults
    let density   = zone.biomass_density.unwrap_or(1.0);

    let inv = |v: f32| {
        let v = v.clamp(0.001, 100.0);
        ((v.log10() + 3.0) / 5.0).clamp(0.0, 1.0)
    };
    let f = |v: f32| 1.0 - 0.9 * (inv(v) - 1.0).powi(2);
    let opacity = (f(fertility) + f(density)) * 0.5;

    let coefficient = match zone.distribution {
        Flat | FlatRing | Rect => 0.0,
        ExteriorGradual => -1.0,
        Ring | CentricGradual => 1.0,
    };
    let k = if coefficient == 0.0 { 0.5 } else { 1.0 };

    // base: use the user's FertilityColor when available; the measured canonical value here is
    // (231.1,154.5,0). Code default FertilityColor is (11,128,0) green.
    let base = user.fertility_color;                          // sRGB 0..255
    Some(Rgba::new(
        (base.r as f32 * k).round() as u8,                    // straight alpha: no RGB pre-multiply
        (base.g as f32 * k).round() as u8,
        (base.b as f32 * k).round() as u8,
        (opacity * 255.0).round() as u8,                      // _Opacity is the shader alpha
    ))
}
```

**Viewer status (implemented in `8104046`, view-dev):** `crates/bibites-view/src/render.rs`
implements the opacity/palette formula and now draws straight-alpha `RGB = base*k, A = opacity*255`
(zone colour fix landed). Headless measurements: Default save `(195,130,0)` over black vs reference
`(195,131,0)`; `lol.zip` Flat `(107,71,0)` vs expected `(107,72,0)`; the grid shows through the zone as
in the game. The `(231,154,0)` base/fit work is done; remaining optional `UserSettings.FertilityColor`
sourcing is unchanged.

---

## 4. Edge cases

| case | behaviour |
|---|---|
| Missing `distribution` | Game default **CentricGradual** → golden, k=1. Do not default to Flat (that produces brown and is the likely cause of the earlier mismatch). |
| Missing `fertility` / `biomassDensity` | Default 1.0 each. |
| fertility/density at min 0.001 | f=0.1 → opacity 0.1 (near-black fill; Flat also ×0.5). |
| at max 100 | f=1 → opacity 1 (Flat stays ×0.5). |
| values outside 0.001…100 | Clamp before log (negative/zero would be invalid; game settings clamp). |
| `Flat`, `FlatRing`, `Rect` | coefficient 0 → half-brightness brown palette. |
| `Ring`, `CentricGradual`, `ExteriorGradual` | coefficient ±1 → full golden palette. |
| `ExteriorGradual` | Same palette as Ring/Centric; only pellet spawn pattern differs. |
| Meat/None material | Fill colour unchanged (verified Plant vs Meat); pellet sprites change. `None` zones may be hidden depending on game logic. |
| `ShowFertility = false` | No fill (sprite disabled). |
| Over-cap `pelletBiomass > maxBiomass`, recycling, decay | No fill change (opacity settings-only). |
| Time / population growth | No fill change; only pellet/bibite sprites differ. |
| Resolution/zoom | Shader draws a non-uniform pattern (dominant colours vary e.g. 188–211); a uniform fill with the formula matches the dominant value within ~1–2/255 at reference zoom, ±4 % at whole-world zoom. |
| User changes FertilityColor | Expected to change the base (global `_fertilityColor`); needs a capture to confirm (see §5). |

---

## 5. What would tighten this — extra captures `[P]`

Route through parity-dev's hidden-window harness (`tools/parity/capture.py`/lab), remove the transient
plugin in the same turn; do not reuse visible windows. One session, multiple `new_game`/`load` calls is
enough.

1. **FertilityColor sweep (key unknown):** set `UserSettings.FertilityColor` to several known colours
   (e.g. code default `(11,128,0)`, pure red, white) in the lab profile (or the user profile with explicit
   consent), capture Default newgame, sample the fill. Confirms whether `base = _fertilityColor` or a
   Shader-Graph constant. Until then use the measured golden base for canonical screenshots and mark
   medium confidence.
2. **Coefficient/palette matrix at a fixed 1920×1080 population camera** (`distribution` = Flat, FlatRing,
   Rect, Ring, CentricGradual, ExteriorGradual; same fertility/density) to confirm `k=0.5/1.0` and quantify
   the pattern modulation per distribution.
3. **Fertility/density sweep at CentricGradual/Ring** (same as §2 but non-Flat) to validate the golden ramp
   independently of the reference frames.
4. **Material/None/ShowFertility:** Plant vs Meat vs None zone; `ShowFertility` on/off; capture with the
   user setting toggled. The viewer currently has no ShowFertility wiring and always draws the fill; the
   canonical references all use the default (true), so a `ShowFertility=false` reference is only needed if
   we want that toggle to match the game.
5. **Time invariance:** close-up zone frames at t=0.25 s, 20 s, 120 s on the same Default world, plus an
   over-cap/recycle state — confirms no biomass/time term (we already have newgame vs save full frames;
   close-ups would be cleaner).
6. **Pattern/UV:** the Shader Graph output is not uniform (radial bands/dither). If pixel-exact zone art is
   wanted, extract the Shader Graph HLSL or fit a 2-D pattern from high-res frames per distribution; the
   uniform formula here is the colour contract, not the pattern.

The earlier 24-variant sweep scripts are scratch artifacts under `target/`:
`target/zone_capture_matrix.py`, `target/zone-variants/`, `target/zone-samples/` (not committed). The run
reused the hidden-window lab and stopped its own game; no listener/plugin was left. A second matrix attempt
was blocked by another game instance already running; it did not launch anything.

---

## 6. Answers to the task report

* **Formula/ramp:** opacity from fertility + zone biomass density (code-exact, log InverseLerp quadratic);
  palette factor 0.5 for Flat/FlatRing/Rect vs 1.0 otherwise (fitted); base colour measured
  `(231.1,154.5,0)` sRGB; fill = base × factor × opacity.
* **Expected colours at known states:**
  * Default newgame and t=20 s save: `(196,131,0)` / `(195,131,0)`; task reference `(194,130,0)` —
    residual 1–2/255.
  * `lol.zip` zone: `(107,72,0)` — formula `(107.2,72.1)`, residual <1/255.
  * Flat Default variant: `(98,66,0)`; fertility/density sweep residual ≤0.5/255 (17 samples).
  * Meat material identical to Plant at the same settings.
* **Residual error:** ≤0.5/255 on the 17-point opacity calibration; 1–2/255 vs the reference golden
  measurement; ~4 % (7–8/255) when the whole-world fit frame samples the distribution edge pattern instead
  of the population zoom (pattern modulation, not a ramp error).
* **Key correction:** zone fill does **not** depend on current biomass or time; the "darker/browner at
  t=20 s" in the save-based comparison is the viewer's biomass-dependent alpha (`render.rs`) and constant
  base, not the game. Brown vs golden in the game is `lol`'s `Flat` distribution vs Default's
  `CentricGradual` default.
* **Gaps:** whether the base colour is the user `FertilityColor` or a Shader-Graph constant; exact
  Shader Graph pattern; golden ramp at non-Flat distribution (2 samples) and non-default FertilityColor;
  meat/None/ShowFertility combinations. Extra captures in §5 close them.

---

## 7. Implementation status (view-dev, commit `8104046`)

The viewer implements the colour contract (`crates/bibites-view/src/{render,assets}.rs`): the old
`pellet_biomass/max_biomass` alpha ramp is gone; `opacity=(f(fertility)+f(biomassDensity))/2` (same log
quadratic), `palette=0.5` for `Flat`/`FlatRing`/`Rect` else `1.0`, and straight-alpha
`RGB=base*palette, A=opacity*255` (landed in `8104046`). Headless check at 1920x1080, centre (0,0),
orthographic size 120:

| scene | viewer | game reference | residual |
|---|---|---|---|
| Default save zone | `(195,130,0)` | `(195,131,0)` | 1/255 (green) |
| `lol.zip` Flat zone | `(107,71,0)` | `(107,72,0)` | 1/255 (green) |

So the formula and straight-alpha compositing are confirmed on both palettes; the grid shows through the
zone as in the game. Remaining viewer follow-ups:

* **Base colour source:** viewer uses a measured constant `[231,154,0]`. If the §5 FertilityColor sweep
  confirms `base_srgb == UserSettings.FertilityColor`, view-dev will read the runtime/user setting instead
  (game code default is `(11,128,0)`; the measured golden implies a custom profile colour or a fixed
  shader constant).
* **ShowFertility toggle:** viewer currently always draws the zone fill. The game shows it only when
  `UserSettings.ShowFertility` is true (default true); canonical reference frames use true, so this does
  not affect parity today. Wire it if/when the setting source is available and a `false` reference exists.
* **Pattern:** the shader's non-uniform radial pattern is still approximated by a flat fill; acceptable for
  colour parity (dominant-colour residual ≤1/255), not pixel-exact zone art.

No further changes to this document are needed unless the optional §5 captures change the base-colour
confidence or the ShowFertility behaviour.
