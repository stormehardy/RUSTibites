//! Simulation settings: typed model and exact defaults. Owner: fmt-dev.
//!
//! This module is the single source of truth for every simulation setting: the
//! exact field types, names, default values, ranges, units and runtime
//! mutability rules extracted from the decompiled game. It is pure data plus
//! small accessors: no simulation logic, no I/O and no external dependencies.
//!
//! Spec: `docs/spec/04-formats-settings.md`, sections 2, 3.2, 3.3, 3.4 and 7.1;
//! interface contract: `docs/INTERFACES.md` section 3.
//!
//! # Three scopes
//!
//! The original game splits settings into three serialized scopes:
//!
//! * [`crate::settings::ScenarioSettings`] - the 78 "scenario" settings shared by every world
//!   (mutation, collisions, metabolism, food, healing, ...). In the game's JSON
//!   they are the top-level keys of `settings.bb8settings`, each wrapped in the
//!   Unity `{ "interactableChanged": ..., "Value": x }` object (spec 2).
//! * [`crate::settings::IndependentSettings`] - the 14 "scenario-independent" settings
//!   (simulation size, TPS, vision factors, species limits). They are nested
//!   under `independents` in saves and `recommended` in scenarios (spec 3.2).
//! * [`crate::settings::MaterialSettings`] - physical parameters for the four matter materials
//!   (Plant, Meat, Fat, Armor), stored under `materials` (spec 3.4).
//!
//! [`crate::settings::SimSettings`] aggregates all three plus the five rate values that the
//! engine's hot loop needs, and is what every other module consumes read-only.
//!
//! # Naming
//!
//! Every Rust field is the `snake_case` form of the game's JSON key (the C#
//! field name): `backgroundMutationChance` -> `background_mutation_chance`,
//! `simTPS` -> `sim_tps`, `SimulationSize` -> `simulation_size`,
//! `ageingThreshold` -> `ageing_threshold` (British spelling preserved). The
//! original JSON key is available at runtime through [`crate::settings::SettingMeta::key`], so
//! file-format code never needs to duplicate the mapping.
//!
//! # Exact defaults
//!
//! The defaults are the `DefaultValue` literals from the C# source
//! (`Here/SettingScripts/ScenarioSettings.cs`,
//! `ScenarioIndependentSettings.cs`, `MatterMaterialSettings.cs`, plus the
//! shipped `MatterMaterial` assets for material values). A table-driven test
//! (`defaults_match_spec_tables`) asserts every one of the 78 + 14 defaults and
//! all 4 x 9 material values against the spec; another asserts every
//! min/max range. If a default ever drifts, `cargo test -p bibites-core` fails.
//!
//! # Key-based access (for `bibites-io`)
//!
//! Each scope exposes:
//!
//! * [`crate::settings::SettingMeta`] tables: `ScenarioSettings::SETTINGS`,
//!   `IndependentSettings::SETTINGS`, `MaterialSetting::FIELDS`.
//! * `meta(key)` for metadata lookup.
//! * `get_by_key(key)` / `set_by_key(key, value)` for generic JSON bridging.
//! * `set_by_key` mirrors the game's `SetValue` clamp rules: only settings whose
//!   UI range is hard (`clamped == true`, the game's `canGoOutOfBounds=false`)
//!   are clamped. [`crate::settings::ScenarioSettings::clamp_all`] (and the equivalents) clamps
//!   every numeric field and is the import policy: file loaders call it after
//!   applying a file and log when it reports `> 0`.
//!
//! Unknown keys return `None`; unknown JSON fields are the file layer's job to
//! preserve (such a value is never rejected here).

/// The storage kind of a setting value.
///
/// Mirrors the three `Setting<T>` subclasses the game serializes: `FloatSetting`
/// (`f32`), `IntSetting` (`i32`) and `BoolSetting` (`bool`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingKind {
    /// Single-precision floating point (`FloatSetting`).
    Float,
    /// Signed 32-bit integer (`IntSetting`).
    Int,
    /// Boolean (`BoolSetting`).
    Bool,
}

/// A dynamically typed setting value.
///
/// Used by the key-based accessors so that the file layer can bridge JSON
/// without duplicating every field. The variant must match the metadata's
/// [`SettingKind`]; mismatched writes are rejected (`None`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingValue {
    /// A `FloatSetting` value.
    Float(f32),
    /// An `IntSetting` value.
    Int(i32),
    /// A `BoolSetting` value.
    Bool(bool),
}

impl SettingValue {
    /// Returns this value's [`SettingKind`].
    #[must_use]
    pub fn kind(self) -> SettingKind {
        match self {
            Self::Float(_) => SettingKind::Float,
            Self::Int(_) => SettingKind::Int,
            Self::Bool(_) => SettingKind::Bool,
        }
    }

    /// Returns the value as `f64` for range checks and reporting.
    ///
    /// Booleans map to `1.0`/`0.0`. The `f32` widening is exact, so comparing
    /// this against a `f64` range bound is correct.
    #[must_use]
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Float(v) => f64::from(v),
            Self::Int(v) => f64::from(v),
            Self::Bool(v) => {
                if v {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// Returns the contained `f32`, or `None` for other kinds.
    #[must_use]
    pub fn as_f32(self) -> Option<f32> {
        match self {
            Self::Float(v) => Some(v),
            _ => None,
        }
    }

    /// Returns the contained `i32`, or `None` for other kinds.
    #[must_use]
    pub fn as_i32(self) -> Option<i32> {
        match self {
            Self::Int(v) => Some(v),
            _ => None,
        }
    }

    /// Returns the contained `bool`, or `None` for other kinds.
    #[must_use]
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(v),
            _ => None,
        }
    }
}

/// Static metadata for one setting: JSON key, type, default, range, units and
/// the game's mutability rules.
///
/// The metadata order inside a scope's table is the game's serialization order
/// (the C# field declaration order), which is also the canonical write order in
/// `docs/spec/04-formats-settings.md`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingMeta {
    /// The game's JSON key (C# field name), e.g. `"backgroundMutationChance"`
    /// or `"SimulationSize"`.
    pub key: &'static str,
    /// Storage kind (`Float`, `Int`, `Bool`).
    pub kind: SettingKind,
    /// The exact default value from the C# initializer.
    pub default: SettingValue,
    /// Inclusive minimum, or `None` for booleans / unbounded settings.
    pub min: Option<f64>,
    /// Inclusive maximum, or `None` for booleans / unbounded settings.
    pub max: Option<f64>,
    /// UI unit suffix exactly as documented (e.g. `"%`, `"E/u^2*s"`); empty
    /// when the setting is unitless. Display-only; never affects simulation.
    pub units: &'static str,
    /// Whether the game's setter hard-clamps to `[min, max]` (C#
    /// `canGoOutOfBounds == false`). `false` means the game accepts values
    /// outside the UI range for that setting.
    pub clamped: bool,
    /// Whether a scenario animator (`SettingsChanger`) may target this setting
    /// (C# `controllable != false`).
    pub animatable: bool,
    /// Human-readable label from the game's settings UI.
    pub label: &'static str,
}

impl SettingMeta {
    /// Clamps `value` to this setting's inclusive range.
    ///
    /// Non-numeric values are returned unchanged; a `Float` stays `f32` and an
    /// `Int` stays `i32`. This helper is used by
    /// [`ScenarioSettings::set_by_key`] and friends when the game itself
    /// clamps, and by the import-time `clamp_all` helpers.
    #[must_use]
    pub fn clamp_value(self, value: SettingValue) -> SettingValue {
        match value {
            SettingValue::Float(v) => {
                let mut v = v;
                if let Some(min) = self.min {
                    v = v.max(min as f32);
                }
                if let Some(max) = self.max {
                    v = v.min(max as f32);
                }
                SettingValue::Float(v)
            }
            SettingValue::Int(v) => {
                let mut v = v;
                if let Some(min) = self.min {
                    v = v.max(min as i32);
                }
                if let Some(max) = self.max {
                    v = v.min(max as i32);
                }
                SettingValue::Int(v)
            }
            other => other,
        }
    }
}

/// Private helper used by the generated `clamp_all` methods: clamps a single
/// field in place, returning whether it changed.
trait ClampSetting: Copy {
    fn clamp_setting(&mut self, min: Option<f64>, max: Option<f64>) -> bool;
}

impl ClampSetting for f32 {
    fn clamp_setting(&mut self, min: Option<f64>, max: Option<f64>) -> bool {
        let mut clamped = *self;
        if let Some(min) = min {
            clamped = clamped.max(min as f32);
        }
        if let Some(max) = max {
            clamped = clamped.min(max as f32);
        }
        if clamped == *self {
            false
        } else {
            *self = clamped;
            true
        }
    }
}

impl ClampSetting for i32 {
    fn clamp_setting(&mut self, min: Option<f64>, max: Option<f64>) -> bool {
        let mut clamped = *self;
        if let Some(min) = min {
            clamped = clamped.max(min as i32);
        }
        if let Some(max) = max {
            clamped = clamped.min(max as i32);
        }
        if clamped == *self {
            false
        } else {
            *self = clamped;
            true
        }
    }
}

impl ClampSetting for bool {
    fn clamp_setting(&mut self, _min: Option<f64>, _max: Option<f64>) -> bool {
        false
    }
}

/// Defines one settings scope: the struct, its exact [`Default`] values, its
/// [`crate::settings::SettingMeta`] table and the key-based accessors.
///
/// Each entry is
/// `(field, json_key, type, SettingValue variant, default, min, max, units, clamped, animatable, label)`.
/// The table order is the canonical serialization order; keep entries in
/// `docs/spec/04-formats-settings.md` table order.
macro_rules! setting_scope {
    (
        $(#[$attr:meta])*
        $scope:ident {
            $( ($field:ident, $json:literal, $ty:ty, $variant:ident, $default:expr, $min:expr,
                $max:expr, $units:literal, $clamped:literal, $animatable:literal,
                $label:literal) ),* $(,)?
        }
    ) => {
        $(#[$attr])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $scope {
            $(
                #[doc = $label]
                pub $field: $ty,
            )*
        }

        impl $scope {
            /// Number of settings in this scope.
            pub const COUNT: usize = Self::SETTINGS.len();

            /// Metadata for every setting in this scope, in canonical
            /// serialization order.
            pub const SETTINGS: &[SettingMeta] = &[
                $(
                    SettingMeta {
                        key: $json,
                        kind: SettingKind::$variant,
                        default: SettingValue::$variant($default),
                        min: $min,
                        max: $max,
                        units: $units,
                        clamped: $clamped,
                        animatable: $animatable,
                        label: $label,
                    },
                )*
            ];

            /// Returns the metadata for `key`, or `None` if it is not part of
            /// this scope.
            #[must_use]
            pub fn meta(key: &str) -> Option<&'static SettingMeta> {
                Self::SETTINGS.iter().find(|meta| meta.key == key)
            }

            /// Returns a fresh instance with every field set to its exact
            /// game default.
            #[must_use]
            pub fn game_defaults() -> Self {
                Self {
                    $( $field: $default, )*
                }
            }

            /// Returns the current value of `key`, or `None` for unknown keys.
            #[must_use]
            pub fn get_by_key(&self, key: &str) -> Option<SettingValue> {
                match key {
                    $( $json => Some(SettingValue::$variant(self.$field)), )*
                    _ => None,
                }
            }

            /// Applies `value` to `key` without any range handling.
            ///
            /// Returns the applied value, or `None` when the key is unknown or
            /// the value's variant does not match the setting's kind.
            pub fn set_raw(&mut self, key: &str, value: SettingValue) -> Option<SettingValue> {
                match (key, value) {
                    $(
                        ($json, SettingValue::$variant(v)) => {
                            self.$field = v;
                            Some(SettingValue::$variant(v))
                        }
                    )*
                    _ => None,
                }
            }

            /// Sets `key` to `value`, mirroring the game's `SetValue` clamp
            /// rules: hard-clamped settings (`clamped == true`) are clamped to
            /// `[min, max]`, all others are stored verbatim.
            ///
            /// Returns the applied value, or `None` for unknown keys or a
            /// kind mismatch. Unknown JSON keys are the file layer's concern.
            pub fn set_by_key(&mut self, key: &str, value: SettingValue) -> Option<SettingValue> {
                let meta = Self::meta(key)?;
                let value = if meta.clamped {
                    meta.clamp_value(value)
                } else {
                    value
                };
                self.set_raw(key, value)
            }

            /// Clamps only the fields the game's own setters hard-clamp
            /// (`clamped == true`); returns how many changed.
            ///
            /// This mirrors the game's `SetValue` behaviour exactly, so it is
            /// safe to call after a runtime animator write.
            pub fn clamp_all(&mut self) -> usize {
                let mut clamped = 0;
                $(
                    if $clamped && ClampSetting::clamp_setting(&mut self.$field, $min, $max) {
                        clamped += 1;
                    }
                )*
                clamped
            }

            /// Clamps **every** numeric field to its inclusive range; returns
            /// how many changed.
            ///
            /// This is the import-hardening policy (`docs/spec/04-formats-settings.md`
            /// Q6/9.7): file loaders apply a file, then call this and log when
            /// it reports `> 0`. The game's own JSON load does not clamp, so
            /// this is deliberately stricter than [`Self::set_by_key`].
            pub fn clamp_to_ranges(&mut self) -> usize {
                let mut clamped = 0;
                $(
                    if ClampSetting::clamp_setting(&mut self.$field, $min, $max) {
                        clamped += 1;
                    }
                )*
                clamped
            }
        }

        impl Default for $scope {
            fn default() -> Self {
                Self::game_defaults()
            }
        }
    };
}

/// Complete effective simulation settings.
///
/// Built by `bibites-io` from a scenario/save/settings file, or from
/// [`SimSettings::game_defaults`] for a vanilla run. Every subsystem reads it
/// through a shared reference and must not cache values across ticks (runtime
/// settings animators may mutate them; `docs/INTERFACES.md` section 3).
///
/// The five broadcast rate fields mirror the corresponding
/// [`crate::settings::IndependentSettings`] fields so the hot loop does not have to reach through
/// two levels of nesting. They are authoritative at runtime; the nested
/// `independents` values are what gets serialized. File loaders must call
/// [`SimSettings::sync_rates_from_independents`] after loading (or construct via
/// [`SimSettings::from_parts`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SimSettings {
    /// Simulation ticks per second (default 30; `1 / sim_tps` is the fixed
    /// timestep). Mirrors `independents.sim_tps`.
    pub sim_tps: u32,
    /// The brain updates every `brain_tps` simulation ticks (default 2, i.e.
    /// 15 Hz at 30 TPS). Mirrors `independents.brain_tps`.
    pub brain_tps: u32,
    /// Material decay updates every `decay_tps` simulation ticks (default 2).
    /// Mirrors `independents.decay_tps`.
    pub decay_tps: u32,
    /// Number of brain updates between field-of-view entity lookups (default
    /// 10). Mirrors `independents.vision_lookup_update_factor`.
    pub vision_lookup_updates: u32,
    /// Number of brain updates between vision sense updates (default 5).
    /// Mirrors `independents.vision_sense_update_factor`.
    pub vision_sense_updates: u32,
    /// Lower bound of `pelletEnergy` (`ScenarioSettings::meta("pelletEnergy").min`,
    /// default 10). The game's zone spawning clamps each pellet's average energy
    /// to at least this value (`Zone.SpawnAPellet`: `max(avgPelletEnergy, pelletEnergy.minValue)`),
    /// so it is load-bearing runtime state, not just UI metadata.
    pub pellet_energy_min: f32,
    /// User setting `UserSettings.AllowEasterEggs` (default off): a mutated child may be born a radioactive, rainbow, producer or reaper
    /// bibite (`BibiteGenes.Mutate`). A player's preference, not part of a scenario or a save: the viewer sets it.
    pub allow_easter_eggs: bool,
    /// User setting `UserSettings.EasterEggsProbability` (default 500 000, 1 to 100 000 000): on average one child in this many is an
    /// easter egg while they are allowed.
    pub easter_eggs_period: u32,
    /// The 78 shared simulation settings.
    pub scenario: ScenarioSettings,
    /// The 14 scenario-independent settings.
    pub independents: IndependentSettings,
    /// Per-material physical parameters.
    pub materials: MaterialSettings,
}

impl SimSettings {
    /// Returns the exact settings of a vanilla run: all game defaults, 30 TPS,
    /// brain every 2 ticks, decay every 2 ticks, vision lookup every 10 brain
    /// updates, vision senses every 5 brain updates.
    #[must_use]
    pub fn game_defaults() -> Self {
        let independents = IndependentSettings::default();
        Self::from_parts(
            ScenarioSettings::default(),
            independents,
            MaterialSettings::default(),
        )
    }

    /// Builds an aggregate from already-parsed scopes and synchronises the
    /// broadcast rate fields with `independents`.
    #[must_use]
    pub fn from_parts(
        scenario: ScenarioSettings,
        independents: IndependentSettings,
        materials: MaterialSettings,
    ) -> Self {
        let pellet_energy_min = scenario_pellet_energy_min();
        let mut settings = Self {
            sim_tps: 0,
            brain_tps: 0,
            decay_tps: 0,
            vision_lookup_updates: 0,
            vision_sense_updates: 0,
            pellet_energy_min,
            allow_easter_eggs: false,
            easter_eggs_period: 500_000,
            scenario,
            independents,
            materials,
        };
        settings.sync_rates_from_independents();
        settings
    }

    /// Copies the serialized rate settings into the broadcast fields.
    ///
    /// Values are clamped to a sane positive range (`<= 1` becomes `1`) so that
    /// malformed files cannot produce a division by zero; valid files always
    /// come from the game's own constrained setters (5..100 TPS, factors
    /// >= 1) and are unaffected. File loaders call this after applying a file.
    pub fn sync_rates_from_independents(&mut self) {
        self.sim_tps = positive_u32(self.independents.sim_tps);
        self.brain_tps = positive_u32(self.independents.brain_tps);
        self.decay_tps = positive_u32(self.independents.decay_tps);
        self.vision_lookup_updates = positive_u32(self.independents.vision_lookup_update_factor);
        self.vision_sense_updates = positive_u32(self.independents.vision_sense_update_factor);
    }

    /// Period passed to stateful brain neurons, in seconds:
    /// `brain_tps / sim_tps` (default `2 / 30 = 1/15`). `docs/ARCHITECTURE.md`
    /// section 6.
    ///
    /// # Panics
    ///
    /// Debug-asserts that `sim_tps > 0`; [`SimSettings::game_defaults`] and
    /// [`SimSettings::sync_rates_from_independents`] guarantee this. In release
    /// builds a zero `sim_tps` yields infinity rather than panicking.
    #[must_use]
    pub fn brain_period(&self) -> f32 {
        debug_assert!(self.sim_tps > 0, "sim_tps must be positive");
        self.brain_tps as f32 / self.sim_tps as f32
    }

    /// Fixed simulation timestep in seconds: `1 / sim_tps` (default `1/30`).
    ///
    /// # Panics
    ///
    /// Debug-asserts that `sim_tps > 0`; see [`SimSettings::brain_period`].
    #[must_use]
    pub fn fixed_dt(&self) -> f32 {
        debug_assert!(self.sim_tps > 0, "sim_tps must be positive");
        1.0 / self.sim_tps as f32
    }

    /// The number of simulation ticks between two brain updates.
    #[must_use]
    pub fn brain_ticks(&self) -> u32 {
        self.brain_tps
    }

    /// Brain updates per second (`sim_tps / brain_tps`); 15 Hz at the defaults.
    #[must_use]
    pub fn brain_hz(&self) -> f32 {
        self.sim_tps as f32 / self.brain_tps as f32
    }
}

impl Default for SimSettings {
    fn default() -> Self {
        Self::game_defaults()
    }
}

/// Converts a serialized rate-like integer into a positive `u32`, clamping
/// `<= 0` to `1` (defensive: the game's own UI ranges are 1..=100).
#[must_use]
fn positive_u32(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(1).max(1)
}

/// The lower runtime bound of pellet energy, derived from the scenario
/// setting's metadata (`pelletEnergy.minValue` in the game, default 10).
#[must_use]
fn scenario_pellet_energy_min() -> f32 {
    ScenarioSettings::meta("pelletEnergy")
        .and_then(|meta| meta.min)
        .unwrap_or(10.0) as f32
}

setting_scope! {
    /// The 78 scenario settings shared by every world.
    ScenarioSettings {
    (background_mutation_chance, "backgroundMutationChance", f32, Float, 1.5_f32, Some(0.0_f64), Some(20.0_f64), "", false, true, "Background Mutation Chance"),
    (background_mutation_variance, "backgroundMutationVariance", f32, Float, 0.05_f32, Some(0.0_f64), Some(0.5_f64), "%", false, true, "Background Mutation Variance"),
    (relative_mutation_share, "relativeMutationShare", f32, Float, 0.75_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Mutation Value Relativity"),
    (synapse_mutation_chance, "synapseMutationChance", f32, Float, 0.6_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Synapse Mutation Probability"),
    (neuron_mutation_chance, "neuronMutationChance", f32, Float, 0.4_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Neuron Mutation Probability"),
    (synapse_change_chance, "synapseChangeChance", f32, Float, 0.75_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Synapse Strength Mutation Probability"),
    (synapse_flip_chance, "synapseFlipChance", f32, Float, 0.025_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Synapse Flip Probability"),
    (synapse_toggle_chance, "synapseToggleChance", f32, Float, 0.025_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Synapse Toggle Probability"),
    (synapse_add_chance, "synapseAddChance", f32, Float, 0.1_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Synapse Add Probability"),
    (synapse_remove_chance, "synapseRemoveChance", f32, Float, 0.1_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Synapse Removal Probability"),
    (neuron_default_chance, "neuronDefaultChance", f32, Float, 0.7_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Default Activation Mutation Probability"),
    (neuron_change_chance, "neuronChangeChance", f32, Float, 0.1_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Neuron Function Mutation Probability"),
    (neuron_add_chance, "neuronAddChance", f32, Float, 0.1_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Neuron Add Probability"),
    (neuron_remove_chance, "neuronRemoveChance", f32, Float, 0.1_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Neuron Removal Probability"),
    (pellet_collision, "pelletCollision", bool, Bool, true, None, None, "", false, true, "Pellet Collision"),
    (pellet_merge, "pelletMerge", bool, Bool, false, None, None, "", false, true, "Pellet Merge"),
    (pellet_rotation, "pelletRotation", bool, Bool, true, None, None, "", false, true, "Pellet Rotation"),
    (egg_collision, "eggCollision", bool, Bool, true, None, None, "", false, true, "Egg Collision"),
    (disable_herding, "disableHerding", bool, Bool, false, None, None, "", false, true, "Disable Herding"),
    (min_pellet_size, "minPelletSize", f32, Float, 0.1_f32, Some(0.001_f64), Some(10.0_f64), "u^2", true, true, "Minimum Pellet Size"),
    (corpses_enabled, "corpsesEnabled", bool, Bool, true, None, None, "", false, true, "Corpses"),
    (prevent_mutations, "preventMutations", bool, Bool, false, None, None, "", false, true, "Prevent Mutations"),
    (void_avoidance, "voidAvoidance", bool, Bool, false, None, None, "", false, true, "Void-No-Mo'"),
    (void_avoidance_distance, "voidAvoidanceDistance", f32, Float, 100.0_f32, Some(0.0001_f64), Some(1000.0_f64), "u", false, true, "Void Avoidance Distance"),
    (prevent_red_phero_production, "preventRedPheroProduction", bool, Bool, false, None, None, "", false, true, "Prevent Red Pheromones Production"),
    (prevent_green_phero_production, "preventGreenPheroProduction", bool, Bool, false, None, None, "", false, true, "Prevent Green Pheromones Production"),
    (prevent_blue_phero_production, "preventBluePheroProduction", bool, Bool, false, None, None, "", false, true, "Prevent Blue Pheromones Production"),
    (enable_red_death, "enableRedDeath", bool, Bool, false, None, None, "", false, true, "The Red Death Bloom"),
    (red_death_fill, "redDeathFill", f32, Float, 0.0_f32, Some(0.0_f64), Some(1.0_f64), "%", true, true, "Red Death Fill"),
    (red_death_base_damages, "redDeathBaseDamages", f32, Float, 0.5_f32, Some(0.0_f64), Some(100.0_f64), "HP/s", false, true, "Red Death Base Damage"),
    (red_death_damage_velocity, "redDeathDamageVelocity", f32, Float, 0.15_f32, Some(0.0_f64), Some(10.0_f64), "HP/s^2", false, true, "Red Death Damages Velocity"),
    (shade_outside_of_bounds, "shadeOutsideOfBounds", bool, Bool, true, None, None, "", false, true, "Add Shaded Bounds"),
    (shade_avoidance, "shadeAvoidance", bool, Bool, false, None, None, "", false, true, "Bibites Fear Shade"),
    (world_wrapping, "worldWrapping", bool, Bool, true, None, None, "", false, true, "World Wraps Behind Shade"),
    (initial_seeding, "initialSeeding", f32, Float, 0.75_f32, Some(0.0_f64), Some(1.0_f64), "%", false, false, "Initial Seeding"),
    (pellet_energy, "pelletEnergy", f32, Float, 200.0_f32, Some(10.0_f64), Some(5000.0_f64), "E", false, true, "Default pellet Size"),
    (fat_sustain, "fatSustain", f32, Float, 0.0_f32, Some(0.0_f64), Some(0.05_f64), "%/s", false, true, "Fat Storage Cost"),
    (drag_coefficient, "dragCoefficient", f32, Float, 5.0_f32, Some(0.0_f64), Some(25.0_f64), "Nu/s", false, true, "Drag Coefficient"),
    (disable_drag_on_held_pellet, "disableDragOnHeldPellet", bool, Bool, false, None, None, "", false, true, "Disable Drag On Held Pellet"),
    (arm_muscle_pressure, "armMusclePressure", f32, Float, 50.0_f32, Some(1.0_f64), Some(1000.0_f64), "N/u", false, true, "Arm Muscles Force"),
    (forward_force_size_power, "forwardForceSizePower", f32, Float, 1.5_f32, Some(0.0_f64), Some(2.5_f64), "", false, true, "Muscles Sizing Propel Power"),
    (turn_force_size_power, "turnForceSizePower", f32, Float, 3.0_f32, Some(0.0_f64), Some(4.0_f64), "", false, true, "Muscles Sizing Turning Power"),
    (backward_fraction, "backwardFraction", f32, Float, 0.5_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Backward Force Efficiency"),
    (bibite_mass_density, "bibiteMassDensity", f32, Float, 0.025_f32, Some(0.001_f64), Some(0.25_f64), "g/u^2", false, true, "Base Bibite Mass Density"),
    (stomach_content_contribution, "stomachContentContribution", f32, Float, 1.0_f32, Some(0.0_f64), Some(2.0_f64), "%", false, true, "Stomach Content Weight Contribution"),
    (biting_pressure, "bitingPressure", f32, Float, 80.0_f32, Some(1.0_f64), Some(500.0_f64), "N/u", false, true, "Jaw Muscles Strength"),
    (throwing_force_factor, "throwingForceFactor", f32, Float, 0.25_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Throwing Force Factor"),
    (biting_throw_force_factor, "bitingThrowForceFactor", f32, Float, 0.25_f32, Some(0.0_f64), Some(1.0_f64), "%", false, true, "Biting Repulsion Force Factor"),
    (jaw_muscles_sizing_power, "jawMusclesSizingPower", f32, Float, 2.0_f32, Some(0.0_f64), Some(3.0_f64), "", false, true, "Jaw Sizing Power"),
    (bite_period_factor, "bitePeriodFactor", f32, Float, 1.0_f32, Some(0.0_f64), Some(10.0_f64), "X", false, true, "Bite Period Factor"),
    (biting_damage_factor, "bitingDamageFactor", f32, Float, 5.0_f32, Some(0.0_f64), Some(20.0_f64), "%", false, true, "Biting Damage Factor"),
    (collision_damage_constant, "collisionDamageConstant", f32, Float, 0.1_f32, Some(0.0_f64), Some(2.0_f64), "Dmg/(u/s)", false, true, "Collision Damage Constant"),
    (collision_damage_threshold, "collisionDamageThreshold", f32, Float, 50.0_f32, Some(0.0_f64), Some(200.0_f64), "u/s", false, true, "Collision Damage Threshold"),
    (base_metabolism_cost, "baseMetabolismCost", f32, Float, 0.001_f32, Some(0.0_f64), Some(0.1_f64), "E/u^2*s", false, true, "Default Metabolism Cost"),
    (energy_usage_efficiency, "energyUsageEfficiency", f32, Float, 0.8_f32, Some(0.1_f64), Some(1.0_f64), "%", true, true, "Energy Usage Efficiency"),
    (move_muscles_cost, "moveMusclesCost", f32, Float, 0.02_f32, Some(0.0_f64), Some(1.0_f64), "E/u^2*s", false, true, "Move Muscles Activation Cost"),
    (neuron_birth_cost, "neuronBirthCost", f32, Float, 0.1_f32, Some(0.0_f64), Some(1.0_f64), "E", false, true, "Neuron Birth Cost"),
    (synapse_birth_cost, "synapseBirthCost", f32, Float, 0.025_f32, Some(0.0_f64), Some(0.5_f64), "E", false, true, "Synapse Birth Cost"),
    (neuron_upkeep_cost, "neuronUpkeepCost", f32, Float, 0.001_f32, Some(0.0_f64), Some(0.05_f64), "E/s", false, true, "Neuron Upkeep Cost"),
    (synapse_upkeep_cost, "synapseUpkeepCost", f32, Float, 0.00025_f32, Some(0.0_f64), Some(0.015_f64), "E/s", false, true, "Synapse Upkeep Cost"),
    (pheromone_production_cost, "pheromoneProductionCost", f32, Float, 0.01_f32, Some(0.0_f64), Some(0.1_f64), "E/s", false, true, "Pheromone Production Cost"),
    (pheromone_production_strength, "pheromoneProductionStrength", f32, Float, 10.0_f32, Some(0.0_f64), Some(100.0_f64), "u^2", false, true, "Pheromone Production rate"),
    (ageing_threshold, "ageingThreshold", f32, Float, 1.0_f32, Some(0.0_f64), Some(24.0_f64), "h", false, true, "Ageing Threshold"),
    (age_strength_penalties, "ageStrengthPenalties", f32, Float, 0.25_f32, Some(0.0_f64), Some(1.0_f64), "%/h", false, true, "Age Strength Penalty"),
    (age_metabolism_penalties, "ageMetabolismPenalties", f32, Float, 0.5_f32, Some(0.0_f64), Some(1.0_f64), "%/h", false, true, "Age Metabolism Penalty"),
    (health_per_area, "healthPerArea", f32, Float, 1.25_f32, Some(0.05_f64), Some(5.0_f64), "HP/u^2", false, true, "Health to Area Factor"),
    (storable_energy_per_area, "storableEnergyPerArea", f32, Float, 1.5_f32, Some(0.05_f64), Some(5.0_f64), "E/u^2", false, true, "Storable Energy by Area"),
    (base_body_growth_cost, "baseBodyGrowthCost", f32, Float, 2.0_f32, Some(0.05_f64), Some(10.0_f64), "E/u^2", false, true, "Base Body Growth Cost"),
    (view_angle_added_cost, "viewAngleAddedCost", f32, Float, 0.003_f32, Some(0.0_f64), Some(0.05_f64), "E/(deg*u^2)", false, true, "View Angle Added Cost"),
    (view_radius_added_cost, "viewRadiusAddedCost", f32, Float, 0.002_f32, Some(0.0_f64), Some(0.05_f64), "E/u^2", false, true, "View Radius Added Cost"),
    (health_energy_factor, "healthEnergyFactor", f32, Float, 0.5_f32, Some(0.01_f64), Some(1.0_f64), "%", false, true, "Blood Energy Density Factor"),
    (global_zone_speed, "globalZoneSpeed", f32, Float, 1.0_f32, Some(0.0_f64), Some(100.0_f64), "x", false, true, "Global Zone Speed"),
    (heal_rate, "healRate", f32, Float, 0.001_f32, Some(0.0001_f64), Some(0.02_f64), "% HP/s", false, true, "Heal Rate"),
    (heal_power_factor, "healPowerFactor", f32, Float, 1.0_f32, Some(0.0_f64), Some(2.0_f64), "", false, true, "Heal Power Factor"),
    (heal_efficiency, "healEfficiency", f32, Float, 1.0_f32, Some(0.01_f64), Some(2.0_f64), "%", false, true, "Heal Efficiency"),
    (corpse_decay_factor, "corpseDecayFactor", f32, Float, 1.0_f32, Some(0.0_f64), Some(100.0_f64), "X", false, true, "Corpse Decay Rate"),
    (plant_affinity_power_factor, "plantAffinityPowerFactor", f32, Float, 1.0_f32, Some(0.1_f64), Some(3.0_f64), "", false, true, "Plant Affinity Power Factor"),
    (meat_affinity_power_factor, "meatAffinityPowerFactor", f32, Float, 1.0_f32, Some(0.1_f64), Some(3.0_f64), "", false, true, "Meat Affinity Power Factor"),
    }
}

setting_scope! {
    /// The 14 scenario-independent settings.
    IndependentSettings {
    (simulation_size, "SimulationSize", f32, Float, 2000.0_f32, Some(100.0_f64), Some(20000.0_f64), "u", false, false, "Size of the Simulation Area"),
    (biomass_density, "biomassDensity", f32, Float, 0.0075_f32, Some(0.0_f64), Some(1.0_f64), "E/u^2", false, true, "Global Biomass Density"),
    (pellet_growth, "pelletGrowth", f32, Float, 1e-05_f32, Some(0.0_f64), Some(0.0005_f64), "E/u^2*s", false, true, "Global Fertility"),
    (limit_bibite_birth, "limitBibiteBirth", bool, Bool, false, None, None, "", false, true, "Limit Bibite Birth"),
    (bibite_limit, "bibiteLimit", i32, Int, 100_i32, Some(0.0_f64), Some(500.0_f64), "", false, true, "Bibite Limit"),
    (virgin_spawn_rate, "virginSpawnRate", f32, Float, 1.0_f32, Some(0.0_f64), Some(5.0_f64), "Hz", false, true, "Bibite Spawning Rate"),
    (species_genetic_span, "speciesGeneticSpan", f32, Float, 1.5_f32, Some(0.25_f64), Some(5.0_f64), "", false, false, "Species Genetic Span"),
    (species_span_scaling_factor, "speciesSpanScalingFactor", f32, Float, 0.05_f32, Some(0.0_f64), Some(0.5_f64), "%/species", false, false, "Species Span Scaling Factor"),
    (species_prune_point_limit, "speciesPrunePointLimit", i32, Int, 2_i32, Some(1.0_f64), Some(20.0_f64), "", false, false, "Species Pruning Point Limit"),
    (sim_tps, "simTPS", i32, Int, 30_i32, Some(5.0_f64), Some(100.0_f64), "tps", false, false, "Ticks Per Second"),
    (brain_tps, "brainTPS", i32, Int, 2_i32, Some(1.0_f64), Some(100.0_f64), "X", false, false, "Brain Update Factor"),
    (decay_tps, "decayTPS", i32, Int, 2_i32, Some(1.0_f64), Some(100.0_f64), "X", false, false, "Decay Update Factor"),
    (vision_lookup_update_factor, "visionLookupUpdateFactor", i32, Int, 10_i32, Some(1.0_f64), Some(100.0_f64), "X", false, false, "Vision Lookup Factor"),
    (vision_sense_update_factor, "visionSenseUpdateFactor", i32, Int, 5_i32, Some(1.0_f64), Some(50.0_f64), "X", false, false, "Vision Sensing Factor"),
    }
}

/// The four physical materials of the game (`MatterMaterialManager`):
/// Plant, Meat, Fat and Armor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialKind {
    /// Plant matter, eaten by herbivores; the default zone spawn material.
    Plant,
    /// Meat matter, dropped by corpses and kills.
    Meat,
    /// Fat stored inside bibites (`MatterMaterialManager.Fat`).
    Fat,
    /// Armor material used by the armour organ.
    Armor,
}

impl MaterialKind {
    /// All materials, in the game's `MaterialSettings` order
    /// (`Plant, Meat, Fat, Armor`).
    pub const ALL: [Self; 4] = [Self::Plant, Self::Meat, Self::Fat, Self::Armor];

    /// The game's material name (`"Plant"`, `"Meat"`, `"Fat"`, `"Armor"`),
    /// used by save files (`pellet.material`, `materials.<Name>Settings`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Plant => "Plant",
            Self::Meat => "Meat",
            Self::Fat => "Fat",
            Self::Armor => "Armor",
        }
    }

    /// Parses a material name from a file (`"Plant"` etc.), case-sensitive.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Plant" => Some(Self::Plant),
            "Meat" => Some(Self::Meat),
            "Fat" => Some(Self::Fat),
            "Armor" => Some(Self::Armor),
            _ => None,
        }
    }

    /// The key of this material's object inside `settings.bb8settings`
    /// (`"PlantSettings"`, ...), spec 3.4.
    #[must_use]
    pub fn settings_key(self) -> &'static str {
        match self {
            Self::Plant => "PlantSettings",
            Self::Meat => "MeatSettings",
            Self::Fat => "FatSettings",
            Self::Armor => "ArmorSettings",
        }
    }
}

/// Defines the nine physical fields shared by every material (`MatterMaterial`
/// / `MatterMaterialSettings`, spec 3.4). Entries are
/// `(field, json_key, type, SettingValue variant, metadata default, min, max,
/// units, label)`; the field order is the game's writer order and the real
/// default value lives per-material in
/// [`MaterialSetting::PLANT`] etc.
macro_rules! material_fields {
    (
        $( ($field:ident, $json:literal, $ty:ty, $variant:ident, $default:expr, $min:expr,
            $max:expr, $units:literal, $label:literal) ),* $(,)?
    ) => {
        /// Physical parameters of one matter material.
        ///
        /// Field names and semantics match `pellet::MaterialProperties`
        /// (owned by world-dev); `bibites-io` serializes this type and the world
        /// converts it into runtime material properties at construction.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct MaterialSetting {
            $(
                #[doc = $label]
                pub $field: $ty,
            )*
        }

        impl MaterialSetting {
            /// Number of physical parameters per material.
            pub const COUNT: usize = Self::FIELDS.len();

            /// Metadata for every material parameter, in the game's writer
            /// order. `default` is `NaN` because defaults differ per material;
            /// see [`MaterialSettings::game_defaults`].
            pub const FIELDS: &[SettingMeta] = &[
                $(
                    SettingMeta {
                        key: $json,
                        kind: SettingKind::$variant,
                        default: SettingValue::$variant($default),
                        min: $min,
                        max: $max,
                        units: $units,
                        clamped: false,
                        animatable: false,
                        label: $label,
                    },
                )*
            ];

            /// Returns the metadata for a parameter key, or `None`.
            #[must_use]
            pub fn meta(key: &str) -> Option<&'static SettingMeta> {
                Self::FIELDS.iter().find(|meta| meta.key == key)
            }

            /// Returns the current value of `key`, or `None` for unknown keys.
            #[must_use]
            pub fn get_by_key(&self, key: &str) -> Option<SettingValue> {
                match key {
                    $( $json => Some(SettingValue::$variant(self.$field)), )*
                    _ => None,
                }
            }

            /// Applies `value` to `key` without range handling.
            pub fn set_raw(&mut self, key: &str, value: SettingValue) -> Option<SettingValue> {
                match (key, value) {
                    $(
                        ($json, SettingValue::$variant(v)) => {
                            self.$field = v;
                            Some(SettingValue::$variant(v))
                        }
                    )*
                    _ => None,
                }
            }

        }

        /// The four per-material physical parameter sets (`materials` in
        /// `settings.bb8settings`, spec 3.4).
        ///
        /// The game also defines two non-physical materials that are never
        /// serialized: **Energy** (`energyDensity = 1.0`) and **Health**
        /// (`energyDensity = 0.25`). They are not part of any settings file and
        /// are hardcoded by the body/energy code (provenance: 2026-10-29 parity
        /// runtime dump; see `docs/spec/04-formats-settings.md` section 3.4).
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct MaterialSettings {
            /// Plant matter parameters.
            pub plant: MaterialSetting,
            /// Meat matter parameters.
            pub meat: MaterialSetting,
            /// Fat material parameters (organ-internal).
            pub fat: MaterialSetting,
            /// Armor material parameters.
            pub armor: MaterialSetting,
        }

        impl MaterialSettings {
            /// Returns the defaults captured from the game's shipped
            /// `MatterMaterial` assets (cross-checked against 372 real saves,
            /// spec 3.4).
            #[must_use]
            pub fn game_defaults() -> Self {
                Self {
                    plant: MaterialSetting::PLANT,
                    meat: MaterialSetting::MEAT,
                    fat: MaterialSetting::FAT,
                    armor: MaterialSetting::ARMOR,
                }
            }

            /// Returns the parameter set for `kind`.
            #[must_use]
            pub fn get(&self, kind: MaterialKind) -> &MaterialSetting {
                match kind {
                    MaterialKind::Plant => &self.plant,
                    MaterialKind::Meat => &self.meat,
                    MaterialKind::Fat => &self.fat,
                    MaterialKind::Armor => &self.armor,
                }
            }

            /// Mutably returns the parameter set for `kind`.
            pub fn get_mut(&mut self, kind: MaterialKind) -> &mut MaterialSetting {
                match kind {
                    MaterialKind::Plant => &mut self.plant,
                    MaterialKind::Meat => &mut self.meat,
                    MaterialKind::Fat => &mut self.fat,
                    MaterialKind::Armor => &mut self.armor,
                }
            }

            /// Clamps the fields the game's setters hard-clamp (none for
            /// materials, so this is currently always `0`; kept for API
            /// symmetry with the other scopes).
            pub fn clamp_all(&mut self) -> usize {
                self.plant.clamp_all()
                    + self.meat.clamp_all()
                    + self.fat.clamp_all()
                    + self.armor.clamp_all()
            }

            /// Clamps every numeric material parameter (import-hardening
            /// policy). `fresh_time`/`decay_rate` are only clamped for
            /// materials with `decay == true`, because non-decaying materials
            /// legitimately keep both at `0.0` (below the UI minimum) and the
            /// game omits them from the file (spec 3.4).
            pub fn clamp_to_ranges(&mut self) -> usize {
                self.plant.clamp_to_ranges()
                    + self.meat.clamp_to_ranges()
                    + self.fat.clamp_to_ranges()
                    + self.armor.clamp_to_ranges()
            }
        }

        impl Default for MaterialSettings {
            fn default() -> Self {
                Self::game_defaults()
            }
        }
    };
}

material_fields! {
    (cohesiveness, "cohesiveness", f32, Float, f32::NAN, Some(0.0_f64), Some(1000.0_f64), "N/u", "Cohesiveness"),
    (reactivity, "reactivity", f32, Float, f32::NAN, Some(0.0_f64), Some(10.0_f64), "u/s", "Base Reactivity"),
    (energy_density, "energyDensity", f32, Float, f32::NAN, Some(0.01_f64), Some(100.0_f64), "E/u^2", "Energy Density"),
    (mass_density, "massDensity", f32, Float, f32::NAN, Some(0.001_f64), Some(1.0_f64), "g/u^2", "Mass Density"),
    (min_efficiency, "minEfficiency", f32, Float, f32::NAN, Some(-1.0_f64), Some(1.0_f64), "%", "Min Conversion Efficiency"),
    (max_efficiency, "maxEfficiency", f32, Float, f32::NAN, Some(-1.0_f64), Some(1.0_f64), "%", "Max Conversion Efficiency"),
    (decay, "decay", bool, Bool, false, None, None, "", "Decay"),
    (fresh_time, "freshTime", f32, Float, f32::NAN, Some(0.0_f64), Some(3600.0_f64), "s", "Freshness Time"),
    (decay_rate, "decayRate", f32, Float, f32::NAN, Some(0.001_f64), Some(100.0_f64), "u^2/s", "Decay Rate"),
}

impl MaterialSetting {
    /// Clamps the fields the game's setters hard-clamp. Material fields are
    /// never hard-clamped (`canGoOutOfBounds` defaults to `true`), so this
    /// currently always returns `0`; it exists for API symmetry.
    pub fn clamp_all(&mut self) -> usize {
        0
    }

    /// Clamps every numeric parameter to its UI range, except
    /// `fresh_time`/`decay_rate` when `decay == false` (those legitimately
    /// stay `0.0` and are omitted from files; spec 3.4).
    ///
    /// Returns how many fields changed. This is the import-hardening policy
    /// (Q6/9.7).
    pub fn clamp_to_ranges(&mut self) -> usize {
        let mut clamped = 0;
        if ClampSetting::clamp_setting(&mut self.cohesiveness, Some(0.0), Some(1000.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.reactivity, Some(0.0), Some(10.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.energy_density, Some(0.01), Some(100.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.mass_density, Some(0.001), Some(1.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.min_efficiency, Some(-1.0), Some(1.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.max_efficiency, Some(-1.0), Some(1.0)) {
            clamped += 1;
        }
        if self.decay {
            if ClampSetting::clamp_setting(&mut self.fresh_time, Some(0.0), Some(3600.0)) {
                clamped += 1;
            }
            if ClampSetting::clamp_setting(&mut self.decay_rate, Some(0.001), Some(100.0)) {
                clamped += 1;
            }
        }
        clamped
    }
}

impl MaterialSetting {
    /// Plant matter defaults: no decay. `fresh_time`/`decay_rate` are the
    /// prefab's runtime values (confirmed by the 2026-10-29 parity runtime
    /// dump); they are not serialized while `decay == false` (spec 3.4).
    pub const PLANT: Self = Self {
        cohesiveness: 10.0,
        reactivity: 0.03,
        energy_density: 5.0,
        mass_density: 0.015,
        min_efficiency: -0.25,
        max_efficiency: 0.5,
        decay: false,
        fresh_time: 100.0,
        decay_rate: 0.005,
    };

    /// Meat matter defaults: fresh for 2000 s, then 0.02 u^2/s rot (spec 3.4).
    pub const MEAT: Self = Self {
        cohesiveness: 35.0,
        reactivity: 0.075,
        energy_density: 10.0,
        mass_density: 0.025,
        min_efficiency: 0.15,
        max_efficiency: 0.85,
        decay: true,
        fresh_time: 2000.0,
        decay_rate: 0.02,
    };

    /// Fat material defaults: organ-internal, no decay. Runtime prefab values
    /// as for [`Self::PLANT`] (parity runtime dump 2026-10-29).
    pub const FAT: Self = Self {
        cohesiveness: 0.25,
        reactivity: 1.0,
        energy_density: 30.0,
        mass_density: 0.03,
        min_efficiency: 0.5,
        max_efficiency: 0.85,
        decay: false,
        fresh_time: 1000.0,
        decay_rate: 0.02,
    };

    /// Armor material defaults: organ-internal, no decay. Runtime prefab
    /// values as for [`Self::FAT`] (parity runtime dump 2026-10-29).
    pub const ARMOR: Self = Self {
        cohesiveness: 500.0,
        reactivity: 1.0,
        energy_density: 1.5,
        mass_density: 0.05,
        min_efficiency: 1.0,
        max_efficiency: 1.0,
        decay: false,
        fresh_time: 1000.0,
        decay_rate: 0.02,
    };
}

use crate::error::{Error, Result};

/// Zone pellet-spawn distribution (`SpawnDistribution` in the game; spec 3.5).
///
/// The integer values are the game's enum values and are what old file writers
/// may emit; current files write the name string (see [`Self::name`] and
/// [`Self::from_name`]).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpawnDistribution {
    /// Uniform over the whole disc (`Flat`).
    Flat = 0,
    /// Density highest at the centre, zero at the rim (`CentricGradual`).
    CentricGradual = 1,
    /// Density highest at the rim, zero at the centre (`ExteriorGradual`).
    ExteriorGradual = 2,
    /// Annulus with density zero at both ring edges (`Ring`).
    Ring = 3,
    /// Annulus with uniform density (`FlatRing`).
    FlatRing = 4,
    /// Rectangle (`Rect`).
    Rect = 5,
}

impl SpawnDistribution {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 6] = [
        Self::Flat,
        Self::CentricGradual,
        Self::ExteriorGradual,
        Self::Ring,
        Self::FlatRing,
        Self::Rect,
    ];

    /// The game's JSON name (e.g. `"CentricGradual"`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::CentricGradual => "CentricGradual",
            Self::ExteriorGradual => "ExteriorGradual",
            Self::Ring => "Ring",
            Self::FlatRing => "FlatRing",
            Self::Rect => "Rect",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Flat" => Some(Self::Flat),
            "CentricGradual" => Some(Self::CentricGradual),
            "ExteriorGradual" => Some(Self::ExteriorGradual),
            "Ring" => Some(Self::Ring),
            "FlatRing" => Some(Self::FlatRing),
            "Rect" => Some(Self::Rect),
            _ => None,
        }
    }

    /// Whether this distribution is an annulus (`Ring` or `FlatRing`).
    #[must_use]
    pub fn is_ring(self) -> bool {
        matches!(self, Self::Ring | Self::FlatRing)
    }

    /// Whether this distribution is rectangular.
    #[must_use]
    pub fn is_rect(self) -> bool {
        self == Self::Rect
    }

    /// Whether this distribution is a disc (not a ring, not a rectangle).
    #[must_use]
    pub fn is_circle(self) -> bool {
        !self.is_ring() && !self.is_rect()
    }
}

/// Zone movement mode (`MovementType` in the game; spec 3.5).
///
/// The file enum has five values, but the current settings UI only offers
/// `None`, `Free` and `Attached`; `Repulsed` and `Orbit` are accepted on load
/// for fidelity (the simulation treats them as no movement in 0.6.3.1).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MovementType {
    /// Fixed zone (`None`).
    None = 0,
    /// Random-walking zone (`Free`).
    Free = 1,
    /// Legacy value, no distinct behaviour (`Repulsed`).
    Repulsed = 2,
    /// Legacy value, no distinct behaviour (`Orbit`).
    Orbit = 3,
    /// Locked to another zone (`Attached`).
    Attached = 4,
}

impl MovementType {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Free,
        Self::Repulsed,
        Self::Orbit,
        Self::Attached,
    ];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Free => "Free",
            Self::Repulsed => "Repulsed",
            Self::Orbit => "Orbit",
            Self::Attached => "Attached",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "None" => Some(Self::None),
            "Free" => Some(Self::Free),
            "Repulsed" => Some(Self::Repulsed),
            "Orbit" => Some(Self::Orbit),
            "Attached" => Some(Self::Attached),
            _ => None,
        }
    }
}

/// How spawn-table rows tag spawned bibites (`Tagging`; spec 3.7).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tagging {
    /// No tag (`NoTagging`).
    NoTagging = 0,
    /// Tag with the species name (`SpeciesTagging`).
    SpeciesTagging = 1,
    /// Random alphanumeric tag (`RandomTagging`).
    RandomTagging = 2,
    /// User-provided tag (`CustomTagging`).
    CustomTagging = 3,
}

impl Tagging {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 4] = [
        Self::NoTagging,
        Self::SpeciesTagging,
        Self::RandomTagging,
        Self::CustomTagging,
    ];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::NoTagging => "NoTagging",
            Self::SpeciesTagging => "SpeciesTagging",
            Self::RandomTagging => "RandomTagging",
            Self::CustomTagging => "CustomTagging",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "NoTagging" => Some(Self::NoTagging),
            "SpeciesTagging" => Some(Self::SpeciesTagging),
            "RandomTagging" => Some(Self::RandomTagging),
            "CustomTagging" => Some(Self::CustomTagging),
            _ => None,
        }
    }
}

/// How a spawn-table row randomises genes (`RandomizeGenes`; spec 3.7).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RandomizeGenes {
    /// Exact template copy (`No`).
    No = 0,
    /// Normal mutation pass (`NormalMutations`).
    NormalMutations = 1,
    /// Random body colour only (`Color`).
    Color = 2,
    /// Randomise every gene, keep the brain (`AllGenes`).
    AllGenes = 3,
}

impl RandomizeGenes {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 4] = [Self::No, Self::NormalMutations, Self::Color, Self::AllGenes];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::No => "No",
            Self::NormalMutations => "NormalMutations",
            Self::Color => "Color",
            Self::AllGenes => "AllGenes",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "No" => Some(Self::No),
            "NormalMutations" => Some(Self::NormalMutations),
            "Color" => Some(Self::Color),
            "AllGenes" => Some(Self::AllGenes),
            _ => None,
        }
    }
}

/// Growth stage at spawn (`GrowthAtSpawn`; spec 3.7).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GrowthAtSpawn {
    /// Spawn as an egg (`Egg`).
    Egg = 0,
    /// Spawn just after hatching (`Baby`).
    Baby = 1,
    /// Spawn as an adult (`Adult`).
    Adult = 2,
    /// Spawn at twice the mature size (`Elder`).
    Elder = 3,
}

impl GrowthAtSpawn {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 4] = [Self::Egg, Self::Baby, Self::Adult, Self::Elder];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Egg => "Egg",
            Self::Baby => "Baby",
            Self::Adult => "Adult",
            Self::Elder => "Elder",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Egg" => Some(Self::Egg),
            "Baby" => Some(Self::Baby),
            "Adult" => Some(Self::Adult),
            "Elder" => Some(Self::Elder),
            _ => None,
        }
    }
}

/// Spawn-table spawn logic (`SpawnType`; spec 3.7).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpawnType {
    /// Keep spawning while below the target count (`Continuous`).
    Continuous = 0,
    /// Spawn up to the target once (`OneTime`).
    OneTime = 1,
}

impl SpawnType {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 2] = [Self::Continuous, Self::OneTime];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Continuous => "Continuous",
            Self::OneTime => "OneTime",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Continuous" => Some(Self::Continuous),
            "OneTime" => Some(Self::OneTime),
            _ => None,
        }
    }
}

/// A metric usable by challenge conditions and scoring (`SimMetric`; spec 7.2).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimMetric {
    /// Number of living members of a species; argument = species epithet.
    SpeciesCount = 0,
    /// Total energy of a species; argument = species epithet.
    SpeciesBiomass = 1,
    /// Number of living bibites carrying a tag; argument = tag.
    TagCount = 2,
    /// Total energy of a tag; argument = tag.
    TagBiomass = 3,
    /// Number of pellets of a material; argument = material name.
    MaterialCount = 4,
    /// Total pellet energy of a material; argument = material name.
    MaterialBiomass = 5,
    /// Simulated time in seconds; argument empty.
    Time = 6,
    /// Constant value; argument is a numeric string.
    Constant = 7,
}

impl SimMetric {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 8] = [
        Self::SpeciesCount,
        Self::SpeciesBiomass,
        Self::TagCount,
        Self::TagBiomass,
        Self::MaterialCount,
        Self::MaterialBiomass,
        Self::Time,
        Self::Constant,
    ];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::SpeciesCount => "SpeciesCount",
            Self::SpeciesBiomass => "SpeciesBiomass",
            Self::TagCount => "TagCount",
            Self::TagBiomass => "TagBiomass",
            Self::MaterialCount => "MaterialCount",
            Self::MaterialBiomass => "MaterialBiomass",
            Self::Time => "Time",
            Self::Constant => "Constant",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "SpeciesCount" => Some(Self::SpeciesCount),
            "SpeciesBiomass" => Some(Self::SpeciesBiomass),
            "TagCount" => Some(Self::TagCount),
            "TagBiomass" => Some(Self::TagBiomass),
            "MaterialCount" => Some(Self::MaterialCount),
            "MaterialBiomass" => Some(Self::MaterialBiomass),
            "Time" => Some(Self::Time),
            "Constant" => Some(Self::Constant),
            _ => None,
        }
    }

    /// Whether this metric uses its argument (everything except [`Self::Time`]).
    #[must_use]
    pub fn has_argument(self) -> bool {
        self != Self::Time
    }

    /// Whether this metric's argument may be a list of targets (everything
    /// except [`Self::Time`] and [`Self::Constant`]; spec 7.2).
    #[must_use]
    pub fn has_argument_list(self) -> bool {
        !matches!(self, Self::Time | Self::Constant)
    }
}

/// Comparison operator for challenge conditions (`Comparator`; spec 7.2).
///
/// Note the game's enum order: `LessThan`, `LessThanOrEqual`, `GreaterThan`,
/// `GreaterThanOrEqual`, `Equal`, `NotEqual`.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Comparator {
    /// `LessThan`.
    LessThan = 0,
    /// `LessThanOrEqual`.
    LessThanOrEqual = 1,
    /// `GreaterThan`.
    GreaterThan = 2,
    /// `GreaterThanOrEqual`.
    GreaterThanOrEqual = 3,
    /// `Equal`.
    Equal = 4,
    /// `NotEqual`.
    NotEqual = 5,
}

impl Comparator {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 6] = [
        Self::LessThan,
        Self::LessThanOrEqual,
        Self::GreaterThan,
        Self::GreaterThanOrEqual,
        Self::Equal,
        Self::NotEqual,
    ];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::LessThan => "LessThan",
            Self::LessThanOrEqual => "LessThanOrEqual",
            Self::GreaterThan => "GreaterThan",
            Self::GreaterThanOrEqual => "GreaterThanOrEqual",
            Self::Equal => "Equal",
            Self::NotEqual => "NotEqual",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "LessThan" => Some(Self::LessThan),
            "LessThanOrEqual" => Some(Self::LessThanOrEqual),
            "GreaterThan" => Some(Self::GreaterThan),
            "GreaterThanOrEqual" => Some(Self::GreaterThanOrEqual),
            "Equal" => Some(Self::Equal),
            "NotEqual" => Some(Self::NotEqual),
            _ => None,
        }
    }

    /// The UI symbol for this comparator (`"<"`, `">"`, `"<="`, ...).
    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            Self::LessThan => "<",
            Self::LessThanOrEqual => "<=",
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
            Self::Equal => "==",
            Self::NotEqual => "!=",
        }
    }
}

/// Boolean folding mode of a condition group (`ConditionGroupLogic`; spec 7.2).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConditionGroupLogic {
    /// All sub-conditions must hold (`And`).
    And = 0,
    /// Any sub-condition must hold (`Or`).
    Or = 1,
}

impl ConditionGroupLogic {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 2] = [Self::And, Self::Or];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::And => "And",
            Self::Or => "Or",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "And" => Some(Self::And),
            "Or" => Some(Self::Or),
            _ => None,
        }
    }
}

/// Spline segment interpolation type of an animator point
/// (`InterpolationType`; spec 7.1).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InterpolationType {
    /// Straight line (`Linear`).
    Linear = 0,
    /// Slow arrival (`EaseIn`).
    EaseIn = 1,
    /// Slow departure (`EaseOut`).
    EaseOut = 2,
    /// Slow both ways (`EaseInOut`).
    EaseInOut = 3,
}

impl InterpolationType {
    /// All variants in the game's enum order.
    pub const ALL: [Self; 4] = [Self::Linear, Self::EaseIn, Self::EaseOut, Self::EaseInOut];

    /// The game's JSON name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::EaseIn => "EaseIn",
            Self::EaseOut => "EaseOut",
            Self::EaseInOut => "EaseInOut",
        }
    }

    /// Parses a JSON name, case-sensitive; `None` for unknown names.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "Linear" => Some(Self::Linear),
            "EaseIn" => Some(Self::EaseIn),
            "EaseOut" => Some(Self::EaseOut),
            "EaseInOut" => Some(Self::EaseInOut),
            _ => None,
        }
    }
}

/// One zone of a world/scenario (`ZoneSettings`; spec 3.5).
///
/// The JSON keys are custom (see `docs/spec/04-formats-settings.md` 3.5):
/// `name`, `id`, `material`, `distribution`, `fertility`, `biomassDensity`,
/// `pelletSize`, `renewBiomass` (presence = true), `movement`, `target`,
/// `speed`, `posX`, `posY`, `radiusIsRelative`, `radius` (or `width`/`height`
/// for `Rect`), `insideRadius` (rings only).
///
/// `zone_id`/`target` are *file* zone ids, not arena handles; the game renumbers
/// ids by array position on load, so world stores must not trust them for
/// identity. All numeric values are raw (relative radii/positions are
/// fractions of the simulation size; see spec 3.5 for the derived formulas).
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneSettings {
    /// Display name; pellet groups are keyed by this on load, so keep it unique.
    pub name: String,
    /// File zone id; `None` for template zones that are not in a `zones[]` array.
    pub zone_id: Option<u32>,
    /// Spawn material; `None` means the zone does not spawn pellets.
    pub material: Option<MaterialKind>,
    /// Pellet distribution shape.
    pub distribution: SpawnDistribution,
    /// Multiplier of the global fertility.
    pub fertility: f32,
    /// Multiplier of the global biomass density.
    pub biomass_density: f32,
    /// Multiplier of the global pellet energy.
    pub pellet_size: f32,
    /// Whether the zone recycles pellets after reaching max biomass.
    pub renew_biomass: bool,
    /// Movement mode.
    pub movement: MovementType,
    /// Id of the zone this one targets/moves with (file id).
    pub target: Option<u32>,
    /// Multiplier of the global zone speed.
    pub speed: f32,
    /// X position, fraction of the simulation size (-1..=1).
    pub pos_x: f32,
    /// Y position, fraction of the simulation size (-1..=1).
    pub pos_y: f32,
    /// Whether `radius`/`width`/`height` are relative to the simulation size.
    pub radius_is_relative: bool,
    /// Radius (relative) for non-rectangular zones.
    pub radius_relative: f32,
    /// Radius (absolute, world units) for non-rectangular zones.
    pub radius_absolute: f32,
    /// Inside radius fraction of the outside radius (rings only).
    pub inside_radius: f32,
    /// Width (relative) for rectangular zones.
    pub width_relative: f32,
    /// Width (absolute, world units) for rectangular zones.
    pub width_absolute: f32,
    /// Height (relative) for rectangular zones.
    pub height_relative: f32,
    /// Height (absolute, world units) for rectangular zones.
    pub height_absolute: f32,
}

impl ZoneSettings {
    /// Returns the game's default zone (Plant material, `CentricGradual`,
    /// relative radius 0.25, all multipliers 1; spec 3.5).
    #[must_use]
    pub fn game_defaults() -> Self {
        Self {
            name: String::new(),
            zone_id: None,
            material: Some(MaterialKind::Plant),
            distribution: SpawnDistribution::CentricGradual,
            fertility: 1.0,
            biomass_density: 1.0,
            pellet_size: 1.0,
            renew_biomass: false,
            movement: MovementType::None,
            target: None,
            speed: 1.0,
            pos_x: 0.0,
            pos_y: 0.0,
            radius_is_relative: true,
            radius_relative: 0.25,
            radius_absolute: 1000.0,
            inside_radius: 0.8,
            width_relative: 0.25,
            width_absolute: 1000.0,
            height_relative: 0.5,
            height_absolute: 1000.0,
        }
    }

    /// The template used by zone groups (`ZoneSettings.DefaultTemplate`, then
    /// radius 0.05 as `ZoneGroupSettings` does; spec 3.6).
    #[must_use]
    pub fn zone_group_template() -> Self {
        let mut zone = Self::game_defaults();
        zone.fertility = 10.0;
        zone.biomass_density = 10.0;
        zone.radius_relative = 0.05;
        zone
    }

    /// `template.CopySettings(zone)`: every setting the template has goes to `zone`, except the ones a template does not have (its name,
    /// its zone id and its position: `ZoneSettings.DefaultTemplate` leaves them null), which stay what they were.
    pub fn copy_template_into(&self, zone: &mut Self) {
        zone.material = self.material;
        zone.distribution = self.distribution;
        zone.fertility = self.fertility;
        zone.biomass_density = self.biomass_density;
        zone.pellet_size = self.pellet_size;
        zone.renew_biomass = self.renew_biomass;
        zone.movement = self.movement;
        zone.target = self.target;
        zone.speed = self.speed;
        zone.radius_is_relative = self.radius_is_relative;
        zone.radius_relative = self.radius_relative;
        zone.radius_absolute = self.radius_absolute;
        zone.inside_radius = self.inside_radius;
        zone.width_relative = self.width_relative;
        zone.width_absolute = self.width_absolute;
        zone.height_relative = self.height_relative;
        zone.height_absolute = self.height_absolute;
    }

    /// Clamps the numeric fields the game's setter clamps (`canGoOutOfBounds == false`: the position, the radius, the inside radius, the width and the
    /// height) to their ranges, and returns how many fields changed. Fertility, biomass density, pellet size and the speed factor can go out of bounds in the
    /// game (`NumericSetting.canGoOutOfBounds` defaults to `true`: typing 100 into the 0.01..50 pellet size is kept, and a save holds it): they only keep
    /// to a floor at their slider's minimum here (a zero pellet size would make the zone spawn a million pellets), never to a ceiling. (They were clamped
    /// to the slider's end on load: a vanilla save with pellets of 100x came in at 50x.)
    pub fn clamp_to_ranges(&mut self) -> usize {
        let mut clamped = 0;
        for (value, floor) in [
            (&mut self.fertility, 0.001),
            (&mut self.biomass_density, 0.001),
            (&mut self.pellet_size, 0.01),
            (&mut self.speed, 0.001),
        ] {
            if ClampSetting::clamp_setting(value, Some(floor), None) {
                clamped += 1;
            }
        }
        if ClampSetting::clamp_setting(&mut self.pos_x, Some(-1.0), Some(1.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.pos_y, Some(-1.0), Some(1.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.radius_relative, Some(0.0), Some(1.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.radius_absolute, Some(0.0), Some(25_000.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.inside_radius, Some(0.0), Some(0.999)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.width_relative, Some(0.0), Some(2.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.width_absolute, Some(0.0), Some(25_000.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.height_relative, Some(0.0), Some(2.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.height_absolute, Some(0.0), Some(25_000.0)) {
            clamped += 1;
        }
        clamped
    }

    /// Checks that all numeric fields are finite and the ring parameter is in
    /// range. Returns a human-readable reason on failure.
    pub fn validate(&self) -> Result<()> {
        let finite = [
            ("fertility", self.fertility),
            ("biomassDensity", self.biomass_density),
            ("pelletSize", self.pellet_size),
            ("speed", self.speed),
            ("posX", self.pos_x),
            ("posY", self.pos_y),
            ("radiusRelative", self.radius_relative),
            ("radiusAbsolute", self.radius_absolute),
            ("insideRadius", self.inside_radius),
            ("widthRelative", self.width_relative),
            ("widthAbsolute", self.width_absolute),
            ("heightRelative", self.height_relative),
            ("heightAbsolute", self.height_absolute),
        ];
        for (key, value) in finite {
            if !value.is_finite() {
                return Err(Error::invalid_data(format!(
                    "zone '{}' field {key} is not finite",
                    self.name
                )));
            }
        }
        if self.distribution.is_ring() && !(0.0..=0.999).contains(&self.inside_radius) {
            return Err(Error::invalid_data(format!(
                "zone '{}' insideRadius {} is outside 0..=0.999",
                self.name, self.inside_radius
            )));
        }
        Ok(())
    }
}

impl Default for ZoneSettings {
    fn default() -> Self {
        Self::game_defaults()
    }
}

/// A generated group of zones (`ZoneGroupSettings`; spec 3.6).
///
/// The file stores a single `count` value whose meaning depends on
/// `scale_with_sim` (int zone count vs float density); both live here and the
/// file layer chooses which to write.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneGroupSettings {
    /// Display name; generated zones are named `<name>_<i>`.
    pub name: String,
    /// Whether the number of zones scales with the simulation area.
    pub scale_with_sim: bool,
    /// Fixed number of zones when `scale_with_sim == false`.
    pub zone_count: i32,
    /// Zones per 1,000,000 u^2 when `scale_with_sim == true`.
    pub zone_density: f32,
    /// Template copied into every generated zone.
    pub template: ZoneSettings,
    /// Generated zones; present in saves, regenerated when empty on load.
    pub zones: Vec<ZoneSettings>,
}

impl ZoneGroupSettings {
    /// Returns the game's default zone group (count 10, density 0.66,
    /// template radius 0.05; spec 3.6).
    #[must_use]
    pub fn game_defaults() -> Self {
        Self {
            name: String::new(),
            scale_with_sim: false,
            zone_count: 10,
            zone_density: 0.66,
            template: ZoneSettings::zone_group_template(),
            zones: Vec::new(),
        }
    }

    /// Number of zones to generate for a world of the given simulation size:
    /// `zone_count` when fixed, else `ceil(zone_density * sim_area / 1e6)`
    /// (`ZoneGroupSettings.count`).
    #[must_use]
    pub fn count(&self, simulation_size: f32) -> u32 {
        if self.scale_with_sim {
            let sim_area = 4.0 * simulation_size * simulation_size;
            let count = (self.zone_density * sim_area / 1_000_000.0).ceil();
            if count.is_finite() && count > 0.0 {
                count as u32
            } else {
                1
            }
        } else {
            u32::try_from(self.zone_count).unwrap_or(0)
        }
    }

    /// `ZoneGroupSettings.AlignCountTypes`: the number the group is not counted by follows the one it is (the count from the density when
    /// the count scales with the simulation, the density from the count when it does not).
    pub fn align_count_types(&mut self, simulation_size: f32) {
        let area = 4.0 * simulation_size * simulation_size;
        if self.scale_with_sim {
            self.zone_count = i32::try_from(self.count(simulation_size)).unwrap_or(i32::MAX);
        } else {
            self.zone_density = self.zone_count as f32 / (area / 1_000_000.0);
        }
    }

    /// Clamps the group's own numeric fields and the template (import policy).
    pub fn clamp_to_ranges(&mut self) -> usize {
        let mut clamped = 0;
        if ClampSetting::clamp_setting(&mut self.zone_count, Some(1.0), Some(1000.0)) {
            clamped += 1;
        }
        if ClampSetting::clamp_setting(&mut self.zone_density, Some(0.01), Some(50.0)) {
            clamped += 1;
        }
        clamped + self.template.clamp_to_ranges()
    }

    /// Validates the group and its template.
    pub fn validate(&self) -> Result<()> {
        if self.zone_count < 0 {
            return Err(Error::invalid_data(format!(
                "zone group '{}' has negative zoneCount {}",
                self.name, self.zone_count
            )));
        }
        if !self.zone_density.is_finite() || self.zone_density <= 0.0 {
            return Err(Error::invalid_data(format!(
                "zone group '{}' has invalid zoneDensity {}",
                self.name, self.zone_density
            )));
        }
        self.template.validate()?;
        for zone in &self.zones {
            zone.validate()?;
        }
        Ok(())
    }
}

impl Default for ZoneGroupSettings {
    fn default() -> Self {
        Self::game_defaults()
    }
}

/// One row of a scenario/save spawn table (`BibiteSettings`; spec 3.7).
#[derive(Clone, Debug, PartialEq)]
pub struct BibiteSpawn {
    /// Template file name as bundled in `templates/`, or an absolute path when
    /// `is_external` is set.
    pub path: String,
    /// Relative spawn priority (0.01..=100).
    pub spawn_priority: f32,
    /// Target number of bibites for this template.
    pub minimum: i32,
    /// Gene randomisation mode at spawn.
    pub randomize_genes: RandomizeGenes,
    /// Growth stage at spawn.
    pub growth_at_spawn: GrowthAtSpawn,
    /// Tagging mode.
    pub tagging: Tagging,
    /// Spawn logic (continuous vs once).
    pub spawn_type: SpawnType,
    /// Custom tag, used when `tagging == Tagging::CustomTagging`.
    pub tag: String,
    /// Number spawned so far (runtime counter; emitted only when > 0).
    pub total_spawned: i32,
    /// Whether the template lives outside the local library.
    pub is_external: bool,
    /// File id of the zone to spawn in; `None` means all zones.
    pub spawn_zone: Option<u32>,
}

impl BibiteSpawn {
    /// Returns the game's default spawn row (spec 3.7: priority 1, target 10,
    /// normal mutations, adult, no tagging, continuous).
    #[must_use]
    pub fn game_defaults() -> Self {
        Self {
            path: String::new(),
            spawn_priority: 1.0,
            minimum: 10,
            randomize_genes: RandomizeGenes::NormalMutations,
            growth_at_spawn: GrowthAtSpawn::Adult,
            tagging: Tagging::NoTagging,
            spawn_type: SpawnType::Continuous,
            tag: String::new(),
            total_spawned: 0,
            is_external: false,
            spawn_zone: None,
        }
    }

    /// The champion row defaults used by challenges (`LoadForChampion` forces
    /// `CustomTagging` and the tag `"Champion"`; spec 7.2).
    #[must_use]
    pub fn champion_defaults() -> Self {
        Self {
            tagging: Tagging::CustomTagging,
            tag: "Champion".to_string(),
            ..Self::game_defaults()
        }
    }

    /// Validates numeric fields (the file layer enforces a non-empty `path`).
    pub fn validate(&self) -> Result<()> {
        if !self.spawn_priority.is_finite() || self.spawn_priority <= 0.0 {
            return Err(Error::invalid_data(format!(
                "spawn row '{}' has invalid spawnPriority {}",
                self.path, self.spawn_priority
            )));
        }
        if self.minimum < 0 || self.total_spawned < 0 {
            return Err(Error::invalid_data(format!(
                "spawn row '{}' has negative counts",
                self.path
            )));
        }
        Ok(())
    }
}

impl Default for BibiteSpawn {
    fn default() -> Self {
        Self::game_defaults()
    }
}

/// One control point of a settings-animator spline (`SplinePoint`; spec 7.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplinePoint {
    /// Time in simulated seconds.
    pub x: f32,
    /// Multiplier applied to the animated settings at this point.
    pub y: f32,
    /// Interpolation mode of the segment that starts at this point.
    pub interpolation: InterpolationType,
    /// Easing factor (`2.0` for game-created points).
    pub ease_factor: f32,
}

impl SplinePoint {
    /// Creates a point with the game's default easing (`Linear`, factor 2).
    #[must_use]
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            interpolation: InterpolationType::Linear,
            ease_factor: 2.0,
        }
    }

    /// Creates a point with an explicit interpolation mode and easing factor.
    #[must_use]
    pub fn with_interpolation(
        x: f32,
        y: f32,
        interpolation: InterpolationType,
        ease_factor: f32,
    ) -> Self {
        Self {
            x,
            y,
            interpolation,
            ease_factor,
        }
    }
}

impl Default for SplinePoint {
    fn default() -> Self {
        Self::new(0.0, 1.0)
    }
}

/// One `<setting key> -> base value` entry of a settings animator
/// (`SettingsChanger.settingsBases`; spec 7.1).
#[derive(Clone, Debug, PartialEq)]
pub struct SettingBase {
    /// The game's setting key (`"pelletEnergy"` or `"Zone(0).fertility"`).
    pub key: String,
    /// The value the animator multiplies.
    pub base: f32,
}

/// A spline-driven settings animator (`SettingsChanger`; spec 7.1).
///
/// Data model only: runtime evaluation (factor-at-time and applying
/// `base * factor`) is implemented by the world/clock system.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsChanger {
    /// Display name.
    pub name: String,
    /// Animated settings and their base values; order is preserved.
    pub settings_bases: Vec<SettingBase>,
    /// Control points, ordered by increasing `x` (simulated seconds).
    pub points: Vec<SplinePoint>,
    /// Whether the curve repeats forever.
    pub repeat: bool,
    /// Delay before the animator starts, in simulated seconds. The game never checks the sign (`SettingsChanger.Update`: `time -= start`, nothing runs
    /// while the time is below zero), so a negative start is a phase shift: the curve is already `-start` seconds along at the first tick (the
    /// "Seasonal Desert v3" scenario starts its winter animators at -43,200).
    pub start: f32,
}

impl SettingsChanger {
    /// Returns an empty animator with the game's defaults (repeat = true,
    /// start = 0; spec 7.1).
    #[must_use]
    pub fn game_defaults() -> Self {
        Self {
            name: String::new(),
            settings_bases: Vec::new(),
            points: Vec::new(),
            repeat: true,
            start: 0.0,
        }
    }

    /// The curve's value at time `x` (`SplineHelper.FindValueAtX`, what the editor's graph draws).
    #[must_use]
    pub fn value_at(&self, x: f32) -> f32 {
        crate::world::find_value_at_x(&self.points, x, self.repeat)
    }

    /// Validates point ordering and numeric sanity.
    ///
    /// The game indexes `points[0]` unconditionally while evaluating, so at
    /// least one point is required. Equal `x` values are allowed (a jump); a
    /// decreasing `x` is not.
    pub fn validate(&self) -> Result<()> {
        if self.points.is_empty() {
            return Err(Error::invalid_data(format!(
                "settings changer '{}' has no spline points",
                self.name
            )));
        }
        if !self.start.is_finite() {
            return Err(Error::invalid_data(format!(
                "settings changer '{}' has invalid start {}",
                self.name, self.start
            )));
        }
        let mut previous = f32::NEG_INFINITY;
        for (index, point) in self.points.iter().enumerate() {
            if !point.x.is_finite() || !point.y.is_finite() || !point.ease_factor.is_finite() {
                return Err(Error::invalid_data(format!(
                    "settings changer '{}' point {index} contains a non-finite value",
                    self.name
                )));
            }
            // Two points MAY share an x: that is how a curve jumps (an end value at one y, then the next point at the same x and another y). The
            // points are an ordered list, not a function: `FactorAtTime` takes the first segment whose end is after the time, so at the shared x the
            // curve is already at the later point's value and the zero-width segment between them is never evaluated. Only a point BEFORE its
            // predecessor is refused (the game's editor never makes one).
            if point.x < previous {
                return Err(Error::invalid_data(format!(
                    "settings changer '{}' points must not go back in x (point {index})",
                    self.name
                )));
            }
            previous = point.x;
        }
        for base in &self.settings_bases {
            if base.key.is_empty() || !base.base.is_finite() {
                return Err(Error::invalid_data(format!(
                    "settings changer '{}' has an invalid settingsBases entry",
                    self.name
                )));
            }
        }
        Ok(())
    }
}

impl Default for SettingsChanger {
    fn default() -> Self {
        Self::game_defaults()
    }
}

/// A condition operand: a metric plus its argument string (spec 7.2).
#[derive(Clone, Debug, PartialEq)]
pub struct SimulationMetric {
    /// The metric to evaluate.
    pub metric: SimMetric,
    /// Argument: species epithet, tag, material name, numeric string, or empty.
    pub argument: String,
}

impl SimulationMetric {
    /// Creates a metric operand.
    #[must_use]
    pub fn new(metric: SimMetric, argument: impl Into<String>) -> Self {
        Self {
            metric,
            argument: argument.into(),
        }
    }

    /// The game's default operand: `Time` with an empty argument.
    #[must_use]
    pub fn game_defaults() -> Self {
        Self::new(SimMetric::Time, "")
    }

    /// Validates the argument (only `Constant` must parse as `f32`; other
    /// metrics tolerate unknown/empty targets and evaluate to zero).
    pub fn validate(&self) -> Result<()> {
        if self.metric == SimMetric::Constant && self.argument.trim().parse::<f32>().is_err() {
            return Err(Error::invalid_data(format!(
                "Constant metric argument {:?} is not a number",
                self.argument
            )));
        }
        Ok(())
    }
}

impl Default for SimulationMetric {
    fn default() -> Self {
        Self::game_defaults()
    }
}

/// One comparison between two metric operands (`SimCondition`; spec 7.2).
#[derive(Clone, Debug, PartialEq)]
pub struct SimCondition {
    /// Left operand.
    pub left: SimulationMetric,
    /// Comparison operator.
    pub comparator: Comparator,
    /// Right operand.
    pub right: SimulationMetric,
}

impl SimCondition {
    /// Creates a condition from the game's five JSON fields.
    #[must_use]
    pub fn new(
        left_metric: SimMetric,
        left_argument: impl Into<String>,
        comparator: Comparator,
        right_metric: SimMetric,
        right_argument: impl Into<String>,
    ) -> Self {
        Self {
            left: SimulationMetric::new(left_metric, left_argument),
            comparator,
            right: SimulationMetric::new(right_metric, right_argument),
        }
    }

    /// Validates both operands.
    pub fn validate(&self) -> Result<()> {
        self.left.validate()?;
        self.right.validate()
    }
}

/// A node of a condition tree: either a leaf comparison or a nested group
/// (the game distinguishes them by the presence of a `logic` key; spec 7.2).
#[derive(Clone, Debug, PartialEq)]
pub enum ConditionNode {
    /// A leaf comparison (`SimCondition`).
    Sim(SimCondition),
    /// A nested condition group (`ConditionGroup`).
    Group(ConditionGroup),
}

/// A boolean tree of conditions (`ConditionGroup`; spec 7.2).
///
/// The runtime semantics are: fold left with AND/OR; an empty group evaluates
/// to `false`. The game simplifies the tree on load/save by flattening child
/// groups whose logic matches the parent or that have at most one child
/// (`SimplifyGroup`); [`Self::simplify`] reproduces that structure transform for
/// round-trip comparisons.
#[derive(Clone, Debug, PartialEq)]
pub struct ConditionGroup {
    /// How sub-conditions are folded.
    pub logic: ConditionGroupLogic,
    /// Tree nodes; leaf or nested group.
    pub sub_conditions: Vec<ConditionNode>,
}

impl ConditionGroup {
    /// Creates an empty group with the given logic.
    #[must_use]
    pub fn new(logic: ConditionGroupLogic) -> Self {
        Self {
            logic,
            sub_conditions: Vec::new(),
        }
    }

    /// Creates a group with a single leaf condition, matching the game's
    /// `ConditionGroup(metric1, arg1, comparator, metric2, arg2)` helper.
    #[must_use]
    pub fn single(condition: SimCondition) -> Self {
        Self {
            logic: ConditionGroupLogic::And,
            sub_conditions: vec![ConditionNode::Sim(condition)],
        }
    }

    /// The game's default empty group (`And`).
    #[must_use]
    pub fn game_defaults() -> Self {
        Self::new(ConditionGroupLogic::And)
    }

    /// Whether the group has no sub-conditions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sub_conditions.is_empty()
    }

    /// Total number of leaf conditions in this tree.
    #[must_use]
    pub fn count_conditions(&self) -> usize {
        self.sub_conditions
            .iter()
            .map(|node| match node {
                ConditionNode::Sim(_) => 1,
                ConditionNode::Group(group) => group.count_conditions(),
            })
            .sum()
    }

    /// The game's `SimplifyGroup` pass: recursively simplify children, then
    /// flatten a child group into this one when its logic matches this group's
    /// or when it has at most one sub-condition (spec 7.2).
    pub fn simplify(&mut self) {
        for node in &mut self.sub_conditions {
            if let ConditionNode::Group(group) = node {
                group.simplify();
            }
        }
        let mut kept = Vec::with_capacity(self.sub_conditions.len());
        let mut flattened = Vec::new();
        for node in self.sub_conditions.drain(..) {
            match node {
                ConditionNode::Group(group)
                    if group.sub_conditions.len() > 1 && group.logic != self.logic =>
                {
                    kept.push(ConditionNode::Group(group));
                }
                ConditionNode::Group(group) => flattened.extend(group.sub_conditions),
                sim => kept.push(sim),
            }
        }
        kept.extend(flattened);
        self.sub_conditions = kept;
    }

    /// Validates the whole tree.
    pub fn validate(&self) -> Result<()> {
        for node in &self.sub_conditions {
            match node {
                ConditionNode::Sim(condition) => condition.validate()?,
                ConditionNode::Group(group) => group.validate()?,
            }
        }
        Ok(())
    }

    /// Evaluates the tree; an empty group is `false`. Runtime helper included
    /// here because it is a pure function of the condition data.
    ///
    /// `metric_value` resolves a metric + argument to its current value (the
    /// world supplies species/tag/pellet/time lookups). Pass a reference so
    /// recursive groups reuse the same closure type.
    #[must_use]
    pub fn evaluate<F>(&self, metric_value: &F) -> bool
    where
        F: Fn(SimMetric, &str) -> f32,
    {
        let mut result = false;
        let mut first = true;
        for node in &self.sub_conditions {
            let value = match node {
                ConditionNode::Sim(condition) => {
                    let left = metric_value(condition.left.metric, &condition.left.argument);
                    let right = metric_value(condition.right.metric, &condition.right.argument);
                    compare(condition.comparator, left, right)
                }
                ConditionNode::Group(group) => group.evaluate(metric_value),
            };
            if first {
                result = value;
                first = false;
            } else if self.logic == ConditionGroupLogic::And {
                result &= value;
            } else {
                result |= value;
            }
        }
        result
    }
}

impl Default for ConditionGroup {
    fn default() -> Self {
        Self::game_defaults()
    }
}

/// Computes `left <op> right` with the game's `Mathf.Approximately` semantics
/// for equality.
#[must_use]
pub fn compare(comparator: Comparator, left: f32, right: f32) -> bool {
    match comparator {
        Comparator::LessThan => left < right,
        Comparator::LessThanOrEqual => left <= right,
        Comparator::GreaterThan => left > right,
        Comparator::GreaterThanOrEqual => left >= right,
        // Unity `Mathf.Approximately` is an epsilon comparison; the constant is
        // `f32::EPSILON * 8` scaled by the larger magnitude (spec 7.2).
        Comparator::Equal => approximately(left, right),
        Comparator::NotEqual => !approximately(left, right),
    }
}

/// Unity's `Mathf.Approximately` for two `f32` values.
///
/// Unity uses `Abs(b-a) < Max(1e-6 * Max(Abs(a),Abs(b)), Epsilon*8)` where
/// `Mathf.Epsilon` is the smallest positive denormal (`f32::from_bits(1)`).
#[must_use]
pub fn approximately(left: f32, right: f32) -> bool {
    let diff = (left - right).abs();
    let tolerance = (1e-6 * left.abs().max(right.abs())).max(f32::from_bits(1) * 8.0);
    diff < tolerance
}

impl SimSettings {
    /// Validates that all rates are positive and every serialized numeric
    /// value is finite. This is a sanity check for file loaders; range
    /// clamping is separate (`clamp_to_ranges` on each scope).
    pub fn validate(&self) -> Result<()> {
        if self.sim_tps == 0 || self.brain_tps == 0 || self.decay_tps == 0 {
            return Err(Error::invalid_data(
                "simulation/brain/decay TPS must all be positive",
            ));
        }
        if self.vision_lookup_updates == 0 || self.vision_sense_updates == 0 {
            return Err(Error::invalid_data(
                "vision update factors must both be positive",
            ));
        }
        if !self.pellet_energy_min.is_finite() {
            return Err(Error::invalid_data("pellet_energy_min is not finite"));
        }
        for meta in ScenarioSettings::SETTINGS {
            if let Some(SettingValue::Float(value)) = self.scenario.get_by_key(meta.key)
                && !value.is_finite()
            {
                return Err(Error::invalid_data(format!(
                    "scenario setting {} is not finite",
                    meta.key
                )));
            }
        }
        for meta in IndependentSettings::SETTINGS {
            if let Some(SettingValue::Float(value)) = self.independents.get_by_key(meta.key)
                && !value.is_finite()
            {
                return Err(Error::invalid_data(format!(
                    "independent setting {} is not finite",
                    meta.key
                )));
            }
        }
        for kind in MaterialKind::ALL {
            let material = self.materials.get(kind);
            for meta in MaterialSetting::FIELDS {
                if meta.kind != SettingKind::Float {
                    continue;
                }
                if let Some(SettingValue::Float(value)) = material.get_by_key(meta.key)
                    && !value.is_finite()
                {
                    return Err(Error::invalid_data(format!(
                        "material {} field {} is not finite",
                        kind.name(),
                        meta.key
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Challenge definition (`ChallengeParameters`; spec 7.2).
#[derive(Clone, Debug, PartialEq)]
pub struct ChallengeParameters {
    /// Condition that ends the challenge.
    pub exit_condition: ConditionGroup,
    /// Metric used for the score.
    pub scoring_metric: SimulationMetric,
    /// Whether a higher score is better.
    pub high_score_is_better: bool,
    /// One-star threshold.
    pub one_star_condition: ConditionGroup,
    /// Two-star threshold.
    pub two_star_condition: ConditionGroup,
    /// Three-star threshold.
    pub three_star_condition: ConditionGroup,
    /// Spawn row for the champion.
    pub champion: BibiteSpawn,
}

impl ChallengeParameters {
    /// The game's default challenge: exit after 3600 s, score = tag count of
    /// `"Champion"` (higher is better), stars at 50/100/200 (spec 7.2).
    #[must_use]
    pub fn game_defaults() -> Self {
        Self {
            exit_condition: ConditionGroup::single(SimCondition::new(
                SimMetric::Time,
                "",
                Comparator::GreaterThanOrEqual,
                SimMetric::Constant,
                "3600",
            )),
            scoring_metric: SimulationMetric::new(SimMetric::TagCount, "Champion"),
            high_score_is_better: true,
            one_star_condition: ConditionGroup::single(SimCondition::new(
                SimMetric::TagCount,
                "Champion",
                Comparator::GreaterThanOrEqual,
                SimMetric::Constant,
                "50",
            )),
            two_star_condition: ConditionGroup::single(SimCondition::new(
                SimMetric::TagCount,
                "Champion",
                Comparator::GreaterThanOrEqual,
                SimMetric::Constant,
                "100",
            )),
            three_star_condition: ConditionGroup::single(SimCondition::new(
                SimMetric::TagCount,
                "Champion",
                Comparator::GreaterThanOrEqual,
                SimMetric::Constant,
                "200",
            )),
            champion: BibiteSpawn::champion_defaults(),
        }
    }

    /// Validates the conditions, scoring metric and champion row.
    pub fn validate(&self) -> Result<()> {
        self.exit_condition.validate()?;
        self.scoring_metric.validate()?;
        self.one_star_condition.validate()?;
        self.two_star_condition.validate()?;
        self.three_star_condition.validate()?;
        self.champion.validate()
    }
}

impl ChallengeParameters {
    /// Extracts the constant right-hand thresholds of the three star
    /// conditions when each is a single simple leaf (the default/editor shape:
    /// left metric vs `Constant`). Returns `None` for a condition that is a
    /// composite tree or compares two non-constant metrics.
    #[must_use]
    pub fn star_thresholds(&self) -> [Option<f32>; 3] {
        fn threshold(group: &ConditionGroup) -> Option<f32> {
            let [ConditionNode::Sim(condition)] = group.sub_conditions.as_slice() else {
                return None;
            };
            if condition.right.metric != SimMetric::Constant {
                return None;
            }
            condition.right.argument.trim().parse().ok()
        }
        [
            threshold(&self.one_star_condition),
            threshold(&self.two_star_condition),
            threshold(&self.three_star_condition),
        ]
    }

    /// Replaces simple star thresholds while preserving each condition's left
    /// metric and comparator (the ChallengeParametersHandle star rows). Returns
    /// an error when a star condition is not a single simple leaf; editors
    /// should then use the condition-tree API directly.
    pub fn set_star_thresholds(&mut self, thresholds: [f32; 3]) -> Result<()> {
        fn rebuild(group: &ConditionGroup, threshold: f32) -> Result<ConditionGroup> {
            let [ConditionNode::Sim(condition)] = group.sub_conditions.as_slice() else {
                return Err(Error::invalid_data(
                    "star condition is not a single simple comparison",
                ));
            };
            if !threshold.is_finite() {
                return Err(Error::invalid_data("star threshold must be finite"));
            }
            Ok(ConditionGroup::single(SimCondition {
                left: condition.left.clone(),
                comparator: condition.comparator,
                right: SimulationMetric::new(SimMetric::Constant, threshold.to_string()),
            }))
        }
        let one = rebuild(&self.one_star_condition, thresholds[0])?;
        let two = rebuild(&self.two_star_condition, thresholds[1])?;
        let three = rebuild(&self.three_star_condition, thresholds[2])?;
        self.one_star_condition = one;
        self.two_star_condition = two;
        self.three_star_condition = three;
        self.validate()
    }
}

impl Default for ChallengeParameters {
    fn default() -> Self {
        Self::game_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every scenario-setting default, in canonical order, exactly as the
    /// normative table in `docs/spec/04-formats-settings.md` section 3.3.
    const SCENARIO_DEFAULTS: &[(&str, SettingValue)] = &[
        ("backgroundMutationChance", SettingValue::Float(1.5_f32)),
        ("backgroundMutationVariance", SettingValue::Float(0.05_f32)),
        ("relativeMutationShare", SettingValue::Float(0.75_f32)),
        ("synapseMutationChance", SettingValue::Float(0.6_f32)),
        ("neuronMutationChance", SettingValue::Float(0.4_f32)),
        ("synapseChangeChance", SettingValue::Float(0.75_f32)),
        ("synapseFlipChance", SettingValue::Float(0.025_f32)),
        ("synapseToggleChance", SettingValue::Float(0.025_f32)),
        ("synapseAddChance", SettingValue::Float(0.1_f32)),
        ("synapseRemoveChance", SettingValue::Float(0.1_f32)),
        ("neuronDefaultChance", SettingValue::Float(0.7_f32)),
        ("neuronChangeChance", SettingValue::Float(0.1_f32)),
        ("neuronAddChance", SettingValue::Float(0.1_f32)),
        ("neuronRemoveChance", SettingValue::Float(0.1_f32)),
        ("pelletCollision", SettingValue::Bool(true)),
        ("pelletMerge", SettingValue::Bool(false)),
        ("pelletRotation", SettingValue::Bool(true)),
        ("eggCollision", SettingValue::Bool(true)),
        ("disableHerding", SettingValue::Bool(false)),
        ("minPelletSize", SettingValue::Float(0.1_f32)),
        ("corpsesEnabled", SettingValue::Bool(true)),
        ("preventMutations", SettingValue::Bool(false)),
        ("voidAvoidance", SettingValue::Bool(false)),
        ("voidAvoidanceDistance", SettingValue::Float(100.0_f32)),
        ("preventRedPheroProduction", SettingValue::Bool(false)),
        ("preventGreenPheroProduction", SettingValue::Bool(false)),
        ("preventBluePheroProduction", SettingValue::Bool(false)),
        ("enableRedDeath", SettingValue::Bool(false)),
        ("redDeathFill", SettingValue::Float(0.0_f32)),
        ("redDeathBaseDamages", SettingValue::Float(0.5_f32)),
        ("redDeathDamageVelocity", SettingValue::Float(0.15_f32)),
        ("shadeOutsideOfBounds", SettingValue::Bool(true)),
        ("shadeAvoidance", SettingValue::Bool(false)),
        ("worldWrapping", SettingValue::Bool(true)),
        ("initialSeeding", SettingValue::Float(0.75_f32)),
        ("pelletEnergy", SettingValue::Float(200.0_f32)),
        ("fatSustain", SettingValue::Float(0.0_f32)),
        ("dragCoefficient", SettingValue::Float(5.0_f32)),
        ("disableDragOnHeldPellet", SettingValue::Bool(false)),
        ("armMusclePressure", SettingValue::Float(50.0_f32)),
        ("forwardForceSizePower", SettingValue::Float(1.5_f32)),
        ("turnForceSizePower", SettingValue::Float(3.0_f32)),
        ("backwardFraction", SettingValue::Float(0.5_f32)),
        ("bibiteMassDensity", SettingValue::Float(0.025_f32)),
        ("stomachContentContribution", SettingValue::Float(1.0_f32)),
        ("bitingPressure", SettingValue::Float(80.0_f32)),
        ("throwingForceFactor", SettingValue::Float(0.25_f32)),
        ("bitingThrowForceFactor", SettingValue::Float(0.25_f32)),
        ("jawMusclesSizingPower", SettingValue::Float(2.0_f32)),
        ("bitePeriodFactor", SettingValue::Float(1.0_f32)),
        ("bitingDamageFactor", SettingValue::Float(5.0_f32)),
        ("collisionDamageConstant", SettingValue::Float(0.1_f32)),
        ("collisionDamageThreshold", SettingValue::Float(50.0_f32)),
        ("baseMetabolismCost", SettingValue::Float(0.001_f32)),
        ("energyUsageEfficiency", SettingValue::Float(0.8_f32)),
        ("moveMusclesCost", SettingValue::Float(0.02_f32)),
        ("neuronBirthCost", SettingValue::Float(0.1_f32)),
        ("synapseBirthCost", SettingValue::Float(0.025_f32)),
        ("neuronUpkeepCost", SettingValue::Float(0.001_f32)),
        ("synapseUpkeepCost", SettingValue::Float(0.00025_f32)),
        ("pheromoneProductionCost", SettingValue::Float(0.01_f32)),
        ("pheromoneProductionStrength", SettingValue::Float(10.0_f32)),
        ("ageingThreshold", SettingValue::Float(1.0_f32)),
        ("ageStrengthPenalties", SettingValue::Float(0.25_f32)),
        ("ageMetabolismPenalties", SettingValue::Float(0.5_f32)),
        ("healthPerArea", SettingValue::Float(1.25_f32)),
        ("storableEnergyPerArea", SettingValue::Float(1.5_f32)),
        ("baseBodyGrowthCost", SettingValue::Float(2.0_f32)),
        ("viewAngleAddedCost", SettingValue::Float(0.003_f32)),
        ("viewRadiusAddedCost", SettingValue::Float(0.002_f32)),
        ("healthEnergyFactor", SettingValue::Float(0.5_f32)),
        ("globalZoneSpeed", SettingValue::Float(1.0_f32)),
        ("healRate", SettingValue::Float(0.001_f32)),
        ("healPowerFactor", SettingValue::Float(1.0_f32)),
        ("healEfficiency", SettingValue::Float(1.0_f32)),
        ("corpseDecayFactor", SettingValue::Float(1.0_f32)),
        ("plantAffinityPowerFactor", SettingValue::Float(1.0_f32)),
        ("meatAffinityPowerFactor", SettingValue::Float(1.0_f32)),
    ];

    /// Inclusive ranges of every numeric scenario setting (spec 3.3).
    const SCENARIO_RANGES: &[(&str, Option<f64>, Option<f64>)] = &[
        ("backgroundMutationChance", Some(0.0_f64), Some(20.0_f64)),
        ("backgroundMutationVariance", Some(0.0_f64), Some(0.5_f64)),
        ("relativeMutationShare", Some(0.0_f64), Some(1.0_f64)),
        ("synapseMutationChance", Some(0.0_f64), Some(1.0_f64)),
        ("neuronMutationChance", Some(0.0_f64), Some(1.0_f64)),
        ("synapseChangeChance", Some(0.0_f64), Some(1.0_f64)),
        ("synapseFlipChance", Some(0.0_f64), Some(1.0_f64)),
        ("synapseToggleChance", Some(0.0_f64), Some(1.0_f64)),
        ("synapseAddChance", Some(0.0_f64), Some(1.0_f64)),
        ("synapseRemoveChance", Some(0.0_f64), Some(1.0_f64)),
        ("neuronDefaultChance", Some(0.0_f64), Some(1.0_f64)),
        ("neuronChangeChance", Some(0.0_f64), Some(1.0_f64)),
        ("neuronAddChance", Some(0.0_f64), Some(1.0_f64)),
        ("neuronRemoveChance", Some(0.0_f64), Some(1.0_f64)),
        ("minPelletSize", Some(0.001_f64), Some(10.0_f64)),
        ("voidAvoidanceDistance", Some(0.0001_f64), Some(1000.0_f64)),
        ("redDeathFill", Some(0.0_f64), Some(1.0_f64)),
        ("redDeathBaseDamages", Some(0.0_f64), Some(100.0_f64)),
        ("redDeathDamageVelocity", Some(0.0_f64), Some(10.0_f64)),
        ("initialSeeding", Some(0.0_f64), Some(1.0_f64)),
        ("pelletEnergy", Some(10.0_f64), Some(5000.0_f64)),
        ("fatSustain", Some(0.0_f64), Some(0.05_f64)),
        ("dragCoefficient", Some(0.0_f64), Some(25.0_f64)),
        ("armMusclePressure", Some(1.0_f64), Some(1000.0_f64)),
        ("forwardForceSizePower", Some(0.0_f64), Some(2.5_f64)),
        ("turnForceSizePower", Some(0.0_f64), Some(4.0_f64)),
        ("backwardFraction", Some(0.0_f64), Some(1.0_f64)),
        ("bibiteMassDensity", Some(0.001_f64), Some(0.25_f64)),
        ("stomachContentContribution", Some(0.0_f64), Some(2.0_f64)),
        ("bitingPressure", Some(1.0_f64), Some(500.0_f64)),
        ("throwingForceFactor", Some(0.0_f64), Some(1.0_f64)),
        ("bitingThrowForceFactor", Some(0.0_f64), Some(1.0_f64)),
        ("jawMusclesSizingPower", Some(0.0_f64), Some(3.0_f64)),
        ("bitePeriodFactor", Some(0.0_f64), Some(10.0_f64)),
        ("bitingDamageFactor", Some(0.0_f64), Some(20.0_f64)),
        ("collisionDamageConstant", Some(0.0_f64), Some(2.0_f64)),
        ("collisionDamageThreshold", Some(0.0_f64), Some(200.0_f64)),
        ("baseMetabolismCost", Some(0.0_f64), Some(0.1_f64)),
        ("energyUsageEfficiency", Some(0.1_f64), Some(1.0_f64)),
        ("moveMusclesCost", Some(0.0_f64), Some(1.0_f64)),
        ("neuronBirthCost", Some(0.0_f64), Some(1.0_f64)),
        ("synapseBirthCost", Some(0.0_f64), Some(0.5_f64)),
        ("neuronUpkeepCost", Some(0.0_f64), Some(0.05_f64)),
        ("synapseUpkeepCost", Some(0.0_f64), Some(0.015_f64)),
        ("pheromoneProductionCost", Some(0.0_f64), Some(0.1_f64)),
        (
            "pheromoneProductionStrength",
            Some(0.0_f64),
            Some(100.0_f64),
        ),
        ("ageingThreshold", Some(0.0_f64), Some(24.0_f64)),
        ("ageStrengthPenalties", Some(0.0_f64), Some(1.0_f64)),
        ("ageMetabolismPenalties", Some(0.0_f64), Some(1.0_f64)),
        ("healthPerArea", Some(0.05_f64), Some(5.0_f64)),
        ("storableEnergyPerArea", Some(0.05_f64), Some(5.0_f64)),
        ("baseBodyGrowthCost", Some(0.05_f64), Some(10.0_f64)),
        ("viewAngleAddedCost", Some(0.0_f64), Some(0.05_f64)),
        ("viewRadiusAddedCost", Some(0.0_f64), Some(0.05_f64)),
        ("healthEnergyFactor", Some(0.01_f64), Some(1.0_f64)),
        ("globalZoneSpeed", Some(0.0_f64), Some(100.0_f64)),
        ("healRate", Some(0.0001_f64), Some(0.02_f64)),
        ("healPowerFactor", Some(0.0_f64), Some(2.0_f64)),
        ("healEfficiency", Some(0.01_f64), Some(2.0_f64)),
        ("corpseDecayFactor", Some(0.0_f64), Some(100.0_f64)),
        ("plantAffinityPowerFactor", Some(0.1_f64), Some(3.0_f64)),
        ("meatAffinityPowerFactor", Some(0.1_f64), Some(3.0_f64)),
    ];

    /// Every scenario-independent default, in canonical order (spec 3.2).
    const INDEPENDENT_DEFAULTS: &[(&str, SettingValue)] = &[
        ("SimulationSize", SettingValue::Float(2000.0_f32)),
        ("biomassDensity", SettingValue::Float(0.0075_f32)),
        ("pelletGrowth", SettingValue::Float(1e-05_f32)),
        ("limitBibiteBirth", SettingValue::Bool(false)),
        ("bibiteLimit", SettingValue::Int(100_i32)),
        ("virginSpawnRate", SettingValue::Float(1.0_f32)),
        ("speciesGeneticSpan", SettingValue::Float(1.5_f32)),
        ("speciesSpanScalingFactor", SettingValue::Float(0.05_f32)),
        ("speciesPrunePointLimit", SettingValue::Int(2_i32)),
        ("simTPS", SettingValue::Int(30_i32)),
        ("brainTPS", SettingValue::Int(2_i32)),
        ("decayTPS", SettingValue::Int(2_i32)),
        ("visionLookupUpdateFactor", SettingValue::Int(10_i32)),
        ("visionSenseUpdateFactor", SettingValue::Int(5_i32)),
    ];

    /// Inclusive ranges of every numeric independent setting (spec 3.2).
    const INDEPENDENT_RANGES: &[(&str, Option<f64>, Option<f64>)] = &[
        ("SimulationSize", Some(100.0_f64), Some(20000.0_f64)),
        ("biomassDensity", Some(0.0_f64), Some(1.0_f64)),
        ("pelletGrowth", Some(0.0_f64), Some(0.0005_f64)),
        ("bibiteLimit", Some(0.0_f64), Some(500.0_f64)),
        ("virginSpawnRate", Some(0.0_f64), Some(5.0_f64)),
        ("speciesGeneticSpan", Some(0.25_f64), Some(5.0_f64)),
        ("speciesSpanScalingFactor", Some(0.0_f64), Some(0.5_f64)),
        ("speciesPrunePointLimit", Some(1.0_f64), Some(20.0_f64)),
        ("simTPS", Some(5.0_f64), Some(100.0_f64)),
        ("brainTPS", Some(1.0_f64), Some(100.0_f64)),
        ("decayTPS", Some(1.0_f64), Some(100.0_f64)),
        ("visionLookupUpdateFactor", Some(1.0_f64), Some(100.0_f64)),
        ("visionSenseUpdateFactor", Some(1.0_f64), Some(50.0_f64)),
    ];

    /// Exact material defaults (spec 3.4): (material, cohesiveness, reactivity,
    /// energy_density, mass_density, min_efficiency, max_efficiency, decay,
    /// fresh_time, decay_rate). `fresh_time`/`decay_rate` of non-decaying
    /// materials are runtime prefab values from the 2026-10-29 parity dump and
    /// are not serialized while `decay == false`.
    #[allow(clippy::type_complexity)]
    const MATERIAL_DEFAULTS: &[(&str, (f32, f32, f32, f32, f32, f32, bool, f32, f32))] = &[
        (
            "Plant",
            (10.0, 0.03, 5.0, 0.015, -0.25, 0.5, false, 100.0, 0.005),
        ),
        (
            "Meat",
            (35.0, 0.075, 10.0, 0.025, 0.15, 0.85, true, 2000.0, 0.02),
        ),
        (
            "Fat",
            (0.25, 1.0, 30.0, 0.03, 0.5, 0.85, false, 1000.0, 0.02),
        ),
        (
            "Armor",
            (500.0, 1.0, 1.5, 0.05, 1.0, 1.0, false, 1000.0, 0.02),
        ),
    ];

    #[test]
    fn scenario_table_has_78_settings() {
        assert_eq!(ScenarioSettings::COUNT, 78);
        assert_eq!(ScenarioSettings::SETTINGS.len(), 78);
        assert_eq!(SCENARIO_DEFAULTS.len(), 78);
    }

    #[test]
    fn independent_table_has_14_settings() {
        assert_eq!(IndependentSettings::COUNT, 14);
        assert_eq!(IndependentSettings::SETTINGS.len(), 14);
        assert_eq!(INDEPENDENT_DEFAULTS.len(), 14);
    }

    #[test]
    fn defaults_match_spec_tables() {
        let defaults = ScenarioSettings::game_defaults();
        for (key, expected) in SCENARIO_DEFAULTS {
            assert_eq!(
                defaults.get_by_key(key),
                Some(*expected),
                "scenario default drifted for {key}"
            );
            let meta = ScenarioSettings::meta(key).expect("missing scenario metadata");
            assert_eq!(
                meta.default, *expected,
                "scenario metadata default drifted for {key}"
            );
        }

        let defaults = IndependentSettings::game_defaults();
        for (key, expected) in INDEPENDENT_DEFAULTS {
            assert_eq!(
                defaults.get_by_key(key),
                Some(*expected),
                "independent default drifted for {key}"
            );
            let meta = IndependentSettings::meta(key).expect("missing independent metadata");
            assert_eq!(
                meta.default, *expected,
                "independent metadata default drifted for {key}"
            );
        }
    }

    #[test]
    fn metadata_order_matches_canonical_order() {
        for (meta, (key, _)) in ScenarioSettings::SETTINGS.iter().zip(SCENARIO_DEFAULTS) {
            assert_eq!(meta.key, *key, "scenario settings are out of order");
        }
        for (meta, (key, _)) in IndependentSettings::SETTINGS
            .iter()
            .zip(INDEPENDENT_DEFAULTS)
        {
            assert_eq!(meta.key, *key, "independent settings are out of order");
        }
    }

    #[test]
    fn ranges_match_spec_tables() {
        for (key, min, max) in SCENARIO_RANGES {
            let meta = ScenarioSettings::meta(key).expect("missing scenario metadata");
            assert_eq!(meta.min, *min, "min drifted for {key}");
            assert_eq!(meta.max, *max, "max drifted for {key}");
        }
        for (key, min, max) in INDEPENDENT_RANGES {
            let meta = IndependentSettings::meta(key).expect("missing independent metadata");
            assert_eq!(meta.min, *min, "min drifted for {key}");
            assert_eq!(meta.max, *max, "max drifted for {key}");
        }
    }

    #[test]
    fn kinds_match_values() {
        for (key, expected) in SCENARIO_DEFAULTS {
            let meta = ScenarioSettings::meta(key).expect("missing scenario metadata");
            assert_eq!(meta.kind, expected.kind(), "kind drifted for {key}");
        }
        for (key, expected) in INDEPENDENT_DEFAULTS {
            let meta = IndependentSettings::meta(key).expect("missing independent metadata");
            assert_eq!(meta.kind, expected.kind(), "kind drifted for {key}");
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for meta in ScenarioSettings::SETTINGS {
            assert!(seen.insert(meta.key), "duplicate scenario key {}", meta.key);
        }
        let mut seen = std::collections::HashSet::new();
        for meta in IndependentSettings::SETTINGS {
            assert!(
                seen.insert(meta.key),
                "duplicate independent key {}",
                meta.key
            );
        }
        let mut seen = std::collections::HashSet::new();
        for meta in MaterialSetting::FIELDS {
            assert!(seen.insert(meta.key), "duplicate material key {}", meta.key);
        }
    }

    #[test]
    fn material_defaults_match_spec_table() {
        let materials = MaterialSettings::game_defaults();
        for (name, expected) in MATERIAL_DEFAULTS {
            let kind = MaterialKind::from_name(name).expect("known material");
            let m = materials.get(kind);
            assert_eq!(kind.name(), *name);
            let actual = (
                m.cohesiveness,
                m.reactivity,
                m.energy_density,
                m.mass_density,
                m.min_efficiency,
                m.max_efficiency,
                m.decay,
                m.fresh_time,
                m.decay_rate,
            );
            assert_eq!(&actual, expected, "material defaults drifted for {name}");
            assert_eq!(kind.settings_key(), format!("{name}Settings"));
        }
        assert_eq!(MaterialSettings::default(), materials);
    }

    #[test]
    fn material_field_ranges_match_spec() {
        let expected = [
            ("cohesiveness", 0.0, 1000.0),
            ("reactivity", 0.0, 10.0),
            ("energyDensity", 0.01, 100.0),
            ("massDensity", 0.001, 1.0),
            ("minEfficiency", -1.0, 1.0),
            ("maxEfficiency", -1.0, 1.0),
            ("freshTime", 0.0, 3600.0),
            ("decayRate", 0.001, 100.0),
        ];
        for (key, min, max) in expected {
            let meta = MaterialSetting::meta(key).expect("missing material metadata");
            assert_eq!(meta.min, Some(min), "material min drifted for {key}");
            assert_eq!(meta.max, Some(max), "material max drifted for {key}");
        }
        let decay = MaterialSetting::meta("decay").expect("decay metadata");
        assert_eq!(decay.kind, SettingKind::Bool);
        assert_eq!(decay.min, None);
    }

    #[test]
    fn sim_settings_defaults() {
        let s = SimSettings::game_defaults();
        assert_eq!(s.sim_tps, 30);
        assert_eq!(s.brain_tps, 2);
        assert_eq!(s.decay_tps, 2);
        assert_eq!(s.vision_lookup_updates, 10);
        assert_eq!(s.vision_sense_updates, 5);
        assert_eq!(s.independents.sim_tps, 30);
        assert_eq!(s.independents.brain_tps, 2);
        assert_eq!(s.brain_period(), 2.0 / 30.0);
        assert_eq!(s.fixed_dt(), 1.0 / 30.0);
        assert_eq!(s.brain_hz(), 15.0);
        assert_eq!(s, SimSettings::default());
        assert_eq!(s.scenario, ScenarioSettings::default());
        assert_eq!(s.materials, MaterialSettings::default());
        // from_parts must synchronise the broadcast fields.
        assert_eq!(
            SimSettings::from_parts(
                ScenarioSettings::default(),
                IndependentSettings::default(),
                MaterialSettings::default()
            ),
            s
        );
    }

    #[test]
    fn rates_follow_independents() {
        let mut s = SimSettings::game_defaults();
        s.independents.sim_tps = 40;
        s.independents.brain_tps = 4;
        s.independents.decay_tps = 3;
        s.independents.vision_lookup_update_factor = 7;
        s.independents.vision_sense_update_factor = 2;
        s.sync_rates_from_independents();
        assert_eq!(s.sim_tps, 40);
        assert_eq!(s.brain_tps, 4);
        assert_eq!(s.decay_tps, 3);
        assert_eq!(s.vision_lookup_updates, 7);
        assert_eq!(s.vision_sense_updates, 2);
        // Defensive: malformed (non-positive) rates become 1, never zero.
        s.independents.sim_tps = 0;
        s.independents.brain_tps = -5;
        s.sync_rates_from_independents();
        assert_eq!(s.sim_tps, 1);
        assert_eq!(s.brain_tps, 1);
    }

    #[test]
    fn set_by_key_uses_game_clamp_rules() {
        let mut s = ScenarioSettings::game_defaults();
        // canGoOutOfBounds=false in the game -> the setter clamps.
        let applied = s
            .set_by_key("minPelletSize", SettingValue::Float(999.0))
            .expect("known key");
        assert_eq!(applied, SettingValue::Float(10.0));
        assert_eq!(s.min_pellet_size, 10.0);
        // canGoOutOfBounds=true -> the game stores the value verbatim.
        let applied = s
            .set_by_key("pelletEnergy", SettingValue::Float(99_999.0))
            .expect("known key");
        assert_eq!(applied, SettingValue::Float(99_999.0));
        assert_eq!(s.pellet_energy, 99_999.0);
        // Kind mismatch and unknown key are rejected without mutation.
        assert_eq!(s.set_by_key("pelletEnergy", SettingValue::Int(1)), None);
        assert_eq!(s.set_by_key("notAKey", SettingValue::Bool(true)), None);
        // `clamp_all` mirrors the game's setters: pelletEnergy is not
        // hard-clamped, so it stays untouched.
        assert_eq!(s.clamp_all(), 0);
        assert_eq!(s.pellet_energy, 99_999.0);
        // Import hardening clamps every numeric setting.
        assert_eq!(s.clamp_to_ranges(), 1);
        assert_eq!(s.pellet_energy, 5000.0);
    }

    #[test]
    fn set_by_key_clamp_flags() {
        let meta = ScenarioSettings::meta("minPelletSize").expect("meta");
        assert!(meta.clamped);
        assert!(meta.animatable);
        let meta = ScenarioSettings::meta("initialSeeding").expect("meta");
        assert!(!meta.animatable);
        assert!(!meta.clamped);
        let meta = IndependentSettings::meta("SimulationSize").expect("meta");
        assert!(!meta.animatable);
        assert!(!meta.clamped);
        let meta = IndependentSettings::meta("virginSpawnRate").expect("meta");
        assert!(meta.animatable);
    }

    #[test]
    fn independent_clamp_all_works() {
        let mut s = IndependentSettings::game_defaults();
        s.sim_tps = 1000;
        s.bibite_limit = -3;
        // No independent setting is hard-clamped by the game's setters.
        assert_eq!(s.clamp_all(), 0);
        // Import hardening clamps both.
        assert_eq!(s.clamp_to_ranges(), 2);
        assert_eq!(s.sim_tps, 100);
        assert_eq!(s.bibite_limit, 0);
        // In-range values are untouched.
        assert_eq!(s.clamp_to_ranges(), 0);
    }

    #[test]
    fn material_clamp_all_reports_changes() {
        let mut materials = MaterialSettings::game_defaults();
        // Defaults are already in range (including the 0.0 fresh_time /
        // decay_rate of non-decaying materials).
        assert_eq!(materials.clamp_to_ranges(), 0);
        materials.meat.decay_rate = 1_000.0;
        assert_eq!(materials.clamp_to_ranges(), 1);
        assert_eq!(materials.meat.decay_rate, 100.0);
        // Non-decaying material rows keep their 0.0 fresh/decay values.
        materials.plant.decay_rate = 0.0;
        assert_eq!(materials.plant.clamp_to_ranges(), 0);
        // No material field is game-hard-clamped.
        assert_eq!(materials.clamp_all(), 0);
    }

    #[test]
    fn values_convert() {
        assert_eq!(SettingValue::Float(1.5).kind(), SettingKind::Float);
        assert_eq!(SettingValue::Int(7).kind(), SettingKind::Int);
        assert_eq!(SettingValue::Bool(true).kind(), SettingKind::Bool);
        assert_eq!(SettingValue::Int(-2).as_f64(), -2.0);
        assert_eq!(SettingValue::Bool(false).as_f64(), 0.0);
        assert_eq!(SettingValue::Float(1.25).as_f32(), Some(1.25));
        assert_eq!(SettingValue::Float(1.25).as_i32(), None);
        assert_eq!(SettingValue::Int(3).as_bool(), None);
        let meta = ScenarioSettings::meta("backgroundMutationChance").expect("meta");
        assert_eq!(
            meta.clamp_value(SettingValue::Float(-5.0)),
            SettingValue::Float(0.0)
        );
        assert_eq!(SettingValue::Float(0.0).as_f64(), 0.0);
    }

    #[test]
    fn material_kind_round_trips() {
        for kind in MaterialKind::ALL {
            assert_eq!(MaterialKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(MaterialKind::from_name("PlantSettings"), None);
        assert_eq!(MaterialKind::from_name("plant"), None);
        let mut materials = MaterialSettings::game_defaults();
        materials.get_mut(MaterialKind::Armor).cohesiveness = 1.0;
        assert_eq!(materials.get(MaterialKind::Armor).cohesiveness, 1.0);
    }
    #[test]
    fn zone_and_spawn_enum_names_round_trip() {
        for distribution in SpawnDistribution::ALL {
            assert_eq!(
                SpawnDistribution::from_name(distribution.name()),
                Some(distribution)
            );
        }
        assert_eq!(SpawnDistribution::from_name("flat"), None);
        assert!(SpawnDistribution::Ring.is_ring());
        assert!(SpawnDistribution::FlatRing.is_ring());
        assert!(!SpawnDistribution::Flat.is_ring());
        assert!(SpawnDistribution::Rect.is_rect());
        assert!(SpawnDistribution::CentricGradual.is_circle());

        for movement in MovementType::ALL {
            assert_eq!(MovementType::from_name(movement.name()), Some(movement));
        }
        for tagging in Tagging::ALL {
            assert_eq!(Tagging::from_name(tagging.name()), Some(tagging));
        }
        for randomize in RandomizeGenes::ALL {
            assert_eq!(RandomizeGenes::from_name(randomize.name()), Some(randomize));
        }
        for growth in GrowthAtSpawn::ALL {
            assert_eq!(GrowthAtSpawn::from_name(growth.name()), Some(growth));
        }
        for spawn_type in SpawnType::ALL {
            assert_eq!(SpawnType::from_name(spawn_type.name()), Some(spawn_type));
        }
    }

    #[test]
    fn condition_and_metric_enum_names_round_trip() {
        for metric in SimMetric::ALL {
            assert_eq!(SimMetric::from_name(metric.name()), Some(metric));
        }
        assert!(!SimMetric::Time.has_argument());
        assert!(SimMetric::Constant.has_argument());
        assert!(!SimMetric::Constant.has_argument_list());
        assert!(!SimMetric::Time.has_argument_list());
        assert!(SimMetric::TagCount.has_argument_list());

        for comparator in Comparator::ALL {
            assert_eq!(Comparator::from_name(comparator.name()), Some(comparator));
            assert!(!comparator.symbol().is_empty());
        }
        for logic in ConditionGroupLogic::ALL {
            assert_eq!(ConditionGroupLogic::from_name(logic.name()), Some(logic));
        }
        for interpolation in InterpolationType::ALL {
            assert_eq!(
                InterpolationType::from_name(interpolation.name()),
                Some(interpolation)
            );
        }
    }

    #[test]
    fn zone_settings_defaults() {
        let zone = ZoneSettings::game_defaults();
        assert_eq!(zone.name, "");
        assert_eq!(zone.zone_id, None);
        assert_eq!(zone.material, Some(MaterialKind::Plant));
        assert_eq!(zone.distribution, SpawnDistribution::CentricGradual);
        assert_eq!(zone.fertility, 1.0);
        assert_eq!(zone.biomass_density, 1.0);
        assert_eq!(zone.pellet_size, 1.0);
        assert!(!zone.renew_biomass);
        assert_eq!(zone.movement, MovementType::None);
        assert_eq!(zone.target, None);
        assert_eq!(zone.speed, 1.0);
        assert_eq!(zone.pos_x, 0.0);
        assert_eq!(zone.pos_y, 0.0);
        assert!(zone.radius_is_relative);
        assert_eq!(zone.radius_relative, 0.25);
        assert_eq!(zone.radius_absolute, 1000.0);
        assert_eq!(zone.inside_radius, 0.8);
        assert_eq!(zone.width_relative, 0.25);
        assert_eq!(zone.width_absolute, 1000.0);
        assert_eq!(zone.height_relative, 0.5);
        assert_eq!(zone.height_absolute, 1000.0);
        assert_eq!(zone, ZoneSettings::default());
        assert!(zone.validate().is_ok());

        let template = ZoneSettings::zone_group_template();
        assert_eq!(template.radius_relative, 0.05);
        assert_eq!(template.fertility, 10.0);
        assert_eq!(template.biomass_density, 10.0);
    }

    #[test]
    fn zone_settings_clamp_and_validate() {
        let mut zone = ZoneSettings::game_defaults();
        zone.fertility = 1000.0;
        zone.pellet_size = 0.0;
        zone.pos_x = 2.0;
        zone.inside_radius = 2.0;
        zone.distribution = SpawnDistribution::Ring;
        // pellet_size (below its floor), pos_x and inside_radius are out of range; a fertility of 1000 is kept (`canGoOutOfBounds`).
        assert_eq!(zone.clamp_to_ranges(), 3);
        assert_eq!(zone.fertility, 1000.0);
        assert_eq!(zone.pellet_size, 0.01);
        assert_eq!(zone.pos_x, 1.0);
        assert_eq!(zone.inside_radius, 0.999);
        assert_eq!(zone.clamp_to_ranges(), 0);
        // The pellet size of a vanilla save typed past the slider's 50 is kept.
        zone.pellet_size = 100.0;
        zone.speed = 5000.0;
        assert_eq!(zone.clamp_to_ranges(), 0);
        assert_eq!((zone.pellet_size, zone.speed), (100.0, 5000.0));
        assert!(zone.validate().is_ok());

        zone.distribution = SpawnDistribution::Ring;
        zone.inside_radius = 1.5;
        assert!(zone.validate().is_err());
        zone.inside_radius = 0.5;
        zone.fertility = f32::NAN;
        assert!(zone.validate().is_err());
    }

    #[test]
    fn zone_group_defaults_and_count() {
        let group = ZoneGroupSettings::game_defaults();
        assert_eq!(group.zone_count, 10);
        assert_eq!(group.zone_density, 0.66);
        assert!(!group.scale_with_sim);
        assert!(group.zones.is_empty());
        assert_eq!(group, ZoneGroupSettings::default());
        assert_eq!(group.count(2000.0), 10);
        assert!(group.validate().is_ok());

        let mut scaled = group.clone();
        scaled.scale_with_sim = true;
        // 0.66 zones per 1e6 u^2, sim area = 4*2000*2000 = 16e6 -> ceil(10.56) = 11.
        assert_eq!(scaled.count(2000.0), 11);
        scaled.zone_count = -2;
        assert!(scaled.validate().is_err());
    }

    #[test]
    fn bibite_spawn_defaults() {
        let spawn = BibiteSpawn::game_defaults();
        assert_eq!(spawn.spawn_priority, 1.0);
        assert_eq!(spawn.minimum, 10);
        assert_eq!(spawn.randomize_genes, RandomizeGenes::NormalMutations);
        assert_eq!(spawn.growth_at_spawn, GrowthAtSpawn::Adult);
        assert_eq!(spawn.tagging, Tagging::NoTagging);
        assert_eq!(spawn.spawn_type, SpawnType::Continuous);
        assert_eq!(spawn.tag, "");
        assert_eq!(spawn.total_spawned, 0);
        assert!(!spawn.is_external);
        assert_eq!(spawn.spawn_zone, None);
        assert_eq!(spawn, BibiteSpawn::default());
        assert!(spawn.validate().is_ok());

        let champion = BibiteSpawn::champion_defaults();
        assert_eq!(champion.tagging, Tagging::CustomTagging);
        assert_eq!(champion.tag, "Champion");
    }

    #[test]
    fn spline_point_defaults() {
        let point = SplinePoint::default();
        assert_eq!(point.x, 0.0);
        assert_eq!(point.y, 1.0);
        assert_eq!(point.interpolation, InterpolationType::Linear);
        assert_eq!(point.ease_factor, 2.0);
        let point = SplinePoint::new(10.0, 0.5);
        assert_eq!(point.x, 10.0);
        assert_eq!(point.y, 0.5);
        let point = SplinePoint::with_interpolation(1.0, 2.0, InterpolationType::EaseInOut, 3.0);
        assert_eq!(point.interpolation, InterpolationType::EaseInOut);
        assert_eq!(point.ease_factor, 3.0);
    }

    #[test]
    fn settings_changer_validation() {
        let mut changer = SettingsChanger::game_defaults();
        assert!(changer.repeat);
        assert_eq!(changer.start, 0.0);
        assert_eq!(changer, SettingsChanger::default());
        // Empty points are rejected because the game indexes points[0].
        assert!(changer.validate().is_err());

        changer.points = vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(10.0, 2.0)];
        changer.settings_bases = vec![SettingBase {
            key: "pelletEnergy".to_string(),
            base: 200.0,
        }];
        assert!(changer.validate().is_ok());

        changer.points.push(SplinePoint::new(10.0, 3.0));
        assert!(changer.validate().is_ok(), "two points at one time are a jump, which the game allows");
        changer.points = vec![SplinePoint::new(0.0, 1.0), SplinePoint::new(-1.0, 2.0)];
        assert!(changer.validate().is_err(), "decreasing x must be rejected");
        changer.points = vec![SplinePoint::new(0.0, 1.0)];
        changer.start = -1.0;
        assert!(
            changer.validate().is_ok(),
            "a negative start is a phase shift in the game (a published scenario starts at -43200)"
        );
        changer.start = f32::NAN;
        assert!(changer.validate().is_err(), "a start that is not a number must be rejected");
        changer.start = 0.0;
        changer.points[0].x = f32::NAN;
        assert!(
            changer.validate().is_err(),
            "non-finite point must be rejected"
        );
    }

    #[test]
    fn simulation_metric_validation() {
        assert!(
            SimulationMetric::new(SimMetric::Constant, "3600")
                .validate()
                .is_ok()
        );
        assert!(
            SimulationMetric::new(SimMetric::Constant, "abc")
                .validate()
                .is_err()
        );
        assert!(
            SimulationMetric::new(SimMetric::Time, "")
                .validate()
                .is_ok()
        );
        // Non-constant metrics tolerate unknown/empty target strings.
        assert!(
            SimulationMetric::new(SimMetric::TagCount, "nobody")
                .validate()
                .is_ok()
        );
        assert_eq!(
            SimulationMetric::game_defaults(),
            SimulationMetric::new(SimMetric::Time, "")
        );
    }

    #[test]
    fn condition_group_simplifies_like_the_game() {
        // Two same-logic groups with multiple children are flattened.
        let mut root = ConditionGroup::new(ConditionGroupLogic::And);
        let mut child = ConditionGroup::new(ConditionGroupLogic::And);
        child
            .sub_conditions
            .push(ConditionNode::Sim(SimCondition::new(
                SimMetric::Time,
                "",
                Comparator::GreaterThanOrEqual,
                SimMetric::Constant,
                "1",
            )));
        child
            .sub_conditions
            .push(ConditionNode::Sim(SimCondition::new(
                SimMetric::Time,
                "",
                Comparator::LessThan,
                SimMetric::Constant,
                "10",
            )));
        root.sub_conditions.push(ConditionNode::Group(child));
        assert_eq!(root.count_conditions(), 2);
        root.simplify();
        assert_eq!(root.sub_conditions.len(), 2);
        assert_eq!(root.count_conditions(), 2);

        // A different-logic group with two children is preserved.
        let mut root = ConditionGroup::new(ConditionGroupLogic::And);
        let mut child = ConditionGroup::new(ConditionGroupLogic::Or);
        child
            .sub_conditions
            .push(ConditionNode::Sim(SimCondition::new(
                SimMetric::Time,
                "",
                Comparator::GreaterThan,
                SimMetric::Constant,
                "1",
            )));
        child
            .sub_conditions
            .push(ConditionNode::Sim(SimCondition::new(
                SimMetric::Time,
                "",
                Comparator::GreaterThan,
                SimMetric::Constant,
                "2",
            )));
        root.sub_conditions.push(ConditionNode::Group(child));
        root.simplify();
        assert_eq!(root.sub_conditions.len(), 1);

        // A single-child group is flattened regardless of logic.
        let mut root = ConditionGroup::new(ConditionGroupLogic::And);
        let mut child = ConditionGroup::new(ConditionGroupLogic::Or);
        child
            .sub_conditions
            .push(ConditionNode::Sim(SimCondition::new(
                SimMetric::Time,
                "",
                Comparator::GreaterThan,
                SimMetric::Constant,
                "1",
            )));
        root.sub_conditions.push(ConditionNode::Group(child));
        root.simplify();
        assert_eq!(root.sub_conditions.len(), 1);
        assert!(matches!(root.sub_conditions[0], ConditionNode::Sim(_)));
    }

    #[test]
    fn condition_group_evaluates() {
        let group = ConditionGroup::single(SimCondition::new(
            SimMetric::TagCount,
            "Champion",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "50",
        ));
        assert!(!group.is_empty());
        assert!(group.evaluate(&|metric, arg| match metric {
            SimMetric::TagCount if arg == "Champion" => 60.0,
            SimMetric::Constant => arg.parse().expect("numeric"),
            _ => 0.0,
        }));
        assert!(!group.evaluate(&|metric, arg| {
            if metric == SimMetric::Constant {
                arg.parse().expect("numeric")
            } else {
                0.0
            }
        }));
        assert!(!ConditionGroup::game_defaults().evaluate(&|_, _| 1.0));

        // OR fold.
        let mut or_group = ConditionGroup::new(ConditionGroupLogic::Or);
        or_group
            .sub_conditions
            .push(ConditionNode::Sim(SimCondition::new(
                SimMetric::Time,
                "",
                Comparator::GreaterThan,
                SimMetric::Constant,
                "100",
            )));
        or_group
            .sub_conditions
            .push(ConditionNode::Sim(SimCondition::new(
                SimMetric::Time,
                "",
                Comparator::GreaterThan,
                SimMetric::Constant,
                "10",
            )));
        assert!(or_group.evaluate(&|metric, arg| match metric {
            SimMetric::Time => 50.0,
            SimMetric::Constant => arg.parse().expect("numeric"),
            _ => 0.0,
        }));
    }

    #[test]
    fn comparator_and_approximately() {
        assert!(compare(Comparator::LessThan, 1.0, 2.0));
        assert!(compare(Comparator::LessThanOrEqual, 2.0, 2.0));
        assert!(compare(Comparator::GreaterThan, 2.0, 1.0));
        assert!(compare(Comparator::GreaterThanOrEqual, 1.0, 1.0));
        assert!(compare(Comparator::Equal, 1.0, 1.0));
        assert!(!compare(Comparator::Equal, 1.0, 1.1));
        assert!(compare(Comparator::NotEqual, 1.0, 1.1));
        assert!(approximately(1.0, 1.0 + 1e-9));
        assert!(!approximately(1.0, 1.01));
        assert!(approximately(0.0, 0.0));
    }

    #[test]
    fn challenge_defaults() {
        let challenge = ChallengeParameters::game_defaults();
        assert_eq!(challenge, ChallengeParameters::default());
        assert!(challenge.high_score_is_better);
        assert_eq!(challenge.scoring_metric.metric, SimMetric::TagCount);
        assert_eq!(challenge.scoring_metric.argument, "Champion");
        assert_eq!(challenge.exit_condition.count_conditions(), 1);
        let exit = match &challenge.exit_condition.sub_conditions[0] {
            ConditionNode::Sim(condition) => condition,
            ConditionNode::Group(_) => panic!("exit should be a leaf condition"),
        };
        assert_eq!(exit.left.metric, SimMetric::Time);
        assert_eq!(exit.comparator, Comparator::GreaterThanOrEqual);
        assert_eq!(exit.right.metric, SimMetric::Constant);
        assert_eq!(exit.right.argument, "3600");
        for (group, threshold) in [
            (&challenge.one_star_condition, "50"),
            (&challenge.two_star_condition, "100"),
            (&challenge.three_star_condition, "200"),
        ] {
            let condition = match &group.sub_conditions[0] {
                ConditionNode::Sim(condition) => condition,
                ConditionNode::Group(_) => panic!("star condition should be a leaf"),
            };
            assert_eq!(condition.left.metric, SimMetric::TagCount);
            assert_eq!(condition.left.argument, "Champion");
            assert_eq!(condition.right.argument, threshold);
        }
        assert_eq!(challenge.champion.tagging, Tagging::CustomTagging);
        assert!(challenge.validate().is_ok());
    }

    #[test]
    fn sim_settings_pellet_energy_min_and_validation() {
        let settings = SimSettings::game_defaults();
        assert_eq!(settings.pellet_energy_min, 10.0);
        assert_eq!(settings.scenario.pellet_energy, 200.0);
        assert!(settings.validate().is_ok());

        let mut invalid = settings.clone();
        invalid.scenario.pellet_energy = f32::NAN;
        assert!(invalid.validate().is_err());

        let mut invalid = settings.clone();
        invalid.sim_tps = 0;
        assert!(invalid.validate().is_err());

        let mut invalid = settings.clone();
        invalid.materials.armor.cohesiveness = f32::INFINITY;
        assert!(invalid.validate().is_err());

        // `pellet_energy_min` follows the metadata bound even after edits to the
        // scenario value (the bound itself is a code constant).
        let mut edited = settings.clone();
        edited.scenario.pellet_energy = 500.0;
        let rebuilt = SimSettings::from_parts(
            edited.scenario.clone(),
            edited.independents.clone(),
            edited.materials,
        );
        assert_eq!(rebuilt.pellet_energy_min, 10.0);
    }
}
