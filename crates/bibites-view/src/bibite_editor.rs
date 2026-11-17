//! The fields of a default-bibite item (`BibiteSettingsHandle`, the rows of `DefaultBibitesPanel`
//! on the Bibite tab): which controls an item shows, their labels, ranges and choices
//! (`BibiteSettings.cs`), and how a slider position or a dropdown click changes the spawn row.

use std::path::Path;

use bibites_core::settings::{
    BibiteSpawn, GrowthAtSpawn, RandomizeGenes, SpawnType, Tagging, ZoneSettings,
};

use crate::dyn_panel::SliderMode;
use crate::zone_editor::SliderInfo;

/// Tagging dropdown.
pub const TAGGING: u8 = 0;
/// Spawn zone dropdown.
pub const SPAWN_ZONE: u8 = 1;
/// Spawn priority slider.
pub const PRIORITY: u8 = 2;
/// Growth at spawn dropdown.
pub const GROWTH: u8 = 3;
/// The custom tag (only with custom tagging).
pub const CUSTOM_TAG: u8 = 4;
/// Spawn logic dropdown.
pub const SPAWN_TYPE: u8 = 5;
/// Target count slider.
pub const MINIMUM: u8 = 6;
/// Mutate genes dropdown.
pub const MUTATIONS: u8 = 7;
/// How many were spawned (a running simulation, "One Time" rows only).
pub const SPAWNED: u8 = 8;

/// How a field is shown and edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A slider.
    Slider,
    /// A dropdown (clicking advances to the next choice).
    Choice,
    /// A value that is only shown (the custom tag, the spawned counter).
    Text,
}

/// The controls shown for `row`, in the prefab's order: the left column (tagging, zone, priority,
/// growth) then the right one (custom tag, logic, target count, mutations)
/// (`BibiteSettingsHandle.TaggingChanged` / `OnChangeSpawnType`).
#[must_use]
pub fn fields_of(row: &BibiteSpawn, in_sim: bool) -> Vec<u8> {
    let mut fields = vec![TAGGING, SPAWN_ZONE, PRIORITY, GROWTH];
    if row.tagging == Tagging::CustomTagging {
        fields.push(CUSTOM_TAG);
    }
    fields.extend([SPAWN_TYPE, MINIMUM, MUTATIONS]);
    if in_sim && row.spawn_type == SpawnType::OneTime {
        fields.push(SPAWNED);
    }
    fields
}

/// Kind of a field.
#[must_use]
pub fn kind_of(field: u8) -> Kind {
    match field {
        PRIORITY | MINIMUM => Kind::Slider,
        CUSTOM_TAG | SPAWNED => Kind::Text,
        _ => Kind::Choice,
    }
}

/// Label of a field (the `Setting.Name`s of `BibiteSettings`).
#[must_use]
pub fn name_of(field: u8) -> &'static str {
    match field {
        TAGGING => "Tagging",
        SPAWN_ZONE => "Spawn Zone",
        PRIORITY => "Priority",
        GROWTH => "Growth At Spawn",
        CUSTOM_TAG => "Tag",
        SPAWN_TYPE => "Spawn Logic",
        MINIMUM => "Target Count",
        MUTATIONS => "Mutate Genes",
        SPAWNED => "Spawned",
        _ => "",
    }
}

/// The C# field of the setting behind `field`, whose name and help text its row's tooltip shows
/// (growth at spawn and the spawn logic have none).
#[must_use]
pub fn help_key(field: u8) -> Option<&'static str> {
    Some(match field {
        TAGGING => "tagging",
        SPAWN_ZONE => "spawnZone",
        PRIORITY => "spawnPriority",
        CUSTOM_TAG => "customTag",
        MINIMUM => "minimumNumber",
        MUTATIONS => "randomizeGenes",
        SPAWNED => "totalSpawned",
        _ => return None,
    })
}

/// Slider description of `field`, `None` for the other kinds
/// (`LogFloatSettingSlider(spawnPriority, 10, ..., 4)`, `LogIntSettingSlider(minimumNumber, 10,
/// ..., 2.1)`).
#[must_use]
pub fn slider_info(field: u8) -> Option<SliderInfo> {
    Some(match field {
        PRIORITY => SliderInfo {
            min: 0.01,
            max: 100.0,
            default: 1.0,
            mode: SliderMode::Log {
                base: 10.0,
                span: Some(4.0),
            },
            units: "",
            precision: 2,
            si: false,
            factor: 1.0,
        },
        MINIMUM => SliderInfo {
            min: 0.0,
            max: 500.0,
            default: 10.0,
            mode: SliderMode::Log {
                base: 10.0,
                span: Some(2.1),
            },
            units: "",
            precision: 0,
            si: false,
            factor: 1.0,
        },
        _ => return None,
    })
}

/// Current numeric value of a slider field.
#[must_use]
pub fn value_of(row: &BibiteSpawn, field: u8) -> f32 {
    match field {
        PRIORITY => row.spawn_priority,
        MINIMUM => row.minimum as f32,
        _ => 0.0,
    }
}

/// Writes a slider value (clamped to the field's range; the target count is a whole number).
pub fn set_value(row: &mut BibiteSpawn, field: u8, value: f32) {
    let Some(info) = slider_info(field) else {
        return;
    };
    set_value_unclamped(row, field, value.clamp(info.min, info.max));
}

/// Writes a slider value as it is, past the slider's ends too (a typed value: the spawn settings
/// `canGoOutOfBounds`).
pub fn set_value_unclamped(row: &mut BibiteSpawn, field: u8, value: f32) {
    match field {
        PRIORITY => row.spawn_priority = value,
        MINIMUM => row.minimum = value.round() as i32,
        _ => {}
    }
}

/// Puts a slider field back to its default (the reset button).
pub fn reset_value(row: &mut BibiteSpawn, field: u8) {
    if let Some(info) = slider_info(field) {
        set_value(row, field, info.default);
    }
}

/// The choices of a dropdown field, in the game's order (`SettingChoices` lists).
fn choice_count(field: u8, zones: &[ZoneSettings]) -> usize {
    match field {
        TAGGING | GROWTH | MUTATIONS => 4,
        SPAWN_TYPE => 2,
        // "All", then every zone.
        SPAWN_ZONE => 1 + zones.len(),
        _ => 0,
    }
}

/// Index of the current choice of a dropdown field in the game's list.
fn choice_index(row: &BibiteSpawn, field: u8, zones: &[ZoneSettings]) -> usize {
    match field {
        TAGGING => match row.tagging {
            Tagging::NoTagging => 0,
            Tagging::SpeciesTagging => 1,
            Tagging::CustomTagging => 2,
            Tagging::RandomTagging => 3,
        },
        GROWTH => match row.growth_at_spawn {
            GrowthAtSpawn::Egg => 0,
            GrowthAtSpawn::Baby => 1,
            GrowthAtSpawn::Adult => 2,
            GrowthAtSpawn::Elder => 3,
        },
        SPAWN_TYPE => match row.spawn_type {
            SpawnType::Continuous => 0,
            SpawnType::OneTime => 1,
        },
        MUTATIONS => match row.randomize_genes {
            RandomizeGenes::No => 0,
            RandomizeGenes::NormalMutations => 1,
            RandomizeGenes::Color => 2,
            RandomizeGenes::AllGenes => 3,
        },
        SPAWN_ZONE => row
            .spawn_zone
            .and_then(|id| zones.iter().position(|zone| zone.zone_id == Some(id)))
            .map_or(0, |position| position + 1),
        _ => 0,
    }
}

/// The label of a dropdown field's current choice.
#[must_use]
pub fn choice_label(row: &BibiteSpawn, field: u8, zones: &[ZoneSettings]) -> String {
    let index = choice_index(row, field, zones);
    match field {
        TAGGING => ["None", "Species", "Custom", "Random Tag"][index].to_owned(),
        GROWTH => ["Egg", "Baby", "Adult", "Elder"][index].to_owned(),
        SPAWN_TYPE => ["Continuous", "One Time"][index].to_owned(),
        MUTATIONS => ["No", "Normal", "Random Color", "All Genes"][index].to_owned(),
        SPAWN_ZONE => {
            if index == 0 {
                "All".to_owned()
            } else {
                zones[index - 1].name.clone()
            }
        }
        _ => String::new(),
    }
}

/// The options of a dropdown field in the game's order, with their tooltips
/// (`BibiteSettings.*Choices`; "All" and every zone for the spawn zone).
#[must_use]
pub fn choices(field: u8, zones: &[ZoneSettings]) -> Vec<crate::dropdown::Choice> {
    use crate::dropdown::Choice;
    // `ChoiceSettingDropdown.OnItemHovered`: "Setting name: Choice" over the choice's text.
    let list = |setting: &str, entries: &[(&str, &'static str)]| -> Vec<Choice> {
        entries
            .iter()
            .map(|&(label, tip)| Choice::of_setting(setting, label, tip))
            .collect()
    };
    match field {
        TAGGING => list(
            "Tagging",
            &[
                (
                    "None",
                    "Bibites spawned from this template will not be tagged",
                ),
                (
                    "Species",
                    "Bibites spawned from this template will be tagged with the species name.",
                ),
                ("Custom", "Define your own tag"),
                (
                    "Random Tag",
                    "Bibites spawned from this template will receive a random alphanumeric tag",
                ),
            ],
        ),
        GROWTH => list(
            "Growth At Spawn",
            &[
                ("Egg", "Bibites will spawn as eggs."),
                (
                    "Baby",
                    "Bibites will spawn as they just had came out of their egg.",
                ),
                (
                    "Adult",
                    "Bibites will spawn as adults, already having the capacity to lay eggs.",
                ),
                (
                    "Elder",
                    "Bibites will spawn as elders, twice as developed as adults, which is not necessarily a stage that a bibite would get to on its own.",
                ),
            ],
        ),
        // `BibiteSettings.spawnType` is never given its `spawnTypeChoices`: the dropdown lists the
        // enum's names with spaces added ("One Time") and its items have no tooltips.
        SPAWN_TYPE => vec![Choice::plain("Continuous"), Choice::plain("One Time")],
        MUTATIONS => list(
            "Mutate Genes",
            &[
                ("No", "Bibites will spawn as exact copy of their template"),
                (
                    "Normal",
                    "Bibites will spawn with mutations present according to their mutation genes as if they were birthed by an actual bibite",
                ),
                (
                    "Random Color",
                    "Each bibite spawned will have a random color",
                ),
                (
                    "All Genes",
                    "Will randomize all genes of each bibite when spawned, only the brain will be preserved.",
                ),
            ],
        ),
        SPAWN_ZONE => std::iter::once(Choice::plain("All"))
            .chain(zones.iter().map(|zone| Choice::plain(zone.name.clone())))
            .collect(),
        _ => Vec::new(),
    }
}

/// Index of the row's current choice of a dropdown field in [`choices`].
#[must_use]
pub fn current_choice(row: &BibiteSpawn, field: u8, zones: &[ZoneSettings]) -> usize {
    choice_index(row, field, zones)
}

/// Advances a dropdown field to its next choice (a click on the dropdown).
pub fn cycle(row: &mut BibiteSpawn, field: u8, zones: &[ZoneSettings]) {
    let count = choice_count(field, zones);
    if count == 0 {
        return;
    }
    let next = (choice_index(row, field, zones) + 1) % count;
    set_choice(row, field, next, zones);
}

/// Sets a dropdown field to option `next` of [`choices`] (a pick in its list).
pub fn set_choice(row: &mut BibiteSpawn, field: u8, next: usize, zones: &[ZoneSettings]) {
    if next >= choice_count(field, zones) {
        return;
    }
    match field {
        TAGGING => {
            row.tagging = [
                Tagging::NoTagging,
                Tagging::SpeciesTagging,
                Tagging::CustomTagging,
                Tagging::RandomTagging,
            ][next];
        }
        GROWTH => {
            row.growth_at_spawn = [
                GrowthAtSpawn::Egg,
                GrowthAtSpawn::Baby,
                GrowthAtSpawn::Adult,
                GrowthAtSpawn::Elder,
            ][next];
        }
        SPAWN_TYPE => {
            row.spawn_type = [SpawnType::Continuous, SpawnType::OneTime][next];
            // `OnChangeSpawnType`: a "One Time" row of a running simulation starts counting afresh.
            if row.spawn_type == SpawnType::OneTime {
                row.total_spawned = 0;
            }
        }
        MUTATIONS => {
            row.randomize_genes = [
                RandomizeGenes::No,
                RandomizeGenes::NormalMutations,
                RandomizeGenes::Color,
                RandomizeGenes::AllGenes,
            ][next];
        }
        SPAWN_ZONE => {
            row.spawn_zone = if next == 0 {
                None
            } else {
                zones[next - 1].zone_id
            };
        }
        _ => {}
    }
}

/// The text of a [`Kind::Text`] field.
#[must_use]
pub fn text_of(row: &BibiteSpawn, field: u8) -> String {
    match field {
        CUSTOM_TAG => row.tag.clone(),
        SPAWNED => row.total_spawned.to_string(),
        _ => String::new(),
    }
}

/// A new default-bibite row for the template file `file_name` (`DefaultBibitesPanel.AddBibite`:
/// the game's defaults with the template's path).
#[must_use]
pub fn new_row(file_name: &str) -> BibiteSpawn {
    BibiteSpawn {
        path: file_name.to_owned(),
        ..BibiteSpawn::game_defaults()
    }
}

/// The name an item shows: the template's display name when the library knows the file,
/// else the file name without its extension (`BibiteSettingsHandle.bibiteName`).
#[must_use]
pub fn display_name(path: &str, library: &[(String, String)]) -> String {
    let file = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);
    if let Some((_, name)) = library.iter().find(|(known, _)| known == file) {
        return name.clone();
    }
    Path::new(file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str, id: u32) -> ZoneSettings {
        let mut zone = ZoneSettings::game_defaults();
        zone.name = name.to_owned();
        zone.zone_id = Some(id);
        zone
    }

    fn option_labels(field: u8, zones: &[ZoneSettings]) -> Vec<String> {
        choices(field, zones)
            .into_iter()
            .map(|choice| choice.label)
            .collect()
    }

    #[test]
    fn the_dropdowns_offer_the_games_options_with_their_tooltips() {
        assert_eq!(
            option_labels(TAGGING, &[]),
            ["None", "Species", "Custom", "Random Tag"]
        );
        assert_eq!(
            option_labels(GROWTH, &[]),
            ["Egg", "Baby", "Adult", "Elder"]
        );
        assert_eq!(option_labels(SPAWN_TYPE, &[]), ["Continuous", "One Time"]);
        assert_eq!(
            option_labels(MUTATIONS, &[]),
            ["No", "Normal", "Random Color", "All Genes"]
        );
        // The spawn zone lists "All", then every zone by name.
        let zones = [zone("Alpha", 1), zone("Beta", 2)];
        assert_eq!(option_labels(SPAWN_ZONE, &zones), ["All", "Alpha", "Beta"]);
        for field in [TAGGING, GROWTH, MUTATIONS] {
            assert!(
                choices(field, &[])
                    .iter()
                    .all(|choice| !choice.tip.is_empty()),
                "field {field}: every option has its tooltip"
            );
        }
        // `spawnType` has no `SettingChoices`: its two items have no tooltip.
        assert!(
            choices(SPAWN_TYPE, &[])
                .iter()
                .all(|choice| choice.tip.is_empty() && choice.title.is_empty())
        );
        assert!(
            choices(PRIORITY, &zones).is_empty(),
            "a slider has no options"
        );
    }

    #[test]
    fn an_item_shows_the_setting_name_and_the_choice_over_the_choices_text() {
        // `ChoiceSettingDropdown.OnItemHovered`: `setting.Name + ": " + choice.label`, then `tooltipText`.
        let titles =
            |field| -> Vec<&'static str> { choices(field, &[]).iter().map(|c| c.title).collect() };
        assert_eq!(
            titles(TAGGING),
            [
                "Tagging: None",
                "Tagging: Species",
                "Tagging: Custom",
                "Tagging: Random Tag"
            ]
        );
        assert_eq!(
            titles(GROWTH),
            [
                "Growth At Spawn: Egg",
                "Growth At Spawn: Baby",
                "Growth At Spawn: Adult",
                "Growth At Spawn: Elder"
            ]
        );
        assert_eq!(
            titles(MUTATIONS),
            [
                "Mutate Genes: No",
                "Mutate Genes: Normal",
                "Mutate Genes: Random Color",
                "Mutate Genes: All Genes"
            ]
        );
        let growth = choices(GROWTH, &[]);
        assert_eq!(growth[0].tip, "Bibites will spawn as eggs.");
        // The spawn zone dropdown is a `TargetZoneDropdown`: no item tooltips.
        assert!(
            choices(SPAWN_ZONE, &[zone("Alpha", 1)])
                .iter()
                .all(|choice| choice.title.is_empty())
        );
    }

    #[test]
    fn picking_an_option_sets_the_field_and_ignores_an_index_out_of_range() {
        let zones = [zone("Alpha", 1), zone("Beta", 2)];
        let mut row = new_row("x.bb8template");
        for field in [TAGGING, GROWTH, SPAWN_TYPE, MUTATIONS, SPAWN_ZONE] {
            for index in 0..choices(field, &zones).len() {
                set_choice(&mut row, field, index, &zones);
                assert_eq!(current_choice(&row, field, &zones), index, "field {field}");
                assert_eq!(
                    choice_label(&row, field, &zones),
                    option_labels(field, &zones)[index],
                    "field {field}, option {index}"
                );
            }
        }
        // The spawn zone is the zone's id; "All" is none.
        set_choice(&mut row, SPAWN_ZONE, 2, &zones);
        assert_eq!(row.spawn_zone, Some(2));
        set_choice(&mut row, SPAWN_ZONE, 0, &zones);
        assert_eq!(row.spawn_zone, None);
        // "One Time" starts counting afresh (`OnChangeSpawnType`).
        row.total_spawned = 7;
        set_choice(&mut row, SPAWN_TYPE, 1, &zones);
        assert_eq!(row.total_spawned, 0);
        // An index out of range changes nothing.
        let before = row.growth_at_spawn;
        set_choice(&mut row, GROWTH, 40, &zones);
        assert_eq!(row.growth_at_spawn, before);
    }

    #[test]
    fn a_new_row_has_the_games_defaults_and_the_templates_file_name() {
        let row = new_row("Avarus Nyx.bb8");
        assert_eq!(row.path, "Avarus Nyx.bb8");
        assert_eq!(row.spawn_priority, 1.0);
        assert_eq!(row.minimum, 10);
        assert_eq!(row.tagging, Tagging::NoTagging);
        assert_eq!(row.growth_at_spawn, GrowthAtSpawn::Adult);
        assert_eq!(row.spawn_type, SpawnType::Continuous);
        assert_eq!(row.randomize_genes, RandomizeGenes::NormalMutations);
        assert_eq!(row.spawn_zone, None);
    }

    #[test]
    fn the_shown_fields_follow_the_tagging_and_the_spawn_logic() {
        let mut row = new_row("x.bb8template");
        assert_eq!(
            fields_of(&row, false),
            vec![
                TAGGING, SPAWN_ZONE, PRIORITY, GROWTH, SPAWN_TYPE, MINIMUM, MUTATIONS
            ]
        );
        row.tagging = Tagging::CustomTagging;
        assert!(fields_of(&row, false).contains(&CUSTOM_TAG));
        // "Spawned" only appears for a "One Time" row of a running simulation.
        row.spawn_type = SpawnType::OneTime;
        assert!(!fields_of(&row, false).contains(&SPAWNED));
        assert_eq!(*fields_of(&row, true).last().unwrap(), SPAWNED);
    }

    #[test]
    fn dropdowns_cycle_through_the_games_choices_in_order() {
        let zones = [zone("North", 7), zone("South", 9)];
        let mut row = new_row("x.bb8template");
        let seen: Vec<String> = (0..5)
            .map(|_| {
                let label = choice_label(&row, TAGGING, &zones);
                cycle(&mut row, TAGGING, &zones);
                label
            })
            .collect();
        assert_eq!(seen, ["None", "Species", "Custom", "Random Tag", "None"]);
        let growth: Vec<String> = (0..4)
            .map(|_| {
                cycle(&mut row, GROWTH, &zones);
                choice_label(&row, GROWTH, &zones)
            })
            .collect();
        assert_eq!(growth, ["Elder", "Egg", "Baby", "Adult"]);
        let mutations: Vec<String> = (0..4)
            .map(|_| {
                cycle(&mut row, MUTATIONS, &zones);
                choice_label(&row, MUTATIONS, &zones)
            })
            .collect();
        assert_eq!(mutations, ["Random Color", "All Genes", "No", "Normal"]);
        // The zone dropdown lists "All" and then every zone by name.
        let spawn_zone: Vec<(String, Option<u32>)> = (0..4)
            .map(|_| {
                let label = choice_label(&row, SPAWN_ZONE, &zones);
                let id = row.spawn_zone;
                cycle(&mut row, SPAWN_ZONE, &zones);
                (label, id)
            })
            .collect();
        assert_eq!(
            spawn_zone,
            [
                ("All".to_owned(), None),
                ("North".to_owned(), Some(7)),
                ("South".to_owned(), Some(9)),
                ("All".to_owned(), None),
            ]
        );
    }

    #[test]
    fn switching_to_once_resets_the_spawned_counter() {
        let mut row = new_row("x.bb8template");
        row.total_spawned = 4;
        cycle(&mut row, SPAWN_TYPE, &[]);
        assert_eq!(row.spawn_type, SpawnType::OneTime);
        assert_eq!(row.total_spawned, 0);
        assert_eq!(choice_label(&row, SPAWN_TYPE, &[]), "One Time");
    }

    #[test]
    fn sliders_clamp_round_and_reset() {
        let mut row = new_row("x.bb8template");
        set_value(&mut row, PRIORITY, 1000.0);
        assert_eq!(row.spawn_priority, 100.0);
        set_value(&mut row, PRIORITY, 0.0);
        assert_eq!(row.spawn_priority, 0.01);
        set_value(&mut row, MINIMUM, 123.6);
        assert_eq!(row.minimum, 124);
        set_value(&mut row, MINIMUM, -5.0);
        assert_eq!(row.minimum, 0);
        reset_value(&mut row, MINIMUM);
        reset_value(&mut row, PRIORITY);
        assert_eq!((row.minimum, row.spawn_priority), (10, 1.0));
    }

    #[test]
    fn the_target_count_slider_reaches_zero_at_the_left_end() {
        use crate::dyn_panel::{fraction_range, value_range};
        let info = slider_info(MINIMUM).unwrap();
        let at = |fraction: f32| {
            value_range(info.mode, info.min, info.max, info.default, fraction).round()
        };
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(1.0), 500.0);
        // The default sits where the default pointer is drawn and maps back to itself.
        let fraction = fraction_range(info.mode, info.min, info.max, info.default, 10.0);
        assert_eq!(at(fraction), 10.0);
    }

    #[test]
    fn an_item_is_named_after_its_template() {
        let library = vec![(
            "Avarus Nyx.bb8".to_owned(),
            "Avarus Nyx (the first)".to_owned(),
        )];
        assert_eq!(
            display_name("Avarus Nyx.bb8", &library),
            "Avarus Nyx (the first)"
        );
        assert_eq!(
            display_name("C:/somewhere/Basic bibite.bb8template", &library),
            "Basic bibite"
        );
    }
}
