//! The open zone card (`ZoneSettingsHandle`), the zone group card (`ZoneGroupHandle`) and the template card inside it, laid out the way the
//! game lays them out. The numbers are read from the live game (every state of the controls: with and without a spawned material, circle, ring
//! and rectangle, relative and absolute sizes, the three movements): `zone_card_states.json` holds what the game showed and the tests below
//! hold this module to it.
//!
//! A card is a column of sections (`VerticalLayoutGroup`) of rows in two columns (`HorizontalLayoutGroup`): which rows a section shows depends on
//! the zone (`ZoneSettingsHandle.ChangePelletType` / `ChangeMovementType` / `ShowCorrectSizeSliders`), and the widths of the parts of a row come
//! from the layout elements of its prefab ([`distribute`]).

use bibites_core::settings::{MovementType, SpawnDistribution, ZoneSettings};

use crate::zone_editor as ze;

/// What a row of a card holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A label and a dropdown.
    Dropdown,
    /// A label, the value and a slider.
    Slider,
    /// A label and a checkbox.
    Toggle,
}

/// One control of a card: the field of `zone_editor` it edits, and where it is, from the section's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowPlace {
    /// `zone_editor` field.
    pub field: u8,
    /// How the row looks.
    pub kind: RowKind,
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width of the row.
    pub w: f32,
}

/// The edit section of a card: its rows and its height.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// The controls.
    pub rows: Vec<RowPlace>,
    /// Height of the section (`Edit Section` of a zone card, the box of a template).
    pub height: f32,
}

/// Height of the header of an open zone or group card: its 8 of padding and the 34 high move buttons.
pub const HEADER_H: f32 = 42.0;
/// Height of the bar that closes or opens a card (`CloseButton` / `OpenButton`).
pub const BAR_H: f32 = 20.0;
/// Height of the header of a closed group card (its name only: 8 of padding and 30.47).
pub const CLOSED_HEADER_H: f32 = 38.47;
/// Height of every row of a card.
pub const ROW_H: f32 = 30.0;

/// The parts of a row along the row, as `HorizontalLayoutGroup` sizes them: `parts` are `(minimum, preferred, flexible)` widths, `spacing`
/// sits between two. With room to spare the extra goes to the parts by their flexible weights; with less than the preferred widths every part
/// takes its place between its minimum and its preferred width alike.
#[must_use]
pub fn distribute(available: f32, parts: &[(f32, f32, f32)], spacing: f32) -> Vec<f32> {
    let gaps = spacing * parts.len().saturating_sub(1) as f32;
    let min: f32 = parts.iter().map(|part| part.0).sum::<f32>() + gaps;
    let pref: f32 = parts.iter().map(|part| part.1).sum::<f32>() + gaps;
    let flex: f32 = parts.iter().map(|part| part.2).sum();
    if available >= pref {
        let extra = available - pref;
        parts
            .iter()
            .map(|part| part.1 + if flex > 0.0 { extra * part.2 / flex } else { 0.0 })
            .collect()
    } else {
        let t = if pref > min { ((available - min) / (pref - min)).clamp(0.0, 1.0) } else { 0.0 };
        parts.iter().map(|part| part.0 + (part.1 - part.0) * t).collect()
    }
}

/// The widths of the label, the value and the slider of a slider row `width` wide (`name`: minimum 100, preferred 125; the value: 60, 80; the
/// slider panel: 100, 200; flexible 2, 1, 1; 15 apart).
#[must_use]
pub fn slider_parts(width: f32) -> [f32; 3] {
    let parts = distribute(width, &[(100.0, 125.0, 2.0), (60.0, 80.0, 1.0), (100.0, 200.0, 1.0)], 15.0);
    [parts[0], parts[1], parts[2]]
}

/// The widths of the label and the dropdown of a dropdown row (`name`: preferred 215, flexible 3; the dropdown: preferred 200, flexible 1).
#[must_use]
pub fn dropdown_parts(width: f32) -> [f32; 2] {
    let parts = distribute(width, &[(100.0, 215.0, 3.0), (0.0, 200.0, 1.0)], 15.0);
    [parts[0], parts[1]]
}

/// The widths of the label and the checkbox area of a toggle row (preferred 220 and 200, flexible 3 and 1).
#[must_use]
pub fn toggle_parts(width: f32) -> [f32; 2] {
    let parts = distribute(width, &[(0.0, 220.0, 3.0), (0.0, 200.0, 1.0)], 15.0);
    [parts[0], parts[1]]
}

/// Left edge of the left column, of the right one and the width of a column, for a section `width` wide with `pad` of padding on each side (the
/// two rows of a `Two Rows` group have 5 of padding and 20 between them).
fn columns(pad: f32, width: f32) -> (f32, f32, f32) {
    let column = (width - 2.0 * pad - 10.0 - 20.0) * 0.5;
    let left = pad + 5.0;
    (left, left + column + 20.0, column)
}

fn row(field: u8, kind: RowKind, x: f32, y: f32, w: f32) -> RowPlace {
    RowPlace { field, kind, x, y, w }
}

/// What the movement type adds next to the movement dropdown: a target dropdown, and the speed slider of a free zone.
fn movement_rows(zone: &ZoneSettings) -> (bool, bool) {
    (
        matches!(zone.movement, MovementType::Orbit | MovementType::Attached),
        zone.movement == MovementType::Free,
    )
}

/// The size rows: `(field, is on the right)` of a circle, a ring and a rectangle.
fn size_fields(zone: &ZoneSettings) -> Vec<u8> {
    match zone.distribution {
        SpawnDistribution::Rect => vec![ze::WIDTH, ze::HEIGHT],
        SpawnDistribution::Ring | SpawnDistribution::FlatRing => vec![ze::RADIUS, ze::INSIDE],
        _ => vec![ze::RADIUS],
    }
}

/// The edit section of an open zone card `width` wide (`ZoneSettingsHandle`, below its header): 10 of padding at the top, 20 between the
/// sections, 5 at the bottom.
#[must_use]
pub fn zone_section(width: f32, zone: &ZoneSettings) -> Section {
    use RowKind::{Dropdown, Slider, Toggle};
    let (left, right, column) = columns(10.0, width);
    let mut rows = Vec::new();
    let mut y = 10.0;
    // What it spawns: the material, and when there is one the distribution and the four rows of the spawn.
    rows.push(row(ze::MATERIAL, Dropdown, left, y, column));
    if zone.material.is_some() {
        rows.push(row(ze::DISTRIBUTION, Dropdown, right, y, column));
        rows.push(row(ze::FERTILITY, Slider, left, y + 35.0, column));
        rows.push(row(ze::PELLET_SIZE, Slider, left, y + 65.0, column));
        rows.push(row(ze::BIOMASS, Slider, right, y + 35.0, column));
        rows.push(row(ze::RENEW, Toggle, right, y + 65.0, column));
        y += 95.0;
    } else {
        y += ROW_H;
    }
    y += 20.0;
    // How it moves, and whether its size follows the simulation's.
    let (target, speed) = movement_rows(zone);
    rows.push(row(ze::MOVEMENT, Dropdown, left, y, column));
    let mut right_y = y;
    if target {
        rows.push(row(ze::TARGET, Dropdown, right, right_y, column));
        right_y += 35.0;
    }
    rows.push(row(ze::SIZE_SCALES, Toggle, right, right_y, column));
    if speed {
        rows.push(row(ze::SPEED, Slider, right, right_y + 35.0, column));
    }
    y += if target || speed { 65.0 } else { ROW_H };
    y += 20.0;
    // Where it is (left) and how big (right).
    rows.push(row(ze::POS_X, Slider, left, y, column));
    rows.push(row(ze::POS_Y, Slider, left, y + ROW_H, column));
    for (slot, field) in size_fields(zone).into_iter().enumerate() {
        rows.push(row(field, Slider, right, y + slot as f32 * ROW_H, column));
    }
    y += 2.0 * ROW_H;
    Section { rows, height: y + 5.0 }
}

/// The template card of a zone group `width` wide (`ZoneGroupHandle`'s own `ZoneHandle`, which has no header and no place of its own): a box
/// with 10 of padding, 10 between the sections; a circle's radius takes the whole width.
#[must_use]
pub fn template_section(width: f32, zone: &ZoneSettings) -> Section {
    use RowKind::{Dropdown, Slider, Toggle};
    let (left, right, column) = columns(10.0, width);
    let mut rows = Vec::new();
    let mut y = 10.0;
    rows.push(row(ze::MATERIAL, Dropdown, left, y, column));
    if zone.material.is_some() {
        rows.push(row(ze::DISTRIBUTION, Dropdown, right, y, column));
        rows.push(row(ze::FERTILITY, Slider, left, y + 40.0, column));
        rows.push(row(ze::PELLET_SIZE, Slider, left, y + 75.0, column));
        rows.push(row(ze::BIOMASS, Slider, right, y + 40.0, column));
        rows.push(row(ze::RENEW, Toggle, right, y + 75.0, column));
        y += 105.0;
    } else {
        y += ROW_H;
    }
    y += 10.0;
    let (target, speed) = movement_rows(zone);
    rows.push(row(ze::MOVEMENT, Dropdown, left, y, column));
    let mut right_y = y;
    if target {
        rows.push(row(ze::TARGET, Dropdown, right, right_y, column));
        right_y += 35.0;
    }
    rows.push(row(ze::SIZE_SCALES, Toggle, right, right_y, column));
    if speed {
        rows.push(row(ze::SPEED, Slider, right, right_y + 35.0, column));
    }
    y += if target || speed { 65.0 } else { ROW_H };
    y += 10.0;
    let sizes = size_fields(zone);
    if sizes.len() == 1 {
        // The right row has nothing: the left one takes its room.
        rows.push(row(sizes[0], Slider, left, y, 2.0 * column));
    } else {
        rows.push(row(sizes[0], Slider, left, y, column));
        rows.push(row(sizes[1], Slider, right, y, column));
    }
    y += ROW_H;
    Section { rows, height: y + 10.0 }
}

/// Where the parts of an open zone group card sit, from its top-left corner (the card is `width` wide).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupCard {
    /// Total height.
    pub height: f32,
    /// Top of the line with the count type and the count (`Group Parameters/Line`).
    pub params_y: f32,
    /// Left edge of the left row (the toggle) and of the right one (the count slider); both 457.5 wide at 935.
    pub params_x: [f32; 2],
    /// Width of each of the two rows.
    pub params_w: f32,
    /// Top of the template line (the caption, the count and the reroll button).
    pub template_line_y: f32,
    /// Height of the template line.
    pub template_line_h: f32,
    /// Top of the template box.
    pub template_y: f32,
    /// Top of the bar at the bottom.
    pub bar_y: f32,
}

/// The open group card for a template box of height `template_h`.
#[must_use]
pub fn group_card(width: f32, template_h: f32) -> GroupCard {
    const LINE_H: f32 = 40.47;
    let params_y = HEADER_H + 10.0;
    let template_line_y = params_y + ROW_H + 15.0;
    // (The section's own spacing is -5 and its bottom padding -2.)
    let template_y = template_line_y + LINE_H - 5.0;
    let bar_y = template_y + template_h - 2.0 + 10.0 - 10.0;
    GroupCard {
        height: bar_y + BAR_H,
        params_y,
        params_x: [10.0, 10.0 + (width - 20.0) * 0.5],
        params_w: (width - 20.0) * 0.5,
        template_line_y,
        template_line_h: LINE_H,
        template_y,
        bar_y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("zone_card_states.json")).expect("fixture")
    }

    /// The zone a captured state was made with.
    fn zone_of(state: &str) -> ZoneSettings {
        let mut zone = ZoneSettings::game_defaults();
        zone.material = Some(bibites_core::settings::MaterialKind::Plant);
        zone.distribution = SpawnDistribution::CentricGradual;
        zone.movement = MovementType::None;
        zone.radius_is_relative = true;
        match state {
            "default" => {}
            "no_material" => zone.material = None,
            "ring" => zone.distribution = SpawnDistribution::Ring,
            "rect" => zone.distribution = SpawnDistribution::Rect,
            "free" => zone.movement = MovementType::Free,
            "attached" => zone.movement = MovementType::Attached,
            "absolute" => zone.radius_is_relative = false,
            "rect_absolute" => {
                zone.distribution = SpawnDistribution::Rect;
                zone.radius_is_relative = false;
            }
            "ring_absolute_free" => {
                zone.distribution = SpawnDistribution::Ring;
                zone.radius_is_relative = false;
                zone.movement = MovementType::Free;
            }
            "rect_attached" => {
                zone.distribution = SpawnDistribution::Rect;
                zone.movement = MovementType::Attached;
            }
            "none_free_rect_abs" => {
                zone.material = None;
                zone.movement = MovementType::Free;
                zone.distribution = SpawnDistribution::Rect;
                zone.radius_is_relative = false;
            }
            other => panic!("unknown state {other}"),
        }
        zone
    }

    /// The field a prefab row stands for.
    fn field_of(name: &str) -> u8 {
        match name {
            "pelletDropDown" => ze::MATERIAL,
            "distibutionDropdown" => ze::DISTRIBUTION,
            "fertility" => ze::FERTILITY,
            "size" => ze::PELLET_SIZE,
            "biomass" => ze::BIOMASS,
            "renewToggle" => ze::RENEW,
            "movement" => ze::MOVEMENT,
            "target" => ze::TARGET,
            "speed" => ze::SPEED,
            "radiusType" => ze::SIZE_SCALES,
            "posX" => ze::POS_X,
            "posy" => ze::POS_Y,
            "radius1" | "radius2" => ze::RADIUS,
            "width" | "width2" => ze::WIDTH,
            "height" | "height2" => ze::HEIGHT,
            "insideRadius" => ze::INSIDE,
            other => panic!("unknown row {other}"),
        }
    }

    /// Every captured state, laid out the same way and held to the game's numbers (a column is `width` ± 1.5: the game's layout rounds a
    /// little differently for a row that has company in the section).
    fn check(group: &str, top: f32, layout: &dyn Fn(f32, &ZoneSettings) -> Section, card_height: &dyn Fn(f32) -> f32) {
        let fixture = fixture();
        let states = fixture[group].as_object().expect("states");
        assert!(states.len() >= 9);
        for (state, expected) in states {
            let size = expected["size"].as_array().expect("size");
            let width = size[0].as_f64().expect("width") as f32;
            let section = layout(width, &zone_of(state));
            let height = size[1].as_f64().expect("height") as f32;
            assert!((card_height(section.height) - height).abs() < 0.6, "{group} {state}: height {} against {height}", card_height(section.height));
            let rows = expected["rows"].as_array().expect("rows");
            assert_eq!(rows.len(), section.rows.len(), "{group} {state}: the rows shown");
            for entry in rows {
                let name = entry[0].as_str().expect("name");
                let (x, y, w) = (
                    entry[1].as_f64().expect("x") as f32,
                    entry[2].as_f64().expect("y") as f32,
                    entry[3].as_f64().expect("w") as f32,
                );
                let field = field_of(name);
                let place = section
                    .rows
                    .iter()
                    .find(|place| place.field == field)
                    .unwrap_or_else(|| panic!("{group} {state}: {name} is not laid out"));
                assert!((place.x - x).abs() < 1.5, "{group} {state} {name}: x {} against {x}", place.x);
                assert!((place.y + top - y).abs() < 0.6, "{group} {state} {name}: y {} against {y}", place.y + top);
                assert!((place.w - w).abs() < 1.5, "{group} {state} {name}: w {} against {w}", place.w);
            }
        }
    }

    #[test]
    fn the_zone_card_is_laid_out_as_the_game_lays_it_out() {
        check("zone", HEADER_H, &zone_section, &|edit| HEADER_H + edit + BAR_H);
    }

    #[test]
    fn the_template_card_is_laid_out_as_the_game_lays_it_out() {
        check("template", 0.0, &template_section, &|height| height);
    }

    #[test]
    fn the_group_card_is_laid_out_as_the_game_lays_it_out() {
        let fixture = fixture();
        let open = &fixture["group"]["default"];
        let size = open["size"].as_array().expect("size");
        let width = size[0].as_f64().expect("w") as f32;
        let height = size[1].as_f64().expect("h") as f32;
        let template = template_section(width - 20.0, &zone_of("default"));
        let card = group_card(width, template.height);
        assert!((card.height - height).abs() < 0.6, "height {} against {height}", card.height);
        let nodes = open["nodes"].as_object().expect("nodes");
        let at = |path: &str| -> [f64; 4] {
            let entry = nodes[path].as_array().unwrap_or_else(|| panic!("{path}"));
            [entry[0].as_f64().unwrap(), entry[1].as_f64().unwrap(), entry[2].as_f64().unwrap(), entry[3].as_f64().unwrap()]
        };
        assert!((f64::from(card.params_y) - at("Group Parameters/Line")[1]).abs() < 0.6);
        assert!((f64::from(card.params_x[1]) - at("Group Parameters/Line/RightRow")[0]).abs() < 0.6);
        assert!((f64::from(card.params_w) - at("Group Parameters/Line/LeftRow")[2]).abs() < 0.6);
        assert!((f64::from(card.template_line_y) - at("Group Parameters/Template Section/Line")[1]).abs() < 0.6);
        assert!((f64::from(card.template_y) - at("Group Parameters/Template Section/ZoneHandle")[1]).abs() < 0.6);
        assert!((f64::from(card.bar_y) - at("CloseButton")[1]).abs() < 0.6);
        // The parts of the rows of the line: the count slider's row and the toggle's.
        let [name, value, slider] = slider_parts(card.params_w);
        let count = "Group Parameters/Line/RightRow/count";
        assert!((f64::from(name) - at(&format!("{count}/Name"))[2]).abs() < 0.6);
        assert!((f64::from(value) - at(&format!("{count}/FloatInfoText"))[2]).abs() < 0.6);
        assert!((f64::from(slider) - at(&format!("{count}/NormalSecton"))[2]).abs() < 0.6);
        let [label, check] = toggle_parts(card.params_w);
        assert!((f64::from(label) - at("Group Parameters/Line/LeftRow/countType/Name")[2]).abs() < 0.6);
        assert!((f64::from(check) - at("Group Parameters/Line/LeftRow/countType/Toggle")[2]).abs() < 0.6);
        // A closed card is its header and the bar.
        let closed = &fixture["group"]["closed"];
        let closed_height = closed["size"][1].as_f64().expect("h") as f32;
        assert!((CLOSED_HEADER_H + BAR_H - closed_height).abs() < 0.6);
    }

    #[test]
    fn the_parts_of_a_row_share_the_width_like_the_layout_group_does() {
        // 442.5 and 432.5 wide columns (read from the game).
        let [name, value, slider] = slider_parts(442.5);
        assert!((name - 128.75).abs() < 0.01 && (value - 81.875).abs() < 0.01 && (slider - 201.875).abs() < 0.01);
        let [name, value, slider] = slider_parts(432.5);
        assert!((name - 124.569).abs() < 0.01 && (value - 79.655).abs() < 0.01 && (slider - 198.276).abs() < 0.01);
        let [name, dropdown] = dropdown_parts(442.5);
        assert!((name - 224.375).abs() < 0.01 && (dropdown - 203.125).abs() < 0.01);
        let [name, check] = toggle_parts(442.5);
        assert!((name - 225.625).abs() < 0.01 && (check - 201.875).abs() < 0.01);
    }
}
