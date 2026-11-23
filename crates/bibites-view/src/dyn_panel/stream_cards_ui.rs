//! RUSTibites Improved: the Streams section of the in-game Zones tab: a card for each stream, 5 apart, closed or open on its own bar like a zone group's card. A closed card
//! shows the stream's name ("Stream 1") and the bar that opens it; an open one the name with a delete button and six sliders in two columns: where the stream starts (x
//! and y, in the world's units), how wide it is, where it finishes (x and y) and how strong it is. The sliders are the zone cards' own (the same rows of the same prefab,
//! so they look as the game's do) and are addressed through [`crate::zone_editor::stream_ref`]; a click on a value types a number into it.
//!
//! Not in the title menu's scenario editor: a scenario is a game file, and the streams are kept beside the save.

use super::zone_cards_ui::{bar, delete_button, overlaps};
use super::{CardSliderGeometry, Ctx, DynHit, DynTip, FieldRef, ListsInput, SliderSpec, UiAction, card_slider_row, fraction_range, info_rules, sprite_node, text_node};
use crate::zone_cards::{self, BAR_H, CLOSED_HEADER_H, HEADER_H};
use crate::zone_editor as ze;

/// The space between two cards (`Zone List`'s spacing).
const GAP: f32 = 5.0;
/// From one row of sliders to the next: a 30 high row and 10 between.
const ROW_PITCH: f32 = 40.0;
/// How many rows of two sliders an open card has.
const ROWS: usize = 3;

/// The fields of an open card, row by row: the left column (the start and the width), the right one (the finish and the strength).
const FIELDS: [[u8; 2]; ROWS] = [[ze::STREAM_START_X, ze::STREAM_END_X], [ze::STREAM_START_Y, ze::STREAM_END_Y], [ze::STREAM_WIDTH, ze::STREAM_STRENGTH]];

/// The height of a card.
fn card_height(open: bool) -> f32 {
    if open {
        HEADER_H + 10.0 + ROWS as f32 * ROW_PITCH + BAR_H
    } else {
        CLOSED_HEADER_H + BAR_H
    }
}

/// What a stream's name says: its place in the list.
#[must_use]
pub fn stream_name(index: usize) -> String {
    format!("Stream {}", index + 1)
}

/// The path of the slider row of `field` in the zone card's prefab (every row of a card has one of its own: the Image multipliers are keyed by it).
fn row_path(field: u8) -> &'static str {
    match field {
        ze::STREAM_START_X => "Two Rows (2)/Left Row/posX",
        ze::STREAM_START_Y => "Two Rows (2)/Left Row/posy",
        ze::STREAM_WIDTH => "Two Rows (2)/Left Row/width",
        ze::STREAM_END_X => "Two Rows (2)/Right Row/radius1",
        ze::STREAM_END_Y => "Two Rows (2)/Right Row/height",
        _ => "Two Rows (2)/Right Row/insideRadius",
    }
}

/// The tooltip of a stream setting.
fn tip(field: u8) -> crate::tooltip::Tip {
    let (title, content) = match field {
        ze::STREAM_START_X => ("Start X", "The x of the point where the stream begins. The stream pushes from here towards its finish."),
        ze::STREAM_START_Y => ("Start Y", "The y of the point where the stream begins. The stream pushes from here towards its finish."),
        ze::STREAM_END_X => ("End X", "The x of the point where the stream finishes. Everything in the stream is pushed towards it."),
        ze::STREAM_END_Y => ("End Y", "The y of the point where the stream finishes. Everything in the stream is pushed towards it."),
        ze::STREAM_WIDTH => ("Width", "How wide the band is, across the line from the start to the finish."),
        _ => ("Strength", "How hard the stream pushes the bibites, pellets and eggs in it, as an acceleration in u/s per second (the world's drag sets the speed they settle at)."),
    };
    crate::tooltip::Tip {
        path: "",
        title,
        content,
        ui: true,
        instant: false,
    }
}

/// The streams of the Zones tab: a card each, open or closed on its own bar (the empty list is the grey placeholder). Returns the height of the list (with the 60 of
/// padding the other lists end with).
pub(super) fn layout_streams(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, min_height: f32, y: f32) -> f32 {
    if lists.streams.is_empty() {
        super::layout_placeholder(ctx, "NoGroups", "No Streams", y);
        return min_height;
    }
    let mut cursor = y;
    for index in 0..lists.streams.len() {
        let open = lists.open.contains(&(4, index as u16));
        let height = card_height(open);
        if overlaps(ctx, cursor, height) {
            paint_stream(ctx, lists, index, open, cursor, height);
        }
        cursor += height + GAP;
    }
    (cursor - GAP - y + 60.0).max(min_height)
}

/// One card at `cursor`.
fn paint_stream(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, index: usize, open: bool, cursor: f32, height: f32) {
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let width = x1 - x0;
    let stream = &lists.streams[index];
    let base = format!("UI/ScenarioEditor/Items/4/Item{index}/ZoneHandle(Clone)");
    let (view_top, view_bottom) = (ctx.view_top, ctx.view_bottom);
    let mut root = sprite_node(&base, "base_icon", [x0, cursor.max(view_top), x1, (cursor + height).min(view_bottom)], [1.0; 4]);
    let toggle = UiAction::ZoneOpen(4, index as u16);
    if !open {
        // The name and the bar that opens the card (a click on the name opens it too).
        if ctx.visible(cursor, CLOSED_HEADER_H) {
            root.children.push(text_node(&format!("{base}/Header/zoneName/line"), &stream_name(index), 26.0, [x0 + 10.0, cursor + 8.0, x1 - 10.0, cursor + 38.47]));
            ctx.hits.push(DynHit {
                rect: [x0, cursor, x1 - x0, CLOSED_HEADER_H],
                action: toggle,
            });
        }
        let bar_top = cursor + CLOSED_HEADER_H;
        if ctx.visible(bar_top, BAR_H) {
            bar(ctx, &mut root, &base, (x0, x1), bar_top, false, toggle);
        }
        ctx.add(root);
        return;
    }
    if ctx.visible(cursor, HEADER_H) {
        root.children.push(text_node(&format!("{base}/Header/zoneName/line"), &stream_name(index), 26.0, [x0 + 10.0, cursor + 9.8, x1 - 60.0, cursor + 40.27]));
        delete_button(ctx, &mut root, &base, x1, cursor, (lists.delete_item)(4, index as u16));
    }
    // The six sliders, two columns of three.
    let column_w = (width - 20.0) * 0.5;
    let columns = [x0 + 10.0, x0 + 10.0 + column_w];
    let reference = ze::stream_ref(index);
    for (row, fields) in FIELDS.iter().enumerate() {
        let row_y = cursor + HEADER_H + 10.0 + row as f32 * ROW_PITCH;
        if !ctx.visible(row_y, 30.0) {
            continue;
        }
        for (column, &field) in fields.iter().enumerate() {
            let Some(info) = ze::stream_slider_info(field, lists.sim_size) else {
                continue;
            };
            let value = ze::stream_value_of(stream, field);
            let field_ref = FieldRef::Zone(reference, field);
            let spec = SliderSpec {
                name: ze::stream_name_of(field),
                text: info.format().format(value),
                fraction: fraction_range(info.mode, info.min, info.max, info.default, value),
                default_fraction: fraction_range(info.mode, info.min, info.max, info.default, info.default),
                field: field_ref,
                format: info.format(),
                rules: info_rules(&info, false),
            };
            let [name_w, value_w, panel_w] = zone_cards::slider_parts(column_w);
            let geometry = CardSliderGeometry {
                name_w,
                value_x: name_w + 15.0,
                value_w,
                value_align: "Center",
                panel_x: name_w + 15.0 + value_w + 15.0,
                panel_w,
            };
            let editing = ctx.input.edit.filter(|edit| edit.field == field_ref);
            let x = columns[column];
            card_slider_row(&mut root, &mut ctx.hits, editing, &format!("{base}/Edit Section/{}", row_path(field)), (x, row_y), &geometry, &spec);
            ctx.tips.push(DynTip {
                rect: [x, row_y, name_w, 30.0],
                tip: tip(field),
            });
        }
    }
    let bar_top = cursor + height - BAR_H;
    if ctx.visible(bar_top, BAR_H) {
        bar(ctx, &mut root, &base, (x0, x1), bar_top, true, toggle);
    }
    ctx.add(root);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_card_is_as_high_as_a_closed_group_card_and_an_open_one_holds_three_rows_of_sliders() {
        assert_eq!(card_height(false), CLOSED_HEADER_H + BAR_H);
        assert_eq!(card_height(true), HEADER_H + 10.0 + 3.0 * 40.0 + BAR_H);
        assert!(card_height(true) > card_height(false));
    }

    #[test]
    fn the_cards_are_named_by_their_place_in_the_list_and_every_field_has_a_row_of_its_own_in_the_prefab() {
        assert_eq!((stream_name(0), stream_name(11)), ("Stream 1".to_owned(), "Stream 12".to_owned()));
        let mut paths: Vec<&str> = FIELDS.iter().flatten().map(|field| row_path(*field)).collect();
        assert_eq!(paths.len(), 6);
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), 6, "no two sliders of a card share a row path");
        for field in FIELDS.iter().flatten() {
            assert!(ze::stream_slider_info(*field, 5000.0).is_some() && !ze::stream_name_of(*field).is_empty(), "field {field}");
            assert!(!tip(*field).title.is_empty());
        }
    }
}
