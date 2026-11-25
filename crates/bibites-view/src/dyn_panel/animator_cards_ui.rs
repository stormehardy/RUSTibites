//! The settings animators of the Animation tab (`SettingsChangerHandle`, `SettingsChangerCreator`, `EditableLineGraph`): the cards, the graph of
//! the curve and the creator panel. The geometry is read from the live game (a card's header 42, its "Settings Bases" box, the Start Time and
//! Repeat rows, a 400 high graph; the creator is a 605 wide panel at the left of the tab).

use super::{
    CardSliderGeometry, Ctx, DynHit, DynTip, FieldRef, ListsInput, Node, SliderSpec, UiAction, card_slider_row, card_text, field_image,
    fraction_range, info_rules, sprite_node, text_node,
};
use crate::animators::{self, Animatable, CreatorField, GraphView, HOURS};
use crate::zone_cards::{BAR_H, CLOSED_HEADER_H, HEADER_H, ROW_H};
use crate::zone_editor as ze;
use bibites_core::format::FloatValueFormat;
use bibites_core::settings::SettingsChanger;

const INK: f32 = 0.164_705_88;
/// Height of a base item (`SettingBaseItem`: a 30 high slider row and 5 of room).
const BASE_H: f32 = 37.0;
/// Height of the graph.
const GRAPH_H: f32 = 400.0;
/// What the card spends on the Start Time row, the Repeat row and the 5 between them and the graph.
const CURVE_H: f32 = 40.0 + 5.0 + 30.0 + 5.0 + GRAPH_H;

/// Height of the box that holds the bases: 65 without any, and a base item more for each.
fn bases_height(changer: &SettingsChanger) -> f32 {
    65.0 + changer.settings_bases.len() as f32 * BASE_H
}

/// Height of a card: the closed one is its header and the bar; the open one adds the edit section (10 of padding, the 30.47 high title, the
/// bases, the curve section, 5 at the bottom, 10 between).
fn card_height(open: bool, changer: &SettingsChanger) -> f32 {
    if open {
        HEADER_H + 10.0 + 30.47 + 10.0 + bases_height(changer) + 10.0 + CURVE_H + 5.0 + BAR_H
    } else {
        CLOSED_HEADER_H + BAR_H
    }
}

/// The animators of the Animation tab: a card each, 5 apart, open or closed on its own bar. Returns the height of the list.
pub(super) fn layout_animators(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, min_height: f32, y: f32) -> f32 {
    const GAP: f32 = 5.0;
    let mut cursor = y;
    for index in crate::dyn_panel::card_order(lists.order, 3, lists.changers.len()) {
        let changer = &lists.changers[index];
        let open = lists.open.contains(&(3, index as u16));
        let height = card_height(open, changer);
        if !ctx.dry && cursor + height > ctx.view_top && cursor < ctx.view_bottom {
            paint_card(ctx, lists, index, open, cursor, height);
        }
        cursor += height + GAP;
    }
    // (The list ends with 60 of padding.)
    (cursor - GAP - y + 60.0).max(min_height)
}

/// One animator's card.
fn paint_card(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, index: usize, open: bool, cursor: f32, height: f32) {
    let (x0, x1) = (ctx.geometry.content_x[0], ctx.geometry.content_x[1]);
    let width = x1 - x0;
    let changer = &lists.changers[index];
    let base = format!("DynamicSettingsPanel/AnimationsPanel/Animator List/ChangerHandle(Clone)#{index}");
    let (view_top, view_bottom) = (ctx.view_top, ctx.view_bottom);
    let mut root = sprite_node(&base, "base_icon", [x0, cursor.max(view_top), x1, (cursor + height).min(view_bottom)], [1.0; 4]);
    let reference = ze::animator_ref(index);
    let toggle = UiAction::ZoneOpen(3, index as u16);
    let typing = lists.name_edit.filter(|(edited, _)| *edited == reference).map(|(_, text)| text);
    if !open {
        if ctx.visible(cursor, CLOSED_HEADER_H) {
            root.children.push(text_node(&format!("{base}/Header/nameEdit/line"), &changer.name, 26.0, [x0 + 10.0, cursor + 8.0, x1 - 10.0, cursor + 38.47]));
        }
        let bar_top = cursor + CLOSED_HEADER_H;
        if ctx.visible(bar_top, BAR_H) {
            bar(ctx, &mut root, &base, (x0, x1), bar_top, false, toggle);
        }
        ctx.add(root);
        return;
    }
    // The header: the move arrows, the name (a click types a new one) and the delete button.
    if ctx.visible(cursor, HEADER_H) {
        super::zone_cards_ui::mover(ctx, lists, 3, index, lists.changers.len(), &mut root, &base, x0, cursor);
        let shown = typing.map_or_else(|| changer.name.clone(), crate::text_field::with_caret);
        root.children.push(text_node(&format!("{base}/Header/nameEdit/line"), &shown, 26.0, [x0 + 49.0, cursor + 9.8, x1 - 60.0, cursor + 40.27]));
        ctx.hits.push(DynHit {
            rect: [x0 + 49.0, cursor + 9.8, x1 - 60.0 - x0 - 49.0, 30.5],
            action: UiAction::ZoneNameEdit(reference),
        });
        let (cx, cy) = (x1 - 25.0, cursor + 25.0);
        root.children.push(sprite_node(&format!("{base}/Header/Button (1)/Text (TMP)"), "exit_icon", [cx - 25.0, cy - 25.0, cx + 25.0, cy + 25.0], [1.0; 4]));
        ctx.hits.push(DynHit {
            rect: [x1 - 40.0, cursor + 10.0, 30.0, 30.0],
            action: UiAction::Invoke(crate::ui::ControlCommand::DeleteAnimator(index as u16)),
        });
    }
    let edit_top = cursor + HEADER_H + 10.0;
    // "Settings Bases".
    if ctx.visible(edit_top, 30.47) {
        root.children.push(card_text(
            &format!("{base}/Edit Section/title"),
            "Settings Bases",
            26.0,
            [x0 + 10.0, edit_top, x1 - 10.0, edit_top + 30.47],
            "Center",
            INK,
        ));
    }
    // The bases: a framed box with a row for each animated setting and the "Add Setting" dropdown under them.
    let box_top = edit_top + 30.47 + 10.0;
    let box_h = bases_height(changer);
    let box_visible = !ctx.dry && box_top + box_h > ctx.view_top && box_top < ctx.view_bottom;
    if box_visible {
        root.children.push(sprite_node(
            &format!("{base}/Edit Section/BasesSection"),
            "base_icon",
            [x0 + 10.0, box_top.max(view_top), x1 - 10.0, (box_top + box_h).min(view_bottom)],
            [1.0; 4],
        ));
    }
    let item_x = (x0 + 25.0, x1 - 25.0);
    for (slot, animated) in changer.settings_bases.iter().enumerate() {
        let item_top = box_top + 10.0 + slot as f32 * BASE_H;
        // The 30 high slider sits in the middle of the item's 35 (2 of padding under it for the liner).
        let row_y = item_top + 2.5;
        if !ctx.visible(item_top, BASE_H) {
            continue;
        }
        let entry = lists.catalog.iter().find(|entry| entry.key == animated.key);
        let name = entry.map_or_else(|| animated.key.clone(), |entry| entry.name.clone());
        let item_base = format!("{base}/Edit Section/BasesSection/basesHolder/SettingBaseItem(Clone)#{slot}");
        let field = ze::ANIM_BASE_FIRST + slot as u8;
        let (info, format) = match entry {
            Some(entry) => (animators::base_slider_info(entry), entry.format.clone()),
            None => {
                let fallback = Animatable {
                    key: animated.key.clone(),
                    name: name.clone(),
                    min: 0.0,
                    max: (animated.base.abs() * 10.0).max(1.0),
                    default: animated.base,
                    format: FloatValueFormat::default(),
                };
                (animators::base_slider_info(&fallback), fallback.format)
            }
        };
        let mut format = format;
        format.precision_is_si = false;
        let slider = SliderSpec {
            name: &name,
            text: format.format(animated.base),
            fraction: fraction_range(info.mode, info.min, info.max, info.default, animated.base),
            default_fraction: fraction_range(info.mode, info.min, info.max, info.default, info.default),
            field: FieldRef::Zone(reference, field),
            format,
            rules: info_rules(&info, false),
        };
        // The item (`SettingBaseItem`, read from the live game) is 876 x 37: a 816 wide slider row (the name 228.67 wide, the slider 407.33, the
        // value text 150 on the right, 15 apart) and the 35 x 35 cross, with a 2 high grey liner under them. (The row helper starts its name 10 in.)
        // (The widths follow Unity's layout: the name and the slider share what the value text and the spacing leave, 1 : 2 over their preferred
        // widths 125 and 200; a wider card than the world's 926 widens them.)
        let item_w = item_x.1 - item_x.0;
        let slider_w = item_w - 60.0;
        let extra = slider_w - 30.0 - 150.0 - 325.0;
        let (name_w, panel_w) = (125.0 + extra / 3.0, 200.0 + extra * 2.0 / 3.0);
        let origin = item_x.0 - 10.0;
        let geometry = CardSliderGeometry {
            name_w: name_w + 10.0,
            value_x: 10.0 + name_w + 15.0 + panel_w + 15.0,
            value_w: 150.0,
            value_align: "Center",
            panel_x: 10.0 + name_w + 15.0,
            panel_w,
        };
        let editing = ctx.input.edit.filter(|edit| edit.field == slider.field);
        card_slider_row(&mut root, &mut ctx.hits, editing, &item_base, (origin, row_y), &geometry, &slider);
        let cross = [item_x.1 - 35.0, item_top, item_x.1, item_top + 35.0];
        root.children.push(sprite_node(&format!("{item_base}/RemoveButton"), "base_icon", cross, [1.0, 1.0, 1.0, 0.0]));
        root.children.push(sprite_node(&format!("{item_base}/RemoveButton/icon"), "exit_icon", cross, [1.0; 4]));
        root.children.push(super::field_image(&format!("{item_base}/liner"), None, [item_x.0, item_top + 35.0, item_x.1, item_top + 37.0], [0.433_962, 0.433_962, 0.433_962, 1.0]));
        ctx.hits.push(DynHit {
            rect: [cross[0], cross[1], 35.0, 35.0],
            action: UiAction::Invoke(crate::ui::ControlCommand::RemoveAnimatorBase(index as u16, slot as u16)),
        });
    }
    let add_y = box_top + 10.0 + changer.settings_bases.len() as f32 * BASE_H + 10.0;
    if ctx.visible(add_y, ROW_H) {
        let add_base = format!("{base}/Edit Section/BasesSection/AddLine/addDropdown");
        // `AddLine` (876 x 30): the name 150 wide, then the 460 wide dropdown 15 on.
        root.children.push(card_text(&format!("{add_base}/Name"), "Add Setting:", 20.0, [item_x.0, add_y, item_x.0 + 150.0, add_y + ROW_H], "Left", INK));
        let dx = item_x.0 + 165.0;
        let dw = 460.0;
        root.children.push(sprite_node(&format!("{add_base}/Dropdown"), "UISprite", [dx, add_y, dx + dw, add_y + ROW_H], [1.0; 4]));
        root.children.push(card_text(&format!("{add_base}/Dropdown/Label"), "Select Settings", 16.0, [dx + 10.0, add_y + 7.0, dx + dw - 25.0, add_y + 24.0], "Left", 0.196_078_43));
        root.children.push(sprite_node(&format!("{add_base}/Dropdown/Arrow"), "DropdownArrow", [dx + dw - 35.0, add_y, dx + dw - 5.0, add_y + ROW_H], [1.0; 4]));
        ctx.hits.push(DynHit {
            rect: [dx, add_y, dw, ROW_H],
            action: UiAction::DropdownOpen(crate::ui::DropdownTarget::AnimatorBase(index as u16)),
        });
    }
    // The curve section: the start time (40 high, with its reset button), the repeat toggle and the graph.
    let curve_top = box_top + box_h + 10.0;
    let row_w = width - 20.0;
    if ctx.visible(curve_top, 40.0) {
        let start = animators::creator_field(animators::START);
        let info = animators::creator_slider_info(&start);
        let start_ref = FieldRef::Zone(reference, ze::ANIM_START);
        let spec = SliderSpec {
            name: "Start Time",
            text: start.format().format(changer.start),
            fraction: fraction_range(info.mode, info.min, info.max, info.default, changer.start),
            default_fraction: fraction_range(info.mode, info.min, info.max, info.default, info.default),
            field: start_ref,
            format: start.format(),
            rules: info_rules(&info, false),
        };
        let name_w = row_w * 0.373;
        let value_w = row_w * 0.206;
        let panel_x = name_w + 15.0 + value_w + 15.0;
        let geometry = CardSliderGeometry {
            name_w,
            value_x: name_w + 15.0,
            value_w,
            value_align: "Center",
            panel_x,
            panel_w: row_w - panel_x - 55.0,
        };
        let editing = ctx.input.edit.filter(|edit| edit.field == start_ref);
        let origin = (x0 + 10.0, curve_top + 5.0);
        card_slider_row(&mut root, &mut ctx.hits, editing, &format!("{base}/Edit Section/CurveSection/SettingSlider"), origin, &geometry, &spec);
        let reset = [x1 - 10.0 - 40.0, curve_top, x1 - 10.0, curve_top + 40.0];
        root.children.push(sprite_node(&format!("{base}/Edit Section/CurveSection/SettingSlider/NormalSecton/ResetButton"), "base_icon", reset, [1.0; 4]));
        root.children.push(sprite_node(
            &format!("{base}/Edit Section/CurveSection/SettingSlider/NormalSecton/ResetButton/buttonIcon"),
            "reset_icon",
            [reset[0] + 2.0, reset[1] + 2.0, reset[2] - 2.0, reset[3] - 2.0],
            [1.0; 4],
        ));
        ctx.hits.push(DynHit {
            rect: [reset[0], reset[1], 40.0, 40.0],
            action: UiAction::ZoneReset(reference, ze::ANIM_START),
        });
        ctx.tips.push(DynTip {
            rect: [origin.0, curve_top, name_w, 40.0],
            tip: animator_tip("Start Time", "Time at which the animators will start."),
        });
    }
    let repeat_y = curve_top + 45.0;
    if ctx.visible(repeat_y, ROW_H) {
        let repeat_base = format!("{base}/Edit Section/CurveSection/basicToggle");
        root.children.push(card_text(&format!("{repeat_base}/Name"), "Repeat", 20.0, [x0 + 20.0, repeat_y, x0 + 300.0, repeat_y + ROW_H], "Left", INK));
        let bx = x1 - 10.0 - 50.0 - 15.0;
        let rect = [bx, repeat_y, bx + 30.0, repeat_y + ROW_H];
        root.children.push(sprite_node(&format!("{repeat_base}/Toggle/Background"), "base_icon", rect, [1.0; 4]));
        if changer.repeat {
            root.children.push(sprite_node(&format!("{repeat_base}/Toggle/Background/Checkmark"), "checkmark_icon", rect, [1.0; 4]));
        }
        ctx.hits.push(DynHit {
            rect: [rect[0], rect[1], 30.0, ROW_H],
            action: UiAction::ZoneToggle(reference, ze::ANIM_REPEAT),
        });
        ctx.tips.push(DynTip {
            rect: [x0 + 10.0, repeat_y, 200.0, ROW_H],
            tip: animator_tip(
                "Repeat",
                "If enabled, the change will repeat forever.\nIf Setting Animator is non-repeat, it will be deleted when the last point is reached.",
            ),
        });
    }
    let graph_top = repeat_y + 30.0 + 5.0;
    if !ctx.dry && graph_top + GRAPH_H > ctx.view_top && graph_top < ctx.view_bottom {
        paint_graph(ctx, lists, &mut root, &base, index, [x0 + 10.0, graph_top, row_w, GRAPH_H]);
    }
    let bar_top = cursor + height - BAR_H;
    if ctx.visible(bar_top, BAR_H) {
        bar(ctx, &mut root, &base, (x0, x1), bar_top, true, toggle);
    }
    ctx.add(root);
}

/// A tooltip with the game's own title and text.
fn animator_tip(title: &'static str, content: &'static str) -> crate::tooltip::Tip {
    crate::tooltip::Tip {
        path: "",
        title,
        content,
        ui: true,
        instant: false,
    }
}

/// The bar under (or at the bottom of) a card.
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

/// The graph of a curve (`EditableLineGraph`): `rect` is `[x, y, width, height]` of the 400 high graph.
fn paint_graph(ctx: &mut Ctx<'_>, lists: &ListsInput<'_>, root: &mut Node, base: &str, index: usize, rect: [f32; 4]) {
    let changer = &lists.changers[index];
    let [gx, gy, gw, gh] = rect;
    let path = format!("{base}/Edit Section/CurveSection/EditableGraph/GraphBody");
    root.children.push(field_image(&format!("{path}/BackGround"), None, [gx, gy, gx + gw, gy + gh], [0.156_863, 0.156_863, 0.156_863, 1.0]));
    // The plot: 50 from the left for the value labels, 25 from the right, under the 38.1 of the title; 320.14 high.
    let (px0, py0) = (gx + 50.0, gy + 39.1);
    let (pw, ph) = (gw - 75.0, 320.14);
    // The frame the app keeps while the curve is edited (`EditableLineGraph.min` / `max`), or the curve's own.
    let view = lists
        .graph_views
        .get(index)
        .copied()
        .flatten()
        .unwrap_or_else(|| animators::graph_view(&changer.points));
    let white = [1.0, 1.0, 1.0, 1.0];
    let mut title = card_text(&format!("{path}/Title section/Title"), "Animation Curve", 18.0, [gx, gy + 7.0, gx + gw, gy + 29.0], "Center", 1.0);
    title.texts.iter_mut().for_each(|text| text.color = Some(crate::ui_runtime::Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }));
    root.children.push(title);
    // The grid: 21 vertical and 11 horizontal lines, with the labels.
    let grid = [0.0, 0.0, 0.0, 1.0];
    let label_format = FloatValueFormat {
        prefix: "",
        units: "",
        precision: 2,
        factor: 1.0,
        si: false,
        always_show_sign: false,
        precision_is_si: true,
        is_int: false,
    };
    for k in 0..animators::GRAPH_NX {
        let x = px0 + k as f32 * pw / (animators::GRAPH_NX - 1) as f32;
        root.children.push(field_image(&format!("{path}/Y+Graph/Graph/xLine{k}"), None, [x - 1.0, py0, x + 1.0, py0 + ph], grid));
        let value = (view.min[0] + k as f32 * view.x_step) * HOURS;
        let mut label = card_text(&format!("{path}/xAxis/xHolder/xLine(Clone){k}/FloatInfoText"), &label_format.format(value), 12.0, [x - 25.0, py0 + ph + 2.0, x + 25.0, py0 + ph + 17.0], "Center", 1.0);
        label.texts.iter_mut().for_each(|text| text.color = Some(crate::ui_runtime::Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }));
        root.children.push(label);
    }
    for k in 0..animators::GRAPH_NY {
        let y = py0 + ph - k as f32 * ph / (animators::GRAPH_NY - 1) as f32;
        root.children.push(field_image(&format!("{path}/Y+Graph/Graph/yLine{k}"), None, [px0, y - 1.0, px0 + pw, y + 1.0], grid));
        let value = view.min[1] + k as f32 * view.y_step;
        let mut label = card_text(&format!("{path}/Y+Graph/yAxis/yHolder/yLine(Clone){k}/FloatInfoText"), &label_format.format(value), 11.05, [gx, y - 7.0, gx + 48.0, y + 7.0], "Center", 1.0);
        label.texts.iter_mut().for_each(|text| text.color = Some(crate::ui_runtime::Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }));
        root.children.push(label);
    }
    let mut unit = card_text(&format!("{path}/xLabelSection/xAxisLabel"), "time (h)", 16.0, [gx, gy + 378.0, gx + gw - 25.0, gy + 399.0], "Center", 1.0);
    unit.texts.iter_mut().for_each(|text| text.color = Some(crate::ui_runtime::Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }));
    root.children.push(unit);
    // The curve: its value at every second column of the plot, drawn as a line two wide.
    let columns = (pw / 2.0).floor() as usize;
    let at = |cx: usize| -> f32 {
        let time = view.min[0] + (cx as f32 * 2.0) / pw * (view.max[0] - view.min[0]);
        let value = changer.value_at(time);
        py0 + ph - ((value - view.min[1]) / (view.max[1] - view.min[1])).clamp(-0.05, 1.05) * ph
    };
    let mut previous = at(0);
    for cx in 1..=columns {
        let y = at(cx);
        let (top, bottom) = (previous.min(y) - 1.0, previous.max(y) + 1.0);
        let x = px0 + cx as f32 * 2.0;
        if top < py0 + ph + 2.0 && bottom > py0 - 2.0 {
            root.children.push(field_image(&format!("{path}/Y+Graph/Graph/linesHolder/Line{cx}"), None, [x - 2.0, top.max(py0 - 2.0), x, bottom.min(py0 + ph + 2.0)], white));
        }
        previous = y;
    }
    // The progress marker (`SetCurrentProgress`): a dashed line at the time the world is at, while a simulation runs.
    if let Some(now) = lists.sim_time {
        let mut time = now - changer.start;
        if time >= 0.0 {
            let last = changer.points.last().map_or(0.0, |point| point.x);
            if changer.repeat && last > 0.0 && time > last {
                time %= last;
            }
            let x = px0 + (time - view.min[0]) / (view.max[0] - view.min[0]) * pw;
            if x >= px0 && x <= px0 + pw {
                let mut y = py0;
                while y < py0 + ph {
                    root.children.push(field_image(&format!("{path}/Y+Graph/Graph/indicatorLine{}", y as i32), None, [x - 1.0, y, x + 1.0, (y + 4.0).min(py0 + ph)], white));
                    y += 8.0;
                }
            }
        }
    }
    // The points: a 15 x 15 white disc each (the selected one is bigger).
    for (slot, point) in changer.points.iter().enumerate() {
        let at = view.to_rect([point.x, point.y], [pw, ph]);
        let (cx, cy) = (px0 + at[0], py0 + ph - at[1]);
        let selected = lists.selected_point == Some((index, slot));
        let radius = if selected { 9.0 } else { 7.5 };
        let tint = if selected { [0.65, 0.81, 1.0, 1.0] } else { white };
        root.children.push(sprite_node(
            &format!("{path}/Y+Graph/Graph/SplineHandlesHolder/SplinePointHandle(Clone){slot}"),
            "base_circle_icon",
            [cx - radius, cy - radius, cx + radius, cy + radius],
            tint,
        ));
    }
    // The graph's one hit region: a press picks the point under it (or, with Ctrl, adds one or removes the picked one) and drags it.
    ctx.hits.push(DynHit {
        rect: [px0 - 10.0, py0 - 10.0, pw + 20.0, ph + 20.0],
        action: UiAction::AnimGraph(index as u16, px0.round() as u16, py0.round() as u16, (px0 + pw).round() as u16, (py0 + ph).round() as u16),
    });
    // The selected point's panel (`PointEditor`).
    if let Some((selected, slot)) = lists.selected_point
        && selected == index
        && slot < changer.points.len()
    {
        paint_point_editor(ctx, root, &path, index, slot, changer, &view, [px0, py0, pw, ph]);
    }
}

/// The panel of the selected point (`PointEditor`, read from the live game's dump: 280 wide, 15 of padding, 4 between its 25 high rows): its time and
/// value (a click types a new one), the interpolation and, unless it is linear, the easing; a delete button for every point but the first. It is a
/// `FollowLikeTooltip`: it hangs 10 from the point, to its left when the point is in the right half of the screen (to its right otherwise) and below it
/// when the point is in the upper half (above it otherwise).
#[allow(clippy::too_many_arguments)]
fn paint_point_editor(
    ctx: &mut Ctx<'_>,
    root: &mut Node,
    path: &str,
    index: usize,
    slot: usize,
    changer: &SettingsChanger,
    view: &GraphView,
    plot: [f32; 4],
) {
    use bibites_core::settings::InterpolationType;
    let [px0, py0, pw, ph] = plot;
    let point = changer.points[slot];
    let linear = point.interpolation == InterpolationType::Linear;
    let height = if linear { 113.0 } else { 142.0 };
    let at = view.to_rect([point.x, point.y], [pw, ph]);
    let (cx, cy) = (px0 + at[0], py0 + ph - at[1]);
    let left = if cx > 960.0 { cx - 10.0 - 280.0 } else { cx + 10.0 };
    let top = if cy < 540.0 { cy + 10.0 } else { cy - 10.0 - height };
    let base = format!("{path}/Y+Graph/Graph/PointEditor");
    root.children.push(sprite_node(&base, "base_icon", [left, top, left + 280.0, top + height], [1.0; 4]));
    // The panel swallows the presses on it (its controls come after).
    ctx.hits.push(DynHit {
        rect: [left, top, 280.0, height],
        action: UiAction::Consume,
    });
    let reference = ze::animator_ref(index);
    let (x, mut row_y) = (left + 15.0, top + 15.0);
    let rows: [(&str, String, u8); 2] = [
        ("Time", FloatValueFormat { prefix: "", units: "h", precision: 2, factor: HOURS, si: false, always_show_sign: false, precision_is_si: false, is_int: false }.format(point.x), ze::ANIM_POINT_TIME),
        ("Value", FloatValueFormat { prefix: "", units: "", precision: 2, factor: 1.0, si: false, always_show_sign: false, precision_is_si: false, is_int: false }.format(point.y), ze::ANIM_POINT_VALUE),
    ];
    // `simpleTextLine`: the name 95 wide, 5 apart, the value text taking the rest (150).
    for (label, text, field) in rows {
        root.children.push(card_text(&format!("{base}/{label}/name"), label, 18.0, [x, row_y, x + 95.0, row_y + 25.0], "Left", INK));
        let shown = ctx
            .input
            .edit
            .filter(|edit| edit.field == FieldRef::Zone(reference, field))
            .map_or(text, |edit| crate::text_field::with_caret(&edit.text));
        root.children.push(card_text(&format!("{base}/{label}/line"), &shown, 18.0, [x + 100.0, row_y, x + 250.0, row_y + 25.0], "Left", INK));
        ctx.hits.push(DynHit {
            rect: [x + 100.0, row_y, 150.0, 25.0],
            action: UiAction::DynEditValue(FieldRef::Zone(reference, field)),
        });
        row_y += 29.0;
    }
    // `simpleDropdown`: "Interp." 92 wide and the 158 wide dropdown (the label 16 pt inset 10 / 25, the 20 x 20 arrow 5 from the end).
    root.children.push(card_text(&format!("{base}/simpleDropdown/Title"), "Interp.", 18.0, [x, row_y, x + 92.0, row_y + 25.0], "Left", INK));
    let choice = animators::INTERPOLATIONS.iter().find(|(mode, _)| *mode == point.interpolation).map_or("Linear", |(_, label)| *label);
    let dx = x + 92.0;
    root.children.push(sprite_node(&format!("{base}/simpleDropdown/Dropdown"), "UISprite", [dx, row_y, dx + 158.0, row_y + 25.0], [1.0; 4]));
    root.children.push(card_text(&format!("{base}/simpleDropdown/Dropdown/Label"), choice, 16.0, [dx + 10.0, row_y + 3.0, dx + 133.0, row_y + 22.0], "Left", 0.196_078_43));
    root.children.push(sprite_node(&format!("{base}/simpleDropdown/Dropdown/Arrow"), "DropdownArrow", [dx + 133.0, row_y + 2.5, dx + 153.0, row_y + 22.5], [1.0; 4]));
    ctx.hits.push(DynHit {
        rect: [dx, row_y, 158.0, 25.0],
        action: UiAction::DropdownOpen(crate::ui::DropdownTarget::AnimatorInterpolation(index as u16)),
    });
    row_y += 29.0;
    // `simpleSlider` (only for an interpolation that eases): "Easing" 90 wide, a 100 wide slider panel (its slider 5 in from both ends, a 20 x 20
    // handle), and the value text 50 wide at the right.
    if !linear {
        root.children.push(card_text(&format!("{base}/simpleSlider/Name"), "Easing", 18.0, [x, row_y, x + 90.0, row_y + 25.0], "Left", INK));
        let panel_x = x + 95.0;
        let track = (panel_x + 5.0, panel_x + 95.0);
        let fraction = ((point.ease_factor - 1.0) / 9.0).clamp(0.0, 1.0);
        let centre = track.0 + fraction * (track.1 - track.0);
        root.children.push(sprite_node(&format!("{base}/simpleSlider/NormalSecton/SliderPanel/Slider/Background"), "Background", [track.0, row_y + 5.0, track.1 + 5.0, row_y + 20.0], [1.0, 1.0, 1.0, 0.254_901_97]));
        root.children.push(sprite_node(&format!("{base}/simpleSlider/NormalSecton/SliderPanel/Slider/Fill Area/Fill"), "UISprite", [track.0 - 5.0, row_y + 5.0, centre + 5.0 + 5.0 * fraction, row_y + 20.0], [1.0; 4]));
        root.children.push(sprite_node(&format!("{base}/simpleSlider/NormalSecton/SliderPanel/Slider/Handle Slide Area/Handle"), "base_circle_icon", [centre - 10.0, row_y + 2.5, centre + 10.0, row_y + 22.5], [1.0; 4]));
        root.children.push(card_text(&format!("{base}/simpleSlider/FloatInfoText"), &format!("{:.1}", point.ease_factor), 18.0, [x + 200.0, row_y, x + 250.0, row_y + 25.0], "Right", INK));
        ctx.hits.push(DynHit {
            rect: [panel_x, row_y, 100.0, 25.0],
            action: UiAction::ZoneSlide(reference, ze::ANIM_POINT_EASE, track.0.round() as u16, track.1.round() as u16),
        });
    }
    // The delete button (25 x 25, 5 in from the top right corner; not for the first point, nor when only two are left).
    if slot > 0 && changer.points.len() > 2 {
        let rect = [left + 250.0, top + 5.0, left + 275.0, top + 30.0];
        root.children.push(sprite_node(&format!("{base}/DeleteButton"), "base_icon_thin", rect, [1.0; 4]));
        root.children.push(sprite_node(&format!("{base}/DeleteButton/icon"), "Delete_0", rect, [1.0; 4]));
        ctx.hits.push(DynHit {
            rect: [rect[0], rect[1], 25.0, 25.0],
            action: UiAction::Invoke(crate::ui::ControlCommand::RemoveAnimatorPoint(index as u16, slot as u16)),
        });
    }
}

/// Where an animator creator hangs and what its scene objects are called. The world's settings panel and the scenario editor's Animation tab each
/// hold a copy of the prefab under their `Animator Header/Panel Holder`; the copy hangs from the header's left edge, 3 in and one half header down
/// (the world's, read from the live game: header at (951, 225), creator at (954, 248)).
#[derive(Clone, Copy, Debug)]
pub struct CreatorPlace {
    /// The top left corner of the panel (reference px).
    pub origin: [f32; 2],
    /// The scene path of its `AnimatorPresetPanel` (the Image multipliers of the panel and its buttons, 8, are keyed by it).
    pub base: &'static str,
}

/// The creator of the world's settings panel.
pub const WORLD_CREATOR: CreatorPlace = CreatorPlace {
    origin: [954.0, 248.0],
    base: "UICanvas/MainUI/Layer1/Panels/Right Panels/DynamicSettingsPanel/AnimationsPanel/Scroll View/Viewport/Content/Animator Header/Panel Holder/AnimatorPresetPanel",
};

/// The creator of the scenario editor's Animation tab (its header is the tab's first row: left edge 41, top 125, 46.88 high).
pub const EDITOR_CREATOR: CreatorPlace = CreatorPlace {
    origin: [44.0, 148.0],
    base: "UI/ScenarioEditor/Settings Tuner/Holder/Panels/AnimationsPanel/Scroll View/Viewport/Content/Animator Header/Panel Holder/AnimatorPresetPanel",
};

/// The creator panel (`SettingsChangerCreator`): a 605 wide panel with the preset dropdown, the sliders of the preset and the Cancel and OK
/// buttons, drawn over the tab at the left of its panel. Returns its nodes and click regions.
pub(super) fn creator_panel(lists: &ListsInput<'_>, edit: Option<&crate::value_edit::ValueEdit>, place: CreatorPlace) -> Option<(Vec<Node>, Vec<DynHit>, Vec<DynTip>)> {
    let creator = lists.creator?;
    if !creator.open {
        return None;
    }
    let fields = animators::creator_fields(creator.preset);
    let [x0, y0] = place.origin;
    let width = 605.0;
    let height = 115.0 + fields.len() as f32 * 40.5 + 61.0;
    let base = place.base;
    let mut hits = Vec::new();
    let mut tips = Vec::new();
    let mut root = sprite_node(base, "Button_UI", [x0, y0, x0 + width, y0 + height], [1.0; 4]);
    // The panel swallows the clicks on it.
    hits.push(DynHit {
        rect: [x0, y0, width, height],
        action: UiAction::Consume,
    });
    root.children.push(card_text(&format!("{base}/DialogTitle"), "Animator Presets", 28.0, [x0, y0 + 2.0, x0 + width, y0 + 34.0], "Center", 0.0));
    // The preset dropdown.
    root.children.push(card_text(&format!("{base}/Body/simpleDropdown/Title"), "Preset", 26.0, [x0 + 60.0, y0 + 50.0, x0 + 200.0, y0 + 82.0], "Left", INK));
    let dropdown = [x0 + 210.0, y0 + 46.0, x0 + 532.0, y0 + 80.0];
    root.children.push(sprite_node(&format!("{base}/Body/simpleDropdown/Dropdown"), "UISprite", dropdown, [1.0; 4]));
    root.children.push(card_text(&format!("{base}/Body/simpleDropdown/Dropdown/Label"), creator.preset.label(), 19.65, [dropdown[0] + 10.0, dropdown[1] + 7.0, dropdown[2] - 30.0, dropdown[3] - 7.0], "Left", 0.196_078_43));
    root.children.push(sprite_node(&format!("{base}/Body/simpleDropdown/Dropdown/Arrow"), "DropdownArrow", [dropdown[2] - 28.0, dropdown[1] + 7.0, dropdown[2] - 8.0, dropdown[3] - 7.0], [1.0; 4]));
    hits.push(DynHit {
        rect: [dropdown[0], dropdown[1], dropdown[2] - dropdown[0], dropdown[3] - dropdown[1]],
        action: UiAction::DropdownOpen(crate::ui::DropdownTarget::AnimatorPreset),
    });
    // The sliders of the preset.
    for (slot, field) in fields.iter().copied().enumerate() {
        let row_y = y0 + 115.0 + slot as f32 * 40.5;
        let info: CreatorField = animators::creator_field(field);
        let slider_info = animators::creator_slider_info(&info);
        let value = creator.value(field);
        let reference = FieldRef::Zone(ze::CREATOR_REF, field);
        let spec = SliderSpec {
            name: info.name,
            text: info.format().format(value),
            fraction: fraction_range(slider_info.mode, slider_info.min, slider_info.max, slider_info.default, value),
            default_fraction: fraction_range(slider_info.mode, slider_info.min, slider_info.max, slider_info.default, slider_info.default),
            field: reference,
            format: info.format(),
            rules: info_rules(&slider_info, false),
        };
        let geometry = CardSliderGeometry {
            name_w: 190.0,
            value_x: 200.0,
            value_w: 80.0,
            value_align: "Center",
            panel_x: 311.0,
            panel_w: 249.0,
        };
        let editing = edit.filter(|edit| edit.field == reference);
        let mut row_nodes = Node::default();
        card_slider_row(&mut row_nodes, &mut hits, editing, &format!("{base}/Body/{}", info.name), (x0 + 12.0, row_y), &geometry, &spec);
        // The creator's rows are bigger than a card's: the name is 26, the value 20.
        for child in &mut row_nodes.children {
            let size = if child.path.ends_with("/Name") {
                26.0
            } else if child.path.ends_with("/FloatInfoText") {
                20.0
            } else {
                continue;
            };
            child.texts.iter_mut().for_each(|text| text.font_size = size);
        }
        root.children.extend(row_nodes.children);
        tips.push(DynTip {
            rect: [x0 + 12.0, row_y, 190.0, 30.0],
            tip: crate::tooltip::Tip {
                path: "",
                title: info.name,
                content: info.help,
                ui: true,
                instant: false,
            },
        });
    }
    // Cancel (left) and OK (right).
    let buttons_y = y0 + height - 40.0;
    for (label, rect, command) in [
        ("Cancel", [x0 + 13.0, buttons_y, x0 + 93.0, buttons_y + 35.0], crate::ui::ControlCommand::CloseAnimatorCreator),
        ("OK", [x0 + width - 108.0, buttons_y, x0 + width - 8.0, buttons_y + 35.0], crate::ui::ControlCommand::SubmitAnimatorCreator),
    ] {
        let button = if label == "Cancel" { "DismissButton" } else { "AcceptButton" };
        root.children.push(sprite_node(&format!("{base}/buttonHolder/{button}"), "Button_UI", rect, if label == "Cancel" { [1.0, 1.0, 1.0, 0.878_431] } else { [1.0; 4] }));
        let mut text = card_text(&format!("{base}/buttonHolder/{button}/Text (TMP)"), label, if label == "OK" { 22.35 } else { 20.0 }, [rect[0], rect[1], rect[2], rect[3]], "Center", if label == "OK" { 0.0 } else { INK });
        text.texts.iter_mut().for_each(|component| component.font_size = if label == "OK" { 22.35 } else { 20.0 });
        root.children.push(text);
        hits.push(DynHit {
            rect: [rect[0], rect[1], rect[2] - rect[0], rect[3] - rect[1]],
            action: UiAction::Invoke(command),
        });
    }
    Some((vec![root], hits, tips))
}
