//! RUSTibites Improved's simulation switches, as the viewer holds them: which are on is decided ONCE at start and applied to whatever world the app
//! holds, before every tick it runs (see `App::tick_world`). The switches themselves, and the rule that all off is the Faithful simulation, are in
//! `bibites_core::improved`.
//!
//! Where the choice comes from, strongest first:
//! 1. the environment variable `RUSTIBITES_IMPROVED`, a comma separated list of feature keys (`switch_probe,other`); set but empty means every switch
//!    off, whatever the file says (an explicit choice, like `RUSTIBITES_DATA_DIR`);
//! 2. `improved_features.json` in the port's state folder (`./userdata`, or `RUSTIBITES_DATA_DIR`): `{"enabled": ["switch_probe"]}`;
//! 3. nothing: every switch off.

use std::sync::OnceLock;

use bibites_core::improved::ImprovedFeatures;
use bibites_io::storage::StoragePaths;

/// The environment variable that chooses the switches for one run.
pub const ENV: &str = "RUSTIBITES_IMPROVED";

static CONFIGURED: OnceLock<ImprovedFeatures> = OnceLock::new();

/// The switches chosen for this run: every one off until [`configure`] ran (the tests, and any code path that starts without it).
#[must_use]
pub fn configured() -> ImprovedFeatures {
    CONFIGURED.get().copied().unwrap_or_default()
}

/// What was chosen, and the line that says so.
#[derive(Debug, PartialEq, Eq)]
pub struct Choice {
    pub features: ImprovedFeatures,
    /// One line for the log: what is on, where that came from, and anything that was ignored.
    pub report: String,
}

/// Decides the switches from what the stored file held (`Err`: it could not be read) and the environment variable's value (`None`: not set).
#[must_use]
pub fn choose(stored: Result<Vec<String>, String>, env: Option<&str>) -> Choice {
    let (features, unknown, source, note) = match (env, stored) {
        (Some(list), _) => {
            let (features, unknown) = ImprovedFeatures::from_list(list);
            (features, unknown, format!("from {ENV}"), String::new())
        }
        (None, Ok(keys)) => {
            let (features, unknown) = ImprovedFeatures::from_keys(keys.iter().map(String::as_str));
            // An empty list is also what a missing file reads as: say "nothing chosen" then, not that a file said so.
            let source = if keys.is_empty() { "nothing chosen" } else { "from improved_features.json" };
            (features, unknown, source.to_owned(), String::new())
        }
        (None, Err(error)) => (
            ImprovedFeatures::FAITHFUL,
            Vec::new(),
            "improved_features.json is unreadable".to_owned(),
            format!("; every switch stays off ({error})"),
        ),
    };
    let mut report = if features.is_faithful() {
        format!("improved features: none on, the Faithful simulation ({source})")
    } else {
        format!("improved features on: {} ({source})", features.enabled_keys().join(", "))
    };
    if !unknown.is_empty() {
        report.push_str(&format!("; ignored, no such feature: {}", unknown.join(", ")));
    }
    report.push_str(&note);
    Choice { features, report }
}

/// Reads the file and the environment, remembers the choice for [`configured`] (the first call wins) and returns the line to log.
pub fn configure(paths: &StoragePaths) -> String {
    let stored = bibites_io::storage::read_improved_features(paths).map(|file| file.enabled).map_err(|error| error.to_string());
    let choice = choose(stored, std::env::var(ENV).ok().as_deref());
    let _ = CONFIGURED.set(choice.features);
    choice.report
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::improved::Feature;

    fn probe() -> ImprovedFeatures {
        ImprovedFeatures::FAITHFUL.with(Feature::SwitchProbe, true)
    }

    #[test]
    fn nothing_chosen_is_the_faithful_simulation() {
        let choice = choose(Ok(Vec::new()), None);
        assert!(choice.features.is_faithful());
        assert!(choice.report.contains("none on") && choice.report.contains("Faithful") && choice.report.contains("nothing chosen"), "{}", choice.report);
        assert!(configured().is_faithful(), "before configure ran every switch is off");
    }

    #[test]
    fn the_file_turns_a_switch_on_and_the_environment_overrides_it_either_way() {
        let stored = || Ok(vec!["switch_probe".to_owned()]);
        let from_file = choose(stored(), None);
        assert_eq!(from_file.features, probe());
        assert!(from_file.report.contains("switch_probe") && from_file.report.contains("improved_features.json"), "{}", from_file.report);
        // Set and empty: an explicit "all off".
        assert!(choose(stored(), Some("")).features.is_faithful());
        // Set to a key: on, with the file empty.
        let from_env = choose(Ok(Vec::new()), Some("Switch_Probe"));
        assert_eq!(from_env.features, probe());
        assert!(from_env.report.contains(ENV), "{}", from_env.report);
    }

    #[test]
    fn unknown_keys_and_an_unreadable_file_are_said_and_never_turn_anything_on() {
        let typo = choose(Ok(vec!["switch_prob".to_owned()]), None);
        assert!(typo.features.is_faithful());
        assert!(typo.report.contains("no such feature: switch_prob"), "{}", typo.report);
        let broken = choose(Err("invalid improved_features.json: expected value".to_owned()), None);
        assert!(broken.features.is_faithful());
        assert!(broken.report.contains("unreadable") && broken.report.contains("every switch stays off"), "{}", broken.report);
        // The environment still wins over a broken file.
        assert_eq!(choose(Err("x".to_owned()), Some("switch_probe")).features, probe());
    }
}
