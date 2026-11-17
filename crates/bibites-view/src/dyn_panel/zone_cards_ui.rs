//! The open zone cards and the zone group cards of the Zones tab (`ZoneSettingsHandle`, `ZoneGroupHandle`), drawn on the layout of
//! [`crate::zone_cards`] (numbers read from the live game).

use super::{
    CardSliderGeometry, Ctx, DynHit, DynTip, FieldRef, ListsInput, Node, SliderSpec, UiAction, card_slider_row, card_text, fraction_range, info_rules,
    sprite_node, text_node,
};
use crate::zone_cards::{self, BAR_H, CLOSED_HEADER_H, HEADER_H, ROW_H, RowKind, RowPlace, Section};
use crate::zone_editor as ze;
use bibites_core::settings::ZoneSettings;

/// The dark grey of the text on a light card.
const INK: f32 = 0.164_705_88;

/// Whether a card at `cursor` of `height` shows in the viewport at all (a card that is only partly in it is drawn clipped: its parts that lie wholly
/// inside are emitted).
fn overlaps(ctx: &Ctx<'_>, cursor: f32, height: f32) -> bool {
    !ctx.dry && cursor + height > ctx.view_top && cursor < ctx.view_bottom
}

/// A left-aligned label in a row of a card: 10 in from the row's left edge (the label's own margin).
fn label(root: &mut Node, path: &str, text: &str, x: f32, y: f32, width: f32) {
    root.children.push(card_text(path, text, 20.0, [x + 10.0, y, x + width, y + ROW_H], "Left", INK));
}

/// The path of a row of the edit section in the prefab (the Image multipliers and the node names are keyed by it).
fn row_path(field: u8, template: bool, zone: &ZoneSettings) -> &'static str {
    let relative = zone.radius_is_relative;
    match field {
        ze::MATERIAL => "spawnsection/Line/Two Rows (1)/Left Row/pelletDropDown",
        ze::DISTRIBUTION => "spawnsection/Line/Two Rows (1)/Right Row/distibutionDropdown",
        ze::FERTILITY => "spawnsection/spawnRelatedSection/Left Row/fertility",
        ze::PELLET_SIZE => "spawnsection/spawnRelatedSection/Left Row/size",
        ze::BIOMASS => "spawnsection/spawnRelatedSection/Right Row/biomass",
        ze::RENEW => "spawnsection/spawnRelatedSection/Right Row/renewToggle",
        ze::MOVEMENT => "MovementSection/Two Rows (1)/Left Row/movement",
        ze::TARGET => "MovementSection/Two Rows (1)/Right Row/target",
        ze::SIZE_SCALES => "MovementSection/Two Rows (1)/Right Row/radiusType",
        ze::SPEED => "MovementSection/Two Rows (1)/Right Row/speed",
        ze::POS_X => "Two Rows (2)/Left Row/posX",
        ze::POS_Y => "Two Rows (2)/Left Row/posy",
        ze::RADIUS if template => {
            if relative {
                "Two Rows (2)/Left Row/radius1"
            } else {
                "Two Rows (2)/Left Row/radius2"
            }
        }
        ze::RADIUS => {
            if relative {
                "Two Rows (2)/Right Row/radius1"
            } else {
                "Two Rows (2)/Right Row/radius2"
            }
        }
        ze::WIDTH if template => "Two Rows (2)/Left Row/width",
        ze::WIDTH => {
            if relative {
                "Two Rows (2)/Right Row/width"
            } else {
                "Two Rows (2)/Right Row/width2"
            }
        }
        ze::HEIGHT => "Two Rows (2)/Right Row/height",
        _ => "Two Rows (2)/Right Row/insideRadius",
    }
}

/// What a card's controls are bound to.
#[derive(Clone, Copy)]
struct Binding<'a> {
    /// `FieldRef::Zone` index of the zone or template.
    index: u16,
    /// Settings shown.
    zone: &'a ZoneSettings,
    /// Every zone (the names the target dropdown lists).
    all: &'a [ZoneSettings],
    /// Whether it is a group's template.
    template: bool,
    /// Simulation size (the absolute sliders' upper end).
    sim_size: f32,
}

/// One dropdown row: the label and a dropdown frame holding the current choice and an arrow (`dropdown_parts` gives the widths).
fn dropdown_row(root: &mut Node, hits: &mut Vec<DynHit>, base: &str, (x, y, w): (f32, f32, f32), text: &str, choice: &str, action: UiAction) {
    let [name_w, dropdown_w] = zone_cards::dropdown_parts(w);
    label(root, &format!("{base}/Name"), text, x, y, name_w);
    let dx = x + name_w + 15.0;
    root.children.push(sprite_node(&format!("{base}/Dropdown"), "UISprite", [dx, y, dx + dropdown_w, y + ROW_H], [1.0; 4]));
    root.children.push(card_text(
        &format!("{base}/Dropdown/Label"),
        choice,
        16.0,
        [dx + 10.0, y + 7.0, dx + dropdown_w - 25.0, y + 24.0],
        "Left",
        0.196_078_43,
    ));
    root.children.push(sprite_node(
        &format!("{base}/Dropdown/Arrow"),
        "DropdownArrow",
        [dx + dropdown_w - 35.0, y, dx + dropdown_w - 5.0, y + ROW_H],
        [1.0; 4],
    ));
    hits.push(DynHit {
        rect: [dx, y, dropdown_w, ROW_H],
        action,
    });
}

/// One toggle row: the label and a 30 x 30 box in the middle of the toggle's room, with a checkmark when it is on.
fn toggle_row(root: &mut Node, hits: &mut Vec<DynHit>, base: &str, (x, y, w): (f32, f32, f32), text: &str, on: bool, action: UiAction) {
    let [name_w, toggle_w] = zone_cards::toggle_parts(w);
    label(root, &format!("{base}/Name"), text, x, y, name_w);
    let bx = x + name_w + 15.0 + toggle_w * 0.5 - 15.0;
    let rect = [bx, y, bx + 30.0, y + ROW_H];
    root.children.push(sprite_node(&format!("{base}/Toggle/Background"), "base_icon", rect, [1.0; 4]));
    if on {
        root.children.push(sprite_node(&format!("{base}/Toggle/Background/Checkmark"), "checkmark_icon", rect, [1.0; 4]));
    }
    hits.push(DynHit {
        rect: [rect[0], rect[1], 30.0, ROW_H],
        action,
    });
}

/// One row of an edit section at `origin` (the section's top-left corner).
fn paint_row(ctx: &mut Ctx<'_>, root: &mut Node, base: &str, binding: Binding<'_>, origin: (f32, f32), place: &RowPlace) {
    let (x, y) = (origin.0 + place.x, origin.1 + place.y);
    if !ctx.visible(y, ROW_H) {
        return;
    }
    let field = place.field;
    let zone = binding.zone;
    let row_base = format!("{base}/Edit Section/{}", row_path(field, binding.template, zone));
    let reference = FieldRef::Zone(binding.index, field);
    let text = ze::name_of(field);
    if let Some(tip) = ze::help_key(zone, field).and_then(|key| crate::tooltip::of_item("zone", key)) {
        let name_w = match place.kind {
            RowKind::Dropdown => zone_cards::dropdown_parts(place.w)[0],
            RowKind::Slider => zone_cards::slider_parts(place.w)[0],
            RowKind::Toggle => zone_cards::toggle_parts(place.w)[0],
        };
        ctx.tips.push(DynTip {
            rect: [x, y, name_w, ROW_H],
            tip,
        });
    }
    match place.kind {
        RowKind::Dropdown => dropdown_row(
            root,
            &mut ctx.hits,
            &row_base,
            (x, y, place.w),
            text,
            &ze::choice_label(zone, field, binding.all),
            UiAction::DropdownOpen(crate::ui::DropdownTarget::Zone(binding.index, field)),
        ),
        RowKind::Toggle => toggle_row(
            root,
            &mut ctx.hits,
            &row_base,
            (x, y, place.w),
            text,
            ze::toggle_value(zone, field),
            reference.toggle(),
        ),
        RowKind::Slider => {
            let Some(info) = ze::slider_info(zone, field, binding.sim_size) else {
                return;
            };
            let value = ze::value_of(zone, field);
            let spec = SliderSpec {
                name: text,
                text: info.format().format(value),
                fraction: fraction_range(info.mode, info.min, info.max, info.default, value),
                default_fraction: fraction_range(info.mode, info.min, info.max, info.default, info.default),
                field: reference,
                format: info.format(),
                rules: info_rules(&info, false),
            };
            let [name_w, value_w, panel_w] = zone_cards::slider_parts(place.w);
            let geometry = CardSliderGeometry {
                name_w,
                value_x: name_w + 15.0,
                value_w,
                value_align: "Center",
                panel_x: name_w + 15.0 + value_w + 15.0,
                panel_w,
            };
            let editing = ctx.input.edit.filter(|edit| edit.field == reference);
            card_slider_row(root, &mut ctx.hits, editing, &row_base, (x, y), &geometry, &spec);
        }
    }
}

/// The rows of `section`, drawn on `root` below `origin`.
fn paint_section(ctx: &mut Ctx<'_>, root: &mut Node, base: &str, binding: Binding<'_>, origin: (f32, f32), section: &Section) {
    for place in &section.rows {
        paint_row(ctx, root, base, binding, origin, place);
    }
}

/// The unframed delete button of a card header: a 30 x 30 button whose icon, 50 x 50, is centred on it.
fn delete_button(ctx: &mut Ctx<'_>, root: &mut Node, base: &str, x1: f32, top: f32, action: UiAction) {
    let (cx, cy) = (x1 - 25.0, top + 25.0);
    root.children.push(sprite_node(&format!("{base}/Header/Delete/Text (TMP)"), "exit_icon", [cx - 25.0, cy - 25.0, cx + 25.0, cy + 25.0], [1.0; 4]));
    ctx.hits.push(DynHit {
        rect: [x1 - 40.0, top + 10.0, 30.0, 30.0],
        action,
    });
}

/// The two move arrows at the left of an open header (`Mover`: 19 x 34, the second one turned over).
pub(super) fn mover(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, list: u8, index: usize, len: usize, root: &mut Node, base: &str, x0: f32, top: f32) {
    // The arrows move the card in the shown order; the first card cannot go up and the last cannot go down (`ReorderableItem.UpdateButtons`).
    let order = crate::dyn_panel::card_order(lists.order, list, len);
    let at = order.iter().position(|item| *item == index).unwrap_or(0);
    if at > 0 {
        ctx.hits.push(DynHit { rect: [x0 + 2.0, top + 8.0, 35.0, 17.0], action: UiAction::ListMove(list, index as u16, true) });
    }
    if at + 1 < order.len() {
        ctx.hits.push(DynHit { rect: [x0 + 2.0, top + 25.0, 35.0, 17.0], action: UiAction::ListMove(list, index as u16, false) });
    }
    let up = sprite_node(&format!("{base}/Header/Mover/UpButton"), "Arrow", [x0 + 2.0, top + 8.0, x0 + 37.0, top + 25.0], [1.0; 4]);
    let mut down = sprite_node(&format!("{base}/Header/Mover/DownButton"), "Arrow", [x0 + 2.0, top + 25.0, x0 + 37.0, top + 42.0], [1.0; 4]);
    down.rect.local_rotation = crate::ui_runtime::Quaternion {
        x: 0.0,
        y: 0.0,
        z: 1.0,
        w: 0.0,
    };
    root.children.push(up);
    root.children.push(down);
}

/// The bar at the bottom of an open card (`CloseButton`) or under a closed one (`OpenButton`): a framed bar with a 35 x 17 arrow, turned over
/// while the card is closed.
fn bar(ctx: &mut Ctx<'_>, root: &mut Node, base: &str, (x0, x1): (f32, f32), top: f32, open: bool, action: UiAction) {
    let name = if open { "CloseButton" } else { "OpenButton" };
    root.children.push(sprite_node(&format!("{base}/{name}"), "base_icon", [x0, top, x1, top + BAR_H], [1.0; 4]));
    let centre = (x0 + x1) * 0.5;
    let mut arrow = sprite_node(&format!("{base}/{name}/icon"), "Arrow", [centre - 17.5, top + 1.5, centre + 17.5, top + 18.5], [1.0; 4]);
    if !open {
        arrow.rect.local_rotation = crate::ui_runtime::Quaternion {
            x: 0.0,
            y: 0.0,
            z: 1.0,
            w: 0.0,
        };
    }
    root.children.push(arrow);
    ctx.hits.push(DynHit {
        rect: [x0, top, x1 - x0, BAR_H],
        action,
    });
}

/// The name of a card: the text, or the field being typed into.
fn name_text(lists: &ListsInput<'_>, reference: u16, name: &str) -> String {
    lists
        .name_edit
        .filter(|(edited, _)| *edited == reference)
        .map_or_else(|| name.to_owned(), |(_, text)| crate::text_field::with_caret(text))
}

/// The height of an open zone card: its header, the edit section and the bar.
pub(super) fn open_zone_height(width: f32, zone: &ZoneSettings) -> f32 {
    HEADER_H + zone_cards::zone_section(width, zone).height + BAR_H
}

/// An open zone card of the world's Zones list (`ZoneSettingsHandle` with its edit section open): the header with the move arrows, the name and
/// the delete button, the two columns of controls and the bar that closes it. Returns its height.
pub(super) fn paint_open_zone(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, index: usize, cursor: f32) -> f32 {
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let zone = &lists.zones[index];
    let section = zone_cards::zone_section(x1 - x0, zone);
    let height = HEADER_H + section.height + BAR_H;
    if !overlaps(ctx, cursor, height) {
        return height;
    }
    let base = format!("UI/ScenarioEditor/Items/0/Item{index}/ZoneHandle(Clone)");
    let (view_top, view_bottom) = (ctx.view_top, ctx.view_bottom);
    let mut root = sprite_node(&base, "base_icon", [x0, cursor.max(view_top), x1, (cursor + height).min(view_bottom)], [1.0; 4]);
    let reference = index as u16;
    if ctx.visible(cursor, HEADER_H) {
        mover(ctx, lists, 0, index, lists.zones.len(), &mut root, &base, x0, cursor);
        root.children.push(text_node(
            &format!("{base}/Header/zoneName/line"),
            &name_text(lists, reference, &zone.name),
            26.0,
            [x0 + 49.0, cursor + 9.8, x1 - 60.0, cursor + 40.27],
        ));
        ctx.hits.push(DynHit {
            rect: [x0 + 49.0, cursor + 9.8, x1 - 60.0 - x0 - 49.0, 30.5],
            action: UiAction::ZoneNameEdit(reference),
        });
        delete_button(ctx, &mut root, &base, x1, cursor, (lists.delete_item)(0, index as u16));
    }
    let binding = Binding {
        index: reference,
        zone,
        all: lists.all_zones,
        template: false,
        sim_size: lists.sim_size,
    };
    paint_section(ctx, &mut root, &base, binding, (x0, cursor + HEADER_H), &section);
    let bar_top = cursor + height - BAR_H;
    if ctx.visible(bar_top, BAR_H) {
        bar(ctx, &mut root, &base, (x0, x1), bar_top, true, UiAction::ZoneOpen(0, index as u16));
    }
    ctx.add(root);
    height
}

/// The height of a zone group card.
fn group_height(width: f32, open: bool, group: &bibites_core::settings::ZoneGroupSettings) -> f32 {
    if open {
        zone_cards::group_card(width, zone_cards::template_section(width - 20.0, &group.template).height).height
    } else {
        CLOSED_HEADER_H + BAR_H
    }
}

/// The text of the count or density slider and where its handle sits.
fn group_slider(group: &bibites_core::settings::ZoneGroupSettings, field: u8) -> (SliderSpec<'static>, &'static str) {
    let info = ze::group_slider_info(field).expect("a group slider");
    let value = ze::group_value_of(group, field);
    let reference = FieldRef::Zone(ze::group_ref(0), field);
    let mut format = info.format();
    format.is_int = field == ze::GROUP_COUNT;
    (
        SliderSpec {
            name: if field == ze::GROUP_COUNT { "Zones Count" } else { "Zones Density" },
            text: format.format(value),
            fraction: fraction_range(info.mode, info.min, info.max, info.default, value),
            default_fraction: fraction_range(info.mode, info.min, info.max, info.default, info.default),
            field: reference,
            format,
            rules: info_rules(&info, field == ze::GROUP_COUNT),
        },
        if field == ze::GROUP_COUNT { "count" } else { "density" },
    )
}

/// The zone groups of the Zones tab (`ZoneGroupHandle`): a card each, 5 apart, open or closed on its own bar. An open card shows the group's
/// name with the move arrows and a delete button, whether the count scales with the simulation and the count (or the density), the template
/// zone's settings with a button that rolls the zones again, and a bar that closes it. Returns the height of the list.
pub(super) fn layout_groups(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, min_height: f32, y: f32) -> f32 {
    const GAP: f32 = 5.0;
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let width = x1 - x0;
    let mut cursor = y;
    for index in crate::dyn_panel::card_order(lists.order, 1, lists.groups.len()) {
        let group = &lists.groups[index];
        let open = lists.open.contains(&(1, index as u16));
        let height = group_height(width, open, group);
        if overlaps(ctx, cursor, height) {
            paint_group(ctx, lists, index, open, cursor, height);
        }
        cursor += height + GAP;
    }
    // (The list ends with 60 of padding.)
    (cursor - GAP - y + 60.0).max(min_height)
}

/// One group card at `cursor`.
fn paint_group(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, index: usize, open: bool, cursor: f32, height: f32) {
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let width = x1 - x0;
    let group = &lists.groups[index];
    let base = format!("UI/ScenarioEditor/Items/1/Item{index}/ZoneGroupHandle(Clone)");
    let (view_top, view_bottom) = (ctx.view_top, ctx.view_bottom);
    let mut root = sprite_node(&base, "base_icon", [x0, cursor.max(view_top), x1, (cursor + height).min(view_bottom)], [1.0; 4]);
    let name_ref = ze::group_ref(index);
    let toggle = UiAction::ZoneOpen(1, index as u16);
    if !open {
        // The name only, and the bar that opens the card.
        if ctx.visible(cursor, CLOSED_HEADER_H) {
            root.children.push(text_node(
                &format!("{base}/Header/groupName/line"),
                &group.name,
                26.0,
                [x0 + 10.0, cursor + 8.0, x1 - 10.0, cursor + 38.47],
            ));
        }
        let bar_top = cursor + CLOSED_HEADER_H;
        if ctx.visible(bar_top, BAR_H) {
            bar(ctx, &mut root, &base, (x0, x1), bar_top, false, toggle);
        }
        ctx.add(root);
        return;
    }
    let template = group.template.clone();
    let template_section = zone_cards::template_section(width - 20.0, &template);
    let card = zone_cards::group_card(width, template_section.height);
    if ctx.visible(cursor, zone_cards::HEADER_H) {
        mover(ctx, lists, 1, index, lists.groups.len(), &mut root, &base, x0, cursor);
        root.children.push(text_node(
            &format!("{base}/Header/groupName/line"),
            &name_text(lists, name_ref, &group.name),
            26.0,
            [x0 + 49.0, cursor + 9.8, x1 - 60.0, cursor + 40.27],
        ));
        ctx.hits.push(DynHit {
            rect: [x0 + 49.0, cursor + 9.8, x1 - 60.0 - x0 - 49.0, 30.5],
            action: UiAction::ZoneNameEdit(name_ref),
        });
        delete_button(ctx, &mut root, &base, x1, cursor, (lists.delete_item)(1, index as u16));
    }
    // "Count scales with sim" (left) and the count or the density (right).
    let line_y = cursor + card.params_y;
    if ctx.visible(line_y, ROW_H) {
        let [label_w, check_w] = zone_cards::toggle_parts(card.params_w);
        let left = x0 + card.params_x[0];
        let toggle_base = format!("{base}/Group Parameters/Line/LeftRow/countType");
        root.children.push(card_text(
            &format!("{toggle_base}/Name"),
            "Count scales with sim",
            20.0,
            [left + 10.0, line_y, left + label_w, line_y + ROW_H],
            "Left",
            INK,
        ));
        let bx = left + label_w + 15.0 + check_w * 0.5 - 15.0;
        let rect = [bx, line_y, bx + 30.0, line_y + ROW_H];
        root.children.push(sprite_node(&format!("{toggle_base}/Toggle/Background"), "base_icon", rect, [1.0; 4]));
        if group.scale_with_sim {
            root.children.push(sprite_node(&format!("{toggle_base}/Toggle/Background/Checkmark"), "checkmark_icon", rect, [1.0; 4]));
        }
        ctx.hits.push(DynHit {
            rect: [rect[0], rect[1], 30.0, ROW_H],
            action: UiAction::ZoneToggle(name_ref, ze::GROUP_SCALE),
        });
        ctx.tips.push(DynTip {
            rect: [left, line_y, label_w, ROW_H],
            tip: group_tip(ze::GROUP_SCALE),
        });
        let field = if group.scale_with_sim { ze::GROUP_DENSITY } else { ze::GROUP_COUNT };
        let (mut spec, name) = group_slider(group, field);
        spec.field = FieldRef::Zone(name_ref, field);
        let right = x0 + card.params_x[1];
        let [name_w, value_w, panel_w] = zone_cards::slider_parts(card.params_w);
        let geometry = CardSliderGeometry {
            name_w,
            value_x: name_w + 15.0,
            value_w,
            value_align: "Center",
            panel_x: name_w + 15.0 + value_w + 15.0,
            panel_w,
        };
        let editing = ctx.input.edit.filter(|edit| edit.field == spec.field);
        card_slider_row(&mut root, &mut ctx.hits, editing, &format!("{base}/Group Parameters/Line/RightRow/{name}"), (right, line_y), &geometry, &spec);
        ctx.tips.push(DynTip {
            rect: [right, line_y, name_w, ROW_H],
            tip: group_tip(field),
        });
    }
    // The template's caption, how many zones the group has and the button that rolls them again.
    let caption_y = cursor + card.template_line_y;
    if ctx.visible(caption_y, card.template_line_h) {
        let line_base = format!("{base}/Group Parameters/Template Section/Line");
        root.children.push(text_node(
            &format!("{line_base}/name"),
            "Template Zone Settings:",
            26.0,
            [x0 + 10.0, caption_y + 5.0, x0 + 293.4, caption_y + 35.5],
        ));
        let count = group.count(lists.sim_size);
        let mut count_text = card_text(
            &format!("{line_base}/FloatInfoText"),
            &format!("count: {count}"),
            26.0,
            [x1 - 170.2, caption_y + 5.0, x1 - 60.0, caption_y + 35.5],
            "Right",
            INK,
        );
        for component in &mut count_text.texts {
            component.font = Some(crate::ui_runtime::FontRef {
                name: "Roboto-Bold SDF NoOutline".to_owned(),
                ..crate::ui_runtime::FontRef::default()
            });
        }
        root.children.push(count_text);
        let reroll = [x1 - 50.0, caption_y, x1 - 10.0, caption_y + 40.0];
        root.children.push(sprite_node(&format!("{line_base}/RerollButton"), "base_icon", reroll, [1.0; 4]));
        root.children.push(sprite_node(&format!("{line_base}/RerollButton/icon"), "reset_icon", reroll, [1.0; 4]));
        ctx.hits.push(DynHit {
            rect: [reroll[0], reroll[1], 40.0, 40.0],
            action: UiAction::Invoke(crate::ui::ControlCommand::RerollZoneGroup(index as u16)),
        });
    }
    // The template zone: a box with the same controls as a zone's, without a name or a place.
    let template_y = cursor + card.template_y;
    let template_base = format!("{base}/Group Parameters/Template Section/ZoneHandle");
    if overlaps(ctx, template_y, template_section.height) {
        root.children.push(sprite_node(
            &template_base,
            "base_icon",
            [x0 + 10.0, template_y.max(view_top), x1 - 10.0, (template_y + template_section.height).min(view_bottom)],
            [1.0; 4],
        ));
        let binding = Binding {
            index: ze::template_ref(index),
            zone: &template,
            all: lists.all_zones,
            template: true,
            sim_size: lists.sim_size,
        };
        paint_section(ctx, &mut root, &template_base, binding, (x0 + 10.0, template_y), &template_section);
    }
    let bar_top = cursor + card.bar_y;
    if ctx.visible(bar_top, BAR_H) {
        bar(ctx, &mut root, &base, (x0, x1), bar_top, true, toggle);
    }
    ctx.add(root);
}

/// The tooltip of a group setting (`ZoneGroupSettings`: its name and help text).
fn group_tip(field: u8) -> crate::tooltip::Tip {
    let (title, content) = match field {
        ze::GROUP_SCALE => (
            "Count scales with sim",
            "Is the number of random zones fixed or should it be proportional to the simulation area (bigger sim => more zones)?",
        ),
        ze::GROUP_COUNT => ("Zones Count", "How many Zones should be generated on startup?"),
        _ => ("Zones Density", "How many Zones should be generated on startup?"),
    };
    crate::tooltip::Tip {
        path: "",
        title,
        content,
        ui: true,
        instant: false,
    }
}
