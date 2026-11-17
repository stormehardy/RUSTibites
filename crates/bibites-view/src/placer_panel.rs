//! The options panels of the two placers: `BibitePlacerPanel` (the UI of `BibitePlacer`, shown while
//! bibites are placed from a template with the left mouse button) and `PlacePelletPanel` (the UI
//! of `PelletPlacer`: material, "Pellet Size Factor" and "Pellet Number").
//!
//! Geometry and texts come from the prefab (`tools/unity/dump_ui_tree.py`, scene `level2`, path id
//! 595, child of `PlacerPanels` which sits 10 units from the bottom-left corner): a `Button_UI`
//! frame 404.79 wide and 234.22 high (34 less when the custom tag row is hidden, growing upwards),
//! 15 units of padding, the directive "You can place bibites by left mouse click.", three
//! dropdown rows (Mutate Genes, Growth At Spawn, Tagging; labels from `BibiteSettings`), the custom
//! tag field (only with custom tagging), "Press close button to exit placement mode." and a 40 x 40
//! close button in the top-right corner.
//!
//! The pellet panel (path id 54 in `level2`, also a child of `PlacerPanels`): a 504.6 x 216.24 `Button_UI`
//! frame with "You can place pellets by left mouse click.", the Material dropdown (Plant, Meat: the
//! physical materials), a logarithmic size slider (0.1 to 1000, default 1) and a whole-number count
//! slider (1 to 10), "Press close button to exit placement mode." and the close button. (The game
//! has no pile radius control: the pile follows from the count and the energy.)
//!
//! Not matched: the dropdowns advance on click (the game opens a list) and there is no tooltip.

use bibites_core::math::Vec2;
use bibites_core::settings::{BibiteSpawn, Tagging};

use bibites_core::format::FloatValueFormat;

use crate::app::App;
use crate::bibite_editor::{GROWTH, MUTATIONS, TAGGING, choice_label, name_of};
use crate::dyn_panel::{SliderMode, fraction_range, log};
use crate::ui::{ControlCommand, Rgba, UiAction, UiAssets, UiDraw, UiFrame};

/// The panel's left edge and its bottom's distance from the screen bottom (canvas units).
const LEFT: f32 = 10.0;
const BOTTOM_GAP: f32 = 10.0;
const WIDTH: f32 = 404.79;
const PAD: f32 = 15.0;
const DARK: Rgba = [28, 28, 28, 255];
const DIM: Rgba = [120, 120, 120, 255];

const HIT_PANEL: u32 = 8700;
const HIT_CLOSE: u32 = 8701;
const HIT_FIELD: u32 = 8702;
const HIT_DROPDOWN: u32 = 8710;

/// Panel height: the layout group's result without and with the custom tag row.
#[must_use]
pub fn panel_height(options: &BibiteSpawn) -> f32 {
    if options.tagging == Tagging::CustomTagging {
        234.22
    } else {
        200.22
    }
}

/// Rough advance of a run of text at `px` (the build step has no font metrics).
fn estimate_width(text: &str, px: f32) -> f32 {
    text.chars().count() as f32 * px * 0.56
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

/// The pellet placer's "Pellet Size Factor" slider.
pub const PELLET_SIZE: u8 = 0;
/// The pellet placer's "Pellet Number" slider.
pub const PELLET_COUNT: u8 = 1;

const PELLET_WIDTH: f32 = 504.6;
const PELLET_HEIGHT: f32 = 216.24;
const HIT_PELLET_PANEL: u32 = 8720;
const HIT_PELLET_CLOSE: u32 = 8721;
const HIT_PELLET_MATERIAL: u32 = 8722;
const HIT_PELLET_SLIDER: u32 = 8723;

/// The value text of the "Pellet Size Factor" slider (`FloatSetting`, precision 1; the log slider
/// shows significant digits).
fn pellet_size_text(value: f32) -> String {
    FloatValueFormat {
        precision: 1,
        precision_is_si: true,
        ..FloatValueFormat::default()
    }
    .format(value)
}

/// The value text of the "Pellet Number" slider (`IntSetting`).
fn pellet_count_text(count: usize) -> String {
    FloatValueFormat {
        precision: 0,
        is_int: true,
        ..FloatValueFormat::default()
    }
    .format(count as f32)
}

/// Draws the panels of the placers that are active.
pub fn draw(frame: &mut UiFrame, app: &App, ui: &UiAssets, width: f32, height: f32) {
    if app.tool == crate::ui::ToolKind::Pellet {
        draw_pellets(frame, &app.tool_settings, ui, width, height);
    }
    if app.placer.is_none() {
        return;
    }
    let editing = if app.tag_edit.is_some() && app.tag_edit_is_placer() {
        app.tag_edit.as_deref()
    } else {
        None
    };
    draw_options(frame, &app.placer_options, editing, ui, width, height);
}

/// Draws the panel for `options`; `editing` is the text being typed into the custom tag field.
pub fn draw_options(
    frame: &mut UiFrame,
    options: &BibiteSpawn,
    editing: Option<&str>,
    ui: &UiAssets,
    width: f32,
    height: f32,
) {
    let s = UiFrame::scaler(width, height);
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let panel_h = panel_height(options);
    // The panel is anchored to the bottom-left corner and grows upwards.
    let top = 1080.0 - BOTTOM_GAP - panel_h;
    let left = LEFT + PAD;
    let right = LEFT + WIDTH - PAD;

    // Frame: `Button_UI`, border 30 at multiplier 6.
    slice(
        frame,
        ui,
        "Button_UI",
        30.0,
        1.0 / 6.0,
        at(LEFT, top),
        dim(WIDTH, panel_h),
        [255, 255, 255, 255],
    );
    frame.hit(
        HIT_PANEL,
        at(LEFT, top),
        dim(WIDTH, panel_h),
        UiAction::Consume,
    );

    frame.text(
        "You can place bibites by left mouse click.",
        at(left, top + 15.0),
        18.0 * s,
        DARK,
    );

    // The three dropdown rows: label, then a 148-wide box at the right end.
    for (index, (field, row_top)) in [(MUTATIONS, 45.11), (GROWTH, 85.11), (TAGGING, 125.11)]
        .into_iter()
        .enumerate()
    {
        let y = top + row_top;
        frame.text(name_of(field), at(left, y + 4.0), 20.0 * s, DARK);
        let box_at = at(right - 148.45, y);
        slice(
            frame,
            ui,
            "UISprite",
            4.0,
            1.0,
            box_at,
            dim(148.45, 30.0),
            [245, 245, 245, 255],
        );
        frame.rect(box_at, dim(148.45, 1.0), [90, 90, 90, 255]);
        frame.rect(
            at(right - 148.45, y + 29.0),
            dim(148.45, 1.0),
            [90, 90, 90, 255],
        );
        frame.text(
            choice_label(options, field, &[]),
            at(right - 148.45 + 9.0, y + 6.0),
            18.0 * s,
            DARK,
        );
        if let Some(texture) = ui.texture_of_alias("DropdownArrow") {
            frame.draws.push(UiDraw::Image {
                texture,
                pos: at(right - 30.0, y + 5.0),
                size: dim(20.0, 20.0),
                tint: [255, 255, 255, 255],
            });
        }
        frame.hit(
            HIT_DROPDOWN + index as u32,
            box_at,
            dim(148.45, 30.0),
            UiAction::DropdownOpen(crate::ui::DropdownTarget::Placer(field)),
        );
    }

    // The custom tag field (`StringFieldHandle`), only with custom tagging.
    let mut directive_top = top + 199.1;
    if options.tagging == Tagging::CustomTagging {
        let field_at = at(left + 10.0, top + 159.11);
        let field_w = right - left - 10.0;
        slice(
            frame,
            ui,
            "InputFieldBackground",
            4.0,
            1.0,
            field_at,
            dim(field_w, 30.0),
            [255, 255, 255, 255],
        );
        frame.rect(field_at, dim(field_w, 1.0), [90, 90, 90, 255]);
        frame.rect(
            at(left + 10.0, top + 159.11 + 29.0),
            dim(field_w, 1.0),
            [90, 90, 90, 255],
        );
        let (text, color) = match editing {
            Some(draft) => (crate::text_field::with_caret(draft), DARK),
            None if options.tag.is_empty() => ("Enter text...".to_owned(), DIM),
            None => (options.tag.clone(), DARK),
        };
        let px = if estimate_width(&text, 20.0) > field_w - 16.0 {
            (20.0 * (field_w - 16.0) / estimate_width(&text, 20.0)).max(12.0)
        } else {
            20.0
        };
        frame.text(text, at(left + 18.0, top + 159.11 + 5.0), px * s, color);
        frame.hit(
            HIT_FIELD,
            field_at,
            dim(field_w, 30.0),
            UiAction::Invoke(ControlCommand::EditPlacerTag),
        );
    } else {
        directive_top -= 34.0;
    }
    frame.text(
        "Press close button to exit placement mode.",
        at(left, directive_top),
        18.0 * s,
        DARK,
    );

    // `CancelBibitePlacement`: 40 x 40 in the top-right corner, `Button_UI` at multiplier 8.
    let close_at = at(LEFT + WIDTH - 40.0, top);
    slice(
        frame,
        ui,
        "Button_UI",
        30.0,
        0.125,
        close_at,
        dim(40.0, 40.0),
        [255, 255, 255, 255],
    );
    if let Some(texture) = ui.texture_of_alias("exit_icon") {
        frame.draws.push(UiDraw::Image {
            texture,
            pos: close_at,
            size: dim(40.0, 40.0),
            tint: [255, 255, 255, 255],
        });
    }
    frame.hit(
        HIT_CLOSE,
        close_at,
        dim(40.0, 40.0),
        UiAction::Invoke(ControlCommand::ClosePlacer),
    );
}

/// Draws the pellet placer's panel for `settings`.
pub fn draw_pellets(
    frame: &mut UiFrame,
    settings: &crate::app::ToolSettings,
    ui: &UiAssets,
    width: f32,
    height: f32,
) {
    let s = UiFrame::scaler(width, height);
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let top = 1080.0 - BOTTOM_GAP - PELLET_HEIGHT;
    let left = LEFT + PAD;
    let right = LEFT + PELLET_WIDTH - PAD;

    slice(
        frame,
        ui,
        "Button_UI",
        30.0,
        1.0 / 6.0,
        at(LEFT, top),
        dim(PELLET_WIDTH, PELLET_HEIGHT),
        [255, 255, 255, 255],
    );
    frame.hit(
        HIT_PELLET_PANEL,
        at(LEFT, top),
        dim(PELLET_WIDTH, PELLET_HEIGHT),
        UiAction::Consume,
    );
    frame.text(
        "You can place pellets by left mouse click.",
        at(left, top + 15.0),
        18.0 * s,
        DARK,
    );

    // The material dropdown: a 200-wide box at the right end of the row.
    let row = top + 47.88;
    frame.text("Material", at(left, row + 4.0), 20.0 * s, DARK);
    let box_at = at(right - 200.0, row);
    slice(
        frame,
        ui,
        "UISprite",
        4.0,
        1.0,
        box_at,
        dim(200.0, 30.0),
        [245, 245, 245, 255],
    );
    frame.rect(box_at, dim(200.0, 1.0), [90, 90, 90, 255]);
    frame.rect(
        at(right - 200.0, row + 29.0),
        dim(200.0, 1.0),
        [90, 90, 90, 255],
    );
    let material = match settings.material {
        bibites_core::pellet::MaterialKind::Meat => "Meat",
        _ => "Plant",
    };
    frame.text(material, at(right - 200.0 + 9.0, row + 6.0), 18.0 * s, DARK);
    if let Some(texture) = ui.texture_of_alias("DropdownArrow") {
        frame.draws.push(UiDraw::Image {
            texture,
            pos: at(right - 30.0, row + 5.0),
            size: dim(20.0, 20.0),
            tint: [255, 255, 255, 255],
        });
    }
    frame.hit(
        HIT_PELLET_MATERIAL,
        box_at,
        dim(200.0, 30.0),
        UiAction::DropdownOpen(crate::ui::DropdownTarget::PelletMaterial),
    );

    // The two sliders: the name, its value, then a 210-wide slider panel at the right end.
    let size_mode = log(10.0, None);
    let sliders = [
        (
            PELLET_SIZE,
            "Pellet Size Factor",
            pellet_size_text(settings.pellet_energy),
            fraction_range(size_mode, 0.1, 1000.0, 1.0, settings.pellet_energy),
            top + 93.12,
        ),
        (
            PELLET_COUNT,
            "Pellet Number",
            pellet_count_text(settings.pellet_count),
            fraction_range(
                SliderMode::Int,
                1.0,
                10.0,
                1.0,
                settings.pellet_count as f32,
            ),
            top + 138.36,
        ),
    ];
    for (index, (field, name, text, fraction, y)) in sliders.into_iter().enumerate() {
        let name_px = if estimate_width(name, 20.0) > 166.0 {
            20.0 * 166.0 / estimate_width(name, 20.0)
        } else {
            20.0
        };
        frame.text(
            name,
            at(left, y + 4.0 + (20.0 - name_px) * 0.5),
            name_px * s,
            DARK,
        );
        let (panel0, panel1) = (right - 210.915, right);
        frame.rect(
            at(panel0, y),
            dim(panel1 - panel0, 30.0),
            [255, 255, 255, 100],
        );
        let (track0, track1) = (panel0 + 10.0, panel1 - 10.0);
        let centre = track0 + fraction.clamp(0.0, 1.0) * (track1 - track0);
        slice(
            frame,
            ui,
            "tiled_bars_empty",
            5.0,
            1.0,
            at(track0, y + 7.5),
            dim(track1 - track0, 15.0),
            [255, 255, 255, 255],
        );
        slice(
            frame,
            ui,
            "tiled_bars_full",
            5.0,
            1.0,
            at(track0, y + 7.5),
            dim((centre + 5.5 - track0).clamp(10.0, track1 - track0), 15.0),
            [255, 255, 255, 255],
        );
        if let Some(texture) = ui.texture_of_alias("base_circle_icon") {
            frame.draws.push(UiDraw::Image {
                texture,
                pos: at(centre - 10.0, y + 5.0),
                size: dim(20.0, 20.0),
                tint: [255, 255, 255, 255],
            });
        }
        // The value sits between the name and the slider, right-aligned.
        frame.text(
            &text,
            at(panel0 - 8.0 - estimate_width(&text, 20.0), y + 4.0),
            20.0 * s,
            DARK,
        );
        frame.hit(
            HIT_PELLET_SLIDER + index as u32,
            at(panel0, y),
            dim(panel1 - panel0, 30.0),
            UiAction::PelletSlide(field, track0.round() as u16, track1.round() as u16),
        );
    }

    frame.text(
        "Press close button to exit placement mode.",
        at(left, top + 183.4),
        18.0 * s,
        DARK,
    );
    // `CancelPelletPlacement`: 40 x 40 in the top-right corner.
    let close_at = at(LEFT + PELLET_WIDTH - 40.0, top);
    slice(
        frame,
        ui,
        "Button_UI",
        30.0,
        0.125,
        close_at,
        dim(40.0, 40.0),
        [255, 255, 255, 255],
    );
    if let Some(texture) = ui.texture_of_alias("exit_icon") {
        frame.draws.push(UiDraw::Image {
            texture,
            pos: close_at,
            size: dim(40.0, 40.0),
            tint: [255, 255, 255, 255],
        });
    }
    frame.hit(
        HIT_PELLET_CLOSE,
        close_at,
        dim(40.0, 40.0),
        UiAction::Invoke(ControlCommand::ClosePellets),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pellet_frame(settings: &crate::app::ToolSettings) -> UiFrame {
        let mut frame = UiFrame::default();
        draw_pellets(&mut frame, settings, &UiAssets::empty(), 1920.0, 1080.0);
        frame
    }

    #[test]
    fn the_pellet_panel_has_a_dropdown_two_sliders_and_a_close_button() {
        let settings = crate::app::ToolSettings::default();
        let frame = pellet_frame(&settings);
        let top = 1080.0 - BOTTOM_GAP - PELLET_HEIGHT;
        let right = LEFT + PELLET_WIDTH - PAD;
        assert_eq!(
            frame.action_at(Vec2::new(LEFT + 3.0, top + 3.0)),
            Some(UiAction::Consume)
        );
        assert_eq!(
            frame.action_at(Vec2::new(right - 100.0, top + 47.88 + 15.0)),
            Some(UiAction::DropdownOpen(
                crate::ui::DropdownTarget::PelletMaterial
            ))
        );
        let Some(UiAction::PelletSlide(field, a, b)) =
            frame.action_at(Vec2::new(right - 100.0, top + 93.12 + 15.0))
        else {
            panic!("no size slider");
        };
        assert_eq!(field, PELLET_SIZE);
        assert_eq!(f32::from(a), (right - 210.915 + 10.0).round());
        assert_eq!(f32::from(b), (right - 10.0).round());
        assert_eq!(
            frame.action_at(Vec2::new(right - 100.0, top + 138.36 + 15.0)),
            Some(UiAction::PelletSlide(PELLET_COUNT, a, b))
        );
        assert_eq!(
            frame.action_at(Vec2::new(LEFT + PELLET_WIDTH - 10.0, top + 10.0)),
            Some(UiAction::Invoke(ControlCommand::ClosePellets))
        );
    }

    #[test]
    fn the_sliders_read_like_the_games_settings() {
        // Size factor: precision 1 (significant digits on the log slider), count: a whole number.
        // (The SI path of the game's `ScientificFormat` leaves a space before the empty unit.)
        assert_eq!(pellet_size_text(1.0).trim_end(), "1.0");
        assert_eq!(pellet_size_text(250.0).trim_end(), "250");
        // `FloatSetting.SI` is on by default, so small values take a milli prefix.
        assert_eq!(pellet_size_text(0.1).trim_end(), "100 m");
        assert_eq!(pellet_count_text(7).trim_end(), "7");
        // The size slider runs from 0.1 (left) through 1 (a quarter) to 1000 (right).
        let mode = log(10.0, None);
        assert!(fraction_range(mode, 0.1, 1000.0, 1.0, 0.1) < 1e-4);
        assert!((fraction_range(mode, 0.1, 1000.0, 1.0, 1.0) - 0.25).abs() < 1e-3);
        assert!((fraction_range(mode, 0.1, 1000.0, 1.0, 1000.0) - 1.0).abs() < 1e-4);
    }

    fn frame_of(options: &BibiteSpawn) -> UiFrame {
        let mut frame = UiFrame::default();
        draw_options(
            &mut frame,
            options,
            None,
            &UiAssets::empty(),
            1920.0,
            1080.0,
        );
        frame
    }

    #[test]
    fn the_panel_grows_upwards_by_the_custom_tag_row() {
        let mut options = BibiteSpawn::game_defaults();
        options.tagging = Tagging::NoTagging;
        assert_eq!(panel_height(&options), 200.22);
        options.tagging = Tagging::CustomTagging;
        assert_eq!(panel_height(&options), 234.22);
    }

    #[test]
    fn it_has_three_dropdowns_a_close_button_and_swallows_clicks() {
        let mut options = BibiteSpawn::game_defaults();
        options.tagging = Tagging::NoTagging;
        let frame = frame_of(&options);
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        for field in [MUTATIONS, GROWTH, TAGGING] {
            assert!(
                actions.contains(&UiAction::DropdownOpen(crate::ui::DropdownTarget::Placer(
                    field
                ))),
                "{field}"
            );
        }
        assert!(actions.contains(&UiAction::Invoke(ControlCommand::ClosePlacer)));
        // No tag field without custom tagging.
        assert!(!actions.contains(&UiAction::Invoke(ControlCommand::EditPlacerTag)));
        let top = 1080.0 - BOTTOM_GAP - panel_height(&options);
        assert_eq!(
            frame.action_at(Vec2::new(LEFT + 3.0, top + 3.0)),
            Some(UiAction::Consume)
        );
        // The close button is in the top-right corner.
        assert_eq!(
            frame.action_at(Vec2::new(LEFT + WIDTH - 10.0, top + 10.0)),
            Some(UiAction::Invoke(ControlCommand::ClosePlacer))
        );
        // The first dropdown (Mutate Genes) sits 45 units below the top at the right end.
        assert_eq!(
            frame.action_at(Vec2::new(LEFT + WIDTH - 40.0, top + 45.11 + 15.0)),
            Some(UiAction::DropdownOpen(crate::ui::DropdownTarget::Placer(
                MUTATIONS
            )))
        );
    }

    #[test]
    fn custom_tagging_shows_the_tag_field() {
        let mut options = BibiteSpawn::game_defaults();
        options.tagging = Tagging::CustomTagging;
        let frame = frame_of(&options);
        let top = 1080.0 - BOTTOM_GAP - panel_height(&options);
        assert_eq!(
            frame.action_at(Vec2::new(LEFT + 100.0, top + 159.11 + 15.0)),
            Some(UiAction::Invoke(ControlCommand::EditPlacerTag))
        );
        // The labels are the game's setting names.
        assert_eq!(name_of(TAGGING), "Tagging");
    }
}
