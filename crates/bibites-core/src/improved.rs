//! The switches of RUSTibites Improved's changes to the simulation.
//!
//! The rule (`docs/IMPROVED.md`): a change to what a tick computes is a [`Feature`], OFF unless it is asked for. [`ImprovedFeatures::default`]
//! (every switch off) IS the Faithful simulation; the pinned runs (`evolution_regression_fingerprint`, `birth_path_is_bit_identical_to_pre_optimization_golden`
//! in `species.rs`) are what keeps that true, and their numbers are never edited on the `improved` branch.
//!
//! The switches belong to the RUN, not to the world's data: a [`crate::world::World`] holds them in `World::improved`, they are NOT written into a save
//! (a save stays the original game's format) and they are NOT part of the state fingerprint (so a switch can never make two runs look different by
//! itself: only what it changes in the simulation does).
//!
//! # Adding a feature
//!
//! 1. A variant of [`Feature`], and a row in each of its methods (`index`, `key`, `title`, `description`) and in [`Feature::ALL`]. The key is stable
//!    (lower snake case): it is what the settings file and `RUSTIBITES_IMPROVED` name it by.
//! 2. In the simulation: `if self.improved.is_on(Feature::TheFeature) { the new behaviour } else { exactly what the game does }`. Nothing else may
//!    differ with the switch off, not even the order of random numbers drawn.
//! 3. Tests: the feature on, the feature off, and the pinned runs unchanged.
//!
//! [`Feature::SwitchProbe`] changes nothing in the simulation. It exists so that the whole chain (settings file / environment -> viewer -> world) can
//! be checked before the first real feature does, and may be removed once there are real ones.

/// One simulation change of RUSTibites Improved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    /// Does nothing to the simulation: proves that a switch reaches the world (see the module's documentation).
    SwitchProbe,
}

impl Feature {
    /// Every feature, in the order they are listed to the player.
    pub const ALL: &'static [Feature] = &[Feature::SwitchProbe];

    /// The feature's place in the switches' bit set (stable: never reuse a number of a removed feature that a settings file may still name).
    #[must_use]
    const fn index(self) -> u32 {
        match self {
            Feature::SwitchProbe => 0,
        }
    }

    /// The stable name the settings file and the environment use.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Feature::SwitchProbe => "switch_probe",
        }
    }

    /// The name shown to the player.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Feature::SwitchProbe => "Switch probe",
        }
    }

    /// One sentence for the player: what the switch changes.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Feature::SwitchProbe => "Changes nothing in the simulation; it only shows that the switches reach the world.",
        }
    }

    /// The feature a key names (case and surrounding spaces do not matter).
    #[must_use]
    pub fn from_key(key: &str) -> Option<Feature> {
        let key = key.trim();
        Feature::ALL.iter().copied().find(|feature| feature.key().eq_ignore_ascii_case(key))
    }
}

// The switches are one `u64`.
const _: () = assert!(Feature::ALL.len() <= 64);

/// Which of the [`Feature`]s are on. Small, `Copy`, and all off by default (the Faithful simulation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ImprovedFeatures {
    bits: u64,
}

impl ImprovedFeatures {
    /// Every switch off: the Faithful simulation.
    pub const FAITHFUL: Self = Self { bits: 0 };

    /// Whether `feature` is on.
    #[must_use]
    pub const fn is_on(self, feature: Feature) -> bool {
        self.bits & (1u64 << feature.index()) != 0
    }

    /// Whether every switch is off.
    #[must_use]
    pub const fn is_faithful(self) -> bool {
        self.bits == 0
    }

    /// Turns `feature` on or off.
    pub fn set(&mut self, feature: Feature, on: bool) {
        let bit = 1u64 << feature.index();
        if on {
            self.bits |= bit;
        } else {
            self.bits &= !bit;
        }
    }

    /// A copy with `feature` turned on or off.
    #[must_use]
    pub fn with(mut self, feature: Feature, on: bool) -> Self {
        self.set(feature, on);
        self
    }

    /// The features that are on, in [`Feature::ALL`]'s order.
    pub fn enabled(self) -> impl Iterator<Item = Feature> {
        Feature::ALL.iter().copied().filter(move |feature| self.is_on(*feature))
    }

    /// The keys of the features that are on (what a settings file stores).
    #[must_use]
    pub fn enabled_keys(self) -> Vec<&'static str> {
        self.enabled().map(Feature::key).collect()
    }

    /// The switches that `keys` name. Keys that name no feature (a typo, or a feature of another version of the program) are returned
    /// beside the result instead of being lost without a word.
    pub fn from_keys<'a>(keys: impl IntoIterator<Item = &'a str>) -> (Self, Vec<String>) {
        let mut features = Self::FAITHFUL;
        let mut unknown = Vec::new();
        for key in keys {
            if key.trim().is_empty() {
                continue;
            }
            match Feature::from_key(key) {
                Some(feature) => features.set(feature, true),
                None => unknown.push(key.trim().to_owned()),
            }
        }
        (features, unknown)
    }

    /// The switches a comma separated list names (`RUSTIBITES_IMPROVED`); see [`Self::from_keys`].
    #[must_use]
    pub fn from_list(list: &str) -> (Self, Vec<String>) {
        Self::from_keys(list.split(','))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SimSettings;
    use crate::sim::state_fingerprint;
    use crate::world::{ScenarioData, World};

    #[test]
    fn every_switch_is_off_by_default_and_that_is_the_faithful_simulation() {
        let features = ImprovedFeatures::default();
        assert!(features.is_faithful());
        assert_eq!(features, ImprovedFeatures::FAITHFUL);
        for feature in Feature::ALL {
            assert!(!features.is_on(*feature), "{feature:?} must be off by default");
        }
        assert!(features.enabled_keys().is_empty());
    }

    #[test]
    fn a_switch_turns_on_and_off_and_nothing_else_moves() {
        let mut features = ImprovedFeatures::default();
        features.set(Feature::SwitchProbe, true);
        assert!(features.is_on(Feature::SwitchProbe) && !features.is_faithful());
        assert_eq!(features.enabled_keys(), vec!["switch_probe"]);
        assert_eq!(features.enabled().collect::<Vec<_>>(), vec![Feature::SwitchProbe]);
        features.set(Feature::SwitchProbe, true); // twice: still on, still one bit
        assert_eq!(features, ImprovedFeatures::FAITHFUL.with(Feature::SwitchProbe, true));
        features.set(Feature::SwitchProbe, false);
        assert!(features.is_faithful());
    }

    #[test]
    fn every_feature_has_a_distinct_stable_key_bit_and_texts() {
        let mut keys = std::collections::BTreeSet::new();
        let mut bits = std::collections::BTreeSet::new();
        for feature in Feature::ALL {
            assert!(keys.insert(feature.key()), "duplicate key {}", feature.key());
            assert!(bits.insert(feature.index()), "duplicate bit of {}", feature.key());
            assert!(!feature.title().is_empty() && !feature.description().is_empty());
            assert_eq!(feature.key(), feature.key().to_ascii_lowercase(), "keys are lower case");
            assert_eq!(Feature::from_key(feature.key()), Some(*feature));
        }
        // Keys written into settings files stay valid: this one must never change.
        assert_eq!(Feature::SwitchProbe.key(), "switch_probe");
        assert_eq!(Feature::SwitchProbe.index(), 0);
    }

    #[test]
    fn keys_are_read_loosely_and_unknown_ones_are_reported() {
        let (features, unknown) = ImprovedFeatures::from_list(" Switch_Probe , nonsense,, ");
        assert!(features.is_on(Feature::SwitchProbe));
        assert_eq!(unknown, vec!["nonsense".to_owned()]);
        let (none, unknown) = ImprovedFeatures::from_list("");
        assert!(none.is_faithful() && unknown.is_empty(), "an empty list is every switch off");
        let (only_unknown, unknown) = ImprovedFeatures::from_keys(["future_feature"]);
        assert!(only_unknown.is_faithful());
        assert_eq!(unknown, vec!["future_feature".to_owned()]);
        // What a settings file stores comes back as the same switches.
        let stored = ImprovedFeatures::FAITHFUL.with(Feature::SwitchProbe, true).enabled_keys();
        assert_eq!(ImprovedFeatures::from_keys(stored.iter().copied()).0, ImprovedFeatures::FAITHFUL.with(Feature::SwitchProbe, true));
    }

    #[test]
    fn a_new_world_has_every_switch_off_and_the_switches_are_not_part_of_the_state_fingerprint() {
        let settings = SimSettings::game_defaults();
        let mut off = World::new(settings.clone(), ScenarioData::default(), 11).expect("world");
        let mut on = World::new(settings, ScenarioData::default(), 11).expect("world");
        assert!(off.improved.is_faithful(), "a world starts Faithful");
        on.improved.set(Feature::SwitchProbe, true);
        for _ in 0..40 {
            off.tick();
            on.tick();
        }
        assert_eq!(state_fingerprint(&off), state_fingerprint(&on), "a switch that changes nothing must change nothing, hash included");
        assert_eq!(off.simulated_time.to_bits(), on.simulated_time.to_bits());
    }
}
