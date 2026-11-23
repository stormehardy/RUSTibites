//! RUSTibites Improved: two check boxes on the Species Distribution window (the Position tab of the species panels) that switch off what it writes over each zone of
//! its map: the zone's NAME, and its BIOMASS (with the number of pellets that comes to). They are a way of looking, not part of the world: the game shows both, so both are on
//! from the start; the player's choice is kept in the user settings (`Species Map Zone Names`, `Species Map Zone Biomass`: not on the Settings screens, the check boxes
//! are on the window itself) and nothing in the simulation notices it. The Zones tab's preview of the world's parameters, which carries the same labels, is left as the game
//! has it.

use super::App;

impl App {
    /// The "Zone names" check box of the Species Distribution window.
    pub fn toggle_map_zone_names(&mut self) {
        self.map_zone_names = !self.map_zone_names;
        self.last_action = if self.map_zone_names { "species map: zone names shown" } else { "species map: zone names hidden" }.to_owned();
    }

    /// The "Zone biomass" check box: the biomass line and the pellet count under it.
    pub fn toggle_map_zone_biomass(&mut self) {
        self.map_zone_biomass = !self.map_zone_biomass;
        self.last_action = if self.map_zone_biomass { "species map: zone biomass shown" } else { "species map: zone biomass hidden" }.to_owned();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;
    use crate::ui::UiAction;
    use bibites_core::math::Vec2;

    fn app() -> App {
        let loaded = LoadedWorld::load(None, 77, 6, 1).expect("starter world");
        App::new(loaded, Vec2::new(1920.0, 1080.0), None)
    }

    #[test]
    fn both_labels_are_shown_from_the_start_as_the_game_shows_them_and_each_check_box_flips_its_own() {
        let mut app = app();
        assert!(app.map_zone_names && app.map_zone_biomass);
        app.apply_ui_actions(&[UiAction::ToggleMapZoneNames]);
        assert!(!app.map_zone_names && app.map_zone_biomass, "only the names went");
        assert_eq!(app.last_action, "species map: zone names hidden");
        app.apply_ui_actions(&[UiAction::ToggleMapZoneBiomass]);
        assert!(!app.map_zone_names && !app.map_zone_biomass);
        assert_eq!(app.last_action, "species map: zone biomass hidden");
        app.apply_ui_actions(&[UiAction::ToggleMapZoneNames, UiAction::ToggleMapZoneBiomass]);
        assert!(app.map_zone_names && app.map_zone_biomass);
    }

    #[test]
    fn the_choices_are_user_settings_that_are_on_by_default() {
        for label in ["Species Map Zone Names", "Species Map Zone Biomass"] {
            let index = crate::user_settings::index_of(label).unwrap_or_else(|| panic!("a setting called {label}"));
            let settings = crate::user_settings::UserSettings::load(None);
            assert!(settings.flag(label), "{label}: on from the start");
            let _ = index;
        }
    }
}
