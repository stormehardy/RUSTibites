//! RUSTibites Improved: keeping the whirlpool towers beside the save (`whirlpool_towers/<save name>.json` in the state folder; never inside a save, which stays in the game's
//! format: the original game has no such tower and would not know what to do with one). A tower is its position, its settings and the name of the zone it follows;
//! what it is doing now (how many bodies it moved last tick) is not kept.

use bibites_core::force_tower::ForceFalloff;
use bibites_core::math::Vec2;
use bibites_core::tools::Tool;
use bibites_core::whirlpool::{Spin, WhirlpoolTower};
use bibites_io::storage::{WhirlpoolTowerRecord, WhirlpoolTowersFile, read_whirlpool_towers, write_whirlpool_towers};

use super::App;

/// The stored form of a whirlpool tower (`zone`: the name of the zone it follows).
fn record_of(tower: &WhirlpoolTower, zone: Option<String>) -> WhirlpoolTowerRecord {
    WhirlpoolTowerRecord {
        x: tower.base.pos.x,
        y: tower.base.pos.y,
        range: tower.range,
        speed: tower.speed,
        spin: tower.spin.key().to_owned(),
        falloff: tower.falloff.key().to_owned(),
        bibites: tower.bibites,
        pellets: tower.pellets,
        eggs: tower.eggs,
        zone,
    }
}

impl App {
    /// The whirlpool towers of the world, as stored.
    fn whirlpool_towers_file(&self) -> WhirlpoolTowersFile {
        WhirlpoolTowersFile {
            towers: self
                .world
                .tools
                .tools()
                .iter()
                .filter_map(|tool| {
                    let Tool::Whirl(tower) = tool else {
                        return None;
                    };
                    let zone = tower.base.attached_zone.and_then(|id| self.world.zones.get(id)).map(|zone| zone.config.0.name.clone());
                    Some(record_of(tower, zone))
                })
                .collect(),
        }
    }

    /// Stores the whirlpool towers under the name of the save the world was saved or loaded as (nothing is stored for a world that has no such name yet).
    pub(crate) fn whirlpool_towers_persist(&mut self) {
        let Some(key) = self.time_tags_key.clone() else {
            return;
        };
        if let Err(error) = write_whirlpool_towers(&self.storage, &key, &self.whirlpool_towers_file()) {
            self.last_action = format!("whirlpool towers not saved: {error}");
        }
    }

    /// The world was loaded from the save `stem`: the world's whirlpool towers are the ones stored for it (none stored: none).
    pub(crate) fn whirlpool_towers_after_load(&mut self, stem: &str) {
        self.remove_whirlpool_towers();
        match read_whirlpool_towers(&self.storage, stem) {
            Ok(file) => {
                for record in &file.towers {
                    self.add_whirlpool_tower_from(record);
                }
            }
            Err(error) => self.last_action = format!("whirlpool towers of {stem} not read: {error}"),
        }
    }

    /// An autosave is about to be written as `autosave_<stamp>.zip`: the towers are stored beside it under its name (the world's own key stays).
    pub(crate) fn whirlpool_towers_beside_autosave(&mut self, stamp: &str) {
        let file = self.whirlpool_towers_file();
        if file.towers.is_empty() {
            return;
        }
        if let Err(error) = write_whirlpool_towers(&self.storage, &format!("autosave_{stamp}"), &file) {
            self.last_action = format!("whirlpool towers of the autosave not saved: {error}");
        }
    }

    /// Takes every whirlpool tower out of the world (a world that is loaded has none of the earlier world's).
    fn remove_whirlpool_towers(&mut self) {
        loop {
            let found = (0..self.world.tools.len()).find(|&index| matches!(self.world.tools.tools().get(index), Some(Tool::Whirl(_))));
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

    /// Puts a stored whirlpool tower into the world (every value is kept inside its range, whatever the file says; a spin or a falloff the program does not know is the default).
    fn add_whirlpool_tower_from(&mut self, record: &WhirlpoolTowerRecord) {
        let id = self.world.place_whirlpool_at(Vec2::new(record.x, record.y));
        let zone = record.zone.as_ref().and_then(|name| self.world.zones.zones.iter().find(|zone| &zone.config.0.name == name)).map(|zone| zone.id);
        let Some(Tool::Whirl(tower)) = self.world.tools.get_mut(id) else {
            return;
        };
        tower.set_range(record.range);
        tower.set_speed(record.speed);
        tower.spin = Spin::from_key(&record.spin).unwrap_or_default();
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
        let root = std::env::temp_dir().join(format!("rustibites_whirlpool_towers_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp folder");
        app.storage = StoragePaths::new(&root);
        (app, root)
    }

    fn towers(app: &App) -> Vec<WhirlpoolTower> {
        app.world.tools.tools().iter().filter_map(|tool| if let Tool::Whirl(tower) = tool { Some(*tower) } else { None }).collect()
    }

    fn tower_id(app: &App) -> bibites_core::ids::ToolId {
        let index = app.world.tools.tools().iter().position(|tool| matches!(tool, Tool::Whirl(_))).expect("a whirlpool tower");
        bibites_core::tools::ToolStore::tool_id_at(index).expect("an id")
    }

    #[test]
    fn the_placer_puts_a_whirlpool_tower_down_selects_it_and_it_starts_counterclockwise_with_a_fading_speed_over_everything() {
        let (mut app, root) = app("place");
        assert!(towers(&app).is_empty());
        app.tool = ToolKind::Whirl;
        app.apply_tool(Vec2::new(120.0, -40.0));
        let placed = towers(&app);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].base.pos, Vec2::new(120.0, -40.0));
        assert_eq!((placed[0].range, placed[0].speed), (bibites_core::whirlpool::WHIRL_RANGE_DEFAULT, bibites_core::whirlpool::WHIRL_SPEED_DEFAULT));
        assert_eq!((placed[0].spin, placed[0].falloff), (Spin::Counterclockwise, ForceFalloff::Fading));
        assert!(placed[0].bibites && placed[0].pellets && placed[0].eggs);
        assert_eq!(app.tool, ToolKind::Inspect, "a tower is placed once");
        assert!(app.selected_tool_id().is_some(), "its panel opens");
        assert!(app.last_action.contains("whirlpool"), "{}", app.last_action);
        // A right click while it is still to be placed cancels it, as for the other towers.
        app.tool = ToolKind::Whirl;
        assert_eq!(app.tool_view().ghost.map(|(kind, _)| kind), Some(crate::render::GhostKind::Whirl));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_panels_controls_change_the_tower_and_stay_inside_their_ranges_and_the_direction_button_turns_it_the_other_way() {
        use bibites_core::whirlpool::{WHIRL_RANGE_MAX, WHIRL_RANGE_MIN, WHIRL_SPEED_MAX, WHIRL_SPEED_MIN};
        let (mut app, root) = app("controls");
        let id = app.world.place_whirlpool_at(Vec2::ZERO);
        app.select_placed_tool(id);
        let get = |app: &App| towers(app)[0];
        panel::slide(&mut app.world, id, panel::FORCE_RANGE, 1.0);
        assert!((get(&app).range - WHIRL_RANGE_MAX).abs() < 1e-1, "{}", get(&app).range);
        panel::slide(&mut app.world, id, panel::FORCE_RANGE, 0.0);
        assert!((get(&app).range - WHIRL_RANGE_MIN).abs() < 1e-2);
        panel::slide(&mut app.world, id, panel::WHIRL_SPEED, 0.0);
        assert!((get(&app).speed - WHIRL_SPEED_MIN).abs() < 1e-2, "the left end is no speed");
        panel::slide(&mut app.world, id, panel::WHIRL_SPEED, 1.0);
        assert!((get(&app).speed - WHIRL_SPEED_MAX).abs() < 1e-2, "the right end is the fastest");
        panel::slide(&mut app.world, id, panel::WHIRL_SPEED, 0.5);
        assert!((get(&app).speed - WHIRL_SPEED_MAX * 0.5).abs() < 1e-2);
        // The direction and the switches, through the panel's own actions.
        assert_eq!(get(&app).spin, Spin::Counterclockwise);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::SPIN)]);
        assert_eq!(get(&app).spin, Spin::Clockwise);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::SPIN)]);
        assert_eq!(get(&app).spin, Spin::Counterclockwise);
        assert_eq!(get(&app).falloff, ForceFalloff::Fading);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::FALLOFF)]);
        assert_eq!(get(&app).falloff, ForceFalloff::Constant);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::FALLOFF)]);
        assert_eq!(get(&app).falloff, ForceFalloff::Fading);
        for (field, read) in [
            (panel::ON_BIBITES, (|tower: &WhirlpoolTower| tower.bibites) as fn(&WhirlpoolTower) -> bool),
            (panel::ON_PELLETS, |tower: &WhirlpoolTower| tower.pellets),
            (panel::ON_EGGS, |tower: &WhirlpoolTower| tower.eggs),
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
        app.whirlpool_towers_persist();
        assert!(!app.storage.whirlpool_towers_path("My save").exists());
        let first = app.world.place_whirlpool_at(Vec2::new(5.0, 6.0));
        let second = app.world.place_whirlpool_at(Vec2::new(-70.0, 80.0));
        panel::slide(&mut app.world, first, panel::FORCE_RANGE, 0.7);
        panel::slide(&mut app.world, first, panel::WHIRL_SPEED, 0.9);
        panel::flip_switch(&mut app.world, second, panel::SPIN);
        panel::flip_switch(&mut app.world, second, panel::FALLOFF);
        panel::flip_switch(&mut app.world, second, panel::ON_PELLETS);
        panel::flip_switch(&mut app.world, second, panel::ON_EGGS);
        panel::slide(&mut app.world, second, panel::WHIRL_SPEED, 0.1);
        let before = towers(&app);
        app.whirlpool_towers_persist();
        // Loaded as that save: the towers of the world are the stored ones (what was in the world before goes), nothing else of the world changes.
        let _ = app.world.place_whirlpool_at(Vec2::new(999.0, 999.0));
        app.whirlpool_towers_after_load("My save");
        let after = towers(&app);
        assert_eq!(after.len(), 2);
        for (a, b) in before.iter().zip(&after) {
            assert_eq!((a.base.pos, a.range, a.speed, a.spin, a.falloff), (b.base.pos, b.range, b.speed, b.spin, b.falloff));
            assert_eq!((a.bibites, a.pellets, a.eggs), (b.bibites, b.pellets, b.eggs));
        }
        assert!(after[1].spin == Spin::Clockwise && after[1].falloff == ForceFalloff::Constant && after[1].bibites && !after[1].pellets && !after[1].eggs, "a clockwise one on bibites only");
        // A save that has none stored: none.
        app.whirlpool_towers_after_load("Never saved");
        assert!(towers(&app).is_empty());
        // An autosave gets its own file; one with no towers gets none.
        app.whirlpool_towers_after_load("My save");
        app.whirlpool_towers_beside_autosave("2026-11-18_20-00");
        assert!(app.storage.whirlpool_towers_path("autosave_2026-11-18_20-00").exists());
        app.whirlpool_towers_after_load("Never saved");
        app.whirlpool_towers_beside_autosave("2026-11-18_20-05");
        assert!(!app.storage.whirlpool_towers_path("autosave_2026-11-18_20-05").exists());
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
        let id = app.world.place_whirlpool_at(zone_pos + Vec2::new(30.0, 0.0));
        app.select_placed_tool(id);
        // The tower's "follow zone" list is the colour killer's: "None" and the zones closest to it.
        let choices = panel::zone_choices(&app.world, app.world.tools.get(id).expect("the tower"));
        let index = choices.zones.iter().position(|zone| *zone == Some(zone_id)).expect("the zone 30u away is among the closest");
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::ToolZone), UiAction::DropdownPick(index as u16)]);
        assert_eq!((towers(&app)[0].base.attached_zone, towers(&app)[0].base.pos), (Some(zone_id), zone_pos));
        app.time_tags_key = Some("Followed".to_owned());
        app.whirlpool_towers_persist();
        app.whirlpool_towers_after_load("Followed");
        assert_eq!(towers(&app)[0].base.attached_zone, Some(zone_id));
        // The zone renamed: the tower stays where it was stored, free.
        app.world.zones.zones[0].config.0.name = "Another zone".to_owned();
        app.whirlpool_towers_after_load("Followed");
        assert_eq!(towers(&app)[0].base.attached_zone, None);
        assert!(app.world.set_tool_position(tower_id(&app), Vec2::new(1.0, 2.0)), "a tower that follows no zone can be moved");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn values_outside_the_ranges_an_unknown_spin_or_falloff_and_a_broken_file_are_kept_inside_and_said() {
        use bibites_core::whirlpool::{WHIRL_RANGE_MIN, WHIRL_SPEED_MAX};
        let (mut app, root) = app("broken");
        let path = app.storage.whirlpool_towers_path("Odd");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"towers":[{"x":1.0,"y":2.0,"range":-5.0,"speed":1.0e9,"spin":"sideways","falloff":"sideways"}]}"#).unwrap();
        app.whirlpool_towers_after_load("Odd");
        let loaded = towers(&app);
        assert_eq!(loaded.len(), 1);
        assert_eq!((loaded[0].range, loaded[0].speed, loaded[0].spin, loaded[0].falloff), (WHIRL_RANGE_MIN, WHIRL_SPEED_MAX, Spin::Counterclockwise, ForceFalloff::Fading));
        std::fs::write(&path, "not json").unwrap();
        app.whirlpool_towers_after_load("Odd");
        assert!(towers(&app).is_empty() && app.last_action.contains("not read"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(root);
    }
}
