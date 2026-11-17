//! P0 themed HUD built from the game's UI atlas.
//!
//! Builds a screen-space [`UiFrame`] from viewer state each frame. Layout uses
//! the atlas's nine-slice frames and icons with the reference sizes from
//! `tools/ui/ui_atlas.json` scaled by the game's `UIScaler`
//! (`min(h/1080, w/1920)`).
//!
//! Phase P0 (per `docs/design/UI_PARITY.md` / `docs/design/UI_ASSET_MAP.md`):
//! time/sim panel, population/ecology readout, species legend, playback strip
//! (pause/play/step + speed), selected-bibite Stats panel with energy/health
//! bars. Stats + Genes tabs, biology/brain panels and graphs are later phases.

use bibites_core::math::Vec2;

use crate::app::App;
use crate::ui::{
    DropdownTarget, InspectorTab, OverlayKind, Rgba, UiAction, UiAssets, UiDraw, UiFrame,
};

const TEXT: Rgba = [236, 236, 240, 255];
const DIM: Rgba = [176, 182, 190, 255];
const ACCENT: Rgba = [255, 196, 96, 255];
const PANEL_FRAME: &[&str] = &["ui/panel/hud_time_frame", "ui/panel/tooltip_frame"];
const POPUP_FRAME: &[&str] = &[
    "ui/panel/popup_frame",
    "ui/panel/tooltip_frame",
    "ui/panel/hud_time_frame",
];
const BAR_FRAME: &[&str] = &["ui/hud/statusbar_frame", "ui/panel/tooltip_frame"];
const BUTTON: &[&str] = &["ui/button/button", "ui/button/button_small"];
const HEALTH_FILL: &[&str] = &["ui/hud/statusbar_health_fill"];
const ENERGY_FILL: &[&str] = &["ui/hud/statusbar_energy_fill"];

/// Builds the P0 HUD frame for the current app state.
#[must_use]
pub fn build(app: &App, ui: &UiAssets, width: f32, height: f32) -> UiFrame {
    let scale = UiFrame::scaler(width, height);
    let mut frame = UiFrame::new(scale);
    let pad = 14.0 * scale;
    let body = 16.0 * scale;
    let small = 14.0 * scale;
    let heading = 20.0 * scale;
    let mono = 16.0 * scale;

    // --- Top-left: simulation time / playback / population -----------------
    let time_size = Vec2::new(470.0 * scale, 158.0 * scale);
    let time_pos = Vec2::new(pad, pad);
    frame.panel(ui, PANEL_FRAME, time_pos, time_size);
    let x = time_pos.x + 14.0 * scale;
    let mut y = time_pos.y + 12.0 * scale;
    frame.bold("The Bibites  -  RUSTibites", Vec2::new(x, y), heading, TEXT);
    y += 26.0 * scale;
    frame.mono(
        crate::format_clock(app.simulated_time()),
        Vec2::new(x, y),
        mono * 1.35,
        TEXT,
    );
    frame.text(
        format!("tick {}", app.world_tick()),
        Vec2::new(x + 150.0 * scale, y + 3.0 * scale),
        small,
        DIM,
    );
    y += 26.0 * scale;
    let state = if app.paused { "paused" } else { "running" };
    frame.text(
        format!(
            "{:.0} TPS   sim/real x{:.2}   speed x{:.2}   {}",
            app.time_keeper.tps(),
            app.time_keeper.sim_seconds_per_second(),
            app.speed,
            state
        ),
        Vec2::new(x, y),
        body,
        if app.paused { ACCENT } else { DIM },
    );
    y += 24.0 * scale;
    let hud = &app.hud;
    frame.text(
        format!(
            "pop {}   eggs {}   species {}",
            hud.population, hud.eggs, hud.species
        ),
        Vec2::new(x, y),
        body,
        TEXT,
    );
    y += 22.0 * scale;
    frame.text(
        format!(
            "gen mean {:.1}   age mean {:.1} h   seed {}",
            hud.generation_mean, hud.age_mean_hours, app.seed
        ),
        Vec2::new(x, y),
        small,
        DIM,
    );

    // --- Species legend -----------------------------------------------------
    let legend = &app.hud.species_legend;
    if !legend.is_empty() {
        let row = 20.0 * scale;
        let legend_size = Vec2::new(470.0 * scale, 42.0 * scale + row * legend.len() as f32);
        let legend_pos = Vec2::new(pad, time_pos.y + time_size.y + 8.0 * scale);
        frame.panel(ui, PANEL_FRAME, legend_pos, legend_size);
        frame.bold(
            "species",
            Vec2::new(legend_pos.x + 14.0 * scale, legend_pos.y + 12.0 * scale),
            body,
            DIM,
        );
        for (index, entry) in legend.iter().enumerate() {
            let baseline = legend_pos.y + 38.0 * scale + row * index as f32;
            let chip = Vec2::splat(11.0 * scale);
            frame.rect(
                Vec2::new(legend_pos.x + 14.0 * scale, baseline),
                chip,
                [entry.color[0], entry.color[1], entry.color[2], 255],
            );
            frame.text(
                format!("#{} {}", entry.id, entry.name),
                Vec2::new(legend_pos.x + 32.0 * scale, baseline - 2.0 * scale),
                small,
                TEXT,
            );
            frame.text(
                format!("{}", entry.count),
                Vec2::new(legend_pos.x + 400.0 * scale, baseline - 2.0 * scale),
                small,
                DIM,
            );
        }
    }

    // --- Bottom playback strip ---------------------------------------------
    let strip_w = 820.0 * scale;
    let strip_h = 46.0 * scale;
    let strip_pos = Vec2::new((width - strip_w) * 0.5, height - strip_h - 12.0 * scale);
    frame.panel(ui, PANEL_FRAME, strip_pos, Vec2::new(strip_w, strip_h));
    let icon = 28.0 * scale;
    let icon_y = strip_pos.y + (strip_h - icon) * 0.5;
    let pause_icon: &[&str] = if app.paused {
        &["ui/hud/play_icon", "ui/hud/pause_icon"]
    } else {
        &["ui/hud/pause_icon", "ui/hud/play_icon"]
    };
    let pause_pos = Vec2::new(strip_pos.x + 14.0 * scale, icon_y);
    frame.image(ui, pause_icon, pause_pos, Vec2::splat(icon));
    frame.hit(1, pause_pos, Vec2::splat(icon), UiAction::TogglePause);
    let step_pos = Vec2::new(strip_pos.x + 52.0 * scale, icon_y);
    frame.image(ui, &["ui/hud/step_icon"], step_pos, Vec2::splat(icon));
    frame.hit(2, step_pos, Vec2::splat(icon), UiAction::Step);
    let speed_x = strip_pos.x + 92.0 * scale;
    frame.text(
        format!("{:.2}x", app.speed),
        Vec2::new(speed_x, strip_pos.y + 14.0 * scale),
        heading,
        ACCENT,
    );
    let minus_pos = Vec2::new(speed_x + 42.0 * scale, strip_pos.y + 10.0 * scale);
    let button_size = Vec2::splat(26.0 * scale);
    if let Some(record) = ui.pick(BUTTON) {
        frame.nine_slice_tinted(
            ui,
            record,
            minus_pos,
            button_size,
            crate::ui::multiply(
                record.default_tint.unwrap_or([255, 255, 255, 255]),
                [80, 84, 96, 255],
            ),
        );
    }
    frame.bold(
        "-",
        Vec2::new(minus_pos.x + 10.0 * scale, minus_pos.y + 3.0 * scale),
        body,
        TEXT,
    );
    frame.hit(
        3,
        minus_pos,
        button_size,
        UiAction::SetSpeed((app.speed / 1.5).max(crate::app::MIN_SPEED)),
    );
    let plus_pos = Vec2::new(minus_pos.x + 32.0 * scale, minus_pos.y);
    if let Some(record) = ui.pick(BUTTON) {
        frame.nine_slice_tinted(
            ui,
            record,
            plus_pos,
            button_size,
            crate::ui::multiply(
                record.default_tint.unwrap_or([255, 255, 255, 255]),
                [80, 84, 96, 255],
            ),
        );
    }
    frame.bold(
        "+",
        Vec2::new(plus_pos.x + 8.0 * scale, plus_pos.y + 3.0 * scale),
        body,
        TEXT,
    );
    frame.hit(
        4,
        plus_pos,
        button_size,
        UiAction::SetSpeed((app.speed * 1.5).min(crate::app::MAX_SPEED)),
    );

    // Overlay visibility toggles (real game icons where available).
    let toggles = [
        ("G", OverlayKind::Grid, app.scene_options.grid, 5_u32),
        ("Z", OverlayKind::Zones, app.scene_options.zones, 6),
        ("P", OverlayKind::Pellets, app.scene_options.pellets, 7),
        (
            "O",
            OverlayKind::Pheromones,
            app.scene_options.pheromones,
            8,
        ),
        ("H", OverlayKind::Hud, app.show_hud, 9),
    ];
    for (index, (label, kind, enabled, id)) in toggles.into_iter().enumerate() {
        let pos = Vec2::new(
            strip_pos.x + 300.0 * scale + index as f32 * 30.0 * scale,
            strip_pos.y + 10.0 * scale,
        );
        let size = Vec2::splat(26.0 * scale);
        if let Some(record) = ui.pick(BUTTON) {
            let tint = if enabled {
                [92, 132, 96, 255]
            } else {
                [62, 64, 74, 255]
            };
            frame.nine_slice_tinted(ui, record, pos, size, tint);
        }
        frame.bold(
            label,
            Vec2::new(pos.x + 9.0 * scale, pos.y + 3.0 * scale),
            body,
            TEXT,
        );
        frame.hit(id, pos, size, UiAction::ToggleOverlay(kind));
    }
    frame.text(
        "space pause   1-8 speed   right/. step   F12 screenshot",
        Vec2::new(strip_pos.x + 468.0 * scale, strip_pos.y + 15.0 * scale),
        small,
        DIM,
    );

    // --- Selection panel (shared with the runtime-UI HUD path) -------------
    draw_selection_panel(&mut frame, app, ui, width, height);

    {
        let scale = UiFrame::scaler(width, height);
        if app.show_ruler {
            // `RulerHandle`: a 5 px bar between 100 and 200 px wide, labelled with
            // its length in world units.
            use bibites_core::format::FloatValueFormat;
            let bar_px = app.ruler_pixels();
            let at = Vec2::new(width - 40.0 * scale - bar_px, height - 40.0 * scale);
            frame.rect(at, Vec2::new(bar_px, 5.0 * scale), [235, 235, 240, 230]);
            frame.rect(
                Vec2::new(at.x, at.y - 4.0 * scale),
                Vec2::new(2.0 * scale, 13.0 * scale),
                [235, 235, 240, 230],
            );
            frame.rect(
                Vec2::new(at.x + bar_px - 2.0 * scale, at.y - 4.0 * scale),
                Vec2::new(2.0 * scale, 13.0 * scale),
                [235, 235, 240, 230],
            );
            let label = FloatValueFormat {
                units: "u",
                precision: 2,
                ..FloatValueFormat::default()
            }
            .format(app.ruler_target);
            frame.text(
                label,
                Vec2::new(at.x + bar_px / 2.0 - 24.0 * scale, at.y - 24.0 * scale),
                14.0 * scale,
                TEXT,
            );
        }
    }
    draw_extra_panels(&mut frame, app, ui, None, width, height, false);

    // Settings page tabs (graphics/gameplay/hotkeys) over the captured panel.
    if app.panels.is_open(crate::ui::PanelKind::Settings) {
        let tab_w = 150.0 * scale;
        let tab_h = 32.0 * scale;
        let total = tab_w * 3.0 + 16.0 * scale;
        let x0 = (width - total) * 0.5;
        for (index, label) in ["Graphics", "Gameplay", "Hotkeys"].into_iter().enumerate() {
            let at = Vec2::new(x0 + index as f32 * (tab_w + 8.0 * scale), 18.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                let tint = if app.settings_page as usize == index {
                    [92, 124, 152, 255]
                } else {
                    [58, 60, 70, 255]
                };
                frame.nine_slice_tinted(ui, record, at, Vec2::new(tab_w, tab_h), tint);
            }
            frame.text(
                label,
                Vec2::new(at.x + 22.0 * scale, at.y + 7.0 * scale),
                small,
                TEXT,
            );
            frame.hit(
                600 + index as u32,
                at,
                Vec2::new(tab_w, tab_h),
                UiAction::SetSettingsPage(index as u32),
            );
        }
        let summary = format!(
            "fullscreen {} | UI scale {:.2} | grid {} | pheromones {:.2} | autosave {} min / {} files | min fps {} | pause after load {}",
            on_off(app.prefs.fullscreen),
            app.prefs.ui_scale,
            on_off(app.prefs.show_grid),
            app.prefs.pheromone_opacity,
            app.prefs.autosave_period_minutes,
            app.prefs.autosave_max_files,
            app.prefs.min_fps,
            on_off(app.prefs.pause_after_load),
        );
        frame.text(
            &summary,
            Vec2::new(30.0 * scale, height - 26.0 * scale),
            13.0 * scale,
            DIM,
        );
    }

    // Hotkeys page rows (custom rebinding).
    if app.panels.is_open(crate::ui::PanelKind::Settings) && app.settings_page == 2 {
        let pos = Vec2::new(40.0 * scale, 120.0 * scale);
        let panel_w = 640.0 * scale;
        let panel_h = 380.0 * scale;
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(panel_w, panel_h));
        frame.bold(
            "HOTKEYS",
            Vec2::new(pos.x + 16.0 * scale, pos.y + 10.0 * scale),
            heading,
            TEXT,
        );
        for (index, action) in App::hotkey_actions().iter().enumerate() {
            let y = pos.y + 48.0 * scale + index as f32 * 32.0 * scale;
            frame.text(*action, Vec2::new(pos.x + 16.0 * scale, y), small, TEXT);
            frame.text(
                app.prefs.hotkeys.get(*action).map_or("-", String::as_str),
                Vec2::new(pos.x + 240.0 * scale, y),
                small,
                ACCENT,
            );
            let at = Vec2::new(pos.x + 430.0 * scale, y - 4.0 * scale);
            let size = Vec2::new(150.0 * scale, 26.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, at, size, [70, 74, 82, 255]);
            }
            frame.text(
                if app.hotkey_listening == Some(index) {
                    "press key..."
                } else {
                    "Rebind"
                },
                Vec2::new(at.x + 12.0 * scale, at.y + 3.0 * scale),
                small,
                TEXT,
            );
            frame.hit(
                950 + index as u32,
                at,
                size,
                UiAction::RebindHotkey(index as u32),
            );
        }
        frame.hit(
            960,
            Vec2::new(pos.x + 16.0 * scale, pos.y + panel_h - 46.0 * scale),
            Vec2::new(150.0 * scale, 30.0 * scale),
            UiAction::ResetHotkeys,
        );
        if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
            frame.nine_slice_tinted(
                ui,
                record,
                Vec2::new(pos.x + 16.0 * scale, pos.y + panel_h - 46.0 * scale),
                Vec2::new(150.0 * scale, 30.0 * scale),
                [70, 74, 82, 255],
            );
        }
        frame.text(
            "Reset Hotkeys",
            Vec2::new(pos.x + 26.0 * scale, pos.y + panel_h - 40.0 * scale),
            small,
            TEXT,
        );
    }

    frame
}

/// Draws the selected-bibite inspector panel (curated HUD layer; the egg's inspector is the runtime Stats panel).
///
/// Kept separate from [`build`] so the runtime-UI layout can replace the HUD
/// chrome while still reusing the live inspector panel.
pub fn draw_selection_panel(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    width: f32,
    _height: f32,
) {
    let scale = UiFrame::scaler(width, _height);
    let pad = 14.0 * scale;
    let small = 14.0 * scale;
    let heading = 20.0 * scale;
    // --- Selection panel ----------------------------------------------------
    if let Some(panel) = app
        .selection
        .as_ref()
        .filter(|panel| app.inspector_visible() && panel.egg.is_none())
    {
        let panel_w = 430.0 * scale;
        let panel_h = 430.0 * scale;
        let panel_pos = Vec2::new(width - panel_w - pad, pad);
        frame.panel(ui, POPUP_FRAME, panel_pos, Vec2::new(panel_w, panel_h));
        let x = panel_pos.x + 14.0 * scale;
        let mut y = panel_pos.y + 12.0 * scale;
        frame.bold(
            if panel.is_corpse {
                "Corpse".to_owned()
            } else {
                format!("Bibite {}", panel.target)
            },
            Vec2::new(x, y),
            heading,
            TEXT,
        );
        y += 26.0 * scale;
        frame.text(
            format!(
                "#{} {}   gen {}",
                panel.species_id.unwrap_or(0),
                panel.species_name,
                panel.generation
            ),
            Vec2::new(x, y),
            small,
            DIM,
        );
        y += 24.0 * scale;
        let energy = ratio(panel.energy, panel.max_energy);
        let health = ratio(panel.health, panel.max_health);
        y = bar(
            frame,
            ui,
            x,
            y,
            panel_w - 28.0 * scale,
            scale,
            "energy",
            energy,
            ENERGY_FILL,
            [90, 190, 240, 255],
            format!("{:.0} / {:.0} E", panel.energy, panel.max_energy),
        );
        y = bar(
            frame,
            ui,
            x,
            y,
            panel_w - 28.0 * scale,
            scale,
            "health",
            health,
            HEALTH_FILL,
            [
                (80.0 + 175.0 * (1.0 - health)) as u8,
                (210.0 * health + 40.0) as u8,
                70,
                255,
            ],
            format!("{:.0} / {:.0} HP", panel.health, panel.max_health),
        );
        let _generic_rows: Vec<(&str, String)> = vec![
            ("age", format!("{:.1} h", panel.age_seconds / 3600.0)),
            (
                "stomach",
                format!("{:.0} / {:.0} u2", panel.stomach, panel.stomach_capacity),
            ),
            ("fat", format!("{:.0} / {:.0} u2", panel.fat, panel.fat_max)),
            ("diet", format!("{:.2} (0 herb / 1 carn)", panel.diet)),
            (
                "body",
                format!(
                    "d1 {:.2}  size gene {:.2}  speed gene {:.2}",
                    panel.size, panel.size_gene, panel.speed_gene
                ),
            ),
            (
                "brain",
                format!(
                    "{} nodes  {} synapses",
                    panel.brain_nodes, panel.brain_synapses
                ),
            ),
            ("eggs", format!("{:.2} ready", panel.egg_progress)),
            (
                "outputs",
                format!(
                    "accel {:+.2}  rot {:+.2}  eat {:+.2}",
                    panel.brain_outputs[0], panel.brain_outputs[1], panel.brain_outputs[5]
                ),
            ),
        ];
        let top_rows = match app.inspector_tab {
            InspectorTab::Stats => stats_rows(panel),
            InspectorTab::Genes => bibites_core::genome::GENE_DISPLAY_NAMES
                .iter()
                .enumerate()
                .skip(app.genes_scroll.min(34))
                .map(|(index, name)| (*name, format!("{:.4}", panel.genes[index])))
                .collect(),
            InspectorTab::Biology => panel
                .biology
                .iter()
                .skip(app.genes_scroll.min(panel.biology.len().saturating_sub(1)))
                .cloned()
                .collect(),
            InspectorTab::Brain => panel
                .brain_rows
                .iter()
                .skip(
                    app.genes_scroll
                        .min(panel.brain_rows.len().saturating_sub(1)),
                )
                .cloned()
                .collect(),
        };
        for (label, value) in top_rows {
            if y > panel_pos.y + panel_h - 60.0 * scale {
                break;
            }
            frame.text(label, Vec2::new(x, y), small, DIM);
            let value_x = if app.inspector_tab == InspectorTab::Stats {
                x + 100.0 * scale
            } else {
                x + 170.0 * scale
            };
            frame.text(value, Vec2::new(value_x, y), small, TEXT);
            y += 20.0 * scale;
        }
        // P0b tab content rendered below the core stats rows.
        if app.inspector_tab != InspectorTab::Stats {
            y += 4.0 * scale;
            let rows: Vec<(&str, String)> = match app.inspector_tab {
                InspectorTab::Genes => Vec::new(),
                InspectorTab::Biology => Vec::new(),
                InspectorTab::Brain => Vec::new(),
                InspectorTab::Stats => Vec::new(),
            };
            for (label, value) in rows {
                if y > panel_pos.y + panel_h - 60.0 * scale {
                    break;
                }
                frame.text(label, Vec2::new(x, y), small, DIM);
                frame.text(value, Vec2::new(x + 96.0 * scale, y), small, TEXT);
                y += 19.0 * scale;
            }
        }
        // Inspector tab strip + close button (interactive, UI-first input).
        let tab_y = panel_pos.y + panel_h - 30.0 * scale;
        let mut tab_x = x;
        for (index, tab) in InspectorTab::ALL.into_iter().enumerate() {
            let size = Vec2::new(64.0 * scale, 22.0 * scale);
            let pos = Vec2::new(tab_x, tab_y);
            if let Some(record) = ui.pick(BUTTON) {
                let tint = if tab == app.inspector_tab {
                    [92, 124, 152, 255]
                } else {
                    [58, 60, 70, 255]
                };
                frame.nine_slice_tinted(ui, record, pos, size, tint);
            }
            frame.text(
                tab.label(),
                Vec2::new(pos.x + 8.0 * scale, pos.y + 3.0 * scale),
                small,
                if tab == app.inspector_tab { TEXT } else { DIM },
            );
            frame.hit(20 + index as u32, pos, size, UiAction::SetTab(tab));
            tab_x += 70.0 * scale;
        }
        let close_size = Vec2::splat(22.0 * scale);
        let close_pos = Vec2::new(
            panel_pos.x + panel_w - 30.0 * scale,
            panel_pos.y + 10.0 * scale,
        );
        frame.image(ui, &["ui/button/close_x"], close_pos, close_size);
        frame.hit(30, close_pos, close_size, UiAction::CloseInspector);
    }
}

/// The Load Game browser (rows come from `SaveBrowser`, not the capture); drawn over either UI path. The Save Game panel is
/// `save_panel`'s.
fn draw_save_browser(frame: &mut UiFrame, app: &App, ui: &UiAssets, width: f32, height: f32) {
    let scale = UiFrame::scaler(width, height);
    let body = 16.0 * scale;
    let small = 14.0 * scale;
    let heading = 20.0 * scale;
    let mono = 16.0 * scale;
    let _ = (body, small, heading, mono);
    // Dynamic load browser (rows come from SaveBrowser, not the capture).
    if app.panels.is_open(crate::ui::PanelKind::LoadGame) {
        let panel_w = 820.0 * scale;
        let panel_h = 500.0 * scale;
        let pos = Vec2::new((width - panel_w) * 0.5, (height - panel_h) * 0.5);
        frame.rect(pos, Vec2::new(panel_w, panel_h), [34, 36, 44, 255]);
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(panel_w, panel_h));
        frame.bold(
            "LOAD GAME",
            Vec2::new(pos.x + 24.0 * scale, pos.y + 16.0 * scale),
            heading,
            TEXT,
        );
        frame.text(
            format!(
                "{} saves  |  autosaves: {}",
                app.save_entries.len(),
                on_off(app.include_autosaves)
            ),
            Vec2::new(pos.x + 190.0 * scale, pos.y + 20.0 * scale),
            small,
            DIM,
        );
        let row_h = 34.0 * scale;
        let row_y0 = pos.y + 58.0 * scale;
        for (index, entry) in app.save_entries.iter().take(9).enumerate() {
            let row = Vec2::new(
                pos.x + 20.0 * scale,
                row_y0 + index as f32 * (row_h + 4.0 * scale),
            );
            let row_w = panel_w - 40.0 * scale;
            frame.rect(
                row,
                Vec2::new(row_w, row_h),
                if index == app.save_selected {
                    [72, 64, 40, 255]
                } else {
                    [34, 37, 40, 255]
                },
            );
            frame.text(
                &entry.name,
                Vec2::new(row.x + 10.0 * scale, row.y + 5.0 * scale),
                small,
                TEXT,
            );
            let meta = format!(
                "{}  pop {}  {:.1}s",
                entry.version.as_deref().unwrap_or("-"),
                entry.population.unwrap_or(0),
                entry.sim_time.unwrap_or(0.0),
            );
            frame.text(
                &meta,
                Vec2::new(row.x + 360.0 * scale, row.y + 5.0 * scale),
                small,
                DIM,
            );
            frame.hit(
                100 + index as u32,
                row,
                Vec2::new(row_w, row_h),
                UiAction::SelectSaveRow(index as u32),
            );
        }
        if app.save_entries.is_empty() {
            frame.text(
                "(no saves yet - use Save As)",
                Vec2::new(pos.x + 24.0 * scale, row_y0),
                small,
                DIM,
            );
        }
        let buttons: [(&str, crate::ui::ControlCommand, u32); 2] = [
            ("Load", crate::ui::ControlCommand::LoadSelectedSave, 120),
            ("Delete", crate::ui::ControlCommand::DeleteSave, 122),
        ];
        let mut bx = pos.x + 20.0 * scale;
        let by = pos.y + panel_h - 54.0 * scale;
        for (label, command, id) in buttons {
            let size = Vec2::new(120.0 * scale, 34.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, Vec2::new(bx, by), size, [70, 74, 82, 255]);
            }
            frame.text(
                label,
                Vec2::new(bx + 10.0 * scale, by + 7.0 * scale),
                small,
                TEXT,
            );
            frame.hit(id, Vec2::new(bx, by), size, UiAction::Invoke(command));
            bx += 126.0 * scale;
        }
        let auto = Vec2::new(pos.x + 20.0 * scale + 2.0 * 126.0 * scale, by);
        let auto_size = Vec2::new(150.0 * scale, 34.0 * scale);
        if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
            frame.nine_slice_tinted(ui, record, auto, auto_size, [70, 74, 82, 255]);
        }
        frame.text(
            "Autosaves",
            Vec2::new(auto.x + 10.0 * scale, auto.y + 7.0 * scale),
            small,
            TEXT,
        );
        frame.hit(124, auto, auto_size, UiAction::ToggleAutosaves);
        let close = Vec2::new(pos.x + panel_w - 34.0 * scale, pos.y + 12.0 * scale);
        frame.image(ui, &["ui/button/close_x"], close, Vec2::splat(22.0 * scale));
        frame.hit(125, close, Vec2::splat(22.0 * scale), UiAction::ClosePanel);
    }
}

/// Modal dialogs, dropdowns, tool property panel, challenge picker/tracker,
/// editors and utility panels: drawn over either UI path (the native replay
/// has no captured layout for them).
fn draw_modals(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    width: f32,
    height: f32,
    native_dialogs: bool,
) {
    let scale = UiFrame::scaler(width, height);
    let pad = 14.0 * scale;
    let body = 16.0 * scale;
    let small = 14.0 * scale;
    let heading = 20.0 * scale;
    let mono = 16.0 * scale;
    let _ = (pad, body, small, heading, mono);
    // Modal confirmation (KILL/KillHalf): game wording + Yes/No.
    if let Some(kind) = app.confirm_dialog.as_ref().filter(|_| !native_dialogs) {
        let width_px = 640.0 * scale;
        let height_px = 180.0 * scale;
        let pos = Vec2::new((width - width_px) * 0.5, (height - height_px) * 0.5);
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(width_px, height_px));
        let title = match &kind {
            crate::app::ConfirmKind::KillSelected => "Kill this bibite? Are you sure???".to_owned(),
            crate::app::ConfirmKind::RestartChallenge => {
                "You will lose all progress... Are you sure?".to_owned()
            }
            crate::app::ConfirmKind::KillHalf => {
                "Killing half the population... Are you sure???".to_owned()
            }
            crate::app::ConfirmKind::ClearDefaultBibites => {
                "Delete all default bibites? You will lose all data.".to_owned()
            }
            crate::app::ConfirmKind::DeleteZone(index) => {
                let name = app
                    .world
                    .zones
                    .zones
                    .get(*index)
                    .map_or_else(String::new, |zone| zone.config.0.name.clone());
                format!("Delete zone {name}? You will lose all data.")
            }
            crate::app::ConfirmKind::ClearZones => {
                "Delete all zones? You will lose all data.".to_owned()
            }
            crate::app::ConfirmKind::DeleteAnimator(index) => {
                let name = app
                    .world
                    .settings_changers
                    .get(*index)
                    .map_or_else(String::new, |changer| changer.name.clone());
                format!("Delete settings animator {name}? You will lose all data.")
            }
            crate::app::ConfirmKind::ClearAnimators => {
                "Delete all settings animators? You will lose all data.".to_owned()
            }
            crate::app::ConfirmKind::RemoveHalfPellets => {
                "Removing half of all pellets... Are you sure???".to_owned()
            }
            crate::app::ConfirmKind::OverwriteSave(name) => {
                format!("Overwrite save {name}?")
            }
            crate::app::ConfirmKind::DeleteSave(path) => format!(
                "Delete save {}?",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("save")
            ),
            crate::app::ConfirmKind::PruneSpecies(_) => {
                "Prune this species? This is IRREVERSIBLE!".to_owned()
            }
        };
        frame.bold(
            &title,
            Vec2::new(pos.x + 24.0 * scale, pos.y + 22.0 * scale),
            heading,
            TEXT,
        );
        let button_w = 170.0 * scale;
        let button_h = 40.0 * scale;
        let yes = Vec2::new(pos.x + 40.0 * scale, pos.y + height_px - 62.0 * scale);
        let no = Vec2::new(pos.x + width_px - 210.0 * scale, yes.y);
        for (label, at, action, id, tint) in [
            (
                "Yes",
                yes,
                crate::ui::ControlCommand::Confirm,
                90_u32,
                [120, 60, 52, 255],
            ),
            (
                "No",
                no,
                crate::ui::ControlCommand::Cancel,
                91,
                [70, 74, 82, 255],
            ),
        ] {
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, at, Vec2::new(button_w, button_h), tint);
            }
            frame.bold(
                label,
                Vec2::new(at.x + button_w * 0.5 - 15.0 * scale, at.y + 9.0 * scale),
                body,
                TEXT,
            );
            frame.hit(
                id,
                at,
                Vec2::new(button_w, button_h),
                UiAction::Invoke(action),
            );
        }
    }

    // An open dropdown list (the captured controls' too) is drawn by `dropdown::emit`, over
    // everything, by the caller.

    // Challenge/champion picker.
    if app.panels.is_open(crate::ui::PanelKind::ChallengePicker) {
        let panel_w = 760.0 * scale;
        let panel_h = 460.0 * scale;
        let pos = Vec2::new((width - panel_w) * 0.5, (height - panel_h) * 0.5);
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(panel_w, panel_h));
        frame.bold(
            "CHALLENGE SELECTOR",
            Vec2::new(pos.x + 20.0 * scale, pos.y + 12.0 * scale),
            heading,
            TEXT,
        );
        frame.text(
            "Scenarios",
            Vec2::new(pos.x + 20.0 * scale, pos.y + 46.0 * scale),
            small,
            ACCENT,
        );
        for (index, (name, _path)) in app.challenge_scenarios.iter().take(6).enumerate() {
            let at = Vec2::new(
                pos.x + 20.0 * scale,
                pos.y + 68.0 * scale + index as f32 * 30.0 * scale,
            );
            frame.text(
                if index == app.challenge_scenario_selected {
                    format!("> {name}")
                } else {
                    format!("  {name}")
                },
                at,
                small,
                TEXT,
            );
            frame.hit(
                850 + index as u32,
                at,
                Vec2::new(340.0 * scale, 26.0 * scale),
                UiAction::SelectChallengeRow(index as u32),
            );
        }
        frame.text(
            "Champion template",
            Vec2::new(pos.x + 390.0 * scale, pos.y + 46.0 * scale),
            small,
            ACCENT,
        );
        for (index, (name, _path)) in app.champion_templates.iter().take(6).enumerate() {
            let at = Vec2::new(
                pos.x + 390.0 * scale,
                pos.y + 68.0 * scale + index as f32 * 30.0 * scale,
            );
            frame.text(
                if index == app.champion_selected {
                    format!("> {name}")
                } else {
                    format!("  {name}")
                },
                at,
                small,
                TEXT,
            );
            frame.hit(
                880 + index as u32,
                at,
                Vec2::new(320.0 * scale, 26.0 * scale),
                UiAction::SelectChampionRow(index as u32),
            );
        }
        // Challenge parameter rows (timer + star thresholds).
        let row_y0 = pos.y + 250.0 * scale;
        for index in 0..4 {
            let y = row_y0 + index as f32 * 30.0 * scale;
            let label = match index {
                0 => format!(
                    "Timer: {}s",
                    [300, 600, 1200, 1800, 3600, 7200][app.challenge_timer_index.min(5)]
                ),
                1 => format!(
                    "1 star: {} Champion",
                    [10, 25, 50, 100, 200, 400][app.challenge_star_indices[0].min(5)]
                ),
                2 => format!(
                    "2 stars: {} Champion",
                    [10, 25, 50, 100, 200, 400][app.challenge_star_indices[1].min(5)]
                ),
                _ => format!(
                    "3 stars: {} Champion",
                    [10, 25, 50, 100, 200, 400][app.challenge_star_indices[2].min(5)]
                ),
            };
            frame.text(&label, Vec2::new(pos.x + 20.0 * scale, y), small, DIM);
            let minus = Vec2::new(pos.x + 380.0 * scale, y - 4.0 * scale);
            let plus = Vec2::new(pos.x + 416.0 * scale, y - 4.0 * scale);
            for (at, delta, id) in [
                (minus, -1, 920 + index as u32 * 2),
                (plus, 1, 921 + index as u32 * 2),
            ] {
                frame.rect(at, Vec2::splat(24.0 * scale), [70, 74, 82, 255]);
                frame.text(
                    if delta > 0 { "+" } else { "-" },
                    Vec2::new(at.x + 8.0 * scale, at.y + 2.0 * scale),
                    small,
                    TEXT,
                );
                frame.hit(
                    id,
                    at,
                    Vec2::splat(24.0 * scale),
                    UiAction::AdjustChallengeParam(index as u32, delta),
                );
            }
        }
        let start = Vec2::new(pos.x + 20.0 * scale, pos.y + panel_h - 52.0 * scale);
        let size = Vec2::new(220.0 * scale, 36.0 * scale);
        if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
            frame.nine_slice_tinted(ui, record, start, size, [80, 96, 84, 255]);
        }
        frame.text(
            "Start Challenge",
            Vec2::new(start.x + 16.0 * scale, start.y + 8.0 * scale),
            body,
            TEXT,
        );
        frame.hit(
            910,
            start,
            size,
            UiAction::Invoke(crate::ui::ControlCommand::StartChallenge),
        );
        let close = Vec2::new(pos.x + panel_w - 34.0 * scale, pos.y + 10.0 * scale);
        frame.image(ui, &["ui/button/close_x"], close, Vec2::splat(22.0 * scale));
        frame.hit(911, close, Vec2::splat(22.0 * scale), UiAction::ClosePanel);
    }

    // Local Workshop browser (no Steam).
    if app.panels.is_open(crate::ui::PanelKind::Workshop) {
        let panel_w = 720.0 * scale;
        let panel_h = 440.0 * scale;
        let pos = Vec2::new((width - panel_w) * 0.5, (height - panel_h) * 0.5);
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(panel_w, panel_h));
        frame.bold(
            "WORKSHOP (local files)",
            Vec2::new(pos.x + 20.0 * scale, pos.y + 12.0 * scale),
            heading,
            TEXT,
        );
        let row_h = 34.0 * scale;
        for (index, (name, _path, kind)) in app.workshop.iter().take(9).enumerate() {
            let at = Vec2::new(
                pos.x + 20.0 * scale,
                pos.y + 50.0 * scale + index as f32 * (row_h + 4.0 * scale),
            );
            let size = Vec2::new(panel_w - 40.0 * scale, row_h);
            frame.rect(
                at,
                size,
                if index == app.workshop_selected {
                    [72, 64, 40, 255]
                } else {
                    [34, 37, 40, 255]
                },
            );
            frame.text(
                name,
                Vec2::new(at.x + 10.0 * scale, at.y + 5.0 * scale),
                small,
                TEXT,
            );
            frame.text(
                kind,
                Vec2::new(at.x + 460.0 * scale, at.y + 5.0 * scale),
                small,
                DIM,
            );
            frame.hit(
                800 + index as u32,
                at,
                size,
                UiAction::SelectWorkshopRow(index as u32),
            );
        }
        if app.workshop.is_empty() {
            frame.text(
                "(no local workshop items)",
                Vec2::new(pos.x + 20.0 * scale, pos.y + 54.0 * scale),
                small,
                DIM,
            );
        }
        let buttons = [
            ("Refresh", crate::ui::ControlCommand::WorkshopRefresh, 820),
            ("Export", crate::ui::ControlCommand::WorkshopExport, 821),
            ("Delete", crate::ui::ControlCommand::WorkshopDelete, 822),
        ];
        let by = pos.y + panel_h - 52.0 * scale;
        for (index, (label, command, id)) in buttons.into_iter().enumerate() {
            let at = Vec2::new(pos.x + 20.0 * scale + index as f32 * 160.0 * scale, by);
            let size = Vec2::new(150.0 * scale, 32.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, at, size, [70, 74, 82, 255]);
            }
            frame.text(
                label,
                Vec2::new(at.x + 12.0 * scale, at.y + 6.0 * scale),
                small,
                TEXT,
            );
            frame.hit(id, at, size, UiAction::Invoke(command));
        }
        let extra = [
            ("Rename", crate::ui::ControlCommand::WorkshopRename, 827_u32),
            (
                "Share (Steam)",
                crate::ui::ControlCommand::WorkshopOnline,
                828,
            ),
            ("Update", crate::ui::ControlCommand::WorkshopOnline, 829),
            (
                "Unsubscribe",
                crate::ui::ControlCommand::WorkshopOnline,
                830,
            ),
            ("Submit", crate::ui::ControlCommand::WorkshopOnline, 831),
        ];
        let ey = pos.y + panel_h - 96.0 * scale;
        for (index, (label, command, id)) in extra.into_iter().enumerate() {
            let at = Vec2::new(pos.x + 20.0 * scale + index as f32 * 140.0 * scale, ey);
            let size = Vec2::new(130.0 * scale, 30.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, at, size, [70, 74, 82, 255]);
            }
            frame.text(
                label,
                Vec2::new(at.x + 8.0 * scale, at.y + 4.0 * scale),
                12.0 * scale,
                TEXT,
            );
            frame.hit(id, at, size, UiAction::Invoke(command));
        }
        let close = Vec2::new(pos.x + panel_w - 34.0 * scale, pos.y + 10.0 * scale);
        frame.image(ui, &["ui/button/close_x"], close, Vec2::splat(22.0 * scale));
        frame.hit(826, close, Vec2::splat(22.0 * scale), UiAction::ClosePanel);
    }

    // Achievements: local progress derived from the current world.
    if app.panels.is_open(crate::ui::PanelKind::Achievements) {
        let panel_w = 620.0 * scale;
        let panel_h = 430.0 * scale;
        let pos = Vec2::new((width - panel_w) * 0.5, (height - panel_h) * 0.5);
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(panel_w, panel_h));
        frame.bold(
            "ACHIEVEMENTS (local)",
            Vec2::new(pos.x + 20.0 * scale, pos.y + 12.0 * scale),
            heading,
            TEXT,
        );
        for (index, (name, achieved, progress)) in app.achievement_progress().iter().enumerate() {
            let y = pos.y + 52.0 * scale + index as f32 * 34.0 * scale;
            frame.text(
                name,
                Vec2::new(pos.x + 20.0 * scale, y),
                small,
                if *achieved { TEXT } else { DIM },
            );
            frame.text(
                progress,
                Vec2::new(pos.x + 250.0 * scale, y),
                small,
                if *achieved { [120, 200, 130, 255] } else { DIM },
            );
            frame.text(
                if *achieved { "unlocked" } else { "locked" },
                Vec2::new(pos.x + 520.0 * scale, y),
                small,
                if *achieved { [120, 200, 130, 255] } else { DIM },
            );
        }
        let close = Vec2::new(pos.x + panel_w - 34.0 * scale, pos.y + 10.0 * scale);
        frame.image(ui, &["ui/button/close_x"], close, Vec2::splat(22.0 * scale));
        frame.hit(700, close, Vec2::splat(22.0 * scale), UiAction::ClosePanel);
    }

    // Additional tools: population utilities.
    if app.panels.is_open(crate::ui::PanelKind::Tools) {
        let panel_w = 460.0 * scale;
        let panel_h = 220.0 * scale;
        let pos = Vec2::new((width - panel_w) * 0.5, (height - panel_h) * 0.5);
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(panel_w, panel_h));
        frame.bold(
            "ADDITIONAL TOOLS",
            Vec2::new(pos.x + 16.0 * scale, pos.y + 12.0 * scale),
            heading,
            TEXT,
        );
        let buttons = [
            (
                "Kill half population",
                UiAction::Invoke(crate::ui::ControlCommand::RequestKillHalf),
                710,
            ),
            (
                "Abort all eggs",
                UiAction::Invoke(crate::ui::ControlCommand::AbortEggs),
                711,
            ),
        ];
        let mut by = pos.y + 60.0 * scale;
        for (label, action, id) in buttons {
            let size = Vec2::new(330.0 * scale, 40.0 * scale);
            let at = Vec2::new(pos.x + 20.0 * scale, by);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, at, size, [90, 66, 60, 255]);
            }
            frame.text(
                label,
                Vec2::new(at.x + 12.0 * scale, at.y + 9.0 * scale),
                body,
                TEXT,
            );
            frame.hit(id, at, size, action);
            by += 56.0 * scale;
        }
        let close = Vec2::new(pos.x + panel_w - 34.0 * scale, pos.y + 10.0 * scale);
        frame.image(ui, &["ui/button/close_x"], close, Vec2::splat(22.0 * scale));
        frame.hit(712, close, Vec2::splat(22.0 * scale), UiAction::ClosePanel);
    }

    // Genome editor (library editor genes flow).
    if app.panels.is_open(crate::ui::PanelKind::GeneEditor)
        && let Some(genome) = &app.editor_genome
    {
        let panel_w = 760.0 * scale;
        let panel_h = 560.0 * scale;
        let pos = Vec2::new((width - panel_w) * 0.5, (height - panel_h) * 0.5);
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(panel_w, panel_h));
        frame.bold(
            "GENOME EDITOR",
            Vec2::new(pos.x + 20.0 * scale, pos.y + 12.0 * scale),
            heading,
            TEXT,
        );
        let rows = app.gene_editor_rows();
        let values = genome.values();
        let page_start = (app.editor_gene_page * 14).min(rows.len().saturating_sub(1));
        let page_end = (page_start + 14).min(rows.len());
        for (offset, (name, _)) in rows[page_start..page_end].iter().enumerate() {
            let index = page_start + offset;
            let at = Vec2::new(
                pos.x + 20.0 * scale,
                pos.y + 48.0 * scale + index as f32 * 30.0 * scale,
            );
            let selected = app.editor_gene == index;
            frame.rect(
                at,
                Vec2::new(470.0 * scale, 26.0 * scale),
                if selected {
                    [72, 64, 40, 255]
                } else {
                    [32, 35, 38, 255]
                },
            );
            frame.text(
                name,
                Vec2::new(at.x + 8.0 * scale, at.y + 4.0 * scale),
                13.0 * scale,
                if selected { TEXT } else { DIM },
            );
            let units = bibites_core::genome::ALL_GENES[index].def().units;
            frame.text(
                format!("{:.3} {units}", values[index]),
                Vec2::new(at.x + 300.0 * scale, at.y + 4.0 * scale),
                13.0 * scale,
                TEXT,
            );
            frame.hit(
                400 + offset as u32,
                at,
                Vec2::new(470.0 * scale, 26.0 * scale),
                UiAction::SelectGene(index as u32),
            );
        }
        let prev = Vec2::new(pos.x + 20.0 * scale, pos.y + panel_h - 44.0 * scale);
        let next = Vec2::new(prev.x + 170.0 * scale, prev.y);
        for (label, at, delta, id) in [
            ("Prev genes", prev, -1, 436_u32),
            ("Next genes", next, 1, 437),
        ] {
            let size = Vec2::new(150.0 * scale, 30.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, at, size, [70, 74, 82, 255]);
            }
            frame.text(
                label,
                Vec2::new(at.x + 10.0 * scale, at.y + 6.0 * scale),
                13.0 * scale,
                TEXT,
            );
            frame.hit(id, at, size, UiAction::ScrollGene(delta));
        }
        let minus = Vec2::new(pos.x + 520.0 * scale, pos.y + 48.0 * scale);
        let plus = Vec2::new(pos.x + 520.0 * scale, pos.y + 90.0 * scale);
        for (label, at, delta, id) in [("-", minus, -1, 430_u32), ("+", plus, 1, 431)] {
            let size = Vec2::new(60.0 * scale, 32.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, at, size, [70, 74, 82, 255]);
            }
            frame.bold(
                label,
                Vec2::new(at.x + 22.0 * scale, at.y + 6.0 * scale),
                body,
                TEXT,
            );
            frame.hit(id, at, size, UiAction::AdjustGene(delta));
        }
        let buttons = [
            ("Randomize", crate::ui::ControlCommand::EditorRandomize, 432),
            ("Mutate", crate::ui::ControlCommand::EditorMutate, 433),
            ("Save", crate::ui::ControlCommand::EditorSaveTemplate, 434),
        ];
        let bx = pos.x + 520.0 * scale;
        let mut by = pos.y + 150.0 * scale;
        for (label, command, id) in buttons {
            let size = Vec2::new(150.0 * scale, 32.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, Vec2::new(bx, by), size, [70, 74, 82, 255]);
            }
            frame.text(
                label,
                Vec2::new(bx + 10.0 * scale, by + 6.0 * scale),
                13.0 * scale,
                TEXT,
            );
            frame.hit(id, Vec2::new(bx, by), size, UiAction::Invoke(command));
            by += 42.0 * scale;
        }
        let close = Vec2::new(pos.x + panel_w - 34.0 * scale, pos.y + 10.0 * scale);
        frame.image(ui, &["ui/button/close_x"], close, Vec2::splat(22.0 * scale));
        frame.hit(435, close, Vec2::splat(22.0 * scale), UiAction::ClosePanel);
    }

    // Brain editor (library editor brain flow).
    if app.panels.is_open(crate::ui::PanelKind::BrainEditor)
        && let Some(brain) = &app.editor_brain
    {
        let panel_w = 860.0 * scale;
        let panel_h = 560.0 * scale;
        let pos = Vec2::new((width - panel_w) * 0.5, (height - panel_h) * 0.5);
        frame.panel(ui, POPUP_FRAME, pos, Vec2::new(panel_w, panel_h));
        frame.bold(
            "BRAIN EDITOR",
            Vec2::new(pos.x + 20.0 * scale, pos.y + 12.0 * scale),
            heading,
            TEXT,
        );
        let graph = brain.graph_view();
        let columns = 14_usize;
        let mut positions = Vec::new();
        for (index, node) in graph.nodes.iter().enumerate() {
            let at = if let Some((x, y)) = app.editor_anchors.get(&node.innovation) {
                Vec2::new(pos.x + x * scale, pos.y + y * scale)
            } else {
                let col = index % columns;
                let row = index / columns;
                Vec2::new(
                    pos.x + 30.0 * scale + col as f32 * 55.0 * scale,
                    pos.y + 60.0 * scale + row as f32 * 44.0 * scale,
                )
            };
            positions.push(at);
        }
        for synapse in &graph.synapses {
            let (Some(a), Some(b)) = (
                positions.get(synapse.node_in),
                positions.get(synapse.node_out),
            ) else {
                continue;
            };
            frame.rect(
                Vec2::new(a.x.min(b.x), a.y.min(b.y)),
                Vec2::new((b.x - a.x).abs().max(2.0), (b.y - a.y).abs().max(2.0)),
                [80, 90, 100, 120],
            );
        }
        for (index, _node) in graph.nodes.iter().enumerate() {
            let at = positions[index];
            let selected = app.editor_node == index;
            frame.rect(
                Vec2::new(at.x - 6.0 * scale, at.y - 6.0 * scale),
                Vec2::splat(12.0 * scale),
                if selected {
                    [240, 196, 92, 255]
                } else {
                    [96, 168, 120, 255]
                },
            );
            frame.hit(
                5000 + index as u32,
                Vec2::new(at.x - 8.0 * scale, at.y - 8.0 * scale),
                Vec2::splat(16.0 * scale),
                UiAction::SelectBrainNode(index as u32),
            );
        }
        let buttons = [
            ("Add Node", crate::ui::ControlCommand::EditorAddNode, 520),
            (
                "Add Synapse",
                crate::ui::ControlCommand::EditorAddSynapse,
                521,
            ),
            (
                "Remove Node",
                crate::ui::ControlCommand::EditorRemoveNode,
                522,
            ),
            ("Validate", crate::ui::ControlCommand::EditorValidate, 523),
            (
                "Anchor Node",
                crate::ui::ControlCommand::EditorAnchorNode,
                524,
            ),
            (
                "Clear Anchors",
                crate::ui::ControlCommand::EditorClearAnchors,
                525,
            ),
            ("Save", crate::ui::ControlCommand::EditorSaveTemplate, 526),
            ("Load", crate::ui::ControlCommand::EditorLoadTemplate, 527),
            (
                "Node Function",
                crate::ui::ControlCommand::EditorCycleNodeType,
                8040,
            ),
            ("Bias +", crate::ui::ControlCommand::EditorBiasUp, 8041),
            ("Bias -", crate::ui::ControlCommand::EditorBiasDown, 8042),
            ("Weight +", crate::ui::ControlCommand::EditorWeightUp, 8043),
            (
                "Weight -",
                crate::ui::ControlCommand::EditorWeightDown,
                8044,
            ),
            (
                "Toggle Link",
                crate::ui::ControlCommand::EditorToggleSynapse,
                8045,
            ),
            (
                "Remove Link",
                crate::ui::ControlCommand::EditorRemoveSynapse,
                8046,
            ),
        ];
        for (index, (label, command, id)) in buttons.into_iter().enumerate() {
            let at = Vec2::new(
                pos.x + 20.0 * scale + (index % 6) as f32 * 138.0 * scale,
                pos.y + panel_h - 136.0 * scale + (index / 6) as f32 * 40.0 * scale,
            );
            let size = Vec2::new(130.0 * scale, 32.0 * scale);
            if let Some(record) = ui.pick(&["ui/button/button", "ui/button/button_small"]) {
                frame.nine_slice_tinted(ui, record, at, size, [70, 74, 82, 255]);
            }
            frame.text(
                label,
                Vec2::new(at.x + 10.0 * scale, at.y + 6.0 * scale),
                13.0 * scale,
                TEXT,
            );
            frame.hit(id, at, size, UiAction::Invoke(command));
        }
        frame.text(
            &app.editor_validation,
            Vec2::new(pos.x + 20.0 * scale, pos.y + panel_h - 160.0 * scale),
            13.0 * scale,
            DIM,
        );
        let close = Vec2::new(pos.x + panel_w - 34.0 * scale, pos.y + 10.0 * scale);
        frame.image(ui, &["ui/button/close_x"], close, Vec2::splat(22.0 * scale));
        frame.hit(530, close, Vec2::splat(22.0 * scale), UiAction::ClosePanel);
    }

    // (The challenge's tracker is `challenge_tracker::draw`, the panel of the game's prefab.)
}

/// Panels that have no captured native session (Information, Graphs, the pause
/// menu). Drawn on top of both the curated HUD and the native runtime frame.
pub fn draw_extra_panels(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    width: f32,
    height: f32,
    native_dialogs: bool,
) {
    // "Show Targets": the labels of what the selected bibite's field of view sees hang in the world, under everything the HUD draws.
    if app.show_targets
        && app.scene_options.fov
        && let Some(targets) = app.selected.and_then(|id| app.world.fov_targets(id))
    {
        let labels = crate::target_weights::labels(&targets);
        if !labels.is_empty() {
            let mut under = UiFrame::new(frame.scale);
            crate::target_weights::draw(&mut under, ui, metrics, &app.camera, &labels);
            frame.draws.splice(0..0, under.draws);
        }
    }
    // The drag rectangle and the multi-selection panel sit under every other panel and modal.
    crate::selection_ui::draw(frame, app, ui, width, height);
    // The panel of a selected tower or colour killer.
    crate::tower_panel::draw(frame, app, ui, width, height);
    // The options of the bibite placer and the pellet placer while they are placing.
    crate::placer_panel::draw(frame, app, ui, width, height);
    // `Right Panels/QuickStatsHolder` (the Information panel, the challenge's tracker and its tab under "Info +") is inactive while the dynamic
    // settings panel is open (`DynamicSettingsPanel.onOpen`): it is neither drawn nor clicked then, and comes back when that panel closes.
    if !app.quick_stats_hidden() {
        if app.panels.is_open(crate::ui::PanelKind::Information) {
            crate::information_panel::draw(frame, app, ui, metrics, width, height);
        }
        // The challenge's tracker (`ChallengeInfo`) and its tab under "Info +".
        crate::challenge_tracker::draw(frame, app, ui, metrics, width, height);
    }
    if app.panels.is_open(crate::ui::PanelKind::Graphs) {
        draw_graphs(frame, ui, metrics, app, width, height);
    }
    crate::species_panel::emit(frame, app, ui, metrics, width, height);
    // The pop-up of the stomach, over the Biology panel while the pointer rests on the stomach.
    crate::biology_view::draw_popup(frame, app, ui, metrics, width, height);
    // The Expanded Brain View covers the screen (`Layer2`), over the Lineage panel.
    if let Some(session) = app.expanded_brain.as_ref() {
        let scale = UiFrame::scaler(width, height);
        let drawer = crate::species_panel::Drawer::new(scale, ui, metrics, app.cursor);
        crate::expanded_brain_ui::draw(frame, &drawer, app, session, width, height);
    }
    // The Bibite Editor covers the screen, over the panels and under the dialogs.
    if app.bibite_editor.is_some() {
        crate::template_editor_ui::draw(frame, app, ui, metrics, width, height);
    }
    // The Save Bibite dialog blocks the screen above every panel, the Species panel it opens from (`SpeciesInfoPanel.SaveSpecies`) included.
    crate::save_dialog::draw(frame, app, ui, width, height);
    draw_save_browser(frame, app, ui, width, height);
    draw_modals(frame, app, ui, width, height, native_dialogs);
    draw_escape_menu(frame, ui, metrics, app, width, height);
    // `TopLayer/Save Game Panel`: over the pause menu it was opened from.
    crate::save_panel::draw(frame, app, ui, metrics, width, height);
    // `TopLayer/Popuholder/ChallengeEndPopup`: over the pause menu, under the achievements' popup.
    crate::challenge_end::draw(frame, app, ui, metrics, width, height);
    // `TopLayer/TooltipCanvas/AchievementPopup`: over everything.
    {
        let scale = UiFrame::scaler(width, height);
        let drawer = crate::species_panel::Drawer::new(scale, ui, metrics, app.cursor);
        crate::achievements::draw(frame, &drawer, &app.achievements, width, height);
    }
}

/// `EscapePanel`: pause menu (the game pauses while it is open; level2 `Layer2/EscapePanel`). Every rectangle, font and sprite is read from a live
/// capture of the game at 1920 x 1080: a dark blue veil over the screen, and in its middle the 386.7 wide section (a `base_icon` frame with its
/// multiplier 0.54) with the version in bold at the top left, the close X at the top right and seven 50 high buttons 15 apart (`base_icon`, multiplier
/// 0.8): the label in Boston Black capitals and the button's icon 50 square at its right end.
fn draw_escape_menu(
    frame: &mut UiFrame,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    app: &App,
    width: f32,
    height: f32,
) {
    // The menu stays open under the panels its buttons open (`UserSettingsManager`, `SaveGamePanel`, `LoadGamePanel` on
    // `TopLayer`, the Library and the Bibite Editor after `EscapePanel` in `Layer2`): none of them closes it, they cover it, and
    // Escape (or their close button) shows it again. So it is only drawn, and only takes the clicks, while it is the newest panel.
    if app.panels.top() != Some(crate::ui::PanelKind::Escape) {
        return;
    }
    let s = UiFrame::scaler(width, height);
    // `ChallengeManager.objectsToHide`: Save Game and Load Game are not there in a challenge, and the section closes over them (65 per button: it is a
    // `ContentSizeFitter` around a vertical layout, centred on the screen).
    let hidden = if app.challenge_is_running() { 2 } else { 0 };
    let section_h = 528.0 - 65.0 * hidden as f32;
    let section = [766.667, 540.0 - section_h / 2.0, 386.667, section_h];
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    // `Panel`: a `PlatformWhiteSprite` tinted (0, 0, 0.106, 0.6) over everything.
    frame.rect(Vec2::ZERO, Vec2::new(width, height), [0, 0, 27, 153]);
    frame.hit(
        8100,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::Consume,
    );
    let slice = |frame: &mut UiFrame, rect: [f32; 4], dest_scale: f32| match ui.texture_of_alias("base_icon") {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [5.0; 4],
            dest_scale: dest_scale * s,
            pos: at(rect[0], rect[1]),
            size: dim(rect[2], rect[3]),
            tint: [255, 255, 255, 255],
        }),
        None => {
            frame.rect(at(rect[0], rect[1]), dim(rect[2], rect[3]), [20, 20, 20, 255]);
            frame.rect(
                at(rect[0] + 2.0, rect[1] + 2.0),
                dim(rect[2] - 4.0, rect[3] - 4.0),
                [227, 227, 227, 255],
            );
        }
    };
    let icon = |frame: &mut UiFrame, sprite: &str, rect: [f32; 4]| {
        if let Some(texture) = ui.texture_of_alias(sprite) {
            frame.draws.push(UiDraw::Image {
                texture,
                pos: at(rect[0], rect[1]),
                size: dim(rect[2], rect[3]),
                tint: [255, 255, 255, 255],
            });
        }
    };
    slice(frame, section, 1.0 / 0.54);
    // `VersionText`: LiberationSans SDF 25 Bold (a black face), 10 inside the section's top left corner, 28 high.
    tmp_label(
        frame,
        metrics,
        s,
        "LiberationSans SDF +bold",
        &format!("The Bibites {}", crate::app::GAME_VERSION),
        25.0,
        [0, 0, 0, 255],
        [section[0] + 10.0, section[1] + 10.0, 366.667, 28.0],
        false,
    );
    // `CloseButton`: the plain X (sprite 1727490), 40 square.
    let close = [section[0] + 336.667, section[1] + 10.0, 40.0, 40.0];
    icon(frame, "1727490", close);
    frame.hit(8021, at(close[0], close[1]), dim(close[2], close[3]), UiAction::ClosePanel);
    let entries: [(&str, &str, UiAction); 7] = [
        (
            "Save Game",
            "save_icon",
            UiAction::Invoke(crate::ui::ControlCommand::OpenSavePanel),
        ),
        (
            "Load Game",
            "import_icon",
            UiAction::Invoke(crate::ui::ControlCommand::OpenLoadPanel),
        ),
        (
            "Settings",
            "settings_icon",
            UiAction::Invoke(crate::ui::ControlCommand::OpenSettings),
        ),
        (
            "Bibite Library",
            "credits_icon",
            UiAction::Invoke(crate::ui::ControlCommand::OpenLibrary),
        ),
        (
            "Wiki",
            "help_icon",
            UiAction::Invoke(crate::ui::ControlCommand::OpenWiki),
        ),
        (
            "Back to Menu",
            "home_icon",
            UiAction::Invoke(crate::ui::ControlCommand::ReturnToMenu),
        ),
        ("Exit To Desktop", "exit_icon", UiAction::Quit),
    ];
    for (index, (label, sprite, action)) in entries.into_iter().skip(hidden).enumerate() {
        // `SettingsHolder` (68 under the section's top, 20 in from its left, 15 between its buttons): 346.7 x 50 each.
        let button = [section[0] + 20.0, section[1] + 68.0 + index as f32 * 65.0, 346.667, 50.0];
        slice(frame, button, 1.0 / 0.8);
        // `Text`: Boston Black SDF 23.05, upper case, dark (0.15), centred in the button less 10 each side.
        tmp_label(
            frame,
            metrics,
            s,
            "Boston Black SDF",
            &label.to_uppercase(),
            23.05,
            [38, 38, 38, 255],
            [button[0] + 10.0, button[1] + 10.0, 326.667, 30.0],
            true,
        );
        icon(frame, sprite, [button[0] + 296.667, button[1], 50.0, 50.0]);
        frame.hit(
            8030 + index as u32,
            at(button[0], button[1]),
            dim(button[2], button[3]),
            action,
        );
    }
}

/// A TMP label in the game's font `font` (a ` +bold` suffix asks for TextMeshPro's synthetic bold) at `px` reference px, middle aligned in `rect`
/// (reference px, `[x, y, w, h]`) and left aligned or centred. TMP centres a line on its glyphs' extent: the line height less the font's gap.
#[allow(clippy::too_many_arguments)]
fn tmp_label(
    frame: &mut UiFrame,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    s: f32,
    font: &str,
    text: &str,
    px: f32,
    color: Rgba,
    rect: [f32; 4],
    centred: bool,
) {
    tmp_label_aligned(frame, metrics, s, font, text, px, color, rect, if centred { 0.5 } else { 0.0 });
}

/// [`tmp_label`] with the line `align` of the way along the box's free width (0 left, 0.5 centred, 1 right).
#[allow(clippy::too_many_arguments)]
pub(crate) fn tmp_label_aligned(
    frame: &mut UiFrame,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    s: f32,
    font: &str,
    text: &str,
    px: f32,
    color: Rgba,
    rect: [f32; 4],
    align: f32,
) {
    use crate::ui::FontRole;
    let size = px * s;
    let (width, line, gap) = metrics.map_or(
        (text.chars().count() as f32 * size * 0.55, size * 1.2, 0.0),
        |fonts| {
            (
                fonts.measure_named(Some(font), FontRole::Body, text, size),
                fonts.line_height_named(Some(font), FontRole::Body, size),
                fonts.line_gap_named(Some(font), FontRole::Body, size),
            )
        },
    );
    let x = rect[0] * s + (rect[2] * s - width) * align;
    let y = rect[1] * s + (rect[3] * s - line + gap) * 0.5;
    frame.draws.push(UiDraw::Text {
        font: Some(font.to_owned()),
        text: text.to_owned(),
        pos: Vec2::new(x, y),
        px: size,
        color,
        role: FontRole::Body,
    });
}

/// A sliced `Image` over `rect` (reference px; `border` px of the sprite, drawn `scale` times as big), a flat light rectangle when the sprite is missing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sliced_image(
    frame: &mut UiFrame,
    ui: &UiAssets,
    s: f32,
    sprite: &str,
    border: f32,
    scale: f32,
    rect: [f32; 4],
    tint: Rgba,
) {
    let (pos, size) = (
        Vec2::new(rect[0] * s, rect[1] * s),
        Vec2::new(rect[2] * s, rect[3] * s),
    );
    match ui.texture_of_alias(sprite) {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [border; 4],
            dest_scale: scale * s,
            pos,
            size,
            tint,
        }),
        None => frame.rect(pos, size, crate::ui::multiply([227, 227, 227, 255], tint)),
    }
}

/// A plain `Image` over `rect` (reference px); nothing when the sprite is missing.
pub(crate) fn icon_image(frame: &mut UiFrame, ui: &UiAssets, s: f32, sprite: &str, rect: [f32; 4]) {
    if let Some(texture) = ui.texture_of_alias(sprite) {
        frame.draws.push(UiDraw::Image {
            texture,
            pos: Vec2::new(rect[0] * s, rect[1] * s),
            size: Vec2::new(rect[2] * s, rect[3] * s),
            tint: [255, 255, 255, 255],
        });
    }
}

/// "Stats and Analysis Panel" (`GraphsPanel` / `DataStreamLinesGraph`; level2 `Layer2/StatsPanel`): every rectangle, font and size is read from a live
/// capture of the game at 1920 x 1080 (`/d/ui`): the dark full-screen panel and its 60 high top bar (the page button, the title, the exit button), the
/// data / curve dropdowns, the six time-range tabs and the grey plot with its 26 x 11 grid, whose lines are 2 across. The y axis is rounded up to one
/// significant digit (`graph_axis_max`), the newest point is on the right and missing history is padded with zero.
fn draw_graphs(
    frame: &mut UiFrame,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    app: &App,
    width: f32,
    height: f32,
) {
    const N_X: usize = 26;
    const N_Y: usize = 11;
    const TAB_LABELS: [&str; 6] = ["1h", "10h", "2 days", "2 weeks", "4 months", "2 years"];
    // The prefab's fonts: the labels are `Roboto-Bold SDF NoOutline` (a white face; the dark ones are tinted 0.15), the tabs `LiberationSans SDF` (a black
    // face) and the title the bold `LiberationSans SDF - Fallback`.
    const ROBOTO: &str = "Roboto-Bold SDF NoOutline";
    const LIBERATION: &str = "LiberationSans SDF";
    const TITLE: &str = "LiberationSans SDF - Fallback +bold";
    let s = width / 1920.0;
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let sz = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let white: Rgba = [255, 255, 255, 255];
    let black: Rgba = [0, 0, 0, 255];
    let label_ink: Rgba = [38, 38, 38, 255];
    let label = |frame: &mut UiFrame, font: &str, text: &str, px: f32, color: Rgba, rect: [f32; 4], centred: bool| {
        tmp_label(frame, metrics, s, font, text, px, color, rect, centred);
    };
    // A sliced `Image` (`border` px of the sprite, drawn `scale` times as big), a flat light rectangle when the sprite is missing.
    let sliced = |frame: &mut UiFrame, sprite: &str, border: f32, scale: f32, rect: [f32; 4], tint: Rgba| {
        match ui.texture_of_alias(sprite) {
            Some(texture) => frame.draws.push(UiDraw::NineSlice {
                texture,
                border: [border; 4],
                dest_scale: scale * s,
                pos: at(rect[0], rect[1]),
                size: sz(rect[2], rect[3]),
                tint,
            }),
            None => frame.rect(
                at(rect[0], rect[1]),
                sz(rect[2], rect[3]),
                crate::ui::multiply([227, 227, 227, 255], tint),
            ),
        }
    };
    let icon = |frame: &mut UiFrame, sprite: &str, rect: [f32; 4]| {
        if let Some(texture) = ui.texture_of_alias(sprite) {
            frame.draws.push(UiDraw::Image {
                texture,
                pos: at(rect[0], rect[1]),
                size: sz(rect[2], rect[3]),
                tint: white,
            });
        }
    };
    frame.hit(
        8101,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::Consume,
    );
    frame.rect(Vec2::ZERO, Vec2::new(width, height), [44, 44, 44, 255]);
    frame.rect(at(0.0, 60.0), sz(1920.0, 1020.0), [47, 47, 47, 255]);
    // The title (a zero high box centred on the bar's middle, 30 down).
    label(frame, TITLE, "Stats and Analysis Panel", 24.0, white, [808.1, 30.0, 303.8, 0.0], true);
    // "Go To Gene Analysis" (`OpenGenes`) or, on that page, "Go To Data Graphs" (`OpenData`): each hides itself and shows the other. The button is its
    // label (Roboto-Bold 25) and 15 either side, 39.4 high, 10 from the left edge; its `Image` has the multiplier 0.75.
    let (button_label, to_genes) = if app.graphs_genes {
        ("Go To Data Graphs", false)
    } else {
        ("Go To Gene Analysis", true)
    };
    // (The game's "Go To Gene Analysis" is 228.6 wide: the Roboto-Bold pair table of the game gives 228.59.)
    let text_w = metrics.map_or(button_label.chars().count() as f32 * 25.0 * 0.55, |fonts| {
        fonts.measure_named(Some(ROBOTO), crate::ui::FontRole::Body, button_label, 25.0)
    });
    let button = [10.0, 10.3, text_w + 30.0, 39.4];
    sliced(frame, "base_icon", 5.0, 1.0 / 0.75, button, white);
    label(
        frame,
        ROBOTO,
        button_label,
        25.0,
        label_ink,
        [button[0] + 15.0, button[1] + 5.1, text_w, 29.3],
        false,
    );
    frame.hit(
        8102,
        at(button[0], button[1]),
        sz(button[2], button[3]),
        UiAction::GraphsPage(to_genes),
    );
    // The exit button: a 50 square at the top right (a `base_icon` frame, multiplier 0.75, with the `exit_icon` over it).
    let exit = [1865.0, 5.0, 50.0, 50.0];
    sliced(frame, "base_icon", 5.0, 1.0 / 0.75, exit, white);
    icon(frame, "exit_icon", exit);
    // (Its id is 8103, not 8100: that is the pause menu's, which hangs over this panel, and `UiFrame::interact` answers a click with every action of the
    // id it hit, so the veil of the pause menu closed the pause menu itself, as this button.)
    frame.hit(
        8103,
        at(exit[0], exit[1]),
        sz(exit[2], exit[3]),
        UiAction::ClosePanel,
    );
    if app.graphs_genes {
        crate::gene_analysis::draw(frame, ui, metrics, app, s);
        return;
    }

    let logger = &app.world.data_logger;
    let stream = app.graphs_stream_index();
    let group = &logger.groups[stream];
    let history = &group.history;
    let depth = app.graphs_scale.min(history.n_scales() - 1);
    let marks = app.graphs_curve_marks();

    // Data / Curves dropdowns (`RealDropdown`, `TogglesDropdown`): a click hangs their list. The name is Roboto-Bold 30 with 10 of margin, the box 400 x 40
    // 215 right of it (a `UISprite`, its caption Roboto-Bold 25 and the arrow in its right 40).
    let dropdown = |frame: &mut UiFrame, name: &str, name_x: f32, caption: &str, id: u32, target: DropdownTarget| {
        label(frame, ROBOTO, name, 30.0, white, [name_x + 10.0, 87.4, 190.0, 35.2], false);
        let boxed = [name_x + 215.0, 85.0, 400.0, 40.0];
        sliced(frame, "UISprite", 10.0, 0.5, boxed, white);
        label(frame, ROBOTO, caption, 25.0, label_ink, [boxed[0] + 10.0, 92.0, 365.0, 27.0], false);
        icon(frame, "DropdownArrow", [boxed[0] + 360.0, 85.0, 40.0, 40.0]);
        frame.hit(
            id,
            at(boxed[0], boxed[1]),
            sz(boxed[2], boxed[3]),
            UiAction::DropdownOpen(target),
        );
    };
    dropdown(frame, "Data:", 100.0, group.title, 8000, DropdownTarget::GraphsData);
    dropdown(
        frame,
        "Curves:",
        1205.0,
        &app.graphs_curves_label(),
        8010,
        DropdownTarget::GraphsCurves,
    );

    // Time-range tabs (scales 0..5; `Button`s 238.33 wide, 40 high, 288.33 apart): a scale without data is not interactable, and a disabled Button tints
    // its image with (0.784, 0.784, 0.784, 0.502) (`ColorBlock.disabledColor`).
    for (index, tab_label) in TAB_LABELS.iter().enumerate() {
        if index >= history.n_scales() {
            break;
        }
        let rect = [100.0 + index as f32 * 288.333, 140.0, 238.333, 40.0];
        let enabled = app.graphs_depth_has_data(index);
        let tint: Rgba = if enabled { white } else { [200, 200, 200, 128] };
        sliced(frame, "base_icon", 5.0, 1.0, rect, tint);
        label(frame, LIBERATION, tab_label, 24.0, black, rect, true);
        if enabled {
            frame.hit(
                8001 + index as u32,
                at(rect[0], rect[1]),
                sz(rect[2], rect[3]),
                UiAction::SetGraphsScale(index),
            );
        }
    }
    let legend = [1830.0, 140.0, 40.0, 40.0];
    sliced(frame, "base_icon", 5.0, 1.0, legend, white);
    icon(frame, "help_icon", legend);

    // Plot: `GraphBody` is a grey 1820 x 855 box; the curves live in `DataLines`, 1745 x 779 at (100, 201).
    frame.rect(at(50.0, 175.0), sz(1820.0, 855.0), [84, 84, 84, 255]);
    let plot = at(100.0, 201.0);
    let plot_size = sz(1745.0, 779.0);
    // `nPoints = DataSizeAtDepth + 1`: the logged slots and, last, the group's current value
    // (`PeekCurrentDataPoint`).
    let capacity = history.data_size_at_scale(depth).max(2);
    let n_data = history.n_data_at_scale(depth);
    let n_points = capacity + 1;
    let current = app
        .world
        .data_points()
        .into_iter()
        .nth(stream)
        .unwrap_or_default();
    // Value of curve `j` at slot `k` (0 = oldest slot, newest last).
    let value_at = |j: usize, k: usize| -> f32 {
        if k + 1 == n_points {
            return current.get(j).copied().unwrap_or(0.0);
        }
        let from_newest = n_points - 2 - k;
        if from_newest < n_data {
            history.get(depth, from_newest)[j]
        } else {
            0.0
        }
    };
    // The axis fits the curves that show.
    let shown = |j: usize| marks.get(j).copied().unwrap_or(true);
    let mut max_value = 0.0_f32;
    for j in (0..group.streams.len()).filter(|j| shown(*j)) {
        for k in 0..n_points {
            max_value = max_value.max(value_at(j, k));
        }
    }
    let y_max = bibites_core::datalog::graph_axis_max(max_value);
    let format = graph_format(stream);
    let minutes_per_point = history.scale_step(depth) as f32;
    let span_minutes = minutes_per_point * capacity as f32;
    let (span_value, span_unit) = if span_minutes >= 60.0 * 100.0 {
        (span_minutes / 1440.0, "d")
    } else {
        (span_minutes / 60.0, "h")
    };
    // The grid is `Shapes` lines 2 across, drawn by the pixels whose middle they cover (the 1.98 keeps a line on a pixel boundary 2 wide and one on a
    // pixel middle 1): 11 rows of (10, 10, 10) from the zero line (980) up to the top one (201), 77.9 apart, and 26 black columns from x 101 to 1844,
    // 69.72 apart, from y 214 for 780. The axes' labels are Roboto-Bold 11.05 / 12, white, in 50 wide boxes centred on the lines.
    let line_w = 1.98;
    for r in 0..N_Y {
        let c = 980.0 - 77.9 * r as f32;
        frame.rect(at(101.0, c - line_w / 2.0), sz(1745.0, line_w), [10, 10, 10, 255]);
    }
    for k in 0..N_X {
        let c = 101.0 + 69.72 * k as f32;
        frame.rect(at(c - line_w / 2.0, 214.0), sz(line_w, 780.0), [0, 0, 0, 255]);
        let fraction = k as f32 / (N_X - 1) as f32;
        label(
            frame,
            ROBOTO,
            &format!("{:.2} {span_unit}", span_value * (1.0 - fraction)),
            12.0,
            white,
            [c - 25.0, 987.5, 50.0, 15.0],
            true,
        );
    }
    for r in 0..N_Y {
        let c = 201.0 + 77.9 * r as f32;
        let fraction = r as f32 / (N_Y - 1) as f32;
        label(
            frame,
            ROBOTO,
            &format.format(y_max * (1.0 - fraction)),
            11.05,
            white,
            [50.0, c - 6.5, 50.0, 13.0],
            true,
        );
    }
    // `Color.yellow` and the others are the game's float colours, rounded to a byte.
    let to_color = |c: [f32; 3]| -> Rgba {
        [
            (c[0] * 255.0).round() as u8,
            (c[1] * 255.0).round() as u8,
            (c[2] * 255.0).round() as u8,
            255,
        ]
    };
    // The curves are `Polyline`s 2 across with flat ends: the first curve of a group lies over the others (the four zero lines of a new world show the
    // first one's colour).
    for (j, description) in group.streams.iter().enumerate().rev() {
        if !shown(j) {
            continue;
        }
        let color = to_color(description.color);
        let point = |k: usize| -> Vec2 {
            Vec2::new(
                plot.x + plot_size.x * k as f32 / (n_points - 1) as f32,
                plot.y + plot_size.y * (1.0 - (value_at(j, k) / y_max).clamp(0.0, 1.0)),
            )
        };
        for k in 0..n_points - 1 {
            let (a, b) = (point(k), point(k + 1));
            let steps = ((b - a).length() / 2.0).ceil().max(1.0) as usize;
            for step in 0..=steps {
                let p = a + (b - a) * (step as f32 / steps as f32);
                let (left, right) = (p.x - 1.0, p.x + 1.0);
                if left < plot.x || right > plot.x + plot_size.x {
                    let (left, right) = (left.max(plot.x), right.min(plot.x + plot_size.x));
                    frame.rect(Vec2::new(left, p.y - 1.0), Vec2::new(right - left, 2.0), color);
                } else {
                    frame.rect(p - Vec2::splat(1.0), Vec2::splat(2.0), color);
                }
            }
        }
    }

    // `GraphTooltipTrigger` -> `BaseLineGraph.UpdateTooltip`: while the pointer is over the plot a white line (3 thick) stands at the
    // point nearest to it, and each curve that shows has a dot on it (10 across, the curve's colour) with its value over its upper
    // right (the prefab's `FloatInfoText`: 16 pt bold white, its bottom left corner 5 right of and above the dot's centre).
    // A list that is open (its blocker) takes the pointer away from the plot.
    let over_plot = app.cursor.x >= plot.x
        && app.cursor.x < plot.x + plot_size.x
        && app.cursor.y >= plot.y
        && app.cursor.y < plot.y + plot_size.y;
    if over_plot && app.dropdown_open.is_none() {
        let xs: Vec<f32> = (0..n_points)
            .map(|k| plot_size.x * k as f32 / (n_points - 1) as f32)
            .collect();
        let k = closest_index(&xs, app.cursor.x - plot.x).min(n_points - 1);
        let x = plot.x + xs[k];
        frame.rect(
            Vec2::new(x - 1.5 * s, plot.y),
            Vec2::new(3.0 * s, plot_size.y),
            white,
        );
        for (j, description) in group.streams.iter().enumerate() {
            if !shown(j) {
                continue;
            }
            let value = value_at(j, k);
            let y = plot.y + plot_size.y * (1.0 - (value / y_max).clamp(0.0, 1.0));
            let color = to_color(description.color);
            let diameter = (10.0 * s).round().max(2.0) as u32;
            frame.draws.push(UiDraw::Pixels {
                texture: std::sync::Arc::new(disc_texture(diameter, color)),
                pos: Vec2::new(x - diameter as f32 / 2.0, y - diameter as f32 / 2.0),
                size: Vec2::splat(diameter as f32),
            });
            let px = 16.0 * s;
            frame.bold(
                format.format(value),
                Vec2::new(x + 5.0 * s, y - 5.0 * s - px * 1.17),
                px,
                white,
            );
        }
    }

    // The legend of the curves that show hangs under the "?" button while the pointer rests on
    // the button or the legend (`OnHoverShow`, `OnHoverKeep`).
    if app.graphs_legend_open {
        let (pos, size) = graphs_legend_rect(app, s);
        frame.rect(pos, size, [255, 255, 255, 242]);
        let rows = graphs_legend_rows(app);
        let row_h = LEGEND_ROW * s;
        for (row, (_, description)) in rows.iter().enumerate() {
            let top = pos.y + (LEGEND_PAD_TOP + row as f32 * (LEGEND_ROW + LEGEND_SPACING)) * s;
            // The swatch: a white frame (40 x 14) around the curve's colour.
            let swatch = Vec2::new(
                pos.x + (LEGEND_PAD_LEFT + 5.0) * s,
                top + (LEGEND_ROW - 14.0) * 0.5 * s,
            );
            frame.rect(swatch, sz(40.0, 14.0), [200, 200, 200, 255]);
            frame.rect(
                swatch + sz(3.0, 3.0),
                sz(34.0, 8.0),
                to_color(description.color),
            );
            frame.bold(
                description.label,
                Vec2::new(swatch.x + 50.0 * s, top + 2.0 * s),
                20.0 * s,
                label_ink,
            );
            frame.tips.push(crate::tooltip::TipRect {
                pos: Vec2::new(pos.x, top),
                size: Vec2::new(size.x, row_h),
                tip: crate::tooltip::Tip {
                    path: "",
                    title: description.label,
                    content: description.description,
                    ui: true,
                    instant: false,
                },
            });
        }
    }
}

/// `ListExtend.IndexOfClosest`: the index of the value nearest to `value`; of equal distances the LAST one wins (`!(best < distance)`).
fn closest_index(values: &[f32], value: f32) -> usize {
    let mut best = f32::INFINITY;
    let mut result = 0;
    for (index, candidate) in values.iter().enumerate() {
        let distance = (candidate - value).abs();
        if !(best < distance) {
            best = distance;
            result = index;
        }
    }
    result
}

/// A filled circle `diameter` px across (the `Disc` of a graph's dot), its edge one pixel soft.
pub(crate) fn disc_texture(diameter: u32, color: Rgba) -> crate::ui::UiTexture {
    let radius = diameter as f32 / 2.0;
    let mut rgba = Vec::with_capacity((diameter * diameter * 4) as usize);
    for row in 0..diameter {
        for column in 0..diameter {
            let (dx, dy) = (column as f32 + 0.5 - radius, row as f32 + 0.5 - radius);
            let coverage = (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[
                color[0],
                color[1],
                color[2],
                (f32::from(color[3]) * coverage).round() as u8,
            ]);
        }
    }
    crate::ui::UiTexture {
        width: diameter,
        height: diameter,
        rgba,
    }
}

/// `DataLogger` textFormat of each group: biomass in `E` (SI, 2 digits), the ages in hours, the
/// rest whole numbers (the counts and the brain sizes with SI prefixes, the eggs and the
/// births / deaths without).
fn graph_format(group: usize) -> bibites_core::format::FloatValueFormat {
    use bibites_core::datalog::group as g;
    use bibites_core::format::FloatValueFormat;
    match group {
        g::BIOMASS => FloatValueFormat {
            units: "E",
            precision: 2,
            ..FloatValueFormat::default()
        },
        g::AGE | g::DEATH_AGE => FloatValueFormat {
            units: "h",
            precision: 2,
            si: false,
            ..FloatValueFormat::default()
        },
        g::EGGS_LAID => FloatValueFormat {
            is_int: true,
            si: false,
            ..FloatValueFormat::default()
        },
        g::BIRTH_DEATH => FloatValueFormat {
            is_int: true,
            si: false,
            precision: 0,
            ..FloatValueFormat::default()
        },
        _ => FloatValueFormat {
            is_int: true,
            ..FloatValueFormat::default()
        },
    }
}

/// The legend rows: the curves of the plotted group that show.
fn graphs_legend_rows(app: &App) -> Vec<(usize, &bibites_core::datalog::StreamDescription)> {
    let marks = app.graphs_curve_marks();
    app.world.data_logger.groups[app.graphs_stream_index()]
        .streams
        .iter()
        .enumerate()
        .filter(|(j, _)| marks.get(*j).copied().unwrap_or(true))
        .collect()
}

/// `legendItem`: 27.44 high (2 + 23.44 + 2 around a 20-point label), `Legend`'s padding (left 10,
/// right 20, top and bottom 8) and spacing (3).
const LEGEND_ROW: f32 = 27.44;
const LEGEND_SPACING: f32 = 3.0;
const LEGEND_PAD_LEFT: f32 = 10.0;
const LEGEND_PAD_RIGHT: f32 = 20.0;
const LEGEND_PAD_TOP: f32 = 8.0;

/// The "?" button that shows the legend (position and size in px at width-scale `s`).
#[must_use]
fn graphs_legend_button(s: f32) -> (Vec2, Vec2) {
    (
        Vec2::new(1830.0 * s, 140.0 * s),
        Vec2::new(40.0 * s, 40.0 * s),
    )
}

/// The legend (`Legend`: anchored to the plot's top-right corner, fitted to its rows): wide enough
/// for the widest label next to its 40-wide swatch (5 + 40 + 10 + text + 5), `n` rows high. The label
/// width is an estimate of Roboto-Bold at 20 (the prefab's "Max Nodes" is 100.8 wide).
#[must_use]
fn graphs_legend_rect(app: &App, s: f32) -> (Vec2, Vec2) {
    let rows = graphs_legend_rows(app);
    let text = rows
        .iter()
        .map(|(_, description)| description.label.chars().count() as f32 * 11.2)
        .fold(0.0_f32, f32::max);
    let width = LEGEND_PAD_LEFT + LEGEND_PAD_RIGHT + 5.0 + 40.0 + 10.0 + text + 5.0;
    let n = rows.len() as f32;
    let height = 2.0 * LEGEND_PAD_TOP + n * LEGEND_ROW + (n - 1.0).max(0.0) * LEGEND_SPACING;
    // The right edge is the plot's (1870), the top its top (175).
    (
        Vec2::new((1870.0 - width) * s, 175.0 * s),
        Vec2::new(width * s, height * s),
    )
}

/// Whether the pointer rests on the "?" button or, once the legend shows, on the legend.
#[must_use]
pub fn graphs_legend_hovered(app: &App, cursor: Vec2, shown: bool, width: f32) -> bool {
    let s = width / 1920.0;
    let inside = |(pos, size): (Vec2, Vec2)| {
        cursor.x >= pos.x
            && cursor.x < pos.x + size.x
            && cursor.y >= pos.y
            && cursor.y < pos.y + size.y
    };
    inside(graphs_legend_button(s)) || (shown && inside(graphs_legend_rect(app, s)))
}

/// `BibiteStatsPanel.UpdateBibiteStats` (cs:304-316) rows, formatted with the
/// game's `ScientificFormat` (units are a best guess: the per-field formats
/// live in prefab data that is not decompiled).
fn stats_rows(panel: &crate::app::SelectionPanel) -> Vec<(&'static str, String)> {
    use bibites_core::format::FloatValueFormat;
    let plain = |units: &'static str| FloatValueFormat {
        units,
        precision: 3,
        precision_is_si: true,
        ..FloatValueFormat::default()
    };
    let int = |units: &'static str| FloatValueFormat {
        units,
        precision: 3,
        is_int: true,
        ..FloatValueFormat::default()
    };
    vec![
        ("generation", int("").format(panel.generation as f32)),
        ("length", plain(" u").format(panel.body_length)),
        ("size", plain(" u2").format(panel.real_body_area)),
        ("mass", plain(" g").format(panel.mass)),
        ("speed", plain(" u/s").format(panel.signed_velocity)),
        ("time alive", plain(" s").format(panel.age_seconds)),
        ("travelled", plain(" u").format(panel.travelled)),
        ("eggs laid", int("").format(panel.eggs_laid as f32)),
        ("children", int("").format(panel.alive_children as f32)),
        ("bitten", int("").format(panel.bibites_bitten as f32)),
        ("dmg dealt", plain("").format(panel.damage_dealt)),
        ("attacked", int("").format(panel.times_attacked as f32)),
        ("dmg taken", plain("").format(panel.damage_received)),
    ]
}

/// Draws a status bar and its label; returns the next y baseline.
#[allow(clippy::too_many_arguments)]
fn bar(
    frame: &mut UiFrame,
    ui: &UiAssets,
    x: f32,
    y: f32,
    width: f32,
    scale: f32,
    label: &str,
    fraction: f32,
    fill_candidates: &[&str],
    fill_tint: Rgba,
    value: String,
) -> f32 {
    let frame_h = 18.0 * scale;
    frame.text(label, Vec2::new(x, y), 13.0 * scale, DIM);
    let bar_pos = Vec2::new(x + 66.0 * scale, y);
    let bar_size = Vec2::new(width - 66.0 * scale, frame_h);
    let Some(record) = ui.pick(BAR_FRAME) else {
        return y + 24.0 * scale;
    };
    frame.nine_slice_tinted(
        ui,
        record,
        bar_pos,
        bar_size,
        crate::ui::multiply(
            record.default_tint.unwrap_or([255, 255, 255, 255]),
            [62, 64, 74, 240],
        ),
    );
    let fill_width = (bar_size.x - 4.0 * scale) * fraction.clamp(0.0, 1.0);
    if fill_width > 1.0
        && frame.image(
            ui,
            fill_candidates,
            Vec2::new(bar_pos.x + 2.0 * scale, bar_pos.y + 2.0 * scale),
            Vec2::new(fill_width, bar_size.y - 4.0 * scale),
        )
    {
        // Record the fill tint by patching the last draw (image() used the
        // record's default tint).
        if let Some(crate::ui::UiDraw::Image { tint, .. }) = frame.draws.last_mut() {
            *tint = fill_tint;
        }
    }
    frame.text(
        value,
        Vec2::new(bar_pos.x + 6.0 * scale, y + 2.0 * scale),
        12.0 * scale,
        TEXT,
    );
    y + 24.0 * scale
}

fn on_off(value: bool) -> &'static str {
    if value { "On" } else { "Off" }
}

fn ratio(value: f32, max: f32) -> f32 {
    if max > 0.0 {
        (value / max).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app::{FrameInput, LoadedWorld};
    use crate::ui::UiDraw;
    use bibites_core::datalog::group;

    fn graphs_app() -> App {
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        app.panels.open(crate::ui::PanelKind::Graphs);
        app
    }

    fn graphs_frame(app: &App) -> UiFrame {
        let mut frame = UiFrame::new(1.0);
        draw_graphs(&mut frame, &UiAssets::empty(), None, app, 1920.0, 1080.0);
        frame
    }

    /// The texts of the frame (the axis labels carry a trailing space where the unit would be).
    fn texts(frame: &UiFrame) -> Vec<String> {
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.trim().to_owned()),
                _ => None,
            })
            .collect()
    }

    /// How many 2 x 2 dots of `color` the plot has.
    fn dots(frame: &UiFrame, color: Rgba) -> usize {
        frame
            .draws
            .iter()
            .filter(|draw| matches!(draw, UiDraw::Rect { size, color: c, .. } if *c == color && size.x == 2.0))
            .count()
    }

    /// Logs `points` (one vector per group, the ones given) `times` times.
    fn log(app: &mut App, points: &[Vec<f32>], times: usize) {
        for _ in 0..times {
            app.world.data_logger.log(points);
        }
    }

    #[test]
    fn the_data_and_curves_boxes_carry_the_groups_title_and_open_their_lists() {
        let mut app = graphs_app();
        let frame = graphs_frame(&app);
        let find = |target: crate::ui::DropdownTarget| {
            frame
                .actions
                .iter()
                .find(|(_, action)| *action == UiAction::DropdownOpen(target))
                .and_then(|(id, _)| frame.hits.iter().find(|hit| hit.0 == *id))
                .map(|hit| (hit.1, hit.2))
        };
        let (data_at, data_size) = find(DropdownTarget::GraphsData).expect("the Data box");
        assert_eq!(
            (data_at.x, data_at.y, data_size.x, data_size.y),
            (315.0, 85.0, 400.0, 40.0)
        );
        let (curves_at, _) = find(DropdownTarget::GraphsCurves).expect("the Curves box");
        assert_eq!((curves_at.x, curves_at.y), (1420.0, 85.0));
        let shown = texts(&frame);
        assert!(shown.contains(&"Biomass Data".to_owned()));
        assert!(shown.contains(&"All".to_owned()));
        // The caption follows the group and the curves that show.
        app.set_graphs_stream(5);
        app.toggle_graphs_curve(0);
        let shown = texts(&graphs_frame(&app));
        assert!(shown.contains(&"Egg Laying Data".to_owned()));
        assert!(shown.contains(&"5 Selected".to_owned()), "{shown:?}");
    }

    #[test]
    fn only_the_curves_that_show_are_plotted_and_the_axis_fits_them() {
        let mut app = graphs_app();
        // Plants (green) far above everything else, then a few minutes of history.
        let counts = vec![vec![100.0, 0.0, 0.0, 0.0], vec![100.0, 50_000.0, 20.0]];
        log(&mut app, &counts, 5);
        let green = [0, 255, 0, 255];
        let red = [255, 0, 0, 255];
        let all = graphs_frame(&app);
        assert!(
            dots(&all, [255, 235, 4, 255]) > 0,
            "the Free curve of the biomass group"
        );
        // Hiding the Free curve of the biomass group removes its dots, not the others.
        app.toggle_graphs_curve(3);
        let hidden = graphs_frame(&app);
        assert_eq!(dots(&hidden, [255, 235, 4, 255]), 0);
        assert!(dots(&hidden, green) > 0 && dots(&hidden, red) > 0);
        // The count group: with the plants plotted the axis tops at 50 k, without them at the
        // bibites' 100.
        app.set_graphs_stream(group::COUNT);
        let with_plants = texts(&graphs_frame(&app));
        assert!(!with_plants.contains(&"100".to_owned()), "{with_plants:?}");
        app.toggle_graphs_curve(1);
        let without = texts(&graphs_frame(&app));
        assert!(without.contains(&"100".to_owned()), "{without:?}");
        assert!(without.contains(&"90".to_owned()), "{without:?}");
        // Nothing plotted at all: the axis is the default 0..1 (`Approximately(max, 0) -> 1`).
        app.toggle_graphs_curve(0);
        app.toggle_graphs_curve(2);
        let none = graphs_frame(&app);
        assert_eq!(dots(&none, green), 0);
        assert!(texts(&none).contains(&"1".to_owned()));
    }

    #[test]
    fn the_pointer_over_the_plot_shows_a_line_and_the_values_at_the_nearest_point() {
        let mut app = graphs_app();
        let counts = vec![vec![100.0, 0.0, 0.0, 0.0], vec![100.0, 5_000.0, 20.0]];
        log(&mut app, &counts, 5);
        app.set_graphs_stream(group::COUNT);
        let plot_x = 100.0;
        let plot_w = 1745.0;
        let plot_y = 201.0;
        // Away from the plot: no line, no dots (a white 3-wide rectangle is the line).
        let line = |frame: &UiFrame| -> Option<(Vec2, Vec2)> {
            frame.draws.iter().find_map(|draw| match draw {
                UiDraw::Rect { pos, size, color }
                    if *color == [255, 255, 255, 255] && size.x == 3.0 =>
                {
                    Some((*pos, *size))
                }
                _ => None,
            })
        };
        app.cursor = Vec2::new(50.0, 100.0);
        assert!(line(&graphs_frame(&app)).is_none());
        // Over the plot at its very left: the line is at the first point, spanning the plot.
        app.cursor = Vec2::new(plot_x + 3.0, plot_y + 100.0);
        let frame = graphs_frame(&app);
        let (at, size) = line(&frame).expect("the indicator line");
        assert!((at.x + 1.5 - plot_x).abs() < 1e-3, "{at:?}");
        assert_eq!((at.y, size.y), (plot_y, 779.0));
        // The values of the curves that show, in the group's format, sit by dots of the curves' colours.
        let shown = texts(&frame);
        // The oldest point has no history (the plot is padded with zero), so its values are 0.
        assert!(
            shown.iter().filter(|text| *text == "0").count() >= 3,
            "{shown:?}"
        );
        let discs = frame
            .draws
            .iter()
            .filter(|draw| matches!(draw, UiDraw::Pixels { size, .. } if size.x == 10.0))
            .count();
        assert_eq!(discs, 3, "one dot per curve that shows");
        // At the right end the line stands at the newest point, the group's current value: the bibites and the plants.
        app.cursor = Vec2::new(plot_x + plot_w - 2.0, plot_y + 100.0);
        let frame = graphs_frame(&app);
        let (at, _) = line(&frame).expect("the indicator line");
        assert!((at.x + 1.5 - (plot_x + plot_w)).abs() < 1e-3, "{at:?}");
        let shown = texts(&frame);
        let current = app.world.data_points()[group::COUNT].clone();
        let bibites = graph_format(group::COUNT)
            .format(current[0])
            .trim()
            .to_owned();
        assert!(shown.contains(&bibites), "the bibites {bibites}: {shown:?}");
        // A curve that is switched off has no dot; an open list takes the pointer away from the plot.
        app.toggle_graphs_curve(1);
        let discs_after = graphs_frame(&app)
            .draws
            .iter()
            .filter(|draw| matches!(draw, UiDraw::Pixels { size, .. } if size.x == 10.0))
            .count();
        assert_eq!(discs_after, 2);
        // An open list (its blocker covers the plot) takes the pointer away from it.
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::GraphsData)]);
        assert!(app.dropdown_open.is_some());
        assert!(line(&graphs_frame(&app)).is_none());
    }

    /// How many white bars (rectangles more than 2 wide and 3 high, as the dots and lines are not) the frame has.
    fn bars(frame: &UiFrame) -> Vec<(Vec2, Vec2)> {
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Rect { pos, size, color }
                    if *color == [255, 255, 255, 255] && size.x > 3.0 && size.y > 3.0 =>
                {
                    Some((*pos, *size))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_page_button_swaps_between_the_data_graphs_and_the_gene_analysis() {
        let mut app = graphs_app();
        let frame = graphs_frame(&app);
        assert!(texts(&frame).contains(&"Go To Gene Analysis".to_owned()));
        assert!(
            frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::GraphsPage(true))
        );
        app.apply_ui_actions(&[UiAction::GraphsPage(true)]);
        let frame = graphs_frame(&app);
        let shown = texts(&frame);
        for expected in [
            "Go To Data Graphs",
            "Gene Distribution:",
            "Diet",
            "Time:",
            "Now",
        ] {
            assert!(
                shown.contains(&expected.to_owned()),
                "{expected}: {shown:?}"
            );
        }
        // The data page's controls are gone; its button is the way back.
        assert!(!shown.contains(&"Data:".to_owned()));
        assert!(
            frame
                .actions
                .iter()
                .all(|(_, action)| !matches!(action, UiAction::SetGraphsScale(_)))
        );
        assert!(
            frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::GraphsPage(false))
        );
        app.apply_ui_actions(&[UiAction::GraphsPage(false)]);
        assert!(texts(&graphs_frame(&app)).contains(&"Data:".to_owned()));
    }

    #[test]
    fn the_gene_page_has_the_dropdown_the_grid_with_the_genes_values_and_the_counts_axis() {
        let mut app = graphs_app();
        app.apply_ui_actions(&[UiAction::GraphsPage(true)]);
        let frame = graphs_frame(&app);
        let shown = texts(&frame);
        // 26 labels across the Diet gene's whole 0 to 1 range, in its own format (2 digits, no units).
        for label in ["0.00", "0.04", "0.40", "0.52", "1.00"] {
            assert!(shown.contains(&label.to_owned()), "{label}: {shown:?}");
        }
        // 11 whole numbers up the counts axis, which tops at 10 for a handful of bibites.
        for label in ["0", "1", "5", "10"] {
            assert!(shown.contains(&label.to_owned()), "{label}: {shown:?}");
        }
        // The dropdown box is where the prefab has it and opens the genes list.
        let (_, at, size) = frame
            .actions
            .iter()
            .find(|(_, action)| *action == UiAction::DropdownOpen(DropdownTarget::GraphsGene))
            .and_then(|(id, _)| frame.hits.iter().find(|hit| hit.0 == *id))
            .copied()
            .expect("the gene box");
        assert_eq!((at.x, at.y, size.x, size.y), (365.0, 85.1, 250.0, 35.0));
        // The slider has its handle (on top) and its track, both ending where the slider does.
        let slides: Vec<UiAction> = frame
            .actions
            .iter()
            .map(|(_, action)| *action)
            .filter(|action| {
                matches!(
                    action,
                    UiAction::GenesTimeSlide(..) | UiAction::GenesTimeSlideHandle(..)
                )
            })
            .collect();
        assert_eq!(
            slides,
            [
                UiAction::GenesTimeSlideHandle(250, 1670),
                UiAction::GenesTimeSlide(250, 1670)
            ]
        );
    }

    #[test]
    fn the_gene_page_draws_the_population_as_bars_and_a_logged_hour_as_it_was() {
        let mut app = graphs_app();
        app.apply_ui_actions(&[UiAction::GraphsPage(true)]);
        // Now: every living bibite and egg is in one of the bars; the bars stand on the plot's bottom (1 above it).
        let now = app.world.current_gene_point(16);
        let population: u32 = now.data.iter().map(|count| u32::from(*count)).sum();
        assert!(population >= 8, "the starter world's bibites: {population}");
        let frame = graphs_frame(&app);
        let drawn = bars(&frame);
        assert_eq!(
            drawn.len(),
            now.data.iter().filter(|count| **count > 0).count()
        );
        for (pos, size) in &drawn {
            assert!(
                (pos.y + size.y - (156.16 + 778.68 - 1.0)).abs() < 1e-2,
                "{pos:?} {size:?}"
            );
        }
        // An hour of the genes stream: 5 bibites that were all in one bucket; "1h ago" is the slider's text.
        let hour = vec![bibites_core::genome::Genome::defaults().genes; 5];
        app.world
            .data_logger
            .genes
            .push(bibites_core::genes_log::GenesBuckets::from_genes(&hour));
        app.graphs_gene_set_time(0);
        let frame = graphs_frame(&app);
        assert!(texts(&frame).contains(&"1h ago".to_owned()));
        let drawn = bars(&frame);
        assert_eq!(drawn.len(), 1, "one bucket holds the five");
        // 5 of the axis' 10: half the plot's height, 1.. wide in the bucket's place on the gene's 0 to 1 axis.
        assert!((drawn[0].1.y - 0.5 * 778.68).abs() < 1e-2, "{:?}", drawn[0]);
    }

    #[test]
    fn the_bar_under_the_pointer_has_the_values_it_covers_its_count_and_its_share() {
        let mut app = graphs_app();
        app.apply_ui_actions(&[UiAction::GraphsPage(true)]);
        let hour = vec![bibites_core::genome::Genome::defaults().genes; 8];
        app.world
            .data_logger
            .genes
            .push(bibites_core::genes_log::GenesBuckets::from_genes(&hour));
        app.graphs_gene_set_time(0);
        let frame = graphs_frame(&app);
        assert!(frame.tips.is_empty(), "nothing is hovered");
        let (pos, size) = bars(&frame)[0];
        app.cursor = Vec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0);
        let frame = graphs_frame(&app);
        assert_eq!(frame.tips.len(), 1);
        let tip = frame.tips[0].tip;
        // The default Diet gene is 0.5 (Genome::defaults): the bucket of the 0.05 wide range that holds it.
        assert!(
            tip.title.starts_with('[') && tip.title.contains(" : ") && tip.title.ends_with(']'),
            "{}",
            tip.title
        );
        assert_eq!(tip.content, "count: 8 (100.0%)");
        // An open list takes the pointer away from the bars.
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::GraphsGene)]);
        assert!(graphs_frame(&app).tips.is_empty());
    }

    #[test]
    fn the_nearest_point_is_the_last_of_equal_distances() {
        // `IndexOfClosest` keeps replacing the best while `!(best < distance)`, so a tie goes to the later index.
        assert_eq!(closest_index(&[0.0, 10.0, 20.0], 5.0), 1);
        assert_eq!(closest_index(&[0.0, 10.0, 20.0], 4.0), 0);
        assert_eq!(closest_index(&[0.0, 10.0, 20.0], 99.0), 2);
        assert_eq!(closest_index(&[0.0, 10.0, 20.0], -3.0), 0);
    }

    #[test]
    fn the_newest_point_is_the_current_value_of_the_group() {
        let mut app = graphs_app();
        // Nothing logged yet: the plot still ends on the live value (`PeekCurrentDataPoint`).
        let current = app.world.data_points()[group::COUNT].clone();
        assert!(current[0] >= 8.0, "the starter bibites: {current:?}");
        app.set_graphs_stream(group::COUNT);
        let frame = graphs_frame(&app);
        let white = [255, 255, 255, 255];
        let dots_up: Vec<f32> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Rect {
                    pos, size, color, ..
                } if *color == white && size.x == 2.0 => Some(pos.y),
                _ => None,
            })
            .collect();
        let lowest = dots_up.iter().copied().fold(f32::MIN, f32::max);
        let highest = dots_up.iter().copied().fold(f32::MAX, f32::min);
        assert!(
            highest < lowest,
            "the white curve rises to the live value: {highest} {lowest}"
        );
    }

    #[test]
    fn a_scale_tab_without_data_has_no_hit_and_the_formats_follow_the_groups() {
        let app = graphs_app();
        let frame = graphs_frame(&app);
        let tabs: Vec<usize> = frame
            .actions
            .iter()
            .filter_map(|(_, action)| match action {
                UiAction::SetGraphsScale(index) => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(tabs, [0], "a new world has data at the finest scale only");
        // `DataLogger` textFormat of each group.
        assert_eq!(graph_format(group::BIOMASS).units, "E");
        for age in [group::AGE, group::DEATH_AGE] {
            let format = graph_format(age);
            assert_eq!((format.units, format.si, format.precision), ("h", false, 2));
        }
        for whole in [
            group::COUNT,
            group::BRAIN_SIZE,
            group::EGGS_LAID,
            group::BIRTH_DEATH,
        ] {
            assert!(graph_format(whole).is_int, "group {whole}");
        }
        assert!(graph_format(group::COUNT).si && !graph_format(group::EGGS_LAID).si);
        assert_eq!(graph_format(group::BIRTH_DEATH).precision, 0);
    }

    #[test]
    fn the_legend_shows_while_the_pointer_rests_on_the_button_or_the_legend() {
        let mut app = graphs_app();
        let at = |app: &mut App, x: f32, y: f32| {
            app.handle_input(&FrameInput {
                dt: 0.016,
                cursor: Vec2::new(x, y),
                ..FrameInput::default()
            });
        };
        assert!(!texts(&graphs_frame(&app)).contains(&"Bibites".to_owned()));
        // On the "?" button the legend opens, listing every curve of the group.
        at(&mut app, 1850.0, 160.0);
        assert!(app.graphs_legend_open);
        let frame = graphs_frame(&app);
        for label in ["Bibites", "Plants", "Meats", "Free"] {
            assert!(texts(&frame).contains(&label.to_owned()), "{label}");
        }
        // Its rows carry the curves' descriptions as tooltips.
        assert_eq!(frame.tips.len(), 4);
        assert_eq!(frame.tips[0].tip.title, "Bibites");
        assert!(
            frame.tips[0]
                .tip
                .content
                .contains("biomass level of bibites")
        );
        // Moving down onto the legend keeps it; a curve that is off is not listed.
        at(&mut app, 1860.0, 185.0);
        assert!(app.graphs_legend_open);
        app.toggle_graphs_curve(2);
        assert!(!texts(&graphs_frame(&app)).contains(&"Meats".to_owned()));
        // Away from both, it closes; the legend area alone does not open it.
        at(&mut app, 900.0, 600.0);
        assert!(!app.graphs_legend_open);
        at(&mut app, 1860.0, 185.0);
        assert!(!app.graphs_legend_open);
        // And it is gone with the panel.
        at(&mut app, 1850.0, 160.0);
        app.panels.close(crate::ui::PanelKind::Graphs);
        at(&mut app, 1850.0, 160.0);
        assert!(!app.graphs_legend_open);
    }
}
