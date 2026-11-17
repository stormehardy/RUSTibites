//! The selection rectangle (`SelectionBox`) and the multi-selection panel (`SelectionPanel`).
//!
//! Geometry comes from the game's prefab (read with `tools/unity/dump_ui_tree.py`, scene `level2`,
//! `UICanvas/MainUI/Layer1/Panels/UtilityPanels/SelectionPanel`): a `base_icon` panel anchored
//! 10 canvas units from the top-left corner, 15 units of padding (10 at the bottom), 20 units between
//! its sections, black text of size 26 (title) and 20; the delete-all button is a 60x60 `Button_UI`
//! with a `Delete_0` icon and the label "Delete" under it, the lay button (only when species are
//! selected) sits 90 units below it. Sizes are in canvas units, drawn at the HUD's UI scale.

use bibites_core::math::Vec2;

use crate::app::App;
use crate::selection::{self, Kind, Tally};
use crate::ui::{ControlCommand, Rgba, UiAction, UiAssets, UiDraw, UiFrame};

const BLACK: Rgba = [0, 0, 0, 255];

/// Panel placement and spacing (canvas units).
const PANEL_X: f32 = 10.0;
const PANEL_Y: f32 = 10.0;
const PANEL_W: f32 = 397.69;
const PAD: f32 = 15.0;
const PAD_BOTTOM: f32 = 10.0;
const GAP: f32 = 20.0;
const TITLE_H: f32 = 30.47;
const ROW_H: f32 = 23.44;
const ROW_GAP: f32 = 10.0;
/// Space above the first entity row inside the type section.
const ROWS_INSET: f32 = 3.0;
/// Height the buttons column takes: the delete button and its label, or both buttons.
const BUTTONS_H: f32 = 90.0;
const BUTTONS_BOTH_H: f32 = 150.0;
const BUTTON: f32 = 60.0;
/// Width of a row's name box before the energy starts (the prefab's `Name` is 120-128 wide).
const NAME_W: f32 = 122.0;
/// Species rows are 25 apart; at most five show (`speciesSectionLayout.preferredHeight`).
const SPECIES_PITCH: f32 = 25.0;
const SPECIES_ROWS_SHOWN: usize = 5;

/// Draws the rectangle of a drag in progress and the multi-selection panel.
pub fn draw(frame: &mut UiFrame, app: &App, ui: &UiAssets, width: f32, height: f32) {
    draw_rectangle(frame, app, ui);
    draw_panel(frame, app, ui, width, height);
}

/// `SelectionBox`: a translucent `gray_panel2` frame from the press to the cursor.
fn draw_rectangle(frame: &mut UiFrame, app: &App, ui: &UiAssets) {
    let Some(drag) = app.drag.filter(crate::app::DragState::is_rectangle) else {
        return;
    };
    let min = Vec2::new(
        drag.start.x.min(drag.cursor.x),
        drag.start.y.min(drag.cursor.y),
    );
    let size = Vec2::new(
        (drag.start.x - drag.cursor.x).abs(),
        (drag.start.y - drag.cursor.y).abs(),
    );
    // `Image` colour (1, 1, 1, 0.392), 9-sliced with border 4 at pixelsPerUnitMultiplier 2.
    slice(
        frame,
        ui,
        "gray_panel2",
        4.0,
        0.5,
        min,
        size,
        [255, 255, 255, 100],
    );
}

#[allow(clippy::too_many_arguments)]
fn slice(
    frame: &mut UiFrame,
    ui: &UiAssets,
    sprite: &str,
    border: f32,
    dest_scale: f32,
    pos: Vec2,
    size: Vec2,
    tint: Rgba,
) {
    match ui.texture_of_alias(sprite) {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [border; 4],
            dest_scale,
            pos,
            size,
            tint,
        }),
        None => frame.rect(pos, size, tint),
    }
}

fn icon(frame: &mut UiFrame, ui: &UiAssets, sprite: &str, pos: Vec2, size: Vec2) {
    if let Some(texture) = ui.texture_of_alias(sprite) {
        frame.draws.push(UiDraw::Image {
            texture,
            pos,
            size,
            tint: [255, 255, 255, 255],
        });
    }
}

/// Rough advance of a run of digits and letters at `px` (the build step has no font metrics).
fn estimate_width(text: &str, px: f32) -> f32 {
    text.chars().count() as f32 * px * 0.56
}

fn draw_panel(frame: &mut UiFrame, app: &App, ui: &UiAssets, width: f32, height: f32) {
    let Some(multi) = app.multi.as_ref() else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let info = &multi.info;

    let kind_rows: Vec<(Kind, &str, Tally)> = Kind::ALL
        .iter()
        .zip(info.kinds)
        .filter(|(_, tally)| tally.count > 0)
        .map(|((kind, label), tally)| (*kind, *label, tally))
        .collect();
    let species_shown = !info.species.is_empty();
    let rows = kind_rows.len();
    let entities_h = if rows == 0 {
        ROW_H
    } else {
        ROW_H + ROW_GAP + ROWS_INSET + rows as f32 * ROW_H + (rows - 1) as f32 * ROW_GAP
    };
    let buttons_h = if species_shown {
        BUTTONS_BOTH_H
    } else {
        BUTTONS_H
    };
    let content_h = entities_h.max(buttons_h);
    let species_rows = info.species.len().min(SPECIES_ROWS_SHOWN);
    // `OnSpeciesWidthChange`: the list, and with it the panel, grows to fit its widest row.
    let widest_row = info
        .species
        .iter()
        .take(SPECIES_ROWS_SHOWN)
        .map(|species| row_width(&species.name, &species.tally))
        .fold(0.0_f32, f32::max);
    let panel_w = PANEL_W.max(widest_row + 2.0 * (PAD + 6.0));
    let list_h = 6.0 + species_rows as f32 * SPECIES_PITCH;
    let species_h = ROW_H + ROW_GAP + list_h;
    let panel_h = PAD
        + TITLE_H
        + GAP
        + content_h
        + if species_shown { GAP + species_h } else { 0.0 }
        + PAD_BOTTOM;

    // Frame: `base_icon`, colour (1, 1, 1, 0.949), border 5 at multiplier 1.
    slice(
        frame,
        ui,
        "base_icon",
        5.0,
        1.0,
        at(PANEL_X, PANEL_Y),
        dim(panel_w, panel_h),
        [255, 255, 255, 242],
    );
    // The panel swallows the clicks that land on it.
    frame.hit(
        8500,
        at(PANEL_X, PANEL_Y),
        dim(panel_w, panel_h),
        UiAction::Consume,
    );

    let left = PANEL_X + PAD;
    // The top block keeps the prefab's width; only the species list follows the panel's width.
    let right = PANEL_X + PANEL_W - PAD;
    let list_right = PANEL_X + panel_w - PAD;
    let title_top = PANEL_Y + PAD;
    frame.text(
        "Selection Panel",
        at(left, title_top + 2.0),
        26.0 * s,
        BLACK,
    );
    // `ClosePanel`: 30x30, 10 units from the top-right corner.
    let close_at = at(PANEL_X + panel_w - 10.0 - 30.0, PANEL_Y + 10.0);
    icon(frame, ui, "1727490", close_at, dim(30.0, 30.0));
    frame.hit(
        8501,
        close_at,
        dim(30.0, 30.0),
        UiAction::Invoke(ControlCommand::SelectionClose),
    );

    // Entities section: "N  Entities ( E )" and one row per kind present.
    let content_top = title_top + TITLE_H + GAP;
    let count = selection::format::entity_count().format(info.total_count() as f32);
    let energy = selection::format::total_energy().format(info.total_energy());
    frame.text(
        format!("{count} Entities ( {energy} )"),
        at(left, content_top + 1.0),
        20.0 * s,
        BLACK,
    );
    for (index, (kind, label, tally)) in kind_rows.iter().enumerate() {
        let top = content_top + ROW_H + ROW_GAP + ROWS_INSET + index as f32 * (ROW_H + ROW_GAP);
        row(
            frame,
            ui,
            s,
            (left, top, label, tally),
            RowAction::Kind(*kind),
            8510 + index as u32,
        );
    }

    // Buttons column on the right edge.
    let button_x = right - BUTTON;
    let kill_at = at(button_x, content_top);
    slice(
        frame,
        ui,
        "Button_UI",
        30.0,
        0.25,
        kill_at,
        dim(BUTTON, BUTTON),
        [255, 255, 255, 255],
    );
    icon(
        frame,
        ui,
        "Delete_0",
        at(
            button_x + (BUTTON - 50.0) / 2.0,
            content_top + (BUTTON - 55.0) / 2.0,
        ),
        dim(50.0, 55.0),
    );
    frame.text(
        "Delete",
        at(
            button_x + (BUTTON - estimate_width("Delete", 20.0)) / 2.0,
            content_top + BUTTON,
        ),
        20.0 * s,
        BLACK,
    );
    frame.hit(
        8502,
        kill_at,
        dim(BUTTON, BUTTON),
        UiAction::Invoke(ControlCommand::SelectionKill),
    );
    if species_shown {
        let lay_top = content_top + 90.0;
        let lay_at = at(button_x, lay_top);
        slice(
            frame,
            ui,
            "Button_UI",
            30.0,
            0.25,
            lay_at,
            dim(BUTTON, BUTTON),
            [255, 255, 255, 255],
        );
        icon(frame, ui, "lay_egg_tool", lay_at, dim(BUTTON, BUTTON));
        frame.text(
            "LAY",
            at(
                button_x + (BUTTON - estimate_width("LAY", 20.0)) / 2.0,
                lay_top + BUTTON,
            ),
            20.0 * s,
            BLACK,
        );
        frame.hit(
            8503,
            lay_at,
            dim(BUTTON, BUTTON),
            UiAction::Invoke(ControlCommand::SelectionLay),
        );
    }

    // Species section: "N Species:" over a `Long_UI` list of the most numerous ones.
    if species_shown {
        let top = content_top + content_h + GAP;
        let species_count = selection::format::row_count().format(info.species.len() as f32);
        frame.text(
            format!("{species_count} Species:"),
            at(left, top + 1.0),
            20.0 * s,
            BLACK,
        );
        let list_top = top + ROW_H + ROW_GAP;
        // The prefab frames the list with `Long_UI` (a dark box with a thick light border, 14 units
        // at this height, which swallows a 31-unit frame and leaves the rows unreadable); a plain
        // light box with a dark outline keeps the rows legible.
        frame.rect(
            at(left, list_top),
            dim(list_right - left, list_h),
            [30, 30, 30, 255],
        );
        frame.rect(
            at(left + 2.0, list_top + 2.0),
            dim(list_right - left - 4.0, list_h - 4.0),
            [214, 214, 214, 255],
        );
        for (index, species) in info.species.iter().take(SPECIES_ROWS_SHOWN).enumerate() {
            let tally = species.tally;
            row(
                frame,
                ui,
                s,
                (
                    left + 6.0,
                    list_top + 3.0 + index as f32 * SPECIES_PITCH,
                    &species.name,
                    &tally,
                ),
                RowAction::Species(species.species.value()),
                8530 + index as u32,
            );
        }
    }
}

/// What a row's X button removes.
#[derive(Clone, Copy)]
enum RowAction {
    Kind(Kind),
    Species(u32),
}

/// Where a row's energy text starts, relative to the row: after the X, the count box and a name
/// box that grows with the name (`HorizontalLayoutGroup`).
fn energy_offset(label: &str) -> f32 {
    20.0 + 50.0 + NAME_W.max(estimate_width(label, 20.0) + 12.0)
}

/// Width a row needs: up to the end of its energy text.
fn row_width(label: &str, tally: &Tally) -> f32 {
    energy_offset(label)
        + estimate_width(&selection::format::row_energy().format(tally.energy), 20.0)
}

/// One `SelectedEntityTypeReference` row: the X button, the count, the name and the energy.
fn row(
    frame: &mut UiFrame,
    ui: &UiAssets,
    s: f32,
    (x, y, label, tally): (f32, f32, &str, &Tally),
    action: RowAction,
    id: u32,
) {
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let button = at(x, y + (ROW_H - 20.0) / 2.0);
    icon(frame, ui, "1727490", button, dim(20.0, 20.0));
    let command = match action {
        RowAction::Kind(kind) => ControlCommand::SelectionRemoveKind(kind.index()),
        RowAction::Species(raw) => ControlCommand::SelectionRemoveSpecies(raw),
    };
    frame.hit(id, button, dim(20.0, 20.0), UiAction::Invoke(command));
    let text_y = y + (ROW_H - 20.0) / 2.0 - 1.0;
    // Count in a 50-wide box after the button, then the name, then the energy.
    let count = selection::format::row_count().format(tally.count as f32);
    frame.text(
        &count,
        at(
            x + 20.0 + (50.0 - estimate_width(&count, 20.0)) / 2.0,
            text_y,
        ),
        20.0 * s,
        BLACK,
    );
    frame.text(label, at(x + 20.0 + 50.0, text_y), 20.0 * s, BLACK);
    frame.text(
        selection::format::row_energy().format(tally.energy),
        at(x + energy_offset(label), text_y),
        20.0 * s,
        BLACK,
    );
}
