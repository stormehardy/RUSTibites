//! RUSTibites Improved: keeping the force towers beside the save (`force_towers/<save name>.json` in the state folder; never inside a save, which stays in the game's
//! format: the original game has no such tower and would not know what to do with one). A tower is its position, its settings and the name of the zone it follows;
//! what it is doing now (how many bodies it moved last tick) is not kept.

use bibites_core::force_tower::{ForceFalloff, ForceTower};
use bibites_core::math::Vec2;
use bibites_core::tools::Tool;
use bibites_io::storage::{ForceTowerRecord, ForceTowersFile, read_force_towers, write_force_towers};

use super::App;

/// The stored form of a force tower (`zone`: the name of the zone it follows).
fn record_of(tower: &ForceTower, zone: Option<String>) -> ForceTowerRecord {
    ForceTowerRecord {
        x: tower.base.pos.x,
        y: tower.base.pos.y,
        range: tower.range,
        strength: tower.strength,
        falloff: tower.falloff.key().to_owned(),
        bibites: tower.bibites,
        pellets: tower.pellets,
        eggs: tower.eggs,
        zone,
    }
}

impl App {
    /// The force towers of the world, as stored.
    fn force_towers_file(&self) -> ForceTowersFile {
        ForceTowersFile {
            towers: self
                .world
                .tools
                .tools()
                .iter()
                .filter_map(|tool| {
                    let Tool::Force(tower) = tool else {
                        return None;
                    };
                    let zone = tower.base.attached_zone.and_then(|id| self.world.zones.get(id)).map(|zone| zone.config.0.name.clone());
                    Some(record_of(tower, zone))
                })
                .collect(),
        }
    }

    /// Stores the force towers under the name of the save the world was saved or loaded as (nothing is stored for a world that has no such name yet).
    pub(crate) fn force_towers_persist(&mut self) {
        let Some(key) = self.time_tags_key.clone() else {
            return;
        };
        if let Err(error) = write_force_towers(&self.storage, &key, &self.force_towers_file()) {
            self.last_action = format!("force towers not saved: {error}");
        }
    }

    /// The world was loaded from the save `stem`: the world's force towers are the ones stored for it (none stored: none).
    pub(crate) fn force_towers_after_load(&mut self, stem: &str) {
        self.remove_force_towers();
        match read_force_towers(&self.storage, stem) {
            Ok(file) => {
                for record in &file.towers {
                    self.add_force_tower_from(record);
                }
            }
            Err(error) => self.last_action = format!("force towers of {stem} not read: {error}"),
        }
    }

    /// An autosave is about to be written as `autosave_<stamp>.zip`: the towers are stored beside it under its name (the world's own key stays).
    pub(crate) fn force_towers_beside_autosave(&mut self, stamp: &str) {
        let file = self.force_towers_file();
        if file.towers.is_empty() {
            return;
        }
        if let Err(error) = write_force_towers(&self.storage, &format!("autosave_{stamp}"), &file) {
            self.last_action = format!("force towers of the autosave not saved: {error}");
        }
    }

    /// Takes every force tower out of the world (a world that is loaded has none of the earlier world's).
    fn remove_force_towers(&mut self) {
        loop {
            let found = (0..self.world.tools.len()).find(|&index| matches!(self.world.tools.tools().get(index), Some(Tool::Force(_))));
            let Some(index) = found else {
                return;
            };
            let Some(id) = bibites_core::tools::ToolStore::tool_id_at(index) else {
                return;
            };
            if !self.world.remove_tool(id) {
                return;
            }
        }
    }

    /// Puts a stored force tower into the world (every value is kept inside its range, whatever the file says; a falloff the program does not know is the default).
    fn add_force_tower_from(&mut self, record: &ForceTowerRecord) {
        let id = self.world.place_force_tower_at(Vec2::new(record.x, record.y));
        let zone = record.zone.as_ref().and_then(|name| self.world.zones.zones.iter().find(|zone| &zone.config.0.name == name)).map(|zone| zone.id);
        let Some(Tool::Force(tower)) = self.world.tools.get_mut(id) else {
            return;
        };
        tower.set_range(record.range);
        tower.set_strength(record.strength);
        tower.falloff = ForceFalloff::from_key(&record.falloff).unwrap_or_default();
        tower.bibites = record.bibites;
        tower.pellets = record.pellets;
        tower.eggs = record.eggs;
        // The zone it followed, found by its name (the first of that name); a zone that is gone leaves it where it was.
        if zone.is_some() {
            self.world.set_tool_attached_zone(id, zone);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;
    use crate::tower_panel as panel;
    use crate::ui::{DropdownTarget, ToolKind, UiAction};
    use bibites_io::storage::StoragePaths;

    /// An app with a state folder of its own (the tests must not touch the player's).
    fn app(name: &str) -> (App, std::path::PathBuf) {
        let loaded = LoadedWorld::load(None, 77, 6, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        let root = std::env::temp_dir().join(format!("rustibites_force_towers_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp folder");
        app.storage = StoragePaths::new(&root);
        (app, root)
    }

    fn towers(app: &App) -> Vec<ForceTower> {
        app.world.tools.tools().iter().filter_map(|tool| if let Tool::Force(tower) = tool { Some(*tower) } else { None }).collect()
    }

    fn tower_id(app: &App) -> bibites_core::ids::ToolId {
        let index = app.world.tools.tools().iter().position(|tool| matches!(tool, Tool::Force(_))).expect("a force tower");
        bibites_core::tools::ToolStore::tool_id_at(index).expect("an id")
    }

    #[test]
    fn the_placer_puts_a_force_tower_down_selects_it_and_it_starts_as_a_pull_that_fades_over_everything() {
        let (mut app, root) = app("place");
        assert!(towers(&app).is_empty());
        app.tool = ToolKind::Force;
        app.apply_tool(Vec2::new(120.0, -40.0));
        let placed = towers(&app);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].base.pos, Vec2::new(120.0, -40.0));
        assert_eq!((placed[0].range, placed[0].strength), (bibites_core::force_tower::FORCE_RANGE_DEFAULT, bibites_core::force_tower::FORCE_STRENGTH_DEFAULT));
        assert_eq!(placed[0].falloff, ForceFalloff::Fading);
        assert!(placed[0].bibites && placed[0].pellets && placed[0].eggs);
        assert_eq!(app.tool, ToolKind::Inspect, "a tower is placed once");
        assert!(app.selected_tool_id().is_some(), "its panel opens");
        assert!(app.last_action.contains("force"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_panels_controls_change_the_tower_and_stay_inside_their_ranges() {
        use bibites_core::force_tower::{FORCE_RANGE_MAX, FORCE_RANGE_MIN, FORCE_STRENGTH_MAX, FORCE_STRENGTH_MIN};
        let (mut app, root) = app("controls");
        let id = app.world.place_force_tower_at(Vec2::ZERO);
        app.select_placed_tool(id);
        let get = |app: &App| towers(app)[0];
        panel::slide(&mut app.world, id, panel::FORCE_RANGE, 1.0);
        assert!((get(&app).range - FORCE_RANGE_MAX).abs() < 1e-1, "{}", get(&app).range);
        panel::slide(&mut app.world, id, panel::FORCE_RANGE, 0.0);
        assert!((get(&app).range - FORCE_RANGE_MIN).abs() < 1e-2);
        panel::slide(&mut app.world, id, panel::STRENGTH, 0.0);
        assert!((get(&app).strength - FORCE_STRENGTH_MIN).abs() < 1e-2, "the left end is the strongest push");
        panel::slide(&mut app.world, id, panel::STRENGTH, 1.0);
        assert!((get(&app).strength - FORCE_STRENGTH_MAX).abs() < 1e-2, "the right end is the strongest pull");
        panel::slide(&mut app.world, id, panel::STRENGTH, 0.5);
        assert!(get(&app).strength.abs() < 1e-2, "the middle is no force");
        // The switches, through the panel's own actions.
        assert_eq!(get(&app).falloff, ForceFalloff::Fading);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::FALLOFF)]);
        assert_eq!(get(&app).falloff, ForceFalloff::Constant);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::FALLOFF)]);
        assert_eq!(get(&app).falloff, ForceFalloff::Fading);
        for (field, read) in [
            (panel::ON_BIBITES, (|tower: &ForceTower| tower.bibites) as fn(&ForceTower) -> bool),
            (panel::ON_PELLETS, |tower: &ForceTower| tower.pellets),
            (panel::ON_EGGS, |tower: &ForceTower| tower.eggs),
        ] {
            assert!(read(&get(&app)));
            app.apply_ui_actions(&[UiAction::ToolSwitch(field)]);
            assert!(!read(&get(&app)), "field {field} switched off");
            app.apply_ui_actions(&[UiAction::ToolSwitch(field)]);
            assert!(read(&get(&app)), "field {field} switched on again");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_towers_are_kept_beside_the_save_by_its_name_and_come_back_with_what_they_were_set_to() {
        let (mut app, root) = app("persist");
        // A world without towers writes no file.
        app.time_tags_key = Some("My save".to_owned());
        app.force_towers_persist();
        assert!(!app.storage.force_towers_path("My save").exists());
        let first = app.world.place_force_tower_at(Vec2::new(5.0, 6.0));
        let second = app.world.place_force_tower_at(Vec2::new(-70.0, 80.0));
        panel::slide(&mut app.world, first, panel::FORCE_RANGE, 0.7);
        panel::slide(&mut app.world, first, panel::STRENGTH, 0.9);
        panel::flip_switch(&mut app.world, second, panel::FALLOFF);
        panel::flip_switch(&mut app.world, second, panel::ON_PELLETS);
        panel::flip_switch(&mut app.world, second, panel::ON_EGGS);
        panel::slide(&mut app.world, second, panel::STRENGTH, 0.1);
        let before = towers(&app);
        app.force_towers_persist();
        // Loaded as that save: the towers of the world are the stored ones (what was in the world before goes), nothing else of the world changes.
        let _ = app.world.place_force_tower_at(Vec2::new(999.0, 999.0));
        app.force_towers_after_load("My save");
        let after = towers(&app);
        assert_eq!(after.len(), 2);
        for (a, b) in before.iter().zip(&after) {
            assert_eq!((a.base.pos, a.range, a.strength, a.falloff), (b.base.pos, b.range, b.strength, b.falloff));
            assert_eq!((a.bibites, a.pellets, a.eggs), (b.bibites, b.pellets, b.eggs));
        }
        assert!(after[1].strength < 0.0 && after[1].falloff == ForceFalloff::Constant && after[1].bibites && !after[1].pellets && !after[1].eggs, "a push on bibites only");
        // A save that has none stored: none.
        app.force_towers_after_load("Never saved");
        assert!(towers(&app).is_empty());
        // An autosave gets its own file; one with no towers gets none.
        app.force_towers_after_load("My save");
        app.force_towers_beside_autosave("2026-11-18_20-00");
        assert!(app.storage.force_towers_path("autosave_2026-11-18_20-00").exists());
        app.force_towers_after_load("Never saved");
        app.force_towers_beside_autosave("2026-11-18_20-05");
        assert!(!app.storage.force_towers_path("autosave_2026-11-18_20-05").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_tower_follows_a_zone_keeps_the_zone_by_its_name_and_stays_put_when_the_zone_is_gone() {
        let (mut app, root) = app("follow");
        assert!(!app.world.zones.zones.is_empty(), "the starter world has zones");
        let (zone_id, zone_pos) = {
            let zone = &app.world.zones.zones[0];
            (zone.id, zone.pos)
        };
        let id = app.world.place_force_tower_at(zone_pos + Vec2::new(30.0, 0.0));
        app.select_placed_tool(id);
        // The tower's "follow zone" list is the colour killer's: "None" and the zones closest to it.
        let choices = panel::zone_choices(&app.world, app.world.tools.get(id).expect("the tower"));
        let index = choices.zones.iter().position(|zone| *zone == Some(zone_id)).expect("the zone 30u away is among the closest");
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::ToolZone), UiAction::DropdownPick(index as u16)]);
        assert_eq!((towers(&app)[0].base.attached_zone, towers(&app)[0].base.pos), (Some(zone_id), zone_pos));
        app.time_tags_key = Some("Followed".to_owned());
        app.force_towers_persist();
        app.force_towers_after_load("Followed");
        assert_eq!(towers(&app)[0].base.attached_zone, Some(zone_id));
        // The zone renamed: the tower stays where it was stored, free.
        app.world.zones.zones[0].config.0.name = "Another zone".to_owned();
        app.force_towers_after_load("Followed");
        assert_eq!(towers(&app)[0].base.attached_zone, None);
        assert!(app.world.set_tool_position(tower_id(&app), Vec2::new(1.0, 2.0)), "a tower that follows no zone can be moved");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn values_outside_the_ranges_an_unknown_falloff_and_a_broken_file_are_kept_inside_and_said() {
        use bibites_core::force_tower::{FORCE_RANGE_MIN, FORCE_STRENGTH_MAX};
        let (mut app, root) = app("broken");
        let path = app.storage.force_towers_path("Odd");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"towers":[{"x":1.0,"y":2.0,"range":-5.0,"strength":1.0e9,"falloff":"sideways"}]}"#).unwrap();
        app.force_towers_after_load("Odd");
        let loaded = towers(&app);
        assert_eq!(loaded.len(), 1);
        assert_eq!((loaded[0].range, loaded[0].strength, loaded[0].falloff), (FORCE_RANGE_MIN, FORCE_STRENGTH_MAX, ForceFalloff::Fading));
        std::fs::write(&path, "not json").unwrap();
        app.force_towers_after_load("Odd");
        assert!(towers(&app).is_empty() && app.last_action.contains("not read"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(root);
    }
}
