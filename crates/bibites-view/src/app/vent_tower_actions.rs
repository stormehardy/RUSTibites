//! RUSTibites Improved: keeping the hydrothermal vent towers beside the save (`vent_towers/<save name>.json` in the state folder; never inside a save, which stays in the
//! game's format: the original game has no such tower and would not know what to do with one). The pellets a vent has shot are ordinary pellets and stay in the save; only
//! the vent's settings are kept here, so after a load a vent counts and recycles what it shoots from then on.

use bibites_core::math::Vec2;
use bibites_core::tools::Tool;
use bibites_core::vent::HydrothermalVent;
use bibites_io::storage::{VentTowerRecord, VentTowersFile, read_vent_towers, write_vent_towers};

use super::App;

/// The stored form of a vent (`zone`: the name of the zone it follows).
fn record_of(vent: &HydrothermalVent, zone: Option<String>) -> VentTowerRecord {
    VentTowerRecord {
        x: vent.base.pos.x,
        y: vent.base.pos.y,
        velocity: vent.velocity,
        range: vent.range,
        pellet_energy: vent.pellet_energy,
        rate: vent.rate,
        recycle: vent.recycle,
        cap: vent.cap,
        shot: vent.shot,
        zone,
    }
}

impl App {
    /// The vent towers of the world, as stored.
    fn vent_towers_file(&self) -> VentTowersFile {
        VentTowersFile {
            towers: self
                .world
                .tools
                .tools()
                .iter()
                .filter_map(|tool| {
                    let Tool::Vent(vent) = tool else {
                        return None;
                    };
                    let zone = vent.base.attached_zone.and_then(|id| self.world.zones.get(id)).map(|zone| zone.config.0.name.clone());
                    Some(record_of(vent, zone))
                })
                .collect(),
        }
    }

    /// Stores the vents under the name of the save the world was saved or loaded as (nothing is stored for a world that has no such name yet).
    pub(crate) fn vent_towers_persist(&mut self) {
        let Some(key) = self.time_tags_key.clone() else {
            return;
        };
        if let Err(error) = write_vent_towers(&self.storage, &key, &self.vent_towers_file()) {
            self.last_action = format!("vent towers not saved: {error}");
        }
    }

    /// The world was loaded from the save `stem`: the world's vent towers are the ones stored for it (none stored: none).
    pub(crate) fn vent_towers_after_load(&mut self, stem: &str) {
        self.remove_vent_towers();
        match read_vent_towers(&self.storage, stem) {
            Ok(file) => {
                for record in &file.towers {
                    self.add_vent_tower_from(record);
                }
            }
            Err(error) => self.last_action = format!("vent towers of {stem} not read: {error}"),
        }
    }

    /// An autosave is about to be written as `autosave_<stamp>.zip`: the vents are stored beside it under its name (the world's own key stays).
    pub(crate) fn vent_towers_beside_autosave(&mut self, stamp: &str) {
        let file = self.vent_towers_file();
        if file.towers.is_empty() {
            return;
        }
        if let Err(error) = write_vent_towers(&self.storage, &format!("autosave_{stamp}"), &file) {
            self.last_action = format!("vent towers of the autosave not saved: {error}");
        }
    }

    /// Takes every vent out of the world (a world that is loaded has none of the earlier world's). The pellets they shot stay.
    fn remove_vent_towers(&mut self) {
        loop {
            let found = (0..self.world.tools.len()).find(|&index| matches!(self.world.tools.tools().get(index), Some(Tool::Vent(_))));
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

    /// Puts a stored vent into the world (every value is kept inside its range, whatever the file says).
    fn add_vent_tower_from(&mut self, record: &VentTowerRecord) {
        let id = self.world.place_vent_at(Vec2::new(record.x, record.y));
        let Some(Tool::Vent(vent)) = self.world.tools.get_mut(id) else {
            return;
        };
        vent.set_velocity(record.velocity);
        vent.set_range(record.range);
        vent.set_pellet_energy(record.pellet_energy);
        vent.set_rate(record.rate);
        vent.set_cap(record.cap);
        vent.recycle = record.recycle;
        vent.shot = record.shot;
        // The zone it followed, found by its name (the first of that name); a zone that is gone leaves it where it was.
        let zone = record.zone.as_ref().and_then(|name| self.world.zones.zones.iter().find(|zone| &zone.config.0.name == name)).map(|zone| zone.id);
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
        let root = std::env::temp_dir().join(format!("rustibites_vent_towers_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp folder");
        app.storage = StoragePaths::new(&root);
        (app, root)
    }

    fn vents(app: &App) -> Vec<HydrothermalVent> {
        app.world.tools.tools().iter().filter_map(|tool| if let Tool::Vent(vent) = tool { Some(*vent) } else { None }).collect()
    }

    fn vent_id(app: &App) -> bibites_core::ids::ToolId {
        let index = app.world.tools.tools().iter().position(|tool| matches!(tool, Tool::Vent(_))).expect("a vent");
        bibites_core::tools::ToolStore::tool_id_at(index).expect("an id")
    }

    #[test]
    fn the_placer_puts_a_vent_where_the_cursor_is_with_the_worlds_pellet_size_selects_it_and_goes_back_to_inspecting() {
        let (mut app, root) = app("place");
        assert!(vents(&app).is_empty());
        app.tool = ToolKind::Vent;
        app.apply_tool(Vec2::new(120.0, -40.0));
        let placed = vents(&app);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].base.pos, Vec2::new(120.0, -40.0));
        assert_eq!(placed[0].pellet_energy, app.world.settings.scenario.pellet_energy.clamp(bibites_core::vent::VENT_PELLET_MIN, bibites_core::vent::VENT_PELLET_MAX));
        assert!(!placed[0].recycle, "recycling is off until it is switched on");
        assert_eq!(app.tool, ToolKind::Inspect, "a tower is placed once");
        assert!(app.selected_tool_id().is_some(), "its panel opens");
        assert!(app.last_action.contains("vent"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_panels_controls_change_the_vent_and_stay_inside_its_ranges() {
        let (mut app, root) = app("controls");
        let id = app.world.place_vent_at(Vec2::ZERO);
        app.select_placed_tool(id);
        let get = |app: &App| vents(app)[0];
        panel::slide(&mut app.world, id, panel::VELOCITY, 1.0);
        assert!((get(&app).velocity - bibites_core::vent::VENT_VELOCITY_MAX).abs() < 1e-2, "{}", get(&app).velocity);
        panel::slide(&mut app.world, id, panel::VELOCITY, 0.0);
        assert!((get(&app).velocity - bibites_core::vent::VENT_VELOCITY_MIN).abs() < 1e-2);
        panel::slide(&mut app.world, id, panel::PELLET_SIZE, 1.0);
        assert!((get(&app).pellet_energy - bibites_core::vent::VENT_PELLET_MAX).abs() < 1e-1);
        panel::slide(&mut app.world, id, panel::RANGE, 1.0);
        assert!((get(&app).range - bibites_core::vent::VENT_RANGE_MAX).abs() < 1e-1, "{}", get(&app).range);
        panel::slide(&mut app.world, id, panel::RANGE, 0.0);
        assert!((get(&app).range - bibites_core::vent::VENT_RANGE_MIN).abs() < 1e-2);
        panel::slide(&mut app.world, id, panel::RATE, 1.0);
        assert!((get(&app).rate - bibites_core::vent::VENT_RATE_MAX).abs() < 1e-3);
        panel::slide(&mut app.world, id, panel::CAP, 0.5);
        let cap = get(&app).cap;
        assert!(cap > bibites_core::vent::VENT_CAP_MIN && cap < bibites_core::vent::VENT_CAP_MAX, "{cap}");
        // The switch: recycling on and off.
        assert!(!get(&app).recycle);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::RECYCLE)]);
        assert!(get(&app).recycle);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::RECYCLE)]);
        assert!(!get(&app).recycle);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_vents_are_kept_beside_the_save_by_its_name_and_come_back_with_what_they_were_set_to() {
        let (mut app, root) = app("persist");
        // A world without vents writes no file.
        app.time_tags_key = Some("My save".to_owned());
        app.vent_towers_persist();
        assert!(!app.storage.vent_towers_path("My save").exists());
        let first = app.world.place_vent_at(Vec2::new(5.0, 6.0));
        let second = app.world.place_vent_at(Vec2::new(-70.0, 80.0));
        panel::slide(&mut app.world, first, panel::VELOCITY, 0.5);
        panel::slide(&mut app.world, first, panel::RANGE, 0.7);
        panel::slide(&mut app.world, first, panel::CAP, 0.25);
        panel::flip_switch(&mut app.world, second, panel::RECYCLE);
        panel::slide(&mut app.world, second, panel::RATE, 0.8);
        panel::slide(&mut app.world, second, panel::PELLET_SIZE, 0.3);
        if let Some(Tool::Vent(vent)) = app.world.tools.get_mut(second) {
            vent.shot = 321;
        }
        let before = vents(&app);
        app.vent_towers_persist();
        // Loaded as that save: the vents of the world are the stored ones (what was in the world before goes), nothing else of the world changes.
        let _ = app.world.place_vent_at(Vec2::new(999.0, 999.0));
        app.vent_towers_after_load("My save");
        let after = vents(&app);
        assert_eq!(after.len(), 2);
        for (a, b) in before.iter().zip(&after) {
            assert_eq!((a.base.pos, a.recycle, a.shot), (b.base.pos, b.recycle, b.shot));
            assert_eq!((a.velocity, a.range, a.pellet_energy, a.rate, a.cap), (b.velocity, b.range, b.pellet_energy, b.rate, b.cap));
        }
        // A save that has none stored: none.
        app.vent_towers_after_load("Never saved");
        assert!(vents(&app).is_empty());
        // An autosave gets its own file; one with no vents gets none.
        app.vent_towers_after_load("My save");
        app.vent_towers_beside_autosave("2026-11-17_20-00");
        assert!(app.storage.vent_towers_path("autosave_2026-11-17_20-00").exists());
        app.vent_towers_after_load("Never saved");
        app.vent_towers_beside_autosave("2026-11-17_20-05");
        assert!(!app.storage.vent_towers_path("autosave_2026-11-17_20-05").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_vent_follows_a_zone_keeps_the_zone_by_its_name_and_stays_put_when_the_zone_is_gone() {
        let (mut app, root) = app("follow");
        assert!(!app.world.zones.zones.is_empty(), "the starter world has zones");
        let (zone_id, zone_pos) = {
            let zone = &app.world.zones.zones[0];
            (zone.id, zone.pos)
        };
        let id = app.world.place_vent_at(zone_pos + Vec2::new(30.0, 0.0));
        app.select_placed_tool(id);
        // The vent's "follow zone" list is the colour killer's: "None" and the zones closest to it.
        let choices = panel::zone_choices(&app.world, app.world.tools.get(id).expect("the vent"));
        let index = choices.zones.iter().position(|zone| *zone == Some(zone_id)).expect("the zone 30u away is among the closest");
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::ToolZone), UiAction::DropdownPick(index as u16)]);
        assert_eq!((vents(&app)[0].base.attached_zone, vents(&app)[0].base.pos), (Some(zone_id), zone_pos));
        app.time_tags_key = Some("Followed".to_owned());
        app.vent_towers_persist();
        app.vent_towers_after_load("Followed");
        assert_eq!(vents(&app)[0].base.attached_zone, Some(zone_id));
        // The zone renamed: the vent stays where it was stored, free.
        app.world.zones.zones[0].config.0.name = "Another zone".to_owned();
        app.vent_towers_after_load("Followed");
        assert_eq!(vents(&app)[0].base.attached_zone, None);
        assert!(app.world.set_tool_position(vent_id(&app), Vec2::new(1.0, 2.0)), "a vent that follows no zone can be moved");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn values_outside_the_ranges_and_a_broken_file_are_kept_inside_and_said() {
        let (mut app, root) = app("broken");
        let path = app.storage.vent_towers_path("Odd");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"towers":[{"x":1.0,"y":2.0,"velocity":99999.0,"range":1.0e9,"pellet_energy":0.0,"rate":-3.0,"cap":1.0e12}]}"#).unwrap();
        app.vent_towers_after_load("Odd");
        let loaded = vents(&app);
        assert_eq!(loaded.len(), 1);
        use bibites_core::vent::{VENT_CAP_MAX, VENT_PELLET_MIN, VENT_RANGE_MAX, VENT_RATE_MIN, VENT_VELOCITY_MAX};
        assert_eq!((loaded[0].velocity, loaded[0].pellet_energy, loaded[0].rate, loaded[0].cap, loaded[0].range), (VENT_VELOCITY_MAX, VENT_PELLET_MIN, VENT_RATE_MIN, VENT_CAP_MAX, VENT_RANGE_MAX));
        std::fs::write(&path, "not json").unwrap();
        app.vent_towers_after_load("Odd");
        assert!(vents(&app).is_empty() && app.last_action.contains("not read"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(root);
    }
}
