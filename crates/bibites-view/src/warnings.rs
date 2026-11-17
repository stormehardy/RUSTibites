//! The game's action warnings (`SettingScripts.ActionWarnings`): confirmations with a "do not show
//! again" checkbox (`PopupManager.DisplayActionWarning`). Whether one is switched off is the
//! `PlayerPrefs` int under its key (`ActionWarning.val`: `GetInt(key, 0) == 1`).

/// One of the game's action warnings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionWarning {
    /// Deleting a bibite template file in the Library (`BibiteTemplateSelectorPanel.DeleteSelected`).
    DeleteBibiteFile,
    /// Deleting a save in the Load Game / Save Game panels.
    DeleteSaveFile,
    /// Pruning a species of the lineage (`SpeciesInfoPanel.PruneSpecies`).
    PruneSpecies,
    /// Starting the simulation from a scenario edited in the Scenario Editor.
    StartScenarioAfterEdit,
    /// Deleting a Steam workshop item (not part of this port: there is no workshop).
    DeleteWorkshopItem,
}

/// Every warning, in the order of `ActionWarnings`.
pub const ALL: [ActionWarning; 5] = [
    ActionWarning::DeleteBibiteFile,
    ActionWarning::DeleteSaveFile,
    ActionWarning::PruneSpecies,
    ActionWarning::StartScenarioAfterEdit,
    ActionWarning::DeleteWorkshopItem,
];

/// `ActionWarnings.allInfoShown`: the warnings the Settings screen's "Reset Warnings" button
/// (`UserSettingsManager.ResetAllWarnings`) shows again. The file and workshop warnings are not
/// in the list, so that button leaves them as they are.
pub const ALL_INFO_SHOWN: [ActionWarning; 3] = [
    ActionWarning::DeleteSaveFile,
    ActionWarning::PruneSpecies,
    ActionWarning::StartScenarioAfterEdit,
];

impl ActionWarning {
    /// The `PlayerPrefs` key.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::DeleteBibiteFile => "fileDeletionWarning",
            Self::DeleteSaveFile => "saveDeletionWarning",
            Self::PruneSpecies => "pruneSpeciesWarning",
            Self::StartScenarioAfterEdit => "startScenarioAfterEdit",
            Self::DeleteWorkshopItem => "deleteWorkshopItem",
        }
    }

    /// The popup's title (`ActionWarning.title`).
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::DeleteBibiteFile => "Deleting Bibite File",
            Self::DeleteSaveFile => "Deleting Save File",
            Self::PruneSpecies => "Pruning Species Warning",
            // `ScenariosEditor.StartSimulation` passes its own title.
            Self::StartScenarioAfterEdit => "Starting Simulation After Edit",
            Self::DeleteWorkshopItem => "Delete Workshop Item",
        }
    }

    /// The popup's text (`ActionWarning.warningText`).
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::DeleteBibiteFile => {
                "Are you sure you want to delete this file?\nThis action cannot be reverted."
            }
            Self::DeleteSaveFile => "Are you sure you want to delete this save file?",
            Self::PruneSpecies => {
                "You're about to prune a species, this process is IRREVERSIBLE!\nAre you sure?"
            }
            Self::StartScenarioAfterEdit => {
                "You're about to launch the Simulation from a modified Scenario. Make sure to save the scenario first if you intend to use it again later.\nIt's also possible to save the scenario once in-game.\n\nAre you sure you are Ready"
            }
            Self::DeleteWorkshopItem => {
                "You're about to permanently and irreversibly delete your workshop Item.\n\nAre you sure?"
            }
        }
    }

    /// The popup's title as the caller shows it: `DisplayActionWarning(.., title)` overrides it
    /// (`ScenariosEditor.StartSimulation` says "Launching Simulation").
    #[must_use]
    pub fn shown_title(self) -> &'static str {
        match self {
            Self::StartScenarioAfterEdit => "Launching Simulation",
            other => other.title(),
        }
    }

    /// The proceed button's label: "Yes" unless the caller gives one
    /// (`ScenariosEditor.StartSimulation` passes "YES").
    #[must_use]
    pub fn proceed_label(self) -> &'static str {
        match self {
            Self::StartScenarioAfterEdit => "YES",
            _ => "Yes",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_texts_are_the_games() {
        assert_eq!(ActionWarning::DeleteBibiteFile.key(), "fileDeletionWarning");
        assert_eq!(ActionWarning::DeleteSaveFile.key(), "saveDeletionWarning");
        assert_eq!(ActionWarning::PruneSpecies.key(), "pruneSpeciesWarning");
        assert_eq!(
            ActionWarning::StartScenarioAfterEdit.key(),
            "startScenarioAfterEdit"
        );
        assert_eq!(
            ActionWarning::DeleteWorkshopItem.key(),
            "deleteWorkshopItem"
        );
        assert_eq!(ActionWarning::DeleteSaveFile.title(), "Deleting Save File");
        assert_eq!(
            ActionWarning::PruneSpecies.text(),
            "You're about to prune a species, this process is IRREVERSIBLE!\nAre you sure?"
        );
        // The keys are distinct, so one warning's flag never switches another off.
        let mut keys: Vec<_> = ALL.iter().map(|warning| warning.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ALL.len());
    }

    #[test]
    fn reset_warnings_covers_only_the_games_info_list() {
        assert_eq!(ALL_INFO_SHOWN.len(), 3);
        assert!(!ALL_INFO_SHOWN.contains(&ActionWarning::DeleteBibiteFile));
        assert!(!ALL_INFO_SHOWN.contains(&ActionWarning::DeleteWorkshopItem));
        assert!(ALL_INFO_SHOWN.contains(&ActionWarning::DeleteSaveFile));
    }

    #[test]
    fn the_scenario_start_warning_has_the_callers_title_and_button() {
        let warning = ActionWarning::StartScenarioAfterEdit;
        assert_eq!(warning.shown_title(), "Launching Simulation");
        assert_eq!(warning.proceed_label(), "YES");
        assert_eq!(ActionWarning::DeleteSaveFile.proceed_label(), "Yes");
        assert_eq!(
            ActionWarning::DeleteSaveFile.shown_title(),
            "Deleting Save File"
        );
    }
}
