//! Game-asset core UI layer (P0).
//!
//! Loads `tools/ui/ui_atlas.json` (spec-dev's logical UI atlas) and the real
//! extracted sprites/fonts under `assets/game/`, builds a screen-space display
//! list ([`UiFrame`]) and draws it with either backend:
//!
//! * the interactive window draws with macroquad (`draw_frame_macroquad`);
//! * the headless software rasteriser draws the same list (`render.rs`), so
//!   CI screenshots include the real HUD without a display.
//!
//! Layout follows the atlas metadata: nine-slice borders, natural sizes,
//! pivots, default tints and the reference font sizes scaled by
//! `min(h/1080, w/1920)` (`UIScaler`). Records sharing one texture load it
//! once (86 records share extracted textures).
//!
//! The game art is user-supplied and not part of the MIT repo; when
//! `tools/ui/ui_atlas.json` is absent the caller keeps the old built-in HUD.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bibites_core::math::Vec2;
use fontdue::{Font, FontSettings};
use serde::Deserialize;

/// RGBA colour in 8-bit sRGB.
pub type Rgba = [u8; 4];

/// Font roles resolved by `ui_atlas.json` (`fonts.roles`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontRole {
    /// Body text (Roboto-Regular).
    Body,
    /// Headings / emphasis (Roboto-Bold).
    Bold,
    /// Numeric/monospace counters (Inconsolata-SemiBold).
    Mono,
}

/// One decoded UI texture (shared by many logical records).
#[derive(Clone, Debug)]
pub struct UiTexture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA rows, top to bottom.
    pub rgba: Vec<u8>,
}

/// One record of `tools/ui/ui_atlas.json` (only the fields the renderer uses).
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct UiRecord {
    /// Logical id (atlas key).
    pub logical: String,
    /// Extracted PNG path relative to the repo root.
    #[serde(default)]
    pub file: Option<String>,
    /// Kind (`sprite`, `nine_slice`, `icon`, `font`, `material`, ...).
    #[serde(default)]
    pub kind: String,
    /// Whether the sprite is a 9-slice frame.
    #[serde(default)]
    pub nine_slice: bool,
    /// Unity border `[left, bottom, right, top]` in texture pixels.
    #[serde(default)]
    pub border: Option<[f32; 4]>,
    /// Natural size in pixels.
    #[serde(default)]
    pub natural_size_px: Option<[f32; 2]>,
    /// Default tint.
    #[serde(default)]
    pub default_tint: Option<Rgba>,
    /// Normalised pivot `[x, y]`.
    #[serde(default)]
    pub pivot: Option<[f32; 2]>,
    /// Priority bucket (`P0`, `P0b`, ...).
    #[serde(default)]
    pub priority: String,
}

#[derive(Deserialize)]
struct AtlasJson {
    atlas: HashMap<String, UiRecord>,
}

/// Loaded UI assets: records, shared textures and fonts.
pub struct UiAssets {
    /// Repository root the manifest paths are relative to.
    pub root: PathBuf,
    /// Logical records by key.
    records: HashMap<String, UiRecord>,
    /// Shared textures.
    textures: Vec<UiTexture>,
    /// Logical id -> texture slot.
    texture_of: HashMap<String, u16>,
    /// Textures by source file (dedupe).
    texture_by_file: HashMap<String, u16>,
    /// Explicit sprite-name overrides (native button-theme sprites).
    texture_by_alias: HashMap<String, u16>,
}

impl std::fmt::Debug for UiAssets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UiAssets")
            .field("records", &self.records.len())
            .field("textures", &self.textures.len())
            .finish()
    }
}

impl UiAssets {
    /// No records and no textures: every sprite lookup misses, so drawing falls back to flat
    /// rectangles. For tests of the layout and the hit regions.
    #[cfg(test)]
    #[must_use]
    pub fn empty() -> Self {
        Self {
            root: PathBuf::from("."),
            records: HashMap::new(),
            textures: Vec::new(),
            texture_of: HashMap::new(),
            texture_by_file: HashMap::new(),
            texture_by_alias: HashMap::new(),
        }
    }

    /// Default locations of `tools/ui/ui_atlas.json` (cwd then crate).
    #[must_use]
    pub fn candidates() -> Vec<PathBuf> {
        vec![
            PathBuf::from("tools/ui/ui_atlas.json"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("tools/ui/ui_atlas.json"),
        ]
    }

    /// Finds and loads the UI atlas, returning `Ok(None)` when it is absent.
    pub fn discover() -> Result<Option<Self>, String> {
        for candidate in Self::candidates() {
            if candidate.is_file() {
                return Self::load(&candidate).map(Some);
            }
        }
        Ok(None)
    }

    /// Loads an atlas JSON and decodes every referenced texture
    /// (deduplicated by source file).
    pub fn load(atlas_path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(atlas_path)
            .map_err(|error| format!("could not read {}: {error}", atlas_path.display()))?;
        let parsed: AtlasJson = serde_json::from_str(&text)
            .map_err(|error| format!("invalid UI atlas JSON: {error}"))?;
        // `tools/ui/ui_atlas.json` -> repository root (paths in the atlas are
        // repo-relative, e.g. `assets/game/...`).
        let root = atlas_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let mut assets = Self {
            root,
            records: parsed.atlas,
            textures: Vec::new(),
            texture_of: HashMap::new(),
            texture_by_file: HashMap::new(),
            texture_by_alias: HashMap::new(),
        };
        // Eagerly decode each unique file; the UI set is ~10 MB and loading it
        // once keeps per-frame drawing allocation-free.
        let mut failures = 0;
        let mut ids: Vec<String> = assets.records.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let Some(file) = assets
                .records
                .get(&id)
                .and_then(|record| record.file.clone())
            else {
                continue;
            };
            // Material records point at JSON material dumps, not images.
            if !file.to_ascii_lowercase().ends_with(".png") {
                continue;
            }
            let key = file.clone();
            if let Some(texture) = assets.texture_by_file.get(&key).copied() {
                assets.texture_of.insert(id, texture);
                continue;
            }
            let path = assets.resolve(&file);
            match decode_png(&path) {
                Ok(texture) => {
                    assets.textures.push(texture);
                    let slot = (assets.textures.len() - 1) as u16;
                    assets.texture_by_file.insert(key, slot);
                    assets.texture_of.insert(id, slot);
                }
                Err(error) => {
                    failures += 1;
                    if failures <= 5 {
                        eprintln!("ui: {error}");
                    }
                }
            }
        }
        if assets.records.is_empty() {
            return Err("UI atlas has no records".to_owned());
        }
        Ok(assets)
    }

    /// Absolute path for a manifest-relative file.
    #[must_use]
    pub fn resolve(&self, file: &str) -> PathBuf {
        let path = Path::new(file);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    /// First available record among `candidates` (graceful fallback chrome).
    #[must_use]
    pub fn pick(&self, candidates: &[&str]) -> Option<&UiRecord> {
        candidates.iter().find_map(|id| self.records.get(*id))
    }

    /// Texture slot for a logical record.
    #[must_use]
    pub fn texture_of(&self, logical: &str) -> Option<u16> {
        self.texture_of.get(logical).copied()
    }

    /// Texture slot for a source file path (the runtime UI dump's sprite
    /// manifest carries the authoritative PNG path; the joined `logical` field
    /// is sometimes the wrong alias).
    #[must_use]
    pub fn texture_of_file(&self, file: &str) -> Option<u16> {
        self.texture_by_file.get(file).copied()
    }

    /// Registers a PNG under a runtime sprite name (overrides atlas joins).
    pub fn add_texture_alias(&mut self, name: &str, path: &Path) -> Result<u16, String> {
        let texture = decode_png(path)?;
        self.textures.push(texture);
        let slot = u16::try_from(self.textures.len() - 1)
            .map_err(|_| "too many UI textures".to_owned())?;
        self.texture_by_alias.insert(name.to_owned(), slot);
        Ok(slot)
    }

    /// Registers a cut of an alias's pixels (`x, y, w, h`) under a new sprite name (sprites that are a part of another one).
    pub fn add_cropped_alias(&mut self, name: &str, source: &str, rect: [u32; 4]) -> bool {
        let Some(slot) = self.texture_of_alias(source) else {
            return false;
        };
        let Some(texture) = self.textures.get(usize::from(slot)) else {
            return false;
        };
        let [x, y, w, h] = rect;
        if x + w > texture.width || y + h > texture.height {
            return false;
        }
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let start = ((row * texture.width + x) * 4) as usize;
            rgba.extend_from_slice(&texture.rgba[start..start + (w * 4) as usize]);
        }
        let cut = UiTexture {
            width: w,
            height: h,
            rgba,
        };
        let Ok(new_slot) = u16::try_from(self.textures.len()) else {
            return false;
        };
        self.textures.push(cut);
        self.texture_by_alias.insert(name.to_owned(), new_slot);
        true
    }

    /// Registers a copy of an alias's sprite with every pixel white and the alpha kept, under a new name: what the game's `flatMat`
    /// (`Unlit/flatMat`) draws, the sprite as a mask for the `Image`'s colour (the species panel's favourite heart).
    pub fn add_flat_alias(&mut self, name: &str, source: &str) -> bool {
        let Some(texture) = self
            .texture_of_alias(source)
            .and_then(|slot| self.textures.get(usize::from(slot)))
        else {
            return false;
        };
        let mut flat = texture.clone();
        for pixel in flat.rgba.chunks_exact_mut(4) {
            pixel[..3].fill(255);
        }
        let Ok(slot) = u16::try_from(self.textures.len()) else {
            return false;
        };
        self.textures.push(flat);
        self.texture_by_alias.insert(name.to_owned(), slot);
        true
    }

    /// Registers the outline `nodeActivation` draws around a selected neuron (`_Selected` 1), as a picture of the sprite's size under a new name: the
    /// four samples `radius` texels away (left, right, above, below; the texture clamps) give `outline = 1 - prod(1 - alpha) - alpha`, and the
    /// shader mixes the pixel (colour and alpha) toward blue (0, 0.0119, 1) by `sat(outline + alpha) * sat(outline)`; on the transparent pixels
    /// round the sprite that is blue of that alpha.
    pub fn add_outline_alias(&mut self, name: &str, source: &str, radius: i32) -> bool {
        let Some(texture) = self
            .texture_of_alias(source)
            .and_then(|slot| self.textures.get(usize::from(slot)))
        else {
            return false;
        };
        let (width, height) = (texture.width as i32, texture.height as i32);
        let alpha = |x: i32, y: i32| -> f32 {
            let at = ((y.clamp(0, height - 1) * width + x.clamp(0, width - 1)) * 4 + 3) as usize;
            f32::from(texture.rgba[at]) / 255.0
        };
        let mut rgba = vec![0_u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let centre = alpha(x, y);
                let around = [
                    alpha(x - radius, y),
                    alpha(x + radius, y),
                    alpha(x, y - radius),
                    alpha(x, y + radius),
                ];
                let outline = 1.0 - around.iter().map(|a| 1.0 - a).product::<f32>() - centre;
                let weight = (outline + centre).clamp(0.0, 1.0) * outline.clamp(0.0, 1.0);
                if weight <= 0.0 {
                    continue;
                }
                let at = ((y * width + x) * 4) as usize;
                rgba[at + 1] = (weight * 0.011_904 * 255.0).round() as u8;
                rgba[at + 2] = (weight * 255.0).round() as u8;
                rgba[at + 3] = (weight * 255.0).round() as u8;
            }
        }
        let Ok(slot) = u16::try_from(self.textures.len()) else {
            return false;
        };
        self.textures.push(UiTexture {
            width: texture.width,
            height: texture.height,
            rgba,
        });
        self.texture_by_alias.insert(name.to_owned(), slot);
        true
    }

    /// The decoded pixels of a texture slot.
    #[must_use]
    pub fn texture_pixels(&self, slot: u16) -> Option<&UiTexture> {
        self.textures.get(slot as usize)
    }

    /// Sprite-name override slot, when registered.
    #[must_use]
    pub fn texture_of_alias(&self, name: &str) -> Option<u16> {
        self.texture_by_alias.get(name).copied()
    }

    /// Decodes and registers one PNG for runtime-sprite lookup by file path.
    pub fn add_texture_file(&mut self, file: &str) -> Result<u16, String> {
        if let Some(slot) = self.texture_of_file(file) {
            return Ok(slot);
        }
        let path = self.resolve(file);
        let texture = decode_png(&path)?;
        self.textures.push(texture);
        let slot = u16::try_from(self.textures.len() - 1)
            .map_err(|_| "too many UI textures".to_owned())?;
        self.texture_by_file.insert(file.to_owned(), slot);
        Ok(slot)
    }

    /// Texture data by slot.
    #[must_use]
    pub fn texture(&self, slot: u16) -> Option<&UiTexture> {
        self.textures.get(slot as usize)
    }

    /// Number of decoded textures.
    #[must_use]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Font path for a role, honouring `fonts.roles` when present (the atlas
    /// generator keeps the TTF paths in `fonts.files`; the renderer resolves
    /// the three roles it needs).
    #[must_use]
    pub fn font_path(&self, role: FontRole) -> PathBuf {
        let name = match role {
            FontRole::Body => "Roboto-Regular.ttf",
            FontRole::Bold => "Roboto-Bold.ttf",
            FontRole::Mono => "Inconsolata-SemiBold.ttf",
        };
        self.resolve(&format!("assets/game/fonts/{name}"))
    }
}

/// Stable widget id: the identity of a click area. [`UiFrame::interact`] answers a click with every action registered under the id of the topmost area
/// under the pointer, [`UiFrame::action_at`] with the first one, and a button held down is found (and drawn pressed) by its id: two panels that can be up
/// together must not share an id (a click on the Save Bibite dialog closed the Information panel behind it when both had 8801). Each panel owns a block
/// of its own, written down at its constants: below 8000 the curated HUD (`hud.rs`), 8000.. the Graphs page and the pause menu, 8150 the achievements'
/// pop-up, 8210.. the Gene Analysis page, 8500.. the selection panel, 8600.. the tower panel, 8700.. the placers, 8800.. the Information panel, 8900.. the
/// Zones preview (8901 the click area of its zones), 8950.. the Lineage panel, 9000.. the Biology pop-ups, 9010.. the Save Bibite dialog, 9300.. the zone colour
/// pop-up (`zone_color_ui.rs`), 9400.. the challenge tracker and end pop-up,
/// 9500.. the Save Game panel, 9600.. the time tags' controls of the three panels that show them and 9700.. 9800.. 9900.. their tags' remove buttons
/// (`time_tags::TagPanel::hit_base`); 20_000.. the captured runtime frames, 46_000.. the dropdown and the editors; the lists that grow with the world's
/// content start far apart (lineage shapes 200_000, neurons 900_000.., tags 930_000). `UiFrame::shared_ids` (tests) lists the ids a frame has twice.
pub type UiId = u32;

/// Title-menu screen commands (the native menu panels emit these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    /// "Return to Menu" / close button.
    Back,
    /// Switch between the scenario list and the challenge list.
    ToggleChallenges,
    /// Select the n-th visible scenario/challenge row.
    SelectScenario(u16),
    /// "Start Simulation".
    StartSimulation,
    /// The challenge panel's "Select Champion" / "Select Other" (`ScenarioSelectorPanel.SelectChampion`): the Library opens to pick.
    SelectChampion,
    /// The challenge panel's "Start Challenge" (`true`) and the small start button beside it (`false`: the scenario as a scenario),
    /// `ScenarioSelectorPanel.StartChallenge(asChallenge)`.
    StartChallenge(bool),
    /// "TPS settings" panel toggle.
    TpsSettings,
    /// Open the scenarios folder in the file explorer.
    OpenFolder,
    /// Select the n-th row of the Load Game save list.
    SelectSave(u16),
    /// A press on the handle of the Load Game list's scrollbar (the drag follows the held button).
    ListBarHandle,
    /// A press on the track of the Load Game list's scrollbar, above (`-1`) or under (`1`) the handle: the list moves a page.
    ListBarPage(i8),
    /// "Load" in the Load Game panel.
    LoadSelected,
    /// Toggle "include autosaves" in the Load Game panel.
    ToggleAutosaves,
    /// Ask to delete the selected save (the game shows a confirmation).
    DeleteSave,
    /// Answer the delete confirmation.
    ConfirmDelete(bool),
    /// Tick / untick the "do not show again" checkbox of the warning popup that is up.
    ToggleDoNotShowAgain,
    /// The Settings screen's "Reset Warnings" button (`UserSettingsManager.ResetAllWarnings`).
    ResetWarnings,
    /// The Library's "Templates" (0) and "Saved" (1) buttons (`BibiteTemplateSelectorPanel.OpenTab`).
    LibraryTab(u8),
    /// The Library's Reload button (`FillLists`).
    LibraryReload,
    /// The Library's folder button (`OpenSaveFolder`).
    LibraryOpenFolder,
    /// The Library's Delete button (`DeleteSelectedAction`: the warning asks first).
    LibraryDelete,
    /// Answer the Library's delete warning.
    ConfirmLibraryDelete(bool),
    /// The OK button of the error popup (`PopupManager.DisplayError`).
    DismissError,
    /// A feature that is not available yet (shows a status message).
    Unavailable(u8),
    /// Open a link (`URLManager`), by its place in `menu::URLS`: 0 wiki, 1 research Discord, 2 Patreon, 3 Reddit, 4 Twitter, 5 BlueSky, 6 YouTube.
    OpenUrl(u8),
    /// Select a settings tab (0 graphics, 1 gameplay, 2 hotkeys, 3 music, 4 species names, 5 towers, 6 extra hotkeys and 7 calibration: RUSTibites Improved's, and the
    /// calibration page Faithful has too).
    SettingsTab(u8),
    /// The Calibration page's "Run Test" button: starts the calibration test, or stops the one that is going on.
    CalibrationRun,
    /// The Extra Hotkeys page's key button (RUSTibites Improved): the next key pressed is the Hide Objects hotkey's key.
    KeyRebind,
    /// The Extra Hotkeys page's "Reset" button: every extra hotkey has its default key again.
    KeysReset,
    /// The Species Names page's "Open File" button (RUSTibites Improved): the file of the player's words in their text editor.
    NamesOpenFile,
    /// The Species Names page's "Reload" button: the file is read again now.
    NamesReload,
    /// The Species Names page's "Show New" button: other sample names.
    NamesSamples,
    /// The Music page's "Open Folder" button (RUSTibites Improved): the folder of the music in the file manager.
    MusicOpenFolder,
    /// The Music page's "Previous" button.
    MusicPrevious,
    /// The Music page's "Next" button.
    MusicNext,
    /// Select the n-th row of the Bibite Library template list.
    SelectTemplate(u16),
    /// The Library's submit button (`SubmitTemplate`): hands the selected template to whoever opened the Library to pick one, and in
    /// a Library opened to edit ("Edit Bibite") opens the Bibite Editor on it.
    StartTemplate,
    /// The Library's "Edit Button" (`EditSelectedBibite`): the Bibite Editor on the selected template.
    EditTemplate,
    /// The Library's "CreateNew" button (`OpenEditorForNewBibite`): the Bibite Editor on a new template.
    NewTemplate,
    /// Flip the checkbox setting `n`.
    SettingToggle(u16),
    /// Reset setting `n` to its default.
    SettingReset(u16),
    /// Click on the slider of setting `n` whose track spans x `.1..=.2` (menu pixels).
    SettingSlide(u16, u16, u16),
    /// Click on TPS-panel slider `n` whose track spans x `.1..=.2` (menu pixels).
    TpsSlide(u8, u16, u16),
    /// Reset TPS-panel slider `n` to its default.
    TpsReset(u8),
    /// "Save as Default" of the TPS panel.
    TpsSaveDefault,
    /// Press a main-menu button while a side panel is open (index = main menu row).
    MainButton(u8),
    /// New Game screen: "New Scenario" (opens the scenario editor on the default scenario).
    NewScenario,
    /// New Game screen: edit the selected scenario.
    EditScenario,
    /// Scenario editor: "Start" (asks first).
    EditorStart,
    /// Scenario editor: "Back" (asks first).
    EditorBack,
    /// Scenario editor: "Save Scenario" (opens the save form).
    EditorSave,
    /// Scenario editor dialog: answer yes (`true`) or no/cancel (`false`).
    EditorAnswer(bool),
    /// Scenario editor save form: focus the name (0) or description (1) field.
    EditorFocus(u8),
    /// Scenario editor: the `+` button of list `.0` (0 zones, 1 zone groups, 2 default bibites,
    /// 3 animators).
    EditorAdd(u8),
    /// Scenario editor: the "remove all" button of list `.0`.
    EditorClear(u8),
    /// Scenario editor challenge tab: turn the scenario into a challenge (`true`) or back.
    EditorChallenge(bool),
    /// Scenario editor: delete item `.1` of list `.0` (asks first).
    EditorDelete(u8, u16),
}

/// The control a dropdown list belongs to: what the list offers and what picking an option does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropdownTarget {
    /// Dropdown field `.1` of zone `.0` (`zone_editor` field ids), in the world or the editor.
    Zone(u16, u8),
    /// Dropdown field `.1` of default bibite `.0` (`bibite_editor` field ids).
    Bibite(u16, u8),
    /// One of the bibite placer's dropdowns (`bibite_editor` field id).
    Placer(u8),
    /// The pellet placer's material dropdown.
    PelletMaterial,
    /// The selected tower's Follow Zone dropdown.
    ToolZone,
    /// RUSTibites Improved: the selected gene selector's gene dropdown.
    ToolGene,
    /// The dropdown row of the Settings screens' setting `.0` (an index of `user_settings::DEFS`).
    Setting(u16),
    /// New Game screen modifier `.0` (size, biomass, fertility, radioactivity, bibite limit).
    Modifier(u8),
    /// The Graphs panel's Data dropdown (`RealDropdown`): the stream groups.
    GraphsData,
    /// The Graphs panel's Curves dropdown (`TogglesDropdown`): which curves of the group show.
    GraphsCurves,
    /// The Gene Analysis page's dropdown (`GraphsPanel.genesDropdown`, a `RealDropdown`): the gene whose distribution is shown.
    GraphsGene,
    /// A captured dropdown control without a target of its own ([`crate::app::App::open_dropdown`]).
    Captured,
    /// The species panels' "Compared species" dropdown (`SpeciesTabManager.comparedSpeciesDropdowns`):
    /// the species of the graph.
    SpeciesCompared,
    /// The Bibite Editor's node panel's "Activation Function" dropdown (`NodeEditPanel.functionDropdown`): the functions a hidden
    /// node can have.
    NodeFunction,
    /// The animator creator's preset dropdown.
    AnimatorPreset,
    /// The "Add Setting" dropdown of animator `.0`: the settings no animator has yet.
    AnimatorBase(u16),
    /// The Interp. dropdown of the point editor of the graph of animator `.0`.
    AnimatorInterpolation(u16),
}

/// Actions a widget can emit (full functional-UI framework).
#[allow(dead_code)] // TODO(bind): runtime-control registry/panel bindings construct the rest
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiAction {
    /// A press on the plot of animator `.0`'s graph (`.1`, `.2`, `.3`, `.4`: the plot's left, top, right and bottom edge in reference px): the point
    /// under it is picked and dragged (with Ctrl it is removed, or a point is added on empty ground).
    AnimGraph(u16, u16, u16, u16, u16),
    /// A dropdown was clicked: its list opens under it (`TMP_Dropdown.Show`).
    DropdownOpen(DropdownTarget),
    /// An option of the open dropdown list was clicked (index into its choices).
    DropdownPick(u16),
    /// The click that closes the open dropdown list without picking (its blocker).
    DropdownClose,
    /// Swallow the click (full-screen modal blocker behind a panel).
    Consume,
    /// Toggle pause/run.
    TogglePause,
    /// Advance exactly one simulation tick.
    Step,
    /// Set the speed multiplier (1..=8).
    SetSpeed(f32),
    /// Selects a Graphs-panel time-range tab (data-logger scale index).
    SetGraphsScale(usize),
    /// "Go To Gene Analysis" (`true`) / "Go To Data Graphs" (`false`): `GraphsPanel.OpenGeneAnalysis` / `OpenHistoricalGraph`.
    GraphsPage(bool),
    /// RUSTibites Improved: a panel's "Tag time" button (the name field opens; pressed again as "Add tag" it sets the tag).
    TimeTagStart(crate::time_tags::TagPanel),
    /// RUSTibites Improved: a press on the name field of a time tag (keeps it open).
    TimeTagField,
    /// RUSTibites Improved: the remove button of time tag `.0`.
    TimeTagRemove(u32),
    /// RUSTibites Improved: a panel's show / hide button of the time tags.
    TimeTagsShow(crate::time_tags::TagPanel),
    /// RUSTibites Improved: a click on a panel's graph while its time tag name field is open: the tag is set at `.1` (seconds of the world's clock), the
    /// moment the pointer stood on when the frame was drawn.
    TimeTagPlace(crate::time_tags::TagPanel, f64),
    /// RUSTibites Improved: a click on zone number `.0` in the Zones tab's Preview: the zone's colour pop-up opens at the pointer.
    ZoneColorOpen(u16),
    /// RUSTibites Improved: the colour pop-up's colour wheel was pressed (the wheel follows the pointer while the button is held, see `App::zone_color_drag_to`).
    ZoneColorWheel,
    /// RUSTibites Improved: the colour pop-up's brightness bar was pressed (held like the wheel).
    ZoneColorValueBar,
    /// RUSTibites Improved: the colour pop-up's hex field was pressed: the keyboard is in it.
    ZoneColorHexField,
    /// RUSTibites Improved: "Fertility Color" of the colour pop-up of zone `.0`: the zone's colour is taken away.
    ZoneColorClear(u16),
    /// RUSTibites Improved: "All different" of the colour pop-up: every zone of the world gets a colour of its own.
    ZoneColorSpread,
    /// RUSTibites Improved: "Done" or a click outside the colour pop-up: it closes and the zone keeps its colour.
    ZoneColorClose,
    /// A control of the Bibite Editor.
    Editor(crate::template_editor_ui::EditorAction),
    /// A control of the Expanded Brain View.
    ExpandedBrain(crate::expanded_brain_ui::ExAction),
    /// A control of the Save Game panel.
    SavePanel(crate::save_panel::SaveAction),
    /// The Gene Analysis page's time slider was pressed on its track (track x range, reference px).
    GenesTimeSlide(u16, u16),
    /// The same on its handle, which keeps its place under the cursor.
    GenesTimeSlideHandle(u16, u16),
    /// Pressing a bottom-left toolbar button shows its panel at once (`OnHoverShow.ShowElement`;
    /// 0 placer, 1 user actions, 2 view). Resting on the button opens it too, see
    /// [`crate::app::HoverPanel`].
    ShowToolbar(u8),
    /// Dismisses the lineage "not enough data" dialog.
    DismissPopup,
    /// Flips the boolean scenario setting at this index of `ScenarioSettings::SETTINGS`.
    ToggleScenarioSetting(u16),
    /// Time-speed slider pressed (track x range, reference px).
    SpeedSlide(u16, u16),
    /// Dynamic settings panel: select a tab.
    DynTab(u8),
    /// Dynamic settings panel: flip the toggle `SETTING_UI[n]`.
    DynToggle(u16),
    /// Dynamic settings panel: pressed on the slider of `SETTING_UI[n]` (track x range, reference px).
    DynSlide(u16, u16, u16),
    /// The species Position tab's time slider was pressed on its track (track x range, reference px).
    PositionSlide(u16, u16),
    /// The same on its handle, which keeps its place under the cursor.
    PositionSlideHandle(u16, u16),
    /// A neuron of the species Brain tab was pressed (its index): it follows the cursor while held.
    BrainNode(u16),
    /// The Brain tab's Sort button (`SortN`).
    BrainSort,
    /// The Brain tab's Save pos. button (`SaveNodePositions`).
    BrainSavePositions,
    /// The Brain tab's Reset pos. button (`ResetNodePositions`).
    BrainResetPositions,
    /// The Brain tab's "Highlight differences" toggle.
    BrainToggleDifferences,
    /// The Sort toggle of the Biology panel's Flow details pop-up (`UserSettings.SortBiologyPanelEnergyDetails`).
    ToggleEnergySort,
    /// RUSTibites Improved: the "Zone names" check box of the Species Distribution window.
    ToggleMapZoneNames,
    /// RUSTibites Improved: the "Zone biomass" check box of the Species Distribution window.
    ToggleMapZoneBiomass,
    /// A neuron of the inspector's Brain tab was pressed (its index): it follows the cursor while held.
    InspectorBrainNode(u16),
    /// The inspector's Brain tab's Sort button (`SortN`).
    InspectorBrainSort,
    /// Dynamic settings panel: reset `SETTING_UI[n]` to its default.
    DynReset(u16),
    /// A slider row's value text was pressed: its edit field opens (`SettingSlider.OpenEditPanel`).
    DynEditValue(crate::dyn_panel::FieldRef),
    /// A press on the open edit field itself, which keeps the focus.
    ValueEditInside,
    /// Dynamic settings panel: open/close sub-panel `n` of the current tab.
    DynSub(u16),
    /// Dynamic settings panel: reset every setting of sub-panel `n`.
    DynResetGroup(u16),
    /// Dynamic settings panel: "Save Scenario" button.
    DynSaveScenario,
    /// Scenario editor zone item: pressed on the slider of field `.1` of zone `.0`
    /// (track x range, reference px).
    ZoneSlide(u16, u8, u16, u16),
    /// Scenario editor zone item: flip toggle field `.1` of zone `.0`.
    ZoneToggle(u16, u8),
    /// Scenario editor zone item: advance dropdown field `.1` of zone `.0`.
    ZoneCycle(u16, u8),
    /// Scenario editor zone item: reset field `.1` of zone `.0`.
    ZoneReset(u16, u8),
    /// Starts dragging slider `.1` of default bibite `.0` (track x range in reference px).
    BibiteSlide(u16, u8, u16, u16),
    /// Advances dropdown `.1` of default bibite `.0` to its next choice.
    BibiteCycle(u16, u8),
    /// Resets slider `.1` of default bibite `.0`.
    BibiteReset(u16, u8),
    /// Pressed on slider `.0` of the selected tower's or colour killer's panel (`tower_panel`
    /// field id; track x range in reference px).
    ToolSlide(u8, u16, u16),
    /// The tower panel's "Follow Zone" dropdown: advance to the next choice.
    ToolZone,
    /// One of the bibite placer's dropdowns (`bibite_editor` field `.0`: tagging, growth at
    /// spawn, mutate genes): advance to the next choice.
    PlacerCycle(u8),
    /// One of the colour killer panel's two switches (Body / Eyes, Any / Most): flip field `.0`.
    ToolSwitch(u8),
    /// Scenario editor: open / close the edit section of item `.1` of list `.0`.
    ZoneOpen(u8, u16),
    /// A card's move arrow (`ReorderableItem.MoveUp` / `MoveDown`): item `.1` of list `.0` (0 zones, 1 zone groups, 3 settings animators)
    /// moves one place up (`.2`) or down in the list as it is shown (the data keeps its order).
    ListMove(u8, u16, bool),
    /// The name line of zone `.0` in the world's Zones list (`TextLineHandle.OpenEditSection`):
    /// type a new name.
    ZoneNameEdit(u16),
    /// Scenario editor material block: pressed on the slider of field `.1` of material `.0`
    /// (track x range, reference px).
    MaterialSlide(u8, u8, u16, u16),
    /// Scenario editor material block: flip the decay toggle of material `.0`.
    MaterialToggle(u8),
    /// Scenario editor material block: reset field `.1` of material `.0`.
    MaterialReset(u8, u8),
    /// Scenario editor material block: reset every parameter of material `.0`.
    MaterialResetAll(u8),
    /// Title-menu screen command.
    Menu(MenuCommand),
    /// Set the speed from a slider (1..=8).
    SetSpeedSlider(f32),
    /// Toggle one scene layer.
    ToggleOverlay(OverlayKind),
    /// Switch the inspector tab.
    SetTab(InspectorTab),
    /// Close the selection/inspector panel.
    CloseInspector,
    /// Open a panel/modal.
    OpenPanel(PanelKind),
    /// Close the topmost panel/modal.
    ClosePanel,
    /// Toggle a panel.
    TogglePanel(PanelKind),
    /// Change one control value (toggles/sliders/dropdowns/colour pickers).
    SetValue(ControlValue),
    /// Invoke a named command (save/load/spawn/tools/lineage/library/...).
    Invoke(ControlCommand),
    /// Activate a title-menu row (index in the current menu view).
    MenuRow(u32),
    /// Activate an audited runtime control by stable id (bench registry).
    ActivateControl(u64),
    /// Select a dynamic save/load browser row.
    SelectSaveRow(u32),
    /// Toggle inclusion of autosaves in the browser.
    ToggleAutosaves,
    /// The Save Bibite dialog: focus a text field (`.0`: 0 name, 1 description).
    SaveDialogFocus(u8),
    /// The Save Bibite dialog: flip a checkbox (`.0`: 0 "Save as template", 1 "Format JSON").
    SaveDialogToggle(u8),
    /// The scenario saver: a control of its form (`.0`: one of `scenario_saver::ACTION_*`).
    ScenarioSaver(u8),
    /// The pellet placer's material dropdown: the other material (`PelletPlacer.ChangeMaterial`).
    PelletMaterial,
    /// Pressed on a pellet placer slider (`.0`: 0 pellet size factor, 1 pellet number; track x range
    /// in reference px).
    PelletSlide(u8, u16, u16),
    /// Pressed on the Information panel's tags list scroll bar (`true`: the handle, which is then dragged;
    /// `false`: the track, which pages towards the click).
    InformationScroll(bool),
    /// Select a genome editor row.
    SelectGene(u32),
    /// Adjust the selected gene by +/-1 step.
    AdjustGene(i32),
    /// Page the genome editor list by +/-1 page.
    ScrollGene(i32),
    /// Select a brain editor node.
    SelectBrainNode(u32),
    /// Select a species row in the lineage panel.
    SelectSpecies(u32),
    /// Switch the species panel's tab (0 info, 1 genes, 2 brain, 3 position:
    /// `SpeciesTabManager.OpenPanel`).
    SetLineageTab(u32),
    /// The species name was clicked: it becomes a text field (`SpeciesInfoPanel.EditSpeciesName`).
    SpeciesNameEdit,
    /// The species notes field was clicked: it takes the keyboard.
    SpeciesNotes,
    /// The parent species button (`SpeciesTabManager.SelectParent`).
    SpeciesParent,
    /// The root species button (`SpeciesTabManager.SelectRoot`).
    SpeciesRoot,
    /// Save the species' template (`SpeciesInfoPanel.SaveSpecies`).
    SpeciesSave,
    /// Place bibites of the species (`SpeciesInfoPanel.PlaceSpecies`).
    SpeciesPlace,
    /// Prune the species from the tree (`SpeciesInfoPanel.PruneSpecies`, with its warning).
    SpeciesPrune,
    /// Edit the species' bibite (`SpeciesPanel.EditBibite`).
    SpeciesEdit,
    /// The close tab: the species panels close, the graph stays (`CloseSpeciesPanels`).
    SpeciesClose,
    /// Begin rebinding the hotkey action at this index.
    RebindHotkey(u32),
    /// Restore all default hotkeys.
    ResetHotkeys,
    /// Adjust a challenge parameter row (0 timer, 1..3 stars) by +/-1.
    AdjustChallengeParam(u32, i32),
    /// Switch the Settings page (0 graphics, 1 gameplay, 2 hotkeys).
    SetSettingsPage(u32),
    /// Select a local Workshop file row.
    SelectWorkshopRow(u32),
    /// Select a challenge scenario row.
    SelectChallengeRow(u32),
    /// Select a champion template row.
    SelectChampionRow(u32),
    /// Pick an item from the open dropdown (index into its options).
    PickDropdownChoice(u32),
    /// A press on a species of the lineage tree (`LineageSpeciesNode.OnPointerClick`): the species' id.
    SelectLineageNode(u32),
    /// The Lineage panel's gear button (`LineagePanelSettingsManager.TogglePanel`).
    ToggleLineageSettings,
    /// A toggle of the Lineage panel's Settings panel (`lineage_settings::LineageOption::index`).
    LineageOption(u8),
    /// The Lineage panel's Mesh (`false`) and Tree (`true`) buttons (`SpeciesPanel.SelectPanel(bool tree)`).
    SelectLineagePanel(bool),
    /// Quit the viewer.
    Quit,
    /// Toggle the help overlay.
    ToggleHelp,
}

/// Panels, modals and inspector views addressable from the UI framework.
#[allow(dead_code)] // TODO(bind): panel views render these kinds incrementally
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelKind {
    /// In-game HUD.
    Hud,
    /// Stats inspector tab.
    Stats,
    /// Genes inspector tab.
    Genes,
    /// Biology inspector tab.
    Biology,
    /// Brain inspector tab.
    Brain,
    /// Simulation settings.
    Settings,
    /// Bibite library/editor list.
    Library,
    /// Achievements gallery.
    Achievements,
    /// Additional tools / community links.
    Tools,
    /// Credits.
    Credits,
    /// Load-game panel.
    LoadGame,
    /// Save-game panel.
    SaveGame,
    /// Scenario picker.
    ScenarioPicker,
    /// Scenario/settings editor.
    ScenarioEditor,
    /// Genome editor.
    GeneEditor,
    /// The Bibite Editor (`BibiteEditorPanel`): the genes and the brain of a template.
    BibiteEditor,
    /// The Expanded Brain View (`ExpandedBrainPanel`): the selected bibite's brain on the whole window.
    ExpandedBrain,
    /// Brain editor.
    BrainEditor,
    /// In-world simulation settings.
    WorldSettings,
    /// In-world dynamic parameters.
    WorldDynamics,
    /// Local Workshop file browser.
    Workshop,
    /// Challenge/champion picker.
    ChallengePicker,
    /// Escape/pause menu.
    Escape,
    /// Help overlay.
    Help,
    /// Biomass / entity-count history graphs (`GraphsPanel`).
    Graphs,
    /// Counts / energy / top species (`InformationPanel`).
    Information,
    /// The challenge's tracker (`ChallengeTrackerPanel`, `ChallengeInfo`).
    ChallengeTracker,
    /// Lineage graph.
    Lineage,
    /// Species browser.
    Species,
    /// Escape-stack marker, not a panel: a target is selected (`UserControl.bibiteSelectedAction`).
    Target,
    /// Escape-stack marker, not a panel: an inspector panel is open (`BibitePanel`).
    Inspector,
    /// Escape-stack marker, not a panel: the multi-selection panel is open (`SelectionPanel`).
    Multi,
    /// Escape-stack marker, not a panel: the panel of a selected tower or colour killer is open
    /// (`PheroTowerPanel`, `RadTowerPanel`, `ColorKillerPanel`).
    ToolPanel,
    /// Escape-stack marker, not a panel: bibites are being placed from a template
    /// (`BibitePlacer`).
    BibitePlacer,
    /// Escape-stack marker, not a panel: the pellet placer's panel is open (`PelletPlacer`).
    PelletPlacer,
    /// Escape-stack marker, not a panel: the Expanded View's settings panel was opened (`UIPanel.OpenPanel`). It stays on the stack when the view closes
    /// under it (the panel's own object is only deactivated, not closed), and Escape then spends a press on it.
    ExpandedSettings,
}

impl PanelKind {
    /// The kinds that only mark an entry of the Escape stack (`UINavigationManager`) and are not
    /// drawn as panels by the overlay machinery.
    #[must_use]
    pub fn is_escape_marker(self) -> bool {
        matches!(self, Self::Target | Self::Inspector | Self::Multi | Self::ExpandedSettings)
    }

    /// The entries [`crate::app::App`] keeps on the Escape stack for a placer or a tower's panel (`sync_escape_stack`): Escape pops them, and a panel's own
    /// close button does not (it closes the panel it is on, whatever opened after it; the placers have close buttons of their own).
    #[must_use]
    pub fn is_tool_entry(self) -> bool {
        matches!(self, Self::ToolPanel | Self::BibitePlacer | Self::PelletPlacer)
    }
}

/// A typed control value emitted by toggles/sliders/dropdowns/pickers.
#[allow(dead_code)] // TODO(bind): settings/inspector widgets emit these
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlValue {
    /// Boolean toggle.
    Flag(bool),
    /// Integer field (spinner/dropdown index).
    Integer(i32),
    /// Floating-point slider value.
    Float(f32),
    /// Enum/dropdown index.
    Choice(u32),
    /// Colour picker (RGBA).
    Colour([u8; 4]),
}

/// Named commands invoked by buttons/menu rows.
#[allow(dead_code)] // TODO(bind): save/load/spawn/tools commands bind to world IO next
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlCommand {
    /// Toggle pause.
    TogglePause,
    /// Advance one tick.
    Step,
    /// Save the current world.
    SaveWorld,
    /// Quick-save.
    QuickSave,
    /// Load the most recent save.
    QuickLoad,
    /// Open the save panel.
    OpenSavePanel,
    /// Open the load panel.
    OpenLoadPanel,
    /// Open the scenario picker.
    OpenScenarioPicker,
    /// Open the scenario/settings editor.
    OpenScenarioEditor,
    /// Open settings.
    OpenSettings,
    /// Apply edited settings.
    ApplySettings,
    /// Reset settings to defaults.
    ResetSettings,
    /// Apply the scenario editor settings.
    EditorApply,
    /// Save the scenario editor state to the source scenario.
    EditorSave,
    /// Discard the scenario editor changes.
    EditorCancel,
    /// Add a zone in the scenario editor.
    EditorAddZone,
    /// Remove the selected zone.
    EditorRemoveZone,
    /// Open the genome editor.
    OpenGeneEditor,
    /// Open the brain editor.
    OpenBrainEditor,
    /// Randomize the edited genome.
    EditorRandomize,
    /// Mutate one gene of the edited genome.
    EditorMutate,
    /// Add a hidden brain node.
    EditorAddNode,
    /// Remove the selected hidden brain node.
    EditorRemoveNode,
    /// Add a synapse between the first two usable nodes.
    EditorAddSynapse,
    /// Toggle the selected species' favourite flag (`SetFavorite`).
    SpeciesToggleFavorite,
    /// Select a random living member of the selected species
    /// (`SelectRandomOfSpecies`).
    SpeciesSelectRandomMember,
    /// Cycle the selected hidden node's activation function.
    EditorCycleNodeType,
    /// Raise the weight of the connection previous-node -> selected-node.
    EditorWeightUp,
    /// Lower the weight of the connection previous-node -> selected-node.
    EditorWeightDown,
    /// Enable / disable the connection previous-node -> selected-node.
    EditorToggleSynapse,
    /// Remove the connection previous-node -> selected-node.
    EditorRemoveSynapse,
    /// Raise the selected node's bias by one step.
    EditorBiasUp,
    /// Lower the selected node's bias by one step.
    EditorBiasDown,
    /// Validate the edited brain.
    EditorValidate,
    /// Save challenge parameters into the source scenario.
    ChallengeSaveParams,
    /// Explicit status for a captured control with no implementation in this build.
    UnsupportedControl,
    /// Open the local saves/scenarios folder in the OS file manager.
    OpenFolder,
    /// Scroll the currently focused list one step.
    ScrollList,
    /// Save the edited genome/brain as a template.
    EditorSaveTemplate,
    /// Load the selected template into the editor (continue editing).
    EditorLoadTemplate,
    /// Anchor the selected brain node at its current editor position.
    EditorAnchorNode,
    /// Clear all brain node anchors.
    EditorClearAnchors,
    /// Start a local scored challenge.
    StartChallenge,
    /// Open the local Workshop browser.
    OpenWorkshop,
    /// Open the challenge/champion picker.
    OpenChallengePicker,
    /// Refresh the local Workshop entries.
    WorkshopRefresh,
    /// Online Workshop action (Steam-only in this build).
    WorkshopOnline,
    /// Rename the selected local Workshop entry using the draft name.
    WorkshopRename,
    /// Export the selected Workshop entry.
    WorkshopExport,
    /// Delete the selected Workshop entry.
    WorkshopDelete,
    /// `ChallengeEndPopup.ContinueAsScenario`: the world goes on as a scenario (the session is dropped).
    ChallengeContinue,
    /// `ChallengeEndPopup.ReturnToMenu`.
    ChallengeReturnToMenu,
    /// The tracker's close button (`ChallengeTrackerPanel.ClosePanel`).
    ChallengeTrackerClose,
    /// Restart the current local challenge.
    ChallengeRestart,
    /// Spawn a bibite.
    SpawnBibite,
    /// Kill the selected bibite.
    KillSelected,
    /// Ask for confirmation before killing the selected bibite.
    RequestKill,
    /// Ask for confirmation before killing half the population.
    RequestKillHalf,
    /// Ask to remove half of all pellets (`UserActions.RemoveHalfPelletsPressed`).
    RequestRemoveHalfPellets,
    /// The top-right gear: `EscapePanel.OpenPanel` (the pause menu).
    OpenEscapeMenu,
    /// The magnifier beside "Alive children": `BibiteStatsPanel.SelectRandomChild`.
    SelectRandomChild,
    /// The Stats panel's tag label: `BibiteStatsPanel.EditSpeciesTag` (type a species tag).
    EditSpeciesTag,
    /// The `+` of the dynamic panel's Default Bibites list (`DefaultBibitesPanel.
    /// AddBibiteButtonPressed`): opens the template selector.
    AddDefaultBibite,
    /// The list's "remove all" button (`DefaultBibitesPanel.DeleteAllBibites`).
    RequestClearDefaultBibites,
    /// The delete button of default bibite `.0` (`DefaultBibitesPanel.BibiteDeleted`).
    DeleteDefaultBibite(u16),
    /// The Zones tab's `TogglePreview` button: shows or hides the Preview panel.
    ToggleZonePreview,
    /// The `+` of the dynamic panel's Zones list (`ZonesEditorPanel.AddZone`): a new Plant zone
    /// with the game's defaults, named "Zone n", opened for editing.
    AddZone,
    /// The Zones list's "remove all" button (`ZonesEditorPanel.DeleteAllZones`): asks first.
    RequestClearZones,
    /// The delete button of zone `.0` (`ZoneSettingsHandle.DeleteZone`): asks first.
    DeleteZone(u16),
    /// The `+` of the Zone Groups list (`ZonesEditorPanel.AddZoneGroup`): a new group "Pockets<n>" with ten zones, its card open.
    AddZoneGroup,
    /// The Zone Groups list's "remove all" button: the game wires it to `ZonesEditorPanel.DeleteAllZoneGroups`, which does not exist, so
    /// pressing it does nothing there.
    ClearZoneGroups,
    /// The delete button of zone group `.0` (`ZoneGroupHandle.DeleteGroup`): the group and its zones go at once.
    DeleteZoneGroup(u16),
    /// The reroll button of zone group `.0` (`ZoneGroupHandle.GenerateZones`): its zones are generated again at new places.
    RerollZoneGroup(u16),
    /// RUSTibites Improved: the `+` of the Zones tab's Streams list: a new stream across the middle of the view, its card open.
    AddStream,
    /// The Streams list's "remove all" button: asks first.
    RequestClearStreams,
    /// The delete button of stream `.0`: the stream goes at once.
    DeleteStream(u16),
    /// The `+` of the Animation tab (`SettingsChangersEditor.AddNewChanger`): the creator panel opens.
    OpenAnimatorCreator,
    /// The creator's OK button (`SettingsChangerCreator.Submit`): the animator it describes is added and its card opens.
    SubmitAnimatorCreator,
    /// The creator's Cancel button.
    CloseAnimatorCreator,
    /// The Animation tab's "remove all" button (`SettingsChangersEditor.DeleteAllChangers`): asks first.
    RequestClearAnimators,
    /// The delete button of animator `.0` (`SettingsChangerHandle.DeleteChanger`): asks first.
    DeleteAnimator(u16),
    /// The cross of base `.1` of animator `.0` (`SettingBaseItem.Remove`): the animator lets go of that setting.
    RemoveAnimatorBase(u16, u16),
    /// The delete button of the point editor (`EditableLineGraph.RemoveSelectedPoint`): point `.1` of animator `.0`'s curve goes.
    RemoveAnimatorPoint(u16, u16),
    /// The tower panels' Remove button (`PheroTowerPanel.DestroyTarget` and the like): destroys the
    /// selected tower or colour killer.
    RemoveTool,
    /// The Placer's `LoadBibite` button (`BibitePlacer.ButtonFromTemplatePressed`): the template
    /// selector opens with the game paused, then bibites are placed with the left button.
    LoadBibite,
    /// The placer panel's close button (`CancelBibitePlacement`): leaves placement mode.
    ClosePlacer,
    /// The placer panel's custom tag field: type the tag the placed bibites get.
    EditPlacerTag,
    /// The pellet placer's close button (`CancelPelletPlacement`): leaves placement mode.
    ClosePellets,
    /// The Save Bibite dialog's Save button (`SaveController.SaveDialogConfirmed`).
    SaveDialogConfirm,
    /// The Save Bibite dialog's Cancel button (`ChoicePopupHandle.OnDismiss`).
    SaveDialogCancel,
    /// The species name in the Genes / Brain panels: `GenesPanel.OpenLineagePanelAndSelectSpecies`.
    OpenLineageOfSelected,
    /// The brain panel's Help button: `GameManager.brainHelp` opens a web page.
    OpenBrainHelp,
    /// The brain panel's Expanded View button (`GUIManager.OpenBibitePanel(ExpendedBrainPanel)`).
    OpenExpandedBrain,
    /// Confirm the pending dialog action.
    Confirm,
    /// Cancel the pending dialog action.
    Cancel,
    /// Tick / untick the "do not show again" checkbox of the pending action warning.
    ToggleDoNotShowAgain,
    /// The OK button of an error popup (`PopupManager.DisplayError`) in the world.
    DismissError,
    /// "Reset Warnings": the action warnings are shown again (`UserSettingsManager.ResetAllWarnings`).
    ResetWarnings,
    /// Save all selected/first eggs as entity files.
    SaveEggs,
    /// Abort every incubating egg.
    AbortEggs,
    /// Lay the selected bibite's completed eggs.
    LayEggs,
    /// Close the multi-selection panel (`UIPanel.ClosePanel`).
    SelectionClose,
    /// Destroy everything in the multi-selection (`SelectionPanel.KillSelection`).
    SelectionKill,
    /// Make every living selected bibite lay an egg (`SelectionPanel.LayEggs`).
    SelectionLay,
    /// Take a kind of entity out of the multi-selection (index into `selection::Kind::ALL`).
    SelectionRemoveKind(u8),
    /// Take a species' bibites out of the multi-selection (raw `SpeciesId`).
    SelectionRemoveSpecies(u32),
    /// Revive the selected bibite.
    ReviveSelected,
    /// Save the selected bibite entity.
    SaveSelected,
    /// The Save button of the Save Game panel (`SaveGamePanel.AskOverwritePreviousSave`).
    SaveAs,
    /// Load the first row of the save browser.
    LoadSelectedSave,
    /// Delete the first row of the save browser.
    DeleteSave,
    /// Export the selected bibite as a template.
    EditSelected,
    /// Apply the edited species tag to the selection.
    SetTag,
    /// Clear the species tag on the selection.
    ClearTag,
    /// Select a tool.
    SelectTool(ToolKind),
    /// Open the bibite library.
    OpenLibrary,
    /// Open achievements.
    OpenAchievements,
    /// Open additional tools.
    OpenTools,
    /// Open credits.
    OpenCredits,
    /// Open the lineage graph.
    OpenLineage,
    /// Open the history graphs panel.
    OpenGraphs,
    /// Open the information panel.
    OpenInformation,
    /// `BibiteStatsPanel.SelectParent`: the first (`true`, the egg layer) or the second parent becomes the target.
    SelectParent(bool),
    /// The information panel's close button (`InformationPanel.ClosePanel`).
    InformationClose,
    /// The information panel's `count` / `energy` button.
    InformationToggleMode,
    /// A row of the information panel's top species (`SpeciesInfoHandle.OnClick`).
    InformationSelectSpecies(u8),
    /// A row of the information panel's tags (`TagElementHandle.ClickOnTag`), by its place in the list.
    InformationSelectTag(u32),
    /// Show a finer history scale in the graphs panel.
    GraphsScaleDown,
    /// Show a coarser history scale in the graphs panel.
    GraphsScaleUp,
    /// Open the species browser.
    OpenSpecies,
    /// Open the in-game menu.
    OpenMenu,
    /// `EscapePanel` "Back to Menu": leave the world for the title menu.
    ReturnToMenu,
    /// `EscapePanel` "Wiki": open the wiki in the browser.
    OpenWiki,
    /// Quit the viewer.
    Quit,
}

/// World-editing tools.
#[allow(dead_code)] // TODO(bind): tool palette lands with world-IO bindings
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    /// Spawn pellets.
    Pellet,
    /// Place a radio (radiation) tower (`WorldObjectsSpawner.GenerateRadioTower`).
    Tower,
    /// Place a pheromone tower (`WorldObjectsSpawner.GeneratePheromoneTower`).
    PheroTower,
    /// Place a colour killer (`WorldObjectsSpawner.GenerateColorSelector`).
    Killer,
    /// RUSTibites Improved: place a gene selector tower (`gene_selector.rs`).
    GeneSelector,
    /// RUSTibites Improved: place a hydrothermal vent tower (`vent.rs`).
    Vent,
    /// RUSTibites Improved: place a force tower (`force_tower.rs`).
    Force,
    /// RUSTibites Improved: place a whirlpool tower (`whirlpool.rs`).
    Whirl,
    /// Inspect entities.
    Inspect,
}

/// Lineage graph views.
#[allow(dead_code)] // TODO(bind): lineage graph view lands with inspector expansion
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineageView {
    /// Whole-population tree.
    Population,
    /// Ancestors of the selected bibite.
    Ancestors,
    /// Descendants of the selected bibite.
    Descendants,
}

/// Scene overlays toggled from the HUD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// Background grid.
    Grid,
    /// Zone fills.
    Zones,
    /// RUSTibites Improved: each zone in a colour of its own (the toolbar's View panel).
    ZoneColors,
    /// Pellets.
    Pellets,
    /// Pheromone spots.
    Pheromones,
    /// HUD visibility.
    Hud,
    /// World-space scale bar (`RulerHandle`).
    Ruler,
    /// The health / energy bars over the selected bibite (`UserSettings.ShowStatusBars`).
    StatusBars,
    /// The selected bibite's field of view (`UserSettings.ShowBibiteFOW`).
    FieldOfView,
    /// The target weights of the selected bibite (`UserSettings.ShowBibiteTargets`).
    Targets,
}

/// Inspector tabs (P0/P0b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InspectorTab {
    /// Core stats (default).
    #[default]
    Stats,
    /// Genome summary.
    Genes,
    /// Organs/energy flow.
    Biology,
    /// Brain overview.
    Brain,
}

impl InspectorTab {
    /// All tabs in display order.
    pub const ALL: [Self; 4] = [Self::Stats, Self::Genes, Self::Biology, Self::Brain];

    /// Short label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Stats => "stats",
            Self::Genes => "genes",
            Self::Biology => "biology",
            Self::Brain => "brain",
        }
    }
}

/// Pointer interaction state (active widget capture).
#[derive(Clone, Copy, Debug, Default)]
pub struct UiInteraction {
    /// Widget that received the press.
    pub active: Option<UiId>,
}

/// A frame's worth of screen-space UI draws (logical pixels).
#[derive(Clone, Debug, Default)]
pub struct UiFrame {
    /// `UIScaler` factor applied to all sizes (`min(h/1080, w/1920)`).
    pub scale: f32,
    /// Draw list, back to front.
    pub draws: Vec<UiDraw>,
    /// Interactive hit rectangles `(id, pos, size)`.
    pub hits: Vec<(UiId, Vec2, Vec2)>,
    /// Action bound to each interactive id.
    pub actions: Vec<(UiId, UiAction)>,
    /// Hover rectangles of the captured controls that have a `TooltipTrigger`, back to front.
    pub tips: Vec<crate::tooltip::TipRect>,
}

/// One screen-space UI primitive.
#[derive(Clone, Debug)]
pub enum UiDraw {
    /// A clip rectangle (`RectMask2D`): from here on only what falls inside `[x, y, w, h]` (screen pixels, top-left origin) is drawn; `None` lifts it.
    Clip(Option<[f32; 4]>),
    /// Filled rectangle.
    Rect {
        /// Top-left position.
        pos: Vec2,
        /// Size.
        size: Vec2,
        /// Fill colour.
        color: Rgba,
    },
    /// Image from raw pixels (scenario preview).
    Pixels {
        /// Decoded image.
        texture: std::sync::Arc<UiTexture>,
        /// Top-left position.
        pos: Vec2,
        /// Destination size.
        size: Vec2,
    },
    /// A part of a raw-pixel image (`region`: `x, y, w, h` in its pixels) stretched over a rectangle: a
    /// picture cut by a clip rectangle (the renderers have none).
    PixelsRegion {
        /// Decoded image.
        texture: std::sync::Arc<UiTexture>,
        /// The part of the image.
        region: [f32; 4],
        /// Top-left position.
        pos: Vec2,
        /// Destination size.
        size: Vec2,
    },
    /// A part of a UI texture (`region`: `x, y, w, h` in texture pixels) stretched over a rectangle: a sprite cut
    /// by a clip rectangle (the renderers have none).
    ImageRegion {
        /// Texture slot.
        texture: u16,
        /// The part of the texture.
        region: [f32; 4],
        /// Top-left position.
        pos: Vec2,
        /// Destination size.
        size: Vec2,
        /// Tint.
        tint: Rgba,
    },
    /// Nine-slice frame from a UI texture.
    NineSlice {
        /// Texture slot.
        texture: u16,
        /// Unity border `[left, bottom, right, top]` in texture pixels.
        border: [f32; 4],
        /// Destination border size relative to the source border
        /// (`Image.pixelsPerUnitMultiplier`; 1.0 unless the sprite is drawn
        /// with a larger pixels-per-unit than its import setting).
        dest_scale: f32,
        /// Top-left position.
        pos: Vec2,
        /// Destination size.
        size: Vec2,
        /// Tint.
        tint: Rgba,
    },
    /// Stretched sprite/icon.
    Image {
        /// Texture slot.
        texture: u16,
        /// Top-left position.
        pos: Vec2,
        /// Destination size.
        size: Vec2,
        /// Tint.
        tint: Rgba,
    },
    /// A sprite on a node whose transform is turned by a multiple of 90 degrees about z (a foldout's arrow pointing down, the Library's badges): the
    /// image is stretched over the node's rectangle and then turned `turns` quarter turns counter-clockwise; `pos` and `size` are the turned
    /// picture's box.
    ImageTurned {
        /// Texture slot.
        texture: u16,
        /// Quarter turns counter-clockwise (1..=3).
        turns: u8,
        /// Top-left position of the box.
        pos: Vec2,
        /// Size of the box.
        size: Vec2,
        /// Tint.
        tint: Rgba,
    },
    /// Text run.
    Text {
        /// Captured TMP font asset name (`None` uses the role fallback).
        font: Option<String>,
        /// UTF-8 text.
        text: String,
        /// Top-left position.
        pos: Vec2,
        /// Font size in logical pixels.
        px: f32,
        /// Tint.
        color: Rgba,
        /// Font role.
        role: FontRole,
    },
}

impl UiFrame {
    /// Creates an empty frame with the given `UIScaler`.
    #[must_use]
    pub fn new(scale: f32) -> Self {
        Self {
            scale,
            draws: Vec::new(),
            hits: Vec::new(),
            actions: Vec::new(),
            tips: Vec::new(),
        }
    }

    /// Registers an interactive rectangle (drawn separately).
    pub fn hit(&mut self, id: UiId, pos: Vec2, size: Vec2, action: UiAction) {
        self.hits.push((id, pos, size));
        self.actions.push((id, action));
    }

    /// Moves what was added to the frame after it held `draws` draws and `hits` click areas by `(dx, dy)` screen pixels: a panel laid out at its place in
    /// the 1920 x 1080 canvas and then put where the canvas it is on has it (the Zones tab's Preview hangs left of the settings panel, which is flush
    /// with the right edge of the canvas).
    pub fn translate_from(&mut self, draws: usize, hits: usize, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let by = Vec2::new(dx, dy);
        for draw in self.draws.iter_mut().skip(draws) {
            match draw {
                UiDraw::Clip(Some(rect)) => {
                    rect[0] += dx;
                    rect[1] += dy;
                }
                UiDraw::Clip(None) => {}
                UiDraw::Rect { pos, .. }
                | UiDraw::Pixels { pos, .. }
                | UiDraw::PixelsRegion { pos, .. }
                | UiDraw::ImageRegion { pos, .. }
                | UiDraw::NineSlice { pos, .. }
                | UiDraw::Image { pos, .. }
                | UiDraw::ImageTurned { pos, .. }
                | UiDraw::Text { pos, .. } => *pos += by,
            }
        }
        for hit in self.hits.iter_mut().skip(hits) {
            hit.1 += by;
        }
    }

    /// The ids that more than one click area of the frame carries, in order. Two panels that are on screen together must have none in common (see
    /// [`UiId`]); the tests of the panels that hang over one another assert this is empty.
    #[cfg(test)]
    #[must_use]
    pub fn shared_ids(&self) -> Vec<UiId> {
        let mut seen = std::collections::HashSet::new();
        let mut shared: Vec<UiId> = self
            .hits
            .iter()
            .map(|hit| hit.0)
            .filter(|id| !seen.insert(*id))
            .collect();
        shared.sort_unstable();
        shared.dedup();
        shared
    }

    /// `true` when `pointer` is over any interactive rectangle.
    #[must_use]
    #[allow(dead_code)] // window-loop wiring lands next
    pub fn over(&self, pointer: Vec2) -> bool {
        self.hits.iter().any(|(_, pos, size)| {
            pointer.x >= pos.x
                && pointer.y >= pos.y
                && pointer.x < pos.x + size.x
                && pointer.y < pos.y + size.y
        })
    }

    /// The action of the topmost widget under the pointer.
    #[must_use]
    pub fn action_at(&self, pointer: Vec2) -> Option<UiAction> {
        let id = self.widget_at(pointer)?;
        self.actions
            .iter()
            .find(|(action_id, _)| *action_id == id)
            .map(|(_, action)| *action)
    }

    /// Topmost widget under the pointer.
    #[must_use]
    pub fn widget_at(&self, pointer: Vec2) -> Option<UiId> {
        self.hits.iter().rev().find_map(|(id, pos, size)| {
            (pointer.x >= pos.x
                && pointer.y >= pos.y
                && pointer.x < pos.x + size.x
                && pointer.y < pos.y + size.y)
                .then_some(*id)
        })
    }

    /// A button held down shows its `ColorBlock.pressedColor` (200 / 255 of white in the game's `ColorTint` buttons): the frame of widget `id` is drawn
    /// darker for as long as the pointer is held on it (it goes back to normal when the pointer leaves it). Only the images that fill the widget's whole
    /// rectangle are the button's own graphic; a child's icon or text keeps its colour.
    pub fn tint_pressed(&mut self, id: UiId, pointer: Vec2) {
        const PRESSED: f32 = 200.0 / 255.0;
        let Some(&(_, pos, size)) = self.hits.iter().rev().find(|(hit, ..)| *hit == id) else {
            return;
        };
        if self.widget_at(pointer) != Some(id) {
            return;
        }
        let near = |a: Vec2, b: Vec2| (a.x - b.x).abs() < 0.75 && (a.y - b.y).abs() < 0.75;
        for draw in &mut self.draws {
            if let UiDraw::NineSlice { pos: p, size: s, tint, .. } | UiDraw::Image { pos: p, size: s, tint, .. } = draw
                && near(*p, pos)
                && near(*s, size)
            {
                for channel in &mut tint[..3] {
                    *channel = (f32::from(*channel) * PRESSED).round() as u8;
                }
            }
        }
    }

    /// Routes a press/release pair to a widget action (UI-first input).
    pub fn interact(
        &self,
        pointer: Vec2,
        pressed: bool,
        released: bool,
        state: &mut UiInteraction,
    ) -> Vec<UiAction> {
        if pressed {
            state.active = self.widget_at(pointer);
        }
        if released {
            let active = state.active.take();
            if let Some(id) = active {
                if self.widget_at(pointer) == Some(id) {
                    return self
                        .actions
                        .iter()
                        .filter(|(action_id, _)| *action_id == id)
                        .map(|(_, action)| *action)
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// `UIScaler.totalScale` (`CanvasScaler.scaleFactor`): `ScreenResolutionFactor * UIScale`. With "Automatically Detect Screen Resolution" (the
    /// default) the first is `min(height / 1080, width / 1920)` of the window, else the setting's own value.
    #[must_use]
    pub fn scaler(width: f32, height: f32) -> f32 {
        let (automatic, factor, scale) = ui_scale_settings();
        let resolution = if automatic { (height / 1080.0).min(width / 1920.0).max(0.5) } else { factor };
        let total = resolution * scale;
        if width >= 100.0 && height >= 100.0 {
            LAST_CANVAS.with(|cell| cell.set(Some(Vec2::new(width / total, height / total))));
        }
        total
    }

    /// Adds a filled rectangle.
    pub fn rect(&mut self, pos: Vec2, size: Vec2, color: Rgba) {
        self.draws.push(UiDraw::Rect { pos, size, color });
    }

    /// Adds text.
    pub fn text(&mut self, text: impl Into<String>, pos: Vec2, px: f32, color: Rgba) {
        self.draws.push(UiDraw::Text {
            text: text.into(),
            pos,
            px,
            color,
            role: FontRole::Body,
            font: None,
        });
    }

    /// Adds bold text.
    pub fn bold(&mut self, text: impl Into<String>, pos: Vec2, px: f32, color: Rgba) {
        self.draws.push(UiDraw::Text {
            text: text.into(),
            pos,
            px,
            color,
            role: FontRole::Bold,
            font: None,
        });
    }

    /// Adds monospace text.
    pub fn mono(&mut self, text: impl Into<String>, pos: Vec2, px: f32, color: Rgba) {
        self.draws.push(UiDraw::Text {
            text: text.into(),
            pos,
            px,
            color,
            role: FontRole::Mono,
            font: None,
        });
    }

    /// Adds a nine-slice panel from the first available candidate record,
    /// falling back to a translucent rectangle when no record resolves.
    pub fn panel(&mut self, ui: &UiAssets, candidates: &[&str], pos: Vec2, size: Vec2) {
        match ui.pick(candidates) {
            Some(record) => {
                let tint = multiply(
                    record.default_tint.unwrap_or([255, 255, 255, 255]),
                    [58, 60, 70, 238],
                );
                self.nine_slice_tinted(ui, record, pos, size, tint);
            }
            None => self.rect(pos, size, [24, 26, 32, 225]),
        }
    }

    /// Adds a nine-slice record with a custom tint.
    pub fn nine_slice_tinted(
        &mut self,
        ui: &UiAssets,
        record: &UiRecord,
        pos: Vec2,
        size: Vec2,
        tint: Rgba,
    ) {
        let Some(texture) = ui.texture_of(&record.logical) else {
            self.rect(pos, size, tint);
            return;
        };
        self.draws.push(UiDraw::NineSlice {
            texture,
            border: record.border.unwrap_or([0.0; 4]),
            dest_scale: nine_slice_dest_scale(&record.logical),
            pos,
            size,
            tint,
        });
    }

    /// Adds a nine-slice record.
    #[allow(dead_code)] // window-loop wiring lands next
    pub fn nine_slice(&mut self, ui: &UiAssets, record: &UiRecord, pos: Vec2, size: Vec2) {
        let Some(texture) = ui.texture_of(&record.logical) else {
            self.rect(pos, size, record.default_tint.unwrap_or([0, 0, 0, 185]));
            return;
        };
        self.draws.push(UiDraw::NineSlice {
            texture,
            border: record.border.unwrap_or([0.0; 4]),
            dest_scale: nine_slice_dest_scale(&record.logical),
            pos,
            size,
            tint: record.default_tint.unwrap_or([255, 255, 255, 255]),
        });
    }

    /// Adds an icon/texture at `pos`/`size` (first available candidate).
    pub fn image(&mut self, ui: &UiAssets, candidates: &[&str], pos: Vec2, size: Vec2) -> bool {
        let Some(record) = ui.pick(candidates) else {
            return false;
        };
        let Some(texture) = ui.texture_of(&record.logical) else {
            return false;
        };
        self.draws.push(UiDraw::Image {
            texture,
            pos,
            size,
            tint: record.default_tint.unwrap_or([255, 255, 255, 255]),
        });
        true
    }
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

/// One rasterised glyph.
#[derive(Clone, Debug)]
pub struct Glyph {
    /// fontdue metrics.
    pub metrics: fontdue::Metrics,
    /// Alpha coverage bitmap (`width * height`).
    pub coverage: Vec<u8>,
}

/// TTF font set with an on-demand glyph cache.
/// Which text rasteriser the UI uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextMode {
    /// SDF when the game font assets exist, otherwise TTF.
    Auto,
    /// Force the fontdue TTF fallback.
    Ttf,
    /// Force TextMeshPro SDF (falls back to TTF when assets are missing).
    Sdf,
}

pub struct FontSet {
    body: Font,
    bold: Font,
    mono: Font,
    cache: HashMap<(FontRole, char, u32), Glyph>,
    sdf: Option<std::sync::Arc<crate::text_sdf::SdfFonts>>,
    text_mode: TextMode,
}

impl std::fmt::Debug for FontSet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FontSet")
            .field("cached_glyphs", &self.cache.len())
            .finish()
    }
}

impl FontSet {
    /// Loads the three UI font roles from `assets/game/fonts/`.
    pub fn load(ui: &UiAssets) -> Result<Self, String> {
        let load = |role: FontRole| -> Result<Font, String> {
            let path = ui.font_path(role);
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("could not read font {}: {error}", path.display()))?;
            Font::from_bytes(bytes, FontSettings::default())
                .map_err(|error| format!("invalid TTF {}: {error}", path.display()))
        };
        let sdf = crate::text_sdf::SdfFonts::discover(&ui.root).map(std::sync::Arc::new);
        if let Some(sdf) = sdf.as_ref() {
            crate::text_sdf::share(sdf);
        }
        Ok(Self {
            body: load(FontRole::Body)?,
            bold: load(FontRole::Bold)?,
            mono: load(FontRole::Mono)?,
            cache: HashMap::new(),
            sdf,
            text_mode: TextMode::Auto,
        })
    }

    /// Selects the text rasteriser (SDF/TTF/auto).
    pub fn set_text_mode(&mut self, mode: TextMode) {
        self.text_mode = mode;
    }

    /// Active SDF font set, when enabled and available.
    #[must_use]
    pub fn sdf(&self) -> Option<&crate::text_sdf::SdfFonts> {
        if self.text_mode == TextMode::Ttf {
            return None;
        }
        self.sdf.as_deref()
    }

    /// The SDF fonts (shared), when active: the window's GPU path draws text with them too.
    #[must_use]
    pub fn sdf_handle(&self) -> Option<std::sync::Arc<crate::text_sdf::SdfFonts>> {
        if self.text_mode == TextMode::Ttf {
            return None;
        }
        self.sdf.clone()
    }

    /// Whether SDF text is active.
    #[must_use]
    pub fn using_sdf(&self) -> bool {
        self.sdf().is_some()
    }

    #[allow(dead_code)]
    fn font(&self, role: FontRole) -> &Font {
        match role {
            FontRole::Body => &self.body,
            FontRole::Bold => &self.bold,
            FontRole::Mono => &self.mono,
        }
    }

    /// Rasterises (and caches) one glyph at a pixel size.
    pub fn glyph(&mut self, role: FontRole, character: char, px: f32) -> &Glyph {
        let key = (role, character, px.round().max(1.0) as u32);
        if !self.cache.contains_key(&key) {
            let font = match role {
                FontRole::Body => &self.body,
                FontRole::Bold => &self.bold,
                FontRole::Mono => &self.mono,
            };
            let (metrics, coverage) = font.rasterize(character, key.2 as f32);
            self.cache.insert(key, Glyph { metrics, coverage });
        }
        self.cache.get(&key).expect("glyph inserted above")
    }

    /// Advances width of a text run at `px` (no kerning; readable labels only).
    #[allow(dead_code)]
    pub fn measure(&mut self, role: FontRole, text: &str, px: f32) -> f32 {
        let mut width = 0.0;
        for character in text.chars() {
            let glyph = self.glyph(role, character, px);
            width += glyph.metrics.advance_width;
        }
        width
    }

    /// Ascent used to convert a top-left text origin to a baseline.
    pub fn ascent(&mut self, role: FontRole, px: f32) -> f32 {
        let glyph = self.glyph(role, 'M', px);
        glyph.metrics.height as f32 + glyph.metrics.ymin as f32
    }
}

// ---------------------------------------------------------------------------
// PNG decoding
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Window backend (macroquad)
// ---------------------------------------------------------------------------

/// GPU-side UI resources for the interactive window.
pub struct UiGpu {
    textures: Vec<macroquad::prelude::Texture2D>,
    body: macroquad::text::Font,
    bold: macroquad::text::Font,
    mono: macroquad::text::Font,
    /// The raw-pixel pictures (bibite previews) uploaded so far, by the address of their image.
    pixels: std::cell::RefCell<PixelTextures>,
    /// The game's SDF fonts: text is drawn by the software renderer's own routine (cached as textures), so the window's text is the headless
    /// picture's text (TMP's faces, kerning, synthetic bold, italic and tracking), not a TTF stand-in at the nearest whole size.
    sdf: Option<std::sync::Arc<crate::text_sdf::SdfFonts>>,
    /// The text runs rasterised so far.
    texts: std::cell::RefCell<TextTextures>,
}

/// A text run's identity: what the rasteriser reads, and the quarter-pixel phase it starts at.
#[derive(Clone, PartialEq, Eq, Hash)]
struct TextKey {
    text: String,
    font: Option<String>,
    role: u8,
    px: u32,
    color: [u8; 4],
    phase: [u8; 2],
}

/// The texture of a rasterised run (`None`: nothing was drawn), where it starts relative to the run's whole-pixel origin, and the last call it was used.
type TextEntry = (Option<(macroquad::prelude::Texture2D, [i32; 2])>, u64);

/// Text runs on the GPU: rasterised the first time they are drawn at a phase, dropped once unused for a while.
#[derive(Default)]
struct TextTextures {
    tick: u64,
    map: std::collections::HashMap<TextKey, TextEntry>,
}

impl TextTextures {
    /// Calls a run may go unused before its texture is dropped (a changing number leaves one behind per value).
    const KEEP_CALLS: u64 = 240;
    /// Pixels kept around the run so the rasteriser's writes left of or above its origin land inside the canvas.
    const PAD: i32 = 6;
    /// Phases per pixel the origin is quantised to.
    const PHASES: f32 = 8.0;

    fn end_frame(&mut self) {
        let tick = self.tick;
        self.map.retain(|_, entry| entry.1 + Self::KEEP_CALLS >= tick);
        self.tick += 1;
    }

    /// Draws `text` as the software renderer would at `pos` (logical pixels). On a display scaled by the system (125 %, 150 %: `dpi` physical pixels to a logical
    /// one) the run is rasterised at the PHYSICAL pixel size and put on whole physical pixels: drawn at the logical size it was stretched by the scale with
    /// nearest sampling, which blurred and roughened every glyph.
    #[allow(clippy::too_many_arguments)] // the run's inputs stay explicit
    fn draw(
        &mut self,
        sdf: &crate::text_sdf::SdfFonts,
        text: &str,
        font: Option<&str>,
        role: FontRole,
        pos: Vec2,
        px: f32,
        color: Rgba,
    ) {
        use macroquad::prelude::{DrawTextureParams, FilterMode, Texture2D, WHITE, draw_texture_ex, screen_dpi_scale, vec2};
        let dpi = screen_dpi_scale().max(1.0);
        let (pos, px) = (pos * dpi, px * dpi);
        let quantise = |value: f32| -> (i64, u8) {
            let steps = (value * Self::PHASES).round() as i64;
            (steps.div_euclid(Self::PHASES as i64), steps.rem_euclid(Self::PHASES as i64) as u8)
        };
        let ((ix, phase_x), (iy, phase_y)) = (quantise(pos.x), quantise(pos.y));
        let key = TextKey {
            text: text.to_string(),
            font: font.map(str::to_string),
            role: match role {
                FontRole::Body => 0,
                FontRole::Bold => 1,
                FontRole::Mono => 2,
            },
            px: px.to_bits(),
            color,
            phase: [phase_x, phase_y],
        };
        let tick = self.tick;
        let entry = self.map.entry(key).or_insert_with(|| {
            let (_, bold, italic, spacing) = font.map_or(("", false, false, 0.0), crate::text_sdf::split_style_full);
            let face = font.and_then(|name| sdf.font_named(name)).unwrap_or_else(|| sdf.font(role));
            let origin = (
                f32::from(phase_x) / Self::PHASES + Self::PAD as f32,
                f32::from(phase_y) / Self::PHASES + Self::PAD as f32,
            );
            let mut writes: Vec<(i64, i64, Rgba)> = Vec::new();
            face.draw_line_styled(text, origin, px, color, (bold, spacing), italic, |x, y, rgba| writes.push((x, y, rgba)));
            let bounds = writes.iter().fold(None, |bounds: Option<[i64; 4]>, &(x, y, _)| {
                Some(bounds.map_or([x, y, x, y], |b| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]))
            });
            let texture = bounds.and_then(|[x0, y0, x1, y1]| {
                let (w, h) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
                if w == 0 || h == 0 || w > usize::from(u16::MAX) || h > usize::from(u16::MAX) {
                    return None;
                }
                // The software renderer's own blend (straight alpha, over), onto a clear canvas.
                let mut pixels = vec![[0u8; 4]; w * h];
                for (x, y, src) in writes {
                    let dst = &mut pixels[(y - y0) as usize * w + (x - x0) as usize];
                    let src_alpha = u32::from(src[3]);
                    if src_alpha == 0 {
                        continue;
                    }
                    let dst_alpha = u32::from(dst[3]);
                    if dst_alpha == 0 || (src_alpha >= 255 && dst_alpha >= 255) {
                        *dst = src;
                        continue;
                    }
                    let inverse = 255 - src_alpha;
                    let out_alpha = src_alpha + dst_alpha * inverse / 255;
                    if out_alpha == 0 {
                        *dst = [0; 4];
                        continue;
                    }
                    for channel in 0..3 {
                        let numerator = u32::from(src[channel]) * src_alpha * 255 + u32::from(dst[channel]) * dst_alpha * inverse;
                        dst[channel] = (numerator / (255 * out_alpha)).min(255) as u8;
                    }
                    dst[3] = out_alpha as u8;
                }
                let bytes: Vec<u8> = pixels.into_iter().flatten().collect();
                let texture = Texture2D::from_rgba8(w as u16, h as u16, &bytes);
                texture.set_filter(FilterMode::Nearest);
                Some((texture, [x0 as i32 - Self::PAD, y0 as i32 - Self::PAD]))
            });
            (texture, tick)
        });
        entry.1 = tick;
        if let Some((texture, offset)) = entry.0.as_ref() {
            draw_texture_ex(
                texture,
                (ix as i32 + offset[0]) as f32 / dpi,
                (iy as i32 + offset[1]) as f32 / dpi,
                WHITE,
                DrawTextureParams {
                    dest_size: Some(vec2(texture.width() / dpi, texture.height() / dpi)),
                    ..Default::default()
                },
            );
        }
    }
}

/// Raw-pixel pictures on the GPU: made the first frame they are drawn, dropped once unused for a while.
#[derive(Default)]
struct PixelTextures {
    /// Frames drawn so far.
    tick: u64,
    /// The pictures: the image kept alive (so its address stays its own), its texture, the last frame it was drawn.
    map: std::collections::HashMap<
        usize,
        (
            std::sync::Arc<UiTexture>,
            macroquad::prelude::Texture2D,
            u64,
        ),
    >,
}

impl PixelTextures {
    /// How many frames a picture may go undrawn before its texture is dropped. A picture that is drawn again later uploads again (a
    /// few milliseconds); pictures that change every frame (a brain diagram being dragged, the Expanded Brain View's synapses while
    /// its simulation runs) leave one texture behind each frame, so the pile has to stay small.
    const KEEP_FRAMES: u64 = 6;

    /// The texture of `image`, uploaded on first use.
    fn texture_of(&mut self, image: &std::sync::Arc<UiTexture>) -> macroquad::prelude::Texture2D {
        let key = std::sync::Arc::as_ptr(image) as usize;
        let tick = self.tick;
        let entry = self.map.entry(key).or_insert_with(|| {
            let texture = macroquad::prelude::Texture2D::from_rgba8(
                image.width.min(u32::from(u16::MAX)) as u16,
                image.height.min(u32::from(u16::MAX)) as u16,
                &image.rgba,
            );
            texture.set_filter(macroquad::prelude::FilterMode::Nearest);
            (image.clone(), texture, tick)
        });
        entry.2 = tick;
        entry.1.clone()
    }

    /// Ends a frame: pictures nobody drew lately go.
    fn end_frame(&mut self) {
        let tick = self.tick;
        self.map
            .retain(|_, entry| entry.2 + Self::KEEP_FRAMES >= tick);
        self.tick += 1;
    }
}

/// Uploads UI textures and fonts for the window renderer.
///
/// Must run with a live macroquad context (inside the window loop).
pub fn upload_macroquad(ui: &UiAssets, sdf: Option<std::sync::Arc<crate::text_sdf::SdfFonts>>) -> Result<UiGpu, String> {
    let mut textures = Vec::with_capacity(ui.textures.len());
    for texture in &ui.textures {
        let image = macroquad::prelude::Texture2D::from_rgba8(
            texture.width as u16,
            texture.height as u16,
            &texture.rgba,
        );
        image.set_filter(macroquad::prelude::FilterMode::Linear);
        textures.push(image);
    }
    let load = |role: FontRole| -> Result<macroquad::text::Font, String> {
        let path = ui.font_path(role);
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("could not read font {}: {error}", path.display()))?;
        macroquad::text::load_ttf_font_from_bytes(&bytes)
            .map_err(|error| format!("invalid TTF {}: {error}", path.display()))
    };
    Ok(UiGpu {
        textures,
        body: load(FontRole::Body)?,
        bold: load(FontRole::Bold)?,
        mono: load(FontRole::Mono)?,
        pixels: std::cell::RefCell::new(PixelTextures::default()),
        sdf,
        texts: std::cell::RefCell::new(TextTextures::default()),
    })
}

/// Draws a UI frame with macroquad (window mode).
pub fn draw_frame_macroquad(frame: &UiFrame, gpu: &UiGpu) {
    use macroquad::prelude::*;

    for draw in &frame.draws {
        match draw {
            UiDraw::Clip(rect) => match rect {
                // A camera whose viewport is the rectangle and which maps the screen's pixels to themselves: what lies outside the viewport is cut
                // by the clip volume (the crate forbids `unsafe`, which a scissor box would need).
                Some(r) => {
                    let dpi = screen_dpi_scale();
                    let mut camera = Camera2D::from_display_rect(Rect::new(r[0], r[1], r[2], r[3]));
                    // The real window showed the clipped rows upside down: on the screen (no render target) this camera's y runs the other way.
                    camera.zoom.y = -camera.zoom.y;
                    camera.viewport = Some((
                        (r[0] * dpi) as i32,
                        ((screen_height() - (r[1] + r[3])) * dpi) as i32,
                        (r[2] * dpi) as i32,
                        (r[3] * dpi) as i32,
                    ));
                    set_camera(&camera);
                }
                None => set_default_camera(),
            },
            UiDraw::Rect { pos, size, color } => {
                draw_rectangle(pos.x, pos.y, size.x, size.y, color_rgba(*color));
            }
            UiDraw::Image {
                texture,
                pos,
                size,
                tint,
            } => {
                let Some(texture) = gpu.textures.get(*texture as usize) else {
                    continue;
                };
                draw_texture_ex(
                    texture,
                    pos.x,
                    pos.y,
                    color_rgba(*tint),
                    DrawTextureParams {
                        dest_size: Some(glam::Vec2::new(size.x, size.y)),
                        ..Default::default()
                    },
                );
            }
            UiDraw::ImageTurned {
                texture,
                turns,
                pos,
                size,
                tint,
            } => {
                let Some(texture) = gpu.textures.get(*texture as usize) else {
                    continue;
                };
                // The quad is the node's rectangle (the box with its sides swapped for an odd number of turns), turned about the box's centre
                // (macroquad turns clockwise on the screen).
                let centre = glam::Vec2::new(pos.x + size.x * 0.5, pos.y + size.y * 0.5);
                let quad = if turns % 2 == 1 {
                    glam::Vec2::new(size.y, size.x)
                } else {
                    glam::Vec2::new(size.x, size.y)
                };
                draw_texture_ex(
                    texture,
                    centre.x - quad.x * 0.5,
                    centre.y - quad.y * 0.5,
                    color_rgba(*tint),
                    DrawTextureParams {
                        dest_size: Some(quad),
                        rotation: -f32::from(*turns) * std::f32::consts::FRAC_PI_2,
                        pivot: Some(centre),
                        ..Default::default()
                    },
                );
            }
            UiDraw::ImageRegion {
                texture,
                region,
                pos,
                size,
                tint,
            } => {
                let Some(texture) = gpu.textures.get(*texture as usize) else {
                    continue;
                };
                draw_texture_ex(
                    texture,
                    pos.x,
                    pos.y,
                    color_rgba(*tint),
                    DrawTextureParams {
                        dest_size: Some(glam::Vec2::new(size.x, size.y)),
                        source: Some(macroquad::math::Rect::new(
                            region[0], region[1], region[2], region[3],
                        )),
                        ..Default::default()
                    },
                );
            }
            UiDraw::Pixels { texture, pos, size } => {
                let handle = gpu.pixels.borrow_mut().texture_of(texture);
                draw_texture_ex(
                    &handle,
                    pos.x,
                    pos.y,
                    WHITE,
                    DrawTextureParams {
                        dest_size: Some(glam::Vec2::new(size.x, size.y)),
                        ..Default::default()
                    },
                );
            }
            UiDraw::PixelsRegion {
                texture,
                region,
                pos,
                size,
            } => {
                let handle = gpu.pixels.borrow_mut().texture_of(texture);
                draw_texture_ex(
                    &handle,
                    pos.x,
                    pos.y,
                    WHITE,
                    DrawTextureParams {
                        dest_size: Some(glam::Vec2::new(size.x, size.y)),
                        source: Some(Rect::new(region[0], region[1], region[2], region[3])),
                        ..Default::default()
                    },
                );
            }
            UiDraw::NineSlice {
                texture,
                border,
                dest_scale,
                pos,
                size,
                tint,
            } => {
                let Some(texture) = gpu.textures.get(*texture as usize) else {
                    continue;
                };
                draw_nine_slice(
                    texture,
                    *border,
                    *pos,
                    *size,
                    *tint,
                    frame.scale * *dest_scale,
                );
            }
            UiDraw::Text {
                text,
                pos,
                px,
                color,
                role,
                font,
            } => {
                if let Some(sdf) = gpu.sdf.as_deref() {
                    gpu.texts.borrow_mut().draw(sdf, text, font.as_deref(), *role, *pos, *px, *color);
                    continue;
                }
                let font = match role {
                    FontRole::Body => &gpu.body,
                    FontRole::Bold => &gpu.bold,
                    FontRole::Mono => &gpu.mono,
                };
                draw_text_ex(
                    text,
                    pos.x,
                    pos.y + px * 0.86,
                    TextParams {
                        font: Some(font),
                        font_size: px.round().max(6.0) as u16,
                        font_scale: 1.0,
                        color: color_rgba(*color),
                        ..Default::default()
                    },
                );
            }
        }
    }
    gpu.pixels.borrow_mut().end_frame();
    gpu.texts.borrow_mut().end_frame();
}

fn color_rgba(color: Rgba) -> macroquad::prelude::Color {
    macroquad::prelude::Color::new(
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    )
}

/// Destination border scale for sliced sprites the game draws with a larger
/// pixels-per-unit than the sprite's import setting. The capture does not record
/// `Image.pixelsPerUnitMultiplier`, so the factor is measured from the game's
/// own UI render: `Button_UI` (30 px border, 20 px dark frame) shows a 4 px frame.
pub(crate) fn nine_slice_dest_scale(sprite_name: &str) -> f32 {
    if sprite_name.eq_ignore_ascii_case("Button_UI") {
        0.2
    } else {
        1.0
    }
}

/// `UnityEngine.UI.Image.AdjustBorders`: if the destination is smaller than the
/// sum of two opposite borders, both shrink proportionally so the corners still
/// meet; otherwise they keep their size.
pub(crate) fn shrink_borders(first: f32, second: f32, extent: f32) -> (f32, f32) {
    let total = first + second;
    if total > extent && total > 0.0 {
        let factor = (extent / total).max(0.0);
        (first * factor, second * factor)
    } else {
        (first, second)
    }
}

fn draw_nine_slice(
    texture: &macroquad::prelude::Texture2D,
    border: [f32; 4],
    pos: Vec2,
    size: Vec2,
    tint: Rgba,
    ui_scale: f32,
) {
    use macroquad::prelude::*;

    let texture_size = texture.size();
    let (width, height) = (texture_size.x, texture_size.y);
    let [left, bottom, right, top] = border;
    let (bl, br) = shrink_borders(left * ui_scale, right * ui_scale, size.x);
    let (bt, bb) = shrink_borders(top * ui_scale, bottom * ui_scale, size.y);
    let xs = [pos.x, pos.x + bl, pos.x + size.x - br, pos.x + size.x];
    let ys = [pos.y, pos.y + bt, pos.y + size.y - bb, pos.y + size.y];
    let sx = [0.0, left, width - right, width];
    let sy = [0.0, top, height - bottom, height];
    for row in 0..3 {
        for col in 0..3 {
            let w = xs[col + 1] - xs[col];
            let h = ys[row + 1] - ys[row];
            if w <= 0.0 || h <= 0.0 {
                continue;
            }
            draw_texture_ex(
                texture,
                xs[col],
                ys[row],
                color_rgba(tint),
                DrawTextureParams {
                    dest_size: Some(glam::Vec2::new(w, h)),
                    source: Some(Rect::new(
                        sx[col],
                        sy[row],
                        (sx[col + 1] - sx[col]).max(1.0),
                        (sy[row + 1] - sy[row]).max(1.0),
                    )),
                    ..Default::default()
                },
            );
        }
    }
}
fn decode_png(path: &Path) -> Result<UiTexture, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let texture = decode_png_reader(std::io::BufReader::new(file), &path.display().to_string())?;
    Ok(pad_to_sprite_rect(path, texture))
}

/// How an exported sprite PNG sits inside the sprite's rect (`sprite_trims.json`).
struct SpriteTrim {
    /// Repo-relative PNG path (forward slashes).
    file: String,
    /// Sprite rect size in pixels.
    full: [u32; 2],
    /// Top-left of the exported (trimmed) image inside the sprite rect.
    offset: [f32; 2],
    /// Size of the exported image.
    size: [u32; 2],
}

/// The trim table written by `tools/unity/export_sprite_trims.py` (empty when absent).
fn sprite_trims() -> &'static [SpriteTrim] {
    static TRIMS: std::sync::OnceLock<Vec<SpriteTrim>> = std::sync::OnceLock::new();
    TRIMS.get_or_init(|| {
        let candidates = [
            PathBuf::from("assets/game/reference/functional_ui_native/sprite_trims.json"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets/game/reference/functional_ui_native/sprite_trims.json"),
        ];
        let Some(text) = candidates
            .iter()
            .find_map(|candidate| std::fs::read_to_string(candidate).ok())
        else {
            return Vec::new();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Vec::new();
        };
        let Some(trims) = value.get("trims").and_then(serde_json::Value::as_object) else {
            return Vec::new();
        };
        let pair = |value: Option<&serde_json::Value>| -> Option<[f64; 2]> {
            let array = value?.as_array()?;
            Some([array.first()?.as_f64()?, array.get(1)?.as_f64()?])
        };
        trims
            .iter()
            .filter_map(|(file, entry)| {
                let full = pair(entry.get("full"))?;
                let offset = pair(entry.get("offset"))?;
                let size = pair(entry.get("size"))?;
                Some(SpriteTrim {
                    file: file.clone(),
                    full: [full[0] as u32, full[1] as u32],
                    offset: [offset[0] as f32, offset[1] as f32],
                    size: [size[0] as u32, size[1] as u32],
                })
            })
            .collect()
    })
}

/// Pads an exported (trimmed) sprite PNG back to the sprite's full rect. Unity draws the whole
/// rect, transparent margins included, into an `Image`: stretching the trimmed export instead
/// enlarges the artwork (a 28x22 check in a 35x35 sprite) and misplaces 9-slice borders.
fn pad_to_sprite_rect(path: &Path, texture: UiTexture) -> UiTexture {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let Some(trim) = sprite_trims()
        .iter()
        .find(|trim| normalized.ends_with(&trim.file))
    else {
        return texture;
    };
    if texture.width != trim.size[0] || texture.height != trim.size[1] {
        return texture;
    }
    let (full_w, full_h) = (trim.full[0], trim.full[1]);
    if full_w < texture.width || full_h < texture.height {
        return texture;
    }
    let ox = (trim.offset[0].round().max(0.0) as u32).min(full_w - texture.width);
    let oy = (trim.offset[1].round().max(0.0) as u32).min(full_h - texture.height);
    let mut rgba = vec![0_u8; (full_w * full_h * 4) as usize];
    for row in 0..texture.height {
        let src = (row * texture.width * 4) as usize;
        let dst = (((row + oy) * full_w + ox) * 4) as usize;
        let len = (texture.width * 4) as usize;
        rgba[dst..dst + len].copy_from_slice(&texture.rgba[src..src + len]);
    }
    UiTexture {
        width: full_w,
        height: full_h,
        rgba,
    }
}

/// Decodes an in-memory PNG (scenario preview images).
pub fn decode_png_bytes(bytes: &[u8]) -> Result<UiTexture, String> {
    decode_png_reader(std::io::Cursor::new(bytes), "embedded image")
}

fn decode_png_reader<R: std::io::Read>(reader: R, label: &str) -> Result<UiTexture, String> {
    let decoder = png::Decoder::new(reader);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("PNG header error in {}: {error}", label))?;
    let mut buffer = vec![0_u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("PNG decode error in {}: {error}", label))?;
    let raw = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => raw
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => raw
            .iter()
            .flat_map(|&value| [value, value, value, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => raw
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => {
            return Err(format!("indexed UI PNG {} is not supported", label));
        }
    };
    if rgba.len() != (info.width * info.height * 4) as usize {
        return Err(format!("unexpected UI PNG layout in {}", label));
    }
    Ok(UiTexture {
        width: info.width,
        height: info.height,
        rgba,
    })
}

/// Component-wise multiply of two 8-bit colours (used for chrome tints).
#[must_use]
pub fn multiply(a: Rgba, b: Rgba) -> Rgba {
    [
        (u16::from(a[0]) * u16::from(b[0]) / 255) as u8,
        (u16::from(a[1]) * u16::from(b[1]) / 255) as u8,
        (u16::from(a[2]) * u16::from(b[2]) / 255) as u8,
        (u16::from(a[3]) * u16::from(b[3]) / 255) as u8,
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_held_button_is_drawn_in_its_pressed_colour_while_the_pointer_is_on_it() {
        let mut frame = UiFrame::new(1.0);
        let (pos, size) = (Vec2::new(100.0, 50.0), Vec2::new(60.0, 40.0));
        frame.draws.push(UiDraw::NineSlice { texture: 0, border: [4.0; 4], dest_scale: 1.0, pos, size, tint: [255, 255, 255, 255] });
        // The button's icon is smaller than it and keeps its colour.
        frame.draws.push(UiDraw::Image { texture: 1, pos: pos + Vec2::new(10.0, 10.0), size: Vec2::new(20.0, 20.0), tint: [255, 255, 255, 255] });
        frame.hit(7, pos, size, UiAction::Quit);
        let tint_of = |frame: &UiFrame, index: usize| match &frame.draws[index] {
            UiDraw::NineSlice { tint, .. } | UiDraw::Image { tint, .. } => *tint,
            _ => panic!("not an image"),
        };
        let mut away = frame.clone();
        away.tint_pressed(7, Vec2::new(10.0, 10.0));
        assert_eq!(tint_of(&away, 0), [255, 255, 255, 255], "the pointer left the button");
        frame.tint_pressed(7, Vec2::new(120.0, 70.0));
        assert_eq!(tint_of(&frame, 0), [200, 200, 200, 255], "`ColorBlock.pressedColor` of the default colour block");
        assert_eq!(tint_of(&frame, 1), [255, 255, 255, 255]);
        // An id that is not on the frame changes nothing.
        let mut other = frame.clone();
        other.tint_pressed(9, Vec2::new(120.0, 70.0));
        assert_eq!(tint_of(&other, 0), [200, 200, 200, 255]);
    }

    use super::*;

    #[test]
    fn scaler_matches_ui_parity_formula() {
        assert!((UiFrame::scaler(1920.0, 1080.0) - 1.0).abs() < 1e-6);
        assert!((UiFrame::scaler(1280.0, 720.0) - (720.0_f32 / 1080.0)).abs() < 1e-6);
        assert!((UiFrame::scaler(3840.0, 2160.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_ui_scale_settings_multiply_the_canvas_scale_and_shrink_the_canvas() {
        // `UIScaler.totalScale = ScreenResolutionFactor * UIScale` (this thread's own settings).
        set_ui_scale_settings(true, 1.0, 0.5);
        assert!((UiFrame::scaler(1920.0, 1080.0) - 0.5).abs() < 1e-6);
        assert_eq!(canvas_size(), Some(Vec2::new(3840.0, 2160.0)), "half the scale: twice the canvas");
        // Without the automatic detection the setting's own resolution factor is used, whatever the window is.
        set_ui_scale_settings(false, 2.0, 1.25);
        assert!((UiFrame::scaler(1280.0, 720.0) - 2.5).abs() < 1e-6);
        set_ui_scale_settings(true, 1.0, 1.0);
        assert!((UiFrame::scaler(1280.0, 800.0) - 1280.0 / 1920.0).abs() < 1e-6);
        assert_eq!(canvas_size(), Some(Vec2::new(1920.0, 1200.0)), "a window that is not 16:9 has a taller canvas");
    }

    #[test]
    fn ui_hit_testing_and_actions() {
        let mut frame = UiFrame::new(1.0);
        frame.hit(
            7,
            Vec2::new(10.0, 10.0),
            Vec2::new(50.0, 20.0),
            UiAction::TogglePause,
        );
        frame.hit(
            9,
            Vec2::new(10.0, 40.0),
            Vec2::new(50.0, 20.0),
            UiAction::SetSpeed(4.0),
        );
        assert!(frame.over(Vec2::new(20.0, 15.0)));
        assert!(!frame.over(Vec2::new(80.0, 15.0)));
        let mut state = UiInteraction::default();
        assert!(
            frame
                .interact(Vec2::new(20.0, 15.0), true, false, &mut state)
                .is_empty()
        );
        assert_eq!(state.active, Some(7));
        assert_eq!(
            frame.interact(Vec2::new(22.0, 16.0), false, true, &mut state),
            vec![UiAction::TogglePause]
        );
        // Release outside cancels the click.
        frame.interact(Vec2::new(20.0, 15.0), true, false, &mut state);
        assert!(
            frame
                .interact(Vec2::new(200.0, 200.0), false, true, &mut state)
                .is_empty()
        );
        assert_eq!(state.active, None);
    }

    /// The rule behind the panels' id blocks (see [`UiId`]): a click answers with the actions of EVERY area on the id it hit, wherever the others are.
    #[test]
    fn areas_that_share_an_id_answer_together_and_shared_ids_lists_them() {
        let mut frame = UiFrame::new(1.0);
        frame.hit(5, Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0), UiAction::TogglePause);
        frame.hit(6, Vec2::new(20.0, 0.0), Vec2::new(10.0, 10.0), UiAction::Quit);
        assert!(frame.shared_ids().is_empty());
        let mut state = UiInteraction::default();
        assert_eq!(frame.interact(Vec2::new(5.0, 5.0), true, true, &mut state), vec![UiAction::TogglePause]);
        // Another panel's area on id 5, far from the first one: a click on it fires both actions.
        frame.hit(5, Vec2::new(100.0, 100.0), Vec2::new(10.0, 10.0), UiAction::Consume);
        assert_eq!(frame.shared_ids(), vec![5]);
        assert_eq!(frame.interact(Vec2::new(105.0, 105.0), true, true, &mut state), vec![UiAction::TogglePause, UiAction::Consume]);
    }

    #[test]
    fn ui_atlas_loads_when_present() {
        let Ok(Some(ui)) = UiAssets::discover() else {
            eprintln!("UI atlas not present; skipping");
            return;
        };
        if !ui.root.join("assets").join("game").join("ui").is_dir() {
            eprintln!("UI pictures not present (the repository holds the atlas metadata only); skipping");
            return;
        }
        assert!(ui.records.len() > 200, "records: {}", ui.records.len());
        assert!(ui.texture_count() > 100, "textures: {}", ui.texture_count());
        let panel = ui
            .pick(&["ui/panel/hud_time_frame", "ui/panel/tooltip_frame"])
            .expect("panel record");
        assert!(panel.nine_slice);
        assert!(panel.border.is_some());
        let mut fonts = FontSet::load(&ui).expect("fonts");
        let width = fonts.measure(FontRole::Body, "RUSTibites", 16.0);
        assert!(width > 20.0, "measured width {width}");
    }
}

thread_local! {
    /// The size in canvas units of the window the UI was last scaled for (`window / totalScale`); `None` before any.
    static LAST_CANVAS: std::cell::Cell<Option<Vec2>> = const { std::cell::Cell::new(None) };
    /// `(AutomaticallyDetectScreenResolution, ScreenResolutionFactor, UIScale)` (the UI is built on one thread; a test's thread has its own).
    static UI_SCALE_SETTINGS: std::cell::Cell<(bool, f32, f32)> = const { std::cell::Cell::new((true, 1.0, 1.0)) };
}

/// The canvas the UI is laid out in: the window over the UI's scale (1920 x 1080 at the capture's window and scale 1).
#[must_use]
pub fn canvas_size() -> Option<Vec2> {
    LAST_CANVAS.with(std::cell::Cell::get)
}

/// The user settings that scale the whole UI: `(AutomaticallyDetectScreenResolution, ScreenResolutionFactor, UIScale)`.
#[must_use]
pub fn ui_scale_settings() -> (bool, f32, f32) {
    UI_SCALE_SETTINGS.with(std::cell::Cell::get)
}

/// Sets them (the start-up read of the player's user settings and a change on the settings screens do).
pub fn set_ui_scale_settings(automatic: bool, resolution: f32, scale: f32) {
    UI_SCALE_SETTINGS.with(|cell| cell.set((automatic, resolution.clamp(0.25, 4.0), scale.clamp(0.25, 3.0))));
}
