//! The Zones tab's Preview panel (`ZonePreview` under `ZoneEditorPanel`), which the `TogglePreview`
//! button in the corner of the tab shows and hides (`ObjectToggle`).
//!
//! Prefab values (level2): a `gray_panel` (border 10) of 464.06 x 600 whose right edge is the
//! dynamic panel's left edge, centred on the Zones tab's panel; a vertical layout with padding
//! 30 / 30 / 20 / 30 and spacing 20: the title "Preview" (Roboto-Bold 40, white, 46.88 high), a
//! holder (404.06 x 368.73) with the 368.73-square preview (`gray_panel` frame, a navy
//! `BackGround` inset by 6 that the zone shapes fill) and the three estimation lines (Roboto-Bold
//! 24, white, 28.13 high, spacing 5: label on the left, value flush right) -
//! "Total Biomass:", "Pellet Production:" and "Estimated Pellets:" for all the zones of the world.

use bibites_core::math::Vec2;

use crate::app::App;
use crate::ui::{FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame};

/// The panel (reference px): `x0, y0, x1, y1`.
pub const PANEL: [f32; 4] = [455.94, 307.0, 920.0, 907.0];
/// The preview square (the `ZonePreview` frame).
pub const SQUARE: [f32; 4] = [503.6, 393.88, 872.33, 762.61];
/// What the zone shapes are drawn on (the navy `BackGround`, 6 inside the frame).
pub const BACKGROUND: [f32; 4] = [509.6, 399.88, 866.33, 756.61];
/// Left and right edge of the estimation lines.
const LINE_X: [f32; 2] = [485.94, 890.0];
/// Top of the estimation block (`Estimation`: 94.39 high, bottom at 570 inside the panel).
const ESTIMATION_TOP: f32 = 782.61;
const LINE_H: f32 = 28.13;
const LINE_SPACING: f32 = 5.0;
const TITLE_CENTRE: Vec2 = Vec2::new(687.97, 350.44);
const WHITE: Rgba = [255, 255, 255, 255];
const BOLD_FONT: &str = "Roboto-Bold SDF NoOutline";
/// Id of the panel's click-swallowing hit.
const HIT_PANEL: u32 = 8900;

/// Whether the panel is up: it belongs to the Zones tab of the dynamic panel.
#[must_use]
pub fn is_open(app: &App) -> bool {
    app.zone_preview_open
        && app.panels.is_open(crate::ui::PanelKind::WorldDynamics)
        && app.dyn_panel.tab == 1
}

/// The three estimation lines (`BiomassPreview`): total biomass, pellet production and the number
/// of pellets the world's zones should hold, summed over every zone.
#[must_use]
pub fn estimation(app: &App) -> [String; 3] {
    let zones = &app.world.zones;
    let size = app.world.settings.independents.simulation_size;
    let biomass: f32 = zones.zones().iter().map(|zone| zone.max_biomass).sum();
    let growth: f32 = zones.zones().iter().map(|zone| zone.total_growth).sum();
    let pellets: u32 = zones
        .zones()
        .iter()
        .map(|zone| zone.estimated_pellets(&zones.globals, size))
        .fold(0_u32, u32::saturating_add);
    let formats = crate::scenario_editor::estimation_formats();
    [
        formats[0].format(biomass),
        formats[1].format(growth),
        formats[2].format(pellets as f32),
    ]
}

/// Draws the panel (when it is up) with its zone shapes and the estimation.
pub fn emit(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    width: f32,
    height: f32,
) {
    if !is_open(app) {
        return;
    }
    let s = UiFrame::scaler(width, height);
    // The panel hangs left of the settings panel, which is flush with the right edge of the canvas: all of it (laid out for 1920 x 1080 below) moves
    // with that panel.
    let (draws_from, hits_from) = (frame.draws.len(), frame.hits.len());
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let measure = |text: &str, px: f32| -> f32 {
        metrics.map_or(text.chars().count() as f32 * px * 0.5, |fonts| {
            fonts.measure_named(Some(BOLD_FONT), FontRole::Bold, text, px)
        })
    };
    let (x0, y0, x1, y1) = (PANEL[0], PANEL[1], PANEL[2], PANEL[3]);
    // The frames are `gray_panel` slices (border 10 at 100 ppu, multiplier 1).
    let slice = |frame: &mut UiFrame, rect: [f32; 4]| match ui.texture_of_alias("gray_panel") {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [10.0; 4],
            dest_scale: 1.0,
            pos: at(rect[0], rect[1]),
            size: at(rect[2] - rect[0], rect[3] - rect[1]),
            tint: WHITE,
        }),
        None => frame.rect(
            at(rect[0], rect[1]),
            at(rect[2] - rect[0], rect[3] - rect[1]),
            [48, 48, 48, 255],
        ),
    };
    slice(frame, [x0, y0, x1, y1]);
    // The panel swallows the clicks meant for the world under it.
    frame.hit(
        HIT_PANEL,
        at(x0, y0),
        at(x1 - x0, y1 - y0),
        UiAction::Consume,
    );
    // Title, centred.
    let title_px = 40.0 * s;
    let title_w = measure("Preview", title_px);
    frame.bold(
        "Preview",
        Vec2::new(
            TITLE_CENTRE.x * s - title_w * 0.5,
            (TITLE_CENTRE.y - 20.0) * s,
        ),
        title_px,
        WHITE,
    );
    // The square: frame, navy background, then the zones.
    slice(frame, SQUARE);
    frame.rect(
        at(BACKGROUND[0], BACKGROUND[1]),
        at(BACKGROUND[2] - BACKGROUND[0], BACKGROUND[3] - BACKGROUND[1]),
        [0, 0, 27, 255],
    );
    let scenario = bibites_core::world::ScenarioData {
        zones: app
            .world
            .zones
            .zones()
            .iter()
            .map(|zone| zone.config.0.clone())
            .collect(),
        ..bibites_core::world::ScenarioData::default()
    };
    let shapes = crate::scenario_editor::preview_shapes(&app.world.settings, &scenario);
    let base = app
        .assets
        .as_ref()
        .map_or([231, 154, 0], |assets| assets.background.zone_base);
    crate::ui_runtime::emit_zone_preview(
        frame,
        &shapes,
        [base[0], base[1], base[2], 255],
        BACKGROUND,
        s,
    );
    // Each zone's name, biomass and pellet count over its middle (`ZonePreview`'s labels).
    let size = app.world.settings.independents.simulation_size;
    let labels: Vec<crate::species_panel::position::ZoneLabel> = app
        .world
        .zones
        .zones()
        .iter()
        .map(|zone| {
            let centre = crate::scenario_editor::zone_shape(&zone.config, size).centre;
            crate::species_panel::position::ZoneLabel::of(zone, &app.world.zones.globals, size, centre)
        })
        .collect();
    crate::species_panel::position::emit_zone_labels(
        frame,
        &labels,
        [
            BACKGROUND[0],
            BACKGROUND[1],
            BACKGROUND[2] - BACKGROUND[0],
            BACKGROUND[3] - BACKGROUND[1],
        ],
        s,
        &|text, px| measure(text, px),
    );
    // The estimation lines.
    let labels = ["Total Biomass:", "Pellet Production:", "Estimated Pellets:"];
    for (index, (label, value)) in labels.iter().zip(estimation(app)).enumerate() {
        let top = ESTIMATION_TOP + index as f32 * (LINE_H + LINE_SPACING);
        let px = 24.0 * s;
        let text_top = (top + (LINE_H - 24.0) * 0.5) * s;
        frame.bold(*label, Vec2::new(LINE_X[0] * s, text_top), px, WHITE);
        let value_w = measure(&value, px);
        frame.bold(
            value,
            Vec2::new(LINE_X[1] * s - value_w, text_top),
            px,
            WHITE,
        );
    }
    let dx = crate::ui_runtime::dyn_panel_offset(Vec2::new(width / s, height / s)) * s;
    frame.translate_from(draws_from, hits_from, dx, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_hangs_left_of_the_dynamic_panel_with_the_prefabs_size() {
        assert!((PANEL[2] - PANEL[0] - 464.06).abs() < 0.01);
        assert!((PANEL[3] - PANEL[1] - 600.0).abs() < 0.01);
        assert_eq!(
            PANEL[2],
            crate::dyn_panel::PANEL[0],
            "its right edge is the dynamic panel's left edge"
        );
        // The frame is a 368.73 square, its background 6 inside.
        assert!((SQUARE[2] - SQUARE[0] - 368.73).abs() < 0.01);
        assert!((SQUARE[3] - SQUARE[1] - 368.73).abs() < 0.01);
        for (inner, outer) in BACKGROUND.iter().zip(SQUARE) {
            assert!((inner - outer).abs() - 6.0 < 0.01);
        }
        // Estimation: three 28.13 lines and two 5 gaps end at the panel's inner bottom (570 + top).
        let bottom = ESTIMATION_TOP + 3.0 * LINE_H + 2.0 * LINE_SPACING;
        assert!((bottom - (PANEL[1] + 570.0)).abs() < 0.01, "{bottom}");
    }

    #[test]
    fn the_panel_is_up_only_on_the_zones_tab_while_toggled_on() {
        let loaded = crate::app::LoadedWorld::load(None, 1, 2, 1).expect("world");
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        assert!(!is_open(&app));
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::ToggleZonePreview,
        )]);
        assert!(
            app.zone_preview_open && !is_open(&app),
            "the tab is not open"
        );
        app.panels.open(crate::ui::PanelKind::WorldDynamics);
        app.dyn_panel.tab = 1;
        assert!(is_open(&app));
        app.dyn_panel.tab = 0;
        assert!(!is_open(&app));
        app.dyn_panel.tab = 1;
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::ToggleZonePreview,
        )]);
        assert!(!is_open(&app));
    }

    #[test]
    fn the_panel_moves_with_the_settings_panel_in_a_wider_window() {
        let loaded = crate::app::LoadedWorld::load(None, 3, 2, 1).expect("world");
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        app.apply_ui_actions(&[UiAction::Invoke(
            crate::ui::ControlCommand::ToggleZonePreview,
        )]);
        app.panels.open(crate::ui::PanelKind::WorldDynamics);
        app.dyn_panel.tab = 1;
        assert!(is_open(&app));
        let assets = UiAssets::empty();
        let place = |width: f32, height: f32| {
            let mut frame = UiFrame::new(UiFrame::scaler(width, height));
            emit(&mut frame, &app, &assets, None, width, height);
            let (_, pos, size) = *frame
                .hits
                .iter()
                .find(|(id, ..)| *id == HIT_PANEL)
                .expect("the panel takes the clicks on it");
            (frame.scale, pos, size, frame.draws.len())
        };
        let (scale, pos, size, drawn) = place(1920.0, 1080.0);
        assert_eq!(scale, 1.0);
        assert!((pos.x - PANEL[0]).abs() < 0.01, "{pos:?}");
        // A canvas 640 reference px wider: the settings panel is flush with its right edge, and the preview hangs left of it.
        let (wide_scale, wide_pos, wide_size, wide_drawn) = place(2560.0, 1080.0);
        assert_eq!(wide_scale, 1.0);
        assert!((wide_pos.x - (PANEL[0] + 640.0)).abs() < 0.01, "{wide_pos:?}");
        assert!((wide_pos.y - pos.y).abs() < 0.01 && wide_size == size && wide_drawn == drawn);
    }

    #[test]
    fn the_estimation_sums_every_zone_of_the_world() {
        let loaded = crate::app::LoadedWorld::load(None, 2, 2, 1).expect("world");
        let mut app = App::new(loaded, Vec2::new(1280.0, 800.0), None);
        let before = estimation(&app);
        assert!(
            before[0].ends_with('E')
                && before[1].ends_with("E/s")
                && before[2].ends_with("pellets"),
            "{before:?}"
        );
        // A second zone adds to all three.
        app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::AddZone)]);
        let after = estimation(&app);
        assert_ne!(before, after, "{before:?} {after:?}");
    }
}
