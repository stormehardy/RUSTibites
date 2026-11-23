//! The game's user settings shown on the Settings screens (`UserSettings.cs`).
//!
//! Labels, defaults, ranges and choice lists are copied from the decompiled
//! source; values persist to a small JSON file so they survive restarts.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// How a setting is edited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Checkbox.
    Bool,
    /// Dropdown with these labels.
    Choice(&'static [&'static str]),
    /// Integer slider `min..=max`.
    Int(i32, i32),
    /// Float slider shown as a percentage (`min..=max`).
    Percent(f32, f32),
    /// Float slider in seconds (`min..=max`, one decimal).
    Seconds(f32, f32),
    /// Plain float slider `min..=max` (one decimal, the game's `FloatUserSetting` with units).
    Float(f32, f32),
    /// Colour picker.
    Color,
}

/// A stored value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    /// Checkbox state.
    Bool(bool),
    /// Choice index.
    Choice(usize),
    /// Slider value.
    Number(f32),
    /// RGBA colour.
    Color([u8; 4]),
}

/// A change of the `Fullscreen` setting nobody has applied yet: -1 none, 0 windowed, 1 fullscreen.
static FULLSCREEN_PENDING: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);

/// The new value of the `Fullscreen` setting when it changed since the last call (the window applies it: `Screen.fullScreen = val`).
#[must_use]
pub fn take_fullscreen_change() -> Option<bool> {
    match FULLSCREEN_PENDING.swap(-1, std::sync::atomic::Ordering::Relaxed) {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

/// One setting definition.
#[derive(Clone, Copy, Debug)]
pub struct Def {
    /// Label shown in the row (`Setting.Name`).
    pub label: &'static str,
    /// `UserSettings` key (persistence).
    pub key: &'static str,
    /// Editor.
    pub kind: Kind,
    /// `DefaultValue`.
    pub default: Value,
}

const AUTOSAVE_PERIODS: &[&str] = &[
    "One Minute",
    "Five Minutes",
    "Ten Minutes",
    "Thirty Minutes",
    "One Hour",
    "Five Hours",
    "Ten Hours",
];
/// Minutes of each autosave period choice.
pub const AUTOSAVE_PERIOD_MINUTES: [u32; 7] = [1, 5, 10, 30, 60, 300, 600];

/// Every setting on the Graphics and Game pages.
pub const DEFS: &[Def] = &[
    Def {
        label: "Fullscreen",
        key: "Fullscreen",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Automatically Detect Screen Resolution",
        key: "AutomaticallyDetectScreenResolution",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Screen Resolution",
        key: "ScreenResolutionFactor",
        kind: Kind::Choice(&["720p", "768p", "900p", "1080p", "1440p", "4K"]),
        default: Value::Choice(3),
    },
    Def {
        label: "UI Scale",
        key: "UIScale",
        kind: Kind::Choice(&[
            "Smaller (0.5)",
            "Small (0.75)",
            "Normal (1.0)",
            "Large (1.15)",
            "Larger (1.25)",
        ]),
        default: Value::Choice(2),
    },
    Def {
        label: "Pheromones Strength",
        key: "PheromonesStrength",
        kind: Kind::Percent(0.0, 1.0),
        default: Value::Number(0.1),
    },
    Def {
        label: "Procedural Size",
        key: "ProceduralSize",
        kind: Kind::Choice(&["Genetic", "Maturity", "Both"]),
        default: Value::Choice(2),
    },
    Def {
        label: "Fertility Color",
        key: "FertilityColor",
        kind: Kind::Color,
        default: Value::Color([11, 128, 0, 255]),
    },
    // RUSTibites Improved: the colours the player gave the zones show (`zone_colors.rs`); a view switch of the toolbar's View panel, kept here with the others.
    Def {
        label: "Individually Coloured Zones",
        key: "ImprovedZoneColors",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Background Color",
        key: "Background",
        kind: Kind::Color,
        default: Value::Color([0, 0, 48, 255]),
    },
    Def {
        label: "Show Grid",
        key: "ShowGrid",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    // The grid's look (`BackgroundManager.UpgradeBackgroundLines` hands them to the shader).
    Def {
        label: "Fine Grid Step",
        key: "GridStep1",
        kind: Kind::Int(10, 1000),
        default: Value::Number(100.0),
    },
    Def {
        label: "Fine Grid Line Width",
        key: "GridLineWidth1",
        kind: Kind::Float(1.0, 10.0),
        default: Value::Number(1.0),
    },
    Def {
        label: "Fine Grid Color",
        key: "GridColor1",
        kind: Kind::Color,
        default: Value::Color([255, 255, 255, 64]),
    },
    Def {
        label: "Coarse Grid Step Factor",
        key: "GridStep2",
        kind: Kind::Int(10, 100),
        default: Value::Number(10.0),
    },
    Def {
        label: "Coarse Line Width Factor",
        key: "GridLineWidth2",
        kind: Kind::Int(1, 25),
        default: Value::Number(5.0),
    },
    Def {
        label: "Coarse Grid Color",
        key: "GridColor2",
        kind: Kind::Color,
        default: Value::Color([255, 255, 255, 102]),
    },
    Def {
        label: "Show Experimental Background",
        key: "ShowTexture",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Texture Color",
        key: "TextureColor",
        kind: Kind::Color,
        default: Value::Color([51, 51, 67, 255]),
    },
    Def {
        label: "Show Node Values",
        key: "ShowNodeValues",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Show Difference",
        key: "showBrainDifferences",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Easter Eggs Probability Period",
        key: "EasterEggsProbability",
        kind: Kind::Int(1, 100_000_000),
        default: Value::Number(500_000.0),
    },
    Def {
        label: "Prevent OS Sleep",
        key: "PreventSleep",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Analytics Consent",
        key: "AnalyticsConsent",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Cloud Diagnostic Consent",
        key: "CloudDiagnosticConsent",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Warn Before SaveFile Deletion",
        key: "WarnBeforeDeletingSaves",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Sort Biology Panel Energy Details",
        key: "SortBiologyPanelEnergyDetails",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Show Node Names",
        key: "ShowNodeName",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Autosave",
        key: "AutoSave",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Autosave Period",
        key: "AutoSavePeriod",
        kind: Kind::Choice(AUTOSAVE_PERIODS),
        default: Value::Choice(2),
    },
    Def {
        label: "Real-time autosave period",
        key: "AutoSaveRealTime",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Reload After AutoSaves",
        key: "ReloadAfterAutoSaves",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Pause After Load",
        key: "PauseAfterLoad",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "AutoSave Max Files",
        key: "AutoSaveFiles",
        kind: Kind::Int(1, 100),
        default: Value::Number(25.0),
    },
    Def {
        label: "Allow Easter Eggs",
        key: "AllowEasterEggs",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Minimum FPS",
        key: "MinimumFPS",
        kind: Kind::Int(0, 120),
        default: Value::Number(15.0),
    },
    Def {
        label: "Ignore FPS minimum When Unfocused",
        key: "ignoreMinOnUnfocus",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Allow Arrow Key",
        key: "AllowArrowKeyCameraControl",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Open Zone Editor On Select",
        key: "OpenZoneEditorOnSelect",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Selection Radius",
        key: "SelectionRadius",
        kind: Kind::Percent(0.0, 1.0),
        default: Value::Number(0.075),
    },
    Def {
        label: "Show Pheromones",
        key: "ShowPheromones",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Show Fertility",
        key: "ShowFertility",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Show Ruler",
        key: "ShowRuler",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Show Status Bars",
        key: "ShowStatusBars",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Show Selected Bibite Field Of View",
        key: "ShowBibiteFOW",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Show Targets Priority of Selected Bibite",
        key: "ShowBibiteTargets",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Reselect On Corpses",
        key: "ReselectOnCorpses",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Reselection Delay",
        key: "ReselectionDelay",
        kind: Kind::Seconds(0.0, 10.0),
        default: Value::Number(5.0),
    },
    // "If enabled, this will open the zone editor when a zone is selected (when pressing Z)".
    Def {
        label: "Open Zone Editor On Select",
        key: "OpenZoneEditorOnSelect",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // The Load Game panel's "Include autosaves" checkbox keeps the player's choice
    // (`LoadGamePanel` reads and writes `IncludeAutoSavesInLoadPanel`).
    Def {
        label: "Include autosaves",
        key: "IncludeAutoSavesInLoadPanel",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    // RUSTibites Improved: the jukebox (`jukebox.rs`); the Music page of the Settings screens shows these three rows.
    Def {
        label: "Play Music",
        key: "ImprovedMusicOn",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Shuffle Music",
        key: "ImprovedMusicShuffle",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Music Volume",
        key: "ImprovedMusicVolume",
        kind: Kind::Percent(0.0, 1.0),
        default: Value::Number(0.5),
    },
    // RUSTibites Improved: where new species' names come from (`name_list.rs`); the Species Names page of the Settings screens shows this row.
    Def {
        label: "Species Names",
        key: "ImprovedSpeciesNames",
        kind: Kind::Choice(&["Game's List", "My List", "Both Lists"]),
        default: Value::Choice(1),
    },
    // RUSTibites Improved: whether the gene selector towers act (`Feature::GeneSelectorTower`); on from the start because with no tower in the world nothing changes.
    Def {
        label: "Gene Selector Towers",
        key: "ImprovedGeneTowers",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // RUSTibites Improved: whether the hydrothermal vents act (`Feature::HydrothermalVent`); on from the start for the same reason.
    Def {
        label: "Hydrothermal Vents",
        key: "ImprovedVents",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // RUSTibites Improved: whether the force towers act (`Feature::ForceTower`); on from the start for the same reason.
    Def {
        label: "Force Towers",
        key: "ImprovedForce",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // RUSTibites Improved: whether the whirlpool towers act (`Feature::Whirlpool`); on from the start for the same reason.
    Def {
        label: "Whirlpool Towers",
        key: "ImprovedWhirlpool",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // RUSTibites Improved: whether the streams of the Zones tab push (`Feature::Streams`); on from the start for the same reason (with no stream in the world nothing changes).
    Def {
        label: "Streams",
        key: "ImprovedStreams",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // RUSTibites Improved: what the Species Distribution window (the Position tab) writes over each zone of its map: the zone's name, and its biomass with the pellet count.
    // Both are on from the start (the game shows them); the check boxes are on the window, not on the Settings screens.
    Def {
        label: "Species Map Zone Names",
        key: "ImprovedMapZoneNames",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Species Map Zone Biomass",
        key: "ImprovedMapZoneBiomass",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // Whether the simulation uses the stored calibration of the machine (`machine_calibration.rs`); the Calibration page of the Settings screens shows this row.
    // On from the start: with no calibration run nothing changes, and a calibration only changes how fast the simulation runs, never what it computes.
    Def {
        label: "Use Calibration",
        key: "UseCalibration",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // The Lineage panel's options (`LineagePanelSettingsManager`).
    Def {
        label: "Hide Extinct Lineages",
        key: "HideExtinctLineages",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    Def {
        label: "Show Favorited Lineages",
        key: "ShowLineagesOfFavorited",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Scale Important Species",
        key: "ScaleImportantSpecies",
        kind: Kind::Bool,
        default: Value::Bool(true),
    },
    // The Save Bibite dialog's checkboxes (also rows of the in-world settings panel).
    Def {
        label: "Format JSON",
        key: "FormatBibiteJSON",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
    Def {
        label: "Save as template",
        key: "SaveBibiteAsTemplate",
        kind: Kind::Bool,
        default: Value::Bool(false),
    },
];

/// Index of the definition with this row label.
#[must_use]
pub fn index_of(label: &str) -> Option<usize> {
    let wanted = label.trim().to_ascii_lowercase();
    DEFS.iter()
        .position(|def| def.label.to_ascii_lowercase() == wanted)
}

/// Landmark values of the float settings shown as dropdowns
/// (`SettingLandmarkValues` in `UserSettings.cs`).
fn landmarks(key: &str) -> Option<&'static [f32]> {
    match key {
        "ScreenResolutionFactor" => Some(&[0.667, 0.711, 0.833, 1.0, 1.333, 2.0]),
        "UIScale" => Some(&[0.5, 0.75, 1.0, 1.15, 1.25]),
        _ => None,
    }
}

/// Values of the installed game's own `PlayerPrefs` for every setting, so the
/// port starts with the player's real configuration (read-only).
fn game_values(prefs: &bibites_io::prefs::GamePrefs) -> BTreeMap<&'static str, Value> {
    let mut out = BTreeMap::new();
    for def in DEFS {
        let value = match def.kind {
            Kind::Bool => prefs.flag(def.key).map(Value::Bool),
            Kind::Choice(options) => {
                if let Some(points) = landmarks(def.key) {
                    prefs.float(def.key).map(|stored| {
                        let nearest = points
                            .iter()
                            .enumerate()
                            .min_by(|a, b| {
                                (a.1 - stored as f32)
                                    .abs()
                                    .total_cmp(&(b.1 - stored as f32).abs())
                            })
                            .map_or(0, |(index, _)| index);
                        Value::Choice(nearest)
                    })
                } else {
                    prefs
                        .int(def.key)
                        .map(|index| Value::Choice((index.max(0) as usize).min(options.len() - 1)))
                }
            }
            Kind::Int(..) => prefs.int(def.key).map(|value| Value::Number(value as f32)),
            Kind::Percent(..) | Kind::Seconds(..) | Kind::Float(..) => prefs
                .float(def.key)
                .map(|value| Value::Number(value as f32)),
            Kind::Color => {
                let channel = |suffix: &str| prefs.float(&format!("{}{suffix}", def.key));
                match (channel("R"), channel("G"), channel("B")) {
                    (Some(r), Some(g), Some(b)) => {
                        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                        Some(Value::Color([
                            byte(r),
                            byte(g),
                            byte(b),
                            byte(channel("A").unwrap_or(1.0)),
                        ]))
                    }
                    _ => None,
                }
            }
        };
        if let Some(value) = value {
            out.insert(def.key, value);
        }
    }
    out
}

/// Current values of the user settings.
///
/// Layers, last wins: the game's defaults, the installed game's `PlayerPrefs`
/// (read-only), then the values changed in this port (JSON file).
#[derive(Clone, Debug, Default)]
pub struct UserSettings {
    /// Values changed here (persisted).
    values: BTreeMap<&'static str, Value>,
    /// Values read from the installed game's preferences.
    game: BTreeMap<&'static str, Value>,
    /// `UserSettings.simTPSDefault` & co. stored by the installed game.
    game_tps: [Option<i32>; 5],
    /// TPS defaults saved from the New Game panel here ("Save as Default").
    tps: Option<[i32; 5]>,
    /// "Do not show again" of the action warnings changed here, by `PlayerPrefs` key.
    warnings: BTreeMap<&'static str, bool>,
    /// The installed game's own flags of the action warnings (read-only).
    game_warnings: BTreeMap<&'static str, bool>,
    path: Option<PathBuf>,
}

/// `PlayerPrefs` keys of the five TPS defaults (sim, brain, decay, vision lookup,
/// vision senses).
pub const TPS_DEFAULT_KEYS: [&str; 5] = [
    "simTPSDefault",
    "brainTPSDefault",
    "decayTPSDefault",
    "visionLookupTPSDefault",
    "visionSensesTPSDefault",
];
/// `UserSettings.*TPSDefault.DefaultValue`.
pub const TPS_BUILTIN_DEFAULTS: [i32; 5] = [40, 2, 2, 10, 5];

impl UserSettings {
    /// Loads the settings stored at `path` (defaults for missing keys).
    #[must_use]
    pub fn load(path: Option<PathBuf>) -> Self {
        let prefs = bibites_io::prefs::GamePrefs::global();
        let mut settings = Self {
            values: BTreeMap::new(),
            game: game_values(prefs),
            game_tps: TPS_DEFAULT_KEYS.map(|key| prefs.int(key).map(|value| value as i32)),
            tps: None,
            warnings: BTreeMap::new(),
            game_warnings: crate::warnings::ALL
                .iter()
                .filter_map(|warning| prefs.flag(warning.key()).map(|flag| (warning.key(), flag)))
                .collect(),
            path,
        };
        let stored: serde_json::Value = settings
            .path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(serde_json::Value::Null);
        // Format 2 keeps only the values changed here; older files stored every
        // setting (which would mask the game's own values), so they are ignored.
        let stored = match stored.get("version").and_then(serde_json::Value::as_u64) {
            Some(2) => stored
                .get("values")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            _ => serde_json::Value::Null,
        };
        if let Some(values) = stored
            .get("TpsDefaults")
            .and_then(serde_json::Value::as_array)
            && values.len() == 5
        {
            let mut parsed = TPS_BUILTIN_DEFAULTS;
            for (slot, value) in parsed.iter_mut().zip(values) {
                if let Some(number) = value.as_i64() {
                    *slot = number as i32;
                }
            }
            settings.tps = Some(parsed);
        }
        for warning in crate::warnings::ALL {
            if let Some(flag) = stored
                .get(warning.key())
                .and_then(serde_json::Value::as_bool)
            {
                settings.warnings.insert(warning.key(), flag);
            }
        }
        for def in DEFS {
            let Some(value) = stored.get(def.key) else {
                continue;
            };
            let parsed = match def.kind {
                Kind::Bool => value.as_bool().map(Value::Bool),
                Kind::Choice(options) => value
                    .as_u64()
                    .map(|index| Value::Choice((index as usize).min(options.len() - 1))),
                Kind::Int(..) | Kind::Percent(..) | Kind::Seconds(..) | Kind::Float(..) => {
                    value.as_f64().map(|v| Value::Number(v as f32))
                }
                Kind::Color => value.as_str().and_then(parse_color).map(Value::Color),
            };
            if let Some(parsed) = parsed {
                settings.values.insert(def.key, parsed);
            }
        }
        settings
    }

    /// Value of setting `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Value {
        let def = &DEFS[index];
        self.values
            .get(def.key)
            .or_else(|| self.game.get(def.key))
            .copied()
            .unwrap_or(def.default)
    }

    /// The New Game TPS panel's defaults (`UserSettings.simTPSDefault`, ...): saved here,
    /// else the installed game's, else the built-in 40 / 2 / 2 / 10 / 5.
    #[must_use]
    pub fn tps_defaults(&self) -> [i32; 5] {
        if let Some(saved) = self.tps {
            return saved;
        }
        let mut out = TPS_BUILTIN_DEFAULTS;
        for (slot, stored) in out.iter_mut().zip(self.game_tps) {
            if let Some(value) = stored {
                *slot = value;
            }
        }
        out
    }

    /// "Save as Default" of the TPS panel.
    pub fn set_tps_defaults(&mut self, values: [i32; 5]) {
        self.tps = Some(values);
        self.save();
    }

    /// Colour setting labelled `label` (`[r, g, b, a]`).
    #[must_use]
    pub fn color(&self, label: &str) -> [u8; 4] {
        match index_of(label).map(|index| self.get(index)) {
            Some(Value::Color(color)) => color,
            _ => [0, 0, 0, 255],
        }
    }

    /// Checkbox value of the setting labelled `label` (its default when unknown).
    #[must_use]
    pub fn flag(&self, label: &str) -> bool {
        index_of(label).is_some_and(|index| matches!(self.get(index), Value::Bool(true)))
    }

    /// Numeric value of the setting labelled `label`.
    #[must_use]
    pub fn number(&self, label: &str) -> f32 {
        match index_of(label).map(|index| self.get(index)) {
            Some(Value::Number(value)) => value,
            _ => 0.0,
        }
    }

    /// Choice index of the setting labelled `label`.
    #[must_use]
    pub fn choice(&self, label: &str) -> usize {
        match index_of(label).map(|index| self.get(index)) {
            Some(Value::Choice(index)) => index,
            _ => 0,
        }
    }

    pub fn set(&mut self, index: usize, value: Value) {
        // `UserSettingsManager.UpdateFullscreen` (`Screen.fullScreen = val`) runs when the setting changes: the window loop takes the change.
        if DEFS[index].key == "Fullscreen"
            && let Value::Bool(on) = value
        {
            FULLSCREEN_PENDING.store(i8::from(on), std::sync::atomic::Ordering::Relaxed);
        }
        self.values.insert(DEFS[index].key, value);
        self.save();
    }

    /// Flips a checkbox.
    pub fn toggle(&mut self, index: usize) {
        if let Value::Bool(current) = self.get(index) {
            self.set(index, Value::Bool(!current));
        }
    }

    /// The options of dropdown setting `index`, in the list's order.
    #[must_use]
    pub fn choice_labels(index: usize) -> Option<&'static [&'static str]> {
        match DEFS.get(index)?.kind {
            Kind::Choice(options) => Some(options),
            _ => None,
        }
    }

    /// The landmarks of a `FloatSettingDropdown` setting (Screen Resolution, UI Scale): the value of
    /// each option, then the setting's `precision` and `units`; the item tooltips show the value.
    /// The other dropdown settings are `ChoiceSettingDropdown`s without `SettingChoices`: no tooltips.
    #[must_use]
    pub fn landmark_values(index: usize) -> Option<(&'static [f32], i32, &'static str)> {
        match DEFS.get(index)?.key {
            "ScreenResolutionFactor" => Some((&[0.667, 0.711, 0.833, 1.0, 1.333, 2.0], 2, "X")),
            "UIScale" => Some((&[0.5, 0.75, 1.0, 1.15, 1.25], 0, "")),
            _ => None,
        }
    }

    /// The current option of dropdown setting `index`.
    #[must_use]
    pub fn current_choice(&self, index: usize) -> Option<usize> {
        match self.get(index) {
            Value::Choice(choice) if Self::choice_labels(index).is_some() => Some(choice),
            _ => None,
        }
    }

    /// Picks option `choice` of dropdown setting `index` (a pick in its list).
    pub fn pick_choice(&mut self, index: usize, choice: usize) {
        if let Some(options) = Self::choice_labels(index)
            && choice < options.len()
        {
            self.set(index, Value::Choice(choice));
        }
    }

    /// Restores the default of a setting.
    pub fn reset(&mut self, index: usize) {
        self.set(index, DEFS[index].default);
    }

    /// Sets a slider from a fraction `0..=1` of its track. True when the value changed (a held slider asks every frame: the settings
    /// file is written only when it did).
    pub fn set_fraction(&mut self, index: usize, fraction: f32) -> bool {
        let fraction = fraction.clamp(0.0, 1.0);
        let (value, range) = match DEFS[index].kind {
            Kind::Int(min, max) => ((min as f32 + fraction * (max - min) as f32).round(), (max - min) as f32),
            Kind::Percent(min, max) | Kind::Seconds(min, max) | Kind::Float(min, max) => (min + fraction * (max - min), max - min),
            _ => return false,
        };
        let value = Value::Number(value);
        // A press on a slider's handle works the value out again from the handle's pixel, and the float maths of that round trip is a hair off (about
        // a millionth of the range). `Slider.OnPointerDown` leaves the value alone there, so that is no change: it would pin the setting over the
        // installed game's own value and rewrite the file. A pixel of any track is a thousandth of the range or more, far above the tolerance.
        let unchanged = match (self.get(index), value) {
            (Value::Number(now), Value::Number(new)) => (now - new).abs() <= range.abs() * 1.0e-5,
            (now, new) => now == new,
        };
        if unchanged {
            return false;
        }
        self.set(index, value);
        true
    }

    /// Slider position `0..=1` of a numeric setting.
    #[must_use]
    pub fn fraction(&self, index: usize) -> f32 {
        let (min, max) = match DEFS[index].kind {
            Kind::Int(min, max) => (min as f32, max as f32),
            Kind::Percent(min, max) | Kind::Seconds(min, max) | Kind::Float(min, max) => (min, max),
            _ => return 0.0,
        };
        match self.get(index) {
            Value::Number(value) if max > min => ((value - min) / (max - min)).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// Text shown for a dropdown / slider / colour value.
    #[must_use]
    pub fn display(&self, index: usize) -> String {
        match (DEFS[index].kind, self.get(index)) {
            (Kind::Choice(options), Value::Choice(choice)) => {
                options.get(choice).copied().unwrap_or_default().to_owned()
            }
            (Kind::Int(..), Value::Number(value)) => format!("{value:.0}"),
            (Kind::Seconds(..), Value::Number(value)) => format!("{value:.1} s"),
            (Kind::Float(..), Value::Number(value)) => format!("{value:.1}"),
            (Kind::Percent(..), Value::Number(value)) => {
                let percent = value * 100.0;
                if (percent - percent.round()).abs() < 0.05 {
                    format!("{percent:.0}%")
                } else {
                    format!("{percent:.1}%")
                }
            }
            (Kind::Color, Value::Color(color)) => {
                if color[3] == 255 {
                    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
                } else {
                    format!(
                        "#{:02X}{:02X}{:02X}{:02X}",
                        color[0], color[1], color[2], color[3]
                    )
                }
            }
            _ => String::new(),
        }
    }

    /// Gives the background style the grid settings (`BackgroundManager.UpgradeBackgroundLines`):
    /// the coarse step is the fine step times its factor, the coarse width the fine width times
    /// its factor, the colours are the two grid colours. The shader's width unit is not known,
    /// so the measured defaults (1.5 and 3.0 px for the settings' 1 and 5) scale with the
    /// settings.
    pub fn apply_grid(&self, style: &mut crate::assets::BackgroundStyle) {
        let fine_step = self.number("Fine Grid Step").max(1.0);
        let fine_width = self.number("Fine Grid Line Width").max(0.1);
        let width_factor = self.number("Coarse Line Width Factor").max(1.0);
        style.fine_step = fine_step;
        style.coarse_step = fine_step * self.number("Coarse Grid Step Factor").max(1.0);
        style.fine_color = self.color("Fine Grid Color");
        style.coarse_color = self.color("Coarse Grid Color");
        // The widths are world units (the settings say "u"; the shader measures the lines in the world): the fine line is as wide as the setting
        // (1 u) and the coarse one that times its factor (5 x: the live game's line at x = 0 is 4.8 u across at three zooms).
        style.fine_width = fine_width;
        style.coarse_width = fine_width * width_factor;
    }

    /// Whether the player switched the warning off with its "do not show again" checkbox
    /// (`ActionWarning.doNotShowAgain`): changed here, else the installed game's, else shown.
    #[must_use]
    pub fn warning_off(&self, warning: crate::warnings::ActionWarning) -> bool {
        self.warnings
            .get(warning.key())
            .or_else(|| self.game_warnings.get(warning.key()))
            .copied()
            .unwrap_or(false)
    }

    /// Sets the warning's "do not show again" flag (`ActionWarning.doNotShowAgain = value`).
    pub fn set_warning_off(&mut self, warning: crate::warnings::ActionWarning, off: bool) {
        self.warnings.insert(warning.key(), off);
        self.save();
    }

    /// "Reset Warnings" (`UserSettingsManager.ResetAllWarnings`): the warnings of
    /// `ActionWarnings.allInfoShown` are shown again. The installed game's own flags stay as
    /// they are (they are read-only here), so the reset is stored as an explicit "shown".
    pub fn reset_warnings(&mut self) {
        for warning in crate::warnings::ALL_INFO_SHOWN {
            self.warnings.insert(warning.key(), false);
        }
        self.save();
    }

    /// Autosave period in minutes.
    #[must_use]
    pub fn autosave_period_minutes(&self) -> u32 {
        AUTOSAVE_PERIOD_MINUTES[self.choice("Autosave Period").min(6)]
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut map = serde_json::Map::new();
        for def in DEFS {
            // Only what was changed here; the rest follows the game's own values.
            let Some(value) = self.values.get(def.key).copied() else {
                continue;
            };
            let json = match value {
                Value::Bool(flag) => serde_json::Value::Bool(flag),
                Value::Choice(index) => serde_json::Value::from(index as u64),
                Value::Number(number) => serde_json::Value::from(f64::from(number)),
                Value::Color(color) => serde_json::Value::String(format!(
                    "#{:02X}{:02X}{:02X}{:02X}",
                    color[0], color[1], color[2], color[3]
                )),
            };
            map.insert(def.key.to_owned(), json);
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Some(tps) = self.tps {
            map.insert(
                "TpsDefaults".to_owned(),
                serde_json::Value::from(tps.to_vec()),
            );
        }
        for (key, flag) in &self.warnings {
            map.insert((*key).to_owned(), serde_json::Value::Bool(*flag));
        }
        let mut root = serde_json::Map::new();
        root.insert("version".to_owned(), serde_json::Value::from(2_u64));
        root.insert("values".to_owned(), serde_json::Value::Object(map));
        if let Ok(text) = serde_json::to_string_pretty(&serde_json::Value::Object(root)) {
            write_whole(path, &text);
        }
    }
}

/// Writes `text` to `path` without ever leaving a half-written file there: it goes to a file beside it first, which then takes the old one's place.
/// A held slider rewrites the file at every step, and a kill or a crash in the middle of an in-place write would leave it truncated, which the next
/// start reads as "no settings". Where the swap is refused (another program holds the file open) the text is written in place, as it always was.
fn write_whole(path: &std::path::Path, text: &str) {
    let mut beside = path.as_os_str().to_owned();
    beside.push(".tmp");
    let beside = PathBuf::from(beside);
    if std::fs::write(&beside, text).is_ok() && std::fs::rename(&beside, path).is_ok() {
        return;
    }
    let _ = std::fs::remove_file(&beside);
    let _ = std::fs::write(path, text);
}

fn parse_color(text: &str) -> Option<[u8; 4]> {
    let hex = text.strip_prefix('#')?;
    let byte = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    match hex.len() {
        6 => Some([byte(0..2)?, byte(2..4)?, byte(4..6)?, 255]),
        8 => Some([byte(0..2)?, byte(2..4)?, byte(4..6)?, byte(6..8)?]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) -> (UserSettings, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rustibites-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("settings.json");
        (UserSettings::load(Some(path.clone())), path)
    }

    #[test]
    fn changing_the_fullscreen_setting_asks_the_window_to_follow() {
        let index = index_of("Fullscreen").expect("Fullscreen");
        // No file; the player's own stored value is the start (the game's default is on).
        let mut settings = UserSettings::load(None);
        let start = settings.flag("Fullscreen");
        let _ = take_fullscreen_change();
        // The toggle changes it, once; the next one changes it back.
        settings.toggle(index);
        assert_eq!(take_fullscreen_change(), Some(!start));
        assert_eq!(take_fullscreen_change(), None, "taken once");
        settings.toggle(index);
        assert_eq!(take_fullscreen_change(), Some(start));
        // A reset to the default is a change too; other settings are not.
        settings.reset(index);
        assert_eq!(take_fullscreen_change(), Some(true));
        settings.toggle(index_of("Show Grid").expect("Show Grid"));
        assert_eq!(take_fullscreen_change(), None);
    }

    #[test]
    fn the_grid_rows_have_the_games_defaults_and_ranges() {
        let default = |label: &str| DEFS[index_of(label).expect(label)].default;
        assert_eq!(default("Fine Grid Step"), Value::Number(100.0));
        assert_eq!(default("Fine Grid Line Width"), Value::Number(1.0));
        assert_eq!(default("Coarse Grid Step Factor"), Value::Number(10.0));
        assert_eq!(default("Coarse Line Width Factor"), Value::Number(5.0));
        assert_eq!(
            default("Fine Grid Color"),
            Value::Color([255, 255, 255, 64])
        );
        assert_eq!(
            default("Coarse Grid Color"),
            Value::Color([255, 255, 255, 102])
        );
        let kind = |label: &str| DEFS[index_of(label).expect(label)].kind;
        assert_eq!(kind("Fine Grid Step"), Kind::Int(10, 1000));
        assert_eq!(kind("Fine Grid Line Width"), Kind::Float(1.0, 10.0));
        assert_eq!(kind("Coarse Grid Step Factor"), Kind::Int(10, 100));
        assert_eq!(kind("Coarse Line Width Factor"), Kind::Int(1, 25));
    }

    #[test]
    fn action_warnings_switch_off_per_key_and_persist() {
        use crate::warnings::ActionWarning;
        let (mut settings, path) = fresh("action-warnings");
        // Whatever the installed game says, an explicit value here wins.
        settings.set_warning_off(ActionWarning::DeleteSaveFile, false);
        settings.set_warning_off(ActionWarning::PruneSpecies, false);
        assert!(!settings.warning_off(ActionWarning::DeleteSaveFile));
        settings.set_warning_off(ActionWarning::DeleteSaveFile, true);
        assert!(settings.warning_off(ActionWarning::DeleteSaveFile));
        assert!(
            !settings.warning_off(ActionWarning::PruneSpecies),
            "another warning's flag is its own"
        );
        // The file keeps the game's key names, and a restart finds the flags again.
        let text = std::fs::read_to_string(&path).expect("settings file");
        assert!(text.contains("\"saveDeletionWarning\": true"), "{text}");
        let again = UserSettings::load(Some(path.clone()));
        assert!(again.warning_off(ActionWarning::DeleteSaveFile));
        assert!(!again.warning_off(ActionWarning::PruneSpecies));
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn reset_warnings_shows_the_info_list_again_and_leaves_the_file_warning() {
        use crate::warnings::ActionWarning;
        let (mut settings, path) = fresh("reset-warnings");
        for warning in crate::warnings::ALL {
            settings.set_warning_off(warning, true);
        }
        settings.reset_warnings();
        // `allInfoShown`: save deletion, species pruning and the scenario start.
        assert!(!settings.warning_off(ActionWarning::DeleteSaveFile));
        assert!(!settings.warning_off(ActionWarning::PruneSpecies));
        assert!(!settings.warning_off(ActionWarning::StartScenarioAfterEdit));
        // The bibite file and the workshop warnings are not in that list.
        assert!(settings.warning_off(ActionWarning::DeleteBibiteFile));
        assert!(settings.warning_off(ActionWarning::DeleteWorkshopItem));
        let again = UserSettings::load(Some(path.clone()));
        assert!(!again.warning_off(ActionWarning::DeleteSaveFile));
        assert!(again.warning_off(ActionWarning::DeleteBibiteFile));
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn a_dropdown_setting_lists_its_options_and_a_pick_persists() {
        let (mut settings, path) = fresh("dropdown-settings");
        let index = index_of("Autosave Period").expect("a dropdown setting");
        let labels = UserSettings::choice_labels(index).expect("its options");
        assert!(labels.len() >= 5, "{labels:?}");
        // Picking sets the option, an index out of range does nothing, a setting that is not a
        // dropdown has no options.
        settings.pick_choice(index, 2);
        assert_eq!(settings.current_choice(index), Some(2));
        settings.pick_choice(index, 99);
        assert_eq!(settings.current_choice(index), Some(2));
        let toggle = index_of("Show Grid").expect("a checkbox");
        assert!(UserSettings::choice_labels(toggle).is_none());
        assert!(settings.current_choice(toggle).is_none());
        // The player's pick is kept across a restart.
        let again = UserSettings::load(Some(path.clone()));
        assert_eq!(again.current_choice(index), Some(2));
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn a_press_on_a_sliders_handle_that_works_out_the_same_place_changes_nothing() {
        let (mut settings, path) = fresh("slider-handle-press");
        let width = index_of("Fine Grid Line Width").expect("width");
        assert!(settings.set_fraction(width, 0.123), "a real move is a change");
        let stored = settings.number("Fine Grid Line Width");
        // The file is gone: a call that changed nothing must not write it again.
        std::fs::remove_file(&path).expect("settings file");
        // What a press on the handle works out from the handle's pixel: the same place, a hair off either way.
        for noise in [0.0_f32, 1.0e-7, -1.0e-7, 3.0e-7, -3.0e-7] {
            let fraction = settings.fraction(width) + noise;
            assert!(!settings.set_fraction(width, fraction), "noise of {noise} is not a change");
        }
        assert_eq!(settings.number("Fine Grid Line Width"), stored, "the value is left as it was");
        assert!(!path.exists(), "and the file is not rewritten");
        // One pixel of a 300 px track is a change.
        assert!(settings.set_fraction(width, settings.fraction(width) + 1.0 / 300.0));
        assert!(path.exists());
        // An integer slider moves by whole units only.
        let step = index_of("Fine Grid Step").expect("step");
        assert!(settings.set_fraction(step, 0.5));
        let value = settings.number("Fine Grid Step");
        assert!(!settings.set_fraction(step, settings.fraction(step) + 1.0e-7));
        assert_eq!(settings.number("Fine Grid Step"), value);
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn the_file_is_written_through_one_beside_it_and_a_leftover_one_does_no_harm() {
        let (mut settings, path) = fresh("settings-atomic-write");
        let mut beside = path.as_os_str().to_owned();
        beside.push(".tmp");
        let beside = PathBuf::from(beside);
        // What a kill between the write and the swap would have left.
        std::fs::write(&beside, "{ half a file").expect("leftover");
        settings.toggle(index_of("Show Grid").expect("Show Grid"));
        let text = std::fs::read_to_string(&path).expect("settings file");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("a whole JSON file");
        assert_eq!(parsed["version"], 2);
        assert!(!beside.exists(), "the file beside it took the settings file's place");
        // A second change replaces the first whole, and a restart reads it.
        settings.toggle(index_of("Show Grid").expect("Show Grid"));
        assert!(!beside.exists());
        let again = UserSettings::load(Some(path.clone()));
        assert_eq!(again.flag("Show Grid"), settings.flag("Show Grid"));
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }

    #[test]
    fn the_grid_settings_change_the_background_style_and_persist() {
        let (mut settings, path) = fresh("grid-settings");
        let step = index_of("Fine Grid Step").expect("step");
        let width = index_of("Fine Grid Line Width").expect("width");
        let factor = index_of("Coarse Grid Step Factor").expect("factor");
        settings.set_fraction(step, 0.0); // 10 u
        settings.set_fraction(width, 1.0); // 10
        settings.set_fraction(factor, 1.0); // x100
        let mut style = crate::assets::BackgroundStyle::default();
        settings.apply_grid(&mut style);
        assert_eq!(style.fine_step, 10.0);
        assert_eq!(style.coarse_step, 1000.0, "10 u x 100");
        assert_eq!(style.fine_width, 10.0, "world units");
        assert_eq!(style.coarse_width, 50.0, "10 u x the width factor (5)");
        assert_eq!(settings.display(width), "10.0");
        assert_eq!(settings.display(step), "10");
        // The viewer's own file keeps them across a restart.
        let again = UserSettings::load(Some(path.clone()));
        assert_eq!(again.number("Fine Grid Step"), 10.0);
        assert_eq!(again.number("Fine Grid Line Width"), 10.0);
        let _ = std::fs::remove_dir_all(path.parent().expect("dir"));
    }
}
