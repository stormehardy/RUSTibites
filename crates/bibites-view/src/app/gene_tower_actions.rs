//! RUSTibites Improved: keeping the gene selector towers beside the save (`gene_towers/<save name>.json` in the state folder, by the genes' names; never inside a
//! save, which stays in the game's format: the original game has no such tower and would not know what to do with one).

use bibites_core::gene_selector::GeneSelector;
use bibites_core::genome::GENE_DEFS;
use bibites_core::math::Vec2;
use bibites_core::tools::Tool;
use bibites_io::storage::{GeneTowerRecord, GeneTowersFile, read_gene_towers, write_gene_towers};

use super::App;

/// The stored form of a tower (`zone`: the name of the zone it follows).
fn record_of(selector: &GeneSelector, zone: Option<String>) -> GeneTowerRecord {
    GeneTowerRecord {
        x: selector.base.pos.x,
        y: selector.base.pos.y,
        gene: selector.def().name.to_owned(),
        min: selector.min,
        max: selector.max,
        kill_inside: selector.kill_inside,
        any_or_most: selector.any_or_most,
        kills_per_cycle: selector.kills_per_cycle,
        radius: selector.radius,
        period: selector.period,
        progress: selector.progress,
        killed: selector.killed,
        zone,
    }
}

impl App {
    /// The gene selector towers of the world, as stored.
    fn gene_towers_file(&self) -> GeneTowersFile {
        GeneTowersFile {
            towers: self
                .world
                .tools
                .tools()
                .iter()
                .filter_map(|tool| {
                    let Tool::Gene(selector) = tool else {
                        return None;
                    };
                    let zone = selector.base.attached_zone.and_then(|id| self.world.zones.get(id)).map(|zone| zone.config.0.name.clone());
                    Some(record_of(selector, zone))
                })
                .collect(),
        }
    }

    /// Stores the towers under the name of the save the world was saved or loaded as (nothing is stored for a world that has no such name yet).
    pub(crate) fn gene_towers_persist(&mut self) {
        let Some(key) = self.time_tags_key.clone() else {
            return;
        };
        if let Err(error) = write_gene_towers(&self.storage, &key, &self.gene_towers_file()) {
            self.last_action = format!("gene towers not saved: {error}");
        }
    }

    /// The world was loaded from the save `stem`: the world's gene selector towers are the ones stored for it (none stored: none).
    pub(crate) fn gene_towers_after_load(&mut self, stem: &str) {
        self.remove_gene_towers();
        match read_gene_towers(&self.storage, stem) {
            Ok(file) => {
                for record in &file.towers {
                    self.add_gene_tower_from(record);
                }
            }
            Err(error) => self.last_action = format!("gene towers of {stem} not read: {error}"),
        }
    }

    /// An autosave is about to be written as `autosave_<stamp>.zip`: the towers are stored beside it under its name (the world's own key stays).
    pub(crate) fn gene_towers_beside_autosave(&mut self, stamp: &str) {
        let file = self.gene_towers_file();
        if file.towers.is_empty() {
            return;
        }
        if let Err(error) = write_gene_towers(&self.storage, &format!("autosave_{stamp}"), &file) {
            self.last_action = format!("gene towers of the autosave not saved: {error}");
        }
    }

    /// Takes every gene selector out of the world (a world that is loaded has none of the earlier world's).
    fn remove_gene_towers(&mut self) {
        loop {
            let found = (0..self.world.tools.len()).find(|&index| matches!(self.world.tools.tools().get(index), Some(Tool::Gene(_))));
            let Some(index) = found else {
                return;
            };
            let Some(id) = bibites_core::tools::ToolStore::tool_id_at(index) else {
                return;
            };
            let _ = self.world.tools.remove_tool(id);
        }
    }

    /// Puts a stored tower into the world (a gene the game does not have is the default gene).
    fn add_gene_tower_from(&mut self, record: &GeneTowerRecord) {
        let id = self.world.place_gene_selector_at(Vec2::new(record.x, record.y));
        let Some(Tool::Gene(selector)) = self.world.tools.get_mut(id) else {
            return;
        };
        if let Some(index) = GENE_DEFS.iter().position(|def| def.name == record.gene) {
            selector.set_gene(index as u8);
        }
        selector.set_range(record.min, record.max);
        selector.kill_inside = record.kill_inside;
        selector.any_or_most = record.any_or_most;
        selector.set_kills_per_cycle(record.kills_per_cycle);
        selector.radius = record.radius.max(1.0);
        selector.period = record.period.max(0.1);
        selector.progress = record.progress.max(0.0);
        selector.killed = record.killed;
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
    use bibites_core::genome::Gene;
    use bibites_io::storage::StoragePaths;

    /// An app with a state folder of its own (the tests must not touch the player's).
    fn app(name: &str) -> (App, std::path::PathBuf) {
        let loaded = LoadedWorld::load(None, 77, 6, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        let root = std::env::temp_dir().join(format!("rustibites_gene_towers_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp folder");
        app.storage = StoragePaths::new(&root);
        (app, root)
    }

    fn towers(app: &App) -> Vec<GeneSelector> {
        app.world.tools.tools().iter().filter_map(|tool| if let Tool::Gene(selector) = tool { Some(*selector) } else { None }).collect()
    }

    #[test]
    fn the_placer_puts_a_gene_selector_where_the_cursor_is_selects_it_and_goes_back_to_inspecting() {
        let (mut app, root) = app("place");
        assert!(towers(&app).is_empty());
        app.tool = ToolKind::GeneSelector;
        app.apply_tool(Vec2::new(120.0, -40.0));
        let placed = towers(&app);
        assert_eq!(placed.len(), 1);
        assert_eq!((placed[0].base.pos, placed[0].gene), (Vec2::new(120.0, -40.0), Gene::Diet as u8));
        assert_eq!(app.tool, ToolKind::Inspect, "a tower is placed once");
        assert!(app.selected_tool_id().is_some(), "its panel opens");
        assert!(app.last_action.contains("gene selector"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_panels_controls_change_the_tower_the_range_stays_in_order_and_another_gene_resets_it() {
        let (mut app, root) = app("controls");
        let id = app.world.place_gene_selector_at(Vec2::ZERO);
        app.select_placed_tool(id);
        let get = |app: &App| towers(app)[0];
        let def = *get(&app).def();
        panel::slide(&mut app.world, id, panel::GENE_MAX, 0.5);
        assert!((get(&app).max - (def.min + 0.5 * (def.max - def.min))).abs() < 1e-3, "{}", get(&app).max);
        // A lowest value above the highest drags the highest along.
        panel::slide(&mut app.world, id, panel::GENE_MIN, 0.8);
        assert!(get(&app).min <= get(&app).max && (get(&app).min - (def.min + 0.8 * (def.max - def.min))).abs() < 1e-3);
        panel::slide(&mut app.world, id, panel::KILLS, 1.0);
        assert_eq!(get(&app).kills_per_cycle, bibites_core::gene_selector::GENE_SELECTOR_MAX_KILLS);
        panel::slide(&mut app.world, id, panel::KILLS, 0.0);
        assert_eq!(get(&app).kills_per_cycle, 1);
        panel::slide(&mut app.world, id, panel::PERIOD, 0.0);
        assert!((get(&app).period - 5.0).abs() < 1e-3);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::KILL_INSIDE), UiAction::ToolSwitch(panel::ANY_OR_MOST)]);
        assert!(get(&app).kill_inside && get(&app).any_or_most);
        app.apply_ui_actions(&[UiAction::ToolSwitch(panel::KILL_INSIDE)]);
        assert!(!get(&app).kill_inside);
        // The gene dropdown lists the 35 genes, the current one marked, and a pick sets the gene with its whole range.
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::ToolGene)]);
        let open = app.dropdown_open.as_ref().expect("the list opens");
        assert_eq!(open.choices.len(), bibites_core::genome::N_GENES);
        assert_eq!(open.selected, Gene::Diet as usize);
        assert_eq!(open.choices[Gene::SizeRatio as usize].label, bibites_core::genome::GENE_DEFS[Gene::SizeRatio as usize].display_name);
        app.apply_ui_actions(&[UiAction::DropdownPick(Gene::SizeRatio as u16)]);
        let size = bibites_core::genome::GENE_DEFS[Gene::SizeRatio as usize];
        assert_eq!((get(&app).gene, get(&app).min, get(&app).max), (Gene::SizeRatio as u8, size.min, size.max));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_towers_are_kept_beside_the_save_by_its_name_and_come_back_with_what_they_were_set_to() {
        let (mut app, root) = app("persist");
        // A world without towers writes no file.
        app.time_tags_key = Some("My save".to_owned());
        app.gene_towers_persist();
        assert!(!app.storage.gene_towers_path("My save").exists());
        let first = app.world.place_gene_selector_at(Vec2::new(5.0, 6.0));
        let second = app.world.place_gene_selector_at(Vec2::new(-70.0, 80.0));
        panel::set_gene(&mut app.world, first, Gene::SizeRatio as usize);
        panel::slide(&mut app.world, first, panel::GENE_MAX, 0.4);
        panel::flip_switch(&mut app.world, second, panel::KILL_INSIDE);
        panel::slide(&mut app.world, second, panel::KILLS, 0.2);
        if let Some(Tool::Gene(selector)) = app.world.tools.get_mut(second) {
            selector.progress = 12.5;
            selector.killed = 3;
        }
        let before = towers(&app);
        app.gene_towers_persist();
        let stored = std::fs::read_to_string(app.storage.gene_towers_path("My save")).expect("the file");
        assert!(stored.contains("\"SizeRatio\"") && stored.contains("\"Diet\""), "the genes are named: {stored}");
        // Loaded as that save: the towers of the world are the stored ones (what was in the world before goes), nothing else of the world changes.
        let _ = app.world.place_gene_selector_at(Vec2::new(999.0, 999.0));
        app.gene_towers_after_load("My save");
        assert_eq!(towers(&app).len(), 2);
        let after = towers(&app);
        for (a, b) in before.iter().zip(&after) {
            assert_eq!((a.base.pos, a.gene, a.min, a.max, a.kill_inside, a.any_or_most), (b.base.pos, b.gene, b.min, b.max, b.kill_inside, b.any_or_most));
            assert_eq!((a.kills_per_cycle, a.radius, a.period, a.progress, a.killed), (b.kills_per_cycle, b.radius, b.period, b.progress, b.killed));
        }
        // A save that has none stored: none.
        app.gene_towers_after_load("Never saved");
        assert!(towers(&app).is_empty());
        // An autosave gets its own file; one with no towers gets none.
        app.gene_towers_after_load("My save");
        app.gene_towers_beside_autosave("2026-11-17_20-00");
        assert!(app.storage.gene_towers_path("autosave_2026-11-17_20-00").exists());
        app.gene_towers_after_load("Never saved");
        app.gene_towers_beside_autosave("2026-11-17_20-05");
        assert!(!app.storage.gene_towers_path("autosave_2026-11-17_20-05").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_gene_selector_follows_a_zone_like_the_colour_killer_keeps_the_zone_by_its_name_and_stays_put_when_the_zone_is_gone() {
        let (mut app, root) = app("follow");
        assert!(!app.world.zones.zones.is_empty(), "the starter world has zones");
        let (zone_id, zone_pos, zone_name) = {
            let zone = &app.world.zones.zones[0];
            (zone.id, zone.pos, zone.config.0.name.clone())
        };
        let id = app.world.place_gene_selector_at(zone_pos + Vec2::new(30.0, 0.0));
        app.select_placed_tool(id);
        // The "follow zone" list is the colour killer's: "None" and the closest zones, each with its distance.
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::ToolZone)]);
        let open = app.dropdown_open.as_ref().expect("the list opens");
        assert_eq!(open.choices[0].label, "None");
        assert!(open.choices.len() >= 2 && open.choices.len() <= 6, "{} choices", open.choices.len());
        assert!(open.choices[1..].iter().all(|choice| choice.label.ends_with("u)")), "each zone with its distance");
        let choices = panel::zone_choices(&app.world, app.world.tools.get(id).expect("the tower"));
        let index = choices.zones.iter().position(|zone| *zone == Some(zone_id)).expect("the zone 30u away is among the closest");
        app.apply_ui_actions(&[UiAction::DropdownPick(index as u16)]);
        let tower = |app: &App| towers(app)[0];
        assert_eq!((tower(&app).base.attached_zone, tower(&app).base.pos), (Some(zone_id), zone_pos), "it snaps to the zone's centre at once");
        // The zone moves: the tower goes with it at the next tick's tool phase, and it cannot be dragged away.
        app.world.zones.zones[0].pos = zone_pos + Vec2::new(200.0, -50.0);
        app.world.step_tools(0.0);
        assert_eq!(tower(&app).base.pos, zone_pos + Vec2::new(200.0, -50.0));
        assert!(!app.world.set_tool_position(id, Vec2::ZERO), "a tower that follows a zone is not moved by hand");
        // Kept beside the save by the zone's name, and attached again when the save is loaded.
        app.time_tags_key = Some("Followed".to_owned());
        app.gene_towers_persist();
        let stored = std::fs::read_to_string(app.storage.gene_towers_path("Followed")).expect("the file");
        assert!(stored.contains(&format!("\"zone\": \"{zone_name}\"")) || stored.contains(&format!("\"zone\":\"{zone_name}\"")), "{stored}");
        app.gene_towers_after_load("Followed");
        assert_eq!(tower(&app).base.attached_zone, Some(zone_id));
        // "None" lets it go (it stays where it is and can be moved again).
        app.select_placed_tool(gene_id(&app));
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::ToolZone), UiAction::DropdownPick(0)]);
        assert_eq!(tower(&app).base.attached_zone, None);
        assert!(app.world.set_tool_position(gene_id(&app), Vec2::new(1.0, 2.0)));
        // A zone that is not there any more (here: renamed): the tower stays where it was stored, free.
        app.gene_towers_after_load("Followed");
        app.world.zones.zones[0].config.0.name = "Another zone".to_owned();
        app.gene_towers_after_load("Followed");
        assert_eq!(tower(&app).base.attached_zone, None);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The id of the first gene selector of the world.
    fn gene_id(app: &App) -> bibites_core::ids::ToolId {
        let index = app.world.tools.tools().iter().position(|tool| matches!(tool, Tool::Gene(_))).expect("a gene selector");
        bibites_core::tools::ToolStore::tool_id_at(index).expect("an id")
    }

    #[test]
    fn a_gene_the_game_does_not_have_is_the_default_gene_and_a_broken_file_is_said_and_not_a_panic() {
        let (mut app, root) = app("broken");
        let path = app.storage.gene_towers_path("Odd");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"towers":[{"x":1.0,"y":2.0,"gene":"NoSuchGene","min":-5.0,"max":99999.0,"radius":0.0,"period":0.0}]}"#).unwrap();
        app.gene_towers_after_load("Odd");
        let loaded = towers(&app);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].gene, Gene::Diet as u8, "an unknown gene is the default");
        let def = *loaded[0].def();
        assert_eq!((loaded[0].min, loaded[0].max), (def.min, def.max), "the range is kept inside the gene's own");
        assert!(loaded[0].radius >= 1.0 && loaded[0].period >= 0.1 && loaded[0].kills_per_cycle == 1);
        std::fs::write(&path, "not json").unwrap();
        app.gene_towers_after_load("Odd");
        assert!(towers(&app).is_empty() && app.last_action.contains("not read"), "{}", app.last_action);
        let _ = std::fs::remove_dir_all(root);
    }
}
