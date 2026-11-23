//! Game-style title/main menu for the interactive viewer.
//!
//! The game's captured `main_menu` runtime session is still being prepared by
//! parity; until it lands this module builds a curated menu from the extracted
//! game fonts and the same UI draw list used by the HUD. It is deliberately
//! backend-agnostic: the window uploads the software-rendered frame as a
//! full-screen texture, while `--menu --screenshot` writes the same frame
//! deterministically for tests.

use std::path::{Path, PathBuf};

use bibites_core::math::Vec2;

use crate::ui::{FontRole, FontSet, UiAssets, UiFrame};
use crate::ui_runtime::RuntimeUi;

mod editor;
mod tagline;

/// Menu screen currently shown (mirrors the game's `MenuInitializer` views).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuView {
    /// Title + main buttons (Continue/New/Load/Library/Settings/...).
    Main,
    /// Scenario picker for a new world.
    NewGame,
    /// Save picker (`*.zip` under the saves directory).
    LoadGame,
    /// Options bound to scene visibility + simulation TPS.
    Settings,
    /// Bibite template library (local `*.bb8template` files).
    Library,
    /// Local achievement gallery (informational for now).
    Achievements,
    /// Additional tools / community links.
    Tools,
    /// Credits.
    Credits,
    /// The scenario editor (opened from the New Game screen).
    ScenarioEditor,
    /// Loading transition while a world opens.
    Loading,
}

/// What the Library screen is being used for besides browsing
/// (`BibiteTemplateSelectorPanel.OpenWithCallBacks`: the submit button hands the template to a
/// caller instead of starting something).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplatePick {
    /// A template for a new default bibite (`DefaultBibitesPanel.AddBibiteButtonPressed`).
    DefaultBibite,
    /// The template to place bibites from (`BibitePlacer.ButtonFromTemplatePressed`).
    PlaceBibite,
    /// The champion of a challenge (`ScenarioSelectorPanel.SelectChampion`).
    Champion,
}

/// Result of activating the selected menu item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Nothing to do (view changed, selection updated, ...).
    None,
    /// The template file picked in the Library for the world's default bibites.
    PickedTemplate(PathBuf),
    /// The Library opens the Bibite Editor (`EditSelectedBibite`: on the selected template, `OpenEditorForNewBibite`: on a new one,
    /// `path` is `None`). `submit` is whether the editor has a "Save and submit" button (`EditTemplate(template, AfterEditorSubmit)`).
    EditTemplate {
        /// The template or saved bibite to edit.
        path: Option<PathBuf>,
        /// The editor offers "Save and submit".
        submit: bool,
    },
    /// Load this save/scenario and enter the world.
    Start(PathBuf),
    /// Close the viewer.
    Quit,
}

/// Options edited in the Settings view and applied when a world opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuSettings {
    /// Background grid.
    pub grid: bool,
    /// Zone fills.
    pub zones: bool,
    /// Pellets.
    pub pellets: bool,
    /// Pheromone markers.
    pub pheromones: bool,
    /// HUD overlay.
    pub hud: bool,
    /// Simulation ticks per second (new worlds).
    pub sim_tps: u32,
}

impl Default for MenuSettings {
    fn default() -> Self {
        Self {
            grid: true,
            zones: true,
            pellets: true,
            pheromones: false,
            hud: true,
            sim_tps: 30,
        }
    }
}

/// UI panel/modal state machine shared by the menu and in-game UI.
#[derive(Clone, Debug, Default)]
pub struct PanelState {
    /// Open panels, bottom to top.
    open: Vec<crate::ui::PanelKind>,
    /// Current modal (blocks input to lower layers).
    modal: Option<crate::ui::PanelKind>,
    /// Scroll offset of the top panel.
    scroll: Vec2,
}

impl PanelState {
    /// Opens (or focuses) a panel. The three right-hand panels (statistics, dynamic settings,
    /// ancestral lineage) replace each other, as in `RightmostPanelsManager.OpenPanel`.
    pub fn open(&mut self, panel: crate::ui::PanelKind) {
        use crate::ui::PanelKind;
        const RIGHTMOST: [PanelKind; 3] = [
            PanelKind::Graphs,
            PanelKind::WorldDynamics,
            PanelKind::Lineage,
        ];
        if RIGHTMOST.contains(&panel) {
            self.open
                .retain(|kind| *kind == panel || !RIGHTMOST.contains(kind));
        }
        self.open.retain(|kind| *kind != panel);
        self.open.push(panel);
        self.scroll = Vec2::ZERO;
    }

    /// Closes a specific panel if open.
    pub fn close(&mut self, panel: crate::ui::PanelKind) {
        self.open.retain(|kind| *kind != panel);
        if self.modal == Some(panel) {
            self.modal = None;
        }
    }

    /// Toggles a panel.
    pub fn toggle(&mut self, panel: crate::ui::PanelKind) {
        if self.is_open(panel) {
            self.close(panel);
        } else {
            self.open(panel);
        }
    }

    /// The place of a panel's entry in the Escape stack (0 is the oldest).
    #[must_use]
    pub fn position(&self, panel: crate::ui::PanelKind) -> Option<usize> {
        self.open.iter().position(|kind| *kind == panel)
    }

    /// Whether a panel is open.
    #[must_use]
    pub fn is_open(&self, panel: crate::ui::PanelKind) -> bool {
        self.open.contains(&panel)
    }

    /// Topmost open panel (the Escape-stack markers are not panels).
    #[must_use]
    #[allow(dead_code)] // TODO(bind): panel rendering/scroll wiring
    pub fn top(&self) -> Option<crate::ui::PanelKind> {
        self.modal.or_else(|| {
            self.open
                .iter()
                .rev()
                .find(|kind| !kind.is_escape_marker())
                .copied()
        })
    }

    /// Closes the topmost panel/modal; returns `false` when no panel was open. The Escape-stack
    /// markers stay (a panel's close button never deselects the target), and so do the entries of the placers and towers' panels: the
    /// settings panel's X closes the settings panel even when the pellet placer was opened after it (the entry would only come back, `sync_escape_stack`).
    pub fn close_top(&mut self) -> bool {
        if self.modal.take().is_some() {
            return true;
        }
        match self.open.iter().rposition(|kind| !kind.is_escape_marker() && !kind.is_tool_entry()) {
            Some(index) => {
                self.open.remove(index);
                true
            }
            None => false,
        }
    }

    /// `UINavigationManager.emptyEscapeStack`: nothing is open (the markers count).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modal.is_none() && self.open.is_empty()
    }

    /// Adds `panel` on top unless it is already on the stack (`UIPanel.OpenPanel` does nothing for
    /// an open panel, so it keeps its place).
    pub fn open_once(&mut self, panel: crate::ui::PanelKind) {
        if !self.open.contains(&panel) {
            self.open.push(panel);
        }
    }

    /// `UINavigationManager.Update` on Escape: pops the newest entry (the modal first) and returns
    /// it, so the caller can run what leaving it means; `None` when the stack is empty.
    pub fn pop_escape(&mut self) -> Option<crate::ui::PanelKind> {
        if let Some(modal) = self.modal.take() {
            return Some(modal);
        }
        self.open.pop()
    }

    /// Escape handling: closes modal/top entry; `false` if the stack was empty.
    #[allow(dead_code)] // TODO(bind): panel rendering/scroll wiring
    pub fn escape(&mut self) -> bool {
        self.pop_escape().is_some()
    }

    /// Sets/clears a modal.
    #[allow(dead_code)] // TODO(bind): panel/modal rendering uses this next
    pub fn set_modal(&mut self, modal: Option<crate::ui::PanelKind>) {
        self.modal = modal;
    }

    /// Current modal.
    #[allow(dead_code)] // TODO(bind): panel/modal rendering uses this next
    #[must_use]
    pub fn modal(&self) -> Option<crate::ui::PanelKind> {
        self.modal
    }

    /// Scroll offset of the top panel.
    #[must_use]
    #[allow(dead_code)] // TODO(bind): panel rendering/scroll wiring
    pub fn scroll(&self) -> Vec2 {
        self.scroll
    }

    /// Sets the scroll offset of the top panel.
    #[allow(dead_code)] // TODO(bind): panel rendering/scroll wiring
    pub fn set_scroll(&mut self, scroll: Vec2) {
        self.scroll = scroll;
    }
}

/// A slider of the title menu (or of the pause menu's Settings) that the button is holding: the pointer moves it until the button is
/// let go, wherever the pointer goes (`Slider.OnDrag`; a Settings page's `ScrollRect` does not take the drag from it).
#[derive(Clone, Copy, Debug, PartialEq)]
enum HeldSlider {
    /// Row `.0` of the Settings pages (`DEFS`); its track spans x `.1..=.2` (menu pixels), the pointer took the handle `.3` px off its
    /// middle (0 when the press was on the track, which moved the handle under the pointer).
    Setting(u16, u16, u16, f32),
    /// TPS slider `.0` of the New Game panel; its track spans x `.1..=.2` (menu pixels), grabbed `.3` px off the handle's middle.
    Tps(u8, u16, u16, f32),
}

/// Half the width of a slider's handle (`base_circle_icon`, 30 x 30 reference px): a press this close to the handle's middle is on it.
const SLIDER_HANDLE_HALF: f32 = 15.0;

/// How far off the handle's middle (`handle`, menu pixels) a press at `x` took a slider: on the handle the handle stays where the pointer found
/// it (`Slider.OnPointerDown` keeps the offset), anywhere else on the track it is 0 and the handle jumps under the pointer. `scale` is the
/// menu's pixels per reference pixel.
pub(crate) fn slider_grab(handle: f32, x: f32, scale: f32) -> f32 {
    slider_grab_on(handle, x, scale).unwrap_or(0.0)
}

/// [`slider_grab`], or `None` when the press is not on the handle at all (it is on the track).
pub(crate) fn slider_grab_on(handle: f32, x: f32, scale: f32) -> Option<f32> {
    let off = x - handle;
    (off.abs() <= SLIDER_HANDLE_HALF * scale).then_some(off)
}

/// Interactive menu state (also used by the deterministic screenshot path).
#[derive(Clone, Debug)]
pub struct MenuState {
    view: MenuView,
    selected: usize,
    saves: Vec<PathBuf>,
    saves_dir: PathBuf,
    scenarios: Vec<PathBuf>,
    status: String,
    pending: Option<PathBuf>,
    /// Whether `pending` is a saved game (Continue, Load Game) and not a new scenario: only a loaded game honours *Pause After Load*.
    pending_save: bool,
    /// The champion picked for the challenge list (`ScenarioSelectorPanel.selectedChampion`).
    champion: Option<crate::challenge_run::Champion>,
    /// What the pending world is, when it is a challenge scenario (`StartChallenge`).
    pending_challenge: Option<crate::challenge_run::ChallengeStart>,
    /// The champion's bibite in its 110 x 110 box (`ChampionPreview/BibitePreview`) and what it was made for (file, scale).
    champion_picture: Option<std::sync::Arc<crate::ui::UiTexture>>,
    champion_picture_for: Option<(PathBuf, i64)>,
    settings: MenuSettings,
    settings_path: Option<PathBuf>,
    templates: Vec<bibites_io::storage::LibraryEntry>,
    library_selected: Option<usize>,
    /// The genes of the templates read for the Library's preview, by file (a file that does not read is `None`).
    library_genomes: std::collections::HashMap<PathBuf, Option<bibites_core::genome::Genome>>,
    /// The picture of the selected template's bibite (the Library's `BibitePreview`), made by the window loop
    /// (it has the game's sprites).
    library_picture: Option<std::sync::Arc<crate::ui::UiTexture>>,

    /// The data folder the Library lists (`Bibites/Templates`, `Bibites`) and "Open folder" shows: the one this port saves to ([`library_roots`]).
    library_root: Option<PathBuf>,
    /// The other data folders the Library lists after it (the installed game's, when the port saves to a folder of its own: the alpha's `Play.bat`).
    library_extra: Vec<PathBuf>,
    /// The Library's tab: 0 Templates, 1 Saved (`BibiteTemplateSelectorPanel.OpenTab`).
    library_tab: u8,
    /// The Library's delete warning (`fileDeletionWarning`) is up.
    confirm_library_delete: bool,
    /// The Library is open to pick a template for something else.
    pick: Option<TemplatePick>,
    /// The screen to go back to when the pick ends (the scenario editor).
    pick_return: MenuView,
    pending_template: Option<PathBuf>,
    newgame_scenario: Option<usize>,
    newgame_options: [usize; 5],
    scenario_entries: Vec<ScenarioEntry>,
    showing_challenges: bool,
    selected_entry: Option<usize>,
    /// The open dropdown list (`TMP_Dropdown.Show`): a New Game modifier, a Settings row, or a
    /// zone / default bibite field of the scenario editor.
    dropdown: Option<crate::dropdown::Open>,
    /// The control the last click landed on (reference px: `x, y, w, h`).
    click_rect: [f32; 4],
    /// Ctrl is held (the scenario editor's curve graph adds and removes points with it).
    pub ctrl_down: bool,
    /// Shift is held (a press in a text field of the scenario editor extends the selection).
    pub shift_down: bool,
    /// The pointer (screen px), for the open list's hover row.
    cursor: Vec2,
    data_roots: Vec<PathBuf>,
    challenge_best: Option<(f32, u8, String)>,
    include_autosaves: bool,
    selected_save: Option<usize>,
    selected_preview: Option<std::sync::Arc<crate::ui::UiTexture>>,
    /// How far each list and tab is scrolled (reference px, by [`scroll_index`]): a `ScrollRect` keeps its place while the title scene lives, and one
    /// list's place is not another's.
    scrolls: [f32; SCROLL_VIEWS],
    /// The elastic pull-back's speed (`SmoothDamp`'s velocity).
    scroll_speed: f32,
    /// The Load Game list's scrollbar handle is held: where the pointer took it (reference px under the handle's top).
    bar_grab: Option<f32>,
    /// How the current screen's scroll view differs from the capture's (1080p) in the canvas the screen is drawn in: its left and top edge moved,
    /// its width and height changed (`[dx, dy, dw, dh]`, reference px; set when the screen's frame is built, zero in a canvas of the capture's size).
    list_delta: std::cell::Cell<[f32; 4]>,
    /// The same for the New Game screen's parameters panel, which the TPS settings popup covers (`[dx, dy, dw, dh]`).
    tps_delta: std::cell::Cell<[f32; 4]>,
    /// A press on a scrolling screen waits for its release: where it began, the scroll then, whether it became a drag of the content.
    content_drag: Option<(Vec2, f32, bool)>,
    /// A slider of a Settings page or of the New Game TPS panel is held: the pointer moves it until the button is let go.
    slider_drag: Option<HeldSlider>,
    /// The speed the list keeps after a drag (`ScrollRect.inertia`, `decelerationRate` 0.135 a second).
    inertia: f32,
    /// Modifiers the user changed (only those override the scenario's own values).
    newgame_dirty: [bool; 5],
    save_dates: Vec<String>,
    save_card: Option<bibites_io::storage::SaveCard>,
    save_preview: Option<std::sync::Arc<crate::ui::UiTexture>>,
    confirm_delete: bool,
    /// The error popups waiting (`PopupManager.DisplayError`: source, text), the first one showing.
    errors: Vec<(String, String)>,
    /// The check of the Load list's saves that runs beside the window ([`MenuState::report_corrupted_saves`]).
    save_check: SaveCheckSlot,
    /// The "do not show again" checkbox of the warning popup that is up is ticked
    /// (`WarningHandle.doNotShowAgain.isOn`; every popup starts unticked).
    confirm_checked: bool,
    settings_tab: u8,
    user_settings: crate::user_settings::UserSettings,
    /// The "TPS settings" popup of the New Game screen is showing.
    tps_open: bool,
    /// Drawn over a running world (in-game Settings/Library/Achievements/Tools/Credits
    /// panels reuse the title-menu screens): dim backdrop, no world-starting actions.
    overlay: bool,
    /// Simulation TPS, brain / decay update factors, vision lookup / sensing factors.
    tps_values: [i32; 5],
    /// The scenario draft while the scenario editor is open.
    editor: Option<Box<crate::scenario_editor::EditorState>>,
    /// The animated title video is playing behind the screens that show the background
    /// (`menu_video`): their frames then leave the background out.
    video_background: bool,
}

/// `ScenarioIndependentSettings` keys behind the five TPS sliders.
pub const TPS_SETTING_KEYS: [&str; 5] = [
    "simTPS",
    "brainTPS",
    "decayTPS",
    "visionLookupUpdateFactor",
    "visionSenseUpdateFactor",
];
/// Slider labels, units and ranges (`ScenarioIndependentSettings`).
pub const TPS_SLIDERS: [(&str, &str, i32, i32); 5] = [
    ("Ticks Per Second", " tps", 5, 100),
    ("Brain Update Factor", " X", 1, 100),
    ("Decay Update Factor", " X", 1, 100),
    ("Vision Lookup Factor", " X", 1, 100),
    ("Vision Sensing Factor", " X", 1, 50),
];

impl MenuState {
    /// Builds the initial menu for a saves directory and canonical scenario.
    #[must_use]
    pub fn new(default_scenario: Option<PathBuf>, saves_dir: &Path) -> Self {
        let saves = scan_saves(saves_dir);
        let default_scenario = default_scenario.filter(|path| path.is_file());
        let status = match (&default_scenario, saves.first()) {
            (Some(_), _) => String::new(),
            (None, Some(_)) => "Default scenario not found; choose a save".to_owned(),
            (None, None) => "No Default.zip or saves found".to_owned(),
        };
        let scenarios = default_scenario
            .as_deref()
            .and_then(Path::parent)
            .map_or_else(Vec::new, scan_scenarios);
        let user_settings = crate::user_settings::UserSettings::load(Some(
            bibites_io::storage::StoragePaths::default_local()
                .root
                .join("game_user_settings.json"),
        ));
        let mut library_folders = library_roots().into_iter();
        let library_root = library_folders.next();
        let library_extra: Vec<PathBuf> = library_folders.collect();
        let templates = list_library_roots(library_root.as_deref(), &library_extra);
        // `FillLists` selects the first item.
        let library_selected = (!templates.is_empty()).then_some(0);
        Self {
            view: MenuView::Main,
            selected: 0,
            saves,
            saves_dir: saves_dir.to_path_buf(),
            scenarios,
            status,
            pending: None,
            pending_save: false,
            champion: None,
            pending_challenge: None,
            champion_picture: None,
            champion_picture_for: None,
            settings: MenuSettings::default(),
            settings_path: None,
            templates,
            library_selected,
            library_genomes: std::collections::HashMap::new(),
            library_picture: None,
            library_root,
            library_extra,
            library_tab: 0,
            confirm_library_delete: false,
            pick: None,
            pick_return: MenuView::Main,
            pending_template: None,
            newgame_scenario: None,
            newgame_options: [2, 4, 3, 3, 4],
            scenario_entries: Vec::new(),
            showing_challenges: false,
            selected_entry: None,
            dropdown: None,
            click_rect: [0.0; 4],
            ctrl_down: false,
            shift_down: false,
            cursor: Vec2::ZERO,
            data_roots: Vec::new(),
            challenge_best: None,
            include_autosaves: user_settings.flag("Include autosaves"),
            selected_save: None,
            selected_preview: None,
            scroll_speed: 0.0,
            scrolls: {
                let mut scrolls = [0.0; SCROLL_VIEWS];
                scrolls[scroll_index(MenuView::Credits)] = CREDITS_START_SCROLL;
                scrolls
            },
            bar_grab: None,
            list_delta: std::cell::Cell::new([0.0; 4]),
            tps_delta: std::cell::Cell::new([0.0; 4]),
            content_drag: None,
            slider_drag: None,
            inertia: 0.0,
            newgame_dirty: [true; 5],
            save_dates: Vec::new(),
            save_card: None,
            save_preview: None,
            confirm_delete: false,
            errors: Vec::new(),
            save_check: SaveCheckSlot::default(),
            confirm_checked: false,
            settings_tab: 0,
            tps_open: false,
            overlay: false,
            tps_values: user_settings.tps_defaults(),
            editor: None,
            video_background: false,
            user_settings,
        }
    }

    /// Builds a menu that persists settings to `settings_path`.
    #[must_use]
    pub fn new_with_settings(
        default_scenario: Option<PathBuf>,
        saves_dir: &Path,
        settings_path: &Path,
    ) -> Self {
        let mut menu = Self::new(default_scenario, saves_dir);
        if let Some(settings) = load_settings(settings_path) {
            menu.settings = settings;
        }
        menu.settings_path = Some(settings_path.to_path_buf());
        menu
    }

    /// Current options (applied to a newly opened world).
    #[must_use]
    pub fn settings(&self) -> MenuSettings {
        self.settings
    }

    /// Scene visibility implied by the current options.
    #[must_use]
    pub fn scene_options(&self) -> crate::render::SceneOptions {
        crate::render::SceneOptions {
            grid: self.settings.grid,
            zones: self.settings.zones,
            pellets: self.settings.pellets,
            pheromones: self.settings.pheromones,
            ..crate::render::SceneOptions::default()
        }
    }

    /// HUD visibility implied by the current options.
    #[must_use]
    pub fn show_hud(&self) -> bool {
        self.settings.hud
    }

    /// Scenarios offered by the New view.
    #[must_use]
    #[allow(dead_code)] // TODO(bind): menu views render these lists
    pub fn scenarios(&self) -> &[PathBuf] {
        &self.scenarios
    }

    /// Local bibite templates listed by the Library view.
    #[must_use]
    pub fn templates(&self) -> &[bibites_io::storage::LibraryEntry] {
        &self.templates
    }

    /// Selected library template path (when a template row is selected).
    #[must_use]
    pub fn pending_template(&self) -> Option<&Path> {
        self.pending_template.as_deref()
    }

    /// Scenario used by Create New/Accept (default scenario or first scenario).
    fn library_scenario(&self) -> Option<PathBuf> {
        self.scenarios.first().cloned()
    }

    /// Current view.
    #[must_use]
    pub fn view(&self) -> MenuView {
        self.view
    }

    /// Current selected row.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Status line (e.g. missing Default.zip).
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Records a load/status error and returns to the main menu.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.view = MenuView::Main;
        self.selected = 0;
        self.pending = None;
        self.pending_save = false;
        self.pending_challenge = None;
        self.status = message.into();
    }

    /// Saves directory currently scanned.
    #[must_use]
    pub fn saves_dir(&self) -> &Path {
        &self.saves_dir
    }

    /// Paths offered in the Load Game view.
    #[must_use]
    #[allow(dead_code)] // used by the window/load-game integration in follow-ups
    pub fn saves(&self) -> &[PathBuf] {
        &self.saves
    }

    /// Pending world path while the Loading view is shown.
    #[must_use]
    pub fn pending(&self) -> Option<&Path> {
        self.pending.as_deref()
    }

    /// Whether the pending world is a saved game (`SimulationManager.Start`: `loadedGame`) and not a new scenario.
    #[must_use]
    pub fn pending_is_save(&self) -> bool {
        self.pending_save
    }

    /// The challenge the pending world is, when `StartChallenge` started it (the champion row goes into the world).
    #[must_use]
    pub fn pending_challenge(&self) -> Option<&crate::challenge_run::ChallengeStart> {
        self.pending_challenge.as_ref()
    }

    /// The champion picked in the New Game screen's challenge list.
    #[must_use]
    pub fn champion(&self) -> Option<&crate::challenge_run::Champion> {
        self.champion.as_ref()
    }

    /// A menu on the New Game screen with one challenge entry and one Library row of its own (the player's folders are not used).
    #[cfg(test)]
    pub(crate) fn for_challenge_tests(tag: &str) -> (MenuState, PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("rustibites-chal-menu-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu = Self::new(None, &dir);
        menu.user_settings =
            crate::user_settings::UserSettings::load(Some(dir.join("settings.json")));
        let scenario = dir.join("Little Rabbits.zip");
        let champion = dir.join("Basic.bb8template");
        menu.scenario_entries = vec![
            ScenarioEntry {
                path: dir.join("Plain.zip"),
                name: "Plain".to_owned(),
                description: String::new(),
                version: "0.6.3".to_owned(),
                is_official: false,
                is_challenge: false,
                rank: 1,
                stars: Default::default(),
                preview_png: None,
            },
            ScenarioEntry {
                path: scenario.clone(),
                name: "Little Rabbits".to_owned(),
                description: "Grow.".to_owned(),
                version: "0.6.3".to_owned(),
                is_official: false,
                is_challenge: true,
                rank: 2,
                stars: ["a".to_owned(), "b".to_owned(), "c".to_owned()],
                preview_png: None,
            },
        ];
        menu.templates = vec![bibites_io::storage::LibraryEntry {
            name: "Basic bibite".to_owned(),
            description: String::new(),
            version: "0.6.3".to_owned(),
            is_official: true,
            n_hidden_nodes: 0,
            n_synapses: 0,
            path: champion.clone(),
            kind: bibites_io::storage::TemplateKind::Template,
        }];
        menu.view = MenuView::NewGame;
        (menu, scenario, champion)
    }

    /// Picks `path` as the champion without the Library (headless captures and tests); its name is the file's.
    pub fn set_champion(&mut self, path: PathBuf) {
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.champion = Some(crate::challenge_run::Champion { path, name });
    }

    /// Sets the highlighted row (mouse hover/click).
    pub fn select(&mut self, index: usize) {
        if index < self.row_count() {
            self.selected = index;
        }
    }

    /// Maps a window-space click to a menu row using the same layout as
    /// [`MenuState::build_frame`]: `x`, `y`, `width` and `height` are in the menu picture's own pixels (the picture is painted at its size
    /// and stretched to the window; [`picture_point`] brings the pointer there).
    #[must_use]
    pub fn hit_row(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        runtime: Option<&RuntimeUi>,
    ) -> Option<usize> {
        if self.view == MenuView::Loading {
            return None;
        }
        if self.view == MenuView::Main
            && let Some(runtime) = runtime
        {
            for (index, (pos, size)) in runtime.menu_button_rects(width, height).iter().enumerate()
            {
                if x >= pos.x && x <= pos.x + size.x && y >= pos.y && y <= pos.y + size.y {
                    return Some(index);
                }
            }
            return None;
        }
        let panel_width = (width * 0.52).clamp(420.0, 760.0);
        let panel_x = (width - panel_width) * 0.5;
        let panel_y = (height * 0.14).max(48.0);
        let row_width = panel_width - 72.0;
        let mut row_y = panel_y + 138.0;
        for index in 0..self.row_count() {
            if x >= panel_x + 36.0
                && x <= panel_x + 36.0 + row_width
                && y >= row_y
                && y <= row_y + 52.0
            {
                return Some(index);
            }
            row_y += 52.0 + 10.0;
        }
        None
    }

    /// Rows of the current view (game-style main menu order).
    #[must_use]
    pub fn rows(&self) -> Vec<String> {
        match self.view {
            MenuView::Main => vec![
                "Continue".to_owned(),
                "New".to_owned(),
                "Load".to_owned(),
                "Bibite Library".to_owned(),
                "Settings".to_owned(),
                "Achievements".to_owned(),
                "Additional Tools".to_owned(),
                "Credits".to_owned(),
                "Quit".to_owned(),
            ],
            MenuView::NewGame => {
                let mut rows: Vec<String> = self
                    .scenarios
                    .iter()
                    .take(8)
                    .map(|path| {
                        let selected = self.newgame_scenario.is_some_and(|index| {
                            self.scenarios
                                .get(index)
                                .is_some_and(|chosen| chosen == path)
                        });
                        format!("{}{}", if selected { "> " } else { "" }, file_label(path))
                    })
                    .collect();
                if rows.is_empty() {
                    rows.push("(no scenarios found)".to_owned());
                }
                rows.push(format!(
                    "Size: {}",
                    NEWGAME_SIZE_OPTIONS[self.newgame_options[0]]
                ));
                rows.push(format!(
                    "Biomass Density: {}",
                    NEWGAME_BIOMASS_OPTIONS[self.newgame_options[1]]
                ));
                rows.push(format!(
                    "Fertility: {}",
                    NEWGAME_FERTILITY_OPTIONS[self.newgame_options[2]]
                ));
                rows.push(format!(
                    "Radioactivity: {}",
                    NEWGAME_RADIO_OPTIONS[self.newgame_options[3]]
                ));
                rows.push(format!(
                    "Limit Bibites: {}",
                    NEWGAME_LIMIT_OPTIONS[self.newgame_options[4]]
                ));
                rows.push("Create Scenario & Start".to_owned());
                rows.push("Back".to_owned());
                rows
            }
            MenuView::LoadGame => {
                let mut rows: Vec<String> = self
                    .saves
                    .iter()
                    .take(8)
                    .map(|path| file_label(path))
                    .collect();
                if rows.is_empty() {
                    rows.push("(no saves found)".to_owned());
                }
                rows.push("Back".to_owned());
                rows
            }
            MenuView::Settings => vec![
                format!("Grid: {}", on_off(self.settings.grid)),
                format!("Zones: {}", on_off(self.settings.zones)),
                format!("Pellets: {}", on_off(self.settings.pellets)),
                format!("Pheromones: {}", on_off(self.settings.pheromones)),
                format!("HUD: {}", on_off(self.settings.hud)),
                format!("Simulation TPS: {}", self.settings.sim_tps),
                "Apply & Back".to_owned(),
                "Reset Defaults".to_owned(),
                "Back".to_owned(),
            ],
            MenuView::Library => {
                let mut rows: Vec<String> = self
                    .templates()
                    .iter()
                    .take(8)
                    .map(|entry| {
                        let selected = self
                            .library_selected
                            .and_then(|index| self.templates.get(index))
                            .is_some_and(|selected| selected.path == entry.path);
                        format!(
                            "{}{}  [{}]  {}",
                            if selected { "> " } else { "" },
                            entry.name,
                            entry.kind.as_str(),
                            entry.description.chars().take(48).collect::<String>(),
                        )
                    })
                    .collect();
                if rows.is_empty() {
                    rows.push("(no local templates found)".to_owned());
                }
                rows.push("Create New (starter)".to_owned());
                rows.push("Accept".to_owned());
                rows.push("Edit".to_owned());
                rows.push("Back".to_owned());
                rows
            }
            MenuView::Achievements => {
                let mut rows: Vec<String> = ACHIEVEMENTS
                    .iter()
                    .take(7)
                    .map(|name| format!("{name}  [locked]"))
                    .collect();
                rows.push("Back".to_owned());
                rows
            }
            MenuView::Tools => {
                let mut rows: Vec<String> = TOOLS.iter().map(|tool| (*tool).to_owned()).collect();
                rows.push("Back".to_owned());
                rows
            }
            MenuView::Credits => {
                let mut rows: Vec<String> = CREDITS.iter().map(|line| (*line).to_owned()).collect();
                rows.push("Back".to_owned());
                rows
            }
            MenuView::Loading | MenuView::ScenarioEditor => Vec::new(),
        }
    }

    /// Number of selectable rows in the current view.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.rows().len()
    }

    /// Moves the highlight by `delta` rows, wrapping.
    pub fn move_selection(&mut self, delta: isize) {
        let count = self.row_count();
        if count == 0 {
            self.selected = 0;
            return;
        }
        let current = self.selected.min(count - 1) as isize;
        self.selected = (current + delta).rem_euclid(count as isize) as usize;
    }

    /// Activates the highlighted row. View changes mutate `self`; starting a
    /// world or quitting returns an action.
    #[must_use]
    pub fn activate(&mut self) -> MenuAction {
        match self.view {
            MenuView::Main => match self.selected {
                0 => self.continue_latest(),
                1 => {
                    self.view = MenuView::NewGame;
                    self.selected = 0;
                    self.status.clear();
                    MenuAction::None
                }
                2 => {
                    self.open_load_game();
                    MenuAction::None
                }
                3 => {
                    self.open_view(MenuView::Library);
                    MenuAction::None
                }
                4 => {
                    self.open_view(MenuView::Settings);
                    MenuAction::None
                }
                5 => {
                    self.open_view(MenuView::Achievements);
                    MenuAction::None
                }
                6 => {
                    self.open_view(MenuView::Tools);
                    MenuAction::None
                }
                7 => {
                    self.open_view(MenuView::Credits);
                    MenuAction::None
                }
                _ => MenuAction::Quit,
            },
            MenuView::NewGame => self.activate_newgame(),
            MenuView::LoadGame => {
                if self.selected < self.saves.len() {
                    self.start_save(self.selected)
                } else {
                    self.back_to_main();
                    MenuAction::None
                }
            }
            MenuView::Settings => self.activate_setting(self.selected),
            MenuView::Library => self.activate_library(),
            MenuView::Achievements | MenuView::Tools | MenuView::Credits => {
                let rows = self.rows();
                if self.selected + 1 >= rows.len() {
                    self.back_to_main();
                } else {
                    self.status = format!("{} - informational entry", rows[self.selected]);
                }
                MenuAction::None
            }
            MenuView::Loading | MenuView::ScenarioEditor => MenuAction::None,
        }
    }

    fn activate_newgame(&mut self) -> MenuAction {
        let scenario_count = self.scenarios.len().min(8);
        if self.selected < scenario_count {
            self.newgame_scenario = Some(self.selected);
            self.status = format!("scenario: {}", file_label(&self.scenarios[self.selected]));
            return MenuAction::None;
        }
        let option_row = self.selected - scenario_count;
        if option_row < 5 {
            self.newgame_options[option_row] =
                (self.newgame_options[option_row] + 1) % NEWGAME_OPTION_COUNTS[option_row];
            return MenuAction::None;
        }
        if option_row == 5 {
            return self.generate_newgame();
        }
        self.back_to_main();
        MenuAction::None
    }

    /// Applies the New Game panel's modifiers (size, biomass density, fertility, radioactivity,
    /// bibite limit) and its TPS sliders to `settings`: in the game these are the same settings
    /// the scenario editor edits.
    fn apply_newgame_modifiers(&self, settings: &mut bibites_core::settings::SimSettings) {
        use bibites_core::settings::SettingValue;
        let size = NEWGAME_SIZE_VALUES[self.newgame_options[0]];
        let density = NEWGAME_BIOMASS_VALUES[self.newgame_options[1]];
        let fertility = NEWGAME_FERTILITY_VALUES[self.newgame_options[2]];
        let dirty = self.newgame_dirty;
        if dirty[0] {
            settings
                .independents
                .set_raw("SimulationSize", SettingValue::Float(size));
        }
        if dirty[1] {
            settings
                .independents
                .set_raw("biomassDensity", SettingValue::Float(density));
        }
        if dirty[2] {
            settings
                .independents
                .set_raw("pelletGrowth", SettingValue::Float(fertility));
        }
        let (chance, variance) = NEWGAME_RADIO_LEVELS[self.newgame_options[3]];
        if dirty[3] {
            settings
                .scenario
                .set_raw("backgroundMutationChance", SettingValue::Float(chance));
            settings
                .scenario
                .set_raw("backgroundMutationVariance", SettingValue::Float(variance));
        }
        // The panel's TPS values are the live settings when the game starts.
        for (key, value) in TPS_SETTING_KEYS.into_iter().zip(self.tps_values) {
            settings.independents.set_raw(key, SettingValue::Int(value));
        }
        match self.newgame_options[4] {
            _ if !dirty[4] => {}
            4 => {
                settings
                    .independents
                    .set_raw("limitBibiteBirth", SettingValue::Bool(false));
                settings
                    .independents
                    .set_raw("bibiteLimit", SettingValue::Int(50));
            }
            index => {
                settings
                    .independents
                    .set_raw("limitBibiteBirth", SettingValue::Bool(true));
                settings.independents.set_raw(
                    "bibiteLimit",
                    SettingValue::Int([50, 100, 200, 500][index.min(3)]),
                );
            }
        }
    }

    /// Generates a fresh scenario/save from the selected scenario + options.
    fn generate_newgame(&mut self) -> MenuAction {
        let scenario = self
            .newgame_scenario
            .and_then(|index| self.scenarios.get(index))
            .cloned()
            .or_else(|| self.scenarios.first().cloned());
        let Some(scenario) = scenario else {
            self.status = "No scenario selected".to_owned();
            return MenuAction::None;
        };
        let generated: Result<PathBuf, String> = (|| {
            let bundle =
                bibites_io::load_scenario_bundle(&scenario).map_err(|error| error.to_string())?;
            let mut settings = bibites_io::world::scenario_effective_settings(&bundle, None);
            self.apply_newgame_modifiers(&mut settings);
            let templates = bundle
                .templates
                .iter()
                .map(|(name, loaded)| {
                    bibites_core::world::TemplateSource::new(
                        name.clone(),
                        loaded.genome.clone(),
                        loaded.meta.clone(),
                        loaded.brain.clone(),
                    )
                })
                .collect::<Vec<_>>();
            let _world = bibites_core::world::World::new_fresh_scenario(
                settings.clone(),
                bundle.scenario.clone(),
                12_345,
                templates,
            )
            .map_err(|error| error.to_string())?;
            let mut out = bundle;
            out.settings = settings.clone();
            out.settings_file.scenario = settings.scenario.clone();
            out.settings_file.recommended = Some(settings.independents.clone());
            out.settings_file.materials = settings.materials;
            let path = std::env::temp_dir().join("rustibites-newgame-scenario.zip");
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            bibites_io::write_scenario_bundle(&out, &path).map_err(|error| error.to_string())?;
            Ok::<PathBuf, String>(path)
        })();
        match generated {
            Ok(path) => {
                self.begin_loading(path.clone(), false);
                MenuAction::Start(path)
            }
            Err(error) => {
                self.status = format!("Could not generate scenario: {error}");
                MenuAction::None
            }
        }
    }

    fn activate_library(&mut self) -> MenuAction {
        let templates = self.templates.len().min(8);
        if self.selected < templates {
            self.library_selected = Some(self.selected);
            self.status = format!("selected {}", self.templates[self.selected].name);
            return MenuAction::None;
        }
        match self.selected - templates {
            0 => self.start_library_world(None),
            1 => {
                let template = self
                    .library_selected
                    .and_then(|index| self.templates.get(index))
                    .map(|entry| entry.path.clone());
                self.start_library_world(template)
            }
            2 => {
                self.status = "Bibite editor screens are not available yet".to_owned();
                MenuAction::None
            }
            _ => {
                self.back_to_main();
                MenuAction::None
            }
        }
    }

    fn start_library_world(&mut self, template: Option<PathBuf>) -> MenuAction {
        let Some(scenario) = self.library_scenario() else {
            self.status = "No scenario available for a new world".to_owned();
            return MenuAction::None;
        };
        self.pending_template = template;
        self.begin_loading(scenario.clone(), false);
        MenuAction::Start(scenario)
    }

    /// `GameManager.Continue`: the newest save, autosaves included
    /// (`SaveController.GetLastSave`).
    fn continue_latest(&mut self) -> MenuAction {
        let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
        let mut dirs: Vec<PathBuf> = Vec::new();
        for root in &self.data_roots {
            dirs.push(root.join("Savefiles"));
            dirs.push(root.join("Savefiles").join("Autosaves"));
        }
        // The committed test saves / `--saves-dir` are a fallback for machines without
        // the game's own folder (otherwise they would list copies the game never shows).
        if !self
            .data_roots
            .iter()
            .any(|root| root.join("Savefiles").is_dir())
        {
            dirs.push(self.saves_dir.clone());
        }
        let mut seen = std::collections::HashSet::new();
        for dir in dirs {
            for path in scan_saves(&dir) {
                if !seen.insert(std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone())) {
                    continue;
                }
                let modified = std::fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                if newest.as_ref().is_none_or(|(best, _)| modified > *best) {
                    newest = Some((modified, path));
                }
            }
        }
        match newest {
            Some((_, path)) => {
                self.begin_loading(path.clone(), true);
                MenuAction::Start(path)
            }
            None => {
                self.status = "No saves found".to_owned();
                MenuAction::None
            }
        }
    }

    /// `ScenarioSelectorPanel.StartChallenge(asChallenge)`: the selected challenge scenario with the champion's row added; as a
    /// challenge it needs a champion (the "Start Challenge" button is not interactable without one).
    fn start_challenge(&mut self, as_challenge: bool) -> MenuAction {
        let Some(entry) = self.selected_scenario() else {
            self.status = "No scenario selected".to_owned();
            return MenuAction::None;
        };
        if !self.showing_challenges || (as_challenge && self.champion.is_none()) {
            return MenuAction::None;
        }
        let path = entry.path.clone();
        let start = crate::challenge_run::ChallengeStart {
            scenario: path.clone(),
            champion: self.champion.clone(),
            as_challenge,
        };
        self.begin_loading(path.clone(), false);
        self.pending_challenge = Some(start);
        MenuAction::Start(path)
    }

    fn start_save(&mut self, index: usize) -> MenuAction {
        if let Some(path) = self.saves.get(index).cloned() {
            self.begin_loading(path.clone(), true);
            MenuAction::Start(path)
        } else {
            self.status = "No saves found".to_owned();
            MenuAction::None
        }
    }

    fn open_view(&mut self, view: MenuView) {
        self.view = view;
        self.selected = 0;
        self.status.clear();
        // A list opens at its top (`FillLists` builds it again); a tab is only shown again, where it was left.
        if !matches!(view, MenuView::Credits | MenuView::Tools) {
            *self.scroll_slot() = 0.0;
        }
    }

    fn activate_setting(&mut self, index: usize) -> MenuAction {
        match index {
            0 => self.settings.grid = !self.settings.grid,
            1 => self.settings.zones = !self.settings.zones,
            2 => self.settings.pellets = !self.settings.pellets,
            3 => self.settings.pheromones = !self.settings.pheromones,
            4 => self.settings.hud = !self.settings.hud,
            5 => {
                self.settings.sim_tps = match self.settings.sim_tps {
                    15 => 30,
                    30 => 60,
                    60 => 120,
                    _ => 15,
                };
            }
            6 => {
                if let Some(path) = self.settings_path.as_deref()
                    && let Err(error) = save_settings(path, self.settings)
                {
                    self.status = format!("Could not save settings: {error}");
                } else {
                    self.status = "Settings applied".to_owned();
                }
                self.back_to_main();
            }
            7 => {
                self.settings = MenuSettings::default();
                self.status = "Settings reset to defaults".to_owned();
            }
            _ => self.back_to_main(),
        }
        MenuAction::None
    }

    /// Esc: go back one screen (main menu remains).
    pub fn back(&mut self) {
        if self.tps_open {
            // Esc closes the topmost panel first.
            self.tps_open = false;
            return;
        }
        // An open dropdown list closes first (`DropdownItem.OnCancel`).
        if self.dropdown.take().is_some() {
            return;
        }
        // A warning popup is on top of everything: Esc is its Cancel (nothing is deleted or
        // remembered).
        if self.confirm_delete || self.confirm_library_delete {
            self.confirm_delete = false;
            self.confirm_library_delete = false;
            self.confirm_checked = false;
            return;
        }
        match self.view {
            MenuView::Main | MenuView::Loading => {}
            MenuView::ScenarioEditor => self.editor_escape(),
            MenuView::Library if self.pick.is_some() => self.cancel_pick(),
            _ => self.back_to_main(),
        }
    }

    fn back_to_main(&mut self) {
        self.view = MenuView::Main;
        self.selected = 0;
        self.pending_template = None;
    }

    fn begin_loading(&mut self, path: PathBuf, save: bool) {
        self.pending = Some(path);
        self.pending_save = save;
        self.pending_challenge = None;
        self.view = MenuView::Loading;
        self.selected = 0;
    }

    /// What the Library is picking a template for, if anything.
    #[must_use]
    pub fn pick(&self) -> Option<TemplatePick> {
        self.pick
    }

    /// Opens the Library to pick a template (`OpenWithCallBacks(..., submitText: "Select Bibite")`).
    pub fn begin_pick(&mut self, pick: TemplatePick) {
        self.pick = Some(pick);
        self.pick_return = self.view;
        self.library_selected = None;
        self.open_view(MenuView::Library);
    }

    /// Leaves the Library without a pick: back to where it was opened from.
    fn cancel_pick(&mut self) {
        // The champion's Library opens over the New Game screen, which stays where it was (`CancelSelection` closes the Library only).
        let champion = self.pick == Some(TemplatePick::Champion);
        self.pick = None;
        if self.editor.is_some() || champion {
            self.view = self.pick_return;
            self.selected = 0;
        } else {
            self.back_to_main();
        }
    }

    /// The submit button of a Library opened to pick: hands over the selected template.
    fn finish_pick(&mut self) -> MenuAction {
        let Some(entry) = self
            .library_selected
            .and_then(|index| self.templates.get(index))
            .cloned()
        else {
            self.status = "Select a template first".to_owned();
            return MenuAction::None;
        };
        let pick = self.pick.take();
        match pick {
            Some(TemplatePick::DefaultBibite) => {
                let file_name = entry
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned();
                if let Some(editor) = self.editor.as_deref_mut() {
                    // The scenario editor adds a default bibite to its draft (the file travels
                    // with the scenario).
                    match std::fs::read(&entry.path) {
                        Ok(bytes) => editor.add_default_bibite(&file_name, bytes),
                        Err(error) => {
                            self.status = format!("Could not read {file_name}: {error}");
                        }
                    }
                    self.view = self.pick_return;
                    self.selected = 0;
                    MenuAction::None
                } else {
                    // Over a running world the caller owns the rows.
                    self.back_to_main();
                    MenuAction::PickedTemplate(entry.path)
                }
            }
            // The placer is a world feature: the caller takes the file.
            Some(TemplatePick::PlaceBibite) => {
                self.back_to_main();
                MenuAction::PickedTemplate(entry.path)
            }
            // `ScenarioSelectorPanel.OnChampionSelected`: the champion is kept and the New Game screen is where it was.
            Some(TemplatePick::Champion) => {
                self.champion = Some(crate::challenge_run::Champion {
                    path: entry.path,
                    name: entry.name,
                });
                self.view = self.pick_return;
                self.selected = 0;
                MenuAction::None
            }
            None => MenuAction::None,
        }
    }

    /// Human-readable title for the current view.
    #[must_use]
    pub fn view_title(&self) -> &'static str {
        match self.view {
            MenuView::Main => "RUSTIBITES",
            MenuView::NewGame => "NEW SIMULATION",
            MenuView::LoadGame => "LOAD GAME",
            MenuView::Settings => "SETTINGS",
            MenuView::Library => "BIBITE LIBRARY",
            MenuView::Achievements => "ACHIEVEMENTS",
            MenuView::Tools => "ADDITIONAL TOOLS",
            MenuView::Credits => "CREDITS",
            MenuView::ScenarioEditor => "SCENARIO EDITOR",
            MenuView::Loading => "LOADING",
        }
    }

    /// Builds the current menu screen as a UI draw list. The title uses the
    /// extracted game fonts; panel geometry is curated because the game menu
    /// session is not extracted yet.
    #[must_use]
    pub fn build_frame(
        &self,
        ui: &UiAssets,
        fonts: &mut FontSet,
        width: f32,
        height: f32,
        runtime: Option<&RuntimeUi>,
    ) -> UiFrame {
        if self.view == MenuView::Main
            && let Some(runtime) = runtime
            && let Some(mut frame) = runtime.build_menu_frame(false, ui, width, height, fonts.sdf())
        {
            tagline::add(&mut frame, width, height);
            if self.video_background {
                // The animated video is drawn behind this frame.
            } else if let Some(texture) = ui.texture_of_alias("menu_background") {
                frame.draws.insert(
                    0,
                    crate::ui::UiDraw::Image {
                        texture,
                        pos: Vec2::ZERO,
                        size: Vec2::new(width, height),
                        tint: [255, 255, 255, 255],
                    },
                );
            } else {
                frame.draws.insert(
                    0,
                    crate::ui::UiDraw::Rect {
                        pos: Vec2::ZERO,
                        size: Vec2::new(width, height),
                        color: [18, 20, 22, 255],
                    },
                );
            }
            return frame;
        }
        if self.view == MenuView::Loading
            && let Some(runtime) = runtime
            && let Some(frame) =
                runtime.build_menu_loading_frame(self, ui, width, height, fonts.sdf())
        {
            return frame;
        }
        if !matches!(self.view, MenuView::Main | MenuView::Loading)
            && let Some(runtime) = runtime
            && let Some(mut frame) =
                runtime.build_menu_view_frame(self, ui, width, height, fonts.sdf())
        {
            // An open dropdown list hangs over the screen, its blocker covering it.
            if let Some(open) = self.dropdown.as_ref() {
                crate::dropdown::emit(
                    &mut frame,
                    ui,
                    fonts.sdf(),
                    open,
                    Vec2::new(width, height),
                    self.cursor,
                );
            }
            return frame;
        }
        let mut frame = UiFrame::new(1.0);
        frame.rect(Vec2::ZERO, Vec2::new(width, height), [24, 26, 30, 255]);
        // A faint horizon/grid band keeps the menu readable over black.
        frame.rect(
            Vec2::new(0.0, height * 0.52),
            Vec2::new(width, height * 0.48),
            [38, 42, 42, 255],
        );
        let panel_width = (width * 0.52).clamp(420.0, 760.0);
        let panel_x = (width - panel_width) * 0.5;
        let panel_y = (height * 0.14).max(48.0);
        frame.rect(
            Vec2::new(panel_x, panel_y),
            Vec2::new(panel_width, height - panel_y - 48.0),
            [18, 20, 22, 245],
        );
        frame.rect(
            Vec2::new(panel_x, panel_y),
            Vec2::new(panel_width, 4.0),
            [206, 164, 72, 255],
        );
        let title = self.view_title();
        let title_px = 42.0;
        let title_width = fonts.measure(FontRole::Bold, title, title_px);
        frame.bold(
            title,
            Vec2::new((width - title_width) * 0.5, panel_y + 34.0),
            title_px,
            [236, 226, 198, 255],
        );
        let subtitle = "The Bibites-compatible simulation";
        let subtitle_px = 17.0;
        let subtitle_width = fonts.measure(FontRole::Body, subtitle, subtitle_px);
        frame.text(
            subtitle,
            Vec2::new((width - subtitle_width) * 0.5, panel_y + 88.0),
            subtitle_px,
            [172, 176, 178, 255],
        );
        let rows = self.rows();
        if self.view == MenuView::Loading {
            let text = self
                .pending
                .as_ref()
                .map(|path| {
                    format!(
                        "Loading {}",
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("world")
                    )
                })
                .unwrap_or_else(|| "Loading world".to_owned());
            frame.bold(
                &text,
                Vec2::new(panel_x + 36.0, panel_y + 170.0),
                24.0,
                [236, 226, 198, 255],
            );
            frame.text(
                "Preparing simulation...",
                Vec2::new(panel_x + 36.0, panel_y + 208.0),
                17.0,
                [172, 176, 178, 255],
            );
            return frame;
        }
        let row_height = 52.0;
        let row_gap = 10.0;
        let row_width = panel_width - 72.0;
        let mut y = panel_y + 138.0;
        for (index, row) in rows.iter().enumerate() {
            let rect = Vec2::new(panel_x + 36.0, y);
            let size = Vec2::new(row_width, row_height);
            let selected = index == self.selected;
            frame.rect(
                rect,
                size,
                if selected {
                    [92, 78, 42, 255]
                } else {
                    [30, 33, 36, 255]
                },
            );
            if selected {
                frame.rect(rect, Vec2::new(4.0, row_height), [236, 196, 96, 255]);
            }
            let px = 20.0;
            let text_width = fonts.measure(FontRole::Body, row, px);
            frame.text(
                row,
                Vec2::new(
                    rect.x + (row_width - text_width) * 0.5,
                    rect.y + (row_height - px) * 0.5,
                ),
                px,
                if selected {
                    [250, 238, 206, 255]
                } else {
                    [214, 216, 216, 255]
                },
            );
            y += row_height + row_gap;
        }
        let footer = if self.status().is_empty() {
            format!(
                "saves: {}   |   arrow keys / Enter / Esc   |   game runtime UI pending",
                self.saves_dir().display()
            )
        } else {
            self.status().to_owned()
        };
        frame.text(
            &footer,
            Vec2::new(panel_x + 28.0, panel_y + (height - panel_y - 48.0) - 34.0),
            14.0,
            [150, 154, 156, 255],
        );
        frame
    }
}

/// One row of the scenario/challenge list (`ScenarioItemReference`) together with
/// what `ScenarioSelectorPanel.SelectScenarioItem` shows when it is selected.
#[derive(Clone, Debug)]
pub struct ScenarioEntry {
    /// Archive path.
    pub path: PathBuf,
    /// `scenario.info` name.
    pub name: String,
    /// `scenario.info` description.
    pub description: String,
    /// Scenario version text.
    pub version: String,
    /// Shipped scenario flag.
    pub is_official: bool,
    /// Challenge flag.
    pub is_challenge: bool,
    /// Official sort rank (`1073741823` when unranked).
    pub rank: i32,
    /// Star objectives (challenges).
    pub stars: [String; 3],
    /// Preview image (first `.png` of the archive).
    pub preview_png: Option<std::sync::Arc<Vec<u8>>>,
}

/// Reads the scenario cards of every `*.zip` in `dirs` (first directory wins on
/// duplicate file names), hiding scenarios older than 0.5.1 like the game does.
#[must_use]
pub fn load_scenario_entries(paths: &[PathBuf]) -> Vec<ScenarioEntry> {
    let mut entries: Vec<ScenarioEntry> = Vec::new();
    let mut seen: Vec<std::ffi::OsString> = Vec::new();
    for path in paths {
        let Some(file_name) = path.file_name().map(std::ffi::OsString::from) else {
            continue;
        };
        if seen.contains(&file_name) {
            continue;
        }
        let Ok(Some(card)) = bibites_io::read_scenario_card(path) else {
            continue;
        };
        seen.push(file_name);
        entries.push(ScenarioEntry {
            path: path.clone(),
            name: card.info.name.clone(),
            description: card.info.description.clone(),
            version: card.info.version_text.clone(),
            is_official: card.info.is_official,
            is_challenge: card.info.is_challenge,
            rank: card.info.rank.unwrap_or(1_073_741_823),
            stars: [
                card.info.stars[0].clone().unwrap_or_default(),
                card.info.stars[1].clone().unwrap_or_default(),
                card.info.stars[2].clone().unwrap_or_default(),
            ],
            preview_png: card.preview_png.map(std::sync::Arc::new),
        });
    }
    // `ScenarioSelectorPanel.ReloadList`: ascending rank (official first), then name.
    entries.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// Local UTC offset in seconds (queried once from the OS; 0 when unavailable,
/// the standard library has no local-time zone lookup and unsafe is forbidden).
fn local_utc_offset_secs() -> i64 {
    // The game-preferences reader (one PowerShell run, started in the background at launch) already asked the machine for its offset.
    // A PowerShell of its own here made the first Load Game / Save Game screen of a session freeze for seconds (it was 4.8 s in the
    // real window with the first run still cold). UTC when it is unknown, as the autosaves' names do.
    bibites_io::prefs::GamePrefs::global()
        .utc_offset_minutes()
        .map_or(0, |minutes| minutes * 60)
}

/// `FileInfo.LastWriteTime.ToString("dd/MM/yyyy HH:mm")` in local time.
/// `DateTime.Now.ToString("yyyyMMddHHmmss")` for the sprite generator's file names.
pub(crate) fn file_stamp(time: std::time::SystemTime) -> String {
    let secs = time.duration_since(std::time::UNIX_EPOCH).map_or(0, |duration| duration.as_secs()) as i64 + local_utc_offset_secs();
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}{:02}{:02}{:02}", rem / 3_600, (rem % 3_600) / 60, rem % 60)
}

pub(crate) fn format_save_date(time: std::time::SystemTime) -> String {
    let secs = time
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs()) as i64
        + local_utc_offset_secs();
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{day:02}/{month:02}/{year:04} {:02}:{:02}",
        rem / 3_600,
        (rem % 3_600) / 60
    )
}

fn nearest_index(values: &[f32], value: f32) -> usize {
    values
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - value).abs().total_cmp(&(b.1 - value).abs()))
        .map_or(0, |(index, _)| index)
}

impl MenuState {
    /// Points the menu at the data roots (game data folder, local user data, ...):
    /// scenarios come from `<root>/Scenarios`, saves from `<root>/Savefiles`.
    pub fn set_data_roots(&mut self, roots: Vec<PathBuf>) {
        self.data_roots = roots;
        self.reload_scenarios();
        self.reload_saves();
    }

    /// Opens a view directly (deterministic screenshots / tests).
    pub fn open_view_for_capture(&mut self, view: MenuView) {
        self.open_view(view);
    }

    /// Applies a click at `point` (menu-texture pixels) on a native panel view.
    /// Returns the resulting action; main-menu rows are handled by the caller.
    pub fn click_native(
        &mut self,
        ui: &UiAssets,
        fonts: &mut FontSet,
        width: f32,
        height: f32,
        runtime: Option<&RuntimeUi>,
        point: Vec2,
    ) -> Option<MenuAction> {
        if matches!(self.view, MenuView::Main | MenuView::Loading) {
            return None;
        }
        let frame = self.build_frame(ui, fonts, width, height, runtime);
        let actions = frame.interact(point, true, true, &mut crate::ui::UiInteraction::default());
        // The control under the pointer (reference px): where a clicked dropdown hangs its list.
        let under = frame
            .widget_at(point)
            .and_then(|id| frame.hits.iter().find(|hit| hit.0 == id))
            .map(|(_, pos, size)| {
                let scale = frame.scale.max(f32::EPSILON);
                [pos.x / scale, pos.y / scale, size.x / scale, size.y / scale]
            });
        let mut result = MenuAction::None;
        let mut handled = false;
        // The scenario editor's graph and its Ctrl-clicks need where and how the press happened.
        let (ctrl, shift, scale) = (self.ctrl_down, self.shift_down, frame.scale.max(f32::EPSILON));
        if let Some(editor) = self.editor.as_deref_mut() {
            editor.pointer = crate::scenario_editor::Pointer { ctrl, cursor: [point.x, point.y], rect: under, shift, scale };
            // A press anywhere but on the open text field closes it without applying (`ExitEditField`).
            let inside = actions.iter().any(|action| {
                matches!(
                    action,
                    crate::ui::UiAction::ZoneNameEdit(_) | crate::ui::UiAction::DynEditValue(_) | crate::ui::UiAction::ValueEditInside
                )
            });
            if !inside {
                editor.cancel_edit();
            }
        }
        for action in actions {
            // Dropdown controls open their list; the list's rows pick, its blocker closes it.
            match action {
                crate::ui::UiAction::DropdownOpen(target) => {
                    handled = true;
                    let anchor = under.unwrap_or(self.click_rect);
                    self.open_dropdown_list(target, anchor);
                    continue;
                }
                crate::ui::UiAction::DropdownPick(index) => {
                    handled = true;
                    self.pick_dropdown_list(usize::from(index));
                    continue;
                }
                crate::ui::UiAction::DropdownClose => {
                    handled = true;
                    self.dropdown = None;
                    continue;
                }
                _ => {}
            }
            if self.view == MenuView::ScenarioEditor && self.editor_action(action) {
                handled = true;
                continue;
            }
            if let crate::ui::UiAction::Menu(command) = action {
                handled = true;
                if let crate::ui::MenuCommand::SettingSlide(index, x0, x1) = command {
                    let span = f32::from(x1) - f32::from(x0);
                    if span > 0.0 {
                        // `Slider.OnPointerDown`: a press on the handle keeps the handle's place under the pointer (the value does not
                        // move until the pointer does), one on the track moves the handle there.
                        let handle = f32::from(x0) + self.user_settings.fraction(usize::from(index)) * span;
                        let grab = slider_grab(handle, point.x, scale);
                        self.user_settings
                            .set_fraction(usize::from(index), (point.x - grab - f32::from(x0)) / span);
                        // The press holds the slider: the pointer moves it from here (`slider_drag_move`).
                        self.slider_drag = Some(HeldSlider::Setting(index, x0, x1, grab));
                    }
                }
                if command == crate::ui::MenuCommand::ListBarHandle
                    && let Some(bar) = self.list_bar()
                {
                    let scale = frame.scale.max(f32::EPSILON);
                    self.bar_grab = Some(point.y / scale - bar.handle()[1]);
                }
                if let crate::ui::MenuCommand::TpsSlide(index, x0, x1) = command {
                    let span = f32::from(x1) - f32::from(x0);
                    if span > 0.0 {
                        let slot = usize::from(index).min(4);
                        let handle = f32::from(x0) + Self::tps_fraction_of(slot, self.tps_values[slot]) * span;
                        let grab = slider_grab(handle, point.x, scale);
                        self.tps_set_fraction(slot, (point.x - grab - f32::from(x0)) / span);
                        self.slider_drag = Some(HeldSlider::Tps(index, x0, x1, grab));
                    }
                }
                let outcome = self.menu_command(command);
                if outcome != MenuAction::None {
                    result = outcome;
                }
            }
        }
        if !handled && self.dropdown.is_some() {
            // Clicking elsewhere closes an open dropdown.
            self.dropdown = None;
        }
        Some(result)
    }

    /// Whether the current view is one of the native panels (not the title buttons).
    #[must_use]
    pub fn is_native_view(&self) -> bool {
        !matches!(self.view, MenuView::Main | MenuView::Loading)
    }

    /// Up/Down in a native panel: moves the selection of its list.
    pub fn native_move(&mut self, delta: isize) {
        let step = |current: Option<usize>, len: usize| -> Option<usize> {
            if len == 0 {
                return None;
            }
            let base = current.map_or(if delta > 0 { -1 } else { len as isize }, |i| i as isize);
            Some((base + delta).clamp(0, len as isize - 1) as usize)
        };
        match self.view {
            MenuView::NewGame => {
                let visible = self.visible_entries();
                let position = self
                    .selected_entry
                    .and_then(|entry| visible.iter().position(|v| *v == entry));
                if let Some(next) = step(position, visible.len()) {
                    self.select_entry(Some(visible[next]));
                }
            }
            MenuView::LoadGame => {
                if let Some(next) = step(self.selected_save, self.saves.len()) {
                    self.select_save(Some(next));
                }
            }
            MenuView::Library => {
                // Up / Down walk the rows of the open tab.
                let rows = self.library_rows();
                let position = self
                    .library_selected
                    .and_then(|selected| rows.iter().position(|row| *row == selected));
                if let Some(next) = step(position, rows.len()) {
                    self.library_selected = Some(rows[next]);
                }
            }
            _ => {}
        }
    }

    /// Enter in a native panel: start / load / select, or go back for static panels.
    pub fn native_enter(&mut self) -> MenuAction {
        use crate::ui::MenuCommand;
        match self.view {
            MenuView::NewGame => self.menu_command(MenuCommand::StartSimulation),
            MenuView::LoadGame => self.menu_command(MenuCommand::LoadSelected),
            MenuView::Library => self.menu_command(MenuCommand::StartTemplate),
            MenuView::ScenarioEditor => {
                self.editor_enter();
                MenuAction::None
            }
            _ => {
                self.back_to_main();
                MenuAction::None
            }
        }
    }

    /// Re-reads the scenario folders.
    pub fn reload_scenarios(&mut self) {
        let mut paths: Vec<PathBuf> = Vec::new();
        for root in &self.data_roots {
            paths.extend(scan_scenarios(&root.join("Scenarios")));
        }
        if paths.is_empty() {
            paths.extend(self.scenarios.iter().cloned());
        }
        self.scenario_entries = load_scenario_entries(&paths);
        self.scenarios = self
            .scenario_entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        self.selected_entry = None;
        // `ScenarioSelectorPanel.ReloadList` ends with `tpsSettings.ResetValue()`.
        self.tps_values = self.user_settings.tps_defaults();
        self.tps_open = false;
        self.select_first_visible();
    }

    /// Tells the menu that the animated title video plays behind it (or stopped).
    pub fn set_video_background(&mut self, on: bool) {
        self.video_background = on;
    }

    /// Whether the animated title video plays behind this menu.
    #[must_use]
    pub fn video_background(&self) -> bool {
        self.video_background
    }

    /// Whether the current screen shows the title background (so the video must be drawn behind
    /// it): the title buttons, the loading screen and the side panels (Achievements, Tools, Credits),
    /// not the full-screen New Game / Load Game / Library / Settings / editor screens (their roots are
    /// opaque) and not the in-game overlays.
    #[must_use]
    pub fn shows_video(&self) -> bool {
        self.video_background
            && !self.overlay
            && matches!(
                self.view,
                MenuView::Main
                    | MenuView::Loading
                    | MenuView::Achievements
                    | MenuView::Tools
                    | MenuView::Credits
            )
    }

    /// Marks this menu as an in-game panel overlay.
    pub fn set_overlay(&mut self, on: bool) {
        self.overlay = on;
    }

    /// Whether this menu is an in-game panel overlay.
    #[must_use]
    pub fn overlay(&self) -> bool {
        self.overlay
    }

    /// Whether the TPS settings popup is showing.
    #[must_use]
    pub fn tps_open(&self) -> bool {
        self.tps_open
    }

    /// Current TPS slider values.
    #[must_use]
    pub fn tps_values(&self) -> [i32; 5] {
        self.tps_values
    }

    /// The TPS defaults (default pointers / reset targets).
    #[must_use]
    pub fn tps_defaults(&self) -> [i32; 5] {
        self.user_settings.tps_defaults()
    }

    /// Slider position `0..=1` of TPS value `index` (log scale, `LogSettingSlider`).
    #[must_use]
    pub fn tps_fraction_of(index: usize, value: i32) -> f32 {
        let (_, _, min, max) = TPS_SLIDERS[index];
        let (min, max) = (min as f32, max as f32);
        ((value.max(1) as f32 / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
    }

    /// Sets TPS value `index` from a slider fraction (`LogSettingSlider.ToLinear`).
    pub fn tps_set_fraction(&mut self, index: usize, fraction: f32) {
        let (_, _, min, max) = TPS_SLIDERS[index];
        let (min, max) = (min as f32, max as f32);
        let value = min * (max / min).powf(fraction.clamp(0.0, 1.0));
        self.tps_values[index] = (value.round() as i32).clamp(min as i32, max as i32);
    }

    /// Re-reads the save folders (`Savefiles`, plus `Autosaves` when included).
    pub fn reload_saves(&mut self) {
        let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for root in &self.data_roots {
            dirs.push(root.join("Savefiles"));
            if self.include_autosaves {
                dirs.push(root.join("Savefiles").join("Autosaves"));
            }
        }
        // The committed test saves / `--saves-dir` are a fallback for machines without
        // the game's own folder (otherwise they would list copies the game never shows).
        if !self
            .data_roots
            .iter()
            .any(|root| root.join("Savefiles").is_dir())
        {
            dirs.push(self.saves_dir.clone());
        }
        let mut seen = std::collections::HashSet::new();
        for dir in dirs {
            // One `canonicalize` for the folder, not one that opens each of its 1,500 files; the times come with the listing.
            let folder = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
            for (modified, path) in scan_saves_with_times(&dir) {
                if !seen.insert(folder.join(path.file_name().unwrap_or_default())) {
                    continue;
                }
                found.push((modified, path));
            }
        }
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        self.save_dates = found
            .iter()
            .map(|(modified, _)| format_save_date(*modified))
            .collect();
        self.saves = found.into_iter().map(|(_, path)| path).collect();
        self.confirm_delete = false;
        self.select_save((!self.saves.is_empty()).then_some(0));
    }

    /// The title menu's "Load": the list of saves is read from the folders, and the saves are checked beside the window
    /// ([`MenuState::report_corrupted_saves`]); the click that does this must return at once.
    pub fn open_load_game(&mut self) {
        self.view = MenuView::LoadGame;
        self.selected = 0;
        self.status.clear();
        self.scrolls[scroll_index(MenuView::LoadGame)] = 0.0;
        self.reload_saves();
        self.report_corrupted_saves();
    }

    /// The unreadable saves found so far, whose popups wait (diagnosis and tests).
    #[must_use]
    pub fn pending_error_count(&self) -> usize {
        self.errors.len()
    }

    /// `LoadGamePanel.FillList`: a save that cannot be read raises the "Corrupted Save" error popup (one for each, in the list's order).
    ///
    /// The saves are read on a thread of their own: a list of 1,500 autosaves (4.9 GB) took the window many seconds, long enough for Windows to call
    /// it "not responding", before it could even repaint after the click on Load. The popups come up as the thread finds the unreadable ones
    /// ([`MenuState::poll_background`], once a frame), still in the list's order; opening the list again while a check runs stops that one.
    fn report_corrupted_saves(&mut self) {
        // Dropping the check that may still be running stops its thread.
        self.save_check.0 = None;
        if self.saves.is_empty() {
            return;
        }
        let paths = self.saves.clone();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (sender, found) = std::sync::mpsc::channel();
        let stop = std::sync::Arc::clone(&cancel);
        let thread_paths = paths.clone();
        let spawned = std::thread::Builder::new().name("save-check".to_owned()).spawn(move || {
            for path in thread_paths {
                if stop.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                if let Some(reason) = checked_save_problem(&path)
                    && sender.send((path, reason)).is_err()
                {
                    break;
                }
            }
        });
        if spawned.is_ok() {
            self.save_check.0 = Some(SaveCheck { cancel, found });
        } else {
            // No thread to be had: the old way, on this one.
            for path in &paths {
                if let Some(reason) = checked_save_problem(path) {
                    self.errors.push(("Corrupted Save".to_owned(), crate::error_popup::corrupted_text(path, &reason)));
                }
            }
        }
    }

    /// Takes in what the background check of the saves has found since the last call (the window loop calls it every frame the menu is up).
    /// `true` when a popup came up, so the screen needs repainting.
    pub fn poll_background(&mut self) -> bool {
        let Some(check) = self.save_check.0.take() else {
            return false;
        };
        if self.view == MenuView::Loading {
            // A world is being read: the rest of the saves would only compete with it for the disk (dropping the check stops its thread).
            return false;
        }
        let mut raised = false;
        loop {
            match check.found.try_recv() {
                Ok((path, reason)) => {
                    self.errors.push(("Corrupted Save".to_owned(), crate::error_popup::corrupted_text(&path, &reason)));
                    raised = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // The thread is still reading.
                    self.save_check.0 = Some(check);
                    break;
                }
                // The thread has read them all.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
        raised
    }

    /// Whether the check of the saves is still running.
    #[must_use]
    pub fn save_check_running(&self) -> bool {
        self.save_check.0.is_some()
    }

    /// Waits for the check of the saves to finish and takes in what it found (tests, which have no window loop to do it).
    #[cfg(test)]
    fn finish_save_check(&mut self) {
        for _ in 0..4_000 {
            self.poll_background();
            if self.save_check.0.is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the check of the saves did not finish in 20 s");
    }

    /// The error popup that shows now, if any: (source, text).
    #[must_use]
    pub fn error_popup(&self) -> Option<(&str, &str)> {
        self.errors.first().map(|(source, text)| (source.as_str(), text.as_str()))
    }

    /// `LoadGamePanel.SelectSaveItem`: loads the details of one save.
    pub fn select_save(&mut self, index: Option<usize>) {
        self.selected_save = index;
        self.save_card = None;
        self.save_preview = None;
        self.confirm_delete = false;
        if let Some(path) = index.and_then(|i| self.saves.get(i)) {
            let card = bibites_io::storage::read_save_card(path);
            self.save_preview = card
                .preview_png
                .as_ref()
                .and_then(|bytes| crate::ui::decode_png_bytes(bytes).ok())
                .map(std::sync::Arc::new);
            self.save_card = Some(card);
        }
    }

    /// How many saves the Load list holds.
    #[must_use]
    pub fn save_count(&self) -> usize {
        self.saves.len()
    }

    /// One row of the Load list: `(name, last edit)`. The frame asks for the sixteen on screen, not for all of a list of 1,500.
    #[must_use]
    pub fn save_row(&self, index: usize) -> Option<(String, String)> {
        let path = self.saves.get(index)?;
        Some((
            path.file_stem().and_then(|name| name.to_str()).unwrap_or("save").to_owned(),
            self.save_dates.get(index).cloned().unwrap_or_default(),
        ))
    }

    /// The file of row `index` of the Load list.
    #[must_use]
    pub fn save_path(&self, index: usize) -> Option<PathBuf> {
        self.saves.get(index).cloned()
    }

    /// The row of the selected save.
    #[must_use]
    pub fn selected_save_row(&self) -> Option<(String, String)> {
        self.selected_save.and_then(|index| self.save_row(index))
    }

    /// Details of the selected save.
    #[must_use]
    pub fn save_card(&self) -> Option<&bibites_io::storage::SaveCard> {
        self.save_card.as_ref()
    }

    /// Preview image of the selected save.
    #[must_use]
    pub fn save_preview(&self) -> Option<std::sync::Arc<crate::ui::UiTexture>> {
        self.save_preview.clone()
    }

    /// Selected template row of the library.
    #[must_use]
    pub fn library_selected(&self) -> Option<usize> {
        self.library_selected
    }

    /// Makes the Library's picture (`BibiteTemplateGenePreviewer.UpdateTemplate`): the bibite of the selected
    /// template's genes, or the one a previewer with no template shows, at the size of its box
    /// (`pixels_per_unit` screen pixels to a canvas unit). The genes are read from the file once.
    pub fn refresh_library_picture(
        &mut self,
        assets: Option<&crate::assets::GameAssets>,
        previews: &crate::preview::PreviewCache,
        pixels_per_unit: f32,
    ) {
        // The New Game screen's champion (`ChampionPreview/BibitePreview`) is made the same way, once per champion.
        if self.view == MenuView::NewGame {
            self.refresh_champion_picture(assets, previews, pixels_per_unit);
            return;
        }
        if self.view != MenuView::Library {
            return;
        }
        let entry = self
            .library_selected
            .and_then(|index| self.templates.get(index))
            .map(|entry| (entry.path.clone(), entry.kind));
        let genome = match entry {
            Some((path, kind)) => self
                .library_genomes
                .entry(path.clone())
                .or_insert_with(|| {
                    let loaded = match kind {
                        bibites_io::storage::TemplateKind::Template => {
                            bibites_io::load_template(&path)
                        }
                        bibites_io::storage::TemplateKind::Genome => bibites_io::load_genome(&path),
                    };
                    loaded.ok().map(|loaded| loaded.genome)
                })
                .clone(),
            None => None,
        };
        // A file that does not read shows what no template does.
        let (genome, size) = match genome {
            Some(genome) => (genome, None),
            None => (crate::preview::no_template(), Some(1.0)),
        };
        let request = crate::preview::PreviewRequest {
            genome: &genome,
            size,
            style: crate::preview::SELECTOR,
            procedural: crate::preview::ProceduralSize::Both,
            picture: [LIBRARY_PICTURE.0, LIBRARY_PICTURE.1],
            pixels_per_unit,
        };
        self.library_picture = assets.and_then(|assets| previews.get(assets, &request));
    }

    /// The pointer, in menu pixels (the last `set_cursor`).
    #[must_use]
    pub fn cursor(&self) -> Vec2 {
        self.cursor
    }

    /// The Library's picture of the selected template's bibite, if it has been made.
    #[must_use]
    pub fn library_picture(&self) -> Option<std::sync::Arc<crate::ui::UiTexture>> {
        self.library_picture.clone()
    }

    /// The champion's bibite for the challenge panel (`BibiteGenePreviewer.UpdateTemplate` on the picked template).
    fn refresh_champion_picture(
        &mut self,
        assets: Option<&crate::assets::GameAssets>,
        previews: &crate::preview::PreviewCache,
        pixels_per_unit: f32,
    ) {
        let wanted = self
            .champion
            .as_ref()
            .map(|champion| (champion.path.clone(), (pixels_per_unit * 100.0) as i64));
        if self.champion_picture_for == wanted {
            return;
        }
        self.champion_picture_for = wanted.clone();
        self.champion_picture = None;
        let Some((path, _)) = wanted else {
            return;
        };
        let is_template = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bb8template"));
        let loaded = if is_template {
            bibites_io::load_template(&path)
        } else {
            bibites_io::load_genome(&path)
        };
        let Ok(loaded) = loaded else {
            return;
        };
        let request = crate::preview::PreviewRequest {
            genome: &loaded.genome,
            size: None,
            style: crate::preview::CHAMPION,
            procedural: crate::preview::ProceduralSize::Both,
            picture: [110.0, 110.0],
            pixels_per_unit,
        };
        self.champion_picture = assets.and_then(|assets| previews.get(assets, &request));
    }

    /// The champion's picture for the challenge panel, if it has been made.
    #[must_use]
    pub fn champion_picture(&self) -> Option<std::sync::Arc<crate::ui::UiTexture>> {
        self.champion_picture.clone()
    }

    /// The game's user settings (Settings screens).
    #[must_use]
    pub fn user_settings(&self) -> &crate::user_settings::UserSettings {
        &self.user_settings
    }

    /// Selected settings tab (0 graphics, 1 gameplay, 2 hotkeys).
    #[must_use]
    pub fn settings_tab(&self) -> u8 {
        self.settings_tab
    }

    /// Whether the delete confirmation is showing.
    #[must_use]
    pub fn confirm_delete(&self) -> bool {
        self.confirm_delete
    }

    /// Whether the "do not show again" checkbox of the warning popup that is up is ticked.
    #[must_use]
    pub fn confirm_checked(&self) -> bool {
        self.confirm_checked
    }

    /// Gives the menu a settings file of its own with every action warning stored as shown (which
    /// also hides the installed game's own flags), so a test never depends on the player's flags
    /// nor writes their file.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_private_settings(mut self, path: &Path) -> Self {
        self.user_settings = crate::user_settings::UserSettings::load(Some(path.to_path_buf()));
        // Every warning shown (`reset_warnings` leaves the file and workshop ones alone), whatever
        // the installed game remembers.
        for warning in crate::warnings::ALL {
            self.user_settings.set_warning_off(warning, false);
        }
        self
    }

    /// Points the Library at a data folder of a test (`Bibites/Templates`, `Bibites`) and reads it.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_library_root(mut self, root: &Path) -> Self {
        self.library_root = Some(root.to_path_buf());
        self.library_extra.clear();
        self.reload_library();
        self
    }

    /// [`Self::with_library_root`] with other folders listed after it.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_library_roots(mut self, root: &Path, extra: &[PathBuf]) -> Self {
        self.library_root = Some(root.to_path_buf());
        self.library_extra = extra.to_vec();
        self.reload_library();
        self
    }

    /// What the Library lists: the entries of its folder, then of the other folders.
    fn read_library(&self) -> Vec<bibites_io::storage::LibraryEntry> {
        list_library_roots(self.library_root.as_deref(), &self.library_extra)
    }

    /// The scenario editor's "launching the simulation" warning is up
    /// (`ActionWarnings.startScenarioAfterEdit`).
    #[must_use]
    pub fn start_warning_up(&self) -> bool {
        self.editor.as_ref().is_some_and(|editor| {
            matches!(
                editor.popup,
                Some(crate::scenario_editor::EditorPopup::StartWarning)
            )
        })
    }

    /// Whether the player switched the action warning off with its "do not show again" checkbox.
    #[must_use]
    pub fn warning_off(&self, warning: crate::warnings::ActionWarning) -> bool {
        self.user_settings.warning_off(warning)
    }

    /// The Library's tab: 0 Templates, 1 Saved.
    #[must_use]
    pub fn library_tab(&self) -> u8 {
        self.library_tab
    }

    /// The Library's delete warning is up.
    #[must_use]
    pub fn confirm_library_delete(&self) -> bool {
        self.confirm_library_delete
    }

    /// Indices (into [`Self::templates`]) of the entries the open tab lists: the `.bb8template`
    /// files under Templates, the `.bb8` files under Saved.
    #[must_use]
    pub fn library_rows(&self) -> Vec<usize> {
        let wanted = if self.library_tab == 0 {
            bibites_io::storage::TemplateKind::Template
        } else {
            bibites_io::storage::TemplateKind::Genome
        };
        self.templates
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.kind == wanted)
            .map(|(index, _)| index)
            .collect()
    }

    /// The Delete button is enabled while a bibite that is not an official one is selected
    /// (`ItemClicked`: `!isOfficial && !isExternal`; there is no workshop here).
    #[must_use]
    pub fn library_can_delete(&self) -> bool {
        self.library_selected
            .and_then(|index| self.templates.get(index))
            .is_some_and(|entry| !entry.is_official)
    }

    /// `FillLists`: the folders are read again and the first bibite is selected.
    fn reload_library(&mut self) {
        self.templates = self.read_library();
        // The genes the picture was made from were read once per file: a template saved again under its name (the Bibite Editor, the Save dialog) has others now.
        self.library_genomes.clear();
        self.library_selected = (!self.templates.is_empty()).then_some(0);
        self.confirm_library_delete = false;
        self.scrolls[scroll_index(MenuView::Library)] = 0.0;
    }

    /// `EditSelectedBibite`: the Bibite Editor on the selected template or saved bibite (a new one when nothing is selected).
    fn edit_selected_template(&self) -> MenuAction {
        MenuAction::EditTemplate {
            path: self
                .library_selected
                .and_then(|index| self.templates.get(index))
                .map(|entry| entry.path.clone()),
            submit: true,
        }
    }

    /// The Library after its editor closed: `AfterEditorSubmit` (`saved` is the name of the template that was saved and submitted)
    /// fills the lists again and selects the template of that name; `AfterEditorClose` (`saved` is `None`) fills them again and clicks
    /// the item that was selected.
    pub fn refill_library(&mut self, saved: Option<&str>) {
        let before = self
            .library_selected
            .and_then(|index| self.templates.get(index))
            .map(|entry| entry.path.clone());
        let first = self.library_selected.is_some();
        self.templates = self.read_library();
        // (See `reload_library`: the editor that just closed may have saved the template that is selected.)
        self.library_genomes.clear();
        self.confirm_library_delete = false;
        self.library_selected = match saved {
            // `activeItems.FirstOrDefault(t => t.template.name == template.name)` of the Templates list: none selects nothing.
            Some(name) => self.templates.iter().position(|entry| {
                entry.kind == bibites_io::storage::TemplateKind::Template && entry.name == name
            }),
            None => before
                .and_then(|path| self.templates.iter().position(|entry| entry.path == path))
                .or_else(|| (first && !self.templates.is_empty()).then_some(0)),
        };
    }

    /// `DeleteSelected`: the file goes, and the first bibite of the same list is selected. A file
    /// that cannot be deleted says so (the game shows its error popup).
    fn delete_library_entry(&mut self) {
        let Some(index) = self.library_selected else {
            return;
        };
        let Some(entry) = self.templates.get(index).cloned() else {
            return;
        };
        if entry.is_official {
            return;
        }
        match std::fs::remove_file(&entry.path) {
            Ok(()) => {
                self.templates.remove(index);
                self.library_selected = self
                    .templates
                    .iter()
                    .position(|other| other.kind == entry.kind);
                self.scrolls[scroll_index(MenuView::Library)] = 0.0;
            }
            Err(error) => self.status = format!("Couldn't delete template: {error}"),
        }
    }

    /// Deletes the selected save file and refreshes the list (`LoadGamePanel.DeleteSelectedSave`).
    fn delete_selected_save_file(&mut self) {
        let Some(path) = self.selected_save_path().map(Path::to_path_buf) else {
            return;
        };
        match std::fs::remove_file(&path) {
            Ok(()) => {
                let keep = self.selected_save;
                self.reload_saves();
                let last = self.saves.len().checked_sub(1);
                self.select_save(keep.and_then(|i| last.map(|l| i.min(l))));
            }
            Err(error) => self.status = format!("Could not delete save: {error}"),
        }
    }

    /// Path of the selected save.
    #[must_use]
    pub fn selected_save_path(&self) -> Option<&Path> {
        self.selected_save
            .and_then(|index| self.saves.get(index))
            .map(PathBuf::as_path)
    }

    /// Entries shown by the current list (scenarios or challenges).
    #[must_use]
    pub fn visible_entries(&self) -> Vec<usize> {
        self.scenario_entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.is_challenge == self.showing_challenges)
            .map(|(index, _)| index)
            .collect()
    }

    /// All scenario cards.
    #[must_use]
    pub fn scenario_entries(&self) -> &[ScenarioEntry] {
        &self.scenario_entries
    }

    /// Whether the challenge list is showing.
    #[must_use]
    pub fn showing_challenges(&self) -> bool {
        self.showing_challenges
    }

    /// Currently selected scenario card.
    #[must_use]
    pub fn selected_scenario(&self) -> Option<&ScenarioEntry> {
        self.selected_entry
            .and_then(|index| self.scenario_entries.get(index))
    }

    /// Selected card index (into [`MenuState::scenario_entries`]).
    #[must_use]
    pub fn selected_entry_index(&self) -> Option<usize> {
        self.selected_entry
    }

    /// Decoded preview image of the selected scenario.
    #[must_use]
    pub fn selected_preview(&self) -> Option<std::sync::Arc<crate::ui::UiTexture>> {
        self.selected_preview.clone()
    }

    /// Scroll offset of the list or tab on screen (reference px).
    #[must_use]
    pub fn list_scroll(&self) -> f32 {
        self.scrolls[scroll_index(self.view)]
    }

    /// The scroll offset [`MenuState::list_scroll`] reads, to change.
    fn scroll_slot(&mut self) -> &mut f32 {
        &mut self.scrolls[scroll_index(self.view)]
    }

    /// How the scroll view of the current screen differs from its place in the capture (see the `list_delta` field).
    #[must_use]
    pub fn list_delta(&self) -> [f32; 4] {
        self.list_delta.get()
    }

    /// Tells the menu where the scroll view of its screen is in the canvas it is drawn in (`ui_layout`): the lists, their bars and what they scroll
    /// follow the view.
    pub fn set_list_delta(&self, delta: [f32; 4]) {
        self.list_delta.set(delta);
    }

    /// How the New Game screen's parameters panel (the TPS settings popup is drawn over it) differs from its place in the capture.
    #[must_use]
    pub fn tps_delta(&self) -> [f32; 4] {
        self.tps_delta.get()
    }

    /// Tells the menu where the parameters panel is in the canvas it is drawn in.
    pub fn set_tps_delta(&self, delta: [f32; 4]) {
        self.tps_delta.set(delta);
    }

    /// The scrollbar of the list the current screen shows (`Scrollbar Vertical` of its `ScrollRect`, at the view's right edge), there only when the rows
    /// do not fit: the Load Game list's (20 wide, 17 short of the view: the content is 15 + the "Include autosaves" line (50) + the rows, 55 each less
    /// the last spacing, + 50 = 110 + 55 a row, in a view 930 high), the scenario list's (16 wide at (733.2, 108), 946 high, rows 70 apart), the
    /// Library's (12 wide at (1114.8, 91), 963 high; the rows 56 apart start 82 under the view's top), the Settings pages' and the Credits and Additional
    /// Tools tabs' (20 wide at (1875, 100), 955 high: their views are as high as that).
    #[must_use]
    pub fn list_bar(&self) -> Option<crate::scrollbar::VBar> {
        use crate::scrollbar::{Style, VBar};
        let scroll = self.list_scroll();
        let [dx, dy, dw, dh] = self.list_delta();
        let mut bar = match self.view {
            MenuView::LoadGame => VBar::new(
                [890.0, 75.0, 20.0, 913.0],
                110.0 + 55.0 * self.saves.len() as f32,
                930.0,
                scroll,
                Style::SAVES,
            ),
            MenuView::NewGame => VBar::new(
                [733.2, 108.0, 16.0, 946.0],
                self.visible_entries().len() as f32 * 70.0,
                946.0,
                scroll,
                Style::SCENARIOS,
            ),
            MenuView::Library => VBar::new(
                [1114.8, 91.0, 12.0, 963.0],
                self.library_rows().len() as f32 * 56.0 + 83.0,
                963.0,
                scroll,
                Style::LIBRARY,
            ),
            MenuView::Credits => VBar::new(
                [1875.0, 100.0, 20.0, TAB_VIEW],
                CREDITS_CONTENT,
                TAB_VIEW,
                scroll,
                Style::TABS,
            ),
            MenuView::Tools => VBar::new(
                [1875.0, 100.0, 20.0, TAB_VIEW],
                TOOLS_CONTENT,
                TAB_VIEW,
                scroll,
                Style::TABS,
            ),
            // The Settings pages (`PagesPanels/<page>/Scroll View`: a view 920 high, the bar 20 wide, 17 short): the content is what the captures
            // measured for the Graphics (1035), Game (1415) and Hotkeys (1384.8) pages.
            MenuView::Settings => VBar::new(
                [1254.3, 80.0, 20.0, 903.0],
                match self.settings_tab {
                    0 => 1035.0,
                    1 => 1415.0,
                    // The Music, Species Names, Towers, Extra Hotkeys and Calibration pages fit their view.
                    3..=7 => 920.0,
                    _ => 1384.8,
                },
                920.0,
                scroll,
                Style::SAVES,
            ),
            _ => return None,
        };
        // The view of another canvas: the bar stays on its right edge and as long as the view's change says (the content is what it was).
        bar.track = [bar.track[0] + dx + dw, bar.track[1] + dy, bar.track[2], bar.track[3] + dh];
        bar.view += dh;
        bar.visible().then_some(bar)
    }

    /// Whether the scrollbar's handle is held.
    #[must_use]
    pub fn bar_dragging(&self) -> bool {
        self.bar_grab.is_some()
    }

    /// The held button moves the handle (`point` in the window's pixels, `width` x `height` its size): true when the list scrolled.
    pub fn bar_drag_move(&mut self, point: Vec2, width: f32, height: f32) -> bool {
        let (Some(grab), Some(bar)) = (self.bar_grab, self.list_bar()) else {
            return false;
        };
        let scale = crate::ui::UiFrame::scaler(width, height).max(f32::EPSILON);
        let scroll = bar.scroll_for_handle_top(point.y / scale - grab);
        let moved = (scroll - self.list_scroll()).abs() > f32::EPSILON;
        *self.scroll_slot() = scroll;
        moved
    }

    /// A press on a screen that scrolls waits for its release (a drag moves the content, a still press is the click): true when it was held back.
    pub fn press_hold(&mut self, point: Vec2, width: f32, height: f32) -> bool {
        if self.dropdown.is_some() || self.editor.is_some() {
            return false;
        }
        let Some(bar) = self.list_bar() else {
            return false;
        };
        let scale = crate::ui::UiFrame::scaler(width, height).max(f32::EPSILON);
        let track = bar.track;
        let (x, y) = (point.x / scale, point.y / scale);
        if x >= track[0] - 4.0 && x <= track[0] + track[2] + 4.0 && y >= track[1] && y <= track[1] + track[3] {
            return false;
        }
        self.inertia = 0.0;
        self.content_drag = Some((Vec2::new(x, y), self.list_scroll(), false));
        true
    }

    /// The held button moves the content once it has gone past the drag threshold (rubber band past the ends); true when the list moved.
    pub fn content_drag_move(&mut self, point: Vec2, width: f32, height: f32, dt: f32) -> bool {
        let Some((start, start_scroll, moved)) = self.content_drag else {
            return false;
        };
        let scale = crate::ui::UiFrame::scaler(width, height).max(f32::EPSILON);
        let dy = point.y / scale - start.y;
        let moved = moved || dy.abs() > 10.0;
        self.content_drag = Some((start, start_scroll, moved));
        if !moved {
            return false;
        }
        let max = self.scroll_max();
        let wanted = start_scroll - dy;
        let over = if wanted < 0.0 { wanted } else if wanted > max { wanted - max } else { 0.0 };
        let view = 946.0 + self.list_delta()[3];
        let place = wanted.clamp(0.0, max)
            + if over == 0.0 { 0.0 } else { (1.0 - 1.0 / (over.abs() * 0.55 / view + 1.0)) * view * over.signum() };
        let before = self.list_scroll();
        *self.scroll_slot() = place;
        self.inertia = self.inertia * 0.5 + (place - before) / dt.max(1e-4) * 0.5;
        (place - before).abs() > f32::EPSILON
    }

    /// Whether a press is being held back (it may still become a drag or a click).
    #[must_use]
    pub fn content_dragging(&self) -> bool {
        self.content_drag.is_some()
    }

    /// The button was let go: the press point when it was a click (nothing moved), `None` after a drag (the content keeps moving).
    pub fn content_release(&mut self, width: f32, height: f32) -> Option<Vec2> {
        let (start, _, moved) = self.content_drag.take()?;
        if moved {
            return None;
        }
        self.inertia = 0.0;
        let scale = crate::ui::UiFrame::scaler(width, height).max(f32::EPSILON);
        Some(start * scale)
    }

    /// The button was let go: the handle is not held any more.
    pub fn bar_release(&mut self) {
        self.bar_grab = None;
    }

    /// Whether a press at `point` (menu pixels) lands on a slider of the current screen (a Settings row's or a TPS one). The slider takes
    /// such a press at once and holds it (`Slider.OnPointerDown`, then `OnDrag` until the button is let go), where a press elsewhere on a
    /// scrolling page waits for its release (a drag of the page, or a click).
    #[must_use]
    pub fn slider_under(
        &self,
        ui: &UiAssets,
        fonts: &mut FontSet,
        width: f32,
        height: f32,
        runtime: Option<&RuntimeUi>,
        point: Vec2,
    ) -> bool {
        if matches!(self.view, MenuView::Main | MenuView::Loading) || self.dropdown.is_some() || self.editor.is_some() {
            return false;
        }
        let frame = self.build_frame(ui, fonts, width, height, runtime);
        matches!(
            frame.action_at(point),
            Some(crate::ui::UiAction::Menu(
                crate::ui::MenuCommand::SettingSlide(..) | crate::ui::MenuCommand::TpsSlide(..)
            ))
        )
    }

    /// Whether a slider is held.
    #[must_use]
    pub fn slider_dragging(&self) -> bool {
        self.slider_drag.is_some()
    }

    /// The held slider follows the pointer (`point` in menu pixels, like a click): true when its value changed, so the screen has to be
    /// drawn again. The pointer may leave the slider and its row, the value stays at the track's ends (`Slider.UpdateDrag` clamps).
    pub fn slider_drag_move(&mut self, point: Vec2) -> bool {
        let Some(slider) = self.slider_drag else {
            return false;
        };
        match slider {
            HeldSlider::Setting(index, x0, x1, grab) => {
                let span = f32::from(x1) - f32::from(x0);
                span > 0.0
                    && self
                        .user_settings
                        .set_fraction(usize::from(index), (point.x - grab - f32::from(x0)) / span)
            }
            HeldSlider::Tps(index, x0, x1, grab) => {
                let span = f32::from(x1) - f32::from(x0);
                if span <= 0.0 {
                    return false;
                }
                let index = usize::from(index).min(4);
                let before = self.tps_values[index];
                self.tps_set_fraction(index, (point.x - grab - f32::from(x0)) / span);
                self.tps_values[index] != before
            }
        }
    }

    /// The button was let go: no slider is held any more.
    pub fn slider_release(&mut self) {
        self.slider_drag = None;
    }

    /// What one wheel notch moves a list of the current screen: the `ScrollRect.scrollSensitivity` of its prefab (100 for the scenario,
    /// save and template lists and the settings pages, 35 for the Tools and Credits tabs).
    #[must_use]
    pub fn wheel_step(&self) -> f32 {
        match self.view {
            MenuView::Tools | MenuView::Credits => 35.0,
            _ => 100.0,
        }
    }

    /// Scrolls the scenario/save list by `delta` reference px.
    pub fn scroll_list(&mut self, delta: f32) {
        // An open dropdown list takes the wheel.
        if let Some(open) = self.dropdown.as_mut() {
            open.wheel(-delta / 70.0);
            return;
        }
        if let Some(editor) = self.editor.as_deref_mut()
            && self.view == MenuView::ScenarioEditor
        {
            editor.scroll_by(delta);
            return;
        }
        let max = self.scroll_max();
        // `ScrollRect` with the elastic movement type: the wheel may push the content past its ends (`OnScroll` does not clamp it) and
        // [`MenuState::spring`] pulls it back.
        let slot = self.scroll_slot();
        let overshoot = if max > 0.0 { ELASTIC_LIMIT } else { 0.0 };
        *slot = (*slot + delta).clamp(-overshoot, max + overshoot);
    }

    /// How far the list of the current screen scrolls.
    fn scroll_max(&self) -> f32 {
        let rows = self.visible_entries().len().max(self.saves.len()) as f32;
        if let Some(bar) = self.list_bar() {
            // The scenario, Load Game and Library lists and the Settings pages scroll as far as their scrollbar says.
            bar.max_scroll()
        } else if matches!(
            self.view,
            MenuView::LoadGame | MenuView::NewGame | MenuView::Library | MenuView::Settings
        ) {
            0.0
        } else {
            (rows * 70.0 - 946.0 - self.list_delta()[3]).max(0.0)
        }
    }

    /// `ScrollRect.LateUpdate` for the elastic movement: a list past its end is pulled back by `SmoothDamp` with the prefab's `elasticity` (Unity's
    /// default 0.1) as its smooth time.
    pub fn spring(&mut self, dt: f32) {
        let max = self.scroll_max();
        if self.content_drag.is_some() {
            return;
        }
        if self.inertia.abs() > 1.0 {
            let place = *self.scroll_slot();
            if place >= 0.0 && place <= max {
                *self.scroll_slot() = place + self.inertia * dt;
                self.inertia *= 0.135_f32.powf(dt);
                let now = *self.scroll_slot();
                if now < 0.0 || now > max {
                    self.scroll_speed = self.inertia;
                    self.inertia = 0.0;
                }
                return;
            }
        }
        self.inertia = 0.0;
        let place = *self.scroll_slot();
        let target = place.clamp(0.0, max);
        if (place - target).abs() < 0.01 {
            *self.scroll_slot() = target;
            self.scroll_speed = 0.0;
            return;
        }
        let omega = 2.0 / ELASTICITY;
        let x = omega * dt;
        let exp = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
        let change = place - target;
        let temp = (self.scroll_speed + omega * change) * dt;
        self.scroll_speed = (self.scroll_speed - omega * temp) * exp;
        *self.scroll_slot() = target + (change + temp) * exp;
    }

    /// Ends the elastic overshoot at once (the list rests at its end).
    pub fn settle_scroll(&mut self) {
        let max = self.scroll_max();
        let slot = self.scroll_slot();
        *slot = slot.clamp(0.0, max);
        self.scroll_speed = 0.0;
    }

    /// Dropdown labels of the five scenario modifiers (size, biomass density,
    /// fertility, radioactivity, bibite limit).
    #[must_use]
    pub fn modifier_labels(&self) -> [&'static str; 5] {
        [
            NEWGAME_SIZE_OPTIONS[self.newgame_options[0]],
            NEWGAME_BIOMASS_OPTIONS[self.newgame_options[1]],
            NEWGAME_FERTILITY_OPTIONS[self.newgame_options[2]],
            NEWGAME_RADIO_OPTIONS[self.newgame_options[3]],
            NEWGAME_LIMIT_OPTIONS[self.newgame_options[4]],
        ]
    }

    /// Option lists of the five modifiers.
    #[must_use]
    pub fn modifier_options(index: usize) -> &'static [&'static str] {
        match index {
            0 => &NEWGAME_SIZE_OPTIONS,
            1 => &NEWGAME_BIOMASS_OPTIONS,
            2 => &NEWGAME_FERTILITY_OPTIONS,
            3 => &NEWGAME_RADIO_OPTIONS,
            _ => &NEWGAME_LIMIT_OPTIONS,
        }
    }

    /// The items of a New Game modifier's list. The first three are `FloatSettingDropdown`s over the
    /// scenario settings' landmarks: the item's tooltip is "Setting name: Landmark" over the value
    /// (the landmark's own text for "Sterile"). Radioactivity and the limit are `EnumDropdown`s:
    /// their items have no tooltip.
    #[must_use]
    pub fn modifier_choices(index: usize) -> Vec<crate::dropdown::Choice> {
        use crate::dropdown::Choice;
        // `ScenarioIndependentSettings`: the setting's `Name`, `precision`, `units` and landmarks.
        let (name, precision, units, values): (&str, i32, &str, &[f32]) = match index {
            0 => ("Size of the Simulation Area", 2, " u", &NEWGAME_SIZE_VALUES),
            1 => (
                "Global Biomass Density",
                1,
                " E/u²",
                &NEWGAME_BIOMASS_VALUES,
            ),
            2 => ("Global Fertility", 1, " E/u²s", &NEWGAME_FERTILITY_VALUES),
            _ => {
                return Self::modifier_options(index)
                    .iter()
                    .map(|label| Choice::plain(*label))
                    .collect();
            }
        };
        Self::modifier_options(index)
            .iter()
            .zip(values)
            .enumerate()
            .map(|(at, (label, value))| {
                let text =
                    (index == 2 && at == 0).then_some("No Growth at all (or very close to none)");
                Choice::of_landmark(name, *label, *value, precision, units, text)
            })
            .collect()
    }

    /// The open dropdown list, if any.
    #[must_use]
    #[cfg(test)]
    pub fn dropdown_list(&self) -> Option<&crate::dropdown::Open> {
        self.dropdown.as_ref()
    }

    /// Tells the menu where the pointer is (screen px), so the open list can light its row.
    /// Returns whether that changes what is drawn.
    pub fn set_cursor(&mut self, cursor: Vec2) -> bool {
        let moved = cursor != self.cursor;
        self.cursor = cursor;
        moved && self.dropdown.is_some()
    }

    /// The options, the current choice and the look of the list of dropdown `target`.
    #[must_use]
    pub fn dropdown_choices(
        &self,
        target: crate::ui::DropdownTarget,
    ) -> Option<(Vec<crate::dropdown::Choice>, usize, crate::dropdown::Style)> {
        use crate::dropdown::{Choice, ROBOTO_LARGE};
        use crate::ui::DropdownTarget;
        match target {
            DropdownTarget::Modifier(index) => {
                let slot = usize::from(index).min(4);
                Some((
                    Self::modifier_choices(slot),
                    self.newgame_options[slot],
                    ROBOTO_LARGE,
                ))
            }
            DropdownTarget::Setting(index) => {
                let index = usize::from(index);
                let labels = crate::user_settings::UserSettings::choice_labels(index)?;
                let landmarks = crate::user_settings::UserSettings::landmark_values(index);
                let name = crate::user_settings::DEFS[index].label;
                Some((
                    labels
                        .iter()
                        .enumerate()
                        .map(|(at, label)| match landmarks {
                            // `NumericSettingDropdown.OnItemHovered`.
                            Some((values, precision, units)) => Choice::of_landmark(
                                name, *label, values[at], precision, units, None,
                            ),
                            None => Choice::plain(*label),
                        })
                        .collect(),
                    self.user_settings.current_choice(index)?,
                    ROBOTO_LARGE,
                ))
            }
            DropdownTarget::Zone(..)
            | DropdownTarget::Bibite(..)
            | DropdownTarget::AnimatorPreset
            | DropdownTarget::AnimatorBase(_)
            | DropdownTarget::AnimatorInterpolation(_) => self.editor.as_deref()?.dropdown_choices(target),
            _ => None,
        }
    }

    /// `TMP_Dropdown.Show`: the list of `target` hangs under the dropdown at `anchor` (reference px);
    /// clicking the dropdown of the open list closes it.
    pub fn open_dropdown_list(&mut self, target: crate::ui::DropdownTarget, anchor: [f32; 4]) {
        if self
            .dropdown
            .as_ref()
            .is_some_and(|open| open.target == target)
        {
            self.dropdown = None;
            return;
        }
        self.dropdown = self
            .dropdown_choices(target)
            .map(|(choices, selected, style)| {
                crate::dropdown::Open::new(target, anchor, choices, selected, style)
            });
    }

    /// A pick in the open list: the control takes the option and the list closes.
    pub fn pick_dropdown_list(&mut self, index: usize) {
        use crate::ui::DropdownTarget;
        let Some(open) = self.dropdown.take() else {
            return;
        };
        if index >= open.choices.len() {
            return;
        }
        match open.target {
            DropdownTarget::Modifier(slot) => {
                let slot = usize::from(slot).min(4);
                self.newgame_options[slot] = index;
                self.newgame_dirty[slot] = true;
            }
            DropdownTarget::Setting(row) => {
                self.user_settings.pick_choice(usize::from(row), index);
            }
            DropdownTarget::Zone(..)
            | DropdownTarget::Bibite(..)
            | DropdownTarget::AnimatorPreset
            | DropdownTarget::AnimatorBase(_)
            | DropdownTarget::AnimatorInterpolation(_) => {
                if let Some(editor) = self.editor.as_deref_mut() {
                    editor.pick_dropdown(open.target, index);
                }
            }
            _ => {}
        }
    }

    /// Whether "include autosaves" is checked in the Load Game panel.
    #[must_use]
    pub fn include_autosaves(&self) -> bool {
        self.include_autosaves
    }

    /// Selected row of the Load Game list.
    #[must_use]
    pub fn selected_save(&self) -> Option<usize> {
        self.selected_save
    }

    /// Best stored result of the selected challenge (`score`, `stars`, champion).
    #[must_use]
    pub fn challenge_best(&self) -> Option<&(f32, u8, String)> {
        self.challenge_best.as_ref()
    }

    fn select_first_visible(&mut self) {
        let first = self.visible_entries().first().copied();
        self.select_entry(first);
    }

    /// `ScenarioSelectorPanel.SelectScenarioItem`: selects a card and mirrors the
    /// scenario's settings into the modifier dropdowns.
    pub fn select_entry(&mut self, index: Option<usize>) {
        self.selected_entry = index;
        self.dropdown = None;
        self.challenge_best = None;
        // `DeserializeScenario` loads the settings the scenario file defines over the
        // panel's values (official 0.6 scenarios define none of the TPS settings).
        if let Some(entry) = index.and_then(|i| self.scenario_entries.get(i))
            && let Ok(defined) =
                bibites_io::bundle::scenario_defined_numbers(&entry.path, &TPS_SETTING_KEYS)
        {
            for (slot, value) in self.tps_values.iter_mut().zip(defined) {
                if let Some(value) = value {
                    *slot = value.round() as i32;
                }
            }
        }
        self.newgame_scenario = index;
        self.selected_preview = None;
        self.newgame_dirty = [false; 5];
        let Some(entry) = index.and_then(|i| self.scenario_entries.get(i)) else {
            return;
        };
        self.selected_preview = entry
            .preview_png
            .as_ref()
            .and_then(|bytes| crate::ui::decode_png_bytes(bytes).ok())
            .map(std::sync::Arc::new);
        if let Ok(bundle) = bibites_io::load_scenario_bundle(&entry.path) {
            let settings = bibites_io::world::scenario_effective_settings(&bundle, None);
            use bibites_core::settings::SettingValue;
            let float = |value: Option<SettingValue>| match value {
                Some(SettingValue::Float(value)) => Some(value),
                Some(SettingValue::Int(value)) => Some(value as f32),
                _ => None,
            };
            if let Some(size) = float(settings.independents.get_by_key("SimulationSize")) {
                self.newgame_options[0] = nearest_index(&NEWGAME_SIZE_VALUES, size);
            }
            if let Some(density) = float(settings.independents.get_by_key("biomassDensity")) {
                self.newgame_options[1] = nearest_index(&NEWGAME_BIOMASS_VALUES, density);
            }
            if let Some(fertility) = float(settings.independents.get_by_key("pelletGrowth")) {
                self.newgame_options[2] = nearest_index(&NEWGAME_FERTILITY_VALUES, fertility);
            }
            if let Some(chance) = float(settings.scenario.get_by_key("backgroundMutationChance")) {
                let chances: Vec<f32> = NEWGAME_RADIO_LEVELS.iter().map(|level| level.0).collect();
                self.newgame_options[3] = nearest_index(&chances, chance);
            }
            let limited = matches!(
                settings.independents.get_by_key("limitBibiteBirth"),
                Some(SettingValue::Bool(true))
            );
            self.newgame_options[4] = if limited {
                match float(settings.independents.get_by_key("bibiteLimit")) {
                    Some(limit) => nearest_index(&[50.0, 100.0, 200.0, 500.0], limit),
                    None => 4,
                }
            } else {
                4
            };
        }
        if entry.is_challenge {
            let name = entry.name.clone();
            // Results are recorded in the viewer's own storage (`App::storage`); the
            // installed game's stored bests count too.
            let paths = bibites_io::storage::StoragePaths::default_local();
            let high_is_better = bibites_io::load_scenario_bundle(&entry.path)
                .ok()
                .and_then(|bundle| bundle.settings_file.challenge)
                .is_none_or(|challenge| challenge.high_score_is_better);
            if let Ok(Some(best)) = bibites_io::storage::challenge_high_score_with_prefs(
                &paths,
                &name,
                bibites_io::prefs::GamePrefs::global(),
                high_is_better,
            ) {
                self.challenge_best = Some((
                    best.score.unwrap_or(f32::NAN),
                    best.stars,
                    best.champion_name,
                ));
            }
        }
    }

    /// Executes a command emitted by a native menu panel.
    pub fn menu_command(&mut self, command: crate::ui::MenuCommand) -> MenuAction {
        use crate::ui::MenuCommand;
        match command {
            MenuCommand::Back => {
                if self.pick.is_some() {
                    self.cancel_pick();
                } else {
                    self.back_to_main();
                }
                MenuAction::None
            }
            MenuCommand::ToggleChallenges => {
                self.showing_challenges = !self.showing_challenges;
                // `SwitchList(false)` forgets the champion (`selectedChampion = null`); `SwitchList(true)` leaves it.
                if self.showing_challenges {
                    self.champion = None;
                }
                self.select_first_visible();
                MenuAction::None
            }
            // The challenge list has no scenario modifiers (`ParametersPanel` is `showWithScenarios`): no "Start Simulation" there.
            MenuCommand::StartSimulation if self.showing_challenges => MenuAction::None,
            MenuCommand::SelectChampion if self.showing_challenges => {
                self.begin_pick(TemplatePick::Champion);
                MenuAction::None
            }
            MenuCommand::SelectChampion => MenuAction::None,
            MenuCommand::StartChallenge(as_challenge) => self.start_challenge(as_challenge),
            MenuCommand::SelectScenario(row) => {
                let entry = self.visible_entries().get(usize::from(row)).copied();
                self.select_entry(entry);
                MenuAction::None
            }
            MenuCommand::StartSimulation => {
                if self.selected_entry.is_none() {
                    self.status = "No scenario selected".to_owned();
                    return MenuAction::None;
                }
                self.generate_newgame()
            }
            MenuCommand::TpsSettings => {
                self.tps_open = !self.tps_open;
                self.dropdown = None;
                MenuAction::None
            }
            MenuCommand::TpsSlide(..) => MenuAction::None, // applied by `click_native`
            MenuCommand::TpsReset(index) => {
                let index = usize::from(index).min(4);
                self.tps_values[index] = self.user_settings.tps_defaults()[index];
                MenuAction::None
            }
            MenuCommand::TpsSaveDefault => {
                self.user_settings.set_tps_defaults(self.tps_values);
                MenuAction::None
            }
            MenuCommand::OpenFolder => {
                let folder = self
                    .data_roots
                    .first()
                    .map(|root| root.join("Scenarios"))
                    .filter(|path| path.is_dir());
                match folder {
                    Some(folder) => {
                        let _ = std::process::Command::new("explorer").arg(folder).spawn();
                    }
                    None => self.status = "Scenarios folder not found".to_owned(),
                }
                MenuAction::None
            }
            MenuCommand::SelectSave(row) => {
                if usize::from(row) < self.saves.len() {
                    self.select_save(Some(usize::from(row)));
                }
                MenuAction::None
            }
            MenuCommand::ListBarHandle => MenuAction::None,
            MenuCommand::ListBarPage(direction) => {
                if let Some(bar) = self.list_bar() {
                    *self.scroll_slot() = bar.paged(f32::from(direction));
                }
                MenuAction::None
            }
            MenuCommand::LoadSelected => match self.selected_save {
                Some(index) => self.start_save(index),
                None => {
                    self.status = "Select a save first".to_owned();
                    MenuAction::None
                }
            },
            MenuCommand::ToggleAutosaves => {
                self.include_autosaves = !self.include_autosaves;
                // The checkbox is the user setting `IncludeAutoSavesInLoadPanel`.
                if let Some(index) = crate::user_settings::index_of("Include autosaves")
                    && self.user_settings.flag("Include autosaves") != self.include_autosaves
                {
                    self.user_settings.toggle(index);
                }
                self.reload_saves();
                self.report_corrupted_saves();
                MenuAction::None
            }
            MenuCommand::DeleteSave => {
                if self.selected_save.is_none() {
                    return MenuAction::None;
                }
                // `PopupManager.DisplayActionWarning`: once "do not show again" was ticked the
                // save goes at once.
                if self.warning_off(crate::warnings::ActionWarning::DeleteSaveFile) {
                    self.delete_selected_save_file();
                } else {
                    self.confirm_delete = true;
                    self.confirm_checked = false;
                }
                MenuAction::None
            }
            MenuCommand::ConfirmDelete(yes) => {
                let ticked = std::mem::take(&mut self.confirm_checked);
                self.confirm_delete = false;
                if yes {
                    // `WarningHandle.Dismiss`: a ticked box switches the warning off for good.
                    if ticked {
                        self.user_settings
                            .set_warning_off(crate::warnings::ActionWarning::DeleteSaveFile, true);
                    }
                    self.delete_selected_save_file();
                }
                MenuAction::None
            }
            MenuCommand::DismissError => {
                if !self.errors.is_empty() {
                    self.errors.remove(0);
                }
                MenuAction::None
            }
            MenuCommand::ToggleDoNotShowAgain => {
                if self.confirm_delete || self.confirm_library_delete || self.start_warning_up() {
                    self.confirm_checked = !self.confirm_checked;
                }
                MenuAction::None
            }
            // `BibiteTemplateSelectorPanel.OpenTab`: only the shown list changes, the selected
            // bibite stays.
            MenuCommand::LibraryTab(tab) => {
                self.library_tab = tab.min(1);
                self.scrolls[scroll_index(MenuView::Library)] = 0.0;
                MenuAction::None
            }
            MenuCommand::LibraryReload => {
                self.reload_library();
                MenuAction::None
            }
            // `OpenSaveFolder`: the `Bibites` folder in the explorer.
            MenuCommand::LibraryOpenFolder => {
                let folder = self
                    .library_root
                    .as_ref()
                    .map(|root| root.join("Bibites"))
                    .filter(|path| path.is_dir());
                match folder {
                    Some(folder) => {
                        let _ = std::process::Command::new("explorer").arg(folder).spawn();
                    }
                    None => self.status = "Bibites folder not found".to_owned(),
                }
                MenuAction::None
            }
            // `DeleteSelectedAction`: the file-deletion warning asks first, unless it was
            // switched off.
            MenuCommand::LibraryDelete => {
                if !self.library_can_delete() {
                    return MenuAction::None;
                }
                if self.warning_off(crate::warnings::ActionWarning::DeleteBibiteFile) {
                    self.delete_library_entry();
                } else {
                    self.confirm_library_delete = true;
                    self.confirm_checked = false;
                }
                MenuAction::None
            }
            MenuCommand::ConfirmLibraryDelete(yes) => {
                let ticked = std::mem::take(&mut self.confirm_checked);
                self.confirm_library_delete = false;
                if yes {
                    // `WarningHandle.Dismiss`: a ticked box switches the warning off for good.
                    if ticked {
                        self.user_settings.set_warning_off(
                            crate::warnings::ActionWarning::DeleteBibiteFile,
                            true,
                        );
                    }
                    self.delete_library_entry();
                }
                MenuAction::None
            }
            // `UserSettingsManager.ResetAllWarnings`: nothing changes on screen.
            MenuCommand::ResetWarnings => {
                self.user_settings.reset_warnings();
                MenuAction::None
            }
            // The Calibration page's button: the test starts (its line says what it does, or why not), or stops.
            MenuCommand::CalibrationRun => {
                crate::machine_calibration::button_pressed();
                MenuAction::None
            }
            // RUSTibites Improved: the Music page's buttons.
            MenuCommand::MusicOpenFolder => {
                if let Err(message) = crate::jukebox::open_folder() {
                    self.status = message;
                }
                MenuAction::None
            }
            MenuCommand::MusicPrevious => {
                crate::jukebox::previous();
                MenuAction::None
            }
            MenuCommand::MusicNext => {
                crate::jukebox::next();
                MenuAction::None
            }
            // RUSTibites Improved: the Extra Hotkeys page's buttons.
            MenuCommand::KeyRebind => {
                crate::improved_keys::start_listening(crate::improved_keys::Action::HideObjects);
                MenuAction::None
            }
            MenuCommand::KeysReset => {
                crate::improved_keys::reset();
                MenuAction::None
            }
            // RUSTibites Improved: the Species Names page's buttons.
            MenuCommand::NamesOpenFile => {
                if let Err(message) = crate::name_list::open_file() {
                    self.status = message;
                }
                MenuAction::None
            }
            MenuCommand::NamesReload => {
                crate::name_list::reload();
                MenuAction::None
            }
            MenuCommand::NamesSamples => {
                crate::name_list::new_samples();
                MenuAction::None
            }
            MenuCommand::OpenUrl(which) => {
                if let Some(url) = URLS.get(usize::from(which)) {
                    open_url(url);
                }
                MenuAction::None
            }
            MenuCommand::SelectTemplate(row) => {
                if usize::from(row) < self.templates.len() {
                    self.library_selected = Some(usize::from(row));
                }
                MenuAction::None
            }
            MenuCommand::StartTemplate if self.pick.is_some() => self.finish_pick(),
            // A Library opened without a goal (`OpenWithoutGoals`, `OpenForBibiteEditor`) has "Edit Bibite" as its submit button, which
            // is `EditSelectedBibite`, like the "Edit Button" beside it; "CreateNew" is `OpenEditorForNewBibite` (no submit button).
            MenuCommand::StartTemplate | MenuCommand::EditTemplate => self.edit_selected_template(),
            MenuCommand::NewTemplate => MenuAction::EditTemplate {
                path: None,
                submit: false,
            },
            MenuCommand::MainButton(_) if self.overlay => MenuAction::None,
            MenuCommand::SettingToggle(index) => {
                self.user_settings.toggle(usize::from(index));
                MenuAction::None
            }
            MenuCommand::SettingReset(index) => {
                self.user_settings.reset(usize::from(index));
                MenuAction::None
            }
            MenuCommand::SettingSlide(index, x0, x1) => {
                // The pointer position is applied by `click_native` (it knows the click).
                let _ = (index, x0, x1);
                MenuAction::None
            }
            MenuCommand::SettingsTab(tab) => {
                self.settings_tab = tab.min(7);
                self.scrolls[scroll_index(MenuView::Settings)] = 0.0;
                MenuAction::None
            }
            MenuCommand::MainButton(index) => {
                self.view = MenuView::Main;
                self.selected = usize::from(index);
                self.activate()
            }
            MenuCommand::NewScenario => {
                self.open_editor(true);
                MenuAction::None
            }
            MenuCommand::EditScenario => {
                self.open_editor(false);
                MenuAction::None
            }
            MenuCommand::EditorStart
            | MenuCommand::EditorBack
            | MenuCommand::EditorSave
            | MenuCommand::EditorAnswer(_)
            | MenuCommand::EditorFocus(_)
            | MenuCommand::EditorAdd(_)
            | MenuCommand::EditorChallenge(_)
            | MenuCommand::EditorDelete(..)
            | MenuCommand::EditorClear(_) => self.editor_command(command),
            MenuCommand::Unavailable(which) => {
                self.status = match which {
                    0 => "Creating scenarios needs the scenario editor (not available yet)",
                    1 => "Editing scenarios needs the scenario editor (not available yet)",
                    2 => "Challenges need a champion picker (not available yet)",
                    3 => "Importing saves is not available yet",
                    4 => "Changing this setting is not available yet",
                    5 => "The sprite editor is not available",
                    6 => "Warnings reset",
                    7 => "The Patreon simulation is not available",
                    _ => "Not available yet",
                }
                .to_owned();
                MenuAction::None
            }
        }
    }
}

/// A throwaway game data folder for the Library tests: two templates (`mine` and the official
/// one) and one saved bibite. The caller removes it.
#[cfg(test)]
pub(crate) fn throwaway_library(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rustibites-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let templates = root.join("Bibites").join("Templates");
    std::fs::create_dir_all(&templates).expect("temp dirs");
    let template = |official: bool| {
        format!(
            r#"{{"name":"x","description":"d","version":"0.6.3.1","isOfficial":{official},"nodes":[],"synapses":[]}}"#
        )
    };
    std::fs::write(templates.join("mine.bb8template"), template(false)).expect("mine");
    std::fs::write(templates.join("official.bb8template"), template(true)).expect("official");
    std::fs::write(
        root.join("Bibites").join("saved.bb8"),
        r#"{"desc":"s","version":"0.6.3.1","brain":{"Nodes":[],"Synapses":[]}}"#,
    )
    .expect("saved");
    root
}

/// The size of the Library's picture box (`BodyPreview/BackGround`: 695.2 x 400 less 12 on each side).
pub const LIBRARY_PICTURE: (f32, f32) = (671.2, 376.0);

/// `ScrollRect.elasticity` (Unity's default; the prefab's value was not read) and how far the wheel can push a list past its end.
const ELASTICITY: f32 = 0.1;
const ELASTIC_LIMIT: f32 = 150.0;

/// How many lists and tabs keep a scroll place of their own (`scroll_index`).
const SCROLL_VIEWS: usize = 7;

/// The place of a view's scroll offset in `MenuState::scrolls`: the lists and tabs that scroll each have their own (the other screens share the last).
fn scroll_index(view: MenuView) -> usize {
    match view {
        MenuView::NewGame => 0,
        MenuView::LoadGame => 1,
        MenuView::Library => 2,
        MenuView::Settings => 3,
        MenuView::Credits => 4,
        MenuView::Tools => 5,
        _ => 6,
    }
}

/// How far down the Credits tab opens: its prefab stores the `Content` 622.96 down (nearly the end of the 631.57 there are: the team, the thanks).
pub const CREDITS_START_SCROLL: f32 = 622.96;

/// The height of the Credits tab's content (`ContentSizeFitter`: the preferred height of its layout group).
pub const CREDITS_CONTENT: f32 = 1586.57;
/// The height of the Additional Tools tab's content (its Steam Workshop section is inactive, so it is not in the layout).
pub const TOOLS_CONTENT: f32 = 1028.32;
/// The height of the view of both tabs (`Viewport`: the tab less its title).
pub const TAB_VIEW: f32 = 955.0;

/// The links of `URLManager` (`Application.OpenURL`), by the index `MenuCommand::OpenUrl` carries: the wiki, the research Discord, the Patreon page, Reddit,
/// Twitter, BlueSky and YouTube.
pub const URLS: [&str; 7] = [
    "https://the-bibites.fandom.com/wiki/",
    "https://discord.gg/zTpMQrVER6",
    "https://www.patreon.com/leocaussan",
    "https://www.reddit.com/r/TheBibites/",
    "https://twitter.com/TBibites",
    "https://bsky.app/profile/leocaussan.bsky.social",
    "https://www.youtube.com/channel/UCjJEUMnBFHOP2zpBc7vCnsA",
];

/// `Application.OpenURL`: the page opens in the default browser (the tests open nothing).
fn open_url(url: &str) {
    #[cfg(not(test))]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(test)]
    let _ = url;
}

const NEWGAME_SIZE_OPTIONS: [&str; 8] = [
    "Smallest", "Small", "Normal", "Bigger", "Large", "Larger", "Enormous", "Gigantic",
];
const NEWGAME_BIOMASS_OPTIONS: [&str; 9] = [
    "Barren", "Sparser", "Sparse", "Low", "Normal", "High", "Dense", "Denser", "Packed",
];
const NEWGAME_FERTILITY_OPTIONS: [&str; 7] = [
    "Sterile",
    "Arid",
    "Low",
    "Normal",
    "High",
    "Lush",
    "Overgrown",
];
const NEWGAME_RADIO_OPTIONS: [&str; 6] = ["None", "Lower", "Low", "Normal", "Severe", "Extreme"];
const NEWGAME_LIMIT_OPTIONS: [&str; 5] =
    ["Fifty", "One Hundred", "Two Hundred", "Five Hundred", "No"];
const NEWGAME_OPTION_COUNTS: [usize; 5] = [8, 9, 7, 6, 5];
// Absolute landmark values from `ScenarioIndependentSettings.cs:30-88`
// (`simulationSize`, `biomassDensity`, `pelletGrowth` landmarks).
const NEWGAME_SIZE_VALUES: [f32; 8] = [
    1000.0, 1500.0, 2000.0, 3500.0, 5000.0, 7500.0, 10000.0, 15000.0,
];
const NEWGAME_BIOMASS_VALUES: [f32; 9] = [
    0.00075, 0.001, 0.0025, 0.005, 0.0075, 0.0125, 0.025, 0.05, 0.1,
];
const NEWGAME_FERTILITY_VALUES: [f32; 7] = [0.0, 1e-6, 3.5e-6, 1e-5, 2e-5, 5e-5, 1e-4];
/// (backgroundMutationChance, backgroundMutationVariance) per Radioactivity row.
const NEWGAME_RADIO_LEVELS: [(f32, f32); 6] = [
    (0.0, 0.0),
    (0.15, 0.01),
    (0.5, 0.015),
    (1.0, 0.025),
    (2.0, 0.075),
    (4.0, 0.15),
];

const ACHIEVEMENTS: [&str; 7] = [
    "Bibite 100 children",
    "Bibite 1 day old",
    "Bibite Editor 100",
    "Bibite Einstein",
    "Simulation 1 FPS",
    "Simulation 7 days",
    "User snapshot",
];

const TOOLS: [&str; 6] = [
    "The Bibites Wikipedia",
    "Steam Workshop (not available)",
    "Bibite Research Conglomerate (Discord)",
    "The Bibites Patreon",
    "The Custom Sprite Editor",
    "Back to main menu (Esc)",
];

const CREDITS: [&str; 6] = [
    "The Bibites: Digital Life",
    "Omnia Studios",
    "Creator & Crazy Scientist: Leo Caussan",
    "Art: Braxiations",
    "Collaborators: Alex Sawyer, PhatSauce, Aleo, Grimmethy",
    "Thanks to all patrons supporting the project",
];

fn on_off(value: bool) -> &'static str {
    if value { "On" } else { "Off" }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<world>")
        .to_owned()
}

/// Scans `dir` for scenario `*.zip` files (sorted by name).
#[must_use]
pub fn scan_scenarios(dir: &Path) -> Vec<PathBuf> {
    let mut scenarios: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
        })
        .collect();
    scenarios.sort();
    scenarios
}

/// The game's data folder (`Application.persistentDataPath`), whether or not it exists.
fn game_data_root() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")?;
    Some(PathBuf::from(home).join("AppData/LocalLow/The Bibites/The Bibites"))
}

/// The Bibite Library list of a data folder (`FillLists`: templates, then saved bibites).
fn list_library_in(root: &Path) -> Vec<bibites_io::storage::LibraryEntry> {
    bibites_io::storage::list_library_entries(&bibites_io::storage::StoragePaths::new(
        root.to_path_buf(),
    ))
}

/// The data folders the Bibite Library lists: the one this port saves to first (`StoragePaths::default_content`: the installed game's folder, or
/// `RUSTIBITES_DATA_DIR` / `./userdata`), then the installed game's own folder when that is another one. The alpha's `Play.bat` keeps what it saves in a
/// `userdata` folder of its own: its Library lists that and, after it, the player's own bibites.
fn library_roots() -> Vec<PathBuf> {
    library_roots_of(
        bibites_io::storage::StoragePaths::default_content().root,
        game_data_root(),
    )
}

/// [`library_roots`] for a content folder and the game's folder.
fn library_roots_of(content: PathBuf, game: Option<PathBuf>) -> Vec<PathBuf> {
    let mut roots = vec![content];
    if let Some(game) = game
        && !roots.contains(&game)
    {
        roots.push(game);
    }
    roots
}

/// The Library entries of `root` (if any), then of each of `extra`.
fn list_library_roots(
    root: Option<&Path>,
    extra: &[PathBuf],
) -> Vec<bibites_io::storage::LibraryEntry> {
    root.into_iter()
        .chain(extra.iter().map(PathBuf::as_path))
        .flat_map(list_library_in)
        .collect()
}

/// The Bibite Library list (`FillLists`: templates, then saved bibites) of the folders the Library lists.
#[must_use]
pub fn list_template_entries() -> Vec<bibites_io::storage::LibraryEntry> {
    let roots = library_roots();
    list_library_roots(
        roots.first().map(PathBuf::as_path),
        roots.get(1..).unwrap_or_default(),
    )
}

/// Loads persisted menu settings (`key=value` lines); `None` if unreadable.
#[must_use]
pub fn load_settings(path: &Path) -> Option<MenuSettings> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut settings = MenuSettings::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let flag = matches!(value.trim(), "1" | "true" | "on");
        match key.trim() {
            "grid" => settings.grid = flag,
            "zones" => settings.zones = flag,
            "pellets" => settings.pellets = flag,
            "pheromones" => settings.pheromones = flag,
            "hud" => settings.hud = flag,
            "sim_tps" => {
                if let Ok(tps) = value.trim().parse::<u32>() {
                    settings.sim_tps = tps.clamp(1, 1000);
                }
            }
            _ => {}
        }
    }
    Some(settings)
}

/// Persists menu settings.
pub fn save_settings(path: &Path, settings: MenuSettings) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let text = format!(
        "grid={}
zones={}
pellets={}
pheromones={}
hud={}
sim_tps={}
",
        u8::from(settings.grid),
        u8::from(settings.zones),
        u8::from(settings.pellets),
        u8::from(settings.pheromones),
        u8::from(settings.hud),
        settings.sim_tps,
    );
    std::fs::write(path, text)
}

/// Scans `dir` for `*.zip` saves, newest first.
#[must_use]
pub fn scan_saves(dir: &Path) -> Vec<PathBuf> {
    scan_saves_with_times(dir).into_iter().map(|(_, path)| path).collect()
}

/// [`scan_saves`] with the time each save was written, which the directory listing already holds.
fn scan_saves_with_times(dir: &Path) -> Vec<(std::time::SystemTime, PathBuf)> {
    let mut saves: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_zip = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"));
            if !is_zip {
                return None;
            }
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            Some((modified, path))
        })
        .collect();
    saves.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    saves
}

/// The check of the Load list's saves that [`MenuState::report_corrupted_saves`] runs on a thread of its own: the signal to stop it and the
/// unreadable saves it has found so far (path, reason), in the list's order.
#[derive(Debug)]
struct SaveCheck {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    found: std::sync::mpsc::Receiver<(PathBuf, String)>,
}

/// The check belongs to the menu that started it: a copy of a menu has none running.
#[derive(Debug, Default)]
struct SaveCheckSlot(Option<SaveCheck>);

impl Clone for SaveCheckSlot {
    fn clone(&self) -> Self {
        Self(None)
    }
}

impl Drop for SaveCheck {
    /// A check that is dropped (replaced by a newer one, or its menu gone because the world it was started for is open) stops its thread at the
    /// next save, instead of reading the rest of 1,500 files beside the game.
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// What a save looked like when it was last read: its size and its time of writing, and the problem found (none for a good one).
type SaveVerdict = (u64, std::time::SystemTime, Option<String>);

/// [`bibites_io::storage::save_problem`] for the Load list, remembering the verdict of a save that has not changed (the same size and time of
/// writing) for as long as the program runs: the list of a player with 1,500 autosaves is looked at again and again, and a good file stays good.
pub(crate) fn checked_save_problem(path: &Path) -> Option<String> {
    static VERDICTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, SaveVerdict>>> = std::sync::OnceLock::new();
    let verdicts = VERDICTS.get_or_init(Default::default);
    let stamp = std::fs::metadata(path).ok().and_then(|metadata| Some((metadata.len(), metadata.modified().ok()?)));
    if let Some((len, modified)) = stamp
        && let Ok(known) = verdicts.lock()
        && let Some((known_len, known_modified, problem)) = known.get(path)
        && (*known_len, *known_modified) == (len, modified)
    {
        return problem.clone();
    }
    let problem = bibites_io::storage::save_problem(path);
    if let Some((len, modified)) = stamp
        && let Ok(mut known) = verdicts.lock()
    {
        known.insert(path.to_path_buf(), (len, modified, problem.clone()));
    }
    problem
}

/// The pointer (logical window pixels, `screen` big) in the menu picture's pixels (`picture` big): the menus are painted at the picture's
/// size and stretched over the whole window, so this is where the pointer is on what is drawn.
#[must_use]
pub fn picture_point(pointer: Vec2, screen: Vec2, picture: Vec2) -> Vec2 {
    Vec2::new(pointer.x * picture.x / screen.x, pointer.y * picture.y / screen.y)
}

/// Converts the menu draw list into the shared UI-overlay form.
#[must_use]
pub fn menu_overlay<'a>(
    ui: &'a UiAssets,
    frame: &'a UiFrame,
    fonts: &'a mut FontSet,
) -> crate::render::UiOverlay<'a> {
    crate::render::UiOverlay {
        assets: ui,
        frame,
        fonts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::MenuCommand;

    fn menu() -> MenuState {
        MenuState::new(None, Path::new(r"Z:\RUSTibites\testdata\does-not-exist"))
    }

    #[test]
    fn a_held_slider_of_the_settings_pages_or_the_tps_panel_follows_the_pointer_until_the_button_is_let_go() {
        // A press on a slider sets it where it lands and holds it (`click_native` does both); from there the pointer moves it, wherever it
        // goes (the Settings pages used to ignore a drag: the value stayed where the button went down).
        let dir = std::env::temp_dir().join(format!("rustibites-menuslider-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu = MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        let index = crate::user_settings::DEFS
            .iter()
            .position(|def| def.key == "PheromonesStrength")
            .expect("a percent slider");
        assert!(!menu.slider_dragging() && !menu.slider_drag_move(Vec2::new(900.0, 0.0)), "nothing is held");
        menu.slider_drag = Some(HeldSlider::Setting(index as u16, 800, 1100, 0.0));
        assert!(menu.slider_dragging());
        assert!(menu.slider_drag_move(Vec2::new(875.0, 300.0)));
        assert!((menu.user_settings().fraction(index) - 0.25).abs() < 1e-4);
        // The pointer leaves the row (any height) and the track's ends hold.
        assert!(menu.slider_drag_move(Vec2::new(1010.0, 900.0)));
        assert!((menu.user_settings().fraction(index) - 0.7).abs() < 1e-4);
        assert!(menu.slider_drag_move(Vec2::new(5000.0, 0.0)));
        assert_eq!(menu.user_settings().fraction(index), 1.0);
        assert!(!menu.slider_drag_move(Vec2::new(6000.0, 0.0)), "the value did not change: nothing to draw again");
        assert!(menu.slider_drag_move(Vec2::new(-5000.0, 0.0)));
        assert_eq!(menu.user_settings().fraction(index), 0.0);
        // Let go: the pointer moves nothing any more.
        menu.slider_release();
        assert!(!menu.slider_dragging() && !menu.slider_drag_move(Vec2::new(950.0, 0.0)));
        assert_eq!(menu.user_settings().fraction(index), 0.0);
        // A TPS slider (a log scale from 5 to 100) follows the same way.
        menu.slider_drag = Some(HeldSlider::Tps(0, 100, 500, 0.0));
        assert!(menu.slider_drag_move(Vec2::new(500.0, 0.0)));
        assert_eq!(menu.tps_values()[0], 100);
        assert!(menu.slider_drag_move(Vec2::new(100.0, 0.0)));
        assert_eq!(menu.tps_values()[0], 5);
        // A press on the handle keeps the offset it found (`slider_grab`), one on the track (past the handle's 15 px) does not.
        assert_eq!(slider_grab(900.0, 910.0, 1.0), 10.0);
        assert_eq!(slider_grab(900.0, 890.0, 1.0), -10.0);
        assert_eq!(slider_grab(900.0, 930.0, 1.0), 0.0);
        assert_eq!(slider_grab(900.0, 930.0, 2.0), 30.0, "the handle is 30 px half-wide on a 2x menu");
        // A slider grabbed by its handle 10 px off its middle keeps that offset: the value is where the HANDLE is, not the pointer.
        menu.slider_drag = Some(HeldSlider::Setting(index as u16, 800, 1100, 10.0));
        assert!(menu.slider_drag_move(Vec2::new(960.0, 0.0)));
        assert!((menu.user_settings().fraction(index) - 150.0 / 300.0).abs() < 1e-4, "{}", menu.user_settings().fraction(index));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dragging_a_scrolling_screen_moves_the_list_and_a_still_press_is_a_click() {
        let dir = std::env::temp_dir().join(format!("rustibites-menudrag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for index in 0..30 {
            std::fs::write(dir.join(format!("save{index:02}.zip")), b"x").expect("save");
        }
        let mut menu = MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.move_selection(2);
        let _ = menu.activate();
        menu.reload_saves();
        assert!(menu.list_bar().is_some());
        let (w, h) = (1920.0, 1080.0);
        // A press that does not move is given back as the click on release.
        assert!(menu.press_hold(Vec2::new(300.0, 300.0), w, h));
        assert!(!menu.content_drag_move(Vec2::new(300.0, 303.0), w, h, 0.016));
        assert_eq!(menu.content_release(w, h), Some(Vec2::new(300.0, 300.0)));
        // A press that moves drags the content (up: further down the list), is no click, and the list keeps moving after the release.
        assert!(menu.press_hold(Vec2::new(300.0, 600.0), w, h));
        for step in 1..=10 {
            menu.content_drag_move(Vec2::new(300.0, 600.0 - 30.0 * step as f32), w, h, 0.016);
        }
        assert!((menu.list_scroll() - 300.0).abs() < 1e-3, "{}", menu.list_scroll());
        assert_eq!(menu.content_release(w, h), None);
        for _ in 0..10 {
            menu.spring(0.016);
        }
        assert!(menu.list_scroll() > 300.0, "it keeps moving");
        // A press on the scrollbar is left to the bar.
        let track = menu.list_bar().expect("bar").track;
        assert!(!menu.press_hold(Vec2::new(track[0] + 5.0, track[1] + 5.0), w, h));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_load_lists_scrollbar_is_there_only_when_the_rows_do_not_fit_and_pages_and_drags() {
        let dir = std::env::temp_dir().join(format!("rustibites-loadbar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        // Fifteen rows fit (110 + 55 * 15 = 935 does not fit in 930, so use fourteen to be sure), thirty do not.
        for index in 0..14 {
            std::fs::write(dir.join(format!("save{index:02}.zip")), b"x").expect("save");
        }
        let mut menu = MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        menu.move_selection(2);
        let _ = menu.activate();
        assert_eq!(menu.view(), MenuView::LoadGame);
        assert!(menu.list_bar().is_none(), "fourteen rows fit");
        for index in 14..30 {
            std::fs::write(dir.join(format!("save{index:02}.zip")), b"x").expect("save");
        }
        menu.reload_saves();
        let bar = menu.list_bar().expect("thirty rows do not");
        // 110 + 55 * 30 = 1760 in a view 930 high.
        assert_eq!(bar.max_scroll(), 830.0);
        // A press on the track moves a page, as far as the end.
        menu.menu_command(MenuCommand::ListBarPage(1));
        assert_eq!(menu.list_scroll(), 830.0);
        menu.menu_command(MenuCommand::ListBarPage(-1));
        assert_eq!(menu.list_scroll(), 0.0);
        // A held handle follows the pointer (the window is 1920 x 1080: reference px).
        let handle = bar.handle();
        menu.bar_grab = Some(25.0);
        assert!(menu.bar_dragging());
        assert!(menu.bar_drag_move(Vec2::new(900.0, handle[1] + 25.0 + 200.0), 1920.0, 1080.0));
        assert_eq!(
            menu.list_scroll(),
            bar.scroll_for_handle_top(handle[1] + 200.0)
        );
        assert!(menu.list_scroll() > 0.0);
        menu.bar_release();
        assert!(!menu.bar_dragging());
        // Let go, the pointer moves nothing.
        assert!(!menu.bar_drag_move(Vec2::new(900.0, 900.0), 1920.0, 1080.0));
        // Only the Load Game screen has this bar.
        menu.menu_command(MenuCommand::Back);
        assert!(menu.list_bar().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_scenario_and_library_lists_have_scrollbars_of_their_own() {
        use bibites_io::storage::{LibraryEntry, TemplateKind};
        let mut menu = menu();
        // The scenario list: 30 rows 70 apart in a view 946 high; 13 fit.
        menu.view = MenuView::NewGame;
        menu.scenario_entries = (0..30)
            .map(|index| ScenarioEntry {
                path: PathBuf::from(format!("{index}.zip")),
                name: index.to_string(),
                description: String::new(),
                version: String::new(),
                is_official: false,
                is_challenge: false,
                rank: 0,
                stars: Default::default(),
                preview_png: None,
            })
            .collect();
        let bar = menu.list_bar().expect("thirty scenarios do not fit");
        assert_eq!(bar.track, [733.2, 108.0, 16.0, 946.0]);
        assert_eq!(bar.max_scroll(), 30.0 * 70.0 - 946.0);
        // The wheel stops where the bar's end is (and not at the number of saves, which the old formula also counted).
        for _ in 0..40 {
            menu.scroll_list(100.0);
        }
        menu.settle_scroll();
        assert_eq!(menu.list_scroll(), 30.0 * 70.0 - 946.0);
        menu.scenario_entries.truncate(13);
        assert!(menu.list_bar().is_none(), "thirteen fit");
        // The Library's open tab: 40 templates 56 apart from 82 under the view's top.
        menu.view = MenuView::Library;
        menu.templates = (0..40)
            .map(|index| LibraryEntry {
                name: index.to_string(),
                description: String::new(),
                version: String::new(),
                is_official: false,
                n_hidden_nodes: 0,
                n_synapses: 0,
                path: PathBuf::from(format!("{index}.bb8template")),
                kind: TemplateKind::Template,
            })
            .collect();
        menu.library_tab = 0;
        let bar = menu.list_bar().expect("forty templates do not fit");
        assert_eq!(bar.track, [1114.8, 91.0, 12.0, 963.0]);
        assert_eq!(bar.max_scroll(), 40.0 * 56.0 - 880.0);
        // The Saved tab holds none of them: no bar. The other screens have none.
        menu.library_tab = 1;
        assert!(menu.list_bar().is_none());
        // The Settings pages: the contents the captures measured in a view 920 high (115, 495 and 464.8 to scroll), not one number for all.
        menu.view = MenuView::Settings;
        for (tab, scrollable) in [(0, 115.0), (1, 495.0), (2, 464.8)] {
            menu.settings_tab = tab;
            let bar = menu
                .list_bar()
                .expect("every settings page is taller than its view");
            assert_eq!(bar.track, [1254.3, 80.0, 20.0, 903.0]);
            assert!(
                (bar.max_scroll() - scrollable).abs() < 0.01,
                "{tab}: {}",
                bar.max_scroll()
            );
            *menu.scroll_slot() = 0.0;
            for _ in 0..20 {
                menu.scroll_list(100.0);
            }
            menu.settle_scroll();
            assert!(
                (menu.list_scroll() - scrollable).abs() < 0.01,
                "{tab}: the wheel stops at the end"
            );
        }
        // Achievements has none of these (its list is not this one).
        menu.view = MenuView::Achievements;
        assert!(menu.list_bar().is_none());
    }

    #[test]
    fn the_credits_and_tools_tabs_and_every_list_keep_their_own_place() {
        let mut menu = menu();
        // The Credits tab opens where its prefab left it: 622.96 down, with the handle near the end of the track (the capture's 467.1 / 582.8).
        menu.open_view_for_capture(MenuView::Credits);
        assert_eq!(menu.list_scroll(), CREDITS_START_SCROLL);
        let bar = menu.list_bar().expect("the credits do not fit their view");
        assert_eq!(bar.track, [1875.0, 100.0, 20.0, 955.0]);
        assert!((bar.max_scroll() - 631.57).abs() < 0.001);
        assert!((bar.handle()[1] - 467.1).abs() < 0.1, "{:?}", bar.handle());
        assert!((bar.handle()[3] - 582.8).abs() < 0.1, "{:?}", bar.handle());
        // A wheel notch is the ScrollRect's 35, and the end is where the content ends.
        assert_eq!(menu.wheel_step(), 35.0);
        menu.scroll_list(-menu.wheel_step());
        assert!((menu.list_scroll() - (CREDITS_START_SCROLL - 35.0)).abs() < 0.001);
        for _ in 0..40 {
            menu.scroll_list(menu.wheel_step());
        }
        menu.settle_scroll();
        assert!((menu.list_scroll() - 631.57).abs() < 0.001);
        // The Additional Tools tab opens at its top, and has 73.32 to scroll (its Steam Workshop section is not there).
        menu.menu_command(MenuCommand::Back);
        menu.open_view_for_capture(MenuView::Tools);
        assert_eq!(menu.list_scroll(), 0.0);
        let bar = menu.list_bar().expect("the tools do not fit their view");
        assert!(
            (bar.max_scroll() - 73.32).abs() < 0.001,
            "{}",
            bar.max_scroll()
        );
        // The handle (888.3 long in the capture) sits at the top of the track.
        assert!((bar.handle()[3] - 888.3).abs() < 0.1, "{:?}", bar.handle());
        menu.menu_command(MenuCommand::ListBarPage(1));
        assert!((menu.list_scroll() - 73.32).abs() < 0.001);
        // Closed and opened again, each tab is where it was left (a `ScrollRect` is only deactivated); one tab's place is not the other's.
        menu.menu_command(MenuCommand::Back);
        menu.open_view_for_capture(MenuView::Credits);
        assert!((menu.list_scroll() - 631.57).abs() < 0.001);
        menu.menu_command(MenuCommand::Back);
        menu.open_view_for_capture(MenuView::Tools);
        assert!((menu.list_scroll() - 73.32).abs() < 0.001);
        // The lists are not the tabs: a list opens at its top and the scenario list keeps its place under the Library.
        menu.menu_command(MenuCommand::Back);
        menu.view = MenuView::NewGame;
        menu.scenario_entries = (0..30)
            .map(|index| ScenarioEntry {
                path: PathBuf::from(format!("{index}.zip")),
                name: index.to_string(),
                description: String::new(),
                version: String::new(),
                is_official: false,
                is_challenge: false,
                rank: 0,
                stars: Default::default(),
                preview_png: None,
            })
            .collect();
        menu.scroll_list(300.0);
        assert_eq!(menu.list_scroll(), 300.0);
        // The list is elastic: pushed past its start it goes below 0, and the spring brings it back.
        menu.scroll_list(-400.0);
        assert!(menu.list_scroll() < 0.0, "{}", menu.list_scroll());
        for _ in 0..120 {
            menu.spring(1.0 / 60.0);
        }
        assert_eq!(menu.list_scroll(), 0.0);
        menu.scroll_list(300.0);
        menu.open_view_for_capture(MenuView::Library);
        assert_eq!(menu.list_scroll(), 0.0, "the Library opens at its top");
        menu.view = MenuView::NewGame;
        assert_eq!(menu.list_scroll(), 300.0, "the scenario list was not moved");
    }

    #[test]
    fn the_links_are_urlmanagers() {
        // `URLManager`'s `OpenWiki`, `OpenResearchDiscord`, `OpenPatreon`, `OpenReddit`, `OpenTwitter`, `OpenBlueSky`, `OpenYoutube`.
        assert_eq!(URLS[0], "https://the-bibites.fandom.com/wiki/");
        assert_eq!(URLS[1], "https://discord.gg/zTpMQrVER6");
        assert_eq!(URLS[2], "https://www.patreon.com/leocaussan");
        assert_eq!(URLS[3], "https://www.reddit.com/r/TheBibites/");
        assert_eq!(URLS[4], "https://twitter.com/TBibites");
        assert_eq!(URLS[5], "https://bsky.app/profile/leocaussan.bsky.social");
        assert_eq!(
            URLS[6],
            "https://www.youtube.com/channel/UCjJEUMnBFHOP2zpBc7vCnsA"
        );
        // Opening one asks the browser (the tests open nothing); a number past the table does nothing.
        let mut menu = menu();
        for which in 0..=URLS.len() as u8 {
            assert_eq!(
                menu.menu_command(MenuCommand::OpenUrl(which)),
                MenuAction::None
            );
        }
        // The Patreon simulation is another scene of the game, which the port does not have.
        menu.menu_command(MenuCommand::Unavailable(7));
        assert_eq!(menu.status(), "The Patreon simulation is not available");
    }

    #[test]
    fn menu_starts_on_new_game_and_wraps_navigation() {
        let mut menu = menu();
        assert_eq!(menu.view(), MenuView::Main);
        assert_eq!(menu.selected(), 0);
        menu.move_selection(-1);
        assert_eq!(menu.selected(), 8);
        menu.move_selection(1);
        assert_eq!(menu.selected(), 0);
        assert_eq!(
            menu.rows(),
            [
                "Continue",
                "New",
                "Load",
                "Bibite Library",
                "Settings",
                "Achievements",
                "Additional Tools",
                "Credits",
                "Quit"
            ]
        );
        assert_eq!(menu.rows().len(), 9);
    }

    #[test]
    fn missing_default_scenario_keeps_menu_open() {
        let mut menu = menu();
        assert!(matches!(menu.activate(), MenuAction::None));
        assert_eq!(menu.view(), MenuView::Main);
        assert!(!menu.status().is_empty());
    }

    #[test]
    fn load_view_back_returns_to_main() {
        let mut menu = menu();
        menu.move_selection(2);
        assert!(matches!(menu.activate(), MenuAction::None));
        assert_eq!(menu.view(), MenuView::LoadGame);
        menu.back();
        assert_eq!(menu.view(), MenuView::Main);
    }

    #[test]
    fn scan_saves_only_zips_newest_first() {
        let dir = std::env::temp_dir().join(format!("rustibites-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("a.zip"), b"a").expect("a");
        std::fs::write(dir.join("b.txt"), b"b").expect("b");
        std::fs::write(dir.join("c.zip"), b"c").expect("c");
        let saves = scan_saves(&dir);
        assert_eq!(saves.len(), 2);
        assert!(
            saves
                .iter()
                .all(|path| path.extension().is_some_and(|e| e == "zip"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_saves_continue_and_load_are_graceful() {
        let mut menu = menu();
        assert!(matches!(menu.activate(), MenuAction::None)); // Continue
        assert_eq!(menu.view(), MenuView::Main);
        assert_eq!(menu.status(), "No saves found");
        menu.move_selection(2); // Load Game
        assert!(matches!(menu.activate(), MenuAction::None));
        assert_eq!(menu.view(), MenuView::LoadGame);
        assert!(menu.rows()[0].contains("no saves"));
        assert_eq!(menu.rows().last().map(String::as_str), Some("Back"));
        assert!(matches!(menu.activate(), MenuAction::None));
        assert_eq!(menu.view(), MenuView::Main);
    }

    #[test]
    fn default_scenario_activation_starts_loading() {
        let dir =
            std::env::temp_dir().join(format!("rustibites-menu-start-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("testdata")
            .join("real")
            .join("Default.zip");
        if !source.is_file() {
            eprintln!("skipping: testdata/real/Default.zip absent");
            return;
        }
        let scenario = dir.join("Default.zip");
        std::fs::copy(&source, &scenario).expect("scenario copy");
        let mut menu = MenuState::new(Some(scenario.clone()), &dir);
        menu.move_selection(1); // New
        assert!(matches!(menu.activate(), MenuAction::None));
        assert_eq!(menu.view(), MenuView::NewGame);
        assert!(menu.scenarios().iter().any(|path| path == &scenario));
        // New Game now selects a scenario first, then starts from the generated row.
        assert!(matches!(menu.activate(), MenuAction::None));
        let start_row = menu.scenarios().len().min(8) + 5;
        menu.move_selection(start_row as isize);
        assert!(matches!(menu.activate(), MenuAction::Start(_)));
        assert_eq!(menu.view(), MenuView::Loading);
        assert!(menu.pending().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A menu on the Load Game screen over three throwaway saves, with a settings file of its own
    /// (the test never writes the player's).
    fn load_menu(name: &str) -> (MenuState, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rustibites-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        for file in ["a.zip", "b.zip", "c.zip"] {
            std::fs::write(dir.join(file), b"x").expect("save");
        }
        let mut menu = MenuState::new(None, &dir);
        menu.user_settings =
            crate::user_settings::UserSettings::load(Some(dir.join("settings.json")));
        // Shown, whatever the installed game remembers.
        menu.user_settings.reset_warnings();
        menu.view = MenuView::LoadGame;
        menu.reload_saves();
        assert_eq!(menu.saves.len(), 3);
        (menu, dir)
    }

    #[test]
    fn opening_the_load_list_raises_a_corrupted_save_popup_for_each_unreadable_save_and_ok_dismisses_them() {
        use crate::ui::MenuCommand;
        let (mut menu, dir) = load_menu("corrupt");
        assert!(menu.error_popup().is_none());
        menu.report_corrupted_saves();
        // The saves are read beside the window: what they hold comes up as the frames take it in.
        menu.finish_save_check();
        let (source, text) = menu.error_popup().expect("a popup");
        assert_eq!(source, "Corrupted Save");
        assert!(text.starts_with("A corrupted save was found and couldn't ne displayed:
"), "{text}");
        assert!(text.contains(".zip
Delete the file to stop this problem"), "{text}");
        assert_eq!(menu.errors.len(), 3);
        menu.menu_command(MenuCommand::DismissError);
        menu.menu_command(MenuCommand::DismissError);
        assert!(menu.error_popup().is_some());
        menu.menu_command(MenuCommand::DismissError);
        assert!(menu.error_popup().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_saves_are_checked_beside_the_window_so_the_click_on_load_returns_at_once() {
        let (mut menu, dir) = load_menu("beside");
        // The call only starts the check: whatever it finds is taken in by the window loop's `poll_background`, never by the call itself.
        menu.report_corrupted_saves();
        assert!(menu.error_popup().is_none(), "the call itself waited for the saves");
        // Opening the list again while the first check runs replaces it: the popups are not raised twice.
        menu.report_corrupted_saves();
        menu.finish_save_check();
        assert!(!menu.save_check_running());
        assert_eq!(menu.errors.len(), 3, "one popup for each unreadable save, once");
        assert!(!menu.poll_background(), "nothing is left to take in");
        // A list with nothing in it starts no check.
        menu.saves.clear();
        menu.errors.clear();
        menu.report_corrupted_saves();
        assert!(!menu.save_check_running());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_check_of_the_saves_stops_when_a_world_starts_loading_and_when_the_menu_goes_away() {
        use std::sync::atomic::Ordering;
        let (mut menu, dir) = load_menu("stops");
        // The player picked a save: the Loading screen starts, and the rest of the check would only slow the world's own read down.
        menu.report_corrupted_saves();
        let cancel = std::sync::Arc::clone(&menu.save_check.0.as_ref().expect("a check is running").cancel);
        assert!(!cancel.load(Ordering::Relaxed));
        menu.view = MenuView::Loading;
        assert!(!menu.poll_background(), "no popups come up over the Loading screen");
        assert!(!menu.save_check_running(), "the check goes on while a world is read");
        assert!(cancel.load(Ordering::Relaxed), "its thread was not told to stop");
        // The menu goes away (the world is open): its check is told to stop too.
        menu.view = MenuView::LoadGame;
        menu.report_corrupted_saves();
        let cancel = std::sync::Arc::clone(&menu.save_check.0.as_ref().expect("a check is running").cancel);
        drop(menu);
        assert!(cancel.load(Ordering::Relaxed), "a menu that is gone left its check running");
        // A copy of a menu has no check of its own (and so cannot stop the original's).
        let (mut menu, other) = load_menu("stops-copy");
        menu.report_corrupted_saves();
        let copy = menu.clone();
        assert!(!copy.save_check_running());
        drop(copy);
        assert!(menu.save_check_running());
        menu.finish_save_check();
        assert_eq!(menu.errors.len(), 3, "the copy's end did not stop the original's check");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(other);
    }

    #[test]
    fn the_challenge_list_picks_a_champion_in_the_library_and_starts_the_challenge() {
        use crate::challenge_run::{ChallengeStart, Champion};
        use crate::ui::MenuCommand;
        let (mut menu, scenario, champion) = MenuState::for_challenge_tests("pick");
        // The scenario list has no champion panel and no way to pick one.
        assert!(!menu.showing_challenges());
        assert_eq!(
            menu.menu_command(MenuCommand::SelectChampion),
            MenuAction::None
        );
        assert!(menu.pick().is_none());
        // The challenge list (`SwitchList(false)`): nothing picked, "Start Challenge" is not interactable, the modifiers' start is gone.
        menu.menu_command(MenuCommand::ToggleChallenges);
        assert!(menu.showing_challenges() && menu.champion().is_none());
        assert_eq!(
            menu.selected_scenario().map(|entry| entry.name.as_str()),
            Some("Little Rabbits")
        );
        assert_eq!(
            menu.menu_command(MenuCommand::StartChallenge(true)),
            MenuAction::None
        );
        assert_eq!(
            menu.menu_command(MenuCommand::StartSimulation),
            MenuAction::None
        );
        assert!(menu.pending_challenge().is_none() && menu.view() == MenuView::NewGame);
        // "Select Champion" opens the Library to pick (`OpenWithCallBacks`); the submit button brings the New Game screen back.
        menu.menu_command(MenuCommand::SelectChampion);
        assert_eq!(menu.view(), MenuView::Library);
        assert_eq!(menu.pick(), Some(TemplatePick::Champion));
        menu.menu_command(MenuCommand::SelectTemplate(0));
        assert_eq!(
            menu.menu_command(MenuCommand::StartTemplate),
            MenuAction::None
        );
        assert_eq!(menu.view(), MenuView::NewGame);
        assert_eq!(
            menu.champion(),
            Some(&Champion {
                path: champion.clone(),
                name: "Basic bibite".to_owned()
            })
        );
        // Leaving the Library without a pick keeps the champion (`Select Other` then Escape).
        menu.menu_command(MenuCommand::SelectChampion);
        menu.menu_command(MenuCommand::Back);
        assert_eq!(menu.view(), MenuView::NewGame);
        assert!(menu.champion().is_some());
        // "Start Challenge": the scenario with the champion, as a challenge.
        assert_eq!(
            menu.menu_command(MenuCommand::StartChallenge(true)),
            MenuAction::Start(scenario.clone())
        );
        assert_eq!(menu.view(), MenuView::Loading);
        assert_eq!(
            menu.pending_challenge(),
            Some(&ChallengeStart {
                scenario: scenario.clone(),
                champion: Some(Champion {
                    path: champion.clone(),
                    name: "Basic bibite".to_owned()
                }),
                as_challenge: true,
            })
        );
        assert!(!menu.pending_is_save());
        // The small button launches the challenge as a scenario (`StartChallenge(false)`), with or without a champion.
        menu.fail("again");
        menu.view = MenuView::NewGame;
        assert_eq!(
            menu.menu_command(MenuCommand::StartChallenge(false)),
            MenuAction::Start(scenario.clone())
        );
        assert!(
            menu.pending_challenge()
                .is_some_and(|start| !start.as_challenge)
        );
        // A scenario that is not started from the challenge list is not a challenge start.
        menu.fail("again");
        menu.view = MenuView::NewGame;
        menu.menu_command(MenuCommand::ToggleChallenges);
        assert!(!menu.showing_challenges());
        assert!(menu.pending_challenge().is_none());
        // Going back to the challenge list forgets the champion (`SwitchList(false)`).
        menu.menu_command(MenuCommand::ToggleChallenges);
        assert!(menu.champion().is_none());
        let _ = std::fs::remove_dir_all(scenario.parent().expect("dir"));
    }

    #[test]
    fn only_a_save_is_a_loaded_game_for_pause_after_load() {
        let (mut menu, dir) = load_menu("menu-loaded-game");
        assert!(!menu.pending_is_save());
        // Load Game > a save row: `GameManager.saveToLoad`, so `SimulationManager.loadedGame`.
        menu.select(0);
        assert!(matches!(menu.activate(), MenuAction::Start(_)));
        assert_eq!(menu.view(), MenuView::Loading);
        assert!(menu.pending_is_save());
        // A load that failed goes back to the main menu with nothing pending.
        menu.fail("broken");
        assert!(!menu.pending_is_save() && menu.pending().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deleting_a_save_asks_first_and_cancel_remembers_nothing() {
        use crate::warnings::ActionWarning;
        let (mut menu, dir) = load_menu("menu-delete-cancel");
        menu.menu_command(MenuCommand::DeleteSave);
        assert!(menu.confirm_delete());
        assert!(!menu.confirm_checked(), "every popup starts unticked");
        menu.menu_command(MenuCommand::ToggleDoNotShowAgain);
        assert!(menu.confirm_checked());
        menu.menu_command(MenuCommand::ConfirmDelete(false));
        assert!(!menu.confirm_delete() && !menu.confirm_checked());
        assert_eq!(menu.saves.len(), 3, "cancel deleted a save");
        assert!(!menu.warning_off(ActionWarning::DeleteSaveFile));
        // The next popup is unticked again.
        menu.menu_command(MenuCommand::DeleteSave);
        assert!(menu.confirm_delete() && !menu.confirm_checked());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_ticked_confirm_switches_the_delete_warning_off_until_reset_warnings() {
        use crate::warnings::ActionWarning;
        let (mut menu, dir) = load_menu("menu-delete-ticked");
        menu.menu_command(MenuCommand::DeleteSave);
        menu.menu_command(MenuCommand::ToggleDoNotShowAgain);
        menu.menu_command(MenuCommand::ConfirmDelete(true));
        assert_eq!(menu.saves.len(), 2, "the save went");
        assert!(menu.warning_off(ActionWarning::DeleteSaveFile));
        assert!(
            crate::user_settings::UserSettings::load(Some(dir.join("settings.json")))
                .warning_off(ActionWarning::DeleteSaveFile),
            "kept in the settings file"
        );

        // The next delete goes at once, with no popup.
        menu.menu_command(MenuCommand::DeleteSave);
        assert!(!menu.confirm_delete());
        assert_eq!(menu.saves.len(), 1);

        // "Reset Warnings" shows it again.
        menu.menu_command(MenuCommand::ResetWarnings);
        assert!(!menu.warning_off(ActionWarning::DeleteSaveFile));
        menu.menu_command(MenuCommand::DeleteSave);
        assert!(menu.confirm_delete());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_checkbox_does_nothing_without_a_warning_popup() {
        let (mut menu, dir) = load_menu("menu-delete-idle");
        menu.menu_command(MenuCommand::ToggleDoNotShowAgain);
        assert!(!menu.confirm_checked());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A menu over a throwaway game folder: two templates (`mine` and the official one) and one
    /// saved bibite, with a settings file of its own.
    fn library_menu(name: &str) -> (MenuState, PathBuf) {
        let root = throwaway_library(name);
        let menu = MenuState::new(None, &root)
            .with_private_settings(&root.join("s.json"))
            .with_library_root(&root);
        assert_eq!(menu.templates.len(), 3, "{:?}", menu.templates);
        (menu, root)
    }

    #[test]
    fn the_library_tabs_list_their_own_files_and_keep_the_selection() {
        use bibites_io::storage::TemplateKind;
        let (mut menu, root) = library_menu("library-tabs");
        // Templates first (by file name), then the saved bibites; the first one is selected.
        let kinds: Vec<_> = menu.templates.iter().map(|entry| entry.kind).collect();
        assert_eq!(
            kinds,
            [
                TemplateKind::Template,
                TemplateKind::Template,
                TemplateKind::Genome
            ]
        );
        assert_eq!(menu.library_selected(), Some(0));
        assert_eq!(menu.library_tab(), 0);
        assert_eq!(menu.library_rows(), vec![0, 1]);
        menu.menu_command(MenuCommand::LibraryTab(1));
        assert_eq!(menu.library_rows(), vec![2]);
        // Switching tabs only changes the list: the selected bibite stays (`OpenTab`).
        assert_eq!(menu.library_selected(), Some(0));
        menu.menu_command(MenuCommand::SelectTemplate(2));
        assert_eq!(menu.library_selected(), Some(2));
        menu.menu_command(MenuCommand::LibraryTab(0));
        assert_eq!(menu.library_rows(), vec![0, 1]);
        assert_eq!(menu.library_selected(), Some(2));
        // Up / Down walk the open tab.
        menu.view = MenuView::Library;
        menu.menu_command(MenuCommand::SelectTemplate(0));
        menu.native_move(1);
        assert_eq!(menu.library_selected(), Some(1));
        menu.native_move(1);
        assert_eq!(
            menu.library_selected(),
            Some(1),
            "the saved bibite is not on this tab"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_librarys_folders_are_the_one_the_port_saves_to_and_the_games_when_it_is_another() {
        let (game, mine) = (PathBuf::from("game"), PathBuf::from("mine"));
        // The owner's setup: the port saves to the game's own folder.
        assert_eq!(library_roots_of(game.clone(), Some(game.clone())), [game.clone()]);
        // The alpha's `Play.bat`: a folder of its own, then the player's game.
        assert_eq!(library_roots_of(mine.clone(), Some(game.clone())), [mine.clone(), game]);
        // No game installed.
        assert_eq!(library_roots_of(mine.clone(), None), [mine]);
    }

    #[test]
    fn the_library_lists_what_the_port_saved_in_its_own_folder_before_the_games_bibites() {
        use bibites_io::storage::TemplateKind;
        let (mine, game) = (throwaway_library("library-mine"), throwaway_library("library-game"));
        let template = |name: &str| format!(r#"{{"name":"{name}","description":"","version":"0.6.3.1","nodes":[],"synapses":[]}}"#);
        std::fs::write(mine.join("Bibites").join("Templates").join("Saved here.bb8template"), template("Saved here")).expect("the port's template");
        std::fs::write(game.join("Bibites").join("Templates").join("Players own.bb8template"), template("Players own")).expect("the game's template");
        let mut menu = MenuState::new(None, &mine)
            .with_private_settings(&mine.join("s.json"))
            .with_library_roots(&mine, &[game.clone()]);
        // Each folder lists its templates, then its saved bibites (3 of the helper's and the one added here); the port's folder is first.
        let place = |index: usize| usize::from(menu.templates[index].path.starts_with(&game));
        assert_eq!(menu.templates.len(), 8, "{:?}", menu.templates);
        assert_eq!((0..8).map(place).collect::<Vec<_>>(), [0, 0, 0, 0, 1, 1, 1, 1]);
        let named = |name: &str| menu.templates.iter().find(|entry| entry.name == name).map(|entry| (entry.kind, entry.path.clone()));
        assert_eq!(named("Saved here").map(|found| found.0), Some(TemplateKind::Template));
        assert!(named("Saved here").is_some_and(|found| found.1.starts_with(&mine)), "listed from the port's folder");
        assert!(named("Players own").is_some_and(|found| found.1.starts_with(&game)), "and the player's own from the game's");
        assert_eq!(menu.library_selected(), Some(0));
        // "Reload" reads both folders again.
        std::fs::write(mine.join("Bibites").join("Templates").join("Later.bb8template"), template("Later")).expect("a later template");
        menu.menu_command(MenuCommand::LibraryReload);
        assert_eq!(menu.templates.len(), 9);
        assert!(menu.templates.iter().any(|entry| entry.name == "Later"));
        let _ = std::fs::remove_dir_all(mine);
        let _ = std::fs::remove_dir_all(game);
    }

    #[test]
    fn the_library_delete_button_asks_first_and_spares_the_official_bibites() {
        use crate::warnings::ActionWarning;
        let (mut menu, root) = library_menu("library-delete");
        let templates = root.join("Bibites").join("Templates");
        // Index 0 is `mine`, index 1 the official one.
        assert!(menu.library_can_delete());
        menu.menu_command(MenuCommand::SelectTemplate(1));
        assert!(
            !menu.library_can_delete(),
            "the official bibites cannot be deleted"
        );
        menu.menu_command(MenuCommand::LibraryDelete);
        assert!(!menu.confirm_library_delete());
        assert!(templates.join("official.bb8template").is_file());

        // Delete asks; cancelling with the box ticked remembers nothing.
        menu.menu_command(MenuCommand::SelectTemplate(0));
        menu.menu_command(MenuCommand::LibraryDelete);
        assert!(menu.confirm_library_delete() && !menu.confirm_checked());
        menu.menu_command(MenuCommand::ToggleDoNotShowAgain);
        assert!(menu.confirm_checked());
        menu.menu_command(MenuCommand::ConfirmLibraryDelete(false));
        assert!(!menu.confirm_library_delete() && !menu.confirm_checked());
        assert!(templates.join("mine.bb8template").is_file());
        assert!(!menu.warning_off(ActionWarning::DeleteBibiteFile));

        // Confirming with the box ticked deletes the file, selects the first bibite of the same
        // list and switches the warning off.
        menu.menu_command(MenuCommand::LibraryDelete);
        menu.menu_command(MenuCommand::ToggleDoNotShowAgain);
        menu.menu_command(MenuCommand::ConfirmLibraryDelete(true));
        assert!(!templates.join("mine.bb8template").exists());
        assert_eq!(menu.templates.len(), 2);
        assert_eq!(menu.library_selected(), Some(0));
        assert!(menu.templates[0].is_official);
        assert!(menu.warning_off(ActionWarning::DeleteBibiteFile));

        // The saved bibite now goes at once; no other bibite of its list is left to select.
        menu.menu_command(MenuCommand::SelectTemplate(1));
        assert!(menu.library_can_delete());
        menu.menu_command(MenuCommand::LibraryDelete);
        assert!(!menu.confirm_library_delete());
        assert!(!root.join("Bibites").join("saved.bb8").exists());
        assert_eq!(menu.library_selected(), None);
        assert!(!menu.library_can_delete());

        // "Reset Warnings" does not bring this one back (it is not in `allInfoShown`).
        menu.menu_command(MenuCommand::ResetWarnings);
        assert!(menu.warning_off(ActionWarning::DeleteBibiteFile));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_librarys_edit_buttons_open_the_bibite_editor_unless_it_picks_a_template() {
        let (mut menu, root) = library_menu("library-edit");
        menu.view = MenuView::Library;
        menu.menu_command(MenuCommand::SelectTemplate(1));
        let selected = menu.templates[1].path.clone();
        // A Library opened without a goal (`OpenWithoutGoals`) says "Edit Bibite" on its submit button: `EditSelectedBibite`, the
        // editor on the selected bibite with "Save and submit" (`AfterEditorSubmit`). The Edit button beside it is the same.
        let edit = MenuAction::EditTemplate {
            path: Some(selected),
            submit: true,
        };
        assert_eq!(menu.menu_command(MenuCommand::StartTemplate), edit);
        assert_eq!(menu.menu_command(MenuCommand::EditTemplate), edit);
        // Create New (`OpenEditorForNewBibite`) opens a new template with no submit action.
        assert_eq!(
            menu.menu_command(MenuCommand::NewTemplate),
            MenuAction::EditTemplate {
                path: None,
                submit: false
            }
        );
        // The same over a running world.
        menu.set_overlay(true);
        assert_eq!(menu.menu_command(MenuCommand::StartTemplate), edit);
        // Nothing selected: `EditSelectedBibite` still opens the editor, on an empty template.
        menu.library_selected = None;
        assert_eq!(
            menu.menu_command(MenuCommand::StartTemplate),
            MenuAction::EditTemplate {
                path: None,
                submit: true
            }
        );
        // A Library that picks a template hands it back instead (the Edit and Create New buttons still open the editor).
        menu.begin_pick(TemplatePick::PlaceBibite);
        menu.menu_command(MenuCommand::SelectTemplate(0));
        assert!(matches!(
            menu.menu_command(MenuCommand::StartTemplate),
            MenuAction::PickedTemplate(_)
        ));
        menu.begin_pick(TemplatePick::PlaceBibite);
        menu.menu_command(MenuCommand::SelectTemplate(0));
        assert!(matches!(
            menu.menu_command(MenuCommand::EditTemplate),
            MenuAction::EditTemplate { .. }
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_library_fills_its_lists_again_after_the_editor_closes() {
        let (mut menu, root) = library_menu("library-refill");
        menu.view = MenuView::Library;
        menu.menu_command(MenuCommand::SelectTemplate(1));
        let templates = root.join("Bibites").join("Templates");
        std::fs::write(
            templates.join("fresh.bb8template"),
            r#"{"name":"fresh","version":"0.6.3.1","nodes":[],"synapses":[]}"#,
        )
        .expect("a template saved in the editor");
        // `AfterEditorClose`: the lists are read again and the bibite that was selected is still the one (it moved down the list).
        menu.refill_library(None);
        assert_eq!(menu.templates.len(), 4);
        let selected = menu.library_selected().expect("selected");
        assert_eq!(
            menu.templates[selected].path,
            templates.join("official.bb8template")
        );
        // `AfterEditorSubmit`: the template that was saved is selected.
        menu.refill_library(Some("fresh"));
        let selected = menu.library_selected().expect("selected");
        assert_eq!(menu.templates[selected].name, "fresh");
        // A name that the Templates list does not have selects nothing (`FirstOrDefault` is null).
        menu.refill_library(Some("nobody"));
        assert_eq!(menu.library_selected(), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_template_saved_again_gets_a_new_picture_in_the_library() {
        // The picture is made from the genes of the selected file, read once per file: after the editor saved the template under the same name, and when the lists are
        // filled again, they are read again (a file that did not read was remembered as none for good, too).
        let (mut menu, root) = library_menu("library-picture-cache");
        menu.view = MenuView::Library;
        menu.menu_command(MenuCommand::SelectTemplate(1));
        let previews = crate::preview::PreviewCache::default();
        menu.refresh_library_picture(None, &previews, 1.0);
        assert_eq!(menu.library_genomes.len(), 1, "the selected file's genes are kept");
        menu.refill_library(None);
        assert!(menu.library_genomes.is_empty(), "the editor may have saved it again");
        menu.refresh_library_picture(None, &previews, 1.0);
        assert_eq!(menu.library_genomes.len(), 1);
        menu.menu_command(MenuCommand::LibraryReload);
        assert!(menu.library_genomes.is_empty(), "the Reload button fills the lists again");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn escape_closes_a_warning_popup_without_deleting_or_remembering() {
        use crate::warnings::ActionWarning;
        let (mut menu, root) = library_menu("library-escape");
        menu.view = MenuView::Library;
        menu.menu_command(MenuCommand::LibraryDelete);
        assert!(menu.confirm_library_delete());
        menu.menu_command(MenuCommand::ToggleDoNotShowAgain);
        menu.back();
        // The popup went (Cancel), the Library is still open and nothing was deleted or kept.
        assert!(!menu.confirm_library_delete() && !menu.confirm_checked());
        assert_eq!(menu.view(), MenuView::Library);
        assert!(
            root.join("Bibites")
                .join("Templates")
                .join("mine.bb8template")
                .is_file()
        );
        assert!(!menu.warning_off(ActionWarning::DeleteBibiteFile));
        // The next Esc leaves the Library.
        menu.back();
        assert_eq!(menu.view(), MenuView::Main);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_that_cannot_be_deleted_stays_in_the_list_with_a_message() {
        let (mut menu, root) = library_menu("library-delete-fails");
        menu.user_settings
            .set_warning_off(crate::warnings::ActionWarning::DeleteBibiteFile, true);
        // The file vanished behind the Library's back: the delete fails.
        std::fs::remove_file(
            root.join("Bibites")
                .join("Templates")
                .join("mine.bb8template"),
        )
        .expect("remove");
        menu.menu_command(MenuCommand::LibraryDelete);
        assert!(
            menu.status().starts_with("Couldn't delete template"),
            "{}",
            menu.status()
        );
        assert_eq!(menu.templates.len(), 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reload_reads_the_folders_again_and_selects_the_first_bibite() {
        let (mut menu, root) = library_menu("library-reload");
        menu.menu_command(MenuCommand::SelectTemplate(2));
        std::fs::write(
            root.join("Bibites")
                .join("Templates")
                .join("aaa.bb8template"),
            r#"{"name":"a","version":"0.6.3.1","nodes":[],"synapses":[]}"#,
        )
        .expect("new template");
        menu.menu_command(MenuCommand::LibraryReload);
        assert_eq!(menu.templates.len(), 4);
        assert_eq!(menu.library_selected(), Some(0));
        assert_eq!(menu.templates[0].name, "a");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A menu with settings of its own, over an empty folder.
    fn plain_menu(name: &str) -> (MenuState, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rustibites-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let menu = MenuState::new(None, &dir).with_private_settings(&dir.join("s.json"));
        (menu, dir)
    }

    #[test]
    fn a_new_game_modifier_opens_a_list_and_a_pick_sets_the_option() {
        use crate::ui::DropdownTarget as Target;
        let (mut menu, dir) = plain_menu("dropdown-modifier");
        menu.view = MenuView::NewGame;
        let anchor = [1118.0, 795.0, 200.0, 40.0];
        menu.open_dropdown_list(Target::Modifier(1), anchor);
        let list = menu.dropdown_list().expect("the list opens");
        // The biomass dropdown's options, the current one checked, hung under the control.
        assert_eq!(list.choices.len(), NEWGAME_BIOMASS_OPTIONS.len());
        assert_eq!(list.selected, menu.newgame_options[1]);
        assert_eq!(list.anchor, anchor);
        menu.pick_dropdown_list(2);
        assert!(menu.dropdown_list().is_none(), "a pick closes the list");
        assert_eq!(menu.newgame_options[1], 2);
        assert!(menu.newgame_dirty[1]);
        // A second click on the same dropdown closes its list.
        menu.open_dropdown_list(Target::Modifier(1), anchor);
        menu.open_dropdown_list(Target::Modifier(1), anchor);
        assert!(menu.dropdown_list().is_none());
        // Esc closes an open list and nothing else: the screen stays.
        menu.open_dropdown_list(Target::Modifier(3), anchor);
        menu.back();
        assert!(menu.dropdown_list().is_none());
        assert_eq!(menu.view(), MenuView::NewGame);
        menu.back();
        assert_eq!(menu.view(), MenuView::Main);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_game_modifier_item_shows_the_setting_name_and_the_landmark_value() {
        use bibites_core::format::{ScientificOptions, scientific_format};
        let item = |index: usize, at: usize| MenuState::modifier_choices(index).swap_remove(at);
        // `NumericSettingDropdown.OnItemHovered`: `setting.Name + ": " + label` over the value
        // `landmark.value.ScientificFormat(precision, units)`.
        let smallest = item(0, 0);
        assert_eq!(smallest.title, "Size of the Simulation Area: Smallest");
        assert_eq!(smallest.tip, "1.00 ku");
        assert_eq!(item(0, 7).tip, "15.0 ku");
        let normal = item(1, 4);
        assert_eq!(normal.title, "Global Biomass Density: Normal");
        assert_eq!(
            normal.tip,
            scientific_format(
                0.0075,
                1,
                ScientificOptions {
                    units: " E/u²",
                    ..ScientificOptions::default()
                }
            )
        );
        assert_eq!(normal.tip, "7.5 mE/u²");
        // "Sterile" has a text of its own.
        let sterile = item(2, 0);
        assert_eq!(sterile.title, "Global Fertility: Sterile");
        assert_eq!(sterile.tip, "No Growth at all (or very close to none)");
        assert_eq!(item(2, 3).title, "Global Fertility: Normal");
        assert!(item(2, 3).tip.ends_with("E/u²s"), "{}", item(2, 3).tip);
        // Every option of the three has a tooltip; radioactivity and the limit have none.
        for index in 0..3 {
            let choices = MenuState::modifier_choices(index);
            assert_eq!(choices.len(), MenuState::modifier_options(index).len());
            assert!(
                choices
                    .iter()
                    .all(|c| !c.title.is_empty() && !c.tip.is_empty())
            );
        }
        for index in 3..5 {
            let choices = MenuState::modifier_choices(index);
            assert_eq!(choices.len(), MenuState::modifier_options(index).len());
            assert!(
                choices
                    .iter()
                    .all(|c| c.title.is_empty() && c.tip.is_empty())
            );
        }
    }

    #[test]
    fn the_settings_dropdowns_show_item_tooltips_only_for_the_float_ones() {
        use crate::ui::DropdownTarget as Target;
        let (mut menu, dir) = plain_menu("dropdown-setting-tips");
        menu.view = MenuView::Settings;
        let anchor = [900.0, 300.0, 200.0, 40.0];
        // Screen Resolution and UI Scale are `FloatSettingDropdown`s: the value under the name.
        let index = crate::user_settings::index_of("Screen Resolution").expect("a dropdown row");
        menu.open_dropdown_list(Target::Setting(index as u16), anchor);
        let list = menu.dropdown_list().expect("the list opens");
        assert_eq!(list.choices[3].title, "Screen Resolution: 1080p");
        assert_eq!(list.choices[3].tip, "1.00 X");
        assert_eq!(list.choices[0].title, "Screen Resolution: 720p");
        assert_eq!(list.choices[0].tip, "667 mX");
        let index = crate::user_settings::index_of("UI Scale").expect("a dropdown row");
        menu.open_dropdown_list(Target::Setting(index as u16), anchor);
        let list = menu.dropdown_list().expect("the list opens");
        assert_eq!(list.choices[2].title, "UI Scale: Normal (1.0)");
        assert!(!list.choices[2].tip.is_empty());
        // Procedural Size and Autosave Period are `ChoiceSettingDropdown`s without `SettingChoices`.
        for name in ["Procedural Size", "Autosave Period"] {
            let index = crate::user_settings::index_of(name).expect("a dropdown row");
            menu.open_dropdown_list(Target::Setting(index as u16), anchor);
            let list = menu.dropdown_list().expect("the list opens");
            assert!(
                list.choices
                    .iter()
                    .all(|c| c.title.is_empty() && c.tip.is_empty()),
                "{name}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_settings_dropdown_opens_its_options_and_the_wheel_scrolls_the_list() {
        use crate::ui::DropdownTarget as Target;
        let (mut menu, dir) = plain_menu("dropdown-setting");
        menu.view = MenuView::Settings;
        let index = crate::user_settings::index_of("Autosave Period").expect("a dropdown row");
        menu.open_dropdown_list(Target::Setting(index as u16), [900.0, 300.0, 200.0, 40.0]);
        let labels = crate::user_settings::UserSettings::choice_labels(index).expect("options");
        let list = menu.dropdown_list().expect("the list opens");
        assert_eq!(list.choices.len(), labels.len());
        assert_eq!(
            list.choices
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            labels
        );
        assert!(list.max_scroll() > 0.0, "seven options do not fit in 150");
        // The wheel scrolls the list, not the page behind it.
        menu.scroll_list(70.0);
        assert_eq!(menu.dropdown_list().map(|list| list.scroll), Some(30.0));
        assert_eq!(menu.list_scroll(), 0.0);
        // A pick changes the setting and closes the list.
        menu.pick_dropdown_list(4);
        assert!(menu.dropdown_list().is_none());
        assert_eq!(menu.user_settings().current_choice(index), Some(4));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Probe: every title-menu screen at several UI scales and window shapes (`RUSTIBITES_PROBE_OUT` = a folder for the PNGs).
    #[test]
    fn every_menu_screen_at_other_ui_scales() {
        let Ok(out) = std::env::var("RUSTIBITES_PROBE_OUT") else {
            return;
        };
        let Ok(Some(ui)) = UiAssets::discover() else {
            return;
        };
        let Ok(mut fonts) = FontSet::load(&ui) else {
            return;
        };
        let runtime = RuntimeUi::discover().ok().flatten();
        for (scale, w, h) in [(1.0_f32, 1920_u32, 1080_u32), (0.75, 1920, 1080), (1.25, 1920, 1080), (1.0, 2560, 1080), (1.0, 1280, 800)] {
            for (name, view) in [
                ("new", MenuView::NewGame),
                ("load", MenuView::LoadGame),
                ("settings", MenuView::Settings),
                ("library", MenuView::Library),
                ("achievements", MenuView::Achievements),
                ("tools", MenuView::Tools),
                ("credits", MenuView::Credits),
            ] {
                crate::ui::set_ui_scale_settings(true, 1.0, scale);
                let mut menu = menu();
                menu.open_view_for_capture(view);
                let frame = menu.build_frame(&ui, &mut fonts, w as f32, h as f32, runtime.as_ref());
                let overlay = menu_overlay(&ui, &frame, &mut fonts);
                let image = crate::render::render_ui_only(w, h, 1, Some(overlay));
                let _ = image.write_png(format!("{out}/{name}_{scale}_{w}x{h}.png"));
            }
        }
        crate::ui::set_ui_scale_settings(true, 1.0, 1.0);
    }

    #[test]
    fn a_click_on_a_title_button_hits_it_whatever_the_ui_scale_and_the_window() {
        let Ok(Some(runtime)) = RuntimeUi::discover() else {
            return; // extracted game assets are local-only
        };
        let menu = menu();
        // (UI automatic, resolution factor, UI scale), the picture's pixels, the window's logical pixels (a display scaled to 150 %, a window
        // of another shape, a fixed resolution factor that does not follow the size).
        for (automatic, resolution, scale) in [(true, 1.0, 1.0), (true, 1.0, 1.25), (true, 1.0, 0.5), (false, 1.0, 1.0), (false, 2.0, 1.15)] {
            for ((picture_w, picture_h), (screen_w, screen_h)) in [
                ((1920.0, 1080.0), (1920.0, 1080.0)),
                ((2560.0, 1440.0), (1706.7, 960.0)),
                ((2560.0, 1440.0), (2560.0, 1440.0)),
                ((1280.0, 720.0), (1280.0, 720.0)),
                ((2560.0, 1080.0), (2560.0, 1080.0)),
            ] {
                crate::ui::set_ui_scale_settings(automatic, resolution, scale);
                let rects = runtime.menu_button_rects(picture_w, picture_h);
                assert_eq!(rects.len(), 9);
                for (row, (pos, size)) in rects.iter().enumerate() {
                    // The button's centre as it is drawn in the picture, as the pointer finds it on the window.
                    let on_window = Vec2::new(
                        (pos.x + size.x / 2.0) * screen_w / picture_w,
                        (pos.y + size.y / 2.0) * screen_h / picture_h,
                    );
                    let point = picture_point(on_window, Vec2::new(screen_w, screen_h), Vec2::new(picture_w, picture_h));
                    let old = menu.hit_row(on_window.x, on_window.y, screen_w, screen_h, Some(&runtime));
                    if old != Some(row) {
                        eprintln!("(the old test in window pixels missed button {row}: scale {scale}, automatic {automatic}, picture {picture_w}x{picture_h}, window {screen_w}x{screen_h})");
                    }
                    assert_eq!(
                        menu.hit_row(point.x, point.y, picture_w, picture_h, Some(&runtime)),
                        Some(row),
                        "button {row} at scale {scale} (automatic {automatic}, factor {resolution}), picture {picture_w}x{picture_h}, window {screen_w}x{screen_h}"
                    );
                }
            }
        }
        crate::ui::set_ui_scale_settings(true, 1.0, 1.0);
    }

    #[test]
    fn menu_screenshot_frames_are_deterministic() {
        let Ok(Some(ui)) = UiAssets::discover() else {
            return; // extracted game assets are local-only
        };
        let Ok(mut fonts) = FontSet::load(&ui) else {
            return;
        };
        let menu = menu();
        let runtime = RuntimeUi::discover().ok().flatten();
        if let Some(runtime) = runtime.as_ref() {
            assert_eq!(runtime.menu_button_rects(1920.0, 1080.0).len(), 9);
        }
        let render = |ui: &UiAssets, fonts: &mut FontSet| {
            let frame = menu.build_frame(ui, fonts, 640.0, 400.0, runtime.as_ref());
            let overlay = menu_overlay(ui, &frame, fonts);
            crate::render::render_ui_only(640, 400, 1, Some(overlay)).rgba
        };
        let first = render(&ui, &mut fonts);
        let second = render(&ui, &mut fonts);
        assert_eq!(first, second);
    }

    #[test]
    fn menu_views_navigate_and_back() {
        let mut menu = menu();
        // Bibite Library / Achievements / Additional Tools / Credits.
        for (index, view) in [
            (3, MenuView::Library),
            (5, MenuView::Achievements),
            (6, MenuView::Tools),
            (7, MenuView::Credits),
        ] {
            menu.select(index);
            assert!(matches!(menu.activate(), MenuAction::None));
            assert_eq!(menu.view(), view);
            assert!(!menu.rows().is_empty());
            menu.back();
            assert_eq!(menu.view(), MenuView::Main);
        }
        // Settings view exposes scene toggles + TPS + apply/reset.
        menu.select(4);
        assert!(matches!(menu.activate(), MenuAction::None));
        assert_eq!(menu.view(), MenuView::Settings);
        let rows = menu.rows();
        assert!(rows[0].starts_with("Grid:"));
        assert!(rows[5].starts_with("Simulation TPS:"));
        let before = menu.settings();
        menu.select(0);
        let _ = menu.activate();
        assert_ne!(menu.settings().grid, before.grid);
        menu.select(7);
        let _ = menu.activate();
        assert_eq!(menu.settings(), MenuSettings::default());
    }

    #[test]
    fn settings_persist_and_reload() {
        let dir =
            std::env::temp_dir().join(format!("rustibites-menu-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("menu-settings.txt");
        let mut menu = MenuState::new_with_settings(None, &dir, &path);
        menu.select(4);
        let _ = menu.activate(); // Settings view
        menu.select(0);
        let _ = menu.activate(); // Grid -> Off
        menu.select(5);
        let _ = menu.activate(); // TPS 30 -> 60
        menu.select(6);
        let _ = menu.activate(); // Apply & Back
        assert_eq!(menu.view(), MenuView::Main);
        let loaded = MenuState::new_with_settings(None, &dir, &path);
        assert!(!loaded.settings().grid);
        assert_eq!(loaded.settings().sim_tps, 60);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn new_view_lists_scenarios() {
        let dir =
            std::env::temp_dir().join(format!("rustibites-menu-scenarios-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let default = dir.join("Default.zip");
        std::fs::write(&default, b"zip").expect("scenario");
        std::fs::write(dir.join("Ring.zip"), b"zip").expect("ring");
        let mut menu = MenuState::new(Some(default), &dir);
        assert_eq!(menu.scenarios().len(), 2);
        menu.select(1);
        let _ = menu.activate();
        assert_eq!(menu.view(), MenuView::NewGame);
        assert!(menu.rows().iter().any(|row| row == "Default.zip"));
        assert!(menu.rows().iter().any(|row| row == "Ring.zip"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn panel_state_stack_modal_and_escape() {
        use crate::ui::PanelKind;
        let mut panels = PanelState::default();
        assert!(panels.top().is_none());
        panels.open(PanelKind::Settings);
        panels.open(PanelKind::Library);
        assert!(panels.is_open(PanelKind::Settings));
        assert_eq!(panels.top(), Some(PanelKind::Library));
        assert!(panels.escape());
        assert_eq!(panels.top(), Some(PanelKind::Settings));
        panels.set_modal(Some(PanelKind::Escape));
        assert_eq!(panels.top(), Some(PanelKind::Escape));
        assert!(panels.escape());
        assert!(panels.modal().is_none());
        assert_eq!(panels.top(), Some(PanelKind::Settings));
        panels.toggle(PanelKind::Settings);
        assert!(!panels.is_open(PanelKind::Settings));
        assert!(!panels.escape());
    }

    #[test]
    fn a_panels_close_button_closes_the_panel_and_not_the_placer_or_tower_entry_above_it() {
        use crate::ui::PanelKind;
        // `sync_escape_stack` keeps an entry for the pellet placer, a bibite placer and a tower's panel on the stack, above whatever was opened before them.
        let mut panels = PanelState::default();
        panels.open(PanelKind::WorldDynamics);
        panels.open_once(PanelKind::PelletPlacer);
        panels.open_once(PanelKind::ToolPanel);
        panels.open_once(PanelKind::BibitePlacer);
        assert!(panels.close_top());
        assert!(!panels.is_open(PanelKind::WorldDynamics), "the panel whose X it was is closed");
        for entry in [PanelKind::PelletPlacer, PanelKind::ToolPanel, PanelKind::BibitePlacer] {
            assert!(panels.is_open(entry), "{entry:?} stays");
        }
        assert!(!panels.close_top(), "only the entries of the Escape stack are left: no panel to close");
        // Escape pops them, newest first.
        assert_eq!(panels.pop_escape(), Some(PanelKind::BibitePlacer));
        assert_eq!(panels.pop_escape(), Some(PanelKind::ToolPanel));
    }

    #[test]
    fn library_selector_rows_and_actions() {
        let dir = std::env::temp_dir().join(format!("rustibites-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("testdata")
            .join("real")
            .join("Default.zip");
        if !source.is_file() {
            eprintln!("skipping: testdata/real/Default.zip absent");
            return;
        }
        let scenario = dir.join("Default.zip");
        std::fs::copy(&source, &scenario).expect("scenario copy");
        let mut menu = MenuState::new(Some(scenario.clone()), &dir);
        // The library of this test is empty (`MenuState::new` reads the player's own Templates folder: with 221 of them the Create New row is not row 8).
        menu.templates.clear();
        menu.library_selected = None;
        menu.select(3); // Bibite Library row
        assert!(matches!(menu.activate(), MenuAction::None));
        assert_eq!(menu.view(), MenuView::Library);
        let rows = menu.rows();
        assert!(rows.iter().any(|row| row.contains("Create New")));
        assert!(rows.iter().any(|row| row == "Accept"));
        assert!(rows.iter().any(|row| row == "Edit"));
        let template_count = menu.templates().len().min(8);
        // Create New starts the default scenario.
        menu.select(template_count);
        assert_eq!(menu.activate(), MenuAction::Start(scenario.clone()));
        assert_eq!(menu.view(), MenuView::Loading);
        assert!(menu.pending_template().is_none());
        // Back to main and Accept uses the same scenario without a template.
        menu.back_to_main();
        menu.select(3);
        let _ = menu.activate();
        menu.select(template_count + 1);
        assert_eq!(menu.activate(), MenuAction::Start(scenario));
        assert!(menu.pending_template().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn newgame_options_cycle_and_generate_a_world() {
        let default = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("testdata/real/Default.zip");
        if !default.is_file() {
            return; // canonical scenario is local-only
        }
        let dir = std::env::temp_dir().join(format!("rustibites-newgame-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let mut menu = MenuState::new(Some(default), &dir);
        menu.select(1); // New
        let _ = menu.activate();
        assert_eq!(menu.view(), MenuView::NewGame);
        let rows = menu.rows();
        assert!(rows.iter().any(|row| row.starts_with("Size:")));
        assert!(rows.iter().any(|row| row.contains("Biomass Density:")));
        assert!(rows.iter().any(|row| row == "Create Scenario & Start"));
        let scenario_count = menu.scenarios.len().min(8);
        let before = menu.rows()[scenario_count].clone();
        menu.select(scenario_count);
        let _ = menu.activate();
        assert_ne!(
            menu.rows()[scenario_count],
            before,
            "size option did not cycle"
        );
        menu.select(scenario_count + 5); // Create Scenario & Start
        let action = menu.activate();
        assert!(matches!(action, MenuAction::Start(_)));
        assert_eq!(menu.view(), MenuView::Loading);
        assert!(
            !menu.pending_is_save(),
            "a new scenario is not a loaded game"
        );
        let path = menu.pending().expect("generated path").to_path_buf();
        assert!(
            path.is_file(),
            "generated world missing: {}",
            path.display()
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
