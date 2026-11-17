# RUSTibites Spec 04 - File Formats, Settings Defaults, Save/Scenario Round-Trip

Status: **implementation-ready draft for milestone 1** (formats + settings); **implemented for the 0.6.3.x tier and kept current (2026-10-30)**.<br>
Owner: fmt-dev. Task: `task-4`. This document is the authoritative reference for `crates/bibites-io`
and for the settings model in `crates/bibites-core/src/settings.rs`.

## Implementation status (2026-10-30)

**Overall: implemented for the 0.6.3.x round-trip tier; T0 complete, T1/T2 partial** (see §8.5). Code:
`crates/bibites-core/src/settings.rs`, `crates/bibites-io/src/{lib,model,bundle,settings,world,migrate,version}.rs`.
The real game has loaded Rust-written saves, templates and scenarios (see `docs/ARCHITECTURE.md` §8).

| Section | Status | Code | Representative tests / evidence |
|---|---|---|---|
| §0 present version / target | implemented | `version::PRESENT = "0.6.3.1"`; `detect_shape` | version parser tests; round-trip acceptance `testdata/roundtrip/` |
| §1 encoding, formatting, numbers | implemented | `bundle::to_json_bytes` (BOM for zip entries), `lib` file writers (no BOM), f32→f64 shortest-double emission | io `bundle_i3`, `i1`, `settings_i2` |
| §2 settings architecture / 3 scopes | implemented | `settings.rs` (`SimSettings`, `ScenarioSettings`, `IndependentSettings`, `MaterialSettings`), io `settings.rs` | `settings_i2`, `defaults_match_spec_tables` |
| §3.1–3.7 settings tables (78+14+materials+zones+groups+spawn rows) | implemented, exact defaults/ranges/order | `settings.rs` `SETTINGS` metadata + io parse/serialize | `scenario_table_has_78_settings`, `independent_table_has_14_settings`, `defaults_match_spec_tables`, `ranges_match_spec_tables`, `metadata_order_matches_canonical_order`, `zone_and_spawn_enum_names_round_trip` |
| §3.8 animators & challenge | animators implemented end-to-end; challenge types/condition evaluation implemented, challenge **lifecycle/scoring not wired** into the engine | `world.rs` animator evaluation; `settings.rs` `SettingsChanger`/`ConditionGroup`/`SimulationMetric`/`ChallengeParameters` | `settings_animator_applies_scenario_and_zone_settings`, `settings_animator_start_delay_and_repeat_wrap`, `condition_group_evaluates`, `challenge_defaults` |
| §4 `.bb8` / `.bb8template` | implemented (strict genome loaders + tolerant template path) | io `lib.rs`, `model.rs`, `migrate.rs` | io `i1`, `template_compat` (220/220 real templates) |
| §4.7 genome/brain/entity migrations | **partial:** brain gates 0.6a5/0.6a13/0.6a15/0.6a16/0.6.2a1 + gene 0.6a1/0.6a7/0.6a15 implemented; defensive content-signature era detection not implemented | io `migrate.rs` | `i1`, `template_compat` |
| §5 save bundle | implemented (all sections; entity ids/species typed; `data.bin` opaque) | io `bundle.rs`, `world.rs` | `bundle_i3`, `world_i4a`; round-trip acceptance |
| §6 scenario bundle | implemented (settings, scenario.info, bundled templates) | io `bundle.rs`, `world.rs` | `bundle_i3`; scenario acceptance artifact |
| §7 animators & challenge conditions | animator schema/runtime implemented; challenge scoring/lifecycle **partial** (model only) | `settings.rs`, `world.rs` | `settings_changer_validation`, `simulation_metric_validation`, `condition_group_simplifies_like_the_game` |
| §8 version model & migrations | version grammar/ordering implemented; brain/gene migrations implemented; **§8.3 settings migrations not implemented**; 0.6.4a1 folder-layout detection not implemented | io `version.rs`, `migrate.rs` | version tests; **gap** (see notes) |
| §9 round-trip requirements | implemented: unknown-field preservation, canonical emission, f32 formatting, `data.bin` passthrough; verified by loading outputs in the real game | io | `bundle_i3`, `i1`; `tools/roundtrip/*_acceptance.json` |
| §10 Rust design notes | implemented (serde model, zip, errors, no serde in core) | io | workspace tests |
| §11 open questions | Q1/Q2 target decisions recorded; Q7 newer-format handling is **partial** (see notes) | — | — |
| Appendix A reference tables | implemented in `settings.rs`/`model.rs` | — | defaults/ranges tests |

### Implementation notes (deviations and known gaps)

* **Settings migrations (§8.3) are explicitly OUT OF SCOPE for M1** (Lead decision 2026-10-30). The loader applies
  current (0.6.x) semantics to scalar values and does **not** transform old log10 zone values, percent efficiencies,
  `reactivity/30`, `bibiteMassDensity/1.9` or `bitePeriodFactor/35`. T0 files are fully supported; pre-0.6 files load
  structurally only and must not be treated as behaviourally migrated. Future work: implement the §8.3 gates and then
  enable T1/T2 in §8.5.
* **0.6.4a1 folder saves are explicitly OUT OF SCOPE for M1** (Lead decision 2026-10-30): the supported target is flat
  `.zip` saves of the installed 0.6.3.1 build. A folder-save path currently fails with a generic I/O/zip error rather
  than the dedicated `Unsupported("... folder layout ...")` message promised in §0.1/D13; that diagnostic is future
  work, not a supported format.
* **Challenge support is model-only:** `ChallengeParameters` and `ConditionGroup`/`SimulationMetric` parse,
  validate and evaluate (with tests), but challenge start/exit/scoring is not integrated into `World`/CLI. M1 uses
  plain runs.
* **Brain migration RNG:** the 0.6a13 Constant-input removal uses a deterministic input pick instead of Unity
  `Random.Range(1,14)` (D4/§8.4) — the only non-deterministic game migration step.
* `data.bin` is carried opaquely (D11); `speciesData.json` is typed through `LineageRecordIo` with the raw JSON
  preserved as a fallback.
* Tolerant template loading (`D14`, task-44) repairs node-index/duplicate-innovation issues and re-emits
  canonically at 0.6.3.1; it does not preserve original template bytes.

Everything here was extracted from the decompiled installed build in `Here/`, then cross-checked
against real files under `C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites\` (read-only;
all analysis copies were unzipped into `%TEMP%`). External reverse-engineering notes on this
machine (the `BibitesInfo` project: `docs/SAVE_FORMAT.md`, `BibitesAPI/bibites_api/core/catalog/*.json`,
`docs/GAME_GUIDE.md`) were used only as leads; every material claim below was re-verified against
`Here/` source or against real files. Where a fact comes only from those external notes it is
marked **[external]**.

## 0.1 What "present version" means here

The round-trip target for M1 is the **installed game as decompiled**: the build whose managed
assembly is `The Bibites_Data/Managed/BibitesAssembly.dll` and whose `Application.version` string
is **`0.6.3.1`** (Unity 6000.0.44f1, BepInEx 5.4.23.5).

Evidence:

| Evidence | Value | Where |
|---|---|---|
| `Application.version` bundleVersion string | `0.6.3.1` | `The Bibites_Data/globalgamemanagers` (ASCII blob at offset ~2008) |
| Last version stored by the game itself | `0.6.3.1` | PlayerPrefs `VersionOnLastOpen` (`HKCU\Software\The Bibites\The Bibites`, REG_BINARY bytes `30 2E 36 2E 33 2E 31 00`); written by `UserSettings.versionOnLastOpen` (`Here/SettingScripts/UserSettings.cs:559-568`) in `AppInitializer.Awake` (`Here/ManagementScripts/AppInitializer.cs:120`) |
| Newest save header seen on disk | `0.6.3.1` | `Savefiles\ozymandias .zip` scene, 2026-10-25; 1060 of 1787 save zips scanned |
| Decompiled-code save migrations present | up to `0.6.3a3` (settings animators) and `0.6.2a1` (brain) | `Here/ScriptHelpers/SerializationHelper.cs:511`, `Here/ScriptHelpers/BrainUpdater.cs:16` - both are *older* than `0.6.3.1`, so they are consistent with this build (see the version-numbering rule in 4.7: a 4-int `Version(a,b,c,d)` is an **alpha** version `a.b.c + alpha d`, not `a.b.c.d`) |
| Real saves from other builds present in app data | `0.6.3a3` (320), `0.6.3` (41), `0.6.3.3` (1), `0.6.4a1` (365) | scan of `scene.bb8scene.version` over all save zips |
| Compatible-from floor | saves **older than 0.4** are refused (the `0.4` change entry carries `canUpdateFromOlderVersion=false`) | `Here/ScriptHelpers/VersionTracker.cs:9-45`, `Here/ManagementScripts/SaveSystem.cs:724-734` |

The decompiled source in `Here/` is therefore the 0.6.3.1 build: all its version gates are alphas
at or below 0.6.3a3, and the only *release* version constants are 0.6.3.1-era (`ScenarioItemReference`
min 0.5.1) and `0.6.3.3`/`0.6.4a1` only appear in real data, not in `Here/`. Our writers emit
`version: "0.6.3.1"` by default (configurable - see 9.7).

**Newer-build real data is out of scope for M1** (Lead decision): the 365 `0.6.4a1` saves use a
*folder* layout (`<SaveName>/sim.info` + `saves/_00.zip` + `saves/autosaves/*.zip`) that the
decompiled source does not contain (`sim.info`/`_00.zip` are absent from the `BibitesAssembly.dll`
string table; `SaveController` only knows flat `Savefiles/*.zip`,
`Here/ManagementScripts/SaveController.cs:51-53,296-324`). The loader must detect that layout and
return a clear `Unsupported` error. Flat `.zip` saves written by 0.6.3.1 (the target) load normally.
*(Implementation status: the explicit folder-layout detection is not implemented yet — a directory path
currently fails with a generic I/O/zip error; tracked as a gap in the status notes at the top.)*

Anomaly for the Lead's capture phase: the 0.6.4a1 schema adds three scalar settings
(`shadeDistance`, `shadeFadeDistance`, `useNetworkingFeature`), removes `genes.tag` and adds
`genes.isMutant` behaviour **[external]**, so do not copy 0.6.4a1 saves into golden round-trip
fixtures unless we explicitly extend the target.

## 0.2 Confidence labels used below

* **verified** - read in `Here/` and confirmed against at least one real file.
* **verified (source only)** - read in `Here/`, no matching real-file sample available.
* **[external]** - from the `BibitesInfo` docs; not independently re-verified byte for byte.
* **open** - explicitly listed in section 11.

## 0.3 File inventory

| Extension | Path under `%USERPROFILE%\AppData\LocalLow\The Bibites\The Bibites\` | Role |
|---|---|---|
| `.bb8` | `Bibites\*.bb8` (also loose in root from old versions) | one bibite or egg entity snapshot; used as a genome source when loaded as template |
| `.bb8template` | `Bibites\Templates\*.bb8template` | genome + brain + genes template (no body/clock) |
| `.bb8settings` | `SimulationSettings\*.bb8settings`, entry inside save/scenario zips | all simulation settings, 3 scopes |
| `.bb8scene` | entries inside save zips | scene / pellets / pheromones / vars sidecars |
| `.zip` | `Savefiles\*.zip`, `Savefiles\Autosaves\*.zip` | world save bundle |
| `.zip` | `Scenarios\*.zip` | scenario bundle |
| `.json` | `speciesData.json` inside save zip | species lineage database |
| `.bin` | `data.bin` inside save zip | binary statistics history |
| `.info` | `scenario.info` inside scenario zip | scenario metadata |
| `.json` | `Savefiles\<name>\sim.info` (0.6.4a1 folder saves) | **unsupported layout** |


# 1. Encoding, number and key conventions

These rules apply to **every** JSON file in the project. Getting them wrong produces files the game
rejects (or silently misloads).

## 1.1 Character encoding and BOM

| Writer | Encoding | BOM | Verified by |
|---|---|---|---|
| Save zip entries (`settings.bb8settings`, `scene.bb8scene`, `vars.bb8scene`, `pellets.bb8scene`, `pheromones.bb8scene`, `speciesData.json`, `bibites/*.bb8`, `eggs/*.bb8`, `templates/*`) | UTF-8 | **yes** (`EF BB BF`) | `Here/ManagementScripts/SaveSystem.cs:305-309` (`new StreamWriter(stream, Encoding.UTF8)`), bytes of real `settings.bb8settings`/`scene.bb8scene`/`bibite_0.bb8` from `autosave_20261008004410.zip` start `EF BB BF` |
| Scenario zip entries (`settings.bb8settings`, `scenario.info`) | UTF-8 | **yes** | same writer (`SaveSystem.cs:305-309`); real `Default.zip` |
| `.bb8` export file | UTF-8 **no BOM** | no | `Here/ManagementScripts/SaveSystem.cs:149-150` (`File.CreateText`), real `Sample.bb8` starts `{\r\n  "` |
| `.bb8template` file | UTF-8 **no BOM** | no | `Here/ManagementScripts/SaveSystem.cs:131-132`, real `Sample.bb8template` starts `{"name"` |
| `sim.info` (unsupported 0.6.4a1 layout) | UTF-8, 2-space indented | no | real files |

The game's readers wrap entries in `new StreamReader(stream, Encoding.UTF8)`
(`SaveSystem.cs:1064-1068`), which accepts BOM or none. `JObject.Parse` and `JsonUtility`
both tolerate BOM after decoding. **Our writers must emit exactly the BOM/format of the target
file class**, because the validation captures compare semantic content but tooling (and future
versions) may not tolerate surprises; a file with no BOM in a save zip is accepted in practice but
is a deviation.

## 1.2 Whitespace / formatting

* Save and scenario zip JSON: compact (`Formatting.None`), **zero whitespace**, no trailing newline.
  Verified: `autosave_20261008004410.zip` `settings.bb8settings` is a single 10986-byte line; the
  writer is `JObject.ToString(Formatting.None)` at `SaveSystem.cs:308`.
* `.bb8` / `.bb8template`: `JObject.ToString(FormatBibiteJSON ? Formatting.Indented : Formatting.None)`
  (`SaveSystem.cs:132,150`; `UserSettings.FormatBibiteJSON` is a PlayerPrefs user setting,
  `Here/SettingScripts/UserSettings.cs:91-97`). Default in code is `false`; the live install's
  registry shows `FormatBibiteJSON = 1` (indented) which explains why `Sample.bb8` is 2-space
  indented while `Sample.bb8template` (written before/with default) is compact. Both are legal.
  Our writer should default to compact for templates and mirror the user preference for `.bb8`
  exports (CLI flag `--pretty`, default maybe mirror install = pretty for `.bb8`).
* `scenario.info` files written by the game are **indented with a literal TAB**
  (`JObject.ToString(Formatting.Indented)` in `ScenarioSaver.cs:189` plus the stock shipped files
  use tabs); semantically irrelevant.

## 1.3 Numeric representation (critical for parity captures)

The game stores C# `float` (f32) but Newtonsoft `JToken` holds `double`/`long`. The writer
path is `JToken.op_Implicit(float)` -> `JValue(double)` -> shortest round-trippable **double**
representation, which exposes the f32 error tail:

`0.05f` becomes `0.05000000074505806`, `0.1f` becomes `0.10000000149011612`,
`0.0075f` becomes `0.007499999832361937`, `1e-05f` becomes `0.00001` (shortest double for that
exact f32 is `0.00001`), `0.6f` becomes `0.60000002384185791`.

Real example from `autosave_20261008004410.zip`:
`"backgroundMutationVariance":{"interactableChanged":{"m_PersistentCalls":{"m_Calls":[]}},"Value":0.05000000074505806}`.

Rules for our writer:

1. Model values as `f32`. Serialize Go-like shortest round-trip representation of `(f64)(f32)`,
   i.e. the same algorithm Newtonsoft uses for a `double` whose value is exactly the widening of
   the f32. This is what produces the `...05806` tails; do **not** print `0.05`.
2. Integers (`IntSetting`, counts, ids, indexes) are written as JSON integers, never `1.0`.
   (`JToken.op_Implicit(int)`.)
3. `long`/`ulong` (innovations, tower ids, species ids) are JSON integers. `ulong` tower IDs are
   small in practice.
4. Booleans are lower-case `true`/`false`.
5. Enums are written as **name strings** in every hand-written section (`distribution`,
   `movement`, `tagging`, `randomizeGenes`, `growthAtSpawn`, `spawnType`, `comp`, `logic`,
   `d[]` interpolation, `metric1/metric2`). The only JsonUtility-serialized objects are the
   scalar settings (no enum-typed fields there), so numeric enum values are not needed on write;
   readers accept both names and integers (`JToken.ToObject<TEnum>()`).
6. `NaN`/`Infinity`: Newtonsoft writes them as bare `NaN`/`Infinity` tokens, which are invalid
   JSON. They should never occur (all settings are clamped by the UI, and physics never saved
   as NaN); treat any encountered NaN/Inf on export as an error, not a silent string.
7. Negative zero: print `-0.0` only if the f32 is negative zero (Newtonsoft does); do not
   normalize unnecessarily for golden tests.

## 1.4 Key order and unknown fields

* All JSON object readers are key-based: `JObject` indexers (`state["x"]`), `JsonUtility.FromJsonOverwrite`,
  `JToken.ToObject<T>()`. **Order never matters for loading, with one exception: `data.bin`, which is
  a byte layout (section 5.6).** Key order still matters for byte-identical round-trip tests, so we
  emit orders documented in this spec (they are the game's emission orders).
* Unknown fields are ignored by the game's readers in all JSON files:
  * `SerializationHelper.DeserializeGeneralObject` iterates the **JSON** keys and looks up the
    C# field by name; unknown keys are skipped (`Here/ScriptHelpers/SerializationHelper.cs:112-142`).
  * `JObject.ToObject<T>()` ignores extra members (Newtonsoft default).
  * Categories of "unknown" we must preserve on import->export (Rust side) are listed in section 9.
* Duplicate JSON keys must not be produced. (Newtonsoft keeps the last on parse; JsonUtility's
  behaviour is undefined-ish. Our writer emits each key once.)

## 1.5 Paths inside zips

`ZipArchive` entry names are written with `Path.Combine`, so on Windows real saves contain
`bibites\bibite_0.bb8` (backslash) while our writer on Windows would do the same; on Linux it would
produce `/`. The loader matches `entry.FullName.StartsWith("bibites")` / `StartsWith("egg")`
(`SaveSystem.cs:796-806`) and `zip.GetEntry("settings.bb8settings")` for named entries, so:
**always write `bibites/<name>` and `eggs/<name>` with forward slashes** (accepted everywhere and
portable); named top-level entries must be exact strings (`settings.bb8settings`, `scene.bb8scene`,
`vars.bb8scene`, `pellets.bb8scene`, `pheromones.bb8scene`, `data.bin`, `speciesData.json`,
`img.png`, `templates/<file>`). Compression is the `ZipArchiveMode.Create` default (Deflate).
Entry order is irrelevant to the loader, but we write the documented order for parity.

# 2. Settings architecture and the three scopes

The game has exactly three classes of *simulation* settings plus per-object setting objects. Only
one scalar field of each `Setting<T>` is persisted: the private `[SerializeField] Value` (or the
custom-written value). **All metadata (`Name`, `HelperText`, `minValue`, `maxValue`, `units`,
`factor`, `precision`, `DefaultValue`, landmarks, choices) is code-only and never appears in any
file** - so an implementation must bake the defaults from its own code, not from the files.

| Scope | Class (source) | Persisted as | Count |
|---|---|---|---|
| Scenario-independent (world/TPS/species limits) | `ScenarioIndependentSettings` (`Here/SettingScripts/ScenarioIndependentSettings.cs`) | save: `settings.bb8settings.independents`; scenario: `settings.bb8settings.recommended`; legacy .bb8settings: top-level keys | 14 |
| Scenario (shared ecosystem simulation) | `ScenarioSettings` (`Here/SettingScripts/ScenarioSettings.cs`) | top-level scalar keys of `settings.bb8settings` (save and scenario) | 78 |
| Zone | `ZoneSettings` (`Here/SettingScripts/ZoneSettings.cs`) | elements of `zones[]` and `zoneGroups[].template`/`.zones[]` | 20 setting fields whose JSON keys are custom-mapped |
| Zone group | `ZoneGroupSettings` (`Here/SettingScripts/ZoneGroupSettings.cs`) | `zoneGroups[]` | 4 |
| Per-template spawn row | `BibiteSettings` (`Here/SettingScripts/BibiteSettings.cs`) | `bibites[]` | 9 |
| Material physical params | `MatterMaterialSettings` (`Here/SettingScripts/MatterMaterialSettings.cs`) | `materials.<Material>Settings` | 9 |
| Animator | `SettingsChanger` (`Here/SettingScripts/SettingsChanger.cs`) | `settingsChangers[]` | 3 + point arrays |
| Challenge | `ChallengeParameters` (`Here/SettingScripts/ChallengeParameters.cs`) | `challenge` | condition trees |

The two scalar scopes are serialized by Unity `JsonUtility.ToJson`, which is why every scalar
setting is wrapped:

```json
"backgroundMutationChance": {
  "interactableChanged": { "m_PersistentCalls": { "m_Calls": [] } },
  "Value": 1.5
}
```

`interactableChanged` is the public `UnityEvent<bool>` field inherited from `Setting<T>`
(`Here/SettingScripts/Setting.cs:13`); `Value` is the private `[SerializeField]` field declared in
`SimulationSetting<T>`/`NumericSimulationSetting<T>`/`EnumSimulationSetting<T>`
(`Here/SettingScripts/SimulationSetting.cs:7-8`, `NumericSimulationSetting.cs:7-8`,
`EnumSimulationSetting.cs:8-9`). `DefaultValue` is `[NonSerialized]` (`Setting.cs:10-11`), which is
why it never appears.

Wrapper requirements for a file the game must load:

* `Value` is **mandatory** for a setting we want applied. If absent, `JsonUtility` leaves the live
  value untouched (it does not fail). Missing wrapper entirely = same as absent key.
* `interactableChanged` is not needed for loading; `JsonUtility.FromJsonOverwrite` only assigns
  what it finds. The game's own writer emits it, in some files with `"m_Calls":[]` and in others
  with the quirk `"m_Calls":""` (observed inside `independents` of real saves). Both are accepted.
  Our writer emits `{"m_PersistentCalls":{"m_Calls":[]}}` everywhere.
* **JsonUtility does not clamp** to min/max when loading a file; the clamp lives in
  `NumericSetting.SetValue` (`Here/SettingScripts/NumericSetting.cs:56-63`) and is used by UI/animators.
  Therefore malformed out-of-range values load as-is. For behavioural parity we clamp on import
  where the game later clamps implicitly on UI, but must **preserve out-of-range imported values**
  unless documents decide otherwise (see Q6).

All default values in the tables below are the `DefaultValue` literals in the C# initializers; the
`val` literal is identical unless noted. The `src` column is the line in `Here/` where the setting
object starts. UI "units" are display metadata (with the UI `factor` scaling for display); the file
always contains the raw value.


# 3. `.bb8settings` - settings files

## 3.1 File classes and top-level shape

A `.bb8settings` exists in two flavours inside zips and one legacy standalone flavour:

| Flavour | Producer | Top-level keys |
|---|---|---|
| **Save settings** | `SaveSystem.CreateSave` -> `SerializationHelper.SerializeSettings()` (`Here/ManagementScripts/SaveSystem.cs:277`, `Here/ScriptHelpers/SerializationHelper.cs:267-281`) | 78 `ScenarioSettings` scalars + `materials`, `zones`, `bibites`, `settingsChangers`, `zoneGroups`, `independents` = **84 keys** (verified on `autosave_20261008004410.zip`) |
| **Scenario settings** | `ScenarioSaver.SaveScenario` -> `SerializeScenario(saveRecommendations: true)` (`Here/SettingScripts/ScenarioSaver.cs:170`, `SerializationHelper.cs:283-307`) | 78 scalars + `materials`, `zones`, `bibites`, `settingsChangers`, `challenge?`, `zoneGroups`, `recommended` (not `independents`) |
| Legacy standalone `.bb8settings` | old game versions exported from the settings menu; files in `SimulationSettings\` are all from 2022, version ~0.5 | a flat object mixing old + new setting names, e.g. `SimulationSize`, `plantGrowth`, `materials`, `colorR`... (real files `SimulationSettings\Cow World Settings.bb8settings`, all ~7 KB) |
| Legacy settings names in saves | pre-0.6 saves | `settings.json` is accepted as a fallback for `settings.bb8settings`; `scene.json` for `scene.bb8scene` (`SaveSystem.cs:988-996`) |

The save loader picks the scalar scope as `settingsJSON["independents"] ?? settingsJSON`
(`SerializationHelper.cs:309-315`): modern saves keep the 14 independent fields nested, legacy
files keep everything at top level. Scenario loading always starts from `JsonUtility.FromJsonOverwrite(scenarioJson, ScenarioSettings.Instance)`
so the 78 scalars live at the top level, then applies `recommended` as the independent block.

The 78+14 scalar settings and their exact defaults are in tables 3.2 and 3.3. Their JSON key is
always the C# field name (method names are lowerCamel except `SimulationSize`, which is
capitalised). The key order in a file produced by the game is the field declaration order - see
the table row order. **Write all 78 + 14 keys**: the save loader does not reset missing
`ScenarioSettings` keys (it only force-updates the keys present), so a partial save silently
inherits whatever the previous world left in memory; the scenario loader resets every missing
`ScenarioSettings` key to its default (`SerializationHelper.cs:349-363`).

## 3.2 Scenario-independent settings (`independents` / `recommended`)

Serialized by `JsonUtility.ToJson(ScenarioIndependentSettings.Instance)`
(`SerializationHelper.cs:271-279`, `SerializeScenario` line 289-292). Each entry has the
`interactableChanged`/`Value` wrapper from section 2. 14 fields:

| # | JSON key | type | UI label | default | min | max | units | src |
|---|----------|------|----------|---------|-----|-----|-------|-----|
| 1 | `SimulationSize` | f32 | Size of the Simulation Area | `2000` | 100 | 20000 | `u` | 16 |
| 2 | `biomassDensity` | f32 | Global Biomass Density | `0.0075` | 0 | 1 | `E/u^2` | 42 |
| 3 | `pelletGrowth` | f32 | Global Fertility | `0.00001` | 0 | 0.0005 | `E/u^2s` | 68 |
| 4 | `limitBibiteBirth` | bool | Limit Bibite Birth | `false` | - | - | - | 92 |
| 5 | `bibiteLimit` | i32 | Bibite Limit | `100` | 0 | 500 | - | 101 |
| 6 | `virginSpawnRate` | f32 | Bibite Spawning Rate | `1` | 0 | 5 | `Hz` | 113 |
| 7 | `speciesGeneticSpan` | f32 | Species Genetic Span | `1.5` | 0.25 | 5 | - | 126 |
| 8 | `speciesSpanScalingFactor` | f32 | Species Span Scaling Factor | `0.05` | 0 | 0.5 | `%/species` | 140 |
| 9 | `speciesPrunePointLimit` | i32 | Species Pruning Point Limit | `2` | 1 | 20 | - | 156 |
| 10 | `simTPS` | i32 | Ticks Per Second | `30` | 5 | 100 | `tps` | 170 |
| 11 | `brainTPS` | i32 | Brain Update Factor | `2` | 1 | 100 | `X` | 182 |
| 12 | `decayTPS` | i32 | Decay Update Factor | `2` | 1 | 100 | `X` | 194 |
| 13 | `visionLookupUpdateFactor` | i32 | Vision Lookup Factor | `10` | 1 | 100 | `X` | 206 |
| 14 | `visionSenseUpdateFactor` | i32 | Vision Sensing Factor | `5` | 1 | 50 | `X` | 218 |

Notes on semantics / why the default matters:

* `SimulationSize` S: half-width of the square world; zones' positions and relative radii are
  fractions of S; densities use area in `u^2`; `simArea = 4*S*S` (`ScenarioIndependentSettings.cs:230`).
* `biomassDensity` is E/u^2 globally; `pelletGrowth` is E/(u^2*s). Zone `maxBiomass` and `totalGrowth`
  multiply these by zone values and area (`ZoneSettings.cs:315-349`).
* `limitBibiteBirth` + `bibiteLimit` cap births (checked in the bibite spawner); `virginSpawnRate`
  is Hz for template-based spawns.
* `speciesGeneticSpan`, `speciesSpanScalingFactor`, `speciesPrunePointLimit` govern speciation and
  pruning; `usedSpeciesSpan = span + max(activeSpecies-20,0)*scaling` (`GlobalLineageManager.cs:73-79`).
* TPS settings (`simTPS` 30, `brainTPS` 2 => brain period 2/30 s, `decayTPS` 2) are part of the
  saved state: they change dynamics and must be honoured by our engine. `visionLookupUpdateFactor`
  (10 brain updates) and `visionSenseUpdateFactor` (5) likewise.
* Nine independent settings are **not animatable** (`controllable=false`): `SimulationSize`,
  `speciesGeneticSpan`, `speciesSpanScalingFactor`, `speciesPrunePointLimit`, `simTPS`, `brainTPS`,
  `decayTPS`, `visionLookupUpdateFactor`, `visionSenseUpdateFactor`. Scenario `recommended` blocks
  frequently include `SimulationSize`, `biomassDensity`, `pelletGrowth` (real `Scenarios\Default.zip`),
  which is how a scenario proposes a world size; the scenario preview applies `recommended` over the
  live settings with missing keys **reset to their defaults** (`SerializationHelper.cs:364-384`).

### 3.2.1 Precedence of scenario-independent values (save vs. scenario vs. globals)

Verified on real 0.6.3.1 data (2026-10-29):

* **Save zips DO carry the 14 independent values**, as the `independents` object of
  `settings.bb8settings`. The values are per-world, not shared: e.g.
  `Savefiles\autosave_20261008004410.zip` has `SimulationSize=2000, simTPS=30`,
  `Savefiles\autosave_20260612092150.zip` has `SimulationSize=20000, simTPS=15`,
  `Savefiles\jade.zip` has `SimulationSize=15000, simTPS=15`,
  `Savefiles\lol.zip` has `SimulationSize=2000, simTPS=15` (all 14 keys present in each).
  This matches the decompiled writer (`SerializationHelper.SerializeSettings` writes
  `val["independents"] = JsonUtility.ToJson(ScenarioIndependentSettings.Instance)`) and the loader
  (`DeserializeSettings` reads `settingsJSON["independents"] ?? settingsJSON` and applies it).
* A **scenario** carries only `recommended`, and it is usually a small subset (e.g.
  `Scenarios\Default.zip`: `SimulationSize`, `biomassDensity`, `pelletGrowth`; `Ring.zip`:
  `SimulationSize=50000`, `simTPS=15`, `brainTPS=2`). When a NEW world is started from a scenario the
  engine applies `recommended` over the current independent settings, resetting keys that are absent
  to their defaults (3.2 above).
* The installed 0.6.3.1 game has **no code path that loads a standalone `SimulationSettings\*.bb8settings`
  as global settings** (the folder only contains 2022 leftovers; `SimulationSettings` is not
  referenced by the current source). A "game-global" copy of the independent values therefore only
  exists as (a) the live in-memory state built from code defaults + whatever the last scenario/save
  load applied, and (b) PlayerPrefs `simTPSDefault`/`brainTPSDefault`/... defaults
  (`UserSettings.cs:439-467`) used by the settings UI for *new* values.

Loader policy for RUSTibites (decided with the Lead 2026-10-29): load precedence is
**(1) save's `independents` block when the file has one, else (2) `--settings`/`--independents`
override supplied on the CLI, else (3) code defaults**. `SimSettings::sync_rates_from_independents`
must be called after whichever source is applied. When **exporting** a save, mirror the 0.6.3.1
writer and include the `independents` object (do not drop it). A parity-capture manifest must record
the effective independent settings plus `gameVersion` (`0.6.3.1`) so runs can be reproduced.

If a capture is taken from a 0.6.3.3+ *scenario with starting state* (e.g. `Pixel Art_t0.zip`, whose
`settings.bb8settings` has `recommended` and no `independents`, and which also bundles
`scene/pellets/pheromones/vars/speciesData`), those starting-state sections are out of scope for the
0.6.3.1 target and must be reported as a newer-format `Unsupported` case (Q7).

## 3.3 Scenario settings (78 scalars)

Serialized at the top level of both save and scenario `settings.bb8settings`
(`JsonUtility.ToJson(ScenarioSettings.Instance)`, `SerializationHelper.cs:295-307`). The same
wrapper as section 2. Full list with defaults/min/max/units:

| # | JSON key | type | UI label | default | min | max | units | src |
|---|----------|------|----------|---------|-----|-----|-------|-----|
| 1 | `backgroundMutationChance` | f32 | Background Mutation Chance | `1.5` | 0 | 20 | - | 13 |
| 2 | `backgroundMutationVariance` | f32 | Background Mutation Variance | `0.05` | 0 | 0.5 | `%` | 26 |
| 3 | `relativeMutationShare` | f32 | Mutation Value Relativity | `0.75` | 0 | 1 | `%` | 41 |
| 4 | `synapseMutationChance` | f32 | Synapse Mutation Probability | `0.6` | 0 | 1 | `%` | 56 |
| 5 | `neuronMutationChance` | f32 | Neuron Mutation Probability | `0.4` | 0 | 1 | `%` | 71 |
| 6 | `synapseChangeChance` | f32 | Synapse Strength Mutation Probability | `0.75` | 0 | 1 | `%` | 86 |
| 7 | `synapseFlipChance` | f32 | Synapse Flip Probability | `0.025` | 0 | 1 | `%` | 101 |
| 8 | `synapseToggleChance` | f32 | Synapse Toggle Probability | `0.025` | 0 | 1 | `%` | 116 |
| 9 | `synapseAddChance` | f32 | Synapse Add Probability | `0.1` | 0 | 1 | `%` | 131 |
| 10 | `synapseRemoveChance` | f32 | Synapse Removal Probability | `0.1` | 0 | 1 | `%` | 146 |
| 11 | `neuronDefaultChance` | f32 | Default Activation Mutation Probability | `0.7` | 0 | 1 | `%` | 161 |
| 12 | `neuronChangeChance` | f32 | Neuron Function Mutation Probability | `0.1` | 0 | 1 | `%` | 176 |
| 13 | `neuronAddChance` | f32 | Neuron Add Probability | `0.1` | 0 | 1 | `%` | 191 |
| 14 | `neuronRemoveChance` | f32 | Neuron Removal Probability | `0.1` | 0 | 1 | `%` | 206 |
| 15 | `pelletCollision` | bool | Pellet Collision | `true` | - | - | - | 221 |
| 16 | `pelletMerge` | bool | Pellet Merge | `false` | - | - | - | 230 |
| 17 | `pelletRotation` | bool | Pellet Rotation | `true` | - | - | - | 239 |
| 18 | `eggCollision` | bool | Egg Collision | `true` | - | - | - | 248 |
| 19 | `disableHerding` | bool | Disable Herding | `false` | - | - | - | 257 |
| 20 | `minPelletSize` | f32 | Minimum Pellet Size | `0.1` | 0.001 | 10 | `u^2` | 265 |
| 21 | `corpsesEnabled` | bool | Corpses | `true` | - | - | - | 277 |
| 22 | `preventMutations` | bool | Prevent Mutations | `false` | - | - | - | 286 |
| 23 | `voidAvoidance` | bool | Void-No-Mo' | `false` | - | - | - | 294 |
| 24 | `voidAvoidanceDistance` | f32 | Void Avoidance Distance | `100` | 0.0001 | 1000 | `u` | 303 |
| 25 | `preventRedPheroProduction` | bool | Prevent Red Pheromones Production | `false` | - | - | - | 317 |
| 26 | `preventGreenPheroProduction` | bool | Prevent Green Pheromones Production | `false` | - | - | - | 325 |
| 27 | `preventBluePheroProduction` | bool | Prevent Blue Pheromones Production | `false` | - | - | - | 333 |
| 28 | `enableRedDeath` | bool | The Red Death Bloom | `false` | - | - | - | 341 |
| 29 | `redDeathFill` | f32 | Red Death Fill | `0` | 0 | 1 | `%` | 349 |
| 30 | `redDeathBaseDamages` | f32 | Red Death Base Damage | `0.5` | 0 | 100 | `HP/s` | 364 |
| 31 | `redDeathDamageVelocity` | f32 | Red Death Damages Velocity | `0.15` | 0 | 10 | `HP/s^2` | 377 |
| 32 | `shadeOutsideOfBounds` | bool | Add Shaded Bounds | `true` | - | - | - | 390 |
| 33 | `shadeAvoidance` | bool | Bibites Fear Shade | `false` | - | - | - | 397 |
| 34 | `worldWrapping` | bool | World Wraps Behind Shade | `true` | - | - | - | 406 |
| 35 | `initialSeeding` | f32 | Initial Seeding | `0.75` | 0 | 1 | `%` | 415 |
| 36 | `pelletEnergy` | f32 | Default pellet Size | `200` | 10 | 5000 | `E` | 431 |
| 37 | `fatSustain` | f32 | Fat Storage Cost | `0` | 0 | 0.05 | `%/s` | 444 |
| 38 | `dragCoefficient` | f32 | Drag Coefficient | `5` | 0 | 25 | `Nu/s` | 459 |
| 39 | `disableDragOnHeldPellet` | bool | Disable Drag On Held Pellet | `false` | - | - | - | 472 |
| 40 | `armMusclePressure` | f32 | Arm Muscles Force | `50` | 1 | 1000 | `N/u` | 480 |
| 41 | `forwardForceSizePower` | f32 | Muscles Sizing Propel Power | `1.5` | 0 | 2.5 | - | 493 |
| 42 | `turnForceSizePower` | f32 | Muscles Sizing Turning Power | `3` | 0 | 4 | - | 506 |
| 43 | `backwardFraction` | f32 | Backward Force Efficiency | `0.5` | 0 | 1 | `%` | 519 |
| 44 | `bibiteMassDensity` | f32 | Base Bibite Mass Density | `0.025` | 0.001 | 0.25 | `g/u^2` | 534 |
| 45 | `stomachContentContribution` | f32 | Stomach Content Weight Contribution | `1` | 0 | 2 | `%` | 548 |
| 46 | `bitingPressure` | f32 | Jaw Muscles Strength | `80` | 1 | 500 | `N/u` | 563 |
| 47 | `throwingForceFactor` | f32 | Throwing Force Factor | `0.25` | 0 | 1 | `%` | 576 |
| 48 | `bitingThrowForceFactor` | f32 | Biting Repulsion Force Factor | `0.25` | 0 | 1 | `%` | 591 |
| 49 | `jawMusclesSizingPower` | f32 | Jaw Sizing Power | `2` | 0 | 3 | - | 605 |
| 50 | `bitePeriodFactor` | f32 | Bite Period Factor | `1` | 0 | 10 | `X` | 617 |
| 51 | `bitingDamageFactor` | f32 | Biting Damage Factor | `5` | 0 | 20 | `%` | 631 |
| 52 | `collisionDamageConstant` | f32 | Collision Damage Constant | `0.1` | 0 | 2 | `Dmg/(u/s)` | 646 |
| 53 | `collisionDamageThreshold` | f32 | Collision Damage Threshold | `50` | 0 | 200 | `u/s` | 660 |
| 54 | `baseMetabolismCost` | f32 | Default Metabolism Cost | `0.001` | 0 | 0.1 | `E/u^2s` | 674 |
| 55 | `energyUsageEfficiency` | f32 | Energy Usage Efficiency | `0.8` | 0.1 | 1 | `%` | 688 |
| 56 | `moveMusclesCost` | f32 | Move Muscles Activation Cost | `0.02` | 0 | 1 | `E/u^2s` | 703 |
| 57 | `neuronBirthCost` | f32 | Neuron Birth Cost | `0.1` | 0 | 1 | `E` | 717 |
| 58 | `synapseBirthCost` | f32 | Synapse Birth Cost | `0.025` | 0 | 0.5 | `E` | 731 |
| 59 | `neuronUpkeepCost` | f32 | Neuron Upkeep Cost | `0.001` | 0 | 0.05 | `E/s` | 745 |
| 60 | `synapseUpkeepCost` | f32 | Synapse Upkeep Cost | `0.00025` | 0 | 0.015 | `E/s` | 759 |
| 61 | `pheromoneProductionCost` | f32 | Pheromone Production Cost | `0.01` | 0 | 0.1 | `E/s` | 773 |
| 62 | `pheromoneProductionStrength` | f32 | Pheromone Production rate | `10` | 0 | 100 | `u^2` | 787 |
| 63 | `ageingThreshold` | f32 | Ageing Threshold | `1` | 0 | 24 | `h` | 800 |
| 64 | `ageStrengthPenalties` | f32 | Age Strength Penalty | `0.25` | 0 | 1 | `%/h` | 814 |
| 65 | `ageMetabolismPenalties` | f32 | Age Metabolism Penalty | `0.5` | 0 | 1 | `%/h` | 830 |
| 66 | `healthPerArea` | f32 | Health to Area Factor | `1.25` | 0.05 | 5 | `HP/u^2` | 846 |
| 67 | `storableEnergyPerArea` | f32 | Storable Energy by Area | `1.5` | 0.05 | 5 | `E/u^2` | 860 |
| 68 | `baseBodyGrowthCost` | f32 | Base Body Growth Cost | `2` | 0.05 | 10 | `E/u^2` | 874 |
| 69 | `viewAngleAddedCost` | f32 | View Angle Added Cost | `0.003` | 0 | 0.05 | `E/degu^2` | 887 |
| 70 | `viewRadiusAddedCost` | f32 | View Radius Added Cost | `0.002` | 0 | 0.05 | `E/u^2` | 901 |
| 71 | `healthEnergyFactor` | f32 | Blood Energy Density Factor | `0.5` | 0.01 | 1 | `%` | 915 |
| 72 | `globalZoneSpeed` | f32 | Global Zone Speed | `1` | 0 | 100 | `x` | 930 |
| 73 | `healRate` | f32 | Heal Rate | `0.001` | 0.0001 | 0.02 | `% HP/s` | 944 |
| 74 | `healPowerFactor` | f32 | Heal Power Factor | `1` | 0 | 2 | - | 958 |
| 75 | `healEfficiency` | f32 | Heal Efficiency | `1` | 0.01 | 2 | `%` | 970 |
| 76 | `corpseDecayFactor` | f32 | Corpse Decay Rate | `1` | 0 | 100 | `X` | 984 |
| 77 | `plantAffinityPowerFactor` | f32 | Plant Affinity Power Factor | `1` | 0.1 | 3 | - | 997 |
| 78 | `meatAffinityPowerFactor` | f32 | Meat Affinity Power Factor | `1` | 0.1 | 3 | - | 1008 |

Categories used by the game UI (useful to keep engine systems aligned; category assignments are
UI-only, from the external catalog which was generated from the same source): mutation (rows
1-14), physics/collisions (15-20), world/red-death/shade (21-35), food/pellets (36-37),
movement/body (38-53), metabolism/costs (54-62), ageing/health (63-71), zones/healing/corpses
(72-78). The behavioural meaning of each is out of scope for this document (see spec 03 sim
systems), but the defaults are normative for parity: they are exactly the `DefaultValue`
literals above.

UI behaviour metadata that is **not** in files but matters for import/export policy:

* UI-clamped (`canGoOutOfBounds=false`, so `SetValue` clamps but JSON load does not):
  `minPelletSize`, `redDeathFill`, `energyUsageEfficiency`.
* Not controllable/animatable: `initialSeeding` (and, in the independent scope, `SimulationSize`,
  `speciesGeneticSpan`, `speciesSpanScalingFactor`, `speciesPrunePointLimit`, `simTPS`,
  `brainTPS`, `decayTPS`, `visionLookupUpdateFactor`, `visionSenseUpdateFactor`).

In this and the other generated tables, `src` is the 1-based line in the source file named by the
surrounding section where the setting object begins: `Here/SettingScripts/ScenarioSettings.cs` here,
`ScenarioIndependentSettings.cs` for 3.2, `ZoneSettings.cs` for A.1, `BibiteSettings.cs` for 3.7,
`BibiteEditorSettings.cs` for 4.6.

## 3.4 `materials`

Written by `MatterMaterialManager.SaveState` (`Here/SimulationScripts/MatterMaterialManager.cs:145-155`)
and read by `LoadState` (157-170). Keys are the class `Name` + `"Settings"`, in
`MaterialSettings` list order: **`PlantSettings`, `MeatSettings`, `FatSettings`, `ArmorSettings`**.
Each value is an object of `MatterMaterialSettings.SaveState`
(`Here/SettingScripts/MatterMaterialSettings.cs:213-231`):

| JSON key | type | when emitted | required on load | notes |
|---|---|---|---|---|
| `cohesiveness` | f32 | always | no (kept current value) | bite resistance; source field `Cohesiveness` |
| `reactivity` | f32 | always | **yes** | field `Reactivity`; legacy key fallback `digestionRate` (`MatterMaterialSettings.cs:235-237`) |
| `energyDensity` | f32 | always | **yes** | E/u^2 |
| `massDensity` | f32 | always | **yes** | g/u^2 |
| `minEfficiency` | f32 | always | **yes** | 0..1 fraction, negative allowed |
| `maxEfficiency` | f32 | always | **yes** | 0..1 fraction |
| `decay` | bool | always | no (if absent, load stops before freshTime) | |
| `freshTime` | f32 | only if `decay == true` | only if decay | seconds of freshness |
| `decayRate` | f32 | only if `decay == true` | only if decay | u^2/s |

Load-order trap: `MatterMaterialManager.LoadState` iterates `MaterialSettings` in order and
`break`s at the first missing material key, so a file that omits `PlantSettings` also loses
Meat/Fat/Armor overrides (`MatterMaterialManager.cs:157-170`). Missing material keys are not an
error - the current values (defaults on a fresh start) are kept.

Defaults are **not in the decompiled C# source**; they are in the shipped `MatterMaterial`
ScriptableObject assets, which we cannot decompile from the DLL. The serialized defaults below are
the ones observed identically in 372 of 394 real `.bb8settings` (mode), and match the external
`GAME_GUIDE` material table and `BibitesAPI/tests/factory.py`; treat it as verified-by-data plus the
2026-10-29 runtime capture (parity bridge, game 0.6.3.1), which confirmed the prefab values,
including the `freshTime`/`decayRate` values the game keeps in memory for non-decaying materials.
Those two fields are **not serialized while `decay == false`** (the writer only emits them for
decaying materials), so they only affect runtime behaviour, not round-trip bytes.

| material | cohesiveness | reactivity | energyDensity | massDensity | minEfficiency | maxEfficiency | decay | freshTime | decayRate |
|---|---|---|---|---|---|---|---|---|---|
| Plant | 10 | 0.03 | 5 | 0.015 | -0.25 | 0.5 | false | 100 (runtime; omitted from files) | 0.005 (runtime; omitted) |
| Meat | 35 | 0.075 | 10 | 0.025 | 0.15 | 0.85 | true | 2000 | 0.02 |
| Fat | 0.25 | 1 | 30 | 0.03 | 0.5 | 0.85 | false | 1000 (runtime; omitted) | 0.02 (runtime; omitted) |
| Armor | 500 | 1 | 1.5 | 0.05 | 1 | 1 | false | 1000 (runtime; omitted) | 0.02 (runtime; omitted) |

Two further materials exist in `MatterMaterialManager` but are **never serialized anywhere**:
`Energy` (`energyDensity = 1.0`) and `Health` (`energyDensity = 0.25`). They are hardcoded by the
body/energy code (same 2026-10-29 runtime capture as above; body-dev's constants carry that
provenance comment).

The parameter *ranges* (UI min/max) are in `MatterMaterialSettings.cs:16-107` (the default column
is empty there by design; per-material defaults are in the table above and in
`settings::MaterialSetting::{PLANT,MEAT,FAT,ARMOR}`):

| # | JSON key | type | UI label | default | min | max | units | src |
|---|----------|------|----------|---------|-----|-----|-------|-----|
| 1 | `cohesiveness` | f32 | Cohesiveness | - | 0 | 1000 | `N/u` | 16 |
| 2 | `reactivity` | f32 | Base Reactivity | - | 0 | 10 | `u/s` | 27 |
| 3 | `energyDensity` | f32 | Energy Density | - | 0.01 | 100 | `E/u^2` | 37 |
| 4 | `massDensity` | f32 | Mass Density | - | 0.001 | 1 | `g/u^2` | 47 |
| 5 | `maxEfficiency` | f32 | Max Conversion Efficiency | - | -1 | 1 | `%` | 58 |
| 6 | `minEfficiency` | f32 | Min Conversion Efficiency | - | -1 | 1 | `%` | 70 |
| 7 | `decay` | bool | Decay | - | - | - | - | 82 |
| 8 | `freshTime` | f32 | Freshness Time | - | 0 | 3600 | `s` | 88 |
| 9 | `decayRate` | f32 | Decay Rate | - | 0.001 | 100 | `u^2/s` | 99 |

Note that the old 2022 `.bb8settings` files use pre-rename keys (`hardness`, `digestionRate`) and
old defaults; the 0.6.0.16 migration divides reactivity by 30 and re-derives Fat reactivity
(`SerializationHelper.cs:386-400`).

## 3.5 `zones[]`

Written by `ZoneSettings.SaveState` (`Here/SettingScripts/ZoneSettings.cs:672-726`), read by
`ZoneSettings.LoadState` (728-809). `zones` is a JSON array of zone objects; real saves also put
the same zone objects inside `zoneGroups[].template` and `zoneGroups[].zones[]`.

The mapping between C# fields and JSON keys is **custom** (not the field names), so both are given:

| JSON key | C# field | type | default | emitted when | required on load | notes |
|---|---|---|---|---|---|---|
| `name` | `zoneName` | string | `"Zone N"` (constructor) | `zoneName` non-empty | no | duplicates are dangerous: pellets are bucketed by name; keep unique |
| `id` | `zoneID` | int | assigned by `maxID++` (constructor) | `zoneID >= 0` | no | **renumbered to array index on load** (`ScenarioSettings.UpdateAllZonesList`, `ScenarioSettings.cs:1092-1107`); writers must set id = index |
| `material` | `spawnMaterial` | string `Plant`/`Meat` | `Plant` | `spawnMaterial != null` | no | if absent, the following six keys are also omitted by the writer; loader keeps current material |
| `distribution` | `distribution` | enum `SpawnDistribution` | `CentricGradual` | with material | no | see enum table |
| `fertility` | `fertility` | f32 | 1 | with material | no | multiplier of global `pelletGrowth` |
| `biomassDensity` | `biomassDensity` | f32 | 1 | with material | no | multiplier of global `biomassDensity`; C# field name collides with the independent setting name but is scoped to the zone object |
| `pelletSize` | `pelletSize` | f32 | 1 | with material | no | multiplier of `pelletEnergy` |
| `renewBiomass` | `renewBiomass` | bool | false | only when `true` | no | **presence means true** (`LoadState` sets true if key exists, `ZoneSettings.cs:756-759`) |
| `movement` | `movement` | enum `MovementType` | `None` | always | no | |
| `target` | `target` | int zoneID | none | when target set | no | resolves against top-level `zones` only; stale id => no target |
| `speed` | `speed` | f32 | 1 | always | **yes** (dereferenced, `ZoneSettings.cs:768`) | multiplier of `globalZoneSpeed` |
| `posX` | `posX` | f32 | 0 | when `posX != null` | no | fraction of SimulationSize, -1..1, clamped to `1-radius` |
| `posY` | `posY` | f32 | 0 | when `posY != null` | no | |
| `radiusIsRelative` | `sizeScalesWithSim` | bool | true | always | no | selects relative vs absolute radius/width/height |
| `radius` | `radiusRelative` **or** `radiusAbsolute` | f32 | 0.25 (rel) / 1000 (abs) | non-Rect distributions | **yes** for non-Rect (`ToObject<float>` will throw if absent, `ZoneSettings.cs:800-807`) | units depend on `radiusIsRelative`; rel 0..1, abs 0..25000 u |
| `width`, `height` | `widthRelative`/`widthAbsolute`, `heightRelative`/`heightAbsolute` | f32 | 0.25 / 1000 | Rect only | **yes** for Rect (`ZoneSettings.cs:781-795`) | same relative/absolute switch |
| `insideRadius` | `insideRadius` | f32 | 0.8 | Ring, FlatRing | no | fraction of outside radius, 0..0.999 |

Derived values an engine must recompute from a zone object (used for both gameplay and the editor
estimates; formulas are normative):

* `area` (`ZoneSettings.cs:278-287`):
  * Flat / CentricGradual / ExteriorGradual => `pi * absoluteRadius^2`
  * Ring / FlatRing => `pi * absoluteRadius^2 * (1 - insideRadius^2)`
  * Rect => `absoluteWidth * absoluteHeight`
* `absoluteRadius = sizeScalesWithSim ? radiusRelative*S : radiusAbsolute`; analogously width/height
  (`ZoneSettings.cs:353-423`). `absoluteWidth`/`absoluteHeight` are kept in sync with radius for
  circle zones (`2*absoluteRadius`, `ZoneSettings.cs:582-588`).
* `maxBiomass = biomassDensity * area * independent.biomassDensity`
* `totalGrowth = fertility * area * independent.pelletGrowth`
* `estimatedPellets = ceil(maxBiomass / (pelletSize * scenario.pelletEnergy))`

Position cap: `posX/posY` are clamped to `[-(1-relativeRadius), +(1-relativeRadius)]` for circles and
`[-(1-relativeWidth/2), +...]` for rects (`CapPosition`, `ZoneSettings.cs:622-641`). UI-clamped zone
settings (`canGoOutOfBounds=false`): `posX`, `posY`, `radiusRelative`, `radiusAbsolute`,
`insideRadius`, `widthRelative`, `widthAbsolute`, `heightRelative`, `heightAbsolute`; again, JSON
load bypasses the clamp.

Default constructors: `ZoneSettings()` (Plant material, radius 0.25, all defaults above) and
`DefaultZone()` (radiusRelative=1, fertility=1) and `DefaultTemplate()` (used by zone groups:
radiusRelative=0.15, fertility=10, biomassDensity=10, no name/pos) -
`ZoneSettings.cs:545-568`; `ZoneGroupSettings` further sets its template radius to 0.05
(`ZoneGroupSettings.cs:89-90`).

## 3.6 `zoneGroups[]`

Written by `ZoneGroupSettings.SaveState`/`SaveStateWithZones`
(`Here/SettingScripts/ZoneGroupSettings.cs:150-182`), read by `LoadState` (184-205). Saves use
`zoneGroups` with embedded generated zones; scenarios use `SerializeObject(zoneGroups)` which
calls plain `SaveState` (no `zones` key) and the loader regenerates zones when the list is empty
(`SerializationHelper.cs:473-484`).

| JSON key | type | default | notes |
|---|---|---|---|
| `name` | string | `"Group N"` | zone names are rewritten as `name_i` on rename/regeneration |
| `scaleWithSim` | bool | false | if true the number of zones scales with sim area |
| `count` | int **or** f32 | 10 (int) / 0.66 (float density) | int `zoneCount` when `scaleWithSim=false`, float `zoneDensity` (zones per 1000x1000 u^2) when true. Reader chooses based on `scaleWithSim` (`ZoneGroupSettings.cs:191-199`) |
| `template` | zone object | `DefaultTemplate()` with radius 0.05 | template zone; writer may omit `name`/`id` (they are null/-1 on `DefaultTemplate()`); used to generate `count` random-position zones |
| `zones` | zone[] | - | present only in saves; regenerated if empty on load |

The zone-group settings objects themselves:

| # | JSON key | type | UI label | default | min | max | units | src |
|---|----------|------|----------|---------|-----|-----|-------|-----|
| 1 | `groupName` | string | Name | `$"Group {groupCount++}"` | - | - | - | 18 |
| 2 | `scaleWithSim` | bool | Count scales with sim | `false` | - | - | - | 24 |
| 3 | `zoneCount` | i32 | Zones Count | `10` | 1 | 1000 | - | 32 |
| 4 | `zoneDensity` | f32 | Zones Density | `0.66` | 0.01 | 50 | `z/ku^2` | 42 |

## 3.7 `bibites[]` (spawn table)

Written by `BibiteSettings.SaveState` (`Here/SettingScripts/BibiteSettings.cs:163-192`), read by
`LoadState` (211-248). This is how a save/scenario defines which templates spawn and how many.

| JSON key | type | default | emitted | required on load | notes |
|---|---|---|---|---|---|
| `path` | string | - | always | **yes** (dereferenced, `BibiteSettings.cs:217`) | template file name as bundled in `templates/`, or absolute path when `isExternal` |
| `spawnPriority` | f32 | 1 | always | **yes** (dereferenced) | 0.01..100 |
| `minimum` | int | 10 | always | no | target count of this template |
| `randomizeGenes` | enum `RandomizeGenes` | `NormalMutations` | always | no | |
| `growthAtSpawn` | enum `GrowthAtSpawn` | `Adult` | always | no | |
| `tagging` | enum `Tagging` | `NoTagging` | always | no | pre-0.6.3 versions wrote numeric/bin values; current writes names |
| `spawnType` | enum `SpawnType` | `Continuous` | always | no | |
| `tag` | string | `""` | only `CustomTagging` | no | |
| `totalSpawned` | int | 0 | only `> 0` | no | runtime counter |
| `isExternal` | bool | false | only `true` | no | external (non-bundled) template path |
| `spawnZone` | int zoneID | none | when a zone selected | no | omitted = all zones; resolved against top-level zones |

`SaveForChampion`/`LoadForChampion` use a subset without `path`, `tagging`, `tag`,
`totalSpawned`, `isExternal`, `isChampion` (`BibiteSettings.cs:194-272`): `spawnPriority`,
`minimum`, `randomizeGenes`, `growthAtSpawn`, `spawnType`, `spawnZone?`.

Per-template fields and their UI defaults:

| # | JSON key | type | UI label | default | min | max | units | src |
|---|----------|------|----------|---------|-----|-----|-------|-----|
| 1 | `spawnZone` | int (zone id) | Spawn Zone | - | - | - | - | 22 |
| 2 | `tagging` | enum Tagging | Tagging | `NoTagging` | - | - | - | 29 |
| 3 | `customTag` | string | Tag | `""` | - | - | - | 38 |
| 4 | `spawnPriority` | f32 | Priority | `1` | 0.01 | 100 | - | 46 |
| 5 | `spawnType` | enum SpawnType | Spawn Logic | `Continuous` | - | - | - | 58 |
| 6 | `minimumNumber` | i32 | Target Count | `10` | 0 | 500 | - | 65 |
| 7 | `totalSpawned` | i32 | Spawned | `0` | 0 | 1000000 | - | 75 |
| 8 | `randomizeGenes` | enum RandomizeGenes | Mutate Genes | `NormalMutations` | - | - | - | 85 |
| 9 | `growthAtSpawn` | enum GrowthAtSpawn | Growth At Spawn | `Adult` | - | - | - | 94 |

## 3.8 `settingsChangers[]` and `challenge`

Documented in section 7 (animators and conditions). The loader only loads `settingsChangers` when
the file version is `>= 0.6.3a3`, yet the writer always emits them (`SerializationHelper.cs:510-514`).
Every save/scenario written by the installed 0.6.3.1 build satisfies that gate.

## 3.9 0.6.4a1-only scalar settings

Real 0.6.4a1 saves contain three extra top-level scalar settings absent from the decompiled
source: `shadeDistance` (f32 default 1.5), `shadeFadeDistance` (f32 default 1000.0),
`useNetworkingFeature` (bool default false) **[external, `catalog/scenario_settings_extra.json`;
observed in 0.6.4a1 saves]**. They must be preserved as unknown fields when importing/exporting
such a file (section 9), and ignored otherwise. Unknown scalar settings are ignored by
`JsonUtility` so writing them into a 0.6.3.1 save is harmless; we do not add them to saves for the
installed build.


# 4. `.bb8` genome / entity files and `.bb8template`

## 4.1 The four JSON shapes called `.bb8`

The extension `.bb8` is used for several related payloads; the shape is recognised by the loader,
not the extension:

| Shape | Written by | Top-level keys | Used as |
|---|---|---|---|
| A. Saved living bibite | `SaveSystem.SaveBibite` = `SerializeBibite` + `version` + `desc` (`Here/ManagementScripts/SaveSystem.cs:135-151,426-445`) | `transform`, `rb2d`, `genes`, `body`, `clock`, `brain`, `version`, `desc` | importable entity snapshot in `Bibites/`; the template selector also offers it as a genome source |
| B. Saved egg | `SaveSystem.SaveEgg` = `SerializeEgg` + `version` + `desc` (`SaveSystem.cs:153-169,447-465`) | `transform`, `rb2d`, `genes`, `egg`, `brain`, `version`, `desc` | importable egg snapshot |
| C. Save-zip member | `CreateSave` | same as A or B **without** `version`/`desc` (`SaveSystem.cs:286-299`) | world entities |
| D. Old exported bibite (0.6.3a3 era and older) | old `SaveBibite` | A plus extra sections, e.g. `growth`, `immuneSystem`, old body fields (`Sample.bb8` on disk) | readable: `LoadBibite` ignores sections it does not consume, `BibiteTemplate` ignores everything except `genes`/`brain`/`version` |

Real examples used throughout this section:

* `%TEMP%\rustibites_fmt\save_lol\bibites\bibite_0.bb8` (from `Savefiles\lol.zip`, scene version
  0.6.3a3) and `bibite_0.bb8` from `Savefiles\Autosaves\autosave_20261008004410.zip` (0.6.3.1) -
  identical key sets.
* `%TEMP%\rustibites_fmt\save_lol\eggs\egg_0.bb8`.
* `Bibites\Templates\Sample.bb8template`, `Bibites\Templates\Fibonacci turner.bb8template`
  (has `nodeAnchors`).
* `Sample.bb8` (exported with `FormatBibiteJSON=1`, indented, old schema).

The template loader `new BibiteTemplate(path)` for a `.bb8` **reads only**
`version`, `genes.genes.<name>`, `genes.gen` and `brain.Nodes`/`brain.Synapses`, and derives
`name` from the file name (`Here/Utility/BibiteTemplate.cs:63-146`). `body`, `clock`,
`transform`, `rb2d`, `desc`, gene metadata are ignored on that path. The full-entity loader
`LoadBibite` needs `transform`, `genes`, `body`, `clock`, `brain` (`SaveSystem.cs:467-505`).
So: **to be accepted as a genome template, a `.bb8` only needs `genes` + `brain`**; to be accepted
as a live entity it needs the whole A schema.

## 4.2 Genome object (`genes`)

`BibiteGenes.SaveState` (`Here/SimulationScripts.BibiteScripts/BibiteGenes.cs:533-590`),
`LoadState` (592-651). Used identically in A/B/C, in `.bb8template` (different shape: genes map
directly at top level, see 4.5), and in `speciesData.json` templates.

| JSON key | type | emitted | required | notes |
|---|---|---|---|---|
| `tag` | string | when non-empty | no | species tag written on spawn (`Tagging`); runtime tag registry is not saved |
| `speciesID` | long | when species assigned | no | FK into `speciesData.recordedSpecies[].speciesID`; unknown id -> re-speciation |
| `isReady` | bool | always | **yes** (except templates) | `LoadState` casts `(bool)state["isReady"]`; `ResumeBrain/ResumeBody` also gate on it |
| `gen` | int | always | **yes** | generation number; not clamped |
| `isMutant` | bool | only true | no | speciation marker |
| `isRad` / `isRain` / `isKiller` / `isSource` | bool | only true | no | easter-egg flags (`isRain` = transparent, etc.) |
| `parent1` / `parent2` | int | when parent object exists | no | `BibiteID.id` of parent, re-linked on save load only among bibite files (eggs excluded, corpses included; `SaveSystem.cs:815-866`) |
| `genes` | object | always | **yes** | 35 named f32 fields, see 4.6. Missing names fall back to the editor default (`BibiteGenes.cs:610-619`) and the array is clamped to per-gene min/max after load (`CapGenes`, 409-416) |

`isReady` on genes is used by hatching: `EggHatching.ResumeHatch` aborts an egg whose
`eggGene.IsReady`/`eggBrain.isReady` is false, converting the egg into a meat pile
(`Here/SimulationScripts.BibiteScripts/EggHatching.cs:136-152`).

## 4.3 Body, clock and egg sections

### `body` (living bibite only)

`BibiteBody.SaveState` (`Here/SimulationScripts.BibiteScripts/BibiteBody.cs:951-975`) writes fixed
sub-objects first, then every `[SerializeField]` field of the component.

| JSON key | type | required on load | source |
|---|---|---|---|
| `mouth` | object: `attackedLastFrame` bool, `biteProgress` f32, `bibitesBitten` int, `totalDamageDealt` f32, `totalMurders` int, `murderedArea` f32 | no | `BibiteMouth.cs:924` |
| `stomach` | object: `content` array (max 2) of `{material: string, amount: f32, averageChunkAmount: f32}` | no | `BibiteStomach.cs:349`, `StomachContent.cs:49` |
| `phero` | object: `progress` f32 | no | `BibitePheromoneOrgan.cs:91` |
| `health` | f32 | **yes** (`state["health"].ToObject<float>()`, `BibiteBody.cs:1029`) | |
| `energy` | f32 | **yes** (`BibiteBody.cs:1030`) | |
| `eggLayer` | object: `eggProgress` f32, `nEggsLaid` int, `children` int[] optional | no | `BibiteEggLayingOrgan.cs:238` |
| `control` | object: `totalTravel` f32 | no | `BibitePropulsion.cs:332` |
| `fatReservesAmount` | f32 | no | `BibiteFatOrgan` A mount |
| `id` | int | no | `BibiteID.id`; legacy fallback key `bibiteID` (`BibiteBody.cs:1025-1028`) |
| `d2Size` | f32 | effectively yes (growth is recomputed from it: `growth.growth = d2Size / SizeRatio^2`, `BibiteBody.cs:1031`) | `[SerializeField]` at `BibiteBody.cs:84-85` |
| `dying`, `dead` | bool | no | SerializeField `BibiteBody.cs:91-95`; corpse files are `dead:true` |
| `attackedDmg` | f32 | no | SerializeField `BibiteBody.cs:100-101` |
| `timesAttacked` | int | no | SerializeField `BibiteBody.cs:103-104` |
| `totalDamageSuffered` | f32 | no | SerializeField `BibiteBody.cs:106-107` |
| `brainTicksCount`, `visionLookupCount`, `visionSensingCount` | f32 | no | SerializeField `BibiteBody.cs:109-116` |
| `corpseEnergyOffset` | f32 | no | SerializeField `BibiteBody.cs:132-133` |

`body` is **required** for a live-bibite file (`SerializeBibite` always emits it; `LoadBibite`
calls `DeserializeObject(component2, (JObject)state["body"])` which throws on null).
Fields absent from a file keep the freshly spawned component values. Not saved (recomputed):
growth progress, organ states other than `fatReservesAmount`, grab joints, clock period, rad-tower
mutation doses, brain evaluation order, lineage links (re-linked from parent ids), species counts.

### `clock`

`InternalClock` serialized via `SerializeGeneralObject`
(`Here/SimulationScripts.BibiteScripts/InternalClock.cs:16-26`):

| key | type | notes |
|---|---|---|
| `tic` | int | 0/1 internal clock phase |
| `ticProgress` | f32 | seconds into the current clock period; period = gene `ClockSpeed` |
| `timeAlive` | f32 | seconds alive; age in hours = timeAlive/3600 |
| `chronoTime` | f32 | seconds since last `ClkReset>0.75`; used as "Minute" input /60 |

`clock` is required for living bibites (`SaveSystem.cs:488`). Eggs have no `clock`.

### `egg` (egg only)

`EggHatching.SaveState` (`Here/SimulationScripts.BibiteScripts/EggHatching.cs:235-264`):

| key | type | required | notes |
|---|---|---|---|
| `hatchProgress` | f32 | no | seconds hatched so far; hatching completes at gene `HatchTime` |
| `energy` | f32 | no | energy stored in egg; must exceed `totalRequiredEnergy` at hatch or the egg aborts |
| `id` | int | no | observed `0` in real saved eggs; the loader accepts 0 (ids are not re-randomised on load) |

Egg files have `genes`, `egg`, `brain`, `transform`, `rb2d` and no `body`/`clock`
(`SaveSystem.cs:447-465`); `brain` and `genes` are required by `LoadEgg`
(`SaveSystem.cs:507-532`).

### `transform` and `rb2d`

`SerializationHelper.SerializePosition` (`Here/ScriptHelpers/SerializationHelper.cs:221-229`):

```json
"transform": { "position": [x, y], "rotation": degrees, "scale": s },
"rb2d": { "px": x, "py": y, "vx": vx, "vy": vy, "r": degrees }
```

`SerializedTransform` field order is `position[], rotation, scale` (`Here/ScriptHelpers/SerializedTransform.cs:7-14`).
`rb2d` is optional (only if the object has a Rigidbody2D). On load, `transform` is applied first,
then `rb2d` (position and velocity) - so `rb2d` wins for position (`DeserializePosition`,
`SerializationHelper.cs:231-258`). A legacy `OldSerializedTransform` shape is accepted via a
fallback converter (line 240-251); its exact old field names are [external]/open (Q10).

## 4.4 `brain`

`NEATBrain` is serialized by `SerializeGeneralObject` (`Here/SimulationScripts.BibiteScripts/NEATBrain.cs:223-230`),
which emits its three `[SerializeField]` members:

```json
"brain": { "isReady": true, "Nodes": [ ... ], "Synapses": [ ... ] }
```

`isReady` must be `true` for `ResumeBrain` to initialise the brain (`NEATBrain.cs:360-371`);
a saved live bibite with `isReady:false` loads but does nothing.

### Node object

`NEATBrain.Node` is a struct serialized by Newtonsoft `JToken.FromObject` in save files, so its
members are: (order as emitted; fields first, then the read-only property)

| key | type | emitted | notes |
|---|---|---|---|
| `Type` | int enum `NodeType` | always | 0=Input, 1=Sigmoid, 2=Linear, 3=TanH, 4=Sine, 5=ReLu, 6=Gaussian, 7=Latch, 8=Differential, 9=Abs, 10=Mult, 11=Integrator, 12=Inhibitory, 13=SoftLatch |
| `baseActivation` | f32 | always | bias (semantics below); for `Input` always 0 |
| `TypeName` | string | always | redundant mirror of `Type`; ignored on load |
| `Index` | int | always | **must equal array position** (`FinalizeEditing` forces `Nodes[k].Index = k`; loaders also set it) |
| `Inov` | long | always | innovation number; base nodes = Index+1, hidden >= 49 (see 4.7) |
| `Desc` | string | always | input/output name for base nodes (checked/overwritten by `SetBaseNeuronsTypeAndDesc`), `"HiddenN"` for hidden |
| `Value` | f32 | always | last output value (runtime state) |
| `LastInput` | f32 | always | runtime state (used by Differential/Inhibitory/Latch/SoftLatch) |
| `LastOutput` | f32 | always | runtime state |
| `archetype` | int | save files only | read-only convenience property `NodeArchetype` (0 Input / 1 Output / 2 Hidden) recomputed from Index; **ignored on load**; template files omit it |
| (`NIn`, `NOut`) | - | never | `[NonSerialized]`, recomputed on load |

Templates (`.bb8template`, `speciesData.json`) use the reduced `Node.SaveForTemplate` shape:
`{Type, Index, Inov, Desc, baseActivation}` - no runtime state, no TypeName, no archetype
(`NEATBrain.cs:145-162`, verified in `Sample.bb8template`).

### Synapse object

`NEATBrain.Synaps` serialized by Newtonsoft: `{ "Inov": long, "NodeIn": int, "NodeOut": int, "Weight": f32, "En": bool }`
(`NEATBrain.cs:172-183`). `En=false` means disabled (still present in the array and mutated).
Load validity requirements (from `ComputeActiveNeurons` and `FinalizeEditing`):

* `NodeIn`/`NodeOut` must be valid array indices, otherwise the game throws while loading.
* Inputs may only be sources, outputs only sinks, but the loader does not hard-enforce it here;
  it enforces the no-floating-hidden rule: a hidden node with `NIn<1` or `NOut<1` is removed
  (with its synapses) during `ComputeActiveNeurons` (`NEATBrain.cs:488-495`), so such nodes in a
  file are silently dropped. Our exporter should never emit them.
* Duplicate `(NodeIn,NodeOut)` pairs are merged by weight when a brain is copied from a template
  (`CopySynaps`, `NEATBrain.cs:1011-1022`); native saved brains do not contain duplicates.

### Index layout and base node table

`NInputs = 33`, `NOutputs = 15` (`NEATBrain.cs:243-245`); indices 0-32 inputs, 33-47 outputs,
48+ hidden. Base nodes get their type/desc from `SetBaseNeuronsTypeAndDesc` (`NEATBrain.cs:1029-1056`):
all inputs are `Input`; all outputs are `Sigmoid` except the following:

| Index | Name (Outputs) | base Type | base Inov |
|---|---|---|---|
| 33 | Accelerate | TanH | 34 |
| 34 | Rotate | TanH | 35 |
| 35 | Herding | TanH | 36 |
| 36 | EggProduction | TanH | 37 |
| 38 | Want2Eat | TanH | 39 |
| 40 | Grab | TanH | 41 |
| 42 | PhereOut1 | ReLu | 43 |
| 43 | PhereOut2 | ReLu | 44 |
| 44 | PhereOut3 | ReLu | 45 |
| others (37 Want2Lay, 39 Digestion, 41 ClkReset, 45 Want2Grow, 46 Want2Heal, 47 Want2Attack) | - | Sigmoid | Index+1 |

Verified against a real template (`Sample.bb8template`: node 36 Type=3 (TanH) with
`baseActivation=0.2`, node 38 Type=3, node 40 Type=3, nodes 42-44 Type=5 (ReLu), rest of outputs
Type=1 (Sigmoid)). The `0.2` baseActivation on EggProduction is the 0.6a5 migration value
(`BrainUpdater.UpdateTo0_6_0_5`) and is what shipped/default templates carry once upgraded.

The base Inov rule is `Inov = Index + 1` for every base node; hidden nodes receive
`NEATBrain.nodeMaxInov++` (initially `NInputs+NOutputs+1 = 49`, `NEATBrain.cs:251`). The
`baseActivation` column above is the value a *newly created* base node starts with (0); per-file
values differ (e.g. real `Sample.bb8template` has EggProduction baseActivation 0.2, Want2Eat
1.265, Digestion -0.197, Want2Grow -5.36). For nodes created by mutation, the type-dependent
defaults are Mult=1, Inhibitory=1, SoftLatch=5, everything else 0
(`DefaultBaseActivationOfType`, `NEATBrain.cs:1083-1092`). Full list:

| Index | name (Desc) | base Type | base baseActivation | base Inov | role |
|-------|-------------|-----------|---------------------|----------|------|
| 0 | EnergyRatio | Input | 0 | 1 | input |
| 1 | Maturity | Input | 0 | 2 | input |
| 2 | LifeRatio | Input | 0 | 3 | input |
| 3 | Fullness | Input | 0 | 4 | input |
| 4 | Speed | Input | 0 | 5 | input |
| 5 | RotationSpeed | Input | 0 | 6 | input |
| 6 | IsGrabbing | Input | 0 | 7 | input |
| 7 | AttackedDamage | Input | 0 | 8 | input |
| 8 | EggStored | Input | 0 | 9 | input |
| 9 | BibiteCloseness | Input | 0 | 10 | input |
| 10 | BibiteAngle | Input | 0 | 11 | input |
| 11 | NBibites | Input | 0 | 12 | input |
| 12 | PlantCloseness | Input | 0 | 13 | input |
| 13 | PlantAngle | Input | 0 | 14 | input |
| 14 | NPlants | Input | 0 | 15 | input |
| 15 | MeatCloseness | Input | 0 | 16 | input |
| 16 | MeatAngle | Input | 0 | 17 | input |
| 17 | NMeats | Input | 0 | 18 | input |
| 18 | RedBibite | Input | 0 | 19 | input |
| 19 | GreenBibite | Input | 0 | 20 | input |
| 20 | BlueBibite | Input | 0 | 21 | input |
| 21 | Tic | Input | 0 | 22 | input |
| 22 | Minute | Input | 0 | 23 | input |
| 23 | TimeAlive | Input | 0 | 24 | input |
| 24 | PheroSense1 | Input | 0 | 25 | input |
| 25 | PheroSense2 | Input | 0 | 26 | input |
| 26 | PheroSense3 | Input | 0 | 27 | input |
| 27 | Phero1Angle | Input | 0 | 28 | input |
| 28 | Phero2Angle | Input | 0 | 29 | input |
| 29 | Phero3Angle | Input | 0 | 30 | input |
| 30 | Phero1Heading | Input | 0 | 31 | input |
| 31 | Phero2Heading | Input | 0 | 32 | input |
| 32 | Phero3Heading | Input | 0 | 33 | input |
| 33 | Accelerate | TanH | 0 | 34 | output |
| 34 | Rotate | TanH | 0 | 35 | output |
| 35 | Herding | TanH | 0 | 36 | output |
| 36 | EggProduction | TanH | 0 | 37 | output |
| 37 | Want2Lay | Sigmoid | 0 | 38 | output |
| 38 | Want2Eat | TanH | 0 | 39 | output |
| 39 | Digestion | Sigmoid | 0 | 40 | output |
| 40 | Grab | TanH | 0 | 41 | output |
| 41 | ClkReset | Sigmoid | 0 | 42 | output |
| 42 | PhereOut1 | ReLu | 0 | 43 | output |
| 43 | PhereOut2 | ReLu | 0 | 44 | output |
| 44 | PhereOut3 | ReLu | 0 | 45 | output |
| 45 | Want2Grow | Sigmoid | 0 | 46 | output |
| 46 | Want2Heal | Sigmoid | 0 | 47 | output |
| 47 | Want2Attack | Sigmoid | 0 | 48 | output |

### Node bias and activation semantics (needed to load state correctly)

`ExecuteNeuron` (`NEATBrain.cs:612-641`):

* Bias is added to the input for every type except `Input` and `Integrator`; `Mult` multiplies the
  input by `baseActivation`; `Inhibitory`/`SoftLatch` use it as a decay constant
  (`TypeHasBias`, `TypeUsesBiasDifferently`, lines 285-310).
* Clamp of the node output: `[-1024, +1024]`.
* Activation functions (all with f32 arithmetic; `period = brainTPS/simTPS` seconds):
  `Sigmoid = 1/(1+e^-x)`, `Linear = x`, `TanH = tanh(x)`, `Sine = sin(x)`, `ReLu = max(x,0)`,
  `Gaussian = 1/(1+x^2)`, `Latch = x>=1 ? 1 : x<=0 ? 0 : LastOutput`,
  `Differential = (x - LastInput)/period`, `Abs = |x|`, `Mult = x`,
  `Integrator = LastOutput + x*period`,
  `Inhibitory = x - LastInput + LastOutput*exp(-baseActivation*period)`,
  `SoftLatch = SoftLatchFunc(node,x)` (`NEATBrain.cs:1105-1133`).
* `ComputeActiveNeurons` clamps `SoftLatch` baseActivation to `[0.1,100]`; `Inhibitory` mutations
  clamp to `[0.0001,10]`; `Mult`/`Inhibitory` re-types clamp `abs` to `[0.1,1]`
  (`NEATBrain.cs:475-478,916-961`).
* Hidden-node mutation order/processing is not needed to load; only state fields are.

## 4.5 `.bb8template` (species template)

Written by `BibiteTemplate.SaveState` (`Here/Utility/BibiteTemplate.cs:227-271`), read by the
constructor `LoadState` path (132-146, 324-370). This is the canonical "genome-only" format and is
what `speciesData.json` embeds.

| JSON key | type | emitted | required | notes |
|---|---|---|---|---|
| `name` | string | always | no | display name; `.bb8` loader derives it from the file name (`BibiteTemplate.cs:77`) |
| `speciesName` | string | when non-null | no | for `.bb8template` it is **recomputed** as `name.ToSpeciesFormat()` on load (`BibiteTemplate.cs:140`); `ToSpeciesFormat` = capitalise first word + lower-case second (or repeat first), `Here/ScriptHelpers/StringExtend.cs:13-17` |
| `description` | string | when non-null | no | user text |
| `isOfficial` | bool | only true | no | shipped templates flag; not serialized by `SerializeGeneralObject`, added explicitly (`BibiteTemplate.cs:266-269`) |
| `generation` | int | always | no | generation of the specimen the template came from |
| `version` | string | always | **yes** | game version that wrote it; gates gene/brain migrations; `.bb8` path requires it (`Version.CanParse`, `BibiteTemplate.cs:61`) |
| `nodes` | Node[] reduced shape | always | **yes** (dereferenced) | `{Type,Index,Inov,Desc,baseActivation}` |
| `synapses` | Synaps[] | always | **yes** (dereferenced) | `{Inov,NodeIn,NodeOut,Weight,En}` |
| `genes` | object | always | **yes** | 35 named f32 (current); for version `< 0.6.0.1` it may be a **34-element array** of the old order and is migrated (see 4.7) |
| `nodeAnchors` | object | when non-empty | no | brain-editor positions: `{"<Inov>": {"x": f32, "y": f32}, ...}`; keyed by innovation number, follows nodes through migrations (`BrainUpdater.cs:174-189`); a real example is `Bibites\Templates\Fibonacci turner.bb8template` |

`BibiteTemplate.SaveStateForSpecies` (used inside `speciesData.json`) writes only
`version`, `nodes`, `synapses`, `genes` (+ optional `nodeAnchors`) (`BibiteTemplate.cs:280-322`);
no name/description/generation/isOfficial.

## 4.6 The 35 genes: order, defaults, ranges

Order is the `BibiteGenes.Genes` enum (`Here/SimulationScripts.BibiteScripts/BibiteGenes.cs:16-53`),
which is also the order of keys in save files. Defaults/limits are the `BibiteEditorSettings`
`GeneSetting` literals (`Here/SettingScripts/BibiteEditorSettings.cs`), used when a gene is missing
from a file (`BibiteGenes.cs:610-619`, `BibiteTemplate.cs:103-120`). Defaults are normative for
initial genomes and for missing-gene fallback.

| idx | gene key | UI label | default | min | max | units | precision | relative | src |
|-----|----------|----------|---------|-----|-----|-------|-----------|----------|-----|
| 0 | `LayTime` | Lay Time | `15` | 1 | 51 | `s` | 1 |  | 189 |
| 1 | `BroodTime` | Brood Time | `20` | 1 | 101 | `s` | 1 |  | 204 |
| 2 | `HatchTime` | Hatch Time | `10` | 1 | 101 | `s` | 1 |  | 219 |
| 3 | `SizeRatio` | Size Ratio | `1` | 0.05 | 2.55 | - | 2 | yes | 91 |
| 4 | `SpeedRatio` | Metabolism Speed | `0.75` | 0.000001 | 2.5 | `SPD` | 2 | yes | 108 |
| 5 | `ColorR` | Red Color | `0.1` | 0 | 1 | - | 3 |  | 11 |
| 6 | `ColorG` | Green Color | `0.1` | 0 | 1 | - | 3 |  | 27 |
| 7 | `ColorB` | Blue Color | `0.1` | 0 | 1 | - | 3 |  | 43 |
| 8 | `MutationAmountSigma` | Gene Mutation Variance | `0.05` | 0 | 0.25 | `%` | 2 |  | 140 |
| 9 | `AverageMutationNumber` | Average Gene Mutations | `2` | 0 | 30 | - | 1 |  | 125 |
| 10 | `BrainMutationSigma` | Brain Mutation Variance | `0.1` | 0 | 0.25 | `%` | 2 |  | 172 |
| 11 | `BrainAverageMutation` | Average Brain Mutations | `1` | 0 | 10 | - | 1 |  | 157 |
| 12 | `ViewAngle` | View Angle | `75` | 0 | 360 | `deg` | 0 |  | 249 |
| 13 | `ViewRadius` | View Radius | `180` | 0 | 200 | `u` | 1 |  | 234 |
| 14 | `ClockSpeed` | Clock Period | `1` | 0.1 | 10.1 | `s` | 2 |  | 264 |
| 15 | `PheroSense` | Pheromones Sensing Radius | `150` | 0 | 500 | `u` | 0 |  | 279 |
| 16 | `Diet` | Diet | `0.2` | 0 | 1 | - | 2 |  | 75 |
| 17 | `HerdSeparationWeight` | Herd Separation Weight | `1` | 0 | 4 | - | 2 |  | 294 |
| 18 | `HerdAlignmentWeight` | Herd Alignment Weight | `1.5` | 0 | 4 | - | 2 |  | 310 |
| 19 | `HerdCohesionWeight` | Herd Cohesion Weight | `0.75` | 0 | 4 | - | 2 |  | 326 |
| 20 | `HerdVelocityWeight` | Herd Velocity Weight | `0.25` | 0 | 1 | - | 2 |  | 342 |
| 21 | `HerdSeparationDistance` | Herd Separation Distance | `35` | 0 | 100 | `u` | 1 |  | 358 |
| 22 | `GrowthScale` | Growth Scale Factor | `0.15` | 0 | 2 | - | 3 | yes | 373 |
| 23 | `GrowthMaturityFactor` | Growth Maturity Factor | `2` | 0 | 10 | - | 2 |  | 390 |
| 24 | `GrowthMaturityExponent` | Growth Maturity Exponent | `1` | 0 | 10 | - | 2 |  | 406 |
| 25 | `EyeOffset` | Eye Hue Offset | `0.5` | 0 | 1 | - | 2 |  | 59 |
| 26 | `StomachWAG` | Stomach WAG | `2` | -1 | 9 | - | 3 |  | 422 |
| 27 | `WombWAG` | Egg Organ WAG | `1` | -1 | 9 | - | 3 |  | 438 |
| 28 | `FatWAG` | Fat Organ WAG | `1` | -1 | 9 | - | 3 |  | 454 |
| 29 | `ArmorWAG` | Armor WAG | `0.15` | -1 | 9 | - | 3 |  | 470 |
| 30 | `ThroatWAG` | Throat WAG | `1` | -1 | 9 | - | 3 |  | 486 |
| 31 | `MouthMusclesWAG` | Jaw Muscles WAG | `1` | -1 | 9 | - | 3 |  | 502 |
| 32 | `MoveMusclesWAG` | Arm Muscles WAG | `1` | -1 | 9 | - | 3 |  | 518 |
| 33 | `FatStorageThreshold` | Fat Storage Threshold | `0.5` | 0 | 1 | - | 3 |  | 534 |
| 34 | `FatStorageDeadband` | Fat Storage Deadband | `0.25` | 0 | 0.5 | - | 3 |  | 550 |

"relative" = `isPurelyRelative`, i.e. mutation sigma is scaled by the gene's own value
(`BibiteGenes.cs:401-407`). All genes are clamped to [min,max] on load (`CapGenes`).

## 4.7 Version migrations for genomes/brains/templates

`Version` parsing quirk that matters for every threshold below: `new Version(0,6,0,15)` invokes
the 4-parameter constructor `Version(int v0, int v1, int v2, int a = 0)` where the 4th argument is
the **alpha** component, so `Version(0,6,0,15)` means **0.6a15**, not `0.6.0.15`
(`Here/Utility/Version.cs:54-61`; `Present` comes from `Application.version`). Release strings
like `0.6.3.1` parse as four numeric components (`Version.cs:90-116`); an alpha sorts below the
same release number (`Version.cs:127-212`). Version comparisons in the code are therefore
"alpha-era" comparisons: `0.6a5 > 0.6.3.1` is **false**.

### Genes

`BibiteUpdater.UpdateGenesToPresentVersion` (`Here/ScriptHelpers/BibiteUpdater.cs:31-103`):

1. `from < 0.6a7` and the gene remap is needed: overwrite gene slots 28-33 with the default
   values of `EyeOffset`, `StomachWAG`, `WombWAG`, `FatWAG`, `FatStorageThreshold`,
   `FatStorageDeadband` (lines 33-41). This is a game bug preserved for fidelity (note: slots are
   the *current* indices but the defaults are the new genes' - the external reference calls this
   "QUIRK reproduced on purpose"; kept because the game does it).
2. `from < 0.6a15`: Strength/Defence become organ-area genes:
   `strength = genesJson["Strength"] ?? 5`, `defence = genesJson["Defence"] ?? 0`
   (for `from < 0.6a1` and not a saved entity, take old array slots 8/9 instead);
   then either remap the old 34-gene array through `to06a15GenesMap`
   (`{0,1,2,3,4,5,6,7,-1,-1,8,9,10,11,12,13,14,15,16,-1,17,18,19,20,21,22,23,24,25,26,27,28,33,34}`,
   lines 11-17) or keep the array; then multiply slots 26,27,28 (Stomach/Womb/Fat WAG) by 2.5,
   zero slots 29,30,31,32 (Armor, Throat, MouthMuscles, MoveMuscles), compute
   `total = sum(max(0, slots 26..32))`, and set
   `MouthMuscles = total*min(0.01,(strength/35)^2)/rest`,
   `Armor = total*(defence/35)^2/rest`, `MoveMuscles = total*0.05/rest`,
   `Throat = total*0.1/rest`, `rest = 1 - (those four)` (lines 66-100).
   All arithmetic is f32 like the game.

### Brain

`BrainUpdater.VersionChanges` (`Here/ScriptHelpers/BrainUpdater.cs:12-104`), applied for every
change **newer than the file version**. The list below is in source order (newest first). The
"position" values are the `NodeChanges.Position` literals from source; do not reinterpret them -
port `UpdateElementsFromVersion` literally (index map computed newest-first, node/synapse edits
applied oldest-first).

| threshold (Version(...) = alpha) | changes |
|---|---|
| `0.6.2a1` (0,6,2,1) | add 1 input at position 5 |
| `0.6a18` (0,6,0,18) | remove 1 input at 32; remove 1 output at position 15 (the last of the then-16 outputs) |
| `0.6a16` | node index 38 (Want2Eat) becomes `TanH`; `baseActivation = 0.28*old + 0.528`; every weight into node 38 is multiplied by 0.28 (`UpdateTo0_6_0_16`) |
| `0.6a15` | node 38 baseActivation 0 becomes 2.5 (`UpdateTo0_6_0_15`) |
| `0.6a13` | remove input 0 ("Constant"): enabled synapses from node 0 fold their weight into the target's `baseActivation`; then all synapses from node 0 get `NodeIn = Random.Range(1,14)` and `En=false`; `Mult` nodes with activation 0 get 1 (`UpdateTo0_6_0_13`) |
| `0.6a5` | add 1 input at position 8; add 1 output at position 3; node 37 (EggProduction) baseActivation = 0.2; synapses with `Inov==0` get an innovation from the (in,out) dictionary starting at 1 (`UpdateTo0_6_0_5`) |
| `0.4a3` | add 3 inputs at position 28, add 1 input at position 4 |
| `0.3` | no node changes (version floor) |

Mechanics of applying changes (`UpdateElementsFromVersion`, `BrainUpdater.cs:203-328`):
the list of changes newer than the file version is applied **newest-first for the index map** (to
compute `indexMap`: old index -> new index or -1) and **oldest-first for the node/synapse edits**.
Hidden-node indices are shifted by net base-node count change; synapses whose endpoints map to -1
are dropped; afterwards every node gets `Index = arrayPosition`, base nodes get `Inov = Index+1`,
hidden nodes keep their innovation numbers plus the net shift; then
`SetBaseNeuronsTypeAndDesc` rewrites base node types/descriptions; `nodeAnchors` are remapped by
matching innovation numbers. Final sanity check: resulting input/output counts must equal 33/15,
otherwise the game shows an error popup (line 290-293). The 0.6a13 step is **non-deterministic**
(`Random.Range` for disabled synapse inputs); any tool comparing our output with the game's own
upgrade must allow that single field to differ [external; confirmed in `UpdateTo0_6_0_13`].

### Which versions have no migrations

Any file with version >= 0.6.2a1 is untouched by brain migrations, and genes are untouched from
0.6a15 on. Since `0.6.3a3`, `0.6.3.1`, `0.6.3.3`, `0.6.4a1` all compare greater than these
alphas, **current files never trigger gene/brain migrations**. (Careful: versions like `0.6.2`
compare greater than `0.6.2a1` because alpha=0 sorts above alpha>0, `Version.cs:161-165`.)

### Innovation counters and template instantiation

* `NEATBrain.nodeMaxInov` starts at `NInputs+NOutputs+1 = 49`; `connectionMaxInov` starts at 1
  (`NEATBrain.cs:251-253`). Saved `speciesData.json` persists both counters; they must be greater
  than any innovation in use.
* `BibiteTemplate.AlignInnovationsWithSim` (`Here/Utility/BibiteTemplate.cs:372-395`) gives each
  hidden node a fresh `nodeMaxInov++` and each synapse a fresh `connectionMaxInov++` when a
  template is instantiated in a world; `nodeAnchors` are remapped by old->new innovation (the
  method builds a translated dictionary but the code then discards it - another game quirk; in
  practice anchors are recomputed by editor).
* For a .bb8 genome import the template loader leaves inovs as in the file until spawn time.


# 5. Save bundle (`.zip`)

## 5.1 Location, naming, lifecycle

| Kind | Path | Producer |
|---|---|---|
| Manual save | `Savefiles\<name>.zip` | `SaveController.SaveWorld` (`Here/ManagementScripts/SaveController.cs:183-190`) |
| Quick save / quick load | `Savefiles\quick.zip` | `SaveController.QuickSave/QuickLoad` (`SaveController.cs:171-181`) |
| Autosave | `Savefiles\Autosaves\autosave_yyyyMMddHHmmss.zip` | `SaveController.AutoSave` (`SaveController.cs:214-247`); timestamp local time; one save per `AutoSavePeriod` minutes (default 10, `UserSettings.cs:35-40`), not during challenges (`SaveController.cs:235`) |
| Autosave retention | keeps newest `AutoSaveMaxFiles` (default 25, `UserSettings.cs:50-59`) by `LastWriteTime` | `CleanUpAutosaveFolder` (`SaveController.cs:274-294`) |
| Last save (Continue) | newest `*.zip` in `Savefiles` + `Autosaves` | `GetLastSave` (`SaveController.cs:305-310`) |
| 0.6.4a1 folder world | `<name>/sim.info` + `saves\_00.zip` + `saves\autosaves\*.zip` | **not in decompiled source; unsupported** (Lead decision); loader must return `Unsupported` with a clear message |

Saving is destructive-then-write in the game (delete existing file, `ZipArchiveMode.Create`), not
atomic (`SaveSystem.cs:271-275`). Our writer should write to a temp file and rename.

`SaveController.SavePath = Application.persistentDataPath + "/Savefiles"` (`SaveController.cs:51`);
`AutoSavePath = SavePath + "/Autosaves"` (53). Real saves can be truncated (one real file has a
valid `PK` header but is unreadable); our importer must fail cleanly, never panic.

## 5.2 Bundle layout (write order = game's order)

`CreateSave` (`Here/ManagementScripts/SaveSystem.cs:176-303`):

| # | Entry | Content | Required by loader | Notes |
|---|---|---|---|---|
| 1 | `settings.bb8settings` | save settings (84 keys, section 3) | **yes** (`GetSettingsOfSave` dereferences null -> exception) | fallback name `settings.json` accepted (`SaveSystem.cs:988-991`) |
| 2 | `speciesData.json` | species lineage DB (section 5.9) | no (only if version >= 0.6a5; omitted => species recreated) | |
| 3 | `scene.bb8scene` | scene metadata + towers (5.3) | **yes**; its `version` is the file version | fallback `scene.json` |
| 4 | `vars.bb8scene` | `{"towerMaxID": ulong}` (5.6) | no | |
| 5 | `pellets.bb8scene` | pellets grouped by zone (5.4) | no | if absent, legacy `scene.pellets` is used |
| 6 | `pheromones.bb8scene` | pheromone spots (5.5) | no | if absent, legacy `scene.pheromones` |
| 7 | `data.bin` | statistics history binary (5.8) | no | if absent, stats restart empty |
| 8 | `img.png` | 400x400 screenshot banner | no | `SceneScreenShotHandler.SaveScreenshotToZip` |
| 9 | `bibites/bibite_N.bb8` | living bibites **and corpses** | no | N arbitrary, gaps allowed; order irrelevant but parents before children is convenient |
| 10 | `eggs/egg_N.bb8` | eggs | no | |
| 11 | `templates/<file>` | bundled non-default templates referenced by `settings.bibites[].path` (`.bb8` or `.bb8template`) | no | on load, missing local templates are written into the user's `Bibites/` or `Bibites/Templates/` (`CheckTemplatesOfArchive`, `SaveSystem.cs:914-936`); files the user already has are not overwritten |

JSON entries are compact UTF-8-with-BOM (section 1.1); `data.bin` is raw little-endian bytes.
Zip entry order is not used by the loader. The writer uses `Path.Combine` so real saves contain
`bibites\bibite_0.bb8`; the loader matches `StartsWith("bibites")`/`StartsWith("egg")`
(`SaveSystem.cs:794-806`). We write `/`.

### 5.2.1 Entity entries inside a save (version injection and derived fields)

`bibites/*.bb8` and `eggs/*.bb8` **do not carry a top-level `version`** (unlike standalone `.bb8`
templates/entities); the save version lives in `scene.bb8scene`. A reader must therefore inject the
scene version before running the normal entity migrations (`bibites-io` does this in
`bundle::parse_bibite`/`parse_egg`). Save entity entries also omit derived state that the game
recomputes on load:

* Bibite `growth` is derived from `body.d2Size / SizeRatio²` (the gene comes from `genes.genes`).
* Egg `hatchTime` = `HatchTime` gene, `totalRequiredEnergy` = `egg_required_energy(bodyEnergyAtBirth,
  brainUpkeep, SizeRatio)` (`organs.rs`), `sizeGene` = `d1_size_at_birth`, `birthOrientation` =
  `-Vec2::Y`, `isHatching` = `hatchProgress > 0`; only `egg.hatchProgress`/`egg.energy`/`egg.id` are
  stored (`bibites-io` reconstructs these exactly from the genome + effective settings).
* Runtime ids that the game writes as raw ints (`body.id`, `eggLayer.children[]`, `genes.parent1/2`,
  `egg.id`) are kept by `bibites-io` as side vectors on `SaveBundle` (`bibite_ids`,
  `bibite_children`, `bibite_parents`, `egg_ids`) because core ids are generational handles and
  fabricating one from a game raw id is explicitly forbidden (Lead decision 2026-10-29). The World
  loader must build a **raw-id → new `BibiteId` map from spawn order** (the `bibites` vector
  preserves file order; `SaveBundle.bibite_ids[i]` is the game id of `bibites[i]`) and use it when
  restoring `GenomeMeta.parent1/2`, egg-layer children and species lineage links. No core API
  changes/helpers are needed.
* `BibiteSave.rot` / `EggSave.rot` are the game **`transform.rotation.z` in degrees** (not radians);
  core `spawn_loaded`/`to_save` convert deg↔rad at the store boundary, io passes the field through
  unchanged in both directions (do not convert in io; `bibites-io` reads/writes it raw).
* `transform.scale` and `rb2d` angular velocity are derived at runtime; they are not modelled by
  `BibiteSave`/`EggSave` and are re-emitted canonically on write.

## 5.3 `scene.bb8scene`

Written by `CreateSave` (`SaveSystem.cs:178-269`), read by `LoadScene` (589-688). Fields:

| JSON key | type | required | notes |
|---|---|---|---|
| `version` | string | **yes** | `Application.version` of the writing build (`SaveSystem.cs:184`); **the only version used by the loader** (`GetVersionOfFile(sceneOfSave)`, 719). Missing/unparseable -> error popup + abort load. Must be parseable by `Version` regex `^[0-9]+(\.[0-9]+)(\.[0-9]+)?(\.[0-9]+)?([aA][0-9]+)?$` (`Here/Utility/Version.cs:256-259`) |
| `nPellets` | int | no | display only (total pellets incl. free) |
| `nBibites` | int | no | display only = living bibites + eggs (corpses excluded) |
| `colorSelectors` | array | no | Color Killer list (5.3.1); empty array is written |
| `pheroTowers` | array | no | 5.3.2 |
| `radTowers` | array | no | 5.3.2 |
| `simulatedTime` | f64 | no | `TimeKeeper.simulatedTime` (double, seconds; `OneUseScripts/TimeKeeper.cs:17`); loaded as f32 (`SaveSystem.cs:684-687`) |
| `gameName` | string | no | read into `SimulationManager.gameName` (`SaveSystem.cs:720-723`); current writer does **not** emit it (maybe from newer builds) |
| `pellets` | array | legacy only | ignored when `pellets.bb8scene` exists; legacy scene-embedded pellets are loaded into zone[0] (`SaveSystem.cs:637-648`) |
| `pheromones` | array | legacy only | legacy fallback (672-683) |
| `connections`, `pelletSpawner` | - | legacy | pre-0.5a4 saves; `pelletSpawner.spawnCenters` used to synthesise zones on load (`SaveSystem.cs:757-779`) |

Order emitted by the game: `version, nPellets, nBibites, colorSelectors, pheroTowers, radTowers, simulatedTime`
(verified real file).

### 5.3.1 `colorSelectors[]` (Color Killer)

`ColorKiller.SaveState` (`Here/SimulationScripts/ColorKiller.cs:258-276`); all keys except
`followingZone` are **required on load** (`LoadState` dereferences each):

| key | type | notes |
|---|---|---|
| `transform` | SerializedTransform | `{position:[x,y], rotation, scale}` (`SerializePosition`) |
| `rb2d` | object | emitted if the GameObject has a Rigidbody2D (ColorKiller: yes, real file shows it) |
| `hue` | f32 | 0..1 |
| `tolerance` | f32 | 0..1 (circular hue distance) |
| `bodyOrEye` | bool | test body or eye colour |
| `anyOrMost` | bool | any match vs most-of-radius |
| `radius` | f32 | u |
| `period` | f32 | seconds between kills |
| `progress` | f32 | seconds into the current period (runtime state) |
| `followingZone` | int zoneID | optional; resolves against top-level zones |

Real example from `autosave_20261008004410.zip`:
`{"transform":{"position":[500.0,500.0],"rotation":0.0,"scale":1.0},"hue":0.3,"tolerance":0.2,"bodyOrEye":false,"anyOrMost":true,"radius":500.0,"period":45.0,"progress":29.0162334}`.

### 5.3.2 `pheroTowers[]` / `radTowers[]` (EffectTower subclasses)

Base `EffectTower.SaveState` (`Here/SimulationScripts/EffectTower.cs:122-140`) emits
`attachedZone?`, `id`, `x`, `y`; subclasses append their fields:

| object | keys | source |
|---|---|---|
| `radTowers[]` | `attachedZone?`, `id` (ulong), `x`, `y`, `count` (f32), `intensity` (f32), `radius` (f32), `taper` (f32) | `RadTower.cs:110-124`; defaults: count 0.001 N/s, intensity 0.001 (1/s? UI %/s with factor 100), radius min(250, S/2) (default 250, clamped to S/2), taper 1; set by `InitTower` (`RadTower.cs:88-100`) |
| `pheroTowers[]` | `attachedZone?`, `id` (ulong), `x`, `y`, `r`, `g`, `b` (f32, 0..100), `dir` (f32, radians) | `PheroTower.cs:95-109`; defaults all 0 |

`attachedZone` is a zoneID that must resolve against **top-level zones**; `id` must be <=
`vars.towerMaxID`. Real example from the current autosave:
`{"id":2,"x":-200.0,"y":-300.0,"r":5.0,"g":1.0,"b":0.5,"dir":1.2}` - note no `rb2d` (towers are
moved only while placing), and `id`/`x`,`y` order matches the base writer.

## 5.4 `pellets.bb8scene`

Written by `CreateSave` (`SaveSystem.cs:201-228`), read by `GetPelletsOfSave` (998-1007) and
`LoadScene` (649-671). Shape:

```json
{"pellets":[
  {"zone":"free pellets","pellets":[ {"transform":...,"rb2d":...,"pellet":{"material":"Plant","amount":12.5},"matterDecay":{"timeAlive":3.5,"rotAmount":0.1}} , ...]},
  {"zone":"<zone name>","pellets":[ ... ]}
]}
```

* Group list: first the "free pellets" bucket (`WorldObjectsSpawner.freePelletHolder`), one group
  per `ZoneManager.zones` in order (`SaveSystem.cs:204-227`). Empty groups are written
  (real saves can contain a group with `pellets: []`).
* Pellet object (`SerializePellet`, `SaveSystem.cs:395-409`): `transform` (required),
  `rb2d` (optional), `pellet` object (`MatterPellet.SaveState`, `Here/SimulationScripts/MatterPellet.cs:260-271`)
  = `{material: string, amount: f32}`. `material` must be a physical material (`Plant` or `Meat`;
  `FindMaterial`, `MatterMaterialManager.cs:94-97`); `amount` is u^2 (= energy / energyDensity).
* `matterDecay` (optional; only for materials with decay enabled, i.e. Meat by default) is
  `{timeAlive: f32, rotAmount: f32}` (`MatterDecayProcessor.TryGetState/TrySetState`,
  `Here/SimulationScripts/MatterDecayProcessor.cs:264-288`); if present, **both** keys are read.
  Legacy key `meatRot` is also accepted (`SaveSystem.cs:541`).
* On load, only `transform.position`, `pellet.material`, `pellet.amount`, `matterDecay` are used;
  `rb2d` velocity is ignored (pellets start at rest). Pellets are assigned to a zone by **matching
  `zone` against `zoneName`**; unmatched/absent zone becomes a free pellet (`SaveSystem.cs:649-670`).
  Then `zone.ResetList()` rebuilds the zone pellet lists.

## 5.5 `pheromones.bb8scene`

`{"pheromones":[{"transform":...,"rb2d":?,"phero":{...}}]}` (`SerializePheromones`,
`SaveSystem.cs:381-393`; `LoadPheromone`, 558-566). `phero` is
`PheromoneSpot` serialized via `SerializeGeneralObject` (`Here/SimulationScripts/PheromoneSpot.cs:11-30`),
which yields fields in declaration order: `heading` (a `Vector2`!), `Rstrength`, `Gstrength`,
`Bstrength`, `nR`, `nG`, `nB`. Because `Vector2`'s `x`/`y` are public **fields without
`[SerializeField]`**, `SerializeGeneralObject` recurses into it and emits **`{}`** - the heading is
lost on save (real files show `"heading":{}`). Load restores `heading = default(Vector2)`.
Pheromone spots are culled at autosave when outside bounds if shading is on
(`ZoneManager.cs:224-241`); the save itself contains all live spots.

## 5.6 `vars.bb8scene`

`{"towerMaxID": <ulong>}` written from `EffectTower.maxID` (`SaveSystem.cs:178-183`), read if
present (`LoadScene`, 606-609). It must be >= the largest tower `id` in the scene. If absent,
`EffectTower.Start` assigns `++maxID` to towers with id 0 as they load.

## 5.7 `bibites/*.bb8` and `eggs/*.bb8`

Schemas are in section 4 (shapes C). File names are `bibite_<n>.bb8` / `egg_<n>.bb8` with arbitrary
`n` (the game uses the enumeration index of the spawn list at save time; gaps are legal). The loader
does not use the file name; it loads every entry whose `FullName` starts with `bibites`/`egg` and
re-links ids afterwards:

* `eggLayer.children` is re-linked only to bibites/eggs present in the same save, skipping id 0
  (`SaveSystem.cs:815-849`).
* `genes.parent1`/`parent2` are re-linked **only among bibite files** (corpses included, eggs
  excluded) (`SaveSystem.cs:836-866`).

## 5.8 `data.bin` (statistics history)

`SaveSystem` builds the file from a `SaveableBinStack` with exactly four elements, in this order
(`Here/ManagementScripts/SaveSystem.cs:51-63`):

| # | Element | Version gate (don't load before) | Payload header | Source |
|---|---|---|---|---|
| 0 | `DataLogger.Instance` | none | f32 `progress`, i32 `nMinutes`, then a nested stack of 8 stream groups | `Here/Utility/DataLogger.cs:411-443` |
| 1 | `BibiteTracker.instance` | none | i32 `nBirth`, i32 `nDeath`, then a nested stack of 1 (`deathAgesQuantiles` = 3 GK sub-estimators); the 2 ints only exist for version >= 0.6a9 | `Here/ManagementScripts/BibiteTracker.cs:180-213` |
| 2 | `GlobalLineageManager.Instance` | **0.6a15** (`SaveableBinElement` gate, `SaveSystem.cs:59`) | repeated `{i64 speciesID, i32 byteLen, LogLikePresenceArray<Species>}` | `Here/ManagementScripts/GlobalLineageManager.cs:450-488` |
| 3 | `ZoneManager.instance` | **0.6a10** (`SaveSystem.cs:60`) | repeated `{i32 zoneID, i32 byteLen, LogLikePresenceArray<Zone>}` (zoneID = `zones[].id`, i.e. array index after renumbering) | `Here/SimulationScripts/ZoneManager.cs:294-336` |

Framing (`SaveableBinStack`, `Here/ScriptHelpers/SaveableBinStack.cs:8-52`): a stack is simply
`[i32 byteLength][payloadBytes]` per element, concatenated. There is **no magic, version, or
checksum**; a zero-length element is dangerous because several loaders treat `nBytes < 1` as "use
the rest of the file" (`SaveableBinStack.LoadStateBin:34-51`, `DataLogger.LoadStateBin:429-443`,
`BibiteTracker.LoadStateBin:197-213`). All integers/floats are little-endian .NET `BitConverter`.

The nested stream payloads (parallel data streams, gene buckets, GK quantiles, LogLike presence
arrays) are a sizeable binary spec of their own:

* Verified in source and matching the external verified notes: element framing + headers above,
  parallel-stream group layout `i32 nData | u8 width | u8 nScales | f32[width*nData] planar |
  u8[nScales-1] pushCounters | u8[nScales] nDataAtScale` (`Here/Utility/DataParallelStreamGroup.cs`,
  `Here/Utility/DataStreamGroup.cs`), gene-bucket group
  `i32 nPoints | i32 nData | i32 20 | i32 nGene | nPoints*(nGene*(f32 min, f32 max, u16[20] counts)) |
  u8[6] push | u8[7] nDataAtScale` (`Here/Utility/GenesBucketsStreamGroup.cs`), GK estimator
  `i32 n | i16 tupleCount | f32 value[n] | i16 gap[n] | i16 delta[n]`
  (`Here/Utility/GKQuantileEstimator.cs`), species/zone presence arrays
  (`Here/Utility/LogLikePresenceArray.cs`, `LogLikeSpeciesDataArray.cs`, `LogLikeZoneDataArray.cs`,
  `SpeciesDataPoint.cs`, `ZoneDataPoint.cs`).
* **[external]** exact stream set/config table (widths, scales, per-scale feed ratios, per-point
  field encodings, and the "declared GK length > written bytes -> zero padding" quirk) is
  documented in `BibitesInfo/BibitesAPI/docs/SAVE_FORMAT.md` section 7 and was verified there
  against real saves. It is not re-derived here.

**M1 strategy (mandatory):** data.bin is statistics-only and optional for loading. For an
imported save, keep the raw `data.bin` bytes as an opaque sidecar and re-emit them unchanged when
the source version >= 0.6.2a1 (no migration is applied to it by the game either). For a save we
create from scratch, omit `data.bin` (the game starts with empty statistics) or emit the documented
minimal valid blob once the stream encoders are implemented. Never attempt to re-encode an
imported `data.bin` by parsing it in M1.

## 5.9 `speciesData.json`

Written from `GlobalLineageManager.SaveState` (`Here/ManagementScripts/GlobalLineageManager.cs:361-379`),
read by `LoadState` (381-448). Shape:

| key | type | notes |
|---|---|---|
| `recordedSpecies` | array of species | required for useful lineage; loader iterates `state["recordedSpecies"]` |
| `maxEnergy` | f32 | historical max summed active-species energy |
| `maxCount` | f32 (loaded as int in code, `GlobalLineageManager.cs:402-405`; real values are whole numbers) | historical max summed count |
| `logCounter` | int | number of logging ticks |
| `nRecords` | int | capped at 10000 (`GlobalLineageManager.cs:81-96`) |
| `usedSpanText` | object | runtime UI handle serialization, always `{}` in real saves; ignored on load (read via reflection in `DeserializeGeneralObject`? the field is `FloatValueTextHandle`, emitted as `{}`) |
| `activeSpeciesList` | long[] | ids of species with count > 0 (`SaveState:366-374`); loader adds each to `activeSpecies`; unknown ids are skipped (re-speciation) |
| `nodeMaxInnovation` | long | `NEATBrain.nodeMaxInov`; must exceed every node `Inov` in use |
| `connectionMaxInnovation` | long | `NEATBrain.connectionMaxInov`; must exceed every synapse `Inov` |
| `nextSpeciesID` | long | `Species.speciesMaxID`; must exceed every species id |

Species object (`Species.SaveState`, `Here/SimulationScripts.BibiteScripts/Species.cs:324-345`):

| key | type | notes |
|---|---|---|
| `speciesID` | long | unique, > 0 |
| `timeCreation` | f64 | simulation seconds |
| `generationOfFirstSpecimen` | int | |
| `favorite` | bool | protect from pruning |
| `genericName` | string | genus |
| `specificName` | string | species epithet |
| `description` | string | may be `""` |
| `parentID` | long | optional; omitted for roots; unknown id => treated as root (`GlobalLineageManager.cs:413-433`) |
| `template` | template object | `SaveStateForSpecies` shape from 4.5: `version`, `nodes[]`, `synapses[]`, `genes{}`, `nodeAnchors?` |

Real example (first species of `lol.zip`) begins:
`{"speciesID":1775,"timeCreation":1.735681121237576,"generationOfFirstSpecimen":0,"favorite":false,"genericName":"Turny","specificName":"boy","description":"","template":{"version":"0.6.3a3","nodes":[{"Type":0,"Index":0,"Inov":1,"Desc":"EnergyRatio","baseActivation":0.0},...`

Invariants enforced/assumed by the loader: every id in `activeSpeciesList` should exist in
`recordedSpecies` (the code adds `null` otherwise and later code may NRE); both innovation counters
must exceed all in-use innovations (the game uses them to mint new ones and does not scan files);
`nextSpeciesID` > max id. If `speciesData.json` is absent entirely (version >= 0.6a5), the game
keeps the empty in-memory species list and re-speciates as bibites load - safe for a fresh world,
but lineage/history is lost.

## 5.10 Load order and failure modes

`SaveSystem.LoadGame` (`SaveSystem.cs:690-883`) in order:

1. Read settings, species, scene, pellets, pheromones, `vars`, `data.bin` entries into memory.
2. Parse scene version; **only** `scene.version` decides compatibility. If missing/unparseable:
   error popup, abort (`GetVersionOfFile(..., generateError:true)`, 1040-1052). If older than the
   minimum supported (`< 0.4` after VersionTracker checks) -> error popup, abort.
3. `ResetWorld()` destroys the current world (before any content is loaded).
4. Deserialize settings (`DeserializeSettings(settings, version)`), then version-specific settings
   fixups (section 8).
5. Load speciesData (only version >= 0.6a5).
6. Extract missing `templates/*` into the user's library.
7. Legacy zone synthesis for pre-0.5a4 saves.
8. Build spawners from the settings; then `LoadScene` (towers first, then pellets, pheromones,
   `simulatedTime`).
9. Load all bibites, then all eggs.
10. Re-link ids (`eggLayer.children`, `genes.parent1/2`).
11. Load `data.bin`; `ZoneManager.UpdateBiomass`; `ResumeSpawners`.

Failures that the loader itself does not catch result in `SaveController.LoadWorld` showing an
error dialog (`SaveController.cs:192-212`) - the world may be left half-loaded. Our importer should
validate before mutating any state and return `Error::InvalidFormat { file, path, expected }` with
the failing JSON path.

Minimum required content for a save the game will accept (empirically): `settings.bb8settings`,
`scene.bb8scene` with parseable `version`, and settings whose `zones[]` has at least one zone
(if no zone exists, no food/zone manager is built; `ZoneManager.GenerateSpawnPoints` creates none
and pellets become free pellets - loadable but degenerate). Bibite/egg/pellet/pheromone/template
content is optional; `speciesData.json` and `data.bin` optional; `img.png` optional.


# 6. Scenario bundle (`.zip`)

## 6.1 Location and entries

Scenarios live in `Scenarios\*.zip` (`ScenarioSaver.DefaultSimulationSettingsPath`, `Here/SettingScripts/ScenarioSaver.cs:20,66`).
The shipped/official scenarios are re-extracted from Unity `Resources/Scenarios/` into that folder
on every startup (`AppInitializer.ReImportOfficialScenarios`, `Here/ManagementScripts/AppInitializer.cs:123-159`);
so editing a file there is transient. Workshop scenarios are hosted zips downloaded into the
workshop cache and opened directly.

A scenario zip contains only:

| Entry | Required | Content |
|---|---|---|
| `scenario.info` | **yes** | metadata (6.2) |
| `settings.bb8settings` | **yes** | scenario settings: 78 scalars + `materials`, `zones`, `bibites`, `settingsChangers`, `challenge?`, `zoneGroups`, `recommended` (6.3) |
| `img.png` | no | preview image (selected by the first entry whose name ends `.png`, `ScenarioSelectorPanel.cs:382-391`; recommended 1 MB max, `ScenarioSaver.cs:113,132-135`) |
| `templates/<file>` | no | copies of templates referenced by `settings.bibites[]` that are not built-in; extracted into the local library when the scenario is selected (`SaveSystem.CheckTemplatesOfArchive`, `SaveSystem.cs:914-936`, called from `ScenarioSelectorPanel.cs:410`) |

There is **no** `scene.bb8scene`, `pellets.bb8scene`, `pheromones.bb8scene`, `data.bin`,
`speciesData.json`, `img`-less requirement, or bibite entity in a scenario. Starting a scenario
creates an empty world (no save to load) and lets the spawners fill it
(`SimulationManager.Start`, `Here/ManagementScripts/SimulationManager.cs:50-63`).

Minimal real example: `Scenarios\Empty Playground.zip` has exactly `scenario.info`, `img.png`,
`settings.bb8settings` (2175 bytes total).

## 6.2 `scenario.info`

Written by `ScenarioSaver.SaveScenario` (`Here/SettingScripts/ScenarioSaver.cs:166-200`) for
user-saved scenarios; shipped scenarios are hand-authored in the same schema (and older versions).

| key | type | required | semantics |
|---|---|---|---|
| `name` | string | **yes** for display (`ScenarioSelectorPanel.cs:370`) | scenario title; duplicate names are prevented by the saver against default scenario names |
| `desc` | string | yes for display (`ScenarioSelectorPanel.cs:371`) | description; saved as `"No description was given for this scenario."` if empty (`ScenarioSaver.cs:173`) |
| `version` | string | **yes** | version of the game that created the scenario; `ScenarioItemReference.InitScenarioItem` rejects the scenario if missing/unparseable or `< 0.5.1` (`ScenarioItemReference.cs:91-94`) |
| `isOfficial` | bool | no | shipped-scenario flag; only shown as official if the name is also in `GameManager.defaultScenarios` (`ScenarioItemReference.cs:87-90`) |
| `rank` | int | no | ordering of official scenarios in the list (`ScenarioSelectorPanel.cs:359-362` sorts by rank) |
| `isChallenge` | bool | no | challenge scenario; shows stars; challenge scenarios are not autosaved (`SaveController.cs:235`) |
| `star1` / `star2` / `star3` | string | no | human-readable star objectives, written only for challenges (`ScenarioSaver.cs:175-181`) |

Real examples:

* `Scenarios\Default.zip`: `{"name":"Default","isOfficial": true,"rank":1,"version":"0.6", "desc":"..."}`
* `Scenarios\Empty Playground.zip`: `{"name":"Empty Playground","isOfficial":true,"desc":"...","version":"0.6"}`
* `Scenarios\The Ultimate Duel.zip`: `{"name":"The Ultimate Duel","desc":"...","version":"0.6.1a8","isChallenge":true,"isOfficial":true,"star1":"...","star2":"...","star3":"..."}`

## 6.3 Scenario `settings.bb8settings`

`SerializeScenario(saveRecommendations: true)` (`Here/ScriptHelpers/SerializationHelper.cs:283-307`):
the same 84-key shape as a save settings file but with `recommended` instead of `independents`
(and optionally `challenge`), and **without** the `independents` key. It contains:

1. The 78 scenario scalars, emitted by `JsonUtility.ToJson(ScenarioSettings.Instance)`.
   Note: **scenario files must write all 78 scalars**, because the scenario loader resets every
   scalar key that is absent to its default (`SerializationHelper.cs:349-363`). Older shipped
   scenarios rely on this (e.g. they lack `enableRedDeath`, `corpsesEnabled`, `decayTPS` etc. and
   silently get current defaults).
2. `materials` (`PlantSettings`...; section 3.4). If absent, all materials reset to defaults first
   (`MatterMaterialManager.ResetAllMaterials()`, `SerializationHelper.cs:385`).
3. `zones` (array; section 3.5). If absent/empty the scenario has no food zones whatsoever.
   For saves created by very old versions (`< 0.6.0.6`) zone `fertility`, `biomassDensity`,
   `pelletSize`, `speed` are converted with `pow(10, value)` and a biomass redistribution is
   performed (`SerializationHelper.cs:434-470`).
4. `zoneGroups` (array; section 3.6). If absent, empty; each loaded group with `zones.Count < 1` is
   regenerated from its template (`SerializationHelper.cs:473-484`).
5. `bibites` (array of spawn rows; section 3.7). If absent, no templates spawn. Paths refer to
   files that the loader has made available: built-in default templates
   (`Bibites\Templates\<name>.bb8template`, rewritten from Resources every start), or
   `templates/<file>` extracted from this zip by `CheckTemplatesOfArchive`.
6. `settingsChangers` (animators; section 7.1). Only loaded if the **scenario's `version` >= 0.6.3a3**
   (`SerializationHelper.cs:510-514`) - see Q7.
7. `challenge` (only for challenge scenarios; section 7.2).
8. `recommended`: an object in the `ScenarioIndependentSettings` wrapper format (the 14
   `independents` keys; section 3.2). `ScenarioSaver` always writes it (`saveRecommendations: true`).
   Shipped 0.5-era scenarios may use old key names (`capVirginBibites`, `capNumber`, `killZone`,
   `killZoneRadiusFactor`, ...) which the current game **does not migrate** - they are ignored and
   the corresponding present-day settings fall back to defaults.

### Application semantics when a scenario is selected/started

`ScenarioSelectorPanel` opens the zip and calls
`SerializationHelper.DeserializeScenario(settings, scenarioVersion, checkForModifiers: true)`
(`ScenarioSelectorPanel.cs:397`), which:

1. Resets `ScenarioSettings` from the JSON (`JsonUtility.FromJsonOverwrite`), then for each
   `IForceUpdateSetting` scalar: reset to default if the JSON key is absent, else force-update
   (`SerializationHelper.cs:346-363`).
2. If `recommended` exists and `checkForModifiers`, overwrites `ScenarioIndependentSettings` with it
   and resets missing independent keys to their defaults (`SerializationHelper.cs:364-384`).
   So **`recommended` is the scenario's `independents` block**; when it is absent, the live
   independent settings are kept (which in a fresh process are the code defaults).
3. Resets all materials, then applies `materials` (section 3.4), applies migration fixups for old
   versions, loads `challenge`, resets + loads `zones` and renumbers ids, clears + loads
   `zoneGroups` (regenerating zones if empty), clears + loads `bibites`, applies old-version
   fixups, clears + loads `settingsChangers`, rebinds listeners.
4. Starting the world then constructs zones from `ScenarioSettings.allZones`
   (`ZoneManager.GenerateSpawnPoints`) and spawners from `bibites[]`, with the settings as applied;
   there is no scene/pellet/entity load path for scenarios.

For our engine the scenario importer is therefore a pure settings transform: **import scenario ->
materialise the same in-memory `ScenarioSettings`/`ScenarioIndependentSettings` state (defaults
first, then file values, then migrations), and expose it as a world template.** Serialising a
scenario back is the inverse: write all 78 scalars + materials + zones + zoneGroups + bibites +
settingsChangers + challenge? + recommended.

## 6.4 Challenge scenario metadata

Challenge scenarios add `challenge` to settings (section 7.2) and `isChallenge`/`star1-3` to
`scenario.info`. Real example `The Ultimate Duel.zip` has both. `ChallengesProgress` (user
progress: stars/highscores) is local user state, not part of the scenario file.


# 7. Settings animators and challenge conditions

## 7.1 `settingsChangers[]` (Settings Animators)

A `SettingsChanger` is a spline-driven multiplier applied to a set of numeric settings over
simulation time. Written by `SettingsChanger.SaveState` (`Here/SettingScripts/SettingsChanger.cs:180-225`),
read by `LoadState` (227-260). Saved inside `settings.bb8settings` as `settingsChangers` (array,
present in both saves and scenarios; loaded only when the file version is `>= 0.6.3a3` (the alpha in which animators were introduced),
`Here/ScriptHelpers/SerializationHelper.cs:510-514`).

JSON shape (field order = writer order):

| key | type | default (new animator) | notes |
|---|---|---|---|
| `name` | string | `""` | display name (`changerName`) |
| `settingsBases` | object | `{}` | map `"<setting key>" -> baseValue` (f32); the value the animator multiplies. Only numeric settings that were "available" (see below) are in the map |
| `t` | f32[] | `[]` | point x values = **seconds of simulated time**; strictly ascending for sane behaviour (no validation in loader) |
| `y` | f32[] | `[]` | point y values = **multiplier** |
| `d` | string[] | `[]` | interpolation type per point: `Linear`, `EaseIn`, `EaseOut`, `EaseInOut` (enum values 0-3). Loader: `d[i].ToObject<InterpolationType>()` - names or ints accepted |
| `f` | f32[] | `[]` | ease factor per point (default 2 for new points; `SplinePoint` constructor `Here/ScriptHelpers/SplinePoint.cs:30-37`) |
| `repeat` | bool | `true` | repeat forever; when false the animator ends after its last point |
| `start` | f32 | `0` | delay in seconds of simulated time |

Constraints/invariants:

* `t`, `y`, `d`, `f` must have the same length; the loader only iterates over `t.Count` and
  indexes the other arrays, so unequal lengths throw.
* The spline is evaluated with `SplineHelper.Interpolate(SplinePoint p1, SplinePoint p2, float t)`
  (`Here/ScriptHelpers/SplineHelper.cs:60-83`), which uses a per-segment interpolation type derived
  from BOTH point ease factors (`CurveInterpolationFromPoints`, lines 21-58) and, for `EaseInOut`,
  the weighted blend formula at lines 65-71. This is not the simpler per-point
  `Interpolate(float,float,t,type,ease)` overload; use the Vector2 overload's y component exactly.
* `FactorAtTime` (`SettingsChanger.cs:152-178`): if `repeat` and `time > last.x`, wrap with
  `time = points[0].x + (time - points[0].x) % (last.x - points[0].x)`; if `points.Count < 2`,
  return `points[0].y`; otherwise find segment `i` with `time < points[i+1].x` and interpolate.
* `Update` (`SettingsChanger.cs:109-150`) first honours `start` (`time -= start.val`; return while
  time < 0); for non-repeat, once `time > last.x` it sets `ended=true` and returns (no more writes);
  otherwise every tracked setting is set to `base * factor` via `SetFromFloat`
  (so integer settings truncate/round via the `(int)(dynamic)value` cast;
  `NumericSetting.SetFromFloat`, `Here/SettingScripts/NumericSetting.cs:74-77`).
* A base of `0` never moves because `0 * factor == 0`.
* Runtime is driven by `SettingsChangerManager.FixedUpdate` (`Here/SimulationScripts/SettingsChangerManager.cs:37-70`):
  it accumulates `progress` until `period = 0.1` s of real fixed time, then calls `Update(TimeKeeper.simulatedTime)`
  and removes animators with `ended == true`. The manager is disabled when no animators exist.
* Load path additionally requires that each key resolves to a numeric setting that is
  **animatable** and not already claimed by another animator; failures are silently skipped
  (`SettingsChanger.cs:241-247` calling `ScenarioSettings.Instance.RequestAnimateSetting`).
  "Animatable" = controllable numeric settings: `ScenarioSettings` + `ScenarioIndependentSettings`
  scalar settings with `controllable != false` (`SettingsReferenceParser.AssignAllKeys`,
  `Here/ScriptHelpers/SettingsReferenceParser.cs:43-77`) plus zone numeric settings; **not**
  animatable: `initialSeeding`, `SimulationSize`, `speciesGeneticSpan`, `speciesSpanScalingFactor`,
  `speciesPrunePointLimit`, `simTPS`, `brainTPS`, `decayTPS`, `visionLookupUpdateFactor`,
  `visionSenseUpdateFactor` (all have `controllable=false` in source).

### `settingsBases` key syntax

`SettingsReferenceParser.GetSetting(key)` (`Here/ScriptHelpers/SettingsReferenceParser.cs:13-41`)
resolves keys in this order. The resolved objects are `ChangingSetting` instances; their
`Name`/`HelperText`/`WikiLink`/`keyedNamePrefix`/`keyPrefix`/`localKey`/`controllable` fields are
code-only metadata, only the target setting's `Value` is persisted (`Here/SettingScripts/ChangingSetting.cs:7-46`).

1. Zone key: regex `^Zone\((\d+)\)\.(\w+)$`. The number is looked up against **`zoneID`**
   (`zones.FirstOrDefault(z => z.zoneID.ToString() == group1)`), and the suffix is the *C# field
   name* of the zone setting (e.g. `fertility`, `biomassDensity`, `pelletSize`, `speed`,
   `radiusRelative`, `widthRelative`, `insideRadius`, ...). Example: `"Zone(0).fertility"`.
2. Otherwise `ScenarioSettings` public field name (e.g. `"pelletEnergy"`, `"bitePeriodFactor"`).
3. Otherwise `ScenarioIndependentSettings` public field name (e.g. `"pelletGrowth"`,
   `"virginSpawnRate"`).

The writer produces the key via `ChangingSetting.fullKey`
(`Here/SettingScripts/ChangingSetting.cs:33-43`): `keyPrefix + localKey` for zone settings
(`keyPrefix = "Zone(<zoneID at construction time>)."`, set in `ZoneSettings.BaseSubscription`,
`Here/SettingScripts/ZoneSettings.cs:504-512`) and plain field name for the two scalar scopes.
**Quirk (verified in source):** the zone prefix is captured when the `ZoneSettings` object is
constructed, while zone ids are renumbered by array position on load
(`ScenarioSettings.UpdateAllZonesList:1092-1107`); after a load with reordered/removed zones the
stored zone key can point at the wrong zone, and the loader silently drops it. Our writer must
use `Zone(<current array index>).<field>`; our reader should prefer id, and may fall back to
name/sorted-index matching when the id has no animatable setting (documented deviation, Q8).

## 7.2 Challenge objects: `ConditionGroup`, `SimCondition`, `SimulationMetric`

`challenge` is present in a scenario's settings when `ScenarioSettings.challengeParameters != null`
(`isChallenge`, `Here/SettingScripts/ScenarioSettings.cs:1090`). Written by
`ChallengeParameters.SaveState` (`Here/SettingScripts/ChallengeParameters.cs:45-62`):

```json
"challenge": {
  "exit":          {"logic":"And","subs":[<simcondition>...]},
  "scoringMetric": "<SimMetric name>",
  "scoringArg":    "<string, omitted when scoringMetric == Time>",
  "scoringHigh":   true,
  "star1":         {"logic":"Or","subs":[...]},
  "star2":         {...},
  "star3":         {...},
  "champion":      {spawn row subset from BibiteSettings.SaveForChampion}
}
```

Defaults of a fresh `ChallengeParameters` (`ChallengeParameters.cs:10-28`):

| field | default |
|---|---|
| `exitCondition` | `ConditionGroup(Time, "", GreaterThanOrEqual, Constant, "3600")` (2 nested groups: one condition) |
| `scoringMetric` | `SimulationMetric(TagCount, "Champion")` |
| `highScoreIsBetter` | true |
| `oneStarCondition` | `TagCount("Champion") >= Constant("50")` |
| `twoStarCondition` | `TagCount("Champion") >= Constant("100")` |
| `threeStarCondition` | `TagCount("Champion") >= Constant("200")` |
| `championSettings` | default `BibiteSettings` with `tagging=CustomTagging`, tag `"Champion"` on load for champion |

`champion` uses `BibiteSettings.SaveForChampion`: `spawnPriority`, `minimum`, `randomizeGenes`,
`growthAtSpawn`, `spawnType`, optional `spawnZone` (section 3.7).

### ConditionGroup JSON

`Here/SettingsScripts/ConditionGroup.cs:153-201`:

| key | type | notes |
|---|---|---|
| `logic` | `"And"` / `"Or"` | default `And` |
| `subs` | array | elements are either a nested group (has a `logic` key) or a `SimCondition` (no `logic` key) |

Group evaluation (`ConditionGroup.EvaluateIsMet`, lines 61-83): fold left with AND/OR; an empty
group returns `false` (the accumulator starts false). The root group is **simplified** on save and
on load (`SimplifyGroup`, 127-151): child groups are recursively simplified, then a child is
flattened into the parent when its `logic` equals the parent's or when it has <= 1 sub-condition.
Consequence: import->export is **not** structurally identity-preserving; comparisons must compare
the simplified form (this is a known round-trip hazard, section 9).

### SimCondition JSON

`Here/SettingsScripts/SimCondition.cs:95-121`:

| key | type | notes |
|---|---|---|
| `metric1` | SimMetric name | left operand |
| `arg1` | string | left argument (see metric table) |
| `comp` | Comparator name | comparison |
| `metric2` | SimMetric name | right operand |
| `arg2` | string | right argument |

All five are required (loader dereferences each). Evaluation: `Compare(left.Evaluate(), right.Evaluate())`
with `Mathf.Approximately` for `Equal`/`NotEqual` (lines 62-74).

### SimulationMetric

`Here/SettingsScripts/SimulationMetric.cs`; JSON is only ever `{metric: <name>, argument: <string>}`
inside a condition, or `scoringMetric` + `scoringArg` inside a challenge. Argument semantics:

| metric | int | argument | evaluation (source `InternEval`, lines 55-108) |
|---|---|---|---|
| `SpeciesCount` | 0 | species **specificName** | count of that species (`species.count`) |
| `SpeciesBiomass` | 1 | species specificName | summed energy of that species |
| `TagCount` | 2 | tag string | number of live bibites with that tag |
| `TagBiomass` | 3 | tag string | summed energy of that tag |
| `MaterialCount` | 4 | material name (`Plant`/`Meat`) | pellet count of that material |
| `MaterialBiomass` | 5 | material name | summed pellet energy |
| `Time` | 6 | ignored (`""`) | `TimeKeeper.simulatedTime` seconds |
| `Constant` | 7 | numeric string (`"3600"`) | `float.Parse(arg)`; invalid strings throw when evaluated |

`SimMetricExtend.MetricHasArgument` (`Here/SettingsScripts/SimMetricExtend.cs:3-12`): `Time` has no
argument (so `scoringArg` is omitted for `scoringMetric == Time`); `Constant` and `Time` have no
argument *list*.

### Challenge runtime semantics (needed to reproduce scores/high scores)

`Here/SimulationScripts/ChallengeManager.cs`:

* A world is a challenge iff `ScenarioSettings.isChallenge` (`challengeParameters != null`).
  Challenge worlds are started as new worlds only (`!SimulationManager.gameWasLoaded`); loading a
  save of a challenge does not run the challenge.
* Once per second: `exitCondition.EvaluateIsMet()`; when met, stars are computed by checking
  `star3`, then `star2`, then `star1` (highest met wins; `GetStar`), and the score is
  `scoringMetric.Evaluate()`; the game pauses and shows the result.
* `star1/2/3` strings shown to the player are NOT in `challenge`; they live in `scenario.info`
  (`star1`-`star3`), and the numeric conditions live in `challenge.star1`-`star3`.
* High scores are local user state (`ChallengesProgress`), not saved in scenarios.


# 8. Version model and migration table

## 8.1 Version string rules (must be implemented exactly)

`Utility.Version` (`Here/Utility/Version.cs`):

* Fields `V0.V1.V2.V3` + `Alpha`. A string `a.b.c` => (a,b,c,0,0); `a.b.c.d` => (a,b,c,d,0);
  `a.b.cad` (e.g. `0.6.3a3`) => (a,b,c,0,alpha=d). Parsing regex:
  `^[0-9]+(\.[0-9]+)(\.[0-9]+)?(\.[0-9]+)?([aA][0-9]+)?$` (`Version.cs:256-259`);
  unparseable => `Version.Null` = (-1,0,0,0,0) (`Version.cs:23`).
* `ToString()` produces `a.b.c` + (`.d` if V3 != 0) + (`a<alpha>` if Alpha != 0) (`Version.cs:72-75`).
* Ordering (`Version.cs:127-212`): numeric compare V0,V1,V2,V3 then alpha: alpha 0 is a *release* and
  sorts **above** any alpha>0 of the same numeric part; alpha 0 vs alpha 0 equal; for two alphas
  larger alpha wins. So `0.6.3a3 < 0.6.3.1` and `0.6.2a1 < 0.6.2 < 0.6.2.1`.
* **Java-style trap:** the C# constructor overload with four ints is
  `Version(v0,v1,v2,alpha=0)`; the five-int overload is needed for a release 4th component.
  Therefore every `new Version(0,6,0,N)` in the source is **0.6aN** and every
  `new Version(0,6,3,3)` is **0.6.3a3**, while `Version.Parse("0.6.3.1")` is the release
  `0.6.3.1`. This is the single most error-prone detail when porting migrations.
* Save `version` fields are written with `Version.Present.ToString()` / `Application.version`
  (e.g. `"0.6.3.1"`). When we re-export an imported save, we write the **present** version
  `0.6.3.1`, exactly like the game does on save (`SaveSystem.cs:184`, `BibiteTemplate.SaveState:236`,
  `ScenarioSaver.cs:174`).

## 8.2 Compatibility gates (what the game refuses / migrates)

| Gate | Effect | Source |
|---|---|---|
| `VersionTracker.CanUpdateFromVersion(v)` false | load aborts with "earlier version ... incompatible" popup | `Here/ScriptHelpers/VersionTracker.cs:47-71`; `SaveSystem.cs:724-734` |
| `VersionTracker.ChangesSinceVersion(v)` true | warning dialog: structure changed since that version, load attempted anyway | `VersionTracker.cs:47-55`; `SaveSystem.cs:735-744` |
| version older than `0.4` | hard refusal (the `0.4` entry has `canUpdateFromOlderVersion=false`; `0.4` itself is accepted) | `VersionTracker.cs:37-44` |
| scene `version` missing/unparseable | hard refusal ("unknown earlier version") | `SaveSystem.cs:719,1040-1052` |
| scenario `version` missing/unparseable/`< 0.5.1` | scenario is not listed | `UIScripts.UIReferences/ScenarioItemReference.cs:91-94` |
| `version < 0.6a5` | speciesData is not read (and `speciesGeneticSpan` is reset on save load) | `SaveSystem.cs:748-755` |
| `version < 0.6a10` | old speciesData imported via `LineageVersionUpdater.ImportFromPre06a10` | `SaveSystem.cs:782-793`; `Here/Utility.DataLogging/LineageVersionUpdater.cs:14` |
| `version < 0.6a9` | legacy data.bin stream layouts (our M1: unsupported -> drop stats) | `Here/Utility/DataParallelStreamGroup.cs:136`, `BibiteTracker.cs:205`, `GenesBucketsStreamGroup.cs:151` |
| `version >= 0.6.3a3` | `settingsChangers` are loaded | `SerializationHelper.cs:511` |

The game's "ChangesSinceVersion" newest entry is `0.6a15` (`VersionTracker.cs:10-14`), so any
0.6.3.x save loads without a warning.

## 8.3 Settings migrations (old file -> present semantics)

Applied by `SerializationHelper.DeserializeSettings`/`DeserializeScenario` while loading a
save/scenario. They are listed in source order; all thresholds are alphas unless stated.

| gate | migration | source |
|---|---|---|
| `< 0.5a6` | legacy key `plantGrowth.Value` -> `pelletGrowth` | `SerializationHelper.cs:324-327` |
| `< 0.6a6` | zones: `fertility`, `biomassDensity`, `pelletSize`, `speed` are `pow(10, v)`; then each zone `fertility = biomassDensity = (simArea*biomassDensity/sum(biomassDensity*area))`; if `10*max > fertility.maxValue`, scale global `biomassDensity`/`pelletGrowth` by `num6` and divide zone values | `SerializationHelper.cs:434-470` |
| `< 0.6a7` | `bibiteMassDensity /= 1.9`; `pelletGrowth *= 0.7`; `pelletEnergy *= 1.5` | `SerializationHelper.cs:491-505` |
| `< 0.6a9` | material `minEfficiency`/`maxEfficiency` are divided by 100 when `> maxValue` (they used to be percent) | `SerializationHelper.cs:410-422` |
| `< 0.6a13` | `neuronDefaultChance`, `neuronMutationChance` reset to defaults (0.7 / 0.4) | `SerializationHelper.cs:423-427` |
| `< 0.6a16` | every material `reactivity /= 30`; Fat `reactivity = sqrt(old) * pi` | `SerializationHelper.cs:386-400` |
| `< 0.6a17` and `bitePeriodFactor > 10` | `bitePeriodFactor /= 35` | `SerializationHelper.cs:506-509` |
| `>= 0.6.3a3` | load `settingsChangers` (else silently dropped) | `SerializationHelper.cs:510-517` |

Legacy names that are *not* migrated (verified by their absence from `Here/`): `colorR/G/B`,
`mutationChance`, `mutationVariance`, `brainMutationChance`, `synapseMutationChance`,
`killZone`, `killZoneRadiusFactor`, `capVirginBibites`, `capNumber`, `meatRot`, `meatRotTime`,
`meatRotRate`, `initialSeeding` (old shape), `dragCoefficient` (old name is the same),
`forwardForce`, `throwingForce`, `bitingDamage`, `metabolismCost`, `moveCost`, `turnCost`,
`maxEnergyToMaxHealthRatio`, `baseBodyEnergyRatio`, `healthBodyRatio`, `viewAngleBodyCost`,
`viewRadiusBodyCost`, `strengthBodyCost`, `immuneBodyCost`, `virginBibiteTagging`, `randomVirgin`,
`helpfulMutations`, `voidAvoidanceDistance` (kept), etc. Old saves lose those overrides and get
current defaults; the 2022 `.bb8settings` files in `SimulationSettings\` are examples of this
legacy schema (they are loadable only as scenario-style settings; the current game has no
standalone settings import UI at all - `SimulationSettings` is not referenced by current source).

**Implementation status (2026-10-30):** the migrations in this table are **explicitly out of scope for M1** (Lead
decision); `bibites-io` interprets scalar values with current (0.6.x) semantics only. This is acceptable for T0
(0.6.3.x) files; older settings are imported structurally but not behaviourally migrated. Future work: implement
these gates and then enable T1/T2 in §8.5.

## 8.4 Genome/brain/entity migrations

See 4.7 for the full algorithm. Summary of gates (all alphas):

| gate | migration |
|---|---|
| template/genes `version < 0.6a1` | template `genes` may be a 34-float array (old order) -> remap via `to06a15GenesMap`; `.bb8` entities are expected to have named genes already |
| `from < 0.6a7` | six gene slots are overwritten with defaults (game quirk, preserved) |
| `from < 0.6a15` | Strength/Defence -> organ WAG distribution |
| brain `0.6a5` | +input at 8, +output at 3; EggProduction activation 0.2; innovation numbers for old synapses |
| brain `0.6a13` | remove Constant input 0 (folds enabled synapses into targets), Mult activation 0 -> 1 |
| brain `0.6a15` | Want2Eat (node 38) activation 0 -> 2.5 |
| brain `0.6a16` | Want2Eat becomes TanH, activation affine map, weights x0.28 |
| brain `0.6a18` | remove input 32 and output 15 |
| brain `0.6.2a1` | add input at position 5 |
| brain `0.4a3` | add 3 inputs at 28, 1 at 4 |
| brain `0.3` | floor |

The 0.6a13 step uses `Random.Range(1,14)` for disabled synapse inputs (`BrainUpdater.cs:377`), so
an exact byte match with the game's own in-memory upgrade is impossible for brains crossing that
boundary; every other step is deterministic.

## 8.5 M1 migration scope (recommended)

| Tier | Input version | M1 behaviour |
|---|---|---|
| T0 | `>= 0.6.3a3` (all real 0.6.3.x saves, the installed build's own files) | full import/export, no migrations except none needed; animators load |
| T1 | `0.6a5 <= v < 0.6.3a3` | import with settings/brain/gene migrations above; export as 0.6.3.1. Best effort: `data.bin` opaque (T1 data layouts are <= present but >= 0.6a9; if older, drop stats) |
| T2 | `0.4 <= v < 0.6a5` | attempt settings + genes/brain migrations; `data.bin` dropped; speciesData import best-effort (pre-0.6a10 converter is a separate port); mark `PartialSupport` in results |
| T3 | `v < 0.4` | `Unsupported` error (matches the game's own refusal floor) |
| T4 | 0.6.4a1+ / folder saves | `Unsupported` with explicit reason (Lead decision) |

Open: whether the Lead wants T1/T2 implemented in M1 or only T0 with clear errors (Q2).

**Current implementation (2026-10-30):** T0 is complete (all real 0.6.3.x saves/scenarios round-trip); **T1/T2 are
explicitly out of scope for M1** (Lead decision) — brain/gene migrations exist, but the settings migrations of §8.3
do not, so pre-0.6 files are imported with current-semantics interpretation only. T3 returns `Unsupported`; T4
(0.6.4a1 folder layout) is also out of scope and currently fails with a generic I/O/zip error. Implement §8.3 and
the folder-layout diagnostic before claiming T1/T2/T4 support.


# 9. Round-trip requirements

## 9.1 What "round-trip" means for M1

1. **Import**: parse every file the installed 0.6.3.1 game writes (T0 tier) into typed in-memory
   state without panics, with precise errors otherwise.
2. **Export**: emit files that the installed game **loads** and applies with the same semantics.
3. **Re-import**: our own exports must parse back to the same in-memory state (semantic
   idempotence), and exports must be deterministic (same input -> byte-identical output).
4. **Cross-tool**: export from the game -> import in Rust -> export -> load in the game -> save in
   the game -> import in Rust: the world state must be semantically equal apart from documented,
   unavoidable differences in 9.6.

Byte-for-byte equality with the game's own output is a **testing nicety**, not a load requirement.
The one exception is `data.bin` (opaque passthrough, 9.5).

## 9.2 Per-file required/optional matrix (summary)

| File | Must write | May omit | Never write |
|---|---|---|---|
| save `settings.bb8settings` | all 78 scenario scalars (+ wrapper), `independents` (14), `materials` (all four), `zones` and `bibites` (may be empty arrays), `zoneGroups` (may be empty), `settingsChangers` (may be empty) | `challenge` (saves of non-challenge worlds; challenge worlds are not saved with it by the game - `SaveController.cs:235` skips autosave but manual save writes the current settings incl. challenge) | legacy keys, duplicate keys, NaN/Inf |
| scenario `settings.bb8settings` | all 78 scalars, `materials`, `zones`, `bibites`, `zoneGroups`, `settingsChangers`, `recommended` | `challenge` | `independents` (scenario uses `recommended`; writing both is harmless but non-canonical) |
| `scene.bb8scene` | `version` (parseable), `simulatedTime`, the four entry arrays (may be empty) | `nPellets`, `nBibites`, `gameName` | legacy `pellets`/`pheromones`/`connections`/`pelletSpawner` when sidecars exist |
| `vars.bb8scene` | `towerMaxID` = max tower id in the world (>= every `id`) | whole file if no towers | |
| `pellets.bb8scene` | `pellets` array; every group object has `zone` (string) + `pellets` array | empty groups, `rb2d`, `matterDecay` (unless material decays) | stale zone names (dangling names become free pellets) |
| `pheromones.bb8scene` | `pheromones` array; each spot has `transform` + `phero` with 6 numeric fields | whole file when none; `heading` (always lost) | non-empty `heading` (it is discarded) |
| `bibites/*.bb8` | `transform` (or at least `rb2d`), `genes` (with `isReady`, `gen`, all 35 genes), `body` (with `health`, `energy`, `d2Size`, ids), `clock`, `brain` (`isReady` true, valid nodes/synapses) | arbitrary other fields | nodes with invalid indexes, floating hidden nodes, duplicate (in,out) synapses |
| `eggs/*.bb8` | `genes`, `egg`, `brain` (`isReady` true, no floating hidden), `transform`/`rb2d` | `egg.id` (0 allowed) | living-bibite fields |
| `speciesData.json` | `recordedSpecies` array, `activeSpeciesList`, `nodeMaxInnovation`, `connectionMaxInnovation`, `nextSpeciesID`, counters | whole file when a pure fresh world has no species yet (game recreates) | ids that do not resolve, counters below in-use innovations |
| `data.bin` | (M1) none - opaque passthrough if imported | optional entirely | mis-framed payloads, zero-length elements |
| `templates/<file>` | template JSON (4.5) | optional | a `templates/` entry that shadows a built-in template name |
| scenario.info | `name`, `desc`, `version` | `isOfficial`, `rank`, `isChallenge`, `star1-3`, `img.png` | unparseable version |
| `.bb8` export | shape A (living) or B (egg) + `version` (0.6.3.1) + `desc` | - | `desc` must be a string (can be empty) |
| `.bb8template` | `name`, `generation`, `version`, `nodes`, `synapses`, `genes` | `description`, `nodeAnchors`, `isOfficial` (only if true) | `speciesName` mismatch (it is recomputed from `name` on load; write the recomputed value to avoid confusion) |

## 9.3 Canonical emission order (for deterministic/golden output)

Because readers are order-insensitive, this order is for byte stability and readability, matching
the game's writers where they have one:

* `settings.bb8settings`: the 78 scalars in table 3.3 order, then `materials` (Plant, Meat, Fat,
  Armor; each in 3.4 field order), `zones`, `bibites`, `settingsChangers`, `challenge` (scenario
  only), `zoneGroups`, then `independents` (save) / `recommended` (scenario). This is exactly
  `SerializeScenarioSettingsCommon` + `SerializeSettings` order (`SerializationHelper.cs:267-307`).
* Every scalar setting object: `interactableChanged` then `Value` (`JsonUtility` field order:
  inherited `interactableChanged` first, then the private `Value` - matches real files).
* Zone object: `name, id, material, distribution, fertility, biomassDensity, pelletSize,
  renewBiomass?, movement, target?, speed, posX, posY, radiusIsRelative, radius|width,height,
  insideRadius?` (`ZoneSettings.cs:676-725`).
* Zone group: `name, scaleWithSim, count, template, zones?` (`ZoneGroupSettings.cs:150-182`).
* Spawn row: `path, spawnPriority, minimum, randomizeGenes, growthAtSpawn, tagging, spawnType,
  tag?, totalSpawned?, isExternal?, spawnZone?` (`BibiteSettings.cs:167-191`).
* Material: `cohesiveness, reactivity, energyDensity, massDensity, minEfficiency, maxEfficiency,
  decay, freshTime?, decayRate?` (`MatterMaterialSettings.cs:217-230`).
* Animator: `name, settingsBases, t, y, d, f, repeat, start` (`SettingsChanger.cs:194-224`).
* Challenge: `exit, scoringMetric, scoringArg?, scoringHigh, star1, star2, star3, champion`
  (`ChallengeParameters.cs:45-61`).
* Condition group: `logic, subs`; condition: `metric1, arg1, comp, metric2, arg2`
  (`ConditionGroup.cs:164-172`, `SimCondition.cs:104-111`).
* Save scene: `version, nPellets, nBibites, colorSelectors, pheroTowers, radTowers, simulatedTime`
  (`SaveSystem.cs:184-269`).
* Colors/towers/pellets/pheromones: as documented in 5.3-5.5.
* `speciesData.json`: `recordedSpecies, maxEnergy, maxCount, logCounter, nRecords, usedSpanText,
  activeSpeciesList, nodeMaxInnovation, connectionMaxInnovation, nextSpeciesID`
  (`GlobalLineageManager.cs:361-379`; real files match).
* Species object: `speciesID, timeCreation, generationOfFirstSpecimen, favorite, genericName,
  specificName, description, parentID?, template` (`Species.cs:324-333`).
* `.bb8template`: `name, speciesName, description, generation, version, nodes, synapses, genes,
  nodeAnchors?` then `isOfficial` only if true (`BibiteTemplate.cs:227-271`).
* `.bb8`: `transform, rb2d, genes, body, clock, brain, version, desc` for living bibite;
  `transform, rb2d, genes, egg, brain, version, desc` for egg (`SaveSystem.cs:135-169`).
* Brain: `isReady, Nodes, Synapses`; node `Type, baseActivation, TypeName, Index, Inov, Desc,
  Value, LastInput, LastOutput, archetype`; synapse `Inov, NodeIn, NodeOut, Weight, En`.

## 9.4 Number formatting algorithm (produce the game's tails)

Implement one f32->JSON function and use it everywhere:

```text
fn write_f32(v: f32) -> serialized number:
    let d = v as f64;                    // exact widening
    emit serde_json::Number::from_f64(d) // ryu shortest round-trip double
```

* `0.05f` -> `0.05000000074505806`; `0.1f` -> `0.10000000149011612`;
  `0.6f` -> `0.60000002384185791`; `1e-05f` -> `0.00001`;
  `0.0075f` -> `0.007499999832361937`; `1.35f` -> `1.3500000238418579`.
* This matches Newtonsoft because `JToken.op_Implicit(float)` widens to double and Newtonsoft
  writes the shortest double form. (Real save values confirm the tails.)
* Integers write as integers; booleans lower-case; strings escaped per JSON (Newtonsoft escapes
  control chars as `\uXXXX`, non-ASCII is emitted raw UTF-8 by default).
* Reading: parse number -> f64 -> cast to f32 (Newtonsoft `ToObject<float>` semantics: a JSON
  double converted to float32, i.e. round-to-nearest; do not go through a decimal string).
* `version` output is the plain string `"0.6.3.1"` (configurable, 9.7).

## 9.5 Unknown-field preservation

Requirement: import must not lose data that the game might consume or that a newer build might
ignore, because "import then export" is our editing model.

Rules:

1. Each JSON object we model gets an `extra: serde_json::Map<String, Value>` that captures keys we
   do not understand, and is re-emitted **after** known keys.
2. Applies at: settings file top level (captures e.g. 0.6.4a1's `shadeFadeDistance`),
   each scalar setting wrapper, scene, vars, pellets group, pellet object, pheromone spot, bibite
   top level, genes object, body object, brain node/synapse (no extras expected), template,
   species, material, zone, zone group, spawn row, animator, condition, condition group, scenario.info.
3. Unknown JSON **types** are preserved verbatim (Value), not coerced.
4. Unknown zip entries are preserved on save export (except `img.png`, which we cannot regenerate;
   copy the imported bytes through unchanged).
5. Duplicate keys are an error on import (reject) rather than silently last-wins.
6. `data.bin` is preserved opaquely as bytes (no parsing in M1), and only for files whose version
   passes the T0/T1 gates; for older files write `None` and log a `stats_dropped` warning.

## 9.6 Known unavoidable differences (documented, allowed)

| Difference | Cause |
|---|---|
| `version` rewritten to `0.6.3.1` | the game itself writes present version on every save (`SaveSystem.cs:184,237`) |
| `ConditionGroup` structure simplified | `SimplifyGroup` on save+load (7.2) |
| pheromone `heading` becomes `{}` | Vector2 has no `[SerializeField]` fields (5.5) |
| `nPellets` / `nBibites` recomputed | display-only counters |
| `img.png` = imported bytes or absent | we do not render a screenshot |
| body growth/organ state not represented beyond documented fields | game derives them from `d2Size`+genes on load (`BibiteBody.cs:1031`) |
| mutation/lineage statistics (`data.bin`) stale or absent | stats-only, opaque |
| entity ids may be regenerated when we synthesize new entities | ids are random int32 (`BibiteID.cs:9-15`); preserve imported ids exactly |
| `settingsBases` zone keys may be re-based to current zone indexes | game quirk 7.1; we write index-based keys |

## 9.7 Configuration knobs (CLI/env)

* `--version-string` default `0.6.3.1` (written into every version field).
* `--pretty` for `.bb8`/`.bb8template` (default: compact for templates, indented for `.bb8`
  exports to mirror the live install's `FormatBibiteJSON=1`; binary stability tests should pin it).
* `--settings <file>` / `--independents <file>`: game-global fallback for the scenario-independent
  values when a loaded save has no `independents` block (precedence in 3.2.1). Parsed with the
  same `.bb8settings` reader; the CLI records the effective values in the parity manifest.
* `--data-bin=passthrough|drop` (default passthrough when the imported file has one).
* `--strict` (error on unknown required-looking keys) vs default permissive (preserve unknowns).

The parity manifest must record, per capture: `gameVersion` (bridge `/status`, expected `0.6.3.1`),
the effective 14 scenario-independent values, the seed, tick count and the settings file hash used.

## 9.8 Ground-truth verification with the real game (Lead phase)

The game has **no standalone settings import UI** in 0.6.3.1 (no reference to `SimulationSettings`
in current source; the folder only contains 2022 leftovers). Therefore every artifact is validated
by placing it in the live file tree of a **disposable copy** of the game+appdata (never the
user's real profile) and observing the game:

1. **Capture**: start the game, load `Scenarios\Default.zip` (or a specific scenario), save a world
   (`Savefiles\capture.zip`), quit. This produces the ground-truth 0.6.3.1 reference save. Read
   `scene.bb8scene.version` and record it in this spec (Q1).
2. **Settings/scene/pellets/pheromones/species/bibites bundle**: run `bibites-io` round-trip on
   `capture.zip` -> `out.zip`; copy `out.zip` to `<sandbox>\Savefiles\`; start the game, load it
   from the Load Game panel; verify: world loads without error popups, zone count/shape, pellet
   appearance, bibite/spawner counts, settings panel values, species lineage panel, statistics.
   Then save in game and diff the new save against our `out.zip` semantically (settings, zone
   fields, entity genes/brains; ignore ids of newly spawned entities, counters, data.bin).
3. **Scenario**: copy our exported scenario zip to `<sandbox>\Scenarios\`; confirm it appears in
   the scenario list with name/desc/image, can be selected/started, and its settings match
   (open the settings panels); check templates it bundles appear after selection.
4. **`.bb8template`**: copy to `<sandbox>\Bibites\Templates\`; launch; the template appears in the
   bibite template selector; spawn it from the scenario editor and inspect genes/brain; save the
   world and diff the spawned bibite's genes/brain against the template.
5. **`.bb8`**: copy to `<sandbox>\Bibites\`; launch; the saved bibite appears in the template
   selector (it is loaded through `BibiteTemplate`, genes/brain only); spawn and verify.
6. **Negative tests**: corrupt/missing required fields per section 9.2; the game should show its
   error dialog, not crash; our importer must return a typed error.

Automation must never write to `C:\Users\Storme\AppData\LocalLow\The Bibites\The Bibites` or the
game installation; use copies (the task brief allows copying to a temp dir).


# 10. Rust design notes (crates/bibites-io + core settings)

Constraints from the Lead: `crates/bibites-core` is dependency-free and f32; it owns the typed
settings model + exact defaults. `crates/bibites-io` owns all file formats and depends on
`serde`, `serde_json`, `zip` (pending Lead confirmation that crates.io is reachable). Do not add
deps before that confirmation.

## 10.1 Module layout

```
crates/bibites-core/src/settings.rs        # SimSettings (78 scenario + 14 independent + defaults)
                                           #   + zone/material/spawn/animator/condition structs?
                                           #   (Lead decision: typed model lives here; defaults normative)
crates/bibites-io/src/
  lib.rs            # facade: read/write save, scenario, genome, template
  version.rs        # Version parse/compare/format; alpha-vs-release rules (8.1)
  value.rs          # ordered JSON value helpers, extras capture, f32 formatting (1.3, 9.4)
  settings_file.rs  # .bb8settings save/scenario schema + migrations (3, 7)
  zone.rs           # ZoneSettings custom mapping (3.5)
  material.rs       # material settings + defaults (3.4)
  genome.rs         # genes (35), .bb8template, .bb8 shapes A-D (4)
  brain.rs          # Node/Synapse, base node tables, bias/activation (4.4)
  migrations/       # version gates: settings.rs, genes.rs, brain.rs, species.rs
  entity.rs         # BibiteBody/Clock/Egg JSON schemas (4.3)
  save.rs           # zip bundle layout, scene/vars/pellets/pheromones (5.1-5.7)
  species_data.rs   # speciesData.json (5.9)
  databin.rs        # opaque passthrough only in M1 (5.8) + TODO full codec
  scenario.rs       # scenario bundle + scenario.info (6)
  animator.rs       # SettingsChanger (7.1)
  condition.rs      # ConditionGroup/SimCondition/SimulationMetric (7.2)
  error.rs          # Error/Result (see 10.5)
  tests/            # fixtures + golden round-trip
```

`bibites-io` must not depend on `bibites-core`'s simulation internals; it needs only the settings
types and the `Version`/error primitives. Keep a `serde_json::Map`-based "extras" layer in io
rather than polluting the core settings struct with JSON concerns.

## 10.2 serde model rules

* Use explicit `#[serde(rename = "...")]` for every field: keys are a mix of lowerCamel, PascalCase
  (`SimulationSize`), and custom zone keys (`name`, `radiusIsRelative`). Do not rely on
  `rename_all`.
* Keep struct field order = canonical emission order (section 9.3); `serde_json` serialises struct
  fields in declaration order, giving deterministic output.
* Each struct that maps a JSON object carries `#[serde(flatten)] extra: Extras` where
  `type Extras = serde_json::Map<String, serde_json::Value>`. Serde's `flatten` captures unknowns
  and re-emits them, satisfying section 9.5. (Caveat: `flatten` + `deny_unknown_fields` are
  incompatible; do not use `deny_unknown_fields` on extensible objects.)
* Use `Option<T>` + `#[serde(skip_serializing_if = "Option::is_none")]` for the "emitted only
  when true/positive" fields, but keep a separate `Option<bool>` for presence-sensitive fields
  (`renewBiomass`: presence means true; load must not distinguish false vs absent for writing,
  but preserves nothing when absent).
* Scalar settings file: model the 78 + 14 as `BTreeMap<String, ScalarValue>` at the io layer, or
  as a struct of wrapper types in the core; either way, validate all 78+14 keys exist on **import**
  of a save and warn (or error under `--strict`) when missing; always write all keys.
  `ScalarValue = { value: NumberOrBool, extra: Map }` with `interactableChanged` defaulted.
* Enums: use Rust enums with `#[serde(rename_all = ...)]` only where names are uniform
  (`Comparator`, `SimMetric`, `NodeType` are flat names). For JSON read, accept both the name and
  the integer (Newtonsoft accepts both) with a custom deserializer; write names.
* Numbers: settings are f32; ids/counters are i64/u64; `timeCreation` is f64; `simulatedTime` f64.
  Do not model statistics as f32 where the game uses double.
* The `Version` type: struct with u32 x4 + u32 alpha; implement `Ord` exactly as 8.1 (alpha 0
  release ranks above alpha>0); `FromStr` with the exact regex; `Display` with the conditional
  `.V3`/`a<alpha>` parts.

## 10.3 f32 JSON helper

Provide `write_f32` through serde_json's `Number::from_f64(value as f64)`; serde_json uses ryu's
shortest round-trip representation for f64, matching Newtonsoft's doubles (verified list in 9.4).
Never use `format!("{}", f32)` (Rust prints shortest f32, e.g. `0.05`, which is a different JSON
literal than the game's). On read, deserialize to f64 then `as f32` (round-to-nearest) to match
Newtonsoft `ToObject<float>`.

## 10.4 Zip and file IO

* Read: `zip::ZipArchive`; match entries case-sensitively; accept `\` and `/`; tolerate unknown
  entries; error clearly on truncated/corrupt zips (real data has one); never panic.
* Write: `zip::ZipWriter` with Deflate; fixed timestamp (e.g. 1980-01-01 00:00) for deterministic
  bytes; entry names `bibites/bibite_<n>.bb8` etc.; JSON entries prefixed with UTF-8 BOM
  (`\xEF\xBB\xBF`) per 1.1; `.bb8`/`.bb8template` files written without BOM via plain `File`.
* Write save/scenario files atomically: temp file + rename (game deletes first; we must not).
* Never read/write live app data in tests; fixtures are copies under `$TEMP` or checked-in
  synthetic files.

## 10.5 Errors

```rust
pub enum IoError {
    Unsupported { what: String, version: Option<Version> },  // folder saves, 0.6.4a1, T3 saves
    Invalid   { file: String, path: String, reason: String },// JSON path e.g. "zones[2].radius"
    Missing   { file: String, path: String },               // required key absent
    Version   { file: String, found: String, reason: String },
    Zip(String), Io(std::io::Error), Json(String),
}
pub type IoResult<T> = Result<T, IoError>;
```
No `unwrap`/`expect` on parsed file content; index accesses on required arrays must be checked with
the JSON path reported.

## 10.6 Testing plan

* Synthetic fixtures for every shape (minimal save, scenario, template, `.bb8` living/egg, brain
  with hidden nodes, all 35 genes, condition tree, animator) generated from the game's documented
  defaults - checked in.
* Golden export tests: expected bytes for a minimal bundle (pins BOM, whitespace, key order,
  f32 formatting, zip entry order/timestamps).
* Round-trip property: `parse(serialize(parse(x))) == parse(x)` for all fixture families, with
  `data.bin` compared bytewise.
* Import tests against a real 0.6.3.1 save **only when** the environment variable
  `RUSTIBITES_REAL_SAVE_DIR` is set (keeps user data out of the repo; never commit game data).
* Game-load checklist (manual, Lead) from 9.8.

## 10.7 Settings model in core (deliverable for `crates/bibites-core/src/settings.rs`)

When the Lead assigns the implementation task, this module should expose:

* `SimSettings` with the 78 scenario + 14 independent fields, defaults exactly as tables 3.2/3.3,
  plus enums for spawn/material/animator/condition where the engine needs them.
* `Default for SimSettings` = those tables; a unit test that asserts every field equals the
  documented default and that the field list has 78/14 entries (guards parity drift).
* No JSON: serialization types live in `bibites-io`; core settings are plain Rust (f32/i32/bool).
* `#[non_exhaustive]`-free, `Copy` where sensible; f32 everywhere (Lead's core is f32).


# 11. Open questions / ambiguities

Ordered by impact on M1. Each has a concrete resolution path. Q-numbers are referenced from the
text above.

**Q1 - Exact present version string. RESOLVED (2026-10-29).** `0.6.3.1` is confirmed by three
independent signals (section 0.1: `globalgamemanagers` bundleVersion, PlayerPrefs
`VersionOnLastOpen`, newest save headers) and by the parity bridge's `/status` `gameVersion` field.
Keep `--version-string` at `0.6.3.1`; no further action. (The `0.6.3.3` file found in app data is a
newer-build scenario bundle, not the installed game; see 3.2.1/Q7.)

**Q2 - How old must imported saves be?** Section 8.5 proposes T0 (`>= 0.6.3a3`) full support,
T1 (`>= 0.6a5`) migrations, T2 best-effort, T3 error. Implementing all T1/T2 migrations is
significant (settings fixups, brain remaps, pre-0.6a10 species import). The Lead must decide M1
scope. Recommendation: implement T0 fully + T1 settings/brain/gene migrations (they are mechanical),
T2/T3 as explicit `Unsupported` with a roadmap note.

**Q3 - `data.bin` fidelity.** M1 plan: opaque passthrough (no parsing/re-encoding). This means an
imported save keeps its statistics intact, but a save created by us has no stats (game starts
empty). Is displaying/continuity of statistics a benchmark/parity requirement? If yes, a full
`data.bin` codec is needed later (source files listed in 5.8; external SAVE_FORMAT section 7 has a
verified layout). Also confirm: statistics are not used by the simulation itself (they are
display-only) - from `Here/Utility/DataLogger.cs` they only feed charts/UI.

**Q4 - Unknown-field policy.** We preserve unknown JSON keys/types and unknown zip entries
(section 9.5). This is stricter than the game (which ignores them) and guarantees 0.6.4a1 extras
survive. Confirm this is required for "full round-trip", or whether only known fields need
preservation (simplifies types but risks silent data loss).

**Q5 - Pre-0.6a1 `.bb8` files with array genes.** `BibiteTemplate`'s `.bb8` path reads named genes
only (`val["genes"]["genes"][text]`), while `.bb8template` supports the old 34-array via
`BibiteUpdater` with `isSaved=false`. It is unclear whether any pre-0.6a1 `.bb8` entity files
actually exist in the wild and whether the game upgrades them correctly. Resolution: test the
real installer's old files (e.g. `Sample.bb8` is 0.5-era but appears to have named genes) through
the game's template upgrade endpoint in the capture phase; otherwise treat `.bb8` with array genes
as `PartialSupport`/`Unsupported`.

**Q6 - Out-of-range values on import. RESOLVED (2026-10-29, IO I2).** The game's JSON load does
**not** clamp scalars (`JsonUtility`); only `SetValue` clamps, and only for settings whose
`canGoOutOfBounds == false` (see the UI-clamped lists in 3.2/3.3/3.5). RUSTibites therefore clamps
**only** those hard-clamped settings on import (and reports every changed key); all other
out-of-range values are preserved verbatim for round-trip fidelity. This matters in practice:
`Scenarios\Ring.zip` recommends `SimulationSize = 50000` even though the UI range is 100..20000, and
the game applies it un-clamped; clamping it would break `settings_hash` parity. Genes still follow
the game's `CapGenes` on entity load (the io genome reader preserves raw values for templates; the
world clamps via the core `Genome::cap` path when spawning).

**Q7 - 0.6.4a1 and folder saves.** Out of scope per Lead. Our importer must produce a clear
`Unsupported` error (no stack traces) and must not silently treat a folder as a world. Also decide
whether the CLI should *list* such saves in a doctor/status command. Real data has 365 such saves,
so users may hit this immediately.

**Q8 - Animator zone keys.** The game writes `Zone(<id-at-construction>).<field>` and resolves by
current `zoneID` (array index after load); stale ids silently drop animators. We write index-based
keys. Do we add a name-based fallback on read (deviation from game, better round-trip) or mirror the
game exactly (drop on mismatch)? Recommendation: mirror the game on import for parity, but emit
index-based keys so our exports are self-consistent; document the difference. (This is a rare
edge case: only when zones are reordered/removed after animator creation.)

**Q9 - `ConditionGroup` structural simplification.** Save/load flattens nested groups of equal
logic (and single-child groups), so a `challenge` tree does not survive byte OR structure
identity. Accept the simplification (recommended, matches game) and compare simplified trees in
tests. If exact structural preservation is required, we would have to keep the raw JSON and skip
the logic - but then the game would still simplify it on its next save.

**Q10 - Legacy `OldSerializedTransform`.** `SerializationHelper.DeserializePosition` has a fallback
for an older transform shape (`Here/ScriptHelpers/SerializationHelper.cs:238-251`), but the
`OldSerializedTransform` class/fields were not found in the decompiled tree (likely removed/nested).
This only matters for T2 saves; mark unsupported until a real example is captured.

**Q11 - Validation of `.bb8` shape A (full entity).** No current source path loads `.bb8` files as
full entities from disk: `BibiteTemplate` only uses genes/brain. The `LoadBibiteOrEggFromData(json)`
entry point is called from UI code not present as a file-based importer in this build (grep found no
caller). The game validates `.bb8` files as template sources. For our export, we still emit shape A
(same as the game's own "Save Bibite"), and validation is via the template selector (9.8 step 5).
If the Lead wants a full-entity import validation, we need to find the UI that passes a JSON string
(e.g. a drag-drop handler) or use a probe mod.

**Q12 - Float formatting edge cases.** Our rule (`f32 -> f64 -> ryu shortest`) reproduces the
observed tails, but Newtonsoft version-specific formatting (exponent thresholds, `-0.0`, very
small/large magnitudes) may differ in corner cases. Resolution: generate a fuzz corpus of f32
values, write them through the game's JSON writer (probe mod or a save with many settings), diff;
until then, treat byte-golden tests on floats as "best effort" and use semantic comparison.

**Q13 - Determinism / RNG.** No RNG state is saved anywhere (UnityEngine.Random / System.Random
are process-global and not serialized; e.g. `BibiteGenes.SystemRandom` is static,
`BibiteGenes.cs:91`). A save therefore cannot reproduce the exact future behavior of the game, and
the game itself is not deterministic across machines. The benchmark must define determinism
ourselves (our own seeded RNG, documented algorithms) and parity validation must be capture-based
/ statistical, not bit-exact. Confirm with Lead that this is acceptable for "behaviourally
faithful".

**Q14 - `data.bin` zero-length / truncation hazards.** Several loaders interpret `nBytes < 1` as
"whole file" (`SaveableBinStack.LoadStateBin:36-39`, `DataLogger.LoadStateBin:431-435`,
`BibiteTracker.LoadStateBin:199-203`), and real saves have truncated entries (one unreadable zip).
Our importer should reject malformed framing rather than emulate the fallback, and the exporter
must never emit zero-length elements. Confirm that rejecting (rather than best-effort) is preferred.

**Q15 - Template bundling side effects.** The game, when a scenario/save is loaded, writes missing
`templates/*` entries into the user's library (`CheckTemplatesOfArchive`) and on workshop import
moves files. Our importer must not write to the user's library at import time (read-only model);
export should bundle templates like the game (`AddTemplatesToArchive` skips built-ins). Confirm
desired sandboxing for tests and whether a `--extract-templates` option is wanted for parity.

**Q16 - Material defaults provenance. RESOLVED (2026-10-29).** The material defaults (table 3.4)
come from the shipped `MatterMaterial` ScriptableObject assets, not from `Here/`; they are confirmed
by the mode of 372 real saves, the external GAME_GUIDE/API test factory, and a parity-bridge runtime
dump of the installed game. The dump also resolved the runtime-only values for non-decaying
materials (Plant `freshTime=100`, `decayRate=0.005`; Fat/Armor `1000`/`0.02`) and the non-serialized
Energy/Health material densities (1.0 / 0.25). `settings::MaterialSetting::{PLANT,MEAT,FAT,ARMOR}`
and spec 3.4 now encode these.

**Q17 - Missing-template behaviour.** If `settings.bibites[].path` names a template that is not
present (and not bundled), the game shows an error popup at spawn time and skips that row
(`Here/SimulationScripts/BibiteSpawner.cs`/settings UI). Should our engine (a) error at world load,
(b) skip with warning, or (c) substitute a placeholder? Engine policy, confirm with Lead.

**Q18 - Duplicate synapse innovations. RESOLVED (2026-10-29, IO I1).** The decompiled game tolerates
duplicate synapse `Inov` values on load, but they are invalid for innovation alignment/mutation and
no real file in `testdata/real` has them. `bibites-io` I1 rejects duplicates with an
`InvalidData("duplicate synapse innovation ...")` error; this is an intentional divergence from the
game's permissiveness. Revisit only if real data requires accepting them.

**Q19 - Pre-0.6a1 gene arrays. RESOLVED (2026-10-29, IO I1).** Two real 0.5-era templates in
`testdata/real/templates/` store a **28-value** gene array, not the 34-value layout that
`BibiteUpdater.to06a15GenesMap` describes. `bibites-io` I1 supports only 34-value pre-0.6a1 arrays
(template path) / named genes (entity path) and returns `Unsupported` for anything else rather than
guessing. This matches the game's own inability to load those old shipped templates in 0.6.3.1.


# Appendix A - Generated reference tables

These tables were generated programmatically from `Here/` (field initializer literals, source line
numbers) and cross-checked: the 78 `ScenarioSettings` defaults match the independent extraction in
`BibitesInfo/BibitesAPI/bibites_api/core/catalog/scenario_settings.json` exactly (78/78, zero
mismatches), and the 14 independent settings / 20 zone fields / 9 spawn fields / 35 genes /
9 material fields / enums all match the source files. The generator script is not part of the repo
(scratch only); the values are normative and unit tests must assert them.

## A.1 `ZoneSettings` C# fields (for reflection-based key resolution)

The JSON keys are custom (3.5); this table lists the C# field names that appear in animator keys
`Zone(<id>).<field>` and their defaults/ranges:

| # | field | type | UI label | default | min | max | units | src |
|---|-------|------|----------|---------|-----|-----|-------|-----|
| 1 | `zoneName` | string | Name | - | - | - | - | 20 |
| 2 | `spawnMaterial` | string (material name) | Spawn Pellet of Type | `Plant` | - | - | - | 25 |
| 3 | `distribution` | enum SpawnDistribution | Spawn distribution | `CentricGradual` | - | - | - | 34 |
| 4 | `biomassDensity` | f32 | Biomass | `1` | 0.001 | 100 | `x` | 43 |
| 5 | `fertility` | f32 | Fertility | `1` | 0.001 | 100 | `x` | 57 |
| 6 | `pelletSize` | f32 | Pellet Size | `1` | 0.01 | 50 | `x` | 70 |
| 7 | `renewBiomass` | bool | Renew Biomass | `false` | - | - | - | 83 |
| 8 | `movement` | enum MovementType | Movement | `None` | - | - | - | 91 |
| 9 | `target` | int (zone id) | Target | - | - | - | - | 100 |
| 10 | `speed` | f32 | Speed Factor | `1` | 0.001 | 1000 | - | 106 |
| 11 | `posX` | f32 | Pos X | `0` | -1 | 1 | - | 117 |
| 12 | `posY` | f32 | Pos Y | `0` | -1 | 1 | - | 129 |
| 13 | `sizeScalesWithSim` | bool | Size scales with sim | `true` | - | - | - | 141 |
| 14 | `radiusRelative` | f32 | Radius | `0.25` | 0 | 1 | `x` | 149 |
| 15 | `radiusAbsolute` | f32 | Radius | `1000` | 0 | 25000 | `u` | 163 |
| 16 | `insideRadius` | f32 | Inside radius | `0.8` | 0 | 0.999 | `x` | 177 |
| 17 | `widthRelative` | f32 | Width | `0.25` | 0 | 2 | `x` | 191 |
| 18 | `widthAbsolute` | f32 | Width | `1000` | 0 | 25000 | `u` | 205 |
| 19 | `heightRelative` | f32 | Height | `0.5` | 0 | 2 | `x` | 219 |
| 20 | `heightAbsolute` | f32 | Height | `1000` | 0 | 25000 | `u` | 233 |

## A.2 Enumerations (wire values/names)

### Genes

| value | int |
|-------|-----|
| `LayTime` | 0 |
| `BroodTime` | 1 |
| `HatchTime` | 2 |
| `SizeRatio` | 3 |
| `SpeedRatio` | 4 |
| `ColorR` | 5 |
| `ColorG` | 6 |
| `ColorB` | 7 |
| `MutationAmountSigma` | 8 |
| `AverageMutationNumber` | 9 |
| `BrainMutationSigma` | 10 |
| `BrainAverageMutation` | 11 |
| `ViewAngle` | 12 |
| `ViewRadius` | 13 |
| `ClockSpeed` | 14 |
| `PheroSense` | 15 |
| `Diet` | 16 |
| `HerdSeparationWeight` | 17 |
| `HerdAlignmentWeight` | 18 |
| `HerdCohesionWeight` | 19 |
| `HerdVelocityWeight` | 20 |
| `HerdSeparationDistance` | 21 |
| `GrowthScale` | 22 |
| `GrowthMaturityFactor` | 23 |
| `GrowthMaturityExponent` | 24 |
| `EyeOffset` | 25 |
| `StomachWAG` | 26 |
| `WombWAG` | 27 |
| `FatWAG` | 28 |
| `ArmorWAG` | 29 |
| `ThroatWAG` | 30 |
| `MouthMusclesWAG` | 31 |
| `MoveMusclesWAG` | 32 |
| `FatStorageThreshold` | 33 |
| `FatStorageDeadband` | 34 |

### Inputs

| value | int |
|-------|-----|
| `EnergyRatio` | 0 |
| `Maturity` | 1 |
| `LifeRatio` | 2 |
| `Fullness` | 3 |
| `Speed` | 4 |
| `RotationSpeed` | 5 |
| `IsGrabbing` | 6 |
| `AttackedDamage` | 7 |
| `EggStored` | 8 |
| `BibiteCloseness` | 9 |
| `BibiteAngle` | 10 |
| `NBibites` | 11 |
| `PlantCloseness` | 12 |
| `PlantAngle` | 13 |
| `NPlants` | 14 |
| `MeatCloseness` | 15 |
| `MeatAngle` | 16 |
| `NMeats` | 17 |
| `RedBibite` | 18 |
| `GreenBibite` | 19 |
| `BlueBibite` | 20 |
| `Tic` | 21 |
| `Minute` | 22 |
| `TimeAlive` | 23 |
| `PheroSense1` | 24 |
| `PheroSense2` | 25 |
| `PheroSense3` | 26 |
| `Phero1Angle` | 27 |
| `Phero2Angle` | 28 |
| `Phero3Angle` | 29 |
| `Phero1Heading` | 30 |
| `Phero2Heading` | 31 |
| `Phero3Heading` | 32 |

### Outputs

| value | int |
|-------|-----|
| `Accelerate` | 0 |
| `Rotate` | 1 |
| `Herding` | 2 |
| `EggProduction` | 3 |
| `Want2Lay` | 4 |
| `Want2Eat` | 5 |
| `Digestion` | 6 |
| `Grab` | 7 |
| `ClkReset` | 8 |
| `PhereOut1` | 9 |
| `PhereOut2` | 10 |
| `PhereOut3` | 11 |
| `Want2Grow` | 12 |
| `Want2Heal` | 13 |
| `Want2Attack` | 14 |

### NodeType

| value | int |
|-------|-----|
| `Input` | 0 |
| `Sigmoid` | 1 |
| `Linear` | 2 |
| `TanH` | 3 |
| `Sine` | 4 |
| `ReLu` | 5 |
| `Gaussian` | 6 |
| `Latch` | 7 |
| `Differential` | 8 |
| `Abs` | 9 |
| `Mult` | 10 |
| `Integrator` | 11 |
| `Inhibitory` | 12 |
| `SoftLatch` | 13 |

### NodeArchetype

| value | int |
|-------|-----|
| `Input` | 0 |
| `Output` | 1 |
| `Hidden` | 2 |

### SpawnDistribution

| value | int |
|-------|-----|
| `Flat` | 0 |
| `CentricGradual` | 1 |
| `ExteriorGradual` | 2 |
| `Ring` | 3 |
| `FlatRing` | 4 |
| `Rect` | 5 |

### MovementType

| value | int |
|-------|-----|
| `None` | 0 |
| `Free` | 1 |
| `Repulsed` | 2 |
| `Orbit` | 3 |
| `Attached` | 4 |

### Tagging

| value | int |
|-------|-----|
| `NoTagging` | 0 |
| `SpeciesTagging` | 1 |
| `RandomTagging` | 2 |
| `CustomTagging` | 3 |

### RandomizeGenes

| value | int |
|-------|-----|
| `No` | 0 |
| `NormalMutations` | 1 |
| `Color` | 2 |
| `AllGenes` | 3 |

### GrowthAtSpawn

| value | int |
|-------|-----|
| `Egg` | 0 |
| `Baby` | 1 |
| `Adult` | 2 |
| `Elder` | 3 |

### SpawnType

| value | int |
|-------|-----|
| `Continuous` | 0 |
| `OneTime` | 1 |

### SimMetric

| value | int |
|-------|-----|
| `SpeciesCount` | 0 |
| `SpeciesBiomass` | 1 |
| `TagCount` | 2 |
| `TagBiomass` | 3 |
| `MaterialCount` | 4 |
| `MaterialBiomass` | 5 |
| `Time` | 6 |
| `Constant` | 7 |

### Comparator

| value | int |
|-------|-----|
| `LessThan` | 0 |
| `LessThanOrEqual` | 1 |
| `GreaterThan` | 2 |
| `GreaterThanOrEqual` | 3 |
| `Equal` | 4 |
| `NotEqual` | 5 |

### ConditionGroupLogic

| value | int |
|-------|-----|
| `And` | 0 |
| `Or` | 1 |

### InterpolationType

| value | int |
|-------|-----|
| `Linear` | 0 |
| `EaseIn` | 1 |
| `EaseOut` | 2 |
| `EaseInOut` | 3 |

## A.3 Tower settings (scene objects)

| object | JSON key | type | label | default | min | max | units | src |
|--------|----------|------|-------|---------|-----|-----|-------|-----|
| RadTower | `count` | f32 | Count Rate | `0.001` | 0 | 0.1 | `N/s` | 11 |
| RadTower | `intensity` | f32 | Intensity Rate | `0.001` | 0 | 0.1 | `%/s` | 25 |
| RadTower | `radius` | f32 | Radius | `250` | 10 | 10000 | `u` | 40 |
| RadTower | `taper` | f32 | Taper Factor | `1` | 0 | 3 | - | 54 |
| PheroTower | `redStrength` | f32 | Red Phero | `0` | 0 | 100 | `u^2` | 12 |
| PheroTower | `greenStrength` | f32 | Green Phero | `0` | 0 | 100 | `u^2` | 25 |
| PheroTower | `blueStrength` | f32 | Blue Phero | `0` | 0 | 100 | `u^2` | 38 |
| PheroTower | `heading` | f32 | Heading | `0` | 0 | (float)Math.PI * 2 | - | 51 |

## A.4 Three representative real files

**Simplest valid save settings** (from `Scenarios\Default.zip`; note optional zone keys omitted,
`recommended` present, no `interactableChanged` wrapper - an older writer, accepted by the loader):

```json
{
  "zones": [
    {"name":"World","material":"Plant","posX":0.0,"posY":0.0,"radius":1.0,
     "fertility":0.7827918,"speed":0.0,"pelletSize":1.0}
  ],
  "bibites": [
    {"path":"Basic bibite.bb8template","spawnPriority":1.0,"randomizeColor":false,
     "randomizeGenes":false,"tagging":"NoTagging"}
  ],
  "recommended": {
    "SimulationSize":{"Value":2000.0},
    "biomassDensity":{"Value":0.01},
    "pelletGrowth":{"Value":0.00001}
  }
}
```

Note: `randomizeColor` is a legacy key ignored by the current loader; `distribution` and
`biomassDensity` are omitted and default to `CentricGradual` / `1`.

**Modern setting wrapper** (from `autosave_20261008004410.zip`):

```json
"backgroundMutationChance":{"interactableChanged":{"m_PersistentCalls":{"m_Calls":[]}},"Value":12.0}
"independents":{"SimulationSize":{"interactableChanged":{"m_PersistentCalls":{"m_Calls":""}},"Value":2000.0}, ...}
```

(the `""` vs `[]` difference is a Unity quirk; both are accepted - our writer always emits `[]`.)

**Minimal template** (verified against `Sample.bb8template`):

```json
{"name":"Sample","speciesName":"Sample sample","generation":801,"version":"0.6.3.1",
 "nodes":[{"Type":0,"Index":0,"Inov":1,"Desc":"EnergyRatio","baseActivation":0.0}, ...],
 "synapses":[{"Inov":62,"NodeIn":12,"NodeOut":33,"Weight":0.7165215,"En":false}, ...],
 "genes":{"LayTime":30.984577,...35 floats...}}
```

## A.5 Default built-in template and template rewriting

`AppInitializer.Awake` loads all `Resources/BibiteTemplates/*` TextAssets, and for each **deletes and
rewrites** `<persistentDataPath>/Bibites/Templates/<name>.bb8template` on every startup, adding the
lower-cased name to `GameManager.defaultBibites` (`Here/ManagementScripts/AppInitializer.cs:49-90`).
Consequences:

* These built-ins cannot be "edited" persistently: the game overwrites them next launch.
* Their on-disk files are shipped in the **old** 34-gene/old-brain layout and are upgraded in
  memory when used; a tool reading them must run the gene/brain migrations (4.7).
* When a save/scenario bundles a template with the same name as a default template,
  `CheckTemplatesOfArchive` will **not** overwrite it (defaults are in `GameManager.defaultBibites`),
  and `AddTemplatesToArchive` **skips** default templates when exporting (`SaveSystem.cs:885-912`).
  Our exporter should mirror this: do not bundle templates whose name (lower-cased) is a built-in
  default.

## A.6 Where the decompiled source ends and real data continues

| Topic | Source (`Here/`) | Real 0.6.3.1 data | Notes |
|---|---|---|---|
| material defaults | ScriptableObject assets (not in DLL) | mode of 372 saves | table 3.4 |
| 78 scalar defaults | yes | saves contain user-modified values | table 3.3 |
| `nPellets`/`nBibites` semantics | code only | display counters | not used on load |
| `data.bin` full layout | classes present | real blobs | opaque in M1 (5.8) |
| 0.6.3.3/0.6.4a1 extras | absent | present in other builds' saves | preserved as unknowns |
