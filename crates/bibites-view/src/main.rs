//! # rustibites-view
//!
//! First playable 2D viewer for **RUSTibites**, the open-source Rust
//! reimplementation of [The Bibites](https://thebibites.com/).
//!
//! The viewer is a thin, readable client on top of the headless simulation:
//!
//! * [`app`] loads a save/scenario/genome into a [`bibites_core::world::World`]
//!   and drives it with a fixed-timestep accumulator (pause, single-step,
//!   1×–8× speed). It never perturbs the simulation's RNG, so watching a world
//!   is bit-identical to running it headless.
//! * [`render`] turns the world into a small display list and draws it either
//!   with `macroquad` (interactive window) or with a pure-software rasteriser
//!   (headless PNG screenshots, no display required).
//!
//! ## Run
//!
//! ```text
//! rustibites-view [PATH] [options]
//!
//!   PATH                    save/scenario .zip or .bb8/.bb8template genome
//!                           (omit for a generated world with 200 bibites)
//!
//!   --seed N                RNG seed for a freshly built world (default 12345)
//!   --bibites N             population of the generated fallback world (200)
//!   --threads N             simulation worker threads, 0 = auto (default 0); the world times its own ticks and uses them
//!                           when that is faster (a small world ticks on one thread); 1 = the serial tick
//!   --sim-thread            tick the world on a thread of its own while only the camera is used
//!                           (also RUSTIBITES_SIM_THREAD=1)
//!   --width N --height N    window / screenshot size (default 1280×800)
//!
//!   --screenshot PATH       render one frame headlessly to a PNG and exit
//!   --frames N              frames to simulate before the screenshot (60)
//!
//! Controls: drag = pan · wheel = zoom · F = fit world · B = fit bibites ·
//! space = pause · 1–8 = speed · → / . = single step · click = select bibite ·
//! Esc = deselect (again to quit) · G/Z/P/O/H = toggle overlays ·
//! F12 = save a screenshot under `target/`.
//! ```
//!
//! **Alpha limitations:** no menu/loading screen, no sprite artwork (bodies are
//! capsules tinted by the genome colour genes), HUD text is window-only (the
//! software screenshot backend draws geometry, not text), ICO window icon is
//! decoded from `assets/icon/rustibites_256.png` when present.

#![forbid(unsafe_code)]
// Portable/release builds are GUI executables: launching the desktop shortcut
// must not flash a console. Debug builds keep the console for tests/logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod achievements;
mod app;
mod assets;
mod autopanner;
mod kerning;
mod bibite_editor;
mod biology_popups;
mod biology_popups_data;
mod biology_view;
mod brain_editor;
mod brain_editor_ui;
mod brain_layout;
mod brain_panel_layout_data;
mod challenge_end;
mod challenge_menu;
mod challenge_run;
mod challenge_tracker;
mod dropdown;
mod dyn_panel;
mod expanded_brain;
mod expanded_brain_ui;
mod gene_analysis;
mod gene_info_data;
mod hud;
mod information_panel;
mod internals;
mod internals_shaders;
mod inspector_brain;
mod item_help_data;
mod lineage_graph;
mod lineage_mesh;
mod lineage_mesh_view;
mod lineage_settings;
mod lineage_tree;
mod lineage_view;
mod machine_calibration;
mod menu;
mod menu_video;
mod node_info_data;
mod blood_particles;
mod sprite_export;
mod text_field;
mod error_popup;
mod blood_sheet;
mod panel_layout;
mod phero_particles;
mod placer_panel;
mod preview;
mod render;
mod save_dialog;
mod scenario_saver;
mod save_panel;
mod scenario_editor;
mod scrollbar;
mod selection;
mod selection_ui;
mod setting_ui_data;
mod settings_pages;
mod sim_thread;
mod tick_overlap;
mod skin;
mod species_info_layout_data;
mod species_panel;
mod sprites;
mod stats_layout_data;
mod stats_states;
mod stomach_sim;
mod stats_values;
mod tail_mesh;
mod target_weights;
mod template_editor;
mod template_editor_ui;
mod text_sdf;
mod time_format;
mod time_keeper;
mod tooltip;
mod tooltip_data;
mod tower_panel;
mod ui;
mod ui_layout;
mod ui_runtime;
mod unity_layout;
mod user_settings;
mod value_edit;
mod warning_popup;
mod warnings;
mod animator_session;
mod animators;
mod zone_cards;
mod zone_editor;
mod zone_preview;

use std::path::{Path, PathBuf};

use bibites_core::brain::Output as BrainOutput;
use bibites_core::math::Vec2;
use macroquad::miniquad;
use macroquad::prelude::*;

use app::{App, FrameInput, LoadedWorld, ViewMode};
use assets::GameAssets;
use render::ImageStats;

/// Default window/screenshot width in logical pixels.
const DEFAULT_WIDTH: u32 = 1280;
/// Default window/screenshot height in logical pixels.
const DEFAULT_HEIGHT: u32 = 800;
/// Default number of generated-world bibites when no path is given.
const DEFAULT_FALLBACK_BIBITES: usize = 200;
/// Default RNG seed for newly built worlds.
const DEFAULT_SEED: u64 = 12_345;
/// `RUSTIBITES_LOG_FRAMES=1`: once a second the window prints where its CPU time goes (the means over that second, in
/// milliseconds per frame): the simulation steps (`advance`), the scene build, the scene's draw calls, the UI, and the wait in
/// `next_frame` (presenting the frame and waiting for the display).
struct FrameLog {
    enabled: bool,
    since: std::time::Instant,
    frames: u32,
    ticks: u64,
    advance: std::time::Duration,
    scene: std::time::Duration,
    draw_scene: std::time::Duration,
    ui: std::time::Duration,
    present: std::time::Duration,
    /// Waiting for the ticks that ran beside the drawing (they took longer than the drawing and the wait for the display).
    join: std::time::Duration,
}

impl FrameLog {
    fn from_env() -> Self {
        Self {
            enabled: std::env::var("RUSTIBITES_LOG_FRAMES").is_ok_and(|value| value != "0"),
            since: std::time::Instant::now(),
            frames: 0,
            ticks: 0,
            advance: std::time::Duration::ZERO,
            scene: std::time::Duration::ZERO,
            draw_scene: std::time::Duration::ZERO,
            ui: std::time::Duration::ZERO,
            present: std::time::Duration::ZERO,
            join: std::time::Duration::ZERO,
        }
    }

    /// Prints and resets the means when a second has passed.
    fn report(
        &mut self,
        scene: &render::Scene,
        pellets: usize,
        bibites: usize,
        keeper: &time_keeper::TimeKeeper,
    ) {
        if !self.enabled {
            return;
        }
        self.frames += 1;
        let elapsed = self.since.elapsed();
        if elapsed.as_secs_f64() < 1.0 {
            return;
        }
        let frames = f64::from(self.frames);
        let ms = |total: std::time::Duration| total.as_secs_f64() * 1000.0 / frames;
        // How often consecutive sprites use different atlas textures (each change ends a batch of draw calls).
        let mut atlas_switches = 0_usize;
        let mut last_atlas = None;
        for primitive in &scene.primitives {
            if let render::Primitive::Sprite { sprite, .. } = primitive {
                if last_atlas.is_some_and(|atlas| atlas != sprite.atlas) {
                    atlas_switches += 1;
                }
                last_atlas = Some(sprite.atlas);
            }
        }
        let primitives = scene.primitives.len();
        let dots = scene.dots.len();
        eprintln!(
            "frame log: {:.1} fps over {} frames | ms/frame: advance {:.2} ({:.1} ticks/frame), scene {:.2}, draw scene {:.2}, ui {:.2}, present {:.2}, join {:.2} | {primitives} primitives ({atlas_switches} atlas switches) + {dots} dots, {pellets} pellets, {bibites} bibites | time panel: {:.0} fps / {:.0} tps / sim-real x{:.2} (counted here: {:.1} ticks/s)",
            frames / elapsed.as_secs_f64(),
            self.frames,
            ms(self.advance),
            self.ticks as f64 / frames,
            ms(self.scene),
            ms(self.draw_scene),
            ms(self.ui),
            ms(self.present),
            ms(self.join),
            keeper.fps(),
            keeper.tps(),
            keeper.sim_seconds_per_second(),
            self.ticks as f64 / elapsed.as_secs_f64(),
        );
        *self = Self {
            enabled: true,
            ..Self::from_env()
        };
    }
}

/// Frames simulated before a `--screenshot` capture (60 frames = 1 s at 60 fps).
const DEFAULT_SCREENSHOT_FRAMES: u32 = 60;

/// Initial-camera mode (`--fit`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FitMode {
    /// Frame the bibite population with context (default; readable).
    #[default]
    Bibites,
    /// Fit the whole simulation square (everything visible, bibites tiny).
    World,
}

/// Parsed command-line options.
#[derive(Clone, Debug)]
struct Args {
    input: Option<PathBuf>,
    seed: u64,
    bibites: usize,
    threads: usize,
    screenshot: Option<PathBuf>,
    frames: u32,
    width: u32,
    height: u32,
    fit: FitMode,
    /// `--fit` was given (otherwise the window opens on the game's own initial camera).
    fit_given: bool,
    /// `--width`/`--height` was given (otherwise the window opens 16:9 like the game's UI).
    size_given: bool,
    center: Option<Vec2>,
    zoom: Option<f32>,
    /// Explicit `assets/game` directory (`--assets`).
    assets: Option<PathBuf>,
    /// Disable the game-asset renderer (`--no-assets`).
    no_assets: bool,
    /// Unity orthographic camera size for reference-frame matching.
    ortho_size: Option<f32>,
    /// Reference game frame to build a side-by-side comparison against.
    compare_ref: Option<PathBuf>,
    /// Keep the built-in text HUD instead of the themed game-asset HUD.
    simple_hud: bool,
    /// Use the committed exact-layout runtime UI when the dump is present.
    runtime_ui: bool,
    /// Render only the UI into a transparent PNG (HUD pixel A/B).
    ui_only: bool,
    /// Text rasteriser for UI rendering (`ttf`, `sdf`, `auto`).
    ui_text: Option<String>,
    /// Explicit runtime UI atlas (e.g. assets/game/ui_runtime_native.json.gz).
    ui_runtime: Option<PathBuf>,
    /// Dump native runtime controls for all sessions.
    ui_controls: bool,
    /// Dump native runtime controls for specific session(s).
    ui_controls_sessions: Vec<String>,
    /// Output JSON path for `--ui-controls*`.
    ui_controls_out: Option<PathBuf>,
    /// Headless functional-coverage audit over every native control.
    ui_functional_audit: bool,
    /// Representative click→state-effect matrix per native session.
    ui_functional_matrix: bool,
    /// Captured runtime sessions to render headlessly (`--ui-session`).
    ui_session: Vec<String>,
    /// Print all available runtime UI session names and exit.
    ui_session_list: bool,
    /// Time the title menu's Load click against the real save folders and exit (`--time-load-list`).
    time_load_list: bool,
    /// Optional background image for composed UI-session renders.
    ui_background: Option<PathBuf>,
    /// Select the first living bibite before a screenshot (UI verification).
    select_first: bool,
    /// Start paused (the window's captures then show a still world).
    paused: bool,
    /// Switch "Show Targets" on (the target labels of the selected bibite's field of view; UI verification).
    show_targets: bool,
    /// Select the most recently placed tower or colour killer before a screenshot (opens its
    /// panel; a placed colour killer is not selected by itself).
    select_tool: bool,
    /// Open the Save Bibite dialog for the selected bibite before a screenshot
    /// (`--select-first --save-dialog`).
    save_dialog: bool,
    /// Opens the scenario saver with this name typed (a screenshot flag: the form is shown, never answered).
    scenario_saver: Option<String>,
    /// Select zone `n` before a screenshot, as the Z key does (opens the Zones tab on its item).
    select_zone: Option<usize>,
    /// Verification: open the lineage panel on the species at this list index.
    lineage_species: Option<usize>,
    /// Verification: the species panel's tab (0 info, 1 genes, 2 brain, 3 position).
    lineage_tab: Option<u8>,
    /// `--lineage-mesh`: the lineage panel shows the mesh view instead of the tree (headless).
    lineage_mesh: bool,
    /// `--lineage-show-extinct`: "Hide Extinct Lineages" is off for this run (headless).
    lineage_show_extinct: bool,
    /// `--position-time N`: the Position tab's time slider value (headless).
    position_time: Option<i32>,
    /// `--bibite-editor`: the Bibite Editor is open on the selected bibite (or a new template) (headless).
    bibite_editor: bool,
    /// `--editor-scroll N`: the editor's genes list is scrolled N rows (headless).
    editor_scroll: Option<usize>,
    /// `--editor-info`: the editor's stats panel is open (headless).
    editor_info: bool,
    /// `--editor-brain "tab;click 700,500;space 600,400;type tanh"`: steps on the editor's Brain tab (headless).
    editor_brain: Option<String>,
    /// `--expanded-brain "settings;graph;start;run 2;bfs 40"`: the Expanded Brain View of the selected bibite, with steps (headless).
    expanded_brain: Option<String>,
    /// `--achievement ACH_BIBITE_EDITOR`: unlocks that achievement (its popup is up in the screenshot, headless).
    achievement: Option<String>,
    /// `--challenge-champion FILE`: with `--input SCENARIO.zip` (a challenge) the screenshot is of that challenge played with that
    /// champion (headless).
    challenge_champion: Option<PathBuf>,
    /// `--open-tracker`: the challenge tracker is open in the screenshot (headless).
    open_tracker: bool,
    /// `--challenge-seconds N`: simulated seconds the challenge ran before the screenshot (headless).
    challenge_seconds: f32,
    /// `--genes-gene N`: the Gene Analysis page's gene, an index of the genes in the game's order (headless).
    genes_gene: Option<usize>,
    /// `--genes-time N`: the Gene Analysis page's time slider value (headless; `-1` is now).
    genes_time: Option<i32>,
    /// `--brain-differences`: the Brain tab shows the differences with the compared species (headless).
    brain_differences: bool,
    /// `--node-names`: the Brain tab shows the nodes' names (headless; the user setting "Show Node Names").
    node_names: bool,
    /// The lineage panel's Settings panel is open, scrolled by this much (`--lineage-settings 0`).
    lineage_settings: Option<f32>,
    /// The lineage tree's zoom (`--lineage-zoom 0.3`).
    lineage_zoom: Option<f32>,
    /// The lineage tree's scroll position, x right and y up (`--lineage-pos 0,-1200`).
    lineage_pos: Option<(f32, f32)>,
    /// Open the Dynamic settings panel on tab `n` (0 Simulation .. 4 Animation) before a screenshot,
    /// scrolled down by `dyn_scroll` reference px.
    dyn_tab: Option<u8>,
    /// See `dyn_tab`.
    dyn_scroll: f32,
    /// Open the edit field of setting `key` on that tab with `text` typed into it (`--dyn-edit key:text`).
    dyn_edit: Option<(String, String)>,
    /// Rest the pointer here (logical px) long enough for its tooltip to show before a screenshot.
    hover: Option<Vec2>,
    /// Show the Zones tab's Preview panel (`--dyn-tab 1 --zone-preview`).
    zone_preview: bool,
    /// Show the delete-save warning popup (with its "do not show again" checkbox) in the world.
    warning_popup: bool,
    /// Sub-panels (group ids) of that tab to open, and zone items (`z<n>`) to open.
    dyn_open: Vec<u16>,
    /// `--add-zone-group N`: presses the Zone Groups list's `+` N times (a screenshot of the group cards).
    add_zone_groups: u32,
    /// `--add-animator P`: adds a settings animator made by creator preset `P` (0 linear, 1 sine, 2 cosine); repeatable.
    add_animators: Vec<usize>,
    /// `--animator-creator P`: leaves the animator creator open on preset `P`.
    animator_creator: Option<usize>,
    /// `--animator-base N`: gives the first animator `N` bases (the first free settings).
    animator_bases: usize,
    /// `--animator-point I`: selects point `I` of the first animator's curve.
    animator_point: Option<usize>,
    /// Enter the bibite placer's placement mode with this template file before a screenshot
    /// (shows its options panel; `--placer-tagging custom` shows the tag field).
    placer_template: Option<PathBuf>,
    /// The placer's tagging choice for the screenshot (`none`, `species`, `random`, `custom`).
    placer_tagging: Option<String>,
    /// Inspector tab for headless UI verification shots.
    tab: Option<ui::InspectorTab>,
    /// Debug-only headless layer filter (body/mouth/eyes/arms/right-arm/left-arm/exoskeleton).
    only_layer: Option<sprites::OnlyLayer>,
    /// Debug-only: print each sprite layer's analytic alpha bbox for the
    /// selected/first bibite (headless art diagnostics).
    dump_layer_bbox: bool,
    /// Centre the camera on the selected bibite for demo screenshots.
    center_on_selected: bool,
    /// Simulated HUD click (logical pixels) for deterministic UI verification.
    click: Option<Vec2>,
    /// `--menu-view NAME`: open this title-menu view (new, load, ...) for captures.
    menu_view: Option<String>,
    /// `--menu-click X,Y` (repeatable): clicks applied to the title menu before a capture.
    menu_clicks: Vec<Vec2>,
    /// `--menu-drag x0,y0,x1,y1`: a press at the first point (menu pixels), the pointer dragged to the second, the button let go.
    menu_drags: Vec<(Vec2, Vec2)>,
    /// `--menu-champion FILE`: the champion picked in the New Game screen's challenge list before a capture.
    menu_champion: Option<PathBuf>,
    /// `--save-panel NAME`: the pause menu's Save Game panel open over the world for a capture (`-` leaves the name empty, `row:N`
    /// selects the N-th row of the list, anything else is typed into the field).
    save_panel: Option<String>,
    /// Further clicks (`--click` may be repeated; applied in order after `click`).
    extra_clicks: Vec<Vec2>,
    /// `--drag X0,Y0,X1,Y1`: a left-button drag on the world (selection rectangle) before a capture.
    drag: Option<[f32; 4]>,
    /// `--drag-hold`: stop before the release, so the rectangle is still being drawn.
    drag_hold: bool,
    /// Draw the bibites' skin, eye and egg sprites in the interactive window with the GPU hue-match material
    /// (default ON; `--no-gpu-hue-shader` turns it off, and a shader the driver does not take has the same effect: the sprites are then recoloured
    /// on the processor, with the same maths, slower with many bibites). `--screenshot` mode uses the software rasteriser, which has the same maths.
    gpu_hue_shader: bool,
    /// Tick the world on a thread of its own while the window only watches (`--sim-thread`, or `RUSTIBITES_SIM_THREAD=1`).
    sim_thread: bool,
    /// Run each frame's ticks on a helper thread while the frame is drawn and presented, so that the drawing and the wait for the
    /// display do not take the simulation's time (default ON; `--no-overlap`, or `RUSTIBITES_OVERLAP=0`, turns it off).
    overlap_ticks: bool,
    /// The time speed the window starts at (`--speed X`, the slider's target).
    speed: Option<f32>,
    /// Force the game-style title menu (also the default for an argument-less
    /// interactive window).
    menu: bool,
    /// Bypass the title menu and go straight to the world.
    no_menu: bool,
    /// Directory scanned for `Continue`/`Load Game` saves.
    saves_dir: Option<PathBuf>,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            input: None,
            seed: DEFAULT_SEED,
            bibites: DEFAULT_FALLBACK_BIBITES,
            threads: 0,
            screenshot: None,
            frames: DEFAULT_SCREENSHOT_FRAMES,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            fit: FitMode::Bibites,
            fit_given: false,
            size_given: false,
            center: None,
            zoom: None,
            assets: None,
            no_assets: false,
            ortho_size: None,
            compare_ref: None,
            simple_hud: false,
            runtime_ui: true,
            ui_only: false,
            ui_text: None,
            ui_runtime: None,
            ui_controls: false,
            ui_controls_sessions: Vec::new(),
            ui_controls_out: None,
            ui_functional_audit: false,
            ui_functional_matrix: false,
            ui_session: Vec::new(),
            ui_session_list: false,
            time_load_list: false,
            ui_background: None,
            select_first: false,
            paused: false,
            show_targets: false,
            select_tool: false,
            save_dialog: false,
            scenario_saver: None,
            select_zone: None,
            lineage_species: None,
            lineage_tab: None,
            lineage_mesh: false,
            lineage_show_extinct: false,
            position_time: None,
            bibite_editor: false,
            editor_scroll: None,
            editor_info: false,
            editor_brain: None,
            expanded_brain: None,
            achievement: None,
            challenge_champion: None,
            open_tracker: false,
            challenge_seconds: 0.0,
            genes_gene: None,
            genes_time: None,
            brain_differences: false,
            node_names: false,
            lineage_zoom: None,
            lineage_settings: None,
            lineage_pos: None,
            dyn_tab: None,
            dyn_edit: None,
            dyn_scroll: 0.0,
            hover: None,
            zone_preview: false,
            warning_popup: false,
            dyn_open: Vec::new(),
            add_zone_groups: 0,
            add_animators: Vec::new(),
            animator_creator: None,
            animator_bases: 0,
            animator_point: None,
            placer_template: None,
            placer_tagging: None,
            tab: None,
            only_layer: None,
            dump_layer_bbox: false,
            center_on_selected: false,
            click: None,
            menu_view: None,
            menu_clicks: Vec::new(),
            menu_drags: Vec::new(),
            menu_champion: None,
            save_panel: None,
            extra_clicks: Vec::new(),
            drag: None,
            drag_hold: false,
            gpu_hue_shader: true,
            sim_thread: std::env::var("RUSTIBITES_SIM_THREAD").is_ok_and(|value| value != "0"),
            overlap_ticks: std::env::var("RUSTIBITES_OVERLAP").map_or(true, |value| value != "0"),
            speed: None,
            menu: false,
            no_menu: false,
            saves_dir: None,
            help: false,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        eprintln!("run `rustibites-view --help` for usage");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    // Read the installed game's preferences (registry) while the assets load.
    bibites_io::prefs::GamePrefs::prefetch();
    // The title menu's gallery shows the unlocks of this port too.
    achievements::publish_stored(&bibites_io::storage::StoragePaths::default_content());
    let mut args = parse_args(std::env::args().skip(1))?;
    if args.screenshot.is_none() && !args.size_given {
        // The game's UI is authored for 16:9; the 1280x800 default letterboxes it.
        args.height = args.width * 9 / 16;
    }
    if args.help {
        print_usage();
        return Ok(());
    }
    // The machine's calibration (Settings > Calibration), when one was run: every world made from here on starts from it.
    machine_calibration::init(load_user_settings().flag(machine_calibration::LABEL_USE));

    if args.ui_functional_matrix {
        return run_functional_matrix(&args);
    }
    if args.ui_functional_audit {
        return run_functional_audit(&args);
    }
    if args.ui_controls || !args.ui_controls_sessions.is_empty() {
        return run_ui_controls(&args);
    }
    if args.ui_session_list {
        return list_ui_sessions(&args);
    }
    if args.time_load_list {
        time_load_list(&args);
        return Ok(());
    }
    if !args.ui_session.is_empty() {
        return run_ui_sessions(&args);
    }

    let menu_mode =
        args.menu || (args.input.is_none() && !args.no_menu && args.screenshot.is_none());
    if menu_mode {
        let assets = load_assets(&args);
        let mut themed_args = args.clone();
        // The title/menu screens need the extracted game UI fonts even when the
        // in-world HUD was asked to use the simple text fallback.
        themed_args.simple_hud = false;
        let themed = load_themed_ui(&themed_args);
        let mut menu = make_menu(&args);
        if args.menu_view.as_deref() == Some("editor") {
            // The scenario editor on the Default scenario (`--dyn-tab N` picks its tab, `--add-animator P`, `--add-zone-group N` and
            // `--animator-point I` fill it like they fill the world's panel).
            menu.open_editor_for_capture();
            if let Some(tab) = args.dyn_tab {
                let _ = menu.editor_action(ui::UiAction::DynTab(tab));
            }
            for preset in &args.add_animators {
                let _ = menu.editor_action(ui::UiAction::Invoke(ui::ControlCommand::OpenAnimatorCreator));
                menu.editor_creator_preset_for_capture(*preset);
                let _ = menu.editor_action(ui::UiAction::Invoke(ui::ControlCommand::SubmitAnimatorCreator));
            }
            for _ in 0..args.add_zone_groups {
                menu.editor_add_for_capture(1);
            }
            for slot in 0..args.animator_bases {
                menu.editor_base_for_capture(1 + slot * 7);
            }
            if let Some(point) = args.animator_point {
                menu.editor_pick_point_for_capture(0, point);
            }
        } else if let Some(view) = args.menu_view.as_deref() {
            menu.open_view_for_capture(match view {
                "new" => menu::MenuView::NewGame,
                "load" => menu::MenuView::LoadGame,
                "settings" => menu::MenuView::Settings,
                "library" => menu::MenuView::Library,
                "achievements" => menu::MenuView::Achievements,
                "tools" => menu::MenuView::Tools,
                "credits" => menu::MenuView::Credits,
                other => return Err(format!("unknown --menu-view `{other}`")),
            });
        }
        if args.screenshot.is_some()
            && let Some(themed) = themed.as_ref()
        {
            // Headless clicks on native panels (deterministic UI verification).
            let mut fonts = load_themed_ui(&themed_args)
                .map(|themed| themed.fonts)
                .ok_or_else(|| "menu clicks need the extracted game UI assets".to_owned())?;
            for point in args.menu_clicks.iter().copied() {
                let _ = menu.click_native(
                    &themed.ui,
                    &mut fonts,
                    args.width as f32,
                    args.height as f32,
                    themed.runtime.as_ref(),
                    point,
                );
            }
            // A press on a slider, the pointer dragged to a second point and the button let go: the window's own calls, in a row.
            for (from, to) in args.menu_drags.iter().copied() {
                let (w, h) = (args.width as f32, args.height as f32);
                let under = menu.slider_under(&themed.ui, &mut fonts, w, h, themed.runtime.as_ref(), from);
                let _ = menu.click_native(&themed.ui, &mut fonts, w, h, themed.runtime.as_ref(), from);
                let held = menu.slider_dragging();
                let moved = menu.slider_drag_move(to);
                menu.slider_release();
                println!("menu-drag: slider under the press: {under}, held after it: {held}, moved by the drag: {moved}");
            }
            if let Some(path) = args.menu_champion.as_ref() {
                menu.set_champion(path.clone());
            }
        }
        if let Some(path) = args.screenshot.as_ref() {
            let mut menu = menu;
            return run_menu_screenshot(&args, path, assets, themed, &mut menu);
        }
        if themed.is_none() {
            return Err(
                "the title menu needs extracted game UI assets (assets/game); run with --assets \
                 or pass --no-menu for a generated world"
                    .to_owned(),
            );
        }
        // A tiny generated world keeps the shared `App` state valid while the
        // menu is in front; it is replaced as soon as a save/scenario is chosen.
        let loaded = LoadedWorld::load(None, args.seed, 1, args.threads)?;
        run_window(loaded, args, assets, themed, Some(menu));
        return Ok(());
    }

    if let Some(layer) = args.only_layer {
        sprites::set_only_layer(Some(layer));
    }
    let (loaded, challenge_setup) = match (args.challenge_champion.as_ref(), args.input.as_ref()) {
        // The scenario played as a challenge with that champion (`StartChallenge(true)`).
        (Some(champion), Some(scenario)) => {
            let start = challenge_run::ChallengeStart {
                scenario: scenario.clone(),
                champion: Some(challenge_run::Champion {
                    path: champion.clone(),
                    name: champion
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                }),
                as_challenge: true,
            };
            LoadedWorld::load_challenge_start(&start, args.seed, args.threads)?
        }
        _ => (
            LoadedWorld::load(args.input.as_deref(), args.seed, args.bibites, args.threads)?,
            None,
        ),
    };
    let assets = load_assets(&args);
    let themed = load_themed_ui(&args);
    eprintln!(
        "loaded {}: {} bibites spawned, {} live, {} eggs, {} zones, {} pellets, {} species, {} TPS, seed {}",
        loaded.label,
        loaded.spawned,
        loaded.world.bibites.len(),
        loaded.world.eggs.len(),
        loaded.world.zones.zones.len(),
        loaded.world.pellets.len(),
        loaded.world.species.active_species_count(),
        loaded.world.settings.sim_tps,
        loaded.world.seed,
    );

    match &args.screenshot {
        Some(path) => run_screenshot(loaded, challenge_setup, &args, path, assets, themed),
        None => {
            run_window(loaded, args, assets, themed, None);
            Ok(())
        }
    }
}

/// Canonical scenario offered by the menu's `New Game` entry.
/// Applies the game's user settings (Settings screens) to a freshly opened world.
/// `UserSettings.showBrainDifferences` (the Brain tabs' "Highlight differences" toggle, not on the Settings screens): the player's stored
/// value, off by default. Read once when a world opens: the toggle changes it from then on.
fn apply_stored_toggles(app: &mut App) {
    let prefs = bibites_io::prefs::GamePrefs::global();
    app.show_brain_differences = prefs.flag("showBrainDifferences").unwrap_or(false);
    app.sort_energy_details = prefs.flag("SortBiologyPanelEnergyDetails").unwrap_or(false);
    // `UserSettings.EasterEggsProbability`: 1 to 100 000 000, the Settings screen's slider for it is not wired.
    app.easter_eggs_period = prefs
        .int("EasterEggsProbability")
        .and_then(|period| u32::try_from(period).ok())
        .map_or(500_000, |period| period.clamp(1, 100_000_000));
}

fn apply_user_settings(app: &mut App, settings: &user_settings::UserSettings) {
    app.tps_defaults = settings.tps_defaults();
    machine_calibration::calibrator().set_use(settings.flag(machine_calibration::LABEL_USE));
    ui::set_ui_scale_settings(
        settings.flag("Automatically Detect Screen Resolution"),
        [0.667, 0.711, 0.833, 1.0, 1.333, 2.0][settings.choice("Screen Resolution").min(5)],
        [0.5, 0.75, 1.0, 1.15, 1.25][settings.choice("UI Scale").min(4)],
    );
    if let Some(assets) = app.assets.as_mut() {
        // `UserSettings.FertilityColor` (zone shader base) and `Background`
        // (camera clear colour).
        let fertility = settings.color("Fertility Color");
        assets.background.zone_base = [fertility[0], fertility[1], fertility[2]];
        assets.background.color = settings.color("Background Color");
        settings.apply_grid(&mut assets.background);
        assets.background.texture = settings.flag("Show Experimental Background").then(|| settings.color("Texture Color"));
    }
    app.scene_options.grid = settings.flag("Show Grid");
    app.scene_options.pheromones = settings.flag("Show Pheromones");
    app.scene_options.zones = settings.flag("Show Fertility");
    app.scene_options.fov = settings.flag("Show Selected Bibite Field Of View");
    app.show_ruler = settings.flag("Show Ruler");
    app.show_status_bars = settings.flag("Show Status Bars");
    app.show_targets = settings.flag("Show Targets Priority of Selected Bibite");
    app.prefs.show_node_names = settings.flag("Show Node Names");
    app.allow_easter_eggs = settings.flag("Allow Easter Eggs");
    app.easter_eggs_period = (settings.number("Easter Eggs Probability Period") as u32).max(1);
    app.show_brain_differences = settings.flag("Show Difference");
    app.sort_energy_details = settings.flag("Sort Biology Panel Energy Details");
    app.include_autosaves = settings.flag("Include autosaves");
    app.save_as_template = settings.flag("Save as template");
    app.format_bibite_json = settings.flag("Format JSON");
    app.prefs.autosave = settings.flag("Autosave");
    app.prefs.autosave_real_time = settings.flag("Real-time autosave period");
    app.prefs.autosave_period_minutes = settings.autosave_period_minutes();
    app.prefs.autosave_max_files = settings.number("AutoSave Max Files") as u32;
    app.prefs.reload_after_autosaves = settings.flag("Reload After AutoSaves");
    app.prefs.pause_after_load = settings.flag("Pause After Load");
    app.prefs.min_fps = settings.number("Minimum FPS") as u32;
    app.selection_radius = settings.number("Selection Radius").clamp(0.0, 1.0);
    app.reselect_on_corpses = settings.flag("Reselect On Corpses");
    app.open_zone_editor_on_select = settings.flag("Open Zone Editor On Select");
    app.reselection_delay = settings.number("Reselection Delay").clamp(0.0, 10.0);
    app.allow_arrow_keys = settings.flag("Allow Arrow Key");
    app.prefs.pheromone_opacity = settings.number("Pheromones Strength").clamp(0.0, 1.0);
    app.procedural_size = preview::ProceduralSize::of_choice(settings.choice("Procedural Size"));
    sprites::set_procedural_size(app.procedural_size);
}

/// `--time-load-list`: what the click on the title menu's "Load" costs on this machine's own save folders. The click itself (it must return at
/// once: the window cannot repaint before it does), the background check of every save until it is done, and, for comparison, what the click
/// cost when it read every save before returning (the old way, on the files the check has just warmed).
fn time_load_list(args: &Args) {
    let mut menu = make_menu(args);
    let started = std::time::Instant::now();
    menu.open_load_game();
    let click = started.elapsed();
    let saves = menu.save_count();
    while menu.save_check_running() {
        menu.poll_background();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let checked = started.elapsed();
    println!("saves in the list: {saves}");
    println!("the click on Load returned after {:.0} ms", click.as_secs_f64() * 1000.0);
    println!(
        "the check of every save ran beside the window for {:.0} ms more; {} unreadable (popups)",
        (checked - click).as_secs_f64() * 1000.0,
        menu.pending_error_count()
    );
    let old = std::time::Instant::now();
    let mut unreadable = 0;
    for row in 0..saves {
        if let Some(path) = menu.save_path(row)
            && bibites_io::storage::save_problem(&path).is_some()
        {
            unreadable += 1;
        }
    }
    println!(
        "for comparison, the old click read every save before it returned: {:.0} ms ({unreadable} unreadable), with the files already in the cache",
        old.elapsed().as_secs_f64() * 1000.0
    );
}

/// A fresh title menu (also used by the Escape panel's "Back to Menu").
fn make_menu(args: &Args) -> menu::MenuState {
    let settings_path = menu_settings_path();
    let mut menu = menu::MenuState::new_with_settings(
        default_scenario_path(),
        &menu_saves_dir(args),
        &settings_path,
    );
    menu.set_data_roots(menu_data_roots());
    menu
}

/// The player's user settings (the game's `PlayerPrefs` overlaid with this port's changes).
/// The viewer's own user settings file (the changes made in this port).
fn user_settings_file() -> PathBuf {
    bibites_io::storage::StoragePaths::default_local()
        .root
        .join("game_user_settings.json")
}

/// The Biology pop-up's Sort and the Brain tabs' difference toggles are user settings in the game (`SortBiologyPanelEnergyDetails`,
/// `showBrainDifferences`): a change made in the window is stored.
/// F12: the window's current picture (menu or world, UI included) as `screenshots/rustibites-<time>.png` next to the program.
fn save_window_capture() {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |duration| duration.as_secs());
    let dir = std::path::Path::new("screenshots");
    let path = dir.join(format!("rustibites-{seconds}.png"));
    if std::fs::create_dir_all(dir).is_ok() {
        macroquad::prelude::get_screen_data().export_png(&path.to_string_lossy());
        eprintln!("screenshot saved to {}", path.display());
    }
}

/// `RUSTIBITES_SELF_SHOT=<world frame>:<png path>`: the window writes what the GPU drew (the framebuffer, UI included) at that frame and exits,
/// so a real-window check does not depend on an operating-system capture of the window (`tools/window_capture.ps1`).
fn self_shot_request() -> Option<(usize, std::path::PathBuf)> {
    let value = std::env::var("RUSTIBITES_SELF_SHOT").ok()?;
    let (frame, path) = value.split_once(':')?;
    Some((frame.parse().ok()?, std::path::PathBuf::from(path)))
}

/// `RUSTIBITES_INPUT_LOG` is set: the title screen's clicks are logged to stderr with the window's idea of the pointer and the screen.
fn input_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTIBITES_INPUT_LOG").is_some())
}

fn persist_view_toggles(app: &App) {
    thread_local! {
        static LAST: std::cell::Cell<Option<(bool, bool)>> = const { std::cell::Cell::new(None) };
    }
    let now = (app.sort_energy_details, app.show_brain_differences);
    let last = LAST.with(std::cell::Cell::get);
    LAST.with(|cell| cell.set(Some(now)));
    let Some(last) = last else {
        return;
    };
    if last == now {
        return;
    }
    let mut settings = load_user_settings();
    for (label, value, before) in [("Sort Biology Panel Energy Details", now.0, last.0), ("Show Difference", now.1, last.1)] {
        if value != before
            && let Some(index) = user_settings::index_of(label)
        {
            settings.set(index, user_settings::Value::Bool(value));
        }
    }
}

fn load_user_settings() -> user_settings::UserSettings {
    user_settings::UserSettings::load(Some(user_settings_file()))
}

/// Points an interactive world's saves, autosaves, scenarios and single-bibite files at the
/// game's own data folder (`%USERPROFILE%/AppData/LocalLow/The Bibites/The Bibites`, so the
/// original game lists them), or at `RUSTIBITES_DATA_DIR` / `./userdata` without the game.
/// Headless screenshot runs keep the default local folder and never write to the game's.
fn use_game_storage(app: &mut App) {
    // Interactive worlds keep the Save Bibite dialog's two checkboxes in the user settings file.
    app.user_settings_path = Some(user_settings_file());
    app.storage = bibites_io::storage::StoragePaths::default_content();
    // The achievements this port unlocked (the game's own are `GamePrefs`').
    app.achievements = achievements::Achievements::load(&app.storage);
    eprintln!(
        "storage: saves, scenarios and bibites go to {}",
        app.storage.root.display()
    );
}

/// Data roots the title menu reads scenarios/saves from, in priority order: the
/// installed game's folder (`%USERPROFILE%/AppData/LocalLow/The Bibites/The Bibites`),
/// then the viewer's own data folder, then the working-directory test data.
fn menu_data_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = bibites_io::storage::StoragePaths::game_root_guess() {
        roots.push(root);
    }
    let local = bibites_io::storage::StoragePaths::default_local().root;
    if local.is_dir() {
        roots.push(local);
    }
    roots
}

fn default_scenario_path() -> Option<PathBuf> {
    let canonical = PathBuf::from("testdata/real/Default.zip");
    if canonical.is_file() {
        return Some(canonical);
    }
    None
}

/// Directory scanned for `Continue`/`Load Game`; defaults to `saves` and then
/// the committed test saves directory.
fn menu_saves_dir(args: &Args) -> PathBuf {
    if let Some(dir) = &args.saves_dir {
        return dir.clone();
    }
    let user_saves = PathBuf::from("saves");
    if user_saves.is_dir() {
        return user_saves;
    }
    PathBuf::from("testdata/real/saves")
}

/// Persisted menu/settings state for the interactive title menu.
fn menu_settings_path() -> PathBuf {
    PathBuf::from("target/viewer-menu-settings.txt")
}

/// Renders one deterministic menu frame headlessly (`--menu --screenshot`).
fn run_menu_screenshot(
    args: &Args,
    path: &Path,
    assets: Option<GameAssets>,
    themed: Option<ThemedUi>,
    menu: &mut menu::MenuState,
) -> Result<(), String> {
    let Some(mut themed) = themed else {
        return Err(
            "the menu screenshot needs extracted game UI assets (assets/game); pass --assets"
                .to_owned(),
        );
    };
    // The Library's preview picture needs the game's sprites.
    menu.refresh_library_picture(
        assets.as_ref(),
        &preview::PreviewCache::default(),
        ui::UiFrame::scaler(args.width as f32, args.height as f32),
    );
    // What the window pays for every repaint of a menu screen (a hover, a slider drag step): the frame, its draw list and the software
    // picture. The first pass also builds what is cached (fonts, sprites), so the second is the steady cost; the picture is the second's.
    let mut passes = Vec::new();
    let mut picture = None;
    for _ in 0..2 {
        let started = std::time::Instant::now();
        let frame = menu.build_frame(
            &themed.ui,
            &mut themed.fonts,
            args.width as f32,
            args.height as f32,
            themed.runtime.as_ref(),
        );
        let built = started.elapsed();
        let overlay = menu::menu_overlay(&themed.ui, &frame, &mut themed.fonts);
        picture = Some(render::render_ui_only(args.width, args.height, 1, Some(overlay)));
        passes.push((started.elapsed(), built));
    }
    let Some(image) = picture else {
        return Err("the menu was not rendered".to_owned());
    };
    let ms = |time: std::time::Duration| time.as_secs_f64() * 1000.0;
    println!(
        "menu-render: {}x{}: first {:.1} ms, repaint {:.1} ms (frame {:.1} ms, picture {:.1} ms)",
        args.width,
        args.height,
        ms(passes[0].0),
        ms(passes[1].0),
        ms(passes[1].1),
        ms(passes[1].0 - passes[1].1)
    );
    if let Some(click) = args.click {
        let runtime = themed
            .runtime
            .as_ref()
            .ok_or("menu hit test needs the runtime layout")?;
        let rows = runtime.menu_button_rects(args.width as f32, args.height as f32);
        let row = rows.iter().position(|(pos, size)| {
            click.x >= pos.x
                && click.y >= pos.y
                && click.x < pos.x + size.x
                && click.y < pos.y + size.y
        });
        match row {
            Some(row) => println!("menu-hit: row {row} at ({:.0},{:.0})", click.x, click.y),
            None => println!("menu-hit: no row at ({:.0},{:.0})", click.x, click.y),
        }
    }
    println!("menu-screenshot hash: {:016x}", menu_image_hash(&image));
    image.write_png(path).map_err(|error| error.to_string())?;
    println!("screenshot: {}", path.display());
    println!("  image:  {}x{} (menu)", args.width, args.height);
    println!("  menu:   {:?} selected={}", menu.view(), menu.selected());
    Ok(())
}

/// Deterministic FNV-1a hash of a rendered menu image (regression checks).
fn menu_image_hash(image: &render::Image) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in &image.rgba {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Applies the CLI camera overrides on top of the app's default bibite view.
/// Mouse wheel movement in notches (`Input.mouseScrollDelta.y`: +-1 per notch). miniquad
/// reports the raw Windows wheel delta (120 per notch), which would slam the camera zoom to its
/// limits on the first notch.
fn wheel_notches() -> f32 {
    let (_, wheel) = mouse_wheel();
    if cfg!(windows) { wheel / 120.0 } else { wheel }
}

/// The game's initial camera: the scene's Main Camera sits at the origin with orthographic
/// size 400 (`level2`), and `CameraManager` only changes it on player input. Used by the
/// interactive window unless the command line asks for a specific view.
fn apply_game_camera(app: &mut App, args: &Args, screen_height: f32) {
    if args.fit_given || args.center.is_some() || args.zoom.is_some() || args.ortho_size.is_some() {
        return;
    }
    app.set_view(Some(Vec2::ZERO), None);
    app.set_ortho_size(400.0, screen_height);
}

/// `--lineage-species N` and its companions (`--lineage-mesh`, `--lineage-zoom`, `--lineage-pos`, `--lineage-tab`, ...): opens the Lineage panel on the
/// species at that index of its list, for the headless pictures and the real-window captures alike.
fn apply_lineage_flags(app: &mut App, args: &Args) {
    if let Some(index) = args.lineage_species {
        // The panel opens past its five-record gate, as it would after a long run.
        let records = app.world.species.n_records().max(5);
        app.world.species.set_n_records(records);
        if args.lineage_show_extinct {
            app.lineage_options = Some(lineage_tree::TreeSettings {
                hide_extinct: false,
                ..app.lineage_tree_settings()
            });
        }
        app.apply_ui_actions(&[ui::UiAction::Invoke(ui::ControlCommand::OpenLineage)]);
        if args.lineage_mesh {
            app.apply_ui_actions(&[ui::UiAction::SelectLineagePanel(false)]);
        }
        // The Position tab's slider is set before the species is selected, as a value kept from the last one.
        if let Some(time) = args.position_time {
            app.position_value = time;
        }
        if let Some(id) = app.lineage_species.get(index).map(|entry| entry.id) {
            app.select_species_info(Some(id));
        }
        if let Some(tab) = args.lineage_tab {
            app.lineage_tab = tab.min(3);
        }
        if args.node_names {
            app.prefs.show_node_names = true;
        }
        if args.brain_differences {
            app.brain_toggle_differences();
        }
        if let Some(zoom) = args.lineage_zoom {
            app.lineage_view.zoom_to(zoom);
        }
        if let Some(scroll) = args.lineage_settings {
            app.lineage_settings_open = true;
            app.lineage_settings_scroll = scroll;
        }
        if let Some((x, y)) = args.lineage_pos {
            app.lineage_view.pos = [x, y];
            app.lineage_view.clamp();
        }
    }
}

fn apply_view_options(app: &mut App, args: &Args) {
    if args.fit == FitMode::World {
        app.fit_world();
    }
    app.set_view(args.center, args.zoom);
    if args.paused {
        app.paused = true;
    }
}

// ---------------------------------------------------------------------------
// Window mode
// ---------------------------------------------------------------------------

/// Loaded themed UI (atlas + fonts + optional runtime exact-layout data) used
/// by both the macroquad window and the headless screenshot backend.
struct ThemedUi {
    ui: ui::UiAssets,
    fonts: ui::FontSet,
    /// Committed game runtime UI hierarchy (exact layout), when available.
    runtime: Option<ui_runtime::RuntimeUi>,
}

/// Loads the game-asset UI atlas + fonts when available and enabled.
fn load_themed_ui(args: &Args) -> Option<ThemedUi> {
    if args.simple_hud {
        eprintln!("ui: simple HUD requested (--simple-hud)");
        return None;
    }
    let t0 = std::time::Instant::now();
    match ui::UiAssets::discover() {
        Ok(Some(mut atlas)) => match {
            eprintln!("timing: ui assets discovered {:?}", t0.elapsed());
            ui::FontSet::load(&atlas)
        } {
            Ok(mut fonts) => {
                let text_mode = match args.ui_text.as_deref() {
                    Some("ttf") => ui::TextMode::Ttf,
                    Some("sdf") => ui::TextMode::Sdf,
                    _ => ui::TextMode::Auto,
                };
                fonts.set_text_mode(text_mode);
                eprintln!(
                    "ui: {} textures, fonts loaded (text {text_mode:?}, sdf {})",
                    atlas.texture_count(),
                    fonts.using_sdf(),
                );
                eprintln!("timing: fonts {:?}", t0.elapsed());
                let use_runtime = args.runtime_ui || Path::new(NATIVE_ATLAS).is_file();
                let runtime = if use_runtime {
                    let native = Path::new(NATIVE_ATLAS);
                    let (mut runtime, loaded) = if native.is_file() {
                        match ui_runtime::RuntimeUi::load_native(native) {
                            Ok(runtime) => (runtime, true),
                            Err(error) => {
                                eprintln!("ui: native runtime atlas failed ({error})");
                                return None;
                            }
                        }
                    } else {
                        match ui_runtime::RuntimeUi::discover() {
                            Ok(Some(runtime)) => (runtime, false),
                            Ok(None) => return None,
                            Err(error) => {
                                eprintln!("ui: runtime atlas failed ({error})");
                                return None;
                            }
                        }
                    };
                    eprintln!("timing: runtime layout {:?}", t0.elapsed());
                    if loaded {
                        runtime.adopt_native_fixed_sessions();
                        eprintln!(
                            "ui: native runtime atlas {} ({} sessions; menu/HUD composition native)",
                            runtime.source.display(),
                            runtime.extra_sessions.len() + 7
                        );
                    }
                    if loaded {
                        let background = Path::new(
                            "assets/game/reference/functional_ui_native/menu_background/menu_background_title_1920x1080.png",
                        );
                        if background.is_file() {
                            match atlas.add_texture_alias("menu_background", background) {
                                Ok(_) => eprintln!("ui: native menu background loaded"),
                                Err(error) => {
                                    eprintln!("ui: native menu background failed: {error}")
                                }
                            }
                        }
                        eprintln!("timing: background {:?}", t0.elapsed());
                        match load_native_sprite_manifest(&mut atlas) {
                            Ok(count) => {
                                eprintln!("ui: native sprite manifest {count} entries resolved")
                            }
                            Err(error) => eprintln!("ui: native sprite manifest: {error}"),
                        }
                    }
                    eprintln!("timing: sprite manifest {:?}", t0.elapsed());
                    Some(runtime)
                } else {
                    None
                };
                if let Some(runtime) = runtime.as_ref() {
                    let stats = runtime.stats();
                    let panels = runtime.core_panels();
                    let inspector_nodes = runtime
                        .inspector_session
                        .as_ref()
                        .map_or(0, |session| session.node_count);
                    eprintln!(
                        "ui: layout {} ({} nodes, {} core panels; capture {} core nodes, {} inspector nodes)",
                        runtime.source.display(),
                        stats.nodes,
                        panels.len(),
                        runtime.session.core_panel_count,
                        inspector_nodes,
                    );
                }
                Some(ThemedUi {
                    ui: atlas,
                    fonts,
                    runtime,
                })
            }
            Err(error) => {
                eprintln!("warning: UI fonts unusable ({error}); using simple HUD");
                None
            }
        },
        Ok(None) => None,
        Err(error) => {
            eprintln!("warning: UI atlas unusable ({error}); using simple HUD");
            None
        }
    }
}

/// Builds the themed UI frame for the current app state.
///
/// Runtime exact-layout HUD when the committed dump is loaded (with the curated
/// live inspector panel overlaid); otherwise the curated P0 HUD.
/// Prints each sprite layer's analytic screen-space alpha bbox for the
/// selected (or first live) bibite. Source atlas alpha pixels are mapped
/// UV -> pivot -> flip -> rotation -> size -> anchor -> camera so art bbox
/// mismatches can be compared against game layer captures.
fn dump_layer_bboxes(app: &App) {
    let Some(assets) = app.assets.as_ref() else {
        eprintln!("dump-layer-bbox: no game assets loaded");
        return;
    };
    let id = app.selected.or_else(|| {
        app.world
            .bibites
            .iter()
            .find(|(_, record)| !record.is_corpse)
            .map(|(id, _)| id)
    });
    let Some(id) = id else {
        eprintln!("dump-layer-bbox: no live bibite");
        return;
    };
    let Some(record) = app.world.bibites.get(id) else {
        eprintln!("dump-layer-bbox: missing record");
        return;
    };
    eprintln!(
        "dump-layer-bbox bibite {} pos ({:.3}, {:.3}) rot {:.3}",
        id, record.pos.x, record.pos.y, record.rot,
    );
    for (index, sprite) in sprites::bibite_sprites(assets, record, None)
        .iter()
        .enumerate()
    {
        let Some(atlas) = assets.atlases.get(sprite.sprite.atlas as usize) else {
            continue;
        };
        let [rx, ry, rw, rh] = sprite.sprite.rect;
        if rw == 0 || rh == 0 {
            continue;
        }
        let anchor_screen = app.camera.world_to_screen(sprite.anchor);
        let size_screen = sprite.size * app.camera.zoom;
        let angle = -sprite.rotation;
        let (sin, cos) = angle.sin_cos();
        let mut bbox: Option<(f32, f32, f32, f32)> = None;
        for y in 0..rh {
            for x in 0..rw {
                let offset = (((ry + y) * atlas.width + rx + x) * 4) as usize;
                let Some(pixel) = atlas.rgba.get(offset..offset + 4) else {
                    continue;
                };
                if pixel[3] < 16 {
                    continue;
                }
                let mut u = (x as f32 + 0.5) / rw as f32;
                let mut v = (y as f32 + 0.5) / rh as f32;
                if sprite.flip_x {
                    u = 1.0 - u;
                }
                if sprite.flip_y {
                    v = 1.0 - v;
                }
                let local_x = (u - sprite.pivot.x) * size_screen.x;
                let local_y = (v - sprite.pivot.y) * size_screen.y;
                let dx = cos * local_x - sin * local_y;
                let dy = sin * local_x + cos * local_y;
                let sx = anchor_screen.x + dx;
                let sy = anchor_screen.y + dy;
                bbox = Some(match bbox {
                    Some((min_x, min_y, max_x, max_y)) => {
                        (min_x.min(sx), min_y.min(sy), max_x.max(sx), max_y.max(sy))
                    }
                    None => (sx, sy, sx, sy),
                });
            }
        }
        if let Some((min_x, min_y, max_x, max_y)) = bbox {
            eprintln!(
                "  layer {index}: atlas={} rect=[{rx},{ry},{rw},{rh}] pivot=({:.3},{:.3}) flip=({},{}) rot={:.3} size=({:.2},{:.2}) anchor=({:.3},{:.3}) bbox=({:.1},{:.1})-({:.1},{:.1}) px={:.1}x{:.1}",
                atlas.name,
                sprite.pivot.x,
                sprite.pivot.y,
                sprite.flip_x,
                sprite.flip_y,
                sprite.rotation,
                sprite.size.x,
                sprite.size.y,
                sprite.anchor.x,
                sprite.anchor.y,
                min_x,
                min_y,
                max_x,
                max_y,
                max_x - min_x,
                max_y - min_y,
            );
        }
    }
}

fn build_ui_frame(app: &App, themed: &ThemedUi, width: f32, height: f32) -> ui::UiFrame {
    match &themed.runtime {
        Some(runtime) => {
            let use_runtime_inspector = runtime.uses_inspector_session(app);
            // `UserControl.Update`: H deactivates the whole `MainUI` canvas, so nothing is drawn
            // or clickable (the confirmation popups are not part of it).
            let mut frame = if app.show_hud {
                runtime.build_frame_with_fonts(app, &themed.ui, themed.fonts.sdf(), width, height)
            } else {
                ui::UiFrame::new(ui::UiFrame::scaler(width, height))
            };
            if app.show_hud {
                if !use_runtime_inspector {
                    // The expanded Stats panel comes from the runtime inspector
                    // session; other tabs still use the live curated panel.
                    hud::draw_selection_panel(&mut frame, app, &themed.ui, width, height);
                }
                hud::draw_extra_panels(
                    &mut frame,
                    app,
                    &themed.ui,
                    themed.fonts.sdf(),
                    width,
                    height,
                    true,
                );
            }
            // The scenario saver (a form over the whole screen); the overwrite question goes over it.
            runtime.emit_scenario_saver(&mut frame, app, &themed.ui, themed.fonts.sdf(), width, height);
            // The confirmation popup (game wording and look) goes over everything.
            runtime.emit_confirm_dialog(
                &mut frame,
                app,
                &themed.ui,
                themed.fonts.sdf(),
                width,
                height,
            );
            error_popup::emit_world(&mut frame, app, &themed.ui, themed.fonts.sdf(), width, height);
            // The Zones tab's Preview panel hangs left of the dynamic panel.
            zone_preview::emit(
                &mut frame,
                app,
                &themed.ui,
                themed.fonts.sdf(),
                width,
                height,
            );
            // An open dropdown list hangs over the panels (its blocker covers the screen).
            if let Some(open) = app.dropdown_open.as_ref() {
                dropdown::emit(
                    &mut frame,
                    &themed.ui,
                    themed.fonts.sdf(),
                    open,
                    Vec2::new(width, height),
                    app.cursor,
                );
            }
            // The tooltip canvas is on top of all of it.
            tooltip::emit(
                &mut frame,
                app,
                &themed.ui,
                themed.fonts.sdf(),
                width,
                height,
            );
            frame
        }
        None => hud::build(app, &themed.ui, width, height),
    }
}

/// The Bibite Editor alone (the title menu has no world HUD): its screen and its tooltips.
fn build_editor_frame(app: &App, themed: &ThemedUi, width: f32, height: f32) -> ui::UiFrame {
    let mut frame = ui::UiFrame::new(ui::UiFrame::scaler(width, height));
    template_editor_ui::draw(
        &mut frame,
        app,
        &themed.ui,
        themed.fonts.sdf(),
        width,
        height,
    );
    // An open dropdown list (the node panel's Activation Function) hangs over the editor.
    if let Some(open) = app.dropdown_open.as_ref() {
        dropdown::emit(
            &mut frame,
            &themed.ui,
            themed.fonts.sdf(),
            open,
            Vec2::new(width, height),
            app.cursor,
        );
    }
    tooltip::emit(
        &mut frame,
        app,
        &themed.ui,
        themed.fonts.sdf(),
        width,
        height,
    );
    frame
}

/// A frame of the Bibite Editor opened from the title menu's Library: the pointer and the keyboard are its (the menu waits behind
/// it), through the same routing as the world's panels.
fn host_editor_frame(app: &mut App, themed: &ThemedUi) {
    let (width, height) = (screen_width(), screen_height());
    app.set_viewport(width, height);
    let raw_input = gather_input();
    let frame = build_editor_frame(app, themed, width, height);
    let input = route_ui_frame(app, &frame, raw_input, None, width, height);
    while let Some(character) = get_char_pressed() {
        if !character.is_control() {
            app.typed.push(character);
        }
    }
    if input.paste_tag {
        app.clipboard_in = miniquad::window::clipboard_get();
    }
    app.handle_input(&input);
    if let Some(text) = app.clipboard_out.take() {
        miniquad::window::clipboard_set(&text);
    }
}

/// The largest the menus are rendered at: they are painted in software, and a 4K window should not
/// make every hover repaint eight million pixels.
const MENU_MAX_PIXELS: (f32, f32) = (2560.0, 1440.0);

/// The pixel size the menus are rendered at for a window of `width` x `height` logical pixels on a
/// display scaled by `dpi`: the window's own pixels, so their text is not stretched (a menu drawn at
/// the launch size and stretched to a maximised or fullscreen window is soft all over), capped at
/// [`MENU_MAX_PIXELS`]. `None` while the window is too small to draw in (minimised).
fn menu_pixels_for(width: f32, height: f32, dpi: f32) -> Option<(u32, u32)> {
    let (mut w, mut h) = (width * dpi, height * dpi);
    if !(w >= 160.0 && h >= 90.0) {
        return None;
    }
    let shrink = (MENU_MAX_PIXELS.0 / w).min(MENU_MAX_PIXELS.1 / h).min(1.0);
    w *= shrink;
    h *= shrink;
    Some((w.round() as u32, h.round() as u32))
}

/// Software-renders the current menu screen and uploads it as a GPU texture.
/// The window path intentionally reuses the deterministic software text/UI
/// rasteriser so menu screenshots and the window show the same pixels.
fn build_menu_texture(
    menu: &mut menu::MenuState,
    themed: &mut ThemedUi,
    args: &Args,
    assets: Option<&GameAssets>,
    previews: &preview::PreviewCache,
) -> Option<(Texture2D, Vec<tooltip::TipRect>)> {
    menu.refresh_library_picture(
        assets,
        previews,
        ui::UiFrame::scaler(args.width as f32, args.height as f32),
    );
    let frame = menu.build_frame(
        &themed.ui,
        &mut themed.fonts,
        args.width as f32,
        args.height as f32,
        themed.runtime.as_ref(),
    );
    let tips = frame.tips.clone();
    let overlay = menu::menu_overlay(&themed.ui, &frame, &mut themed.fonts);
    let image = render::render_ui_only(args.width, args.height, 1, Some(overlay));
    Some((
        Texture2D::from_rgba8(image.width as u16, image.height as u16, &image.rgba),
        tips,
    ))
}

/// The title menu's tooltip (`TooltipTrigger`): `tips` are in the menu's own pixel space (the size of
/// `args`), the pointer and the box in screen pixels; the box is drawn over the menu texture.
fn draw_menu_tooltip(
    state: &mut tooltip::State,
    tips: &[tooltip::TipRect],
    themed: &ThemedUi,
    gpu: &ui::UiGpu,
    args: &Args,
) {
    let (mouse_x, mouse_y) = mouse_position();
    let screen = Vec2::new(screen_width(), screen_height());
    let point = Vec2::new(
        mouse_x * args.width as f32 / screen.x,
        mouse_y * args.height as f32 / screen.y,
    );
    let target = tips
        .iter()
        .rev()
        .find(|rect| rect.contains(point))
        .and_then(|rect| rect.tip.ui.then_some(tooltip::Target::Control(rect.tip)));
    state.update(target, get_frame_time());
    if let Some((tooltip::Target::Control(tip), alpha)) = state.shown() {
        let mut frame = ui::UiFrame::new(ui::UiFrame::scaler(screen.x, screen.y));
        tooltip::draw(
            &mut frame,
            &themed.ui,
            &tooltip::control_body(&tip),
            alpha,
            Vec2::new(mouse_x, mouse_y),
            screen,
            &tooltip::measurer(themed.fonts.sdf()),
        );
        ui::draw_frame_macroquad(&frame, gpu);
    }
}

fn run_window(
    loaded: LoadedWorld,
    args: Args,
    assets: Option<GameAssets>,
    themed: Option<ThemedUi>,
    mut menu: Option<menu::MenuState>,
) {
    let mut conf = Conf {
        window_title: if menu.is_some() {
            "RUSTibites".to_owned()
        } else {
            format!("RUSTibites viewer - {}", loaded.label)
        },
        window_width: args.width as i32,
        window_height: args.height as i32,
        high_dpi: true,
        ..Default::default()
    };
    if let Some(icon) = load_window_icon() {
        conf.icon = Some(icon);
    }

    macroquad::Window::from_config(conf, async move {
        let mut themed = themed;
        // The menus are painted at `args` size: it follows the window's pixels (below).
        let mut args = args;
        let mut app = App::new(loaded, Vec2::new(screen_width(), screen_height()), assets);
        eprintln!("{}", threads_line(&app.world));
        start_sim_thread_if_asked(&mut app, &args, themed.as_ref());
        use_game_storage(&mut app);
        app.source_scenario = args.input.clone().filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        });
        apply_user_settings(&mut app, &load_user_settings());
        apply_stored_toggles(&mut app);
        apply_view_options(&mut app, &args);
        apply_game_camera(&mut app, &args, screen_height());
        if let Some(ortho) = args.ortho_size {
            app.set_ortho_size(ortho, screen_height());
        }
        // The scene options of the screenshot path that make sense in a window too (the real-window captures compare with the headless ones).
        if args.select_first {
            app.select_first();
        }
        if args.show_targets {
            app.show_targets = true;
        }
        if let Some(tab) = args.tab {
            app.inspector_tab = tab;
            app.inspector_open = true;
        }
        if args.center_on_selected {
            app.center_on_selected();
        }
        apply_lineage_flags(&mut app, &args);
        // `--click` in the window: the scripted presses of the headless path, applied a few frames in (the window may still change size at
        // start: a fullscreen setting switches it to the monitor's, and the UI is laid out for the size the click lands on); the real-window
        // captures of `tools/window_capture.ps1` then show a panel in the state those clicks leave it in. Points are window pixels.
        let mut scripted_clicks: Vec<Vec2> = if menu.is_none() {
            args.click.into_iter().chain(args.extra_clicks.iter().copied()).collect()
        } else {
            Vec::new()
        };
        let mut world_frames = 0usize;
        let self_shot = self_shot_request();
        let textures = render::upload_atlases(app.assets.as_ref());
        let hue_material = if args.gpu_hue_shader && machine_calibration::calibrator().graphics().is_none_or(|graphics| graphics.prefers_gpu_shader()) {
            let skin_params = app
                .assets
                .as_ref()
                .map_or_else(skin::SkinParams::default, |assets| {
                    skin::SkinParams::from(&assets.parameters)
                });
            match render::GpuHueMaterial::load(skin_params) {
                Ok(material) => {
                    eprintln!("renderer: GPU hue-match shader enabled");
                    Some(material)
                }
                Err(error) => {
                    eprintln!(
                        "warning: {error}; the window recolours the bibites' sprites on the processor instead (the same colours, slower with many \
                         bibites; --screenshot is not affected)"
                    );
                    None
                }
            }
        } else {
            eprintln!(
                "renderer: GPU hue-match shader off (--no-gpu-hue-shader, or the calibration found the processor's way faster here): the bibites' sprites are recoloured on the processor (the same colours)"
            );
            None
        };
        // The zones' fill on the GPU (the software fallback is the picture a quarter of the window's size).
        let zone_material = match render::GpuZoneMaterial::load() {
            Ok(material) => Some(material),
            Err(error) => {
                eprintln!("warning: {error}; the zones are drawn by the software rasteriser at a quarter of the resolution");
                None
            }
        };
        let gpu_ui = themed
            .as_ref()
            .and_then(|themed| match ui::upload_macroquad(&themed.ui, themed.fonts.sdf_handle()) {
                Ok(gpu) => Some(gpu),
                Err(error) => {
                    eprintln!("warning: UI upload failed ({error}); using simple HUD");
                    None
                }
            });
        eprintln!(
            "renderer: {} ({} atlas textures)",
            if app.assets.is_some() {
                "game sprites"
            } else {
                "procedural fallback"
            },
            textures.len(),
        );
        let mut menu_texture: Option<Texture2D> = None;
        // The tooltips of the title menu's controls and their hover timer.
        let mut menu_tips: Vec<tooltip::TipRect> = Vec::new();
        let mut menu_tooltip = tooltip::State::default();
        let mut menu_dirty = true;
        let mut loading_waited = false;
        // The world the Loading screen is waiting for: it is read on a thread of its own so that the window keeps answering.
        let mut world_loader: Option<
            std::sync::mpsc::Receiver<Result<(LoadedWorld, Option<challenge_run::ChallengeSetup>), String>>,
        > = None;
        let mut capture_window = false;
        // The title screen's animated background (the game's looping `showOff.webm`).
        let mut menu_video: Option<menu_video::MenuVideo> = None;
        let mut video_texture: Option<Texture2D> = None;
        let mut video_seen = 0_u64;
        let mut video_tried = false;
        // Whether the title screen's video was stopped for the calibration test (it starts again when the test is over).
        let mut video_paused_for_test = false;
        // In-game Settings / Library / Achievements / Tools / Credits panels replay the
        // title-menu screens (live data) over the dimmed world.
        let mut overlay_menu: Option<menu::MenuState> = None;
        let mut overlay_texture: Option<Texture2D> = None;
        let mut overlay_dirty = true;
        let mut frame_log = FrameLog::from_env();
        let mut pool_switches_logged = 0_u32;
        // What the Calibration page said when it was last painted (it is painted again when this changes).
        let mut calibration_line = String::new();
        // The window opens as the stored `Fullscreen` setting says (the game's default is fullscreen; `Screen.fullScreen = val` at start-up).
        if load_user_settings().flag("Fullscreen") {
            miniquad::window::set_fullscreen(true);
        }
        loop {
            // The ticks of the last frame ran beside its drawing (`tick_overlap`): the world is home before anything reads it.
            app.join_overlapped_ticks();
            log_pool_switches(&mut app.world, &mut pool_switches_logged);
            app.begin_frame();
            // The machine's calibration (Settings > Calibration): a new result, or the check box, reaches the world that is open, and the test of the graphics
            // follows the processor's part of the test.
            let world_running = menu.is_none() && !app.paused;
            calibration_frame(&mut app, &args, world_running, &textures, hue_material.as_ref(), zone_material.as_ref());
            // The title screen's background video is decoded by a process of its own: it stops while the test runs (the cores it takes would be measured) and starts again after.
            if machine_calibration::calibrator().running() {
                if menu_video.take().is_some() {
                    video_paused_for_test = true;
                    video_texture = None;
                }
            } else if std::mem::take(&mut video_paused_for_test) {
                video_tried = false;
            }
            // The Fullscreen setting (Settings screens, title menu or world): `Screen.fullScreen = val`.
            if let Some(on) = user_settings::take_fullscreen_change() {
                miniquad::window::set_fullscreen(on);
            }
            // A maximised, fullscreen or scaled window gets menus painted at its own size.
            if let Some((width, height)) = menu_pixels_for(
                screen_width(),
                screen_height(),
                miniquad::window::dpi_scale(),
            ) && (width, height) != (args.width, args.height)
            {
                args.width = width;
                args.height = height;
                menu_dirty = true;
                overlay_dirty = true;
            }
            if menu.is_none() && (menu_video.is_some() || video_tried) {
                // In a world: the decoder stops, the next visit to the title restarts it.
                menu_video = None;
                video_texture = None;
                video_seen = 0;
                video_tried = false;
            }
            if let Some(state) = menu.as_mut() {
                use menu::MenuView;
                if !video_tried && themed.is_some() && !state.overlay() {
                    video_tried = true;
                    menu_video = menu_video::MenuVideo::start(args.width, args.height);
                }
                if let Some(video) = menu_video.as_mut() {
                    if !video.alive() {
                        menu_video = None;
                        video_texture = None;
                        state.set_video_background(false);
                        menu_dirty = true;
                    } else if let Some((counter, frame)) = video.poll(video_seen) {
                        let (width, height) = video.size();
                        match video_texture.as_ref() {
                            Some(texture) => texture.update_from_bytes(width, height, &frame),
                            None => {
                                let texture =
                                    Texture2D::from_rgba8(width as u16, height as u16, &frame);
                                texture.set_filter(FilterMode::Linear);
                                video_texture = Some(texture);
                            }
                        }
                        video_seen = counter;
                        if !state.video_background() {
                            // The first frame arrived: the screens drop their static background.
                            state.set_video_background(true);
                            menu_dirty = true;
                        }
                    }
                }
                // What the background check of the Load list's saves has found (the popups of the unreadable ones).
                if state.poll_background() {
                    menu_dirty = true;
                }
                // The world is read on a thread of its own: a big save takes seconds (7,800 bibites: 4 s) to minutes (the autosaves of a run that
                // had grown to 200,000), and a window that does not take its messages for five seconds is "not responding". The Loading screen
                // goes on being drawn, and the window answers, until the thread has the world.
                if world_loader.is_some() && state.view() != MenuView::Loading {
                    // The player left the Loading screen: the world, when it comes, is not wanted.
                    world_loader = None;
                }
                let mut loading = None;
                if state.view() == MenuView::Loading && loading_waited {
                    if world_loader.is_none() {
                        let path = state.pending().map(Path::to_path_buf).unwrap_or_default();
                        // `StartChallenge`: the scenario with the champion's row in it, and the challenge to run on it.
                        let challenge = state.pending_challenge().cloned();
                        let (seed, bibites, threads) = (args.seed, args.bibites, args.threads);
                        let (sender, receiver) = std::sync::mpsc::channel();
                        let spawned = std::thread::Builder::new().name("world-loader".to_owned()).spawn(move || {
                            let started = std::time::Instant::now();
                            let loaded = match challenge {
                                Some(start) => LoadedWorld::load_challenge_start(&start, seed, threads),
                                None => LoadedWorld::load(Some(&path), seed, bibites, threads).map(|loaded| (loaded, None)),
                            };
                            eprintln!("load: {} took {:.1} s", path.display(), started.elapsed().as_secs_f32());
                            let _ = sender.send(loaded);
                        });
                        match spawned {
                            Ok(_) => world_loader = Some(receiver),
                            Err(error) => loading = Some(Err(format!("could not start a thread to load the world: {error}"))),
                        }
                    }
                    if let Some(receiver) = world_loader.as_ref() {
                        match receiver.try_recv() {
                            Ok(result) => {
                                world_loader = None;
                                loading = Some(result);
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => {}
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                world_loader = None;
                                loading = Some(Err("the loading thread stopped before it had the world".to_owned()));
                            }
                        }
                    }
                }
                if let Some(loading) = loading {
                    let path = state.pending().map(Path::to_path_buf).unwrap_or_default();
                    match loading {
                        Ok((mut loaded, challenge_setup)) => {
                            let opening = std::time::Instant::now();
                            if let Some(template) = state.pending_template() {
                                let template_path = template.to_string_lossy().to_string();
                                if let Err(error) = loaded.world.spawn_bibite_by_template_path(
                                    &template_path,
                                    bibites_core::settings::GrowthAtSpawn::Adult,
                                    bibites_core::settings::Tagging::NoTagging,
                                    "",
                                    Vec2::ZERO,
                                ) {
                                    eprintln!("library template spawn failed: {error}");
                                }
                            }
                            let app_assets = app.assets.take();
                            let menu_settings = state.settings();
                            let menu_scene = state.scene_options();
                            let menu_hud = state.show_hud();
                            let user_settings = state.user_settings().clone();
                            let loaded_save = state.pending_is_save();
                            app = App::new(
                                loaded,
                                Vec2::new(screen_width(), screen_height()),
                                app_assets,
                            );
                            start_sim_thread_if_asked(&mut app, &args, themed.as_ref());
                            use_game_storage(&mut app);
                            app.source_scenario = Some(path.clone());
                            // `ChallengeManager.Start`: the run of the challenge (a scenario that is only started as one has none).
                            if let Some(setup) = challenge_setup {
                                app.begin_challenge(setup);
                            }
                            apply_view_options(&mut app, &args);
                            apply_game_camera(&mut app, &args, screen_height());
                            if let Some(ortho) = args.ortho_size {
                                app.set_ortho_size(ortho, screen_height());
                            }
                            // Apply the title-menu options to the newly opened world.
                            app.scene_options = menu_scene;
                            app.show_hud = menu_hud;
                            let _ = menu_settings;
                            apply_user_settings(&mut app, &user_settings);
                            // `SimulationManager.Start`: a loaded game (not a new scenario) starts paused with *Pause After Load*.
                            if loaded_save && user_settings.flag("Pause After Load") {
                                app.paused = true;
                            }
                            eprintln!("load: the world is open after {:.1} s more on the window's own thread", opening.elapsed().as_secs_f32());
                            menu = None;
                            menu_texture = None;
                            continue;
                        }
                        Err(error) => {
                            state.fail(format!("Could not load {}: {error}", path.display()));
                            menu_dirty = true;
                            menu_texture = None;
                            loading_waited = false;
                        }
                    }
                }
                // The Bibite Editor opened from the Library is over the title menu's panels: the window's pointer and keyboard are
                // its until it closes (the menu waits behind it), and the Library behind it fills its lists again.
                let hosted = app.bibite_editor.is_some() && themed.is_some() && gpu_ui.is_some();
                if hosted && let Some(themed) = themed.as_ref() {
                    host_editor_frame(&mut app, themed);
                }
                if let Some(saved) = app.library_refill.take() {
                    state.refill_library(saved.as_deref());
                    menu_dirty = true;
                }
                let live = !hosted;
                let mut action = menu::MenuAction::None;
                if live && is_key_pressed(KeyCode::Up) {
                    if state.is_native_view() {
                        state.native_move(-1);
                    } else {
                        state.move_selection(-1);
                    }
                    menu_dirty = true;
                }
                if live && is_key_pressed(KeyCode::Down) {
                    if state.is_native_view() {
                        state.native_move(1);
                    } else {
                        state.move_selection(1);
                    }
                    menu_dirty = true;
                }
                if live && is_key_pressed(KeyCode::Escape) {
                    if state.view() == MenuView::Main {
                        miniquad::window::request_quit();
                        break;
                    }
                    state.back();
                    menu_dirty = true;
                }
                if live && is_key_pressed(KeyCode::Enter) {
                    action = if state.is_native_view() {
                        state.native_enter()
                    } else {
                        state.activate()
                    };
                    menu_dirty = true;
                }
                if live
                    && matches!(
                        state.view(),
                        menu::MenuView::NewGame
                            | menu::MenuView::LoadGame
                            | menu::MenuView::Settings
                            | menu::MenuView::Library
                            | menu::MenuView::Tools
                            | menu::MenuView::Credits
                            | menu::MenuView::ScenarioEditor
                    )
                {
                    let wheel = wheel_notches();
                    if wheel != 0.0 {
                        state.scroll_list(-wheel.signum() * state.wheel_step());
                        menu_dirty = true;
                    }
                    let before = state.list_scroll();
                    state.spring(get_frame_time());
                    if state.list_scroll() != before {
                        menu_dirty = true;
                    }
                }
                // An open dropdown list lights the row under the pointer.
                {
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(
                        mouse_x * args.width as f32 / screen_width(),
                        mouse_y * args.height as f32 / screen_height(),
                    );
                    if state.set_cursor(point) {
                        menu_dirty = true;
                    }
                }
                if input_log() && is_mouse_button_pressed(MouseButton::Left) {
                    let (mouse_x, mouse_y) = mouse_position();
                    eprintln!(
                        "press: view {:?}, live {live}, mouse ({mouse_x:.1}, {mouse_y:.1}) of {:.1} x {:.1}, menu picture {} x {}",
                        state.view(),
                        screen_width(),
                        screen_height(),
                        args.width,
                        args.height
                    );
                }
                if live
                    && is_mouse_button_pressed(MouseButton::Left)
                    && !matches!(state.view(), menu::MenuView::Main | menu::MenuView::Loading)
                    && let Some(themed) = themed.as_mut()
                {
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(
                        mouse_x * args.width as f32 / screen_width(),
                        mouse_y * args.height as f32 / screen_height(),
                    );
                    state.ctrl_down = is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
                    state.shift_down = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
                    if let Some(result) = state.click_native(
                        &themed.ui,
                        &mut themed.fonts,
                        args.width as f32,
                        args.height as f32,
                        themed.runtime.as_ref(),
                        point,
                    ) {
                        action = result;
                    }
                    menu_dirty = true;
                } else if live && is_mouse_button_pressed(MouseButton::Left) {
                    let (mouse_x, mouse_y) = mouse_position();
                    // The title menu's picture is painted at `args` (the window's physical pixels, capped), then stretched to the window: the
                    // click is tested in the picture's own space, like every other menu screen (testing it in logical window pixels against
                    // buttons laid out for the window's size went wrong whenever the two layouts differed: a UI scale, a resolution setting, a
                    // display scaling or a resize left the buttons where they were not drawn).
                    let point = menu::picture_point(
                        Vec2::new(mouse_x, mouse_y),
                        Vec2::new(screen_width(), screen_height()),
                        Vec2::new(args.width as f32, args.height as f32),
                    );
                    let row = state.hit_row(
                        point.x,
                        point.y,
                        args.width as f32,
                        args.height as f32,
                        themed.as_ref().and_then(|themed| themed.runtime.as_ref()),
                    );
                    if input_log() {
                        // `RUSTIBITES_INPUT_LOG=1`: what the window thinks the pointer and the screen are at a click on the title screen
                        // (a tester's buttons did not react in fullscreen on a 2560 x 1440 monitor).
                        let (framebuffer_w, framebuffer_h) = miniquad::window::screen_size();
                        eprintln!(
                            "click: mouse ({mouse_x:.1}, {mouse_y:.1}) of screen {:.1} x {:.1} (framebuffer {framebuffer_w} x {framebuffer_h}, dpi scale {:.3}), menu picture {} x {}, row {row:?}, button rects {:?}",
                            screen_width(),
                            screen_height(),
                            miniquad::window::dpi_scale(),
                            args.width,
                            args.height,
                            themed
                                .as_ref()
                                .and_then(|themed| themed.runtime.as_ref())
                                .map(|runtime| runtime.menu_button_rects(args.width as f32, args.height as f32))
                        );
                    }
                    if let Some(row) = row {
                        state.select(row);
                        action = state.activate();
                        menu_dirty = true;
                    }
                }
                if live && state.slider_dragging() {
                    // A slider of the Settings pages or of the TPS panel follows the held mouse, wherever it goes.
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(
                        mouse_x * args.width as f32 / screen_width(),
                        mouse_y * args.height as f32 / screen_height(),
                    );
                    if is_mouse_button_down(MouseButton::Left) && state.slider_drag_move(point) {
                        menu_dirty = true;
                    }
                    if is_mouse_button_released(MouseButton::Left) || !is_mouse_button_down(MouseButton::Left) {
                        state.slider_release();
                    }
                }
                if live && (state.bar_dragging() || state.list_bar().is_some()) {
                    // A list's scrollbar follows the held mouse.
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(
                        mouse_x * args.width as f32 / screen_width(),
                        mouse_y * args.height as f32 / screen_height(),
                    );
                    if is_mouse_button_down(MouseButton::Left)
                        && state.bar_drag_move(point, args.width as f32, args.height as f32)
                    {
                        menu_dirty = true;
                    }
                    if is_mouse_button_released(MouseButton::Left) {
                        state.bar_release();
                        menu_dirty = true;
                    }
                }
                if live && state.view() == menu::MenuView::ScenarioEditor {
                    // Sliders follow the held mouse; the save form takes typed text.
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(
                        mouse_x * args.width as f32 / screen_width(),
                        mouse_y * args.height as f32 / screen_height(),
                    );
                    if is_mouse_button_down(MouseButton::Left)
                        && state.editor_drag(point, args.width as f32, args.height as f32)
                    {
                        menu_dirty = true;
                    }
                    if is_mouse_button_released(MouseButton::Left) {
                        state.editor_release();
                    }
                    if state.editor_typing() {
                        while let Some(character) = get_char_pressed() {
                            state.editor_type(character);
                            menu_dirty = true;
                        }
                        if is_key_pressed(KeyCode::Backspace) {
                            state.editor_backspace();
                            menu_dirty = true;
                        }
                        if (is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl)) && is_key_pressed(KeyCode::V) {
                            if let Some(text) = miniquad::window::clipboard_get() {
                                state.editor_paste(&text);
                                menu_dirty = true;
                            }
                        }
                        if state.editor_tick(get_frame_time()) {
                            menu_dirty = true;
                        }
                        {
                            use value_edit::CaretKey;
                            let shift = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
                            let ctrl = is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
                            let key = [
                                (KeyCode::Left, CaretKey::Left),
                                (KeyCode::Right, CaretKey::Right),
                                (KeyCode::Home, CaretKey::Home),
                                (KeyCode::End, CaretKey::End),
                            ]
                            .into_iter()
                            .find(|(code, _)| is_key_pressed(*code))
                            .map(|(_, key)| key)
                            .or_else(|| (ctrl && is_key_pressed(KeyCode::A)).then_some(CaretKey::SelectAll));
                            let delete = is_key_pressed(KeyCode::Delete);
                            if key.is_some() || delete {
                                state.editor_caret_key(key, delete, shift);
                                menu_dirty = true;
                            }
                        }
                        if is_key_pressed(KeyCode::Tab) {
                            state.editor_next_field();
                            menu_dirty = true;
                        }
                    }
                }
                if action == menu::MenuAction::Quit {
                    miniquad::window::request_quit();
                    break;
                }
                // The Library's Edit Bibite, Edit and Create New buttons: the Bibite Editor opens over the title menu.
                if let menu::MenuAction::EditTemplate { path, submit } = &action {
                    app.open_editor_from_library(path.as_deref(), *submit);
                }
                // The Calibration page's check box reaches the calibrator at once (the title menu hands its settings to a world only when one opens).
                if state.view() == menu::MenuView::Settings {
                    machine_calibration::calibrator().set_use(state.user_settings().flag(machine_calibration::LABEL_USE));
                }
                // The Calibration page is painted again when what it says changes by itself (the test's progress, the hardware answering).
                if state.view() == menu::MenuView::Settings && state.settings_tab() == settings_pages::CALIBRATION_TAB {
                    let line = machine_calibration::page_signature();
                    if line != calibration_line {
                        calibration_line = line;
                        menu_dirty = true;
                    }
                }
                if menu_dirty || menu_texture.is_none() {
                    if let Some(themed) = themed.as_mut() {
                        (menu_texture, menu_tips) = match build_menu_texture(
                            state,
                            themed,
                            &args,
                            app.assets.as_ref(),
                            &app.previews,
                        ) {
                            Some((texture, tips)) => (Some(texture), tips),
                            None => (None, Vec::new()),
                        };
                    }
                    menu_dirty = false;
                }
                clear_background(BLACK);
                if state.shows_video()
                    && let Some(video) = video_texture.as_ref()
                {
                    // Navy base, then the video inset by (2, 1) of 1920x1080 like the game's
                    // `VideoTexture` RawImage.
                    clear_background(macroquad::prelude::Color::new(0.0, 0.0, 0.1216, 1.0));
                    let (sx, sy) = (screen_width() / 1920.0, screen_height() / 1080.0);
                    draw_texture_ex(
                        video,
                        2.0 * sx,
                        1.0 * sy,
                        WHITE,
                        DrawTextureParams {
                            dest_size: Some(macroquad::prelude::vec2(1916.0 * sx, 1078.0 * sy)),
                            ..Default::default()
                        },
                    );
                }
                if let Some(texture) = menu_texture.as_ref() {
                    draw_texture_ex(
                        texture,
                        0.0,
                        0.0,
                        WHITE,
                        DrawTextureParams {
                            dest_size: Some(macroquad::prelude::vec2(
                                screen_width(),
                                screen_height(),
                            )),
                            ..Default::default()
                        },
                    );
                }
                if let (Some(themed), Some(gpu)) = (themed.as_ref(), gpu_ui.as_ref()) {
                    draw_menu_tooltip(&mut menu_tooltip, &menu_tips, themed, gpu, &args);
                    // The Bibite Editor, over everything.
                    if app.bibite_editor.is_some() {
                        let frame =
                            build_editor_frame(&app, themed, screen_width(), screen_height());
                        ui::draw_frame_macroquad(&frame, gpu);
                    }
                }
                if state.view() == MenuView::Loading {
                    loading_waited = true;
                }
                if is_key_pressed(KeyCode::F12) {
                    save_window_capture();
                }
                next_frame().await;
                continue;
            }
            let frame_dt = get_frame_time();
            world_frames += 1;
            // The animated shaders (a radiation tower's area) run on the game's `Time.time`: the window's clock.
            render::set_effect_time(get_time() as f32);
            if self_shot.as_ref().is_some_and(|(frame, _)| *frame == world_frames) {
                capture_window = true;
            }
            if world_frames >= 8
                && !scripted_clicks.is_empty()
                && let Some(themed) = themed.as_ref()
            {
                for point in std::mem::take(&mut scripted_clicks) {
                    let frame = build_ui_frame(&app, themed, screen_width(), screen_height());
                    let actions = frame.interact(point, true, true, &mut ui::UiInteraction::default());
                    eprintln!("window: scripted click {point:?} on {}x{}: {} action(s)", screen_width(), screen_height(), actions.len());
                    remember_click_rect(&mut app, &frame, point, &actions);
                    app.apply_ui_actions(&actions);
                }
                overlay_dirty = true;
            }
            app.set_viewport(screen_width(), screen_height());
            // Hotkey rebinding: capture the next key press globally.
            let mut rebound_this_frame = false;
            if app.hotkey_listening.is_some()
                && let Some(key) = get_last_key_pressed()
            {
                rebound_this_frame = true;
                if key == KeyCode::Escape {
                    app.hotkey_listening = None;
                } else {
                    let label = format!("{key:?}");
                    if let Some(index) = app.hotkey_listening {
                        app.assign_hotkey(index, &label);
                    }
                }
            }
            let overlay_view = overlay_view_of(app.panels.top());
            let overlay_active = overlay_view.is_some() && themed.is_some() && gpu_ui.is_some();
            // The menu behind an in-game panel (its lists of saves and bibites, its copy of the settings, a slider it holds) is made when the panel opens and is dropped when the
            // panel is gone from the stack. Escape closes a panel without the menu going back to its main view (only the Back button does), and the menu left behind listed the
            // saves and bibites of the panel's first opening, answered with the settings of then (and put them back into the running world at every click), and kept a held
            // slider armed. (A panel under another one, the Bibite Editor over the Library, is still on the stack: its menu stays.)
            if overlay_menu.is_some() && !overlay_panel_open(&app) {
                overlay_menu = None;
                overlay_texture = None;
                overlay_dirty = true;
            }
            // A template pick whose Library overlay is gone (Escape closed it) is over: the
            // placer's pause ends with the selector (`onCancelAction`).
            // (The Bibite Editor over the Library keeps the pick: `onCancelAction` only runs when the selector is left.)
            if app.template_pick.is_some()
                && !(overlay_active && overlay_view == Some(menu::MenuView::Library))
                && !(app.bibite_editor.is_some() && app.panels.is_open(ui::PanelKind::Library))
            {
                app.cancel_template_pick();
            }
            let mut start_loading = false;
            if !overlay_active {
                overlay_dirty = true;
            }
            if overlay_active {
                let view = overlay_view.unwrap_or(menu::MenuView::Settings);
                let state = overlay_menu.get_or_insert_with(|| {
                    let mut state = make_menu(&args);
                    state.set_overlay(true);
                    state
                });
                // The popups of the unreadable saves the background check has found (the Load list's "Include autosaves" starts one).
                if state.poll_background() {
                    overlay_dirty = true;
                }
                if state.view() != view {
                    state.open_view_for_capture(view);
                    overlay_dirty = true;
                }
                // The Bibite Editor closed over this Library: it fills its lists again (`AfterEditorClose`, `AfterEditorSubmit`).
                if view == menu::MenuView::Library
                    && let Some(saved) = app.library_refill.take()
                {
                    state.refill_library(saved.as_deref());
                    overlay_dirty = true;
                }
                // The Default Bibites `+` or the Placer's `LoadBibite` opened the Library to pick a
                // template.
                if let Some(purpose) = app.template_pick
                    && view == menu::MenuView::Library
                    && state.pick().is_none()
                {
                    state.begin_pick(match purpose {
                        app::TemplateUse::DefaultBibite => menu::TemplatePick::DefaultBibite,
                        app::TemplateUse::PlaceBibite => menu::TemplatePick::PlaceBibite,
                    });
                    overlay_dirty = true;
                }
                {
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(
                        mouse_x * args.width as f32 / screen_width(),
                        mouse_y * args.height as f32 / screen_height(),
                    );
                    if state.set_cursor(point) {
                        overlay_dirty = true;
                    }
                }
                // A press on a screen that scrolls waits for its release: a drag moves the list, a still press is the click.
                let mut release_click: Option<Vec2> = None;
                if is_mouse_button_pressed(MouseButton::Left) {
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(mouse_x * args.width as f32 / screen_width(), mouse_y * args.height as f32 / screen_height());
                    // A press on a slider is the slider's (`Slider.OnPointerDown`): it is not held back for a drag of the page.
                    let on_slider = themed.as_mut().is_some_and(|themed| {
                        state.slider_under(&themed.ui, &mut themed.fonts, args.width as f32, args.height as f32, themed.runtime.as_ref(), point)
                    });
                    if !on_slider && state.press_hold(point, args.width as f32, args.height as f32) {
                        overlay_dirty = true;
                    }
                }
                if is_mouse_button_down(MouseButton::Left) {
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(mouse_x * args.width as f32 / screen_width(), mouse_y * args.height as f32 / screen_height());
                    if state.content_drag_move(point, args.width as f32, args.height as f32, get_frame_time()) {
                        overlay_dirty = true;
                    }
                }
                if is_mouse_button_released(MouseButton::Left) {
                    release_click = state.content_release(args.width as f32, args.height as f32);
                }
                if (is_mouse_button_pressed(MouseButton::Left) && !state.content_dragging() || release_click.is_some())
                    && let Some(themed) = themed.as_mut()
                {
                    // The panel is rendered by the software rasteriser at the window's
                    // creation size (like the title menu) and stretched to the window.
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = release_click.unwrap_or_else(|| {
                        Vec2::new(mouse_x * args.width as f32 / screen_width(), mouse_y * args.height as f32 / screen_height())
                    });
                    overlay_dirty = true;
                    let result = state.click_native(
                        &themed.ui,
                        &mut themed.fonts,
                        args.width as f32,
                        args.height as f32,
                        themed.runtime.as_ref(),
                        point,
                    );
                    // Changed settings take effect in the running world.
                    apply_user_settings(&mut app, state.user_settings());
                    if matches!(result, Some(menu::MenuAction::Start(_))) {
                        // Load Game picked a save: the title menu's loading flow takes over.
                        start_loading = true;
                    }
                    if let Some(menu::MenuAction::PickedTemplate(path)) = &result {
                        // "Select Bibite": the template becomes a default bibite of the world, or
                        // the one the placer places.
                        match app.template_pick.take() {
                            Some(app::TemplateUse::PlaceBibite) => app.start_bibite_placer(path),
                            _ => app.add_default_bibite(path),
                        }
                    }
                    // "Edit Bibite", the Edit button and Create New: the Bibite Editor opens over the Library.
                    if let Some(menu::MenuAction::EditTemplate { path, submit }) = &result {
                        app.open_editor_from_library(path.as_deref(), *submit);
                    }
                }
                if state.slider_dragging() {
                    // A slider of the Settings pages follows the held mouse, wherever it goes, and the running world takes each value.
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(
                        mouse_x * args.width as f32 / screen_width(),
                        mouse_y * args.height as f32 / screen_height(),
                    );
                    if is_mouse_button_down(MouseButton::Left) && state.slider_drag_move(point) {
                        apply_user_settings(&mut app, state.user_settings());
                        overlay_dirty = true;
                    }
                    if is_mouse_button_released(MouseButton::Left) || !is_mouse_button_down(MouseButton::Left) {
                        state.slider_release();
                    }
                }
                if state.bar_dragging() || state.list_bar().is_some() {
                    // A list's scrollbar follows the held mouse.
                    let (mouse_x, mouse_y) = mouse_position();
                    let point = Vec2::new(
                        mouse_x * args.width as f32 / screen_width(),
                        mouse_y * args.height as f32 / screen_height(),
                    );
                    if is_mouse_button_down(MouseButton::Left)
                        && state.bar_drag_move(point, args.width as f32, args.height as f32)
                    {
                        overlay_dirty = true;
                    }
                    if is_mouse_button_released(MouseButton::Left) {
                        state.bar_release();
                        overlay_dirty = true;
                    }
                }
                let wheel = wheel_notches();
                if wheel != 0.0 {
                    state.scroll_list(-wheel.signum() * state.wheel_step());
                    overlay_dirty = true;
                }
                let before = state.list_scroll();
                state.spring(get_frame_time());
                if state.list_scroll() != before {
                    overlay_dirty = true;
                }
                if state.view() == menu::MenuView::Main {
                    // "Back" (the close button) left the screen: close the panel.
                    app.panels.close_top();
                    app.cancel_template_pick();
                    overlay_menu = None;
                    overlay_texture = None;
                }
            }
            if start_loading && let Some(mut state) = overlay_menu.take() {
                state.set_overlay(false);
                menu = Some(state);
                menu_texture = None;
                menu_dirty = true;
                loading_waited = false;
                app.panels.close_top();
                continue;
            }
            let raw_input = gather_input();
            // Where the world is for this frame (see `sim_thread`): on its own thread while the window only watches.
            app.frame_world(&raw_input);
            let input = if overlay_active {
                // The panel swallows the mouse; only Escape (close) gets through.
                FrameInput {
                    quit: raw_input.quit,
                    ..FrameInput::default()
                }
            } else if let Some(themed) = themed.as_ref() {
                let frame = build_ui_frame(&app, themed, screen_width(), screen_height());
                route_ui_frame(
                    &mut app,
                    &frame,
                    raw_input,
                    themed.runtime.as_ref(),
                    screen_width(),
                    screen_height(),
                )
            } else {
                raw_input
            };
            let ticks_before = app.sim_ticks;
            let input = if rebound_this_frame {
                // The captured key (or Escape cancelling the rebind) is consumed.
                FrameInput {
                    cursor: input.cursor,
                    primary_down: input.primary_down,
                    primary_pressed: input.primary_pressed,
                    primary_released: input.primary_released,
                    wheel: input.wheel,
                    ..FrameInput::default()
                }
            } else {
                input
            };
            // The clipboard and typed characters travel through the app (it has no window).
            while let Some(character) = get_char_pressed() {
                if !character.is_control() {
                    app.typed.push(character);
                }
            }
            if input.paste_tag {
                app.clipboard_in = miniquad::window::clipboard_get();
            }
            app.handle_input(&input);
            app.settle_world();
            if let Some(text) = app.clipboard_out.take() {
                miniquad::window::clipboard_set(&text);
            }
            if let Some(url) = app.pending_url.take() {
                open_in_browser(&url);
            }
            if app.quit {
                miniquad::window::request_quit();
                break;
            }
            if app.return_to_menu {
                // `EscapePanel` "Back to Menu".
                app.return_to_menu = false;
                menu = Some(make_menu(&args));
                menu_texture = None;
                menu_dirty = true;
                loading_waited = false;
                continue;
            }
            // The ticks of a frame run beside its drawing only when the frame hands them over after its UI is built (`dispatch_overlapped_ticks`, below). The simple HUD (no
            // themed UI, or its upload to the GPU failed) reads the world while it draws, and has no UI frame to hand them over after: its frames run their ticks in `advance`.
            // (Without this the owed ticks waited for a dispatch that never came, and the world stood still from the second frame on.)
            app.overlap_ticks = args.overlap_ticks && themed.is_some() && gpu_ui.is_some();
            let section = std::time::Instant::now();
            app.advance(frame_dt);
            frame_log.advance += section.elapsed();
            // `RunChallenge` runs on scaled game time: feed the simulated
            // seconds actually stepped (zero while paused).
            let stepped = (app.sim_ticks.saturating_sub(ticks_before) + u64::from(app.take_joined_ticks())) as f32;
            frame_log.ticks += stepped as u64;
            app.update_challenge(stepped * app.world.settings.fixed_dt());
            app.update_autosave(frame_dt, stepped * app.world.settings.fixed_dt());

            // F12: write the current frame as a timestamped PNG (software path,
            // so the file matches the deterministic headless screenshot mode).
            if is_key_pressed(KeyCode::F12) {
                // What the window shows, UI included (the alpha's bug reports): `screenshots/rustibites-<time>.png`.
                capture_window = true;
            }

            let section = std::time::Instant::now();
            app.rebuild_scene();
            frame_log.scene += section.elapsed();
            // The UI is built from the world too. Once it is built nothing reads the world until the next frame, so the ticks this
            // frame owes run on the helper thread beside the drawing and the wait for the display (`tick_overlap`); the world is
            // home again right after `next_frame`.
            let section = std::time::Instant::now();
            let ui_frame = match (&themed, &gpu_ui) {
                (Some(themed), Some(_)) => {
                    let mut frame = build_ui_frame(&app, themed, screen_width(), screen_height());
                    // A button held down is drawn in its pressed colour.
                    if let Some(id) = app.interaction.active
                        && is_mouse_button_down(MouseButton::Left)
                    {
                        let (x, y) = mouse_position();
                        frame.tint_pressed(id, Vec2::new(x, y));
                    }
                    Some(frame)
                }
                _ => None,
            };
            frame_log.ui += section.elapsed();
            if ui_frame.is_some() {
                app.dispatch_overlapped_ticks();
            }
            let section = std::time::Instant::now();
            render::draw_scene(
                app.drawn_scene(),
                &app.camera,
                &textures,
                hue_material.as_ref(),
                zone_material.as_ref(),
                app.assets.as_ref().map_or(&[][..], |assets| &assets.atlases[..]),
            );
            frame_log.draw_scene += section.elapsed();
            let section = std::time::Instant::now();
            match (&ui_frame, &gpu_ui) {
                (Some(frame), Some(gpu)) => ui::draw_frame_macroquad(frame, gpu),
                // The curated HUD reads the world: no ticks are out in this frame.
                _ => draw_overlay(&app),
            }
            frame_log.ui += section.elapsed();
            // Settings / Library / Achievements / Tools / Credits / Load Game replay the
            // title-menu screens. (The generic "replay a whole captured session" path was
            // removed: the captures are cumulative click-throughs that leaked stale panels.)
            if overlay_active
                && let (Some(themed), Some(state)) = (themed.as_mut(), overlay_menu.as_mut())
            {
                // The Calibration page is painted again when what it says changes by itself.
                if state.view() == menu::MenuView::Settings && state.settings_tab() == settings_pages::CALIBRATION_TAB {
                    let line = machine_calibration::page_signature();
                    if line != calibration_line {
                        calibration_line = line;
                        overlay_dirty = true;
                    }
                }
                if overlay_dirty || overlay_texture.is_none() {
                    overlay_texture = build_menu_texture(
                        state,
                        themed,
                        &args,
                        app.assets.as_ref(),
                        &app.previews,
                    )
                    .map(|(texture, _)| texture);
                    overlay_dirty = false;
                }
                if let Some(texture) = overlay_texture.as_ref() {
                    draw_texture_ex(
                        texture,
                        0.0,
                        0.0,
                        WHITE,
                        DrawTextureParams {
                            dest_size: Some(macroquad::prelude::vec2(
                                screen_width(),
                                screen_height(),
                            )),
                            ..Default::default()
                        },
                    );
                }
            }
            if std::mem::take(&mut capture_window) {
                if let Some((_, path)) = self_shot.as_ref() {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    macroquad::prelude::get_screen_data().export_png(&path.to_string_lossy());
                    eprintln!("window: framebuffer saved to {}", path.display());
                    std::process::exit(0);
                }
                save_window_capture();
            }
            let section = std::time::Instant::now();
            next_frame().await;
            frame_log.present += section.elapsed();
            let section = std::time::Instant::now();
            app.join_overlapped_ticks();
            frame_log.join += section.elapsed();
            let (pellets, bibites) = app.frame_counts();
            frame_log.report(app.drawn_scene(), pellets, bibites, &app.time_keeper);
        }
    });
}

/// `Application.OpenURL`: hands the address to the system's default browser.
fn open_in_browser(url: &str) {
    #[cfg(windows)]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = result {
        eprintln!("could not open {url}: {error}");
    }
}

/// The title-menu screen an in-game panel replays (live-bound), if any.
fn overlay_view_of(panel: Option<ui::PanelKind>) -> Option<menu::MenuView> {
    match panel? {
        ui::PanelKind::Settings => Some(menu::MenuView::Settings),
        ui::PanelKind::Library => Some(menu::MenuView::Library),
        ui::PanelKind::Achievements => Some(menu::MenuView::Achievements),
        ui::PanelKind::Tools => Some(menu::MenuView::Tools),
        ui::PanelKind::Credits => Some(menu::MenuView::Credits),
        ui::PanelKind::LoadGame => Some(menu::MenuView::LoadGame),
        _ => None,
    }
}

/// The panels that replay a title-menu screen (see [`overlay_view_of`]).
const OVERLAY_PANELS: [ui::PanelKind; 6] = [
    ui::PanelKind::Settings,
    ui::PanelKind::Library,
    ui::PanelKind::Achievements,
    ui::PanelKind::Tools,
    ui::PanelKind::Credits,
    ui::PanelKind::LoadGame,
];

/// Whether one of them is anywhere on the Escape stack (not only on top of it: the Bibite Editor opens over the Library, which stays).
fn overlay_panel_open(app: &App) -> bool {
    OVERLAY_PANELS.iter().any(|panel| app.panels.is_open(*panel))
}

/// Fallback dispatch for audited controls that have no concrete `UiAction`.
fn dispatch_runtime_control(control: &ui_runtime::RuntimeControl) -> Vec<ui::UiAction> {
    let key =
        format!("{} {} {}", control.session, control.path, control.label).to_ascii_lowercase();
    if key.contains("workshop") {
        if key.contains("refresh") {
            return vec![ui::UiAction::Invoke(ui::ControlCommand::WorkshopRefresh)];
        }
        if key.contains("export") {
            return vec![ui::UiAction::Invoke(ui::ControlCommand::WorkshopExport)];
        }
        if key.contains("delete") {
            return vec![ui::UiAction::Invoke(ui::ControlCommand::WorkshopDelete)];
        }
        return vec![ui::UiAction::Invoke(ui::ControlCommand::OpenWorkshop)];
    }
    if key.contains("challenge picker")
        || key.contains("challenge selector")
        || key.contains("champion selection")
    {
        return vec![ui::UiAction::Invoke(
            ui::ControlCommand::OpenChallengePicker,
        )];
    }
    if key.contains("challenge parameters") || key.contains("stardescription") {
        return vec![ui::UiAction::Invoke(
            ui::ControlCommand::OpenChallengePicker,
        )];
    }
    let challenge_control = control.label.to_ascii_lowercase().contains("challenge")
        || control.name.to_ascii_lowercase().contains("challenge")
        || control.label.to_ascii_lowercase().contains("start");
    if key.contains("challenge") && challenge_control {
        if key.contains("restart") || key.contains("retry") {
            return vec![ui::UiAction::Invoke(ui::ControlCommand::ChallengeRestart)];
        }
        if key.contains("continue") {
            return vec![ui::UiAction::Invoke(ui::ControlCommand::ChallengeContinue)];
        }
        return vec![ui::UiAction::Invoke(ui::ControlCommand::StartChallenge)];
    }
    if key.contains("dynamic parameters") || key.contains("dynamicparameters") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::WorldDynamics)];
    }
    if key.contains("world settings") || key.contains("worldsettings") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::WorldSettings)];
    }
    if key.contains("gene editor") || key.contains("genes tab") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::OpenGeneEditor)];
    }
    if key.contains("brain editor") || key.contains("brain tab") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::OpenBrainEditor)];
    }
    if key.contains("scenario editor") || key.contains("scenarioeditor") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::OpenScenarioEditor)];
    }
    if key.contains("apply") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::EditorApply)];
    }
    if key.contains("save scenario") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::EditorSave)];
    }
    if key.contains("add zone") || key.contains("zones/add") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::EditorAddZone)];
    }
    if key.contains("remove zone") || key.contains("zones/remove") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::EditorRemoveZone)];
    }
    if key.contains("settings") || key.contains("options") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::Settings)];
    }
    if key.contains("place pellets") || key.contains("placepellets") || key.contains("placer") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::SelectTool(
            ui::ToolKind::Pellet,
        ))];
    }
    if key.contains("phero") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::SelectTool(
            ui::ToolKind::PheroTower,
        ))];
    }
    if key.contains("colorselector") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::SelectTool(
            ui::ToolKind::Killer,
        ))];
    }
    if key.contains("tower") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::SelectTool(
            ui::ToolKind::Tower,
        ))];
    }
    if key.contains("kill half") || key.contains("killhalf") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::RequestKillHalf)];
    }
    if key.contains("removepellets") || key.contains("remove pellets") {
        return vec![ui::UiAction::Invoke(
            ui::ControlCommand::RequestRemoveHalfPellets,
        )];
    }
    if key.contains("grid") {
        return vec![ui::UiAction::ToggleOverlay(ui::OverlayKind::Grid)];
    }
    if key.contains("pheromone") && key.contains("toggle") {
        return vec![ui::UiAction::ToggleOverlay(ui::OverlayKind::Pheromones)];
    }
    if key.contains("lineage") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::Lineage)];
    }
    if key.contains("library") || key.contains("editor") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::Library)];
    }
    if key.contains("achievement") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::Achievements)];
    }
    if key.contains("tool") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::Tools)];
    }
    if key.contains("credit") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::Credits)];
    }
    if key.contains("close") || key.contains("back") {
        return vec![ui::UiAction::ClosePanel];
    }
    if key.contains("kill") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::KillSelected)];
    }
    if key.contains("revive") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::ReviveSelected)];
    }
    if key.contains("lay") || key.contains("egg") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::LayEggs)];
    }
    if key.contains("edit") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::EditSelected)];
    }
    if key.contains("save") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::SaveSelected)];
    }
    if key.contains("load") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::OpenLoadPanel)];
    }
    if key.contains("open folder") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::OpenFolder)];
    }
    if key.contains("scroll view") || key.contains("scrollview") {
        return vec![ui::UiAction::Invoke(ui::ControlCommand::ScrollList)];
    }
    if key.contains("info") || key.contains("help") {
        return vec![ui::UiAction::OpenPanel(ui::PanelKind::Help)];
    }
    // Hard functional requirement: never leave an active control as a silent
    // no-op. Unmapped controls surface an explicit unsupported status.
    vec![ui::UiAction::Invoke(ui::ControlCommand::UnsupportedControl)]
}

/// The startup log line about the worker threads of the tick (`logs/last_run.log` of a tester says whether the pool runs).
fn threads_line(world: &bibites_core::world::World) -> String {
    let threads = world.threads();
    if threads <= 1 {
        "threads: 1 (the serial tick; --threads 0 uses every core)".to_owned()
    } else {
        let gates = world.parallel_gates();
        let how = if gates == bibites_core::sim::ParallelGates::SCALED {
            "the world times its own ticks, inline and on them at several levels of eagerness, and keeps the fastest (a small world ticks on one thread)"
        } else if gates == bibites_core::sim::ParallelGates::FIXED {
            "the pool from a fixed world size, no measuring (RUSTIBITES_GATES=fixed)"
        } else if gates == bibites_core::sim::ParallelGates::ALWAYS {
            "every stage of the tick runs on them, whatever the size of the world (RUSTIBITES_GATES=always)"
        } else {
            "the stages' chunk sizes are those of RUSTIBITES_GATES"
        };
        format!("threads: {threads} workers, {how}")
    }
}

/// The log line for a change of the way a world's ticks run (the world times them every way and keeps the fastest).
fn pool_switch_line(switch: &bibites_core::pool_tuner::PoolSwitch) -> String {
    format!(
        "pool: ticks now run {} (it was {}; tick {}, world size {}: {:.2} ms a tick now, {:.2} ms before)",
        switch.to.label(),
        switch.from.label(),
        switch.tick,
        switch.size,
        switch.to_ns / 1e6,
        switch.from_ns / 1e6
    )
}

/// Changes of the way the ticks run that a run logs at most (a world whose answer keeps changing must not fill the log).
const MAX_LOGGED_POOL_SWITCHES: u32 = 40;

/// The machine's calibration, once a frame (see `machine_calibration`): tells it whether a world is running in the window (a test would measure the world as much as
/// the machine), gives the simulation a new result (or the check box's new state) and, when the processor's part of the test has just finished, runs the test of the
/// graphics (a few frames' worth of drawing into a picture of its own, a second at most).
fn calibration_frame(
    app: &mut App,
    args: &Args,
    world_running: bool,
    textures: &[Texture2D],
    hue: Option<&render::GpuHueMaterial>,
    zone: Option<&render::GpuZoneMaterial>,
) {
    let calibrator = machine_calibration::calibrator();
    calibrator.set_world_running(world_running);
    if calibrator.take_changed() {
        machine_calibration::apply(calibrator);
        // (A world that was given a number of threads by hand keeps it; a world that lives on the simulation thread has its own: it takes the profile when it is next made.)
        if app.sim.is_none() {
            app.world.apply_machine_profile(args.threads == 0);
        }
    }
    if calibrator.take_graphics_due() {
        if let Some(found) = machine_calibration::graphics_test(textures, hue, zone, app.assets.as_ref()) {
            eprintln!("calibration: graphics {found:?}");
            calibrator.set_graphics(found, &machine_calibration::default_root());
        }
    }
    // (`RUSTIBITES_CALIBRATE=tiny|quick|full`: the test runs by itself, its lines are printed and the program ends.)
    if machine_calibration::autorun_finished() {
        std::process::exit(0);
    }
}

/// Logs the changes of the way the world's ticks run since the last call (for the tester's `logs/last_run.log`: which way the machine
/// found faster, and at what size of world).
fn log_pool_switches(world: &mut bibites_core::world::World, logged: &mut u32) {
    for switch in world.take_pool_switches() {
        if *logged < MAX_LOGGED_POOL_SWITCHES {
            *logged += 1;
            eprintln!("{}", pool_switch_line(&switch));
            if *logged == MAX_LOGGED_POOL_SWITCHES {
                eprintln!("pool: {MAX_LOGGED_POOL_SWITCHES} changes logged, the rest are not");
            }
        }
    }
}

/// Starts the simulation thread (`--sim-thread`) when the window draws the game's runtime UI: the curated HUD that stands in
/// without it reads the world's content in every frame, which the thread mode leaves behind.
fn start_sim_thread_if_asked(app: &mut App, args: &Args, themed: Option<&ThemedUi>) {
    // The window's frames run their ticks beside the drawing (see `tick_overlap`).
    app.overlap_ticks = args.overlap_ticks;
    if let Some(speed) = args.speed {
        app.set_target_speed(speed);
    }
    if !args.sim_thread {
        return;
    }
    if themed.is_some_and(|themed| themed.runtime.is_some()) {
        app.enable_sim_thread();
    } else {
        eprintln!(
            "--sim-thread needs the game's runtime UI (assets/game); the world stays on the window's thread"
        );
    }
}

/// A dropdown that was clicked hangs its list under the control the pointer is on: remembers that
/// control's rectangle (reference px) for [`App::open_dropdown_list`].
fn remember_click_rect(app: &mut App, frame: &ui::UiFrame, cursor: Vec2, actions: &[ui::UiAction]) {
    if !actions
        .iter()
        .any(|action| matches!(action, ui::UiAction::DropdownOpen(_)))
    {
        return;
    }
    if let Some(id) = frame.widget_at(cursor)
        && let Some((_, pos, size)) = frame.hits.iter().find(|hit| hit.0 == id)
    {
        let scale = frame.scale.max(f32::EPSILON);
        app.click_rect = [pos.x / scale, pos.y / scale, size.x / scale, size.y / scale];
    }
}

/// Routes a frame's pointer press/release through the themed HUD before
/// simulation controls (camera drag/zoom/selection). Returns the input with
/// mouse fields suppressed when the UI captured the pointer.
fn route_ui_frame(
    app: &mut App,
    frame: &ui::UiFrame,
    mut input: FrameInput,
    runtime: Option<&ui_runtime::RuntimeUi>,
    width: f32,
    height: f32,
) -> FrameInput {
    let over_ui = frame.widget_at(input.cursor).is_some();
    app.lineage_raw = app.panels.is_open(ui::PanelKind::Lineage).then_some(input);
    // The Bibite Editor takes the wheel (it scrolls the genes list, zooms the brain canvas) and keeps the camera from zooming.
    if app.bibite_editor.is_some() && input.wheel != 0.0 && app.dropdown_open.is_none() {
        app.editor_wheel(input.cursor, input.wheel);
        input.wheel = 0.0;
    }
    // The Save Game panel takes the wheel (it scrolls its list) and keeps the camera from zooming; its scrollbar follows the pointer.
    if app.panels.is_open(ui::PanelKind::SaveGame) {
        // A press on a row of the list waits for its release: a drag moves the list (with inertia), a click selects the row.
        let scale = ui::UiFrame::scaler(app.camera.screen.x, app.camera.screen.y);
        if input.primary_pressed
            && let Some(ui::UiAction::SavePanel(action @ save_panel::SaveAction::Row(_))) = frame.action_at(input.cursor)
        {
            let at = input.cursor / scale;
            let list = save_panel::layout(app.camera.screen.x / scale).view;
            if at.x >= list[0] && at.x < list[0] + list[2] && at.y >= list[1] && at.y < list[1] + list[3] {
                app.save_panel_hold_press(input.cursor, action);
                input.primary_pressed = false;
            }
        }
        if let Some(action) = app.save_panel_pointer(input.cursor, input.primary_pressed, input.primary_down, input.dt) {
            app.apply_ui_actions(&[ui::UiAction::SavePanel(action)]);
        }
        if input.wheel != 0.0 {
            app.save_panel_wheel(input.cursor, input.wheel);
            input.wheel = 0.0;
        }
    }
    // The Expanded Brain View takes the wheel (it zooms its canvas) and the right button (it pans it).
    if app.expanded_brain.is_some() && input.wheel != 0.0 && app.dropdown_open.is_none() {
        app.expanded_brain_wheel(input.cursor, input.wheel);
        input.wheel = 0.0;
    }
    let expanded_secondary = app.expanded_brain.is_some() && input.secondary_down;
    if app.expanded_brain.is_some() {
        input.secondary_down = false;
    }
    // The right button belongs to the editor (the Brain tab pans its canvas with it): the camera behind it does not pan.
    let editor_secondary = app.bibite_editor.is_some() && input.secondary_down;
    if app.bibite_editor.is_some() {
        input.secondary_down = false;
    }
    // A custom dropdown's list (the Graphs panel's) closes when the pointer leaves it.
    app.close_dropdown_on_leave(input.cursor, Vec2::new(width, height), frame.scale);
    // An open dropdown list takes the wheel (it scrolls); its blocker takes the clicks.
    if let Some(open) = app.dropdown_open.as_mut()
        && input.wheel != 0.0
    {
        open.wheel(input.wheel);
        input.wheel = 0.0;
    }
    // Tooltips: the pointer resting on a control with a `TooltipTrigger` or on a pellet.
    app.update_tooltip(frame, input.cursor, over_ui, input.dt);
    app.update_bar_hover(Vec2::new(width, height), input.dt);
    persist_view_toggles(app);
    // The bottom-left toolbar panels open and close with the pointer (`OnHoverShow`).
    if let Some(runtime) = runtime {
        // A hidden UI (H) has no buttons to rest on.
        let zones = if app.show_hud {
            runtime.toolbar_zones(app, width, height)
        } else {
            [None; 3]
        };
        app.update_toolbar_hover(&zones, input.cursor, input.dt);
    }
    // Time-speed slider: a held slider follows the cursor.
    if input.primary_pressed
        && let Some(ui::UiAction::SpeedSlide(x0, x1)) = frame.action_at(input.cursor)
    {
        app.speed_drag = Some((x0, x1));
        // A press on the handle keeps the handle where the pointer found it; the speed moves only when the pointer does.
        app.begin_slider_hold(app.speed_slider_grab((f32::from(x0), f32::from(x1)), input.cursor.x), input.cursor.x);
    }
    if let Some((x0, x1)) = app.speed_drag {
        if input.primary_down || input.primary_released {
            app.speed_slide((f32::from(x0), f32::from(x1)), input.cursor.x);
        }
        if input.primary_released || !input.primary_down {
            app.speed_drag = None;
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // The selected tower's panel: a held slider follows the cursor.
    if input.primary_pressed
        && let Some(ui::UiAction::ToolSlide(field, x0, x1)) = frame.action_at(input.cursor)
    {
        app.tool_drag = Some((field, x0, x1));
    }
    if let Some((field, x0, x1)) = app.tool_drag {
        if input.primary_down || input.primary_released {
            app.tool_slide(field, (f32::from(x0), f32::from(x1)), input.cursor.x);
        }
        if input.primary_released || !input.primary_down {
            app.tool_drag = None;
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // A neuron of the species Brain tab follows the cursor while it is held (`UINeuron.OnDrag`).
    if input.primary_pressed
        && let Some(ui::UiAction::BrainNode(node)) = frame.action_at(input.cursor)
    {
        app.brain_press_node(usize::from(node), input.cursor);
    }
    if app.brain_drag.is_some() {
        if input.primary_down || input.primary_released {
            app.brain_drag_to(input.cursor, input.shift_down);
        }
        if input.primary_released || !input.primary_down {
            app.brain_drag = None;
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // The same for a neuron of the inspector's Brain tab.
    if input.primary_pressed
        && let Some(ui::UiAction::InspectorBrainNode(node)) = frame.action_at(input.cursor)
    {
        app.inspector_brain_press_node(usize::from(node), input.cursor);
    }
    if app.inspector_brain_drag.is_some() {
        if input.primary_down || input.primary_released {
            app.inspector_brain_drag_to(input.cursor, input.shift_down);
        }
        if input.primary_released || !input.primary_down {
            app.inspector_brain_drag = None;
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // The species Position tab's time slider follows the cursor the same way.
    if input.primary_pressed
        && let Some(
            action @ (ui::UiAction::PositionSlide(x0, x1)
            | ui::UiAction::PositionSlideHandle(x0, x1)),
        ) = frame.action_at(input.cursor)
    {
        let on_handle = matches!(action, ui::UiAction::PositionSlideHandle(..));
        app.position_press(on_handle, (f32::from(x0), f32::from(x1)), input.cursor.x);
    }
    if app.position_drag.is_some() {
        if input.primary_down || input.primary_released {
            app.position_slide(input.cursor.x);
        }
        if input.primary_released || !input.primary_down {
            app.position_drag = None;
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // The Gene Analysis page's time slider follows the cursor the same way.
    if input.primary_pressed
        && let Some(
            action @ (ui::UiAction::GenesTimeSlide(x0, x1)
            | ui::UiAction::GenesTimeSlideHandle(x0, x1)),
        ) = frame.action_at(input.cursor)
    {
        let on_handle = matches!(action, ui::UiAction::GenesTimeSlideHandle(..));
        app.graphs_gene_press(on_handle, (f32::from(x0), f32::from(x1)), input.cursor.x);
    }
    if app.graphs_gene_drag.is_some() {
        if input.primary_down || input.primary_released {
            app.graphs_gene_slide(input.cursor.x);
        }
        if input.primary_released || !input.primary_down {
            app.graphs_gene_drag = None;
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // The Expanded Brain View's canvas, neurons and sliders follow the pointer while it is held.
    if app.expanded_brain.is_some()
        && app.route_expanded_pointer(frame, &mut input, expanded_secondary)
    {
        return input;
    }
    // The Brain tab's canvas, scroll bars and sliders follow the pointer while it is held.
    if app.bibite_editor.is_some()
        && app.dropdown_open.is_none()
        && app.route_brain_pointer(frame, &mut input, editor_secondary)
    {
        return input;
    }
    // The Bibite Editor's gene sliders follow the cursor the same way.
    if input.primary_pressed
        && let Some(ui::UiAction::Editor(template_editor_ui::EditorAction::GeneSlide(gene, x0, x1))) =
            frame.action_at(input.cursor)
    {
        app.editor_gene_press(gene, (f32::from(x0), f32::from(x1)), input.cursor.x);
    }
    if app.editor_gene_dragging() {
        if input.primary_down || input.primary_released {
            app.editor_gene_slide(input.cursor.x);
        }
        if input.primary_released || !input.primary_down {
            app.editor_gene_release();
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // The pellet placer's sliders follow the cursor the same way.
    if input.primary_pressed
        && let Some(ui::UiAction::PelletSlide(field, x0, x1)) = frame.action_at(input.cursor)
    {
        app.pellet_drag = Some((field, x0, x1));
    }
    if let Some((field, x0, x1)) = app.pellet_drag {
        if input.primary_down || input.primary_released {
            app.pellet_slide(field, (f32::from(x0), f32::from(x1)), input.cursor.x);
        }
        if input.primary_released || !input.primary_down {
            app.pellet_drag = None;
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // The Information panel's tags list: a held scroll bar handle follows the cursor (`Scrollbar.OnDrag`); a press on
    // the track pages towards it.
    if input.primary_pressed
        && let Some(ui::UiAction::InformationScroll(handle)) = frame.action_at(input.cursor)
    {
        app.information.press_bar(handle, input.cursor);
    }
    if app.information.bar_dragging() {
        if input.primary_down {
            app.information.drag_bar(input.cursor);
        }
        if input.primary_released || !input.primary_down {
            app.information.release_bar();
        }
        input.primary_pressed = false;
        input.primary_released = false;
        input.primary_down = false;
        input.wheel = 0.0;
        return input;
    }
    // The Lineage panel's Settings panel: its sliders are the dynamic panel's rows and follow the cursor the same way.
    if app.panels.is_open(ui::PanelKind::Lineage) && app.lineage_settings_open {
        if input.primary_pressed
            && let Some(ui::UiAction::DynSlide(index, x0, x1)) = frame.action_at(input.cursor)
        {
            app.dyn_panel.drag = Some((index, x0, x1));
            // A press on the handle keeps the handle where the pointer found it; the setting moves only when the pointer does.
            app.begin_slider_hold(app.dyn_slider_grab(usize::from(index), (f32::from(x0), f32::from(x1)), input.cursor.x), input.cursor.x);
        }
        if let Some((index, x0, x1)) = app.dyn_panel.drag {
            if input.primary_down || input.primary_released {
                app.dyn_slide(
                    usize::from(index),
                    (f32::from(x0), f32::from(x1)),
                    input.cursor.x,
                );
            }
            if input.primary_released || !input.primary_down {
                app.dyn_panel.drag = None;
            }
            input.primary_pressed = false;
            input.primary_released = false;
            input.primary_down = false;
            input.wheel = 0.0;
            return input;
        }
    }
    // Dynamic settings panel: a held slider follows the cursor; the wheel scrolls the rows.
    if app.panels.is_open(ui::PanelKind::WorldDynamics) {
        if input.primary_pressed
            && let Some(ui::UiAction::DynSlide(index, x0, x1)) = frame.action_at(input.cursor)
        {
            app.dyn_panel.drag = Some((index, x0, x1));
            // A press on the handle keeps the handle where the pointer found it; the setting moves only when the pointer does.
            app.begin_slider_hold(app.dyn_slider_grab(usize::from(index), (f32::from(x0), f32::from(x1)), input.cursor.x), input.cursor.x);
        }
        if let Some((index, x0, x1)) = app.dyn_panel.drag {
            if input.primary_down || input.primary_released {
                app.dyn_slide(
                    usize::from(index),
                    (f32::from(x0), f32::from(x1)),
                    input.cursor.x,
                );
            }
            if input.primary_released || !input.primary_down {
                app.dyn_panel.drag = None;
            }
            input.primary_pressed = false;
            input.primary_released = false;
            input.primary_down = false;
            input.wheel = 0.0;
            return input;
        }
        // A default bibite's slider follows the cursor the same way.
        if input.primary_pressed
            && let Some(ui::UiAction::BibiteSlide(index, field, x0, x1)) =
                frame.action_at(input.cursor)
        {
            app.dyn_panel.bibite_drag = Some((index, field, x0, x1));
        }
        if let Some((index, field, x0, x1)) = app.dyn_panel.bibite_drag {
            if input.primary_down || input.primary_released {
                app.bibite_slide(
                    usize::from(index),
                    field,
                    (f32::from(x0), f32::from(x1)),
                    input.cursor.x,
                );
            }
            if input.primary_released || !input.primary_down {
                app.dyn_panel.bibite_drag = None;
            }
            input.primary_pressed = false;
            input.primary_released = false;
            input.primary_down = false;
            input.wheel = 0.0;
            return input;
        }
        // The graph of a settings animator: a press picks a point (with Ctrl adds or removes one) and drags it.
        if input.primary_pressed
            && let Some(ui::UiAction::AnimGraph(changer, x0, y0, x1, y1)) = frame.action_at(input.cursor)
        {
            app.animator_graph_press(
                usize::from(changer),
                [f32::from(x0), f32::from(y0), f32::from(x1), f32::from(y1)],
                input.cursor,
                input.ctrl_down,
            );
        }
        if app.animator.drag.is_some() {
            if input.primary_down || input.primary_released {
                app.animator_graph_drag(input.cursor);
            }
            if input.primary_released || !input.primary_down {
                app.animator_graph_release();
            }
            input.primary_pressed = false;
            input.primary_released = false;
            input.primary_down = false;
            input.wheel = 0.0;
            return input;
        }
        // So does a slider of a zone's item on the Zones tab.
        if input.primary_pressed
            && let Some(ui::UiAction::ZoneSlide(index, field, x0, x1)) =
                frame.action_at(input.cursor)
        {
            app.dyn_panel.zone_drag = Some((index, field, x0, x1));
        }
        if let Some((index, field, x0, x1)) = app.dyn_panel.zone_drag {
            if input.primary_down || input.primary_released {
                app.zone_slide(
                    usize::from(index),
                    field,
                    (f32::from(x0), f32::from(x1)),
                    input.cursor.x,
                );
            }
            if input.primary_released || !input.primary_down {
                app.dyn_panel.zone_drag = None;
            }
            input.primary_pressed = false;
            input.primary_released = false;
            input.primary_down = false;
            input.wheel = 0.0;
            return input;
        }
        // And a slider of a material block on the Energy tab.
        if input.primary_pressed
            && let Some(ui::UiAction::MaterialSlide(kind, field, x0, x1)) =
                frame.action_at(input.cursor)
        {
            app.dyn_panel.material_drag = Some((kind, field, x0, x1));
        }
        if let Some((kind, field, x0, x1)) = app.dyn_panel.material_drag {
            if input.primary_down || input.primary_released {
                app.material_slide(kind, field, (f32::from(x0), f32::from(x1)), input.cursor.x);
            }
            if input.primary_released || !input.primary_down {
                app.dyn_panel.material_drag = None;
            }
            input.primary_pressed = false;
            input.primary_released = false;
            input.primary_down = false;
            input.wheel = 0.0;
            return input;
        }
        if input.wheel != 0.0 && over_ui {
            app.dyn_panel.scroll = (app.dyn_panel.scroll - input.wheel * 120.0)
                .clamp(0.0, app.dyn_panel.max_scroll.get());
        }
    }
    // The Information panel's tags list scrolls with the wheel (`ScrollRect`); the camera does not zoom then.
    // (Not while the HUD is hidden, H: nothing of the panel is drawn then, and the rectangle of its list is the one it had when it was.)
    if input.wheel != 0.0
        && app.show_hud
        && app.panels.is_open(ui::PanelKind::Information)
        && !app.quick_stats_hidden()
        && app.information.wheel(input.cursor, input.wheel)
    {
        input.wheel = 0.0;
    }
    let captured = app.interaction.active.is_some();
    let mut actions = frame.interact(
        input.cursor,
        input.primary_pressed,
        input.primary_released,
        &mut app.interaction,
    );
    // Resolve generic runtime-control activations to concrete actions.
    if let Some(runtime) = runtime {
        let controls = runtime.controls(width, height);
        let mut resolved = Vec::with_capacity(actions.len());
        for action in actions.drain(..) {
            if let ui::UiAction::ActivateControl(id) = action {
                let Some(control) = controls.iter().find(|control| control.id == id) else {
                    continue;
                };
                if let Some(mapped) = control.action {
                    resolved.push(mapped);
                } else if control.kind == ui_runtime::ControlKind::Dropdown
                    && !control.option_labels().is_empty()
                {
                    app.open_dropdown(control, frame.scale);
                } else if let Some(proposed) = control.proposed_value(input.cursor) {
                    if let ui::UiAction::SetValue(value) = proposed {
                        app.apply_control_value(control, value);
                    } else {
                        resolved.push(proposed);
                    }
                } else {
                    resolved.extend(dispatch_runtime_control(control));
                }
            } else {
                resolved.push(action);
            }
        }
        actions = resolved;
    }
    remember_click_rect(app, frame, input.cursor, &actions);
    // `Input.GetKey(LeftControl)` at the click (a species or tag row repeats its pick with it).
    app.ctrl_held = input.ctrl_down;
    app.shift_held = input.shift_down;
    app.apply_ui_actions(&actions);
    if over_ui || captured || app.interaction.active.is_some() {
        // A press that began on the world keeps seeing the button until it is released, even when
        // the cursor ends over a panel (`UserControl` polls `GetMouseButtonUp` globally).
        if app.drag.is_none() {
            input.primary_pressed = false;
            input.primary_released = false;
            input.primary_down = false;
        }
        if over_ui {
            // Only `MouseScroll` asks `IsPointerOverGameObject`: the right button still drags the
            // camera (and deselects) over a panel.
            input.wheel = 0.0;
        }
    }
    input
}

/// Reads one frame of input from macroquad into the app's backend-agnostic
/// [`FrameInput`].
fn gather_input() -> FrameInput {
    let (mouse_x, mouse_y) = mouse_position();
    // `GUIManager.Update`: the only digit hotkeys are 1-4 (the inspector panels).
    let mut digit = None;
    for (index, key) in [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4]
        .into_iter()
        .enumerate()
    {
        if is_key_pressed(key) {
            digit = Some(index as u32 + 1);
        }
    }
    FrameInput {
        cursor: Vec2::new(mouse_x, mouse_y),
        primary_down: is_mouse_button_down(MouseButton::Left),
        primary_pressed: is_mouse_button_pressed(MouseButton::Left),
        primary_released: is_mouse_button_released(MouseButton::Left),
        wheel: wheel_notches(),
        digit1_down: is_key_down(KeyCode::Key1),
        toggle_pause: is_key_pressed(KeyCode::Space),
        // The arrow keys move the camera (`CameraManager.ArrowsMove`). The game has no key for
        // stepping, fitting the view or toggling layers (the Visibility panel has those), so
        // none is bound; `UserControl.Update` has R random bibite, G highest generation,
        // O oldest, E random egg.
        step: false,
        fit: false,
        fit_bibites: false,
        reset_view: false,
        digit,
        toggle_grid: false,
        toggle_zones: false,
        toggle_pellets: false,
        toggle_pheromones: false,
        toggle_hud: is_key_pressed(KeyCode::H),
        quit: is_key_pressed(KeyCode::Escape),
        select_random_bibite: is_key_pressed(KeyCode::R),
        select_high_generation: is_key_pressed(KeyCode::G),
        select_oldest: is_key_pressed(KeyCode::O),
        select_random_egg: is_key_pressed(KeyCode::E),
        // `Input.GetKeyDown(KeyCode.Z)` has no modifier check: Ctrl + Z also picks a zone.
        select_random_zone: is_key_pressed(KeyCode::Z),
        // `UserControl.Update`: K makes the auto-panner the target.
        select_autopanner: is_key_pressed(KeyCode::K),
        quick_save: is_key_pressed(KeyCode::F5),
        // `BrainPanelCommon.UpdatePanel`: F1 sorts the brain diagram once, F3 six times, F4 eight times, F2 held
        // steps the hidden nodes' physics (with the right Control key held at a scalar of 0).
        brain_sort_keys: [
            is_key_pressed(KeyCode::F1),
            is_key_pressed(KeyCode::F3),
            is_key_pressed(KeyCode::F4),
        ],
        brain_hidden_down: is_key_down(KeyCode::F2),
        right_ctrl_down: is_key_down(KeyCode::RightControl),
        lay_egg: is_key_pressed(KeyCode::Insert),
        kill_target: is_key_pressed(KeyCode::Delete),
        camera_home: is_key_pressed(KeyCode::Home),
        secondary_down: is_mouse_button_down(MouseButton::Right),
        // `Input.GetKey(KeyCode.LeftControl)` (306): the right control key does nothing here.
        ctrl_down: is_key_down(KeyCode::LeftControl),
        shift_down: is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift),
        // `UINavigationManager.Update`: Ctrl + Z reverts the last setting change, with Shift redoes.
        undo: is_key_down(KeyCode::LeftControl)
            && !is_key_down(KeyCode::LeftShift)
            && is_key_pressed(KeyCode::Z),
        redo: is_key_down(KeyCode::LeftControl)
            && is_key_down(KeyCode::LeftShift)
            && is_key_pressed(KeyCode::Z),
        // `GUIManager.Update`: Ctrl + C copies the target's species tag, Ctrl + V tags with it.
        copy_tag: is_key_down(KeyCode::LeftControl) && is_key_pressed(KeyCode::C),
        paste_tag: is_key_down(KeyCode::LeftControl) && is_key_pressed(KeyCode::V),
        backspace: is_key_pressed(KeyCode::Backspace),
        key_left: is_key_pressed(KeyCode::Left),
        key_right: is_key_pressed(KeyCode::Right),
        key_home: is_key_pressed(KeyCode::Home),
        key_end: is_key_pressed(KeyCode::End),
        key_delete: is_key_pressed(KeyCode::Delete),
        select_all_key: is_key_down(KeyCode::LeftControl) && is_key_pressed(KeyCode::A),
        enter: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter),
        middle_down: is_mouse_button_down(MouseButton::Middle),
        middle_pressed: is_mouse_button_pressed(MouseButton::Middle),
        move_axis: {
            let axis = |plus: &[KeyCode], minus: &[KeyCode]| {
                let down = |keys: &[KeyCode]| keys.iter().any(|key| is_key_down(*key));
                f32::from(u8::from(down(plus))) - f32::from(u8::from(down(minus)))
            };
            Vec2::new(
                axis(&[KeyCode::Right, KeyCode::D], &[KeyCode::Left, KeyCode::A]),
                axis(&[KeyCode::Up, KeyCode::W], &[KeyCode::Down, KeyCode::S]),
            )
        },
        // `CameraManager.Update`: PageUp / PageDown / keypad minus rescale the camera speed.
        speed_scale_step: if is_key_pressed(KeyCode::PageUp) {
            1
        } else if is_key_pressed(KeyCode::PageDown) || is_key_pressed(KeyCode::KpSubtract) {
            -1
        } else {
            0
        },
        dt: get_frame_time(),
        zoom_key: if is_key_pressed(KeyCode::Equal) || is_key_pressed(KeyCode::KpAdd) {
            0.25
        } else if is_key_pressed(KeyCode::Minus) || is_key_pressed(KeyCode::KpSubtract) {
            -0.2
        } else {
            0.0
        },
    }
}

/// Draws the HUD overlay: world/playback information, species legend, controls
/// help and the selected-bibite panel.
fn draw_overlay(app: &App) {
    const TEXT: Color = Color::new(0.90, 0.93, 0.96, 1.0);
    const DIM: Color = Color::new(0.62, 0.68, 0.75, 1.0);
    const ACCENT: Color = Color::new(1.0, 0.72, 0.32, 1.0);
    let panel = Color::new(0.02, 0.03, 0.05, 0.78);
    let screen_w = screen_width();
    let screen_h = screen_height();

    if app.show_hud {
        // Top-left: world + playback HUD.
        draw_rectangle(12.0, 12.0, 740.0, 152.0, panel);
        draw_text(
            format!("RUSTibites viewer - {}", app.label),
            24.0,
            38.0,
            22.0,
            TEXT,
        );
        if app.selection.is_some() {
            draw_text("click empty space to deselect", 24.0, 60.0, 16.0, DIM);
        } else {
            draw_text("click a bibite to inspect it", 24.0, 60.0, 16.0, DIM);
        }
        let state = if app.paused { "PAUSED" } else { "running" };
        draw_text(
            format!(
                "sim {}   tick {}   {:.0} TPS (sim/real x{:.2})   {}x speed   {}",
                format_clock(app.simulated_time()),
                app.world_tick(),
                app.time_keeper.tps(),
                app.time_keeper.sim_seconds_per_second(),
                app.speed,
                state,
            ),
            24.0,
            84.0,
            18.0,
            TEXT,
        );
        let hud = &app.hud;
        draw_text(
            format!(
                "pop {}  (+{} corpses)   eggs {}   species {}   gen mean {:.1}   age mean {:.2} h   seed {}",
                hud.population,
                hud.corpses,
                hud.eggs,
                hud.species,
                hud.generation_mean,
                hud.age_mean_hours,
                app.seed,
            ),
            24.0,
            108.0,
            18.0,
            TEXT,
        );
        draw_text(
            format!(
                "plants {} ({:.0} E)   meat {} ({:.0} E)   fps {}   zoom {:.2} px/u",
                hud.plant_pellets,
                hud.plant_biomass,
                hud.meat_pellets,
                hud.meat_biomass,
                app.fps,
                app.camera.zoom,
            ),
            24.0,
            132.0,
            18.0,
            TEXT,
        );
        draw_text(
            format!(
                "view {}   grid {}   zones {}   pellets {}   phero {}   threads {}",
                view_mode_label(app.view_mode),
                on_off(app.scene_options.grid),
                on_off(app.scene_options.zones),
                on_off(app.scene_options.pellets),
                on_off(app.scene_options.pheromones),
                app.world.threads(),
            ),
            24.0,
            156.0,
            15.0,
            DIM,
        );

        draw_species_legend(app);

        // Bottom: two-line controls help.
        draw_rectangle(
            12.0,
            screen_h - 58.0,
            (screen_w - 24.0).max(10.0),
            46.0,
            panel,
        );
        draw_text(
            "drag pan | wheel zoom | space pause | . step | 1-8 speed | \
             click select | Esc deselect/close",
            24.0,
            screen_h - 38.0,
            16.0,
            DIM,
        );
        draw_text(
            "F fit world | B fit bibites | R reset | G grid | Z zones | P pellets | \
             O pheromones | H hud | F12 screenshot",
            24.0,
            screen_h - 18.0,
            16.0,
            DIM,
        );

        if app.paused {
            let label = "PAUSED";
            let size = 34.0;
            let dimensions = measure_text(label, None, size as u16, 1.0);
            draw_text(
                label,
                (screen_w - dimensions.width) * 0.5,
                54.0,
                size,
                ACCENT,
            );
        }

        draw_selection_panel(app);
    } else {
        draw_text("HUD hidden - press H to show", 24.0, 34.0, 18.0, DIM);
        if app.paused {
            draw_text("PAUSED", 24.0, 62.0, 22.0, ACCENT);
        }
        draw_selection_panel(app);
    }
}

/// Species legend: a colour chip + name + live count per active species.
fn draw_species_legend(app: &App) {
    const TEXT: Color = Color::new(0.90, 0.93, 0.96, 1.0);
    const DIM: Color = Color::new(0.62, 0.68, 0.75, 1.0);
    const PANEL: Color = Color::new(0.02, 0.03, 0.05, 0.75);

    let legend = &app.hud.species_legend;
    if legend.is_empty() {
        return;
    }
    let row_height = 20.0;
    let height = 34.0 + legend.len() as f32 * row_height;
    draw_rectangle(12.0, 176.0, 380.0, height, PANEL);
    draw_text("species legend", 24.0, 198.0, 16.0, DIM);
    for (index, entry) in legend.iter().enumerate() {
        let baseline = 198.0 + row_height * (index as f32 + 1.0);
        draw_rectangle(
            24.0,
            baseline - 12.0,
            12.0,
            12.0,
            to_color([
                f32::from(entry.color[0]) / 255.0,
                f32::from(entry.color[1]) / 255.0,
                f32::from(entry.color[2]) / 255.0,
                1.0,
            ]),
        );
        draw_text(
            format!("#{} {}", entry.id, entry.name),
            44.0,
            baseline,
            15.0,
            TEXT,
        );
        draw_text(format!("{}", entry.count), 344.0, baseline, 15.0, DIM);
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn view_mode_label(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Bibites => "bibites",
        ViewMode::World => "world",
        ViewMode::Manual => "manual",
    }
}

/// Draws the selected-bibite info panel (top-right).
fn draw_selection_panel(app: &App) {
    const TEXT: Color = Color::new(0.90, 0.93, 0.96, 1.0);
    const DIM: Color = Color::new(0.62, 0.68, 0.75, 1.0);
    const PANEL: Color = Color::new(0.02, 0.03, 0.05, 0.82);

    let width = 360.0;
    let x = screen_width() - width - 12.0;
    let y = 12.0;
    let Some(panel) = &app.selection else {
        draw_rectangle(x, y, width, 48.0, PANEL);
        draw_text("no selection", x + 14.0, y + 30.0, 17.0, DIM);
        return;
    };

    let species = match panel.species_id {
        Some(id) => format!("#{id} {}", panel.species_name),
        None => panel.species_name.clone(),
    };
    let color = to_color([panel.color[0], panel.color[1], panel.color[2], 1.0]);
    let outputs = &panel.brain_outputs;
    let rows: [(&str, String); 15] = [
        ("id", format!("{}", panel.target)),
        ("species", species),
        ("generation", format!("{}", panel.generation)),
        (
            "age",
            format!(
                "{:.0} min ({:.2} h)",
                panel.age_seconds / 60.0,
                panel.age_seconds / 3600.0
            ),
        ),
        (
            "energy",
            format!(
                "{:.0} / {:.0} E ({:.0}%)",
                panel.energy,
                panel.max_energy,
                ratio(panel.energy, panel.max_energy) * 100.0,
            ),
        ),
        (
            "health",
            format!(
                "{:.0} / {:.0} HP ({:.0}%)",
                panel.health,
                panel.max_health,
                ratio(panel.health, panel.max_health) * 100.0,
            ),
        ),
        (
            "stomach",
            format!("{:.1} / {:.1} u^2", panel.stomach, panel.stomach_capacity),
        ),
        (
            "fat",
            format!(
                "{:.1} / {:.1} u^2 (inflate {:.0}%)",
                panel.fat,
                panel.fat_max,
                panel.fat_inflate * 100.0,
            ),
        ),
        (
            "eggs",
            format!("{:.2} / {} ready", panel.egg_progress, panel.egg_capacity),
        ),
        ("diet", format!("{:.2} (0 herb / 1 carn)", panel.diet)),
        (
            "body",
            format!(
                "d1 {:.2} (gene {:.2}) | speed gene {:.2}",
                panel.size, panel.size_gene, panel.speed_gene
            ),
        ),
        (
            "brain",
            format!(
                "{} nodes | {} synapses",
                panel.brain_nodes, panel.brain_synapses
            ),
        ),
        (
            "outputs",
            format!(
                "accel {:+.2} | rot {:+.2}",
                outputs[BrainOutput::Accelerate.index()],
                outputs[BrainOutput::Rotate.index()],
            ),
        ),
        (
            "outputs",
            format!(
                "eat {:+.2} | bite {:+.2} | lay {:+.2}",
                outputs[BrainOutput::Want2Eat.index()],
                outputs[BrainOutput::Want2Attack.index()],
                outputs[BrainOutput::Want2Lay.index()],
            ),
        ),
        (
            "flags",
            match (panel.is_corpse, panel.is_mutant) {
                (true, _) => "corpse".to_owned(),
                (false, true) => "mutant outlier".to_owned(),
                (false, false) => "-".to_owned(),
            },
        ),
    ];
    let row_height = 21.0;
    let height = 30.0 + rows.len() as f32 * row_height;
    draw_rectangle(x, y, width, height, PANEL);
    let mut baseline = y + 30.0;
    draw_text("selected bibite", x + 14.0, baseline, 19.0, TEXT);
    for (label, value) in rows {
        baseline += row_height;
        draw_text(label, x + 14.0, baseline, 15.0, DIM);
        draw_text(&value, x + 108.0, baseline, 16.0, TEXT);
    }
    // Skin swatch.
    draw_rectangle(x + width - 38.0, y + 14.0, 24.0, 24.0, color);
}

/// `a / b`, or 0 when `b` is not positive.
fn ratio(a: f32, b: f32) -> f32 {
    if b > 0.0 {
        (a / b).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Converts four `0..=1` channels to a macroquad colour.
fn to_color([r, g, b, a]: [f32; 4]) -> Color {
    Color::new(
        r.clamp(0.0, 1.0),
        g.clamp(0.0, 1.0),
        b.clamp(0.0, 1.0),
        a.clamp(0.0, 1.0),
    )
}

/// Formats seconds as `HH:MM:SS.s` (simulation clock).
fn format_clock(seconds: f64) -> String {
    let total = seconds.max(0.0);
    let hours = (total / 3600.0).floor() as u64;
    let minutes = ((total - hours as f64 * 3600.0) / 60.0).floor() as u64;
    let residual = total - (hours * 3600 + minutes * 60) as f64;
    format!(
        "{hours:02}:{minutes:02}:{remaining:04.1}",
        remaining = residual
    )
}

// ---------------------------------------------------------------------------
// Screenshot mode (headless)
// ---------------------------------------------------------------------------

/// Simulates `--frames` frames and rasterises one frame to a PNG, without
/// creating a window or GPU context.
/// Representative click -> observable App-state matrix for every native session.
fn run_functional_matrix(args: &Args) -> Result<(), String> {
    let runtime = load_ui_runtime(args)?;
    let controls = runtime.controls(1920.0, 1080.0);
    let session_names = runtime.session_names();
    let candidates = [
        "continue",
        "new",
        "load",
        "settings",
        "graphic settings",
        "game settings",
        "hotkeys",
        "pause",
        "play",
        "step",
        "speed",
        "placer",
        "user actions",
        "view",
        "stats",
        "genes",
        "biology",
        "brain",
        "info",
        "close",
        "back",
        "save",
        "edit",
        "reset",
        "tower",
        "pellet",
        "kill",
    ];
    let mut report = serde_json::Map::new();
    let mut total_clicked = 0_usize;
    let mut total_effect = 0_usize;
    let mut no_effect: Vec<serde_json::Value> = Vec::new();
    for session in &session_names {
        let loaded = app::LoadedWorld::load(None, 1234, 4, 1)?;
        let mut app = app::App::new(loaded, Vec2::new(1280.0, 800.0), None);
        let mut clicked = 0_usize;
        let mut effects = 0_usize;
        let mut seen = std::collections::BTreeSet::new();
        for control in controls
            .iter()
            .filter(|control| control.session == *session)
        {
            let label = format!("{} {}", control.name, control.label).to_ascii_lowercase();
            if !candidates.iter().any(|candidate| label.contains(candidate)) {
                continue;
            }
            if !seen.insert(control.id) {
                continue;
            }
            let before = app_signature(&app);
            let actions = dispatch_runtime_control(control);
            app.apply_ui_actions(&actions);
            let after = app_signature(&app);
            clicked += 1;
            total_clicked += 1;
            if before != after {
                effects += 1;
                total_effect += 1;
            } else {
                no_effect.push(serde_json::json!({
                    "session": session,
                    "name": control.name,
                    "label": control.label,
                    "kind": format!("{:?}", control.kind),
                    "actions": format!("{actions:?}"),
                }));
            }
            if clicked >= 6 {
                break;
            }
        }
        report.insert(
            session.clone(),
            serde_json::json!({ "clicked": clicked, "effects": effects }),
        );
    }
    let document = serde_json::json!({
        "schema": "rustibites.viewer.functional_matrix.v1",
        "sessions": report,
        "clicked": total_clicked,
        "effects": total_effect,
        "no_effect": no_effect,
    });
    let path = args
        .ui_controls_out
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/ui_functional_matrix.json"));
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let md_path = path.with_extension("md");
    let mut md = String::from("# Native UI functional click matrix\n\n");
    md.push_str(&format!(
        "Clicked {total_clicked} representative controls; {total_effect} produced an observable app-state change; {} no-effect.\n\n",
        document["no_effect"].as_array().map_or(0, Vec::len)
    ));
    for (session, value) in report.iter() {
        md.push_str(&format!(
            "- `{session}`: {}/{} effective\n",
            value["effects"], value["clicked"]
        ));
    }
    std::fs::write(&md_path, md).map_err(|error| error.to_string())?;
    println!(
        "functional-matrix: {total_clicked} clicks, {total_effect} observable effects, {} no-effect -> {} (+md)",
        document["no_effect"].as_array().map_or(0, Vec::len),
        path.display()
    );
    Ok(())
}

/// Compact observable App state used by the functional click matrix.
fn app_signature(app: &app::App) -> String {
    format!(
        "paused={} speed={} selected={:?} ticks={} last={} hud={} panels={:?}",
        app.paused,
        app.speed,
        app.selected,
        app.sim_ticks,
        app.last_action,
        app.show_hud,
        app.panels,
    )
}

/// Audits every native control for a concrete dispatch (functional coverage).
fn run_functional_audit(args: &Args) -> Result<(), String> {
    let runtime = load_ui_runtime(args)?;
    let controls = runtime.controls(1920.0, 1080.0);
    let mut concrete = 0_usize;
    let mut keyword = 0_usize;
    let mut unsupported_count = 0_usize;
    let mut inert: Vec<serde_json::Value> = Vec::new();
    let mut by_kind: std::collections::BTreeMap<String, (usize, usize)> =
        std::collections::BTreeMap::new();
    for control in &controls {
        let entry = by_kind
            .entry(format!("{:?}", control.kind))
            .or_insert((0, 0));
        entry.0 += 1;
        let dispatched = dispatch_runtime_control(control);
        let unsupported =
            dispatched == vec![ui::UiAction::Invoke(ui::ControlCommand::UnsupportedControl)];
        if control.action.is_some() {
            concrete += 1;
        } else if !dispatched.is_empty() && !unsupported {
            keyword += 1;
            concrete += 1;
        } else {
            if unsupported {
                unsupported_count += 1;
            } else {
                inert.push(serde_json::json!({
                "session": control.session,
                "path": control.path,
                "name": control.name,
                "label": control.label,
                    "kind": format!("{:?}", control.kind),
                    "rect": control.rect,
                }));
            }
        }
    }
    let document = serde_json::json!({
        "schema": "rustibites.viewer.functional_audit.v1",
        "total": controls.len(),
        "concrete": concrete,
        "keyword": keyword,
        "inert": inert.len(),
        "unsupported": unsupported_count,
        "by_kind": by_kind.iter().map(|(kind, (total, missing))| serde_json::json!({
            "kind": kind, "total": total, "inert": missing,
        })).collect::<Vec<_>>(),
        "inert_controls": inert,
    });
    let path = args
        .ui_controls_out
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/ui_functional_audit.json"));
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!(
        "functional-audit: {} controls, {concrete} dispatchable ({} keyword), {unsupported_count} explicit-unsupported, {} inert -> {}",
        controls.len(),
        keyword,
        document["inert"],
        path.display()
    );
    Ok(())
}

/// Dumps native runtime controls as JSON for registry regeneration.
fn run_ui_controls(args: &Args) -> Result<(), String> {
    let runtime = load_ui_runtime(args)?;
    let names: Vec<String> = if args.ui_controls_sessions.is_empty() {
        runtime.session_names()
    } else {
        args.ui_controls_sessions.clone()
    };
    let mut sessions = serde_json::Map::new();
    let mut count = 0_usize;
    for name in &names {
        if runtime.session_by_name(name).is_none() {
            eprintln!("ui-controls: unknown session {name}");
            continue;
        }
        let mut records: Vec<serde_json::Value> = runtime
            .controls(1920.0, 1080.0)
            .into_iter()
            .filter(|control| control.session == *name)
            .map(|control| {
                count += 1;
                serde_json::json!({
                    "session": control.session,
                    "id": control.id,
                    "path": control.path,
                    "name": control.name,
                    "label": control.label,
                    "kind": format!("{:?}", control.kind),
                    "rect": control.rect,
                    "value": control.value.map(|value| format!("{value:?}")),
                    "action": control.action.map(|action| format!("{action:?}")),
                    "binding": control.binding,
                })
            })
            .collect();
        records.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
        sessions.insert(name.clone(), serde_json::Value::Array(records));
    }
    let document = serde_json::json!({
        "schema": "rustibites.viewer.controls.v1",
        "screen": { "width": 1920, "height": 1080 },
        "count": count,
        "sessions": sessions,
    });
    let json = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("control JSON encode failed: {error}"))?;
    let path = args
        .ui_controls_out
        .clone()
        .unwrap_or_else(|| PathBuf::from("target/viewer_controls_native.json"));
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, json)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    println!(
        "ui-controls: {count} controls from {} session(s) -> {}",
        names.len(),
        path.display()
    );
    Ok(())
}

/// Loads the runtime atlas requested on the command line (native or curated).
fn load_ui_runtime(args: &Args) -> Result<ui_runtime::RuntimeUi, String> {
    if let Some(path) = &args.ui_runtime {
        if path.as_os_str() == "auto" {
            let native = Path::new(NATIVE_ATLAS);
            if native.is_file() {
                return ui_runtime::RuntimeUi::load_native(native);
            }
            return ui_runtime::RuntimeUi::discover()?.ok_or(
                "runtime UI atlas not found (tools/ui/ui_runtime_atlas.json.gz)".to_owned(),
            );
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("ui_runtime_atlas"))
        {
            return ui_runtime::RuntimeUi::load(path);
        }
        return ui_runtime::RuntimeUi::load_native(path);
    }
    // Prefer the native 1920 recapture for session rendering when present.
    let native = Path::new(NATIVE_ATLAS);
    if native.is_file() {
        return ui_runtime::RuntimeUi::load_native(native);
    }
    ui_runtime::RuntimeUi::discover()?
        .ok_or("runtime UI atlas not found (tools/ui/ui_runtime_atlas.json.gz)".to_owned())
}

/// Native 1920 runtime recapture shipped by parity.
const NATIVE_ATLAS: &str = "assets/game/ui_runtime_native.json.gz";

/// Registers native sprite names from parity's sprite manifest as UI aliases.
fn load_native_sprite_manifest(ui: &mut ui::UiAssets) -> Result<usize, String> {
    let path = Path::new("assets/game/reference/functional_ui_native/sprite_manifest.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(0);
    };
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("invalid sprite manifest: {error}"))?;
    let Some(sprites) = value.get("sprites").and_then(serde_json::Value::as_object) else {
        return Ok(0);
    };
    let mut loaded = 0_usize;
    for (name, entry) in sprites {
        let Some(file) = entry.get("file").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if ui.add_texture_alias(name, Path::new(file)).is_ok() {
            loaded += 1;
        }
    }
    // The lineage tree's sprites (`tools/unity/export_named_sprites.py`), under their sprite names.
    if let Ok(read) = std::fs::read_dir("assets/game/reference/functional_ui_native/lineage") {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if ui.add_texture_alias(stem, &path).is_ok() {
                loaded += 1;
            }
        }
    }
    // Achievement icons in both states (the Achievements screen swaps them live).
    if let Ok(read) = std::fs::read_dir("assets/game/ui/sharedassets0.assets/sheets") {
        for entry in read.flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if (stem.starts_with("ACH_") || stem.starts_with("rockstar_placeholder"))
                && ui.add_texture_alias(stem, &path).is_ok()
            {
                loaded += 1;
            }
        }
    }
    // `tiled_bars_2` and `tiled_bars_3` (the ends of the rate sliders' fills) are the right and the left five columns of `tiled_bars_0`; the extracted
    // files of those two sprites are empty.
    if ui.add_cropped_alias("tiled_bars_2", "tiled_bars_0", [33, 0, 5, 12]) {
        loaded += 1;
    }
    if ui.add_cropped_alias("tiled_bars_3", "tiled_bars_0", [0, 0, 5, 12]) {
        loaded += 1;
    }
    // The species panel's favourite heart is drawn with `flatMat`: the sprite's alpha in the `Image`'s colour.
    if ui.add_flat_alias("output_want2heal#flat", "output_want2heal") {
        loaded += 1;
    }
    // The blue outline of a selected neuron (the Brain tab's highlighted differences): `nodeActivation` samples the sprite two texels away.
    if ui.add_outline_alias("small_circle_base_icon#selected", "small_circle_base_icon", 2) {
        loaded += 1;
    }
    // The other sizes of the pellet sprites (the Biology panel's stomach shows the pellet of the size its amount asks for).
    for name in ["tiny_plant", "tiny_meat", "large_plant", "large_meat"] {
        if ui.texture_of_alias(name).is_none()
            && ui
                .add_texture_alias(name, Path::new(&format!("assets/game/sprites/{name}.png")))
                .is_ok()
        {
            loaded += 1;
        }
    }
    Ok(loaded)
}

/// Prints every captured runtime UI session name.
fn list_ui_sessions(args: &Args) -> Result<(), String> {
    let runtime = load_ui_runtime(args)?;
    let mut names = runtime.session_names();
    names.sort();
    for name in names {
        println!("{name}");
    }
    Ok(())
}

/// Loads a PNG as an RGBA image (used for UI-session backgrounds).
fn load_ui_background(path: &Path) -> Result<render::Image, String> {
    load_reference(path)
}

/// Composites a transparent UI image over a background at 1920x1080.
fn compose_ui_over_background(
    ui_image: &render::Image,
    background: Option<&render::Image>,
) -> render::Image {
    let (width, height) = (ui_image.width, ui_image.height);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            let bg = if let Some(background) = background {
                let sx = (x * background.width / width).min(background.width - 1);
                let sy = (y * background.height / height).min(background.height - 1);
                let j = ((sy * background.width + sx) * 4) as usize;
                [
                    background.rgba[j],
                    background.rgba[j + 1],
                    background.rgba[j + 2],
                ]
            } else {
                [18, 20, 22]
            };
            let alpha = u32::from(ui_image.rgba[i + 3]);
            for (channel, background_channel) in bg.iter().enumerate() {
                let blended = (u32::from(ui_image.rgba[i + channel]) * alpha
                    + u32::from(*background_channel) * (255 - alpha))
                    / 255;
                rgba.push(blended as u8);
            }
            rgba.push(255);
        }
    }
    render::Image {
        width,
        height,
        rgba,
    }
}

/// Renders captured runtime UI sessions at 1920x1080 with the software path.
fn run_ui_sessions(args: &Args) -> Result<(), String> {
    let mut ui = ui::UiAssets::discover()?.ok_or("game UI assets not found (assets/game)")?;
    let runtime = load_ui_runtime(args)?;
    // The joined dump references sprite PNGs that are not in the curated atlas
    // (toolbar icons, play/pause, ruler, ...). Decode them before rendering.
    let extra = runtime.load_sprite_textures(&mut ui)?;
    if extra > 0 {
        eprintln!("ui-session: decoded {extra} runtime sprite texture(s)");
    }
    if runtime.native {
        match load_native_sprite_manifest(&mut ui) {
            Ok(loaded) if loaded > 0 => {
                eprintln!("ui-session: resolved {loaded} native sprite manifest entries");
            }
            Ok(_) => {}
            Err(error) => eprintln!("ui-session: sprite manifest: {error}"),
        }
    }
    // Native toolbar icon sprites (button_theme dump): these are missing from
    // the sharedassets extraction, so register them under their sprite names.
    let theme_sprites =
        Path::new("assets/game/reference/functional_ui_native/in_world/button_theme/sprites");
    let mut theme_loaded = 0_usize;
    if theme_sprites.is_dir()
        && let Ok(read) = std::fs::read_dir(theme_sprites)
    {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if ui.add_texture_alias(stem, &path).is_ok() {
                theme_loaded += 1;
            }
        }
    }
    if theme_loaded > 0 {
        eprintln!("ui-session: loaded {theme_loaded} native button-theme sprite(s)");
    }
    let mut fonts = ui::FontSet::load(&ui)?;
    let text_mode = match args.ui_text.as_deref() {
        Some("ttf") => ui::TextMode::Ttf,
        Some("sdf") => ui::TextMode::Sdf,
        _ => ui::TextMode::Auto,
    };
    fonts.set_text_mode(text_mode);
    let background = args
        .ui_background
        .as_deref()
        .map(load_ui_background)
        .transpose()?;
    let (width, height) = (1920_u32, 1080_u32);
    let names = runtime.session_names();
    let base_path = args
        .screenshot
        .clone()
        .unwrap_or_else(|| PathBuf::from("target").join("ui-session.png"));
    let mut rendered = 0_usize;
    for session in &args.ui_session {
        if !names.iter().any(|name| name == session) {
            eprintln!(
                "ui-session: unknown session {session} (run --ui-session-list for {} names)",
                names.len()
            );
            continue;
        }
        let frame = if session.starts_with("main_menu") {
            runtime.build_menu_like_session_frame(
                session,
                &ui,
                width as f32,
                height as f32,
                fonts.sdf(),
            )
        } else if let Some(metrics) = fonts.sdf() {
            runtime.build_named_session_frame_with_metrics(
                session,
                &ui,
                width as f32,
                height as f32,
                metrics,
            )
        } else {
            runtime.build_named_session_frame(session, &ui, width as f32, height as f32)
        };
        let Some(mut frame) = frame else {
            eprintln!("ui-session: could not build frame for {session}");
            continue;
        };
        if session.contains("world") || session.contains("inspector") {
            append_ruler_draws(&mut frame, width as f32, height as f32);
        }
        if args.ui_only {
            // Captured sessions include their full-screen canvas backdrop; a
            // mask comparison must contain only the UI draw list.
            frame
                .draws
                .retain(|draw| !is_full_screen_ui_draw(draw, width as f32, height as f32));
        }
        let overlay = render::UiOverlay {
            assets: &ui,
            frame: &frame,
            fonts: &mut fonts,
        };
        let ui_image = render::render_ui_only(width, height, 2, Some(overlay));
        let image = if args.ui_only {
            ui_image
        } else {
            compose_ui_over_background(&ui_image, background.as_ref())
        };
        let path = ui_session_path(&base_path, session, args.ui_session.len());
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        image
            .write_png(&path)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        println!(
            "ui-session: {session} -> {} ({}x{}, {})",
            path.display(),
            width,
            height,
            if args.ui_only {
                "transparent UI overlay"
            } else {
                "composed over background"
            }
        );
        rendered += 1;
    }
    if rendered == 0 {
        return Err("no UI sessions rendered".to_owned());
    }
    Ok(())
}

/// Derives the output path for one session render.
/// Whether a UI draw covers the whole screen (captured session backdrop).
fn is_full_screen_ui_draw(draw: &ui::UiDraw, width: f32, height: f32) -> bool {
    let size = match draw {
        ui::UiDraw::Rect { size, .. }
        | ui::UiDraw::Image { size, .. }
        | ui::UiDraw::ImageTurned { size, .. }
        | ui::UiDraw::ImageRegion { size, .. }
        | ui::UiDraw::Pixels { size, .. }
        | ui::UiDraw::PixelsRegion { size, .. }
        | ui::UiDraw::NineSlice { size, .. } => *size,
        ui::UiDraw::Text { .. } | ui::UiDraw::Clip(_) => return false,
    };
    size.x >= width * 0.95 && size.y >= height * 0.95
}

/// Synthesizes the game's procedural 100u ruler for standalone session frames.
///
/// The live HUD draws the bracket/ticks from the current camera zoom; captured
/// session frames have no world/app, so the native 1920 reference geometry is
/// used (bottom-right bracket + "100u" label).
fn append_ruler_draws(_frame: &mut ui::UiFrame, _width: f32, _height: f32) {
    let left = 1770.0;
    let right = 1904.0;
    let top = 1059.0;
    let bottom = 1064.0;
    let tick_width = 5.0;
    let tick_top = 1049.0;
    let white = [255, 255, 255, 255];
    let mut rect = |x: f32, y: f32, w: f32, h: f32, color: [u8; 4]| {
        _frame.draws.push(ui::UiDraw::Rect {
            pos: Vec2::new(x, y),
            size: Vec2::new(w, h),
            color,
        });
    };
    rect(left, top, right - left, bottom - top, white);
    rect(left, tick_top, tick_width, bottom - tick_top, white);
    rect(
        right - tick_width,
        tick_top,
        tick_width,
        bottom - tick_top,
        white,
    );
    // Native label A/B: a plain grey NoOutline glyph (no bright synthetic
    // outline) is the closest measured match (ruler MAE 115.7 vs 119.8 with the
    // material outline; native label AA cannot be reproduced with one glyph).
    _frame.draws.push(ui::UiDraw::Text {
        text: "100u".to_owned(),
        pos: Vec2::new((left + right) * 0.5 - 23.0, 1031.0),
        px: 24.0,
        color: [120, 120, 120, 255],
        role: ui::FontRole::Bold,
        font: Some("Roboto-Bold SDF NoOutline".to_owned()),
    });
}

fn ui_session_path(base: &Path, session: &str, total: usize) -> PathBuf {
    if total <= 1 {
        return base.to_path_buf();
    }
    let stem = base
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("ui-session");
    let extension = base
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("png");
    base.with_file_name(format!("{stem}-{session}.{extension}"))
}

/// `--editor-brain "tab;click 700,500;space 600,400;type tanh;enter"` (headless): steps on the Bibite Editor's Brain tab, through the
/// routines the window's pointer and keys use. `tab` shows the Brain tab, `click X,Y` a left click at a window point, `rightdrag
/// X,Y X,Y` a pan, `wheel X,Y N` the wheel, `space X,Y` Space with the pointer there, `type TEXT` typed characters, `enter` Enter,
/// `info` the help, `values` the eye, `hover X,Y` the pointer resting at a point, `connect` the node panel's outgoing connection, `edit
/// bias|act|weight` a slider's typed-value field, `dropdown X,Y,W,H` the activation function dropdown hanging under that rectangle.
fn run_editor_brain_script(app: &mut App, script: &str) {
    let point = |text: &str| -> Vec2 {
        let mut parts = text.split(',').map(|part| part.trim().parse::<f32>().unwrap_or(0.0));
        Vec2::new(parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0))
    };
    for step in script.split(';').map(str::trim).filter(|step| !step.is_empty()) {
        let (word, rest) = step.split_once(' ').unwrap_or((step, ""));
        match word {
            "tab" => {
                if let Some(session) = app.bibite_editor.as_mut() {
                    session.view.tab = template_editor_ui::Tab::Brain;
                }
            }
            "click" => {
                let at = point(rest);
                app.cursor = at;
                app.editor_brain_press(at, false);
                app.editor_brain_release(at);
            }
            "rightdrag" => {
                let mut ends = rest.split_whitespace();
                let (from, to) = (point(ends.next().unwrap_or("0,0")), point(ends.next().unwrap_or("0,0")));
                app.editor_brain_press(from, true);
                app.editor_brain_drag_to(to);
                app.editor_brain_release(to);
            }
            "wheel" => {
                let mut parts = rest.split_whitespace();
                let at = point(parts.next().unwrap_or("0,0"));
                let notches = parts.next().and_then(|text| text.parse().ok()).unwrap_or(0.0);
                app.cursor = at;
                app.editor_brain_wheel(at, notches);
            }
            "space" => {
                let at = point(rest);
                app.cursor = at;
                app.editor_brain_frame(&FrameInput {
                    cursor: at,
                    toggle_pause: true,
                    ..FrameInput::default()
                });
            }
            "type" => {
                app.typed = rest.to_owned();
                app.editor_brain_type(&FrameInput::default());
            }
            "enter" => {
                app.editor_brain_frame(&FrameInput {
                    enter: true,
                    ..FrameInput::default()
                });
            }
            "info" => app.editor_brain_action(brain_editor_ui::BrainAction::ShowInfo),
            "values" => app.prefs.show_node_values = true,
            "hover" => app.cursor = point(rest),
            "connect" => {
                app.editor_brain_action(brain_editor_ui::BrainAction::AddConnection(true));
            }
            "edit" => {
                let field = match rest {
                    "act" => dyn_panel::FieldRef::BrainAct,
                    "weight" => dyn_panel::FieldRef::BrainWeight,
                    _ => dyn_panel::FieldRef::BrainBias,
                };
                app.apply_ui_actions(&[ui::UiAction::DynEditValue(field)]);
            }
            "dropdown" => {
                let mut parts = rest
                    .split(',')
                    .map(|part| part.trim().parse::<f32>().unwrap_or(0.0));
                app.click_rect = [
                    parts.next().unwrap_or(0.0),
                    parts.next().unwrap_or(0.0),
                    parts.next().unwrap_or(0.0),
                    parts.next().unwrap_or(0.0),
                ];
                app.apply_ui_actions(&[ui::UiAction::DropdownOpen(
                    ui::DropdownTarget::NodeFunction,
                )]);
            }
            other => eprintln!("--editor-brain: unknown step {other:?}"),
        }
    }
}

/// `--expanded-brain "settings;graph;start;run 2;bfs 40;wheel 960,540 -3"` (headless): the Expanded Brain View of the selected bibite
/// opens and takes steps, through the routines the window's pointer and clock use. `settings` the settings panel, `graph` the other graph
/// type, `start` / `stop` the force simulation, `run SECONDS` the simulation's frames (a sixtieth of a second each), `bfs NODE` a click on
/// that node of the brain, `clear` a click on the background, `wheel X,Y N` the wheel, `pan X,Y` the canvas moved to that place (canvas
/// units), `disabled` the toggle for disabled synapses, `drag N X,Y` the Nth neuron dragged to a window point, `hover X,Y` the pointer
/// resting at a point, `edit N` the typed-value field of the Nth setting.
fn run_expanded_brain_script(app: &mut App, script: &str) {
    let point = |text: &str| -> Vec2 {
        let mut parts = text.split(',').map(|part| part.trim().parse::<f32>().unwrap_or(0.0));
        Vec2::new(parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0))
    };
    for step in script.split(';').map(str::trim).filter(|step| !step.is_empty()) {
        let (word, rest) = step.split_once(' ').unwrap_or((step, ""));
        match word {
            "settings" => app.apply_expanded_action(expanded_brain_ui::ExAction::Settings),
            "graph" => app.apply_expanded_action(expanded_brain_ui::ExAction::ToggleGraph),
            "start" => app.apply_expanded_action(expanded_brain_ui::ExAction::Start),
            "stop" => app.apply_expanded_action(expanded_brain_ui::ExAction::Stop),
            "disabled" => app.apply_expanded_action(expanded_brain_ui::ExAction::ToggleDisabled),
            "run" => {
                let seconds: f32 = rest.trim().parse().unwrap_or(1.0);
                let frames = (seconds * 60.0).round() as usize;
                for _ in 0..frames {
                    app.expanded_brain_frame(1.0 / 60.0);
                }
            }
            "bfs" => {
                if let (Ok(node), Some(session)) = (rest.trim().parse::<usize>(), app.expanded_brain.as_mut()) {
                    session.model.bfs(node);
                }
            }
            "clear" => {
                if let Some(session) = app.expanded_brain.as_mut() {
                    session.model.reset_highlight();
                }
            }
            "wheel" => {
                let mut parts = rest.split_whitespace();
                let at = point(parts.next().unwrap_or("0,0"));
                let notches = parts.next().and_then(|text| text.parse().ok()).unwrap_or(0.0);
                app.cursor = at;
                app.expanded_brain_wheel(at, notches);
            }
            "pan" => {
                let to = point(rest);
                app.expanded_brain_pan_to([to.x, to.y]);
            }
            "drag" => {
                let mut parts = rest.split_whitespace();
                let index = parts.next().and_then(|text| text.parse().ok()).unwrap_or(0);
                let to = point(parts.next().unwrap_or("0,0"));
                app.expanded_brain_drag_neuron(index, to);
            }
            "hover" => app.cursor = point(rest),
            "edit" => {
                let index = rest.trim().parse().unwrap_or(0);
                app.apply_ui_actions(&[ui::UiAction::DynEditValue(dyn_panel::FieldRef::Expanded(index))]);
            }
            other => eprintln!("--expanded-brain: unknown step {other:?}"),
        }
    }
}

fn run_screenshot(
    loaded: LoadedWorld,
    challenge_setup: Option<challenge_run::ChallengeSetup>,
    args: &Args,
    path: &Path,
    assets: Option<GameAssets>,
    mut themed: Option<ThemedUi>,
) -> Result<(), String> {
    let viewport = Vec2::new(args.width as f32, args.height as f32);
    // `RUSTIBITES_EFFECT_TIME=<seconds>`: the clock of the animated shaders in this picture (default 0).
    if let Some(seconds) = std::env::var("RUSTIBITES_EFFECT_TIME").ok().and_then(|value| value.parse::<f32>().ok()) {
        render::set_effect_time(seconds);
    }
    let mut app = App::new(loaded, viewport, assets);
    apply_user_settings(&mut app, &load_user_settings());
    apply_stored_toggles(&mut app);
    apply_view_options(&mut app, args);
    if let Some(setup) = challenge_setup {
        app.begin_challenge(setup);
        // The seconds the challenge ran: the world's ticks and the once-a-second checks.
        let fixed_dt = app.world.settings.fixed_dt();
        let ticks = (args.challenge_seconds / fixed_dt).round() as u64;
        for _ in 0..ticks {
            app.world.tick();
            app.update_challenge(fixed_dt);
        }
        if args.open_tracker {
            app.panels.open(ui::PanelKind::ChallengeTracker);
        }
    }
    if let Some(ortho) = args.ortho_size {
        app.set_ortho_size(ortho, args.height as f32);
    }
    if args.select_first {
        app.select_first();
    }
    if args.show_targets {
        app.show_targets = true;
    }
    if args.bibite_editor {
        if app.selected.is_some() {
            app.open_template_editor_for_selection();
        } else {
            // The Library's Create New button.
            app.open_editor_from_library(None, false);
        }
        if let (Some(rows), Some(session)) = (args.editor_scroll, app.bibite_editor.as_mut()) {
            session.view.scroll_rows = rows.min(template_editor_ui::max_scroll_rows());
        }
        if let (true, Some(session)) = (args.editor_info, app.bibite_editor.as_mut()) {
            session.view.info_open = true;
        }
        if let Some(script) = args.editor_brain.as_deref() {
            run_editor_brain_script(&mut app, script);
        }
    }
    if let Some(script) = args.expanded_brain.as_deref() {
        app.open_expanded_brain();
        run_expanded_brain_script(&mut app, script);
    }
    if let Some(id) = args.achievement.as_deref() {
        app.trigger_achievement(id);
    }
    if let Some(name) = args.save_panel.as_deref() {
        // `--saves-dir` is a data folder with a `Savefiles` folder in it (the panel lists its saves).
        if let Some(root) = args.saves_dir.as_ref() {
            app.storage = bibites_io::storage::StoragePaths::new(root.clone());
        }
        app.apply_ui_actions(&[ui::UiAction::Invoke(ui::ControlCommand::OpenEscapeMenu)]);
        app.apply_ui_actions(&[ui::UiAction::Invoke(ui::ControlCommand::OpenSavePanel)]);
        if let Some(row) = name.strip_prefix("row:").and_then(|row| row.parse::<u16>().ok()) {
            app.apply_ui_actions(&[ui::UiAction::SavePanel(save_panel::SaveAction::Row(row))]);
        } else if name != "-"
            && let Some(panel) = app.save_panel.as_mut()
        {
            panel.name = name.to_owned();
        }
    }
    if let Some(gene) = args.genes_gene {
        app.graphs_gene = gene.min(bibites_core::genome::N_GENES - 1);
    }
    if let Some(time) = args.genes_time {
        app.graphs_gene_time = time;
    }
    if let Some(tab) = args.tab {
        app.inspector_tab = tab;
        app.inspector_open = true;
    }
    if args.center_on_selected {
        app.center_on_selected();
    }
    if args.only_layer.is_some() {
        // Layer diagnostics: hide HUD/scene layers so only the requested
        // bibite layer is visible (headless only).
        app.show_hud = false;
        app.scene_options.grid = false;
        app.scene_options.zones = false;
        app.scene_options.pellets = false;
        app.scene_options.pheromones = false;
    }
    apply_lineage_flags(&mut app, &args);
    // Deterministic HUD click for headless UI verification (button/toggle/tab
    // hit-testing goes through the same UI-first routing the window uses).
    if let (Some(first), Some(themed)) = (args.click, themed.as_ref()) {
        for point in std::iter::once(first).chain(args.extra_clicks.iter().copied()) {
            let frame = build_ui_frame(&app, themed, args.width as f32, args.height as f32);
            let actions = frame.interact(point, true, true, &mut ui::UiInteraction::default());
            remember_click_rect(&mut app, &frame, point, &actions);
            app.apply_ui_actions(&actions);
        }
    }

    // A left-button drag on the world through the same input path as the window: press, hold past the
    // click time, release (unless `--drag-hold`).
    if let Some([x0, y0, x1, y1]) = args.drag {
        let (start, end) = (Vec2::new(x0, y0), Vec2::new(x1, y1));
        let frame = |cursor: Vec2, pressed: bool, down: bool, released: bool, dt: f32| FrameInput {
            cursor,
            primary_pressed: pressed,
            primary_down: down,
            primary_released: released,
            dt,
            ..FrameInput::default()
        };
        app.handle_input(&frame(start, true, true, false, 0.016));
        app.handle_input(&frame(end, false, true, false, 0.3));
        if !args.drag_hold {
            app.handle_input(&frame(end, false, false, true, 0.016));
        }
    }

    // Print a few entity positions: handy for composing `--center`/`--zoom`
    // demo shots without opening the window.
    let mut previewed = 0;
    for (_, record) in app.world.bibites.iter() {
        if record.is_corpse {
            continue;
        }
        eprintln!(
            "  bibite {} pos ({:.1}, {:.1}) d1 {:.2}",
            record.id, record.pos.x, record.pos.y, record.body.geometry.d1_size,
        );
        previewed += 1;
        if previewed >= 5 {
            break;
        }
    }
    for zone in &app.world.zones.zones {
        eprintln!(
            "  zone '{}' centre ({:.1}, {:.1}) radius {:.1} rect={} ring={}",
            zone.config.name, zone.pos.x, zone.pos.y, zone.radius, zone.is_rect, zone.is_ring,
        );
    }

    if args.select_tool {
        app.select_last_tool();
    }
    if args.save_dialog {
        app.apply_ui_actions(&[ui::UiAction::Invoke(ui::ControlCommand::SaveSelected)]);
    }
    if let Some(name) = args.scenario_saver.as_ref() {
        app.apply_ui_actions(&[ui::UiAction::DynSaveScenario]);
        if let Some(saver) = app.scenario_saver.as_mut() {
            saver.name.clone_from(name);
            saver.description = "Oases that dry out in winter.".to_owned();
        }
    }
    if let Some(index) = args.select_zone {
        app.select_zone(index);
    }
    if args.zone_preview {
        app.zone_preview_open = true;
    }
    if args.warning_popup {
        // Only shown: a screenshot run never answers it, so nothing is deleted.
        app.confirm_dialog = Some(app::ConfirmKind::DeleteSave(PathBuf::from("example.zip")));
    }
    if let Some(tab) = args.dyn_tab {
        app.panels.open(ui::PanelKind::WorldDynamics);
        app.dyn_panel.tab = tab.min(4);
        app.dyn_panel.scroll = args.dyn_scroll;
        for group in &args.dyn_open {
            app.dyn_panel.toggle_sub(*group);
        }
    }
    for _ in 0..args.add_zone_groups {
        app.apply_ui_actions(&[ui::UiAction::Invoke(ui::ControlCommand::AddZoneGroup)]);
    }
    for preset in &args.add_animators {
        app.apply_ui_actions(&[ui::UiAction::Invoke(ui::ControlCommand::OpenAnimatorCreator)]);
        app.animator.creator.preset = animators::Preset::ALL[(*preset).min(2)];
        app.apply_ui_actions(&[ui::UiAction::Invoke(ui::ControlCommand::SubmitAnimatorCreator)]);
    }
    for slot in 0..args.animator_bases {
        let _ = slot;
        let _ = app.animator_dropdown_pick(ui::DropdownTarget::AnimatorBase(0), 1 + slot * 7);
    }
    if let Some(point) = args.animator_point {
        app.animator.point = Some((0, point));
    }
    if let Some(preset) = args.animator_creator {
        app.apply_ui_actions(&[ui::UiAction::Invoke(ui::ControlCommand::OpenAnimatorCreator)]);
        app.animator.creator.preset = animators::Preset::ALL[preset.min(2)];
    }
    if let Some((key, text)) = &args.dyn_edit
        && let Some((index, _)) = dyn_panel::ui_of(key)
    {
        app.apply_ui_actions(&[ui::UiAction::DynEditValue(dyn_panel::FieldRef::Setting(
            index as u16,
        ))]);
        if let Some(edit) = app.value_edit.as_mut() {
            edit.paste(text);
        }
    }
    if let Some(template) = &args.placer_template {
        app.start_bibite_placer(template);
        if let Some(tagging) = args.placer_tagging.as_deref() {
            app.placer_options.tagging = match tagging {
                "species" => bibites_core::settings::Tagging::SpeciesTagging,
                "random" => bibites_core::settings::Tagging::RandomTagging,
                "custom" => bibites_core::settings::Tagging::CustomTagging,
                _ => bibites_core::settings::Tagging::NoTagging,
            };
        }
    }
    for _ in 0..args.frames {
        app.advance(1.0 / 60.0);
    }
    // `RUSTIBITES_TIME_FRAMES=N`: times N more frames of the window's CPU work (the simulation steps of
    // `advance`, then the scene build; the GPU draw calls of the window are not part of it) and prints the means.
    if let Some(count) = std::env::var("RUSTIBITES_TIME_FRAMES")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|count| *count > 0)
    {
        let (mut advance, mut build) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let ticks_before = app.sim_ticks;
        for _ in 0..count {
            let start = std::time::Instant::now();
            app.advance(1.0 / 60.0);
            advance += start.elapsed();
            let start = std::time::Instant::now();
            app.rebuild_scene();
            build += start.elapsed();
        }
        let ms = |total: std::time::Duration| total.as_secs_f64() * 1000.0 / f64::from(count);
        eprintln!(
            "frame timing over {count} frames: advance {:.3} ms ({} ticks), scene build {:.3} ms, {} primitives + {} dots, {} pellets, {} bibites",
            ms(advance),
            app.sim_ticks - ticks_before,
            ms(build),
            app.scene.primitives.len(),
            app.scene.dots.len(),
            app.world.pellets.len(),
            app.world.bibites.len(),
        );
    }
    app.refresh_overlay(0.0);
    app.rebuild_scene();
    // The pointer rests on a control or a pellet until its tooltip is fully shown.
    if let (Some(point), Some(themed)) = (args.hover, themed.as_ref()) {
        app.cursor = point;
        // The Graphs panel's legend opens while the pointer rests on its "?" button.
        app.graphs_legend_open = app.panels.is_open(ui::PanelKind::Graphs)
            && hud::graphs_legend_hovered(&app, point, false, args.width as f32);
        let frame = build_ui_frame(&app, themed, args.width as f32, args.height as f32);
        let over_ui = frame.widget_at(point).is_some();
        app.update_tooltip(&frame, point, over_ui, 1.0);
        app.update_tooltip(&frame, point, over_ui, 1.0);
        app.update_bar_hover(Vec2::new(args.width as f32, args.height as f32), 1.0);
        app.update_bar_hover(Vec2::new(args.width as f32, args.height as f32), 1.0);
    }
    if args.dump_layer_bbox {
        dump_layer_bboxes(&app);
    }

    let atlases: &[assets::Atlas] = app
        .assets
        .as_ref()
        .map_or(&[], |assets| assets.atlases.as_slice());
    // `H`/`--only-layer` diagnostics disable the HUD: do not build a UI overlay
    // in that case (otherwise layer-only captures are polluted by bright HUD
    // text/sprites even though show_hud is false).
    let hud_frame = if app.show_hud {
        themed
            .as_ref()
            .map(|themed| build_ui_frame(&app, themed, args.width as f32, args.height as f32))
    } else {
        None
    };
    let overlay = match (themed.as_mut(), hud_frame.as_ref()) {
        (Some(themed), Some(frame)) => Some(render::UiOverlay {
            assets: &themed.ui,
            frame,
            fonts: &mut themed.fonts,
        }),
        _ => None,
    };
    let image = if args.ui_only {
        if overlay.is_none() {
            return Err(
                "--ui-only requires the themed UI atlas and fonts (not --simple-hud)".to_owned(),
            );
        }
        render::render_ui_only(args.width, args.height, 1, overlay)
    } else {
        render::render_scene_with_ui(
            &app.scene,
            &app.camera,
            args.width,
            args.height,
            2,
            atlases,
            overlay,
        )
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    image
        .write_png(path)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    let bytes = std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let ImageStats {
        width,
        height,
        non_background,
        distinct_colors,
        green,
        red,
        bright,
        mean_luma,
    } = image.stats();

    println!("screenshot: {}", path.display());
    if args.ui_only {
        println!("  mode:   UI-only alpha overlay (transparent background, HUD A/B)");
    }
    let render_note = if args.ui_only {
        "PNG, UI-only alpha, 1x"
    } else {
        "PNG, 2x supersampled"
    };
    println!("  image:  {width}x{height}, {bytes} bytes ({render_note})");
    println!(
        "  world:  {} | headless software renderer | sim {:.2}s / tick {} | pop {}",
        app.label,
        app.simulated_time(),
        app.world_tick(),
        app.hud.population,
    );
    println!(
        "  scene:  {} primitives | non-background {:.1}% | colors {} | green {} | red {} | bright {} | luma {:.1}",
        app.scene.len(),
        non_background as f64 / f64::from(width * height) * 100.0,
        distinct_colors,
        green,
        red,
        bright,
        mean_luma,
    );
    if let Some(reference_path) = &args.compare_ref {
        let reference = load_reference(reference_path)?;
        let composed = side_by_side(&image, &reference, 8);
        let compare_path = comparison_path(path);
        composed
            .write_png(&compare_path)
            .map_err(|error| format!("could not write {}: {error}", compare_path.display()))?;
        println!(
            "  compare: {} (left = RUSTibites, right = game reference {})",
            compare_path.display(),
            reference_path.display(),
        );
    }
    println!(
        "  camera: center ({:.1}, {:.1}) | zoom {:.3} px/u | fit {}",
        app.camera.center.x,
        app.camera.center.y,
        app.camera.zoom,
        match args.fit {
            FitMode::Bibites => "bibites",
            FitMode::World => "world",
        },
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Game assets / reference comparison
// ---------------------------------------------------------------------------

/// Loads the game-asset renderer when available, logging the reason for any
/// fallback (a missing `assets/game` folder is a normal, supported state).
fn load_assets(args: &Args) -> Option<GameAssets> {
    if args.no_assets {
        eprintln!("renderer: procedural fallback (--no-assets)");
        return None;
    }
    let loaded = match &args.assets {
        Some(directory) => GameAssets::load(directory)
            .map(Some)
            .map_err(|error| format!("{}: {error}", directory.display())),
        None => GameAssets::discover(),
    };
    match loaded {
        Ok(Some(assets)) => {
            eprintln!(
                "game assets: {} atlases, {} bibite size tiers, ortho {:.0}, skin hue {:.2} from {}",
                assets.atlases.len(),
                assets.part_sizes.len(),
                assets.camera.orthographic_size,
                assets.parameters.initial_hue,
                assets.root.display(),
            );
            Some(assets)
        }
        Ok(None) => {
            eprintln!(
                "game assets not found (looked for assets/game/manifest.json); \
                 using the built-in procedural renderer - see docs/VIEWER.md"
            );
            None
        }
        Err(error) => {
            eprintln!(
                "warning: game assets unusable ({error}); \
                 using the built-in procedural renderer"
            );
            None
        }
    }
}

/// Decodes a reference screenshot (PNG, RGBA/RGB/grayscale).
fn load_reference(path: &Path) -> Result<render::Image, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("reference PNG header error: {error}"))?;
    let mut buffer = vec![0_u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("reference PNG decode error: {error}"))?;
    let raw = &buffer[..info.buffer_size()];
    let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(raw),
        png::ColorType::Rgb => {
            for pixel in raw.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &value in raw {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in raw.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => {
            return Err("indexed reference PNG is not supported".to_owned());
        }
    }
    Ok(render::Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// Places two images side by side with a neutral separator (for visual parity
/// review: left = RUSTibites, right = the game).
fn side_by_side(left: &render::Image, right: &render::Image, gap: u32) -> render::Image {
    let width = left.width + gap + right.width;
    let height = left.height.max(right.height);
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    // Separator: mid grey so the two panels are visually distinct.
    for y in 0..height {
        for x in left.width..(left.width + gap).min(width) {
            let offset = ((y * width + x) * 4) as usize;
            rgba[offset..offset + 4].copy_from_slice(&[70, 70, 76, 255]);
        }
    }
    blit(&mut rgba, width, left, 0, 0);
    blit(&mut rgba, width, right, left.width + gap, 0);
    render::Image {
        width,
        height,
        rgba,
    }
}

fn blit(
    target: &mut [u8],
    target_width: u32,
    source: &render::Image,
    x_offset: u32,
    y_offset: u32,
) {
    for y in 0..source.height {
        let target_row = y_offset + y;
        if target_row >= (target.len() as u32 / (target_width * 4).max(1)) {
            break;
        }
        for x in 0..source.width {
            let source_offset = ((y * source.width + x) * 4) as usize;
            let target_offset = (((target_row) * target_width + (x + x_offset)) * 4) as usize;
            if target_offset + 4 <= target.len() {
                target[target_offset..target_offset + 4]
                    .copy_from_slice(&source.rgba[source_offset..source_offset + 4]);
            }
        }
    }
}

/// `foo.png` -> `foo-compare.png`.
fn comparison_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("viewer");
    path.with_file_name(format!("{stem}-compare.png"))
}

// ---------------------------------------------------------------------------
// Window icon
// ---------------------------------------------------------------------------

/// Loads `assets/icon/rustibites_256.png` as a window icon.
///
/// The tracked `.ico` is a multi-resolution Windows resource (used by
/// `assets/icon/rustibites.rc` for the executable); decoding it would need an
/// extra dependency, so the window icon is derived from the same artwork's PNG
/// export instead. Returns `None` when the asset is not found (e.g. an installed
/// binary without the source tree), and miniquad's default is kept.
fn load_window_icon() -> Option<miniquad::conf::Icon> {
    let candidates = [
        PathBuf::from("assets/icon/rustibites_256.png"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("assets")
            .join("icon")
            .join("rustibites_256.png"),
    ];
    for path in candidates {
        match decode_icon(&path) {
            Ok(icon) => return Some(icon),
            Err(error) => eprintln!("window icon: {}: {error}", path.display()),
        }
    }
    None
}

/// Decodes a PNG into a miniquad window icon (16/32/64 px levels).
fn decode_icon(path: &Path) -> Result<miniquad::conf::Icon, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("PNG header error: {error}"))?;
    let mut buffer = vec![0_u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("PNG decode error: {error}"))?;
    let raw = &buffer[..info.buffer_size()];
    let rgba = to_rgba(raw, info.width, info.height, info.color_type)?;

    let mut icon = miniquad::conf::Icon {
        small: [0; 16 * 16 * 4],
        medium: [0; 32 * 32 * 4],
        big: [0; 64 * 64 * 4],
    };
    icon.small
        .copy_from_slice(&resample_rgba(&rgba, info.width, info.height, 16));
    icon.medium
        .copy_from_slice(&resample_rgba(&rgba, info.width, info.height, 32));
    icon.big
        .copy_from_slice(&resample_rgba(&rgba, info.width, info.height, 64));
    Ok(icon)
}

/// Expands any PNG colour type to tightly packed RGBA8.
fn to_rgba(
    raw: &[u8],
    width: u32,
    height: u32,
    color_type: png::ColorType,
) -> Result<Vec<u8>, String> {
    let pixels = (width * height) as usize;
    let mut rgba = Vec::with_capacity(pixels * 4);
    match color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(raw),
        png::ColorType::Rgb => {
            for pixel in raw.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &value in raw {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in raw.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => {
            return Err("indexed PNG was not expanded".to_owned());
        }
    }
    if rgba.len() != pixels * 4 {
        return Err(format!(
            "unexpected PNG layout ({color_type:?}, {width}x{height})"
        ));
    }
    Ok(rgba)
}

/// Nearest-neighbour resample of an RGBA8 image to a square icon level.
fn resample_rgba(src: &[u8], width: u32, height: u32, size: usize) -> Vec<u8> {
    let mut out = vec![0_u8; size * size * 4];
    if width == 0 || height == 0 {
        return out;
    }
    for y in 0..size {
        for x in 0..size {
            let source_x = x * width as usize / size;
            let source_y = y * height as usize / size;
            let source = (source_y * width as usize + source_x) * 4;
            let target = (y * size + x) * 4;
            out[target..target + 4].copy_from_slice(&src[source..source + 4]);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CLI parsing
// ---------------------------------------------------------------------------

/// Parses the command line, rejecting unknown options and non-numeric values.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args::default();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => parsed.help = true,
            "--seed" => {
                parsed.seed = next_value(&mut args, "--seed")?
                    .parse()
                    .map_err(invalid_number)?
            }
            "--bibites" => {
                parsed.bibites = next_value(&mut args, "--bibites")?
                    .parse()
                    .map_err(invalid_number)?
            }
            "--threads" => {
                parsed.threads = next_value(&mut args, "--threads")?
                    .parse()
                    .map_err(invalid_number)?
            }
            "--screenshot" => {
                parsed.screenshot = Some(PathBuf::from(next_value(&mut args, "--screenshot")?))
            }
            "--frames" => {
                parsed.frames = next_value(&mut args, "--frames")?
                    .parse()
                    .map_err(invalid_number)?
            }
            "--width" => {
                parsed.size_given = true;
                parsed.width = next_value(&mut args, "--width")?
                    .parse()
                    .map_err(invalid_number)?
            }
            "--height" => {
                parsed.size_given = true;
                parsed.height = next_value(&mut args, "--height")?
                    .parse()
                    .map_err(invalid_number)?
            }
            "--fit" => {
                parsed.fit_given = true;
                let value = next_value(&mut args, "--fit")?;
                parsed.fit = match value.as_str() {
                    "bibites" => FitMode::Bibites,
                    "world" => FitMode::World,
                    other => {
                        return Err(format!(
                            "invalid `--fit {other}` (expected `bibites` or `world`)"
                        ));
                    }
                };
            }
            "--center" => parsed.center = Some(parse_center(&next_value(&mut args, "--center")?)?),
            "--assets" => parsed.assets = Some(PathBuf::from(next_value(&mut args, "--assets")?)),
            "--no-assets" => parsed.no_assets = true,
            "--ortho-size" => {
                parsed.ortho_size = Some(next_value(&mut args, "--ortho-size")?.parse().map_err(
                    |error: std::num::ParseFloatError| format!("invalid number: {error}"),
                )?)
            }
            "--compare-ref" => {
                parsed.compare_ref = Some(PathBuf::from(next_value(&mut args, "--compare-ref")?))
            }
            "--simple-hud" => parsed.simple_hud = true,
            "--runtime-ui" => parsed.runtime_ui = true,
            "--no-runtime-ui" => parsed.runtime_ui = false,
            "--ui-only" => parsed.ui_only = true,
            "--ui-text" => parsed.ui_text = Some(next_value(&mut args, "--ui-text")?),
            "--ui-runtime" => {
                parsed.ui_runtime = Some(PathBuf::from(next_value(&mut args, "--ui-runtime")?));
            }
            "--ui-controls" => parsed.ui_controls = true,
            "--ui-functional-audit" => parsed.ui_functional_audit = true,
            "--ui-functional-matrix" => parsed.ui_functional_matrix = true,
            "--ui-controls-session" => {
                let value = next_value(&mut args, "--ui-controls-session")?;
                for name in value.split(',') {
                    if !name.trim().is_empty() {
                        parsed.ui_controls_sessions.push(name.trim().to_owned());
                    }
                }
            }
            "--ui-controls-out" => {
                parsed.ui_controls_out =
                    Some(PathBuf::from(next_value(&mut args, "--ui-controls-out")?));
            }
            "--ui-session" => {
                let value = next_value(&mut args, "--ui-session")?;
                for name in value.split(',') {
                    let name = name.trim();
                    if !name.is_empty() {
                        parsed.ui_session.push(name.to_owned());
                    }
                }
            }
            "--ui-session-list" => parsed.ui_session_list = true,
            "--time-load-list" => parsed.time_load_list = true,
            "--ui-background" => {
                parsed.ui_background =
                    Some(PathBuf::from(next_value(&mut args, "--ui-background")?));
            }
            "--select-first" => parsed.select_first = true,
            "--paused" => parsed.paused = true,
            "--show-targets" => parsed.show_targets = true,
            "--select-tool" => parsed.select_tool = true,
            "--save-dialog" => parsed.save_dialog = true,
            "--scenario-saver" => parsed.scenario_saver = Some(next_value(&mut args, "--scenario-saver")?),
            "--dyn-tab" => {
                let value = next_value(&mut args, "--dyn-tab")?;
                parsed.dyn_tab = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --dyn-tab: {error}"))?,
                );
            }
            "--add-animator" => {
                let preset = next_value(&mut args, "--add-animator")?
                    .parse()
                    .map_err(|error| format!("invalid --add-animator: {error}"))?;
                parsed.add_animators.push(preset);
            }
            "--animator-creator" => {
                parsed.animator_creator = Some(
                    next_value(&mut args, "--animator-creator")?
                        .parse()
                        .map_err(|error| format!("invalid --animator-creator: {error}"))?,
                );
            }
            "--animator-base" => {
                parsed.animator_bases = next_value(&mut args, "--animator-base")?
                    .parse()
                    .map_err(|error| format!("invalid --animator-base: {error}"))?;
            }
            "--animator-point" => {
                parsed.animator_point = Some(
                    next_value(&mut args, "--animator-point")?
                        .parse()
                        .map_err(|error| format!("invalid --animator-point: {error}"))?,
                );
            }
            "--add-zone-group" => {
                parsed.add_zone_groups = next_value(&mut args, "--add-zone-group")?
                    .parse()
                    .map_err(|error| format!("invalid --add-zone-group: {error}"))?;
            }
            "--dyn-open" => {
                let value = next_value(&mut args, "--dyn-open")?;
                parsed.dyn_open.push(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --dyn-open: {error}"))?,
                );
            }
            "--dyn-edit" => {
                let value = next_value(&mut args, "--dyn-edit")?;
                let (key, text) = value.split_once(':').unwrap_or((value.as_str(), ""));
                parsed.dyn_edit = Some((key.to_owned(), text.to_owned()));
            }
            "--zone-preview" => parsed.zone_preview = true,
            "--warning-popup" => parsed.warning_popup = true,
            "--hover" => parsed.hover = Some(parse_center(&next_value(&mut args, "--hover")?)?),
            "--dyn-scroll" => {
                let value = next_value(&mut args, "--dyn-scroll")?;
                parsed.dyn_scroll = value
                    .parse()
                    .map_err(|error| format!("invalid --dyn-scroll: {error}"))?;
            }
            "--brain-differences" => parsed.brain_differences = true,
            "--lineage-mesh" => parsed.lineage_mesh = true,
            "--lineage-show-extinct" => parsed.lineage_show_extinct = true,
            "--node-names" => parsed.node_names = true,
            "--bibite-editor" => parsed.bibite_editor = true,
            "--editor-info" => parsed.editor_info = true,
            "--achievement" => {
                parsed.achievement = Some(next_value(&mut args, "--achievement")?);
            }
            "--challenge-champion" => {
                parsed.challenge_champion = Some(PathBuf::from(next_value(
                    &mut args,
                    "--challenge-champion",
                )?));
            }
            "--open-tracker" => parsed.open_tracker = true,
            "--challenge-seconds" => {
                parsed.challenge_seconds = next_value(&mut args, "--challenge-seconds")?
                    .parse()
                    .map_err(|error| format!("invalid --challenge-seconds: {error}"))?;
            }
            "--expanded-brain" => {
                parsed.expanded_brain = Some(next_value(&mut args, "--expanded-brain")?);
            }
            "--editor-brain" => {
                parsed.editor_brain = Some(next_value(&mut args, "--editor-brain")?);
            }
            "--editor-scroll" => {
                let value = next_value(&mut args, "--editor-scroll")?;
                parsed.editor_scroll = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --editor-scroll: {error}"))?,
                );
            }
            "--genes-gene" => {
                let value = next_value(&mut args, "--genes-gene")?;
                parsed.genes_gene = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --genes-gene: {error}"))?,
                );
            }
            "--genes-time" => {
                let value = next_value(&mut args, "--genes-time")?;
                parsed.genes_time = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --genes-time: {error}"))?,
                );
            }
            "--position-time" => {
                let value = next_value(&mut args, "--position-time")?;
                parsed.position_time = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --position-time: {error}"))?,
                );
            }
            "--lineage-tab" => {
                let value = next_value(&mut args, "--lineage-tab")?;
                parsed.lineage_tab = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --lineage-tab: {error}"))?,
                );
            }
            "--lineage-settings" => {
                let value = next_value(&mut args, "--lineage-settings")?;
                parsed.lineage_settings = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --lineage-settings: {error}"))?,
                );
            }
            "--lineage-zoom" => {
                let value = next_value(&mut args, "--lineage-zoom")?;
                parsed.lineage_zoom = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --lineage-zoom: {error}"))?,
                );
            }
            "--lineage-pos" => {
                let value = next_value(&mut args, "--lineage-pos")?;
                let (x, y) = value
                    .split_once(',')
                    .ok_or_else(|| "--lineage-pos needs x,y".to_owned())?;
                parsed.lineage_pos = Some((
                    x.trim()
                        .parse()
                        .map_err(|error| format!("invalid --lineage-pos: {error}"))?,
                    y.trim()
                        .parse()
                        .map_err(|error| format!("invalid --lineage-pos: {error}"))?,
                ));
            }
            "--lineage-species" => {
                let value = next_value(&mut args, "--lineage-species")?;
                parsed.lineage_species = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --lineage-species: {error}"))?,
                );
            }
            "--select-zone" => {
                let value = next_value(&mut args, "--select-zone")?;
                parsed.select_zone = Some(
                    value
                        .parse()
                        .map_err(|error| format!("invalid --select-zone: {error}"))?,
                );
            }
            "--placer" => {
                parsed.placer_template = Some(PathBuf::from(next_value(&mut args, "--placer")?));
            }
            "--placer-tagging" => {
                parsed.placer_tagging = Some(next_value(&mut args, "--placer-tagging")?);
            }
            "--center-on-selected" => parsed.center_on_selected = true,
            "--gpu-hue-shader" => parsed.gpu_hue_shader = true,
            "--no-gpu-hue-shader" => parsed.gpu_hue_shader = false,
            "--sim-thread" => parsed.sim_thread = true,
            "--overlap" => parsed.overlap_ticks = true,
            "--no-overlap" => parsed.overlap_ticks = false,
            "--speed" => {
                parsed.speed = Some(
                    next_value(&mut args, "--speed")?
                        .parse()
                        .map_err(|error| format!("invalid --speed: {error}"))?,
                )
            }
            "--menu" => parsed.menu = true,
            "--no-menu" => parsed.no_menu = true,
            "--saves-dir" => {
                parsed.saves_dir = Some(PathBuf::from(next_value(&mut args, "--saves-dir")?))
            }
            "--menu-view" => parsed.menu_view = Some(next_value(&mut args, "--menu-view")?),
            "--menu-click" => {
                let point = parse_center(&next_value(&mut args, "--menu-click")?)?;
                parsed.menu_clicks.push(point);
            }
            "--menu-drag" => {
                let value = next_value(&mut args, "--menu-drag")?;
                let numbers: Vec<f32> = value
                    .split(',')
                    .map(|part| part.trim().parse::<f32>().map_err(|_| format!("--menu-drag expects x0,y0,x1,y1, got `{value}`")))
                    .collect::<Result<_, _>>()?;
                let [x0, y0, x1, y1] = numbers[..] else {
                    return Err(format!("--menu-drag expects x0,y0,x1,y1, got `{value}`"));
                };
                parsed.menu_drags.push((Vec2::new(x0, y0), Vec2::new(x1, y1)));
            }
            "--save-panel" => parsed.save_panel = Some(next_value(&mut args, "--save-panel")?),
            "--menu-champion" => {
                parsed.menu_champion =
                    Some(PathBuf::from(next_value(&mut args, "--menu-champion")?));
            }
            "--click" => {
                let point = parse_center(&next_value(&mut args, "--click")?)?;
                if parsed.click.is_none() {
                    parsed.click = Some(point);
                } else {
                    parsed.extra_clicks.push(point);
                }
            }
            "--drag" => {
                let value = next_value(&mut args, "--drag")?;
                let numbers: Vec<f32> = value
                    .split(',')
                    .map(|part| part.trim().parse::<f32>())
                    .collect::<Result<_, _>>()
                    .map_err(|error| format!("invalid --drag value: {error}"))?;
                let [x0, y0, x1, y1] = numbers[..] else {
                    return Err("`--drag` must be `X0,Y0,X1,Y1`".to_owned());
                };
                parsed.drag = Some([x0, y0, x1, y1]);
            }
            "--drag-hold" => parsed.drag_hold = true,
            "--only-layer" => {
                let value = next_value(&mut args, "--only-layer")?;
                parsed.only_layer = Some(sprites::OnlyLayer::parse(&value)?);
            }
            "--dump-layer-bbox" => parsed.dump_layer_bbox = true,
            "--tab" => {
                let value = next_value(&mut args, "--tab")?;
                parsed.tab = Some(match value.as_str() {
                    "stats" => ui::InspectorTab::Stats,
                    "genes" => ui::InspectorTab::Genes,
                    "biology" => ui::InspectorTab::Biology,
                    "brain" => ui::InspectorTab::Brain,
                    other => return Err(format!("invalid --tab {other}")),
                });
            }
            "--zoom" => {
                parsed.zoom = Some(next_value(&mut args, "--zoom")?.parse().map_err(
                    |error: std::num::ParseFloatError| format!("invalid number: {error}"),
                )?)
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option `{other}`"));
            }
            path => {
                if parsed.input.is_some() {
                    return Err(format!("unexpected second path `{path}`"));
                }
                parsed.input = Some(PathBuf::from(path));
            }
        }
    }
    if parsed.width == 0 || parsed.height == 0 {
        return Err("--width/--height must be positive".to_owned());
    }
    if parsed.bibites == 0 {
        return Err("--bibites must be positive".to_owned());
    }
    Ok(parsed)
}

fn next_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("`{option}` needs a value"))
}

fn invalid_number(error: std::num::ParseIntError) -> String {
    format!("invalid number: {error}")
}

/// Parses `X,Y` (world units) for `--center`.
fn parse_center(value: &str) -> Result<Vec2, String> {
    let (x, y) = value
        .split_once(',')
        .ok_or_else(|| format!("`--center {value}` must be `X,Y`"))?;
    let x: f32 = x
        .trim()
        .parse()
        .map_err(|error| format!("invalid --center X: {error}"))?;
    let y: f32 = y
        .trim()
        .parse()
        .map_err(|error| format!("invalid --center Y: {error}"))?;
    if !x.is_finite() || !y.is_finite() {
        return Err("--center values must be finite".to_owned());
    }
    Ok(Vec2::new(x, y))
}

fn print_usage() {
    println!(
        "RUSTibites viewer (alpha)\n\
         \n\
         USAGE:\n\
         \x20   rustibites-view [PATH] [options]\n\
         \n\
         PATH:\n\
         \x20   save/scenario .zip, or a .bb8/.bb8template genome.\n\
         \x20   Omit it for a generated starter world with 200 bibites.\n\
         \n\
         OPTIONS:\n\
         \x20   --seed N            RNG seed for a freshly built world (default {DEFAULT_SEED})\n\
         \x20   --bibites N         bibites in the generated fallback world (default {DEFAULT_FALLBACK_BIBITES})\n\
         \x20   --threads N         simulation worker threads, 0 = auto (default 0); the world times its own ticks and uses\n\
         \x20                       them when that is faster (a small world ticks on one thread); 1 = the serial tick\n\
         \x20   --sim-thread        tick the world on a thread of its own while only the camera is used (window; also RUSTIBITES_SIM_THREAD=1)\n\
         \x20   --no-overlap        run a frame's ticks before it is drawn, not beside the drawing (window; also RUSTIBITES_OVERLAP=0)\n\
         \x20   --speed X           the time speed the window starts at (the slider's target; default 1)\n\
         \x20   --width N           window/screenshot width (default {DEFAULT_WIDTH})\n\
         \x20   --height N          window/screenshot height (default {DEFAULT_HEIGHT})\n\
         \x20   --fit MODE          initial camera: `bibites` (default) or `world`\n\
         \x20   --center X,Y        initial camera centre in world units\n\
         \x20   --zoom N            initial camera zoom in screen pixels per world unit\n\
         \x20   --ortho-size N      Unity orthographic size (half visible world height)\n\
         \x20   --assets DIR        game asset directory (default assets/game)\n\
         \x20   --no-assets         force the built-in procedural renderer\n\
         \x20   --compare-ref PNG   side-by-side comparison against a game frame\n\
         \x20   --simple-hud        use the built-in text HUD instead of themed UI\n\
         \x20   --runtime-ui        force the committed exact game UI layout (default when present)\n\
         \x20   --no-runtime-ui     keep the curated HUD layout instead of the runtime layout\n\
         \x20   --ui-only           write only the UI draw list into a transparent PNG (HUD A/B)\n\
         \x20   --ui-text MODE      UI text rasteriser: auto (default), sdf (TMP), ttf\n\
         \x20   --ui-session NAME   render captured runtime UI session(s) headlessly (repeatable/comma)\n\
         \x20   --ui-session-list    print all captured runtime UI session names\n\
         \x20   --ui-background PNG  background image for composed UI-session renders\n\
         \x20   --gpu-hue-shader    draw the skin / eye / egg sprites in the window with the GPU hue-match\n\
         \x20                       material (the default; --no-gpu-hue-shader recolours them on the\n\
         \x20                       processor instead, same colours, slower with many bibites;\n\
         \x20                       --screenshot always uses the software rasteriser)\n\
         \x20   --sim-thread        tick the world on a thread of its own while the window only watches\n\
         \x20                       (smooth frames with a big world; a click, a key or a panel takes the\n\
         \x20                       world back; also RUSTIBITES_SIM_THREAD=1)\n\
         \x20   --menu / --no-menu  force or bypass the game-style title menu (argument-less\n\
         \x20                       interactive windows show the menu by default)\n\
         \x20   --saves-dir DIR     directory scanned by Continue / Load Game\n\
         \x20   --screenshot PATH   render one frame headlessly to a PNG and exit\n\
         \x20   --frames N          frames simulated before the screenshot (default {DEFAULT_SCREENSHOT_FRAMES})\n\
         \x20   -h, --help          show this help\n\
         \n\
         CONTROLS (the game's own):\n\
         \x20   right-drag = pan (and deselect) · Ctrl + right-drag = slide towards the cursor\n\
         \x20   wheel or = / - = zoom · middle-drag = drag-zoom · PageUp / PageDown = camera speed\n\
         \x20   arrows / WASD = move (setting: Allow Arrow Key) · Home = origin and deselect\n\
         \x20   left click = select a bibite or egg · left-drag = rectangle selection\n\
         \x20   Esc = close the newest panel, then deselect, then the pause menu\n\
         \x20   space = pause · H = hide the UI · F5 = quick save\n\
         \x20   R / G / O / E = random / highest-generation / oldest bibite / random egg\n\
         \x20   1-4 = Stats / Genes / Biology / Brain panel of the selected bibite (again = close)\n\
         \x20   Insert = lay an egg · Delete = kill (a corpse: explode it into meat)\n\
         \x20   F12 = save the window (UI included) to screenshots/ next to the program\n\
         \n\
         EXAMPLES:\n\
         \x20   rustibites-view testdata/real/Default.zip\n\
         \x20   rustibites-view testdata/real/Default.zip --screenshot target/default-overview.png --frames 180\n\
         \x20   rustibites-view testdata/real/Default.zip --select-first --center-on-selected --ortho-size 30 --screenshot target/default-closeup.png\n\
         \x20   rustibites-view --screenshot target/viewer-screenshot.png --frames 180\n\
         \x20   rustibites-view testdata/real/Ring.zip            # historical fixture (opt-in)\n\
         \x20   rustibites-view testdata/real/saves/lol.zip --threads 8   # historical save (opt-in)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menus_are_painted_at_the_windows_own_pixel_size_up_to_a_cap() {
        // The launch size, a maximised / fullscreen window and an odd-shaped one: the window's own.
        assert_eq!(menu_pixels_for(1280.0, 720.0, 1.0), Some((1280, 720)));
        assert_eq!(menu_pixels_for(1920.0, 1080.0, 1.0), Some((1920, 1080)));
        assert_eq!(menu_pixels_for(1000.0, 800.0, 1.0), Some((1000, 800)));
        // A scaled display has more pixels than logical points.
        assert_eq!(menu_pixels_for(1280.0, 720.0, 1.5), Some((1920, 1080)));
        // A 4K window is painted at the cap, keeping its shape.
        assert_eq!(menu_pixels_for(3840.0, 2160.0, 1.0), Some((2560, 1440)));
        assert_eq!(menu_pixels_for(1920.0, 1080.0, 2.0), Some((2560, 1440)));
        let (w, h) = menu_pixels_for(3440.0, 1440.0, 1.0).expect("ultrawide");
        assert!(w <= 2560 && h <= 1440 && (w as f32 / h as f32 - 3440.0 / 1440.0).abs() < 0.01);
        // Minimised: nothing to paint into.
        assert_eq!(menu_pixels_for(0.0, 0.0, 1.0), None);
        assert_eq!(menu_pixels_for(120.0, 60.0, 1.0), None);
    }

    #[test]
    fn the_startup_line_and_the_switch_line_tell_how_the_ticks_run() {
        use bibites_core::pool_tuner::{Arm, PoolSwitch};
        let mut loaded = LoadedWorld::load(None, 12_345, 4, 1).expect("fallback world");
        assert!(threads_line(&loaded.world).contains("serial tick"));
        loaded.world.set_threads(4);
        let line = threads_line(&loaded.world);
        assert!(
            line.starts_with("threads: 4 workers") && line.contains("times its own ticks"),
            "{line}"
        );
        loaded
            .world
            .set_parallel_gates(bibites_core::sim::ParallelGates::ALWAYS);
        assert!(threads_line(&loaded.world).contains("RUSTIBITES_GATES=always"));
        let switch = PoolSwitch {
            from: Arm::Inline,
            to: Arm::Pooled(1),
            tick: 4_000,
            size: 1_500,
            from_ns: 1_300_000.0,
            to_ns: 1_100_000.0,
        };
        assert_eq!(
            pool_switch_line(&switch),
            "pool: ticks now run on the pool (level 1) (it was inline; tick 4000, world size 1500: 1.10 ms a tick now, 1.30 ms before)"
        );
        let back = PoolSwitch {
            from: Arm::Pooled(1),
            to: Arm::Inline,
            ..switch
        };
        assert!(pool_switch_line(&back).starts_with("pool: ticks now run inline (it was on the pool (level 1);"));
        // A world with nothing to report logs nothing, and the counter stays.
        let mut logged = 0;
        log_pool_switches(&mut loaded.world, &mut logged);
        assert_eq!(logged, 0);
    }

    #[test]
    fn ui_first_click_routes_to_hud_action_and_suppresses_simulation() {
        use bibites_core::math::Vec2 as CoreVec2;
        let loaded = LoadedWorld::load(None, 12_345, 4, 1).expect("fallback world");
        let mut app = App::new(loaded, CoreVec2::new(640.0, 360.0), None);
        let mut frame = ui::UiFrame::new(1.0);
        frame.hit(
            7,
            CoreVec2::new(0.0, 0.0),
            CoreVec2::new(100.0, 40.0),
            ui::UiAction::TogglePause,
        );
        let press = FrameInput {
            cursor: CoreVec2::new(20.0, 20.0),
            primary_pressed: true,
            ..FrameInput::default()
        };
        let routed = route_ui_frame(&mut app, &frame, press, None, 1280.0, 720.0);
        assert!(!routed.primary_pressed);
        let release = FrameInput {
            cursor: CoreVec2::new(20.0, 20.0),
            primary_released: true,
            ..FrameInput::default()
        };
        let routed = route_ui_frame(&mut app, &frame, release, None, 1280.0, 720.0);
        assert!(!routed.primary_released);
        assert!(app.paused, "UI pause action was not applied");
    }

    #[test]
    fn the_bibite_editor_works_through_the_windows_pointer_routing() {
        use bibites_core::math::Vec2 as CoreVec2;
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, CoreVec2::new(1920.0, 1080.0), None);
        let dir =
            std::env::temp_dir().join(format!("rustibites_editor_route_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        app.storage = bibites_io::storage::StoragePaths::new(dir.clone());
        app.select_first();
        app.open_template_editor_for_selection();
        let build = |app: &App| {
            let mut frame = ui::UiFrame::new(1.0);
            hud::draw_extra_panels(
                &mut frame,
                app,
                &ui::UiAssets::empty(),
                None,
                1920.0,
                1080.0,
                false,
            );
            frame
        };
        let centre_of = |frame: &ui::UiFrame, action: ui::UiAction| -> CoreVec2 {
            let id = frame
                .actions
                .iter()
                .find(|(_, candidate)| *candidate == action)
                .map(|(id, _)| *id)
                .expect("the control is drawn");
            let (_, pos, size) = frame
                .hits
                .iter()
                .find(|(candidate, _, _)| *candidate == id)
                .expect("its hit area");
            CoreVec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0)
        };
        let route = |app: &mut App, input: FrameInput| {
            let frame = build(app);
            route_ui_frame(app, &frame, input, None, 1920.0, 1080.0)
        };
        let diet = bibites_core::genome::GENE_DEFS
            .iter()
            .position(|def| def.name == "Diet")
            .expect("the Diet gene") as u8;
        let gene =
            |app: &App| app.bibite_editor.as_ref().expect("open").model.genes()[usize::from(diet)];
        // The slider's knob is pressed and dragged: the press does not reach the world, the value follows the cursor while the
        // button is held and stays where it was let go.
        let slide =
            ui::UiAction::Editor(template_editor_ui::EditorAction::GeneSlide(diet, 856, 1236));
        let knob = centre_of(&build(&app), slide);
        let routed = route(
            &mut app,
            FrameInput {
                cursor: knob,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(!routed.primary_pressed && app.editor_gene_dragging());
        let routed = route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(856.0 + 380.0 * 0.6, knob.y),
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(
            !routed.primary_down,
            "the held slider keeps the pointer from the world"
        );
        assert!((gene(&app) - 0.6).abs() < 1e-3, "{}", gene(&app));
        route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(856.0 + 380.0 * 0.6, knob.y),
                primary_released: true,
                ..FrameInput::default()
            },
        );
        assert!(!app.editor_gene_dragging());
        assert!((gene(&app) - 0.6).abs() < 1e-3);
        // A click on the row's reset button goes back to the default.
        let reset = centre_of(
            &build(&app),
            ui::UiAction::Editor(template_editor_ui::EditorAction::GeneReset(diet)),
        );
        for input in [
            FrameInput {
                cursor: reset,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
            FrameInput {
                cursor: reset,
                primary_released: true,
                ..FrameInput::default()
            },
        ] {
            route(&mut app, input);
        }
        assert_eq!(
            gene(&app),
            bibites_core::genome::GENE_DEFS[usize::from(diet)].default
        );
        // The wheel scrolls the list a row a notch and does not zoom the camera.
        let zoom = app.camera.zoom;
        let routed = route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(600.0, 500.0),
                wheel: -1.0,
                ..FrameInput::default()
            },
        );
        assert_eq!(routed.wheel, 0.0);
        assert_eq!(app.bibite_editor.as_ref().unwrap().view.scroll_rows, 1);
        assert_eq!(app.camera.zoom, zoom);
        // The close button closes it.
        let back = centre_of(
            &build(&app),
            ui::UiAction::Editor(template_editor_ui::EditorAction::Back),
        );
        for input in [
            FrameInput {
                cursor: back,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
            FrameInput {
                cursor: back,
                primary_released: true,
                ..FrameInput::default()
            },
        ] {
            route(&mut app, input);
        }
        assert!(app.bibite_editor.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_brain_tab_works_through_the_windows_pointer_routing() {
        use brain_editor_ui::{BrainAction, Canvas, Picked, Slider};
        use bibites_core::math::Vec2 as CoreVec2;
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, CoreVec2::new(1920.0, 1080.0), None);
        let dir =
            std::env::temp_dir().join(format!("rustibites_brain_route_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        app.storage = bibites_io::storage::StoragePaths::new(dir.clone());
        app.select_first();
        app.open_template_editor_for_selection();
        app.bibite_editor.as_mut().expect("open").view.tab = template_editor_ui::Tab::Brain;
        let build = |app: &App| {
            let mut frame = ui::UiFrame::new(1.0);
            hud::draw_extra_panels(
                &mut frame,
                app,
                &ui::UiAssets::empty(),
                None,
                1920.0,
                1080.0,
                false,
            );
            frame
        };
        let route = |app: &mut App, input: FrameInput| {
            let frame = build(app);
            route_ui_frame(app, &frame, input, None, 1920.0, 1080.0)
        };
        let brain = |app: &App| app.bibite_editor.as_ref().expect("open").brain.clone();
        let view = |app: &App| app.bibite_editor.as_ref().expect("open").view.brain.clone();
        // The wheel zooms the canvas about the cursor and does not zoom the camera.
        let camera_zoom = app.camera.zoom;
        let routed = route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(600.0, 600.0),
                wheel: 1.0,
                ..FrameInput::default()
            },
        );
        assert_eq!(routed.wheel, 0.0);
        assert!((view(&app).zoom - 1.1).abs() < 1e-5);
        assert_eq!(app.camera.zoom, camera_zoom);
        // A node on the screen that nothing covers: pressed and let go it is selected, and its panel opens.
        let canvas = Canvas::new(brain(&app).content, view(&app).zoom, view(&app).pan);
        let (key, at) = brain(&app)
            .nodes
            .iter()
            .filter_map(|node| {
                let at = canvas.to_screen(node.pos);
                (canvas.contains(at)
                    && brain_editor_ui::pick(&brain(&app), &canvas, at)
                        == Some(Picked::Node(node.key)))
                .then_some((node.key, at))
            })
            .next()
            .expect("a node shows");
        let point = CoreVec2::new(at[0], at[1]);
        for input in [
            FrameInput {
                cursor: point,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
            FrameInput {
                cursor: point,
                primary_released: true,
                ..FrameInput::default()
            },
        ] {
            let routed = route(&mut app, input);
            assert!(!routed.primary_pressed && !routed.primary_released);
        }
        assert_eq!(
            brain(&app).selected,
            Some(brain_editor::Selected::Node(key))
        );
        // The panel's slider is pressed and dragged: its value follows the cursor.
        let frame = build(&app);
        let (slider, x0, x1, centre) = frame
            .actions
            .iter()
            .find_map(|(id, action)| match action {
                ui::UiAction::Editor(template_editor_ui::EditorAction::Brain(
                    BrainAction::Slide(slider, x0, x1),
                )) => {
                    let (_, pos, size) = frame.hits.iter().find(|hit| hit.0 == *id)?;
                    Some((*slider, *x0, *x1, CoreVec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0)))
                }
                _ => None,
            })
            .expect("the open node has a slider");
        route(
            &mut app,
            FrameInput {
                cursor: centre,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.editor_brain_dragging(), Some(false));
        let target = f32::from(x0) + (f32::from(x1) - f32::from(x0)) * 0.75;
        let routed = route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(target, centre.y),
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(!routed.primary_down, "the held slider keeps the pointer from the world");
        let node = brain(&app).node(key).expect("node").clone();
        let (low, high, _) = brain_editor_ui::slider_range(slider, Some(&node));
        let expected = low + (high - low) * 0.75;
        let value = match slider {
            Slider::Bias => node.bias,
            Slider::Act => node.value,
            Slider::Weight => unreachable!("a node's panel"),
        };
        assert!((value - expected).abs() < 0.05 * (high - low).max(1.0), "{value} vs {expected}");
        route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(target, centre.y),
                primary_released: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.editor_brain_dragging(), None);
        // The right button pans the canvas and never the world's camera behind it.
        let canvas = Canvas::new(brain(&app).content, view(&app).zoom, view(&app).pan);
        let start = CoreVec2::new(canvas.viewport[0] + 8.0, canvas.viewport[1] + 8.0);
        let before = view(&app).pan;
        let camera_centre = app.camera.center;
        let routed = route(
            &mut app,
            FrameInput {
                cursor: start,
                secondary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(!routed.secondary_down, "the camera does not pan under the editor");
        for step in [20.0, 60.0] {
            route(
                &mut app,
                FrameInput {
                    cursor: CoreVec2::new(start.x + step, start.y + step / 2.0),
                    secondary_down: true,
                    ..FrameInput::default()
                },
            );
        }
        route(&mut app, FrameInput::default());
        assert_ne!(view(&app).pan, before, "the canvas moved");
        assert_eq!(app.camera.center, camera_centre);
        assert!(brain(&app).selected.is_none(), "moving the content takes the selection away");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_brain_tabs_panels_edit_through_the_windows_pointer_routing() {
        use bibites_core::brain::NodeType;
        use bibites_core::math::Vec2 as CoreVec2;
        use brain_editor_ui::{BrainAction, Canvas};
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, CoreVec2::new(1920.0, 1080.0), None);
        let dir =
            std::env::temp_dir().join(format!("rustibites_brain_panels_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        app.storage = bibites_io::storage::StoragePaths::new(dir.clone());
        app.open_editor_from_library(None, false);
        app.bibite_editor.as_mut().expect("open").view.tab = template_editor_ui::Tab::Brain;
        let build = |app: &App| {
            let mut frame = ui::UiFrame::new(1.0);
            hud::draw_extra_panels(
                &mut frame,
                app,
                &ui::UiAssets::empty(),
                None,
                1920.0,
                1080.0,
                false,
            );
            if let Some(open) = app.dropdown_open.as_ref() {
                dropdown::emit(
                    &mut frame,
                    &ui::UiAssets::empty(),
                    None,
                    open,
                    CoreVec2::new(1920.0, 1080.0),
                    app.cursor,
                );
            }
            frame
        };
        let centre_of = |frame: &ui::UiFrame, wanted: &dyn Fn(&ui::UiAction) -> bool| -> CoreVec2 {
            let id = frame
                .actions
                .iter()
                .find(|(_, action)| wanted(action))
                .map(|(id, _)| *id)
                .expect("the control is drawn");
            let (_, pos, size) = frame
                .hits
                .iter()
                .find(|(candidate, _, _)| *candidate == id)
                .expect("its hit area");
            CoreVec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0)
        };
        let click = |app: &mut App, at: CoreVec2| {
            for input in [
                FrameInput {
                    cursor: at,
                    primary_pressed: true,
                    primary_down: true,
                    ..FrameInput::default()
                },
                FrameInput {
                    cursor: at,
                    primary_released: true,
                    ..FrameInput::default()
                },
            ] {
                let frame = build(app);
                let routed = route_ui_frame(app, &frame, input, None, 1920.0, 1080.0);
                app.handle_input(&routed);
            }
        };
        let brain = |app: &App| app.bibite_editor.as_ref().expect("open").brain.clone();
        // A hidden sigmoid node on the canvas, selected, with its panel open.
        let canvas = {
            let session = app.bibite_editor.as_ref().expect("open");
            Canvas::new(session.brain.content, 1.0, [0.0, 0.0])
        };
        let place = canvas.to_place([500.0, 450.0]);
        {
            let session = app.bibite_editor.as_mut().expect("open");
            session.brain.request_add_node(place);
            session.brain.add_node(NodeType::Sigmoid as usize, bibites_brainsort::Archetype::Hidden);
            session.view.brain.panel_at = brain_editor_ui::to_editor_space([500.0, 450.0]);
        }
        let Some(brain_editor::Selected::Node(key)) = brain(&app).selected else {
            panic!("the node is selected");
        };
        // The Activation Function dropdown opens its list under the control and a pick changes the node's function.
        let open = centre_of(&build(&app), &|action| {
            *action == ui::UiAction::DropdownOpen(ui::DropdownTarget::NodeFunction)
        });
        click(&mut app, open);
        assert!(app.dropdown_open.is_some(), "the list is open");
        let tanh = brain_editor_ui::FUNCTION_CHOICES
            .iter()
            .position(|(kind, _)| *kind == NodeType::TanH)
            .expect("TanH is a choice") as u16;
        let row = centre_of(&build(&app), &|action| {
            *action == ui::UiAction::DropdownPick(tanh)
        });
        click(&mut app, row);
        assert!(app.dropdown_open.is_none());
        assert_eq!(brain(&app).node(key).expect("node").kind, NodeType::TanH);
        // The bias' value text opens its edit field; a typed number and Enter set the bias.
        let value = centre_of(&build(&app), &|action| {
            *action == ui::UiAction::DynEditValue(dyn_panel::FieldRef::BrainBias)
        });
        click(&mut app, value);
        assert!(app.value_edit.is_some(), "the edit field is open");
        app.typed = "2.5".to_owned();
        app.handle_input(&FrameInput::default());
        app.handle_input(&FrameInput {
            enter: true,
            ..FrameInput::default()
        });
        assert!(app.value_edit.is_none());
        assert_eq!(brain(&app).node(key).expect("node").bias, 2.5);
        // The panel's "add outgoing connection" button starts a connection that an output node's click ends.
        let out = centre_of(&build(&app), &|action| {
            *action
                == ui::UiAction::Editor(template_editor_ui::EditorAction::Brain(
                    BrainAction::AddConnection(true),
                ))
        });
        click(&mut app, out);
        assert!(brain(&app).connecting, "the connection waits for its end");
        let target = brain(&app)
            .nodes
            .iter()
            .find(|node| node.archetype() == bibites_brainsort::Archetype::Output)
            .map(|node| (node.key, canvas.to_screen(node.pos)))
            .expect("an output");
        // The new template's outputs are in the middle of the canvas, on the screen.
        assert!(canvas.contains(target.1), "the output shows");
        click(&mut app, CoreVec2::new(target.1[0], target.1[1]));
        let edit = brain(&app);
        assert!(!edit.connecting);
        assert_eq!(edit.edges.len(), 1, "the connection is made");
        assert!(edit.selected.is_none(), "and not selected");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_gene_analysis_page_works_through_the_windows_pointer_routing() {
        use bibites_core::math::Vec2 as CoreVec2;
        let loaded = LoadedWorld::load(None, 31, 8, 1).expect("starter world");
        let mut app = App::new(loaded, CoreVec2::new(1920.0, 1080.0), None);
        for hour in 1..=3 {
            let genes = vec![bibites_core::genome::Genome::defaults().genes; 5 * hour];
            app.world
                .data_logger
                .genes
                .push(bibites_core::genes_log::GenesBuckets::from_genes(&genes));
        }
        app.apply_ui_actions(&[ui::UiAction::OpenPanel(ui::PanelKind::Graphs)]);
        let build = |app: &App| {
            let mut frame = ui::UiFrame::new(1.0);
            hud::draw_extra_panels(
                &mut frame,
                app,
                &ui::UiAssets::empty(),
                None,
                1920.0,
                1080.0,
                false,
            );
            // The open list is drawn over everything by the window's frame builder.
            if let Some(open) = app.dropdown_open.as_ref() {
                dropdown::emit(
                    &mut frame,
                    &ui::UiAssets::empty(),
                    None,
                    open,
                    CoreVec2::new(1920.0, 1080.0),
                    app.cursor,
                );
            }
            frame
        };
        let centre_of = |frame: &ui::UiFrame, action: ui::UiAction| -> CoreVec2 {
            let id = frame
                .actions
                .iter()
                .find(|(_, candidate)| *candidate == action)
                .map(|(id, _)| *id)
                .expect("the control is drawn");
            let (_, pos, size) = frame
                .hits
                .iter()
                .find(|(candidate, _, _)| *candidate == id)
                .expect("its hit area");
            CoreVec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0)
        };
        let route = |app: &mut App, input: FrameInput| {
            let frame = build(app);
            route_ui_frame(app, &frame, input, None, 1920.0, 1080.0)
        };
        let click = |app: &mut App, at: CoreVec2| {
            route(
                app,
                FrameInput {
                    cursor: at,
                    primary_pressed: true,
                    primary_down: true,
                    ..FrameInput::default()
                },
            );
            route(
                app,
                FrameInput {
                    cursor: at,
                    primary_released: true,
                    ..FrameInput::default()
                },
            );
        };
        // "Go To Gene Analysis" opens the page.
        let to_genes = centre_of(&build(&app), ui::UiAction::GraphsPage(true));
        click(&mut app, to_genes);
        assert!(app.graphs_genes);
        // The gene box opens its list; a row picks a gene.
        let gene_box = centre_of(
            &build(&app),
            ui::UiAction::DropdownOpen(ui::DropdownTarget::GraphsGene),
        );
        click(&mut app, gene_box);
        assert!(app.dropdown_open.is_some());
        let row = centre_of(&build(&app), ui::UiAction::DropdownPick(4));
        app.cursor = row;
        click(&mut app, row);
        assert!(app.dropdown_open.is_none());
        assert_eq!(app.graphs_gene, 4);
        // The slider's handle (at "Now", the right end) is pressed and dragged left: the press does not reach the world, the drag
        // goes back in time while it is held and the value stays where it was let go.
        let handle = centre_of(&build(&app), ui::UiAction::GenesTimeSlideHandle(250, 1670));
        let routed = route(
            &mut app,
            FrameInput {
                cursor: handle,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(!routed.primary_pressed && app.graphs_gene_drag.is_some());
        let routed = route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(250.0, handle.y),
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(
            !routed.primary_down,
            "the held slider keeps the pointer from the world"
        );
        assert_eq!(app.graphs_gene_time_value(), 2);
        route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(250.0, handle.y),
                primary_released: true,
                ..FrameInput::default()
            },
        );
        assert!(app.graphs_gene_drag.is_none());
        assert_eq!(app.graphs_gene_time_value(), 2);
        // A press on the track jumps there.
        let track_right = CoreVec2::new(1600.0, handle.y);
        click(&mut app, track_right);
        assert_eq!(app.graphs_gene_time_value(), -1);
        // And "Go To Data Graphs" is the way back.
        let back = centre_of(&build(&app), ui::UiAction::GraphsPage(false));
        click(&mut app, back);
        assert!(!app.graphs_genes);
    }

    #[test]
    fn the_wheel_is_the_cameras_where_the_information_panel_was_while_the_hud_is_hidden() {
        use bibites_core::math::Vec2 as CoreVec2;
        let loaded = LoadedWorld::load(None, 43, 10, 1).expect("starter world");
        let mut app = App::new(loaded, CoreVec2::new(1920.0, 1080.0), None);
        // One species and a tag of their own for every bibite, as the panel's own test does.
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let species = app.world.species.add_template_species("Zzone zzspecies", "", &genome, &brain);
        for (index, id) in ids.iter().enumerate() {
            let record = app.world.bibites.get_mut(*id).expect("record");
            record.meta.species = Some(species);
            record.meta.species_tag = Some(format!("tag {index}"));
        }
        app.apply_ui_actions(&[ui::UiAction::OpenPanel(ui::PanelKind::Information)]);
        let mut frame = ui::UiFrame::new(1.0);
        hud::draw_extra_panels(&mut frame, &app, &ui::UiAssets::empty(), None, 1920.0, 1080.0, false);
        let list = {
            let id = frame
                .actions
                .iter()
                .find(|(_, action)| *action == ui::UiAction::Invoke(ui::ControlCommand::InformationSelectTag(1)))
                .map(|(id, _)| *id)
                .expect("a tag row is drawn");
            let (_, pos, size) = frame.hits.iter().find(|hit| hit.0 == id).expect("its hit area");
            CoreVec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0)
        };
        let wheel_at = |app: &mut App, frame: &ui::UiFrame| {
            let input = FrameInput { cursor: list, wheel: -1.0, ..FrameInput::default() };
            route_ui_frame(app, frame, input, None, 1920.0, 1080.0).wheel
        };
        assert_eq!(wheel_at(&mut app, &frame), 0.0, "over the list the wheel scrolls it");
        // H hides the whole HUD: the window's frame is empty, nothing of the panel is there, and the camera zooms over the place where its list was.
        app.show_hud = false;
        assert_eq!(wheel_at(&mut app, &ui::UiFrame::new(1.0)), -1.0);
    }

    #[test]
    fn the_menu_behind_an_in_game_panel_lives_while_the_panel_is_on_the_stack() {
        use bibites_core::math::Vec2 as CoreVec2;
        let loaded = LoadedWorld::load(None, 51, 4, 1).expect("starter world");
        let mut app = App::new(loaded, CoreVec2::new(1920.0, 1080.0), None);
        assert!(!overlay_panel_open(&app));
        for panel in OVERLAY_PANELS {
            assert!(overlay_view_of(Some(panel)).is_some(), "{panel:?} replays a title-menu screen");
            app.panels.open(panel);
            assert!(overlay_panel_open(&app), "{panel:?}");
            assert!(overlay_view_of(app.panels.top()).is_some());
            // Under another panel (the Bibite Editor opens over the Library) it is still on the stack, and its menu is kept.
            app.panels.open(ui::PanelKind::BibiteEditor);
            assert!(overlay_panel_open(&app), "{panel:?} under another panel");
            assert!(overlay_view_of(app.panels.top()).is_none());
            app.panels.close(ui::PanelKind::BibiteEditor);
            // Escape closes it: no panel of the kind is left, and the menu that served it goes.
            assert_eq!(app.panels.pop_escape(), Some(panel));
            assert!(!overlay_panel_open(&app), "{panel:?} closed");
        }
    }

    #[test]
    fn the_information_panel_takes_the_wheel_the_scroll_bar_and_ctrl_clicks() {
        use bibites_core::math::Vec2 as CoreVec2;
        let loaded = LoadedWorld::load(None, 41, 10, 1).expect("starter world");
        let mut app = App::new(loaded, CoreVec2::new(1920.0, 1080.0), None);
        // One species and a tag of their own for every bibite: ten tags and the untagged ones.
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let species =
            app.world
                .species
                .add_template_species("Zzone zzspecies", "", &genome, &brain);
        for (index, id) in ids.iter().enumerate() {
            let record = app.world.bibites.get_mut(*id).expect("record");
            record.meta.species = Some(species);
            record.meta.species_tag = Some(format!("tag {index}"));
        }
        app.apply_ui_actions(&[ui::UiAction::OpenPanel(ui::PanelKind::Information)]);
        let build = |app: &App| {
            let mut frame = ui::UiFrame::new(1.0);
            hud::draw_extra_panels(
                &mut frame,
                app,
                &ui::UiAssets::empty(),
                None,
                1920.0,
                1080.0,
                false,
            );
            frame
        };
        let centre_of = |frame: &ui::UiFrame, action: ui::UiAction| -> CoreVec2 {
            let id = frame
                .actions
                .iter()
                .find(|(_, candidate)| *candidate == action)
                .map(|(id, _)| *id)
                .expect("the control is drawn");
            let (_, pos, size) = frame
                .hits
                .iter()
                .find(|(candidate, _, _)| *candidate == id)
                .expect("its hit area");
            CoreVec2::new(pos.x + size.x / 2.0, pos.y + size.y / 2.0)
        };
        let route = |app: &mut App, input: FrameInput| {
            let frame = build(app);
            route_ui_frame(app, &frame, input, None, 1920.0, 1080.0)
        };

        // The wheel over the tags list scrolls it, and the camera does not zoom.
        let frame = build(&app);
        let row = centre_of(
            &frame,
            ui::UiAction::Invoke(ui::ControlCommand::InformationSelectTag(1)),
        );
        let routed = route(
            &mut app,
            FrameInput {
                cursor: row,
                wheel: -1.0,
                ..FrameInput::default()
            },
        );
        assert_eq!(routed.wheel, 0.0);
        assert!(
            (app.information.scroll() - 35.0).abs() < 1e-3,
            "{}",
            app.information.scroll()
        );
        // Over the world it is the camera's.
        let routed = route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(300.0, 500.0),
                wheel: 1.0,
                ..FrameInput::default()
            },
        );
        assert_eq!(routed.wheel, 1.0);

        // The handle follows the cursor while it is held, and nothing under it is pressed.
        let frame = build(&app);
        let handle = centre_of(&frame, ui::UiAction::InformationScroll(true));
        let routed = route(
            &mut app,
            FrameInput {
                cursor: handle,
                primary_pressed: true,
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(app.information.bar_dragging());
        assert!(!routed.primary_pressed && !routed.primary_down);
        let before = app.information.scroll();
        route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(handle.x, handle.y + 30.0),
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(app.information.scroll() > before, "the handle moved down");
        route(
            &mut app,
            FrameInput {
                cursor: CoreVec2::new(handle.x, handle.y + 30.0),
                primary_released: true,
                ..FrameInput::default()
            },
        );
        assert!(!app.information.bar_dragging());

        // Ctrl held on a species row: the pick repeats once its target is gone.
        let frame = build(&app);
        let row = centre_of(
            &frame,
            ui::UiAction::Invoke(ui::ControlCommand::InformationSelectSpecies(0)),
        );
        for (pressed, released) in [(true, false), (false, true)] {
            route(
                &mut app,
                FrameInput {
                    cursor: row,
                    primary_pressed: pressed,
                    primary_down: pressed,
                    primary_released: released,
                    ctrl_down: true,
                    ..FrameInput::default()
                },
            );
        }
        assert!(
            app.selected.is_some(),
            "a member of the species is selected"
        );
        assert_eq!(app.reselect, Some(app::Reselect::Species(species)));
    }

    #[test]
    fn gpu_hue_shader_flag_is_on_by_default_and_can_be_switched_off() {
        // Without the material the window recolours the sprites on the processor (it once multiplied the blue art with the colour: "the basic bibite looks blue").
        let default = parse_args(std::iter::empty()).expect("default args");
        assert!(default.gpu_hue_shader);
        let flagged =
            parse_args(["--gpu-hue-shader".to_owned()].into_iter()).expect("flagged args");
        assert!(flagged.gpu_hue_shader);
        let off = parse_args(["--no-gpu-hue-shader".to_owned()].into_iter()).expect("off args");
        assert!(!off.gpu_hue_shader);
    }

    #[test]
    fn screenshot_hash_is_unchanged_by_gpu_hue_flag() {
        // `--gpu-hue-shader` is a window-only material; screenshots must always
        // use the deterministic software rasteriser. Render the same fallback
        // world twice (flag off/on) and require byte-identical PNGs.
        let dir =
            std::env::temp_dir().join(format!("rustibites-gpu-hue-flag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let mut hashes = Vec::new();
        for (enabled, name) in [(false, "off.png"), (true, "on.png")] {
            let mut args = Args {
                no_assets: true,
                simple_hud: true,
                gpu_hue_shader: enabled,
                width: 160,
                height: 120,
                frames: 0,
                bibites: 4,
                seed: 12_345,
                threads: 1,
                ..Args::default()
            };
            let path = dir.join(name);
            args.screenshot = Some(path.clone());
            let loaded =
                LoadedWorld::load(args.input.as_deref(), args.seed, args.bibites, args.threads)
                    .expect("load fallback world");
            let assets = load_assets(&args);
            let themed = load_themed_ui(&args);
            run_screenshot(loaded, None, &args, &path, assets, themed).expect("screenshot");
            let bytes = std::fs::read(&path).expect("read screenshot");
            hashes.push(bytes);
        }
        assert_eq!(hashes[0], hashes[1], "GPU flag changed screenshot bytes");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ui_session_args_parse_repeatable_and_comma_lists() {
        let args = parse_args(
            [
                "--ui-session",
                "workshop_panel,challenge_selector",
                "--ui-session",
                "lineage_species_info",
                "--ui-session-list",
                "--ui-only",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .expect("parse");
        assert_eq!(
            args.ui_session,
            vec![
                "workshop_panel".to_owned(),
                "challenge_selector".to_owned(),
                "lineage_species_info".to_owned()
            ]
        );
        assert!(args.ui_session_list);
        assert!(args.ui_only);
    }

    #[test]
    fn ui_only_session_render_is_transparent_and_1920x1080() {
        let Ok(Some(ui)) = ui::UiAssets::discover() else {
            return; // extracted game assets are local-only
        };
        let Ok(Some(runtime)) = ui_runtime::RuntimeUi::discover() else {
            return;
        };
        let names = runtime.session_names();
        let Some(name) = names
            .iter()
            .find(|name| name.contains("workshop_panel"))
            .or_else(|| names.first())
        else {
            return;
        };
        let path = std::env::temp_dir().join(format!(
            "rustibites-ui-session-{}-{name}.png",
            std::process::id()
        ));
        let args = Args {
            ui_session: vec![name.clone()],
            ui_only: true,
            screenshot: Some(path.clone()),
            ..Args::default()
        };
        let _ = ui;
        run_ui_sessions(&args).expect("ui session render");
        let image = load_reference(&path).expect("load render");
        assert_eq!((image.width, image.height), (1920, 1080));
        let alpha = |x: u32, y: u32| image.rgba[((y * image.width + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0, "corner should be transparent for UI-only");
        assert_eq!(
            alpha(1919, 1079),
            0,
            "corner should be transparent for UI-only"
        );
        assert!(
            image.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0),
            "overlay has no visible UI"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn full_screen_backdrop_detection() {
        use ui::UiDraw;
        let backdrop = UiDraw::Rect {
            pos: Vec2::ZERO,
            size: Vec2::new(1920.0, 1080.0),
            color: [0, 0, 0, 255],
        };
        assert!(is_full_screen_ui_draw(&backdrop, 1920.0, 1080.0));
        let panel = UiDraw::Rect {
            pos: Vec2::ZERO,
            size: Vec2::new(800.0, 600.0),
            color: [0, 0, 0, 255],
        };
        assert!(!is_full_screen_ui_draw(&panel, 1920.0, 1080.0));
    }

    #[test]
    fn native_menu_hit_rects_and_deterministic_hash() {
        let mut image = render::Image {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        assert_eq!(menu_image_hash(&image), menu_image_hash(&image));
        image.rgba[0] = 9;
        assert_ne!(
            menu_image_hash(&image),
            menu_image_hash(&render::Image {
                width: 2,
                height: 1,
                rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
            })
        );
        let Ok(mut runtime) = ui_runtime::RuntimeUi::load_native(Path::new(NATIVE_ATLAS)) else {
            return; // native dump is local-only
        };
        runtime.adopt_native_fixed_sessions();
        let rows = runtime.menu_button_rects(1920.0, 1080.0);
        assert_eq!(rows.len(), 9, "native title rows");
        let (pos, size) = rows[1];
        assert!(
            pos.x > 1200.0,
            "native title panel is right-aligned: {pos:?}"
        );
        let point = Vec2::new(pos.x + size.x * 0.5, pos.y + size.y * 0.5);
        let hit = rows.iter().position(|(pos, size)| {
            point.x >= pos.x
                && point.y >= pos.y
                && point.x < pos.x + size.x
                && point.y < pos.y + size.y
        });
        assert_eq!(hit, Some(1));
    }
}
