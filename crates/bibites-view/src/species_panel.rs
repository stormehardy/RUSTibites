//! The in-world Species panel: the lineage graph's chrome (`AncestralLineagePanel`: the top bar,
//! the graph view), the species tabs (`SpeciesTabManager`) and the Info tab (`SpeciesInfoPanel`)
//! with its actions - rename, notes, favourite, save, place, select, prune, parent and root.
//!
//! Prefab values (level2): the panel is the whole screen on (0.184) grey with a 50-high top bar
//! (the title centred: "Ancestral Lineage Graph -" in Liberation Sans 24, "N Recorded Species" and
//! " (M displayed)" in Roboto-Bold 24, all white; the 40 x 40 buttons Mesh, Tree, Settings and Exit
//! from x 1745 to 1895); the graph view is a navy (0, 0, 0.106) rectangle from (15, 50), 1895 x 1020;
//! the species panels hang from the overlay's top-left corner (15, 50). A selected species shows
//! the tab row (40 x 40 icon tabs at x 45, 95, 145, 195 and a close tab at 1161.69, a selected or
//! hovered tab rises 16 over the others, which sit half behind the panel) and one panel at a time;
//! the Info panel is 332 x 489.71, `base_icon` at 0.84 grey: the name row with its rename icon, the
//! species ID, the preview frame, the root and parent buttons, the six 35 x 35 buttons (favourite
//! toggle, save, place, select, prune, edit), the creation time, the generation, the member and
//! descendant counts and the notes field.
//!
//! The Genes tab (`SpeciesGenesPanel`, `GeneLineInfo`) is 379.52 wide: the species' name (Roboto-Bold
//! 20), the "Parent species:" row (its button and the distance to it as a percentage of the species
//! span, green Roboto-Bold 16), the "Compared species:" row (a dropdown of the species of the graph and
//! the gene distance to it) and the 35 genes in `geneSettings` order, 16.41 high at a 21.41 pitch: the
//! gene's name (LiberationSans 14, flexible), its value in the template (90 wide, right-aligned, 3
//! decimals) and the difference from the compared species (100 wide, centred, signed, in black whose
//! alpha follows the gene's contribution to speciation), with the gene's help and the difference's
//! breakdown as tooltips.
//!
//! The graph itself is still the simple depth / sibling plot (the game's tree and mesh views are
//! not drawn): the species are the nodes you click.

pub(crate) mod brain;
mod layout;
pub(crate) mod position;

use bibites_core::format::FloatValueFormat;
use bibites_core::genome::Gene;
use bibites_core::ids::SpeciesId;
use bibites_core::math::Vec2;
use bibites_core::species::{SpeciesListEntry, SpeciesRegistry};

use crate::app::{App, ConfirmKind, FrameInput};
use crate::tooltip::{self, Tip, TipRect};
use crate::ui::{
    ControlCommand, DropdownTarget, FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame,
};

const WHITE: Rgba = [255, 255, 255, 255];
/// `AncestralLineagePanel`'s `Image`: (0.184).
const PANEL_GREY: Rgba = [47, 47, 47, 255];
/// The Info panel's labels and names: (0.149).
const INK: Rgba = [38, 38, 38, 255];
/// The Info panel's body lines.
const BLACK: Rgba = [0, 0, 0, 255];
/// The input fields' text: (50, 50, 50).
const FIELD_INK: Rgba = [50, 50, 50, 255];
/// The frames' tint: (0.84).
const FRAME_GREY: Rgba = [214, 214, 214, 255];
/// A disabled `Selectable`: `disabledColor` (0.784, 0.784, 0.784, 0.502).
const DISABLED: Rgba = [200, 200, 200, 128];

/// The overlay's top-left corner, where the species panels hang.
const OVERLAY: Vec2 = Vec2::new(15.0, 50.0);
/// A selected (or hovered) tab rises by this much (`TabsButton.verticalTravelOnSelection`).
const TAB_RISE: f32 = 11.0;
/// A tab's image at rest: 40 high, its middle 45 from the top of the screen (the `Tab Element`'s middle is 10 above the panel, the image
/// sits at `TabsButton.initialY` = -5 from it).
const TAB_REST: f32 = 45.0;
/// The Info tab's index.
pub const TAB_INFO: u8 = 0;
/// The Genes tab's index.
pub const TAB_GENES: u8 = 1;
/// The Brain tab's index.
const TAB_BRAIN: u8 = 2;
/// The Position tab's index (the last).
const TAB_POSITION: u8 = 3;

/// Hit ids (8950 - 8999).
const HIT_BACKDROP: u32 = 8950;
const HIT_BAR: u32 = 8951;
const HIT_TAB: u32 = 8960;
const HIT_CLOSE_TAB: u32 = 8965;
const HIT_PANEL: u32 = 8970;
const HIT_NAME: u32 = 8971;
const HIT_ROOT: u32 = 8972;
const HIT_PARENT: u32 = 8973;
const HIT_BUTTON: u32 = 8974;
const HIT_NOTES: u32 = 8981;
const HIT_BAR_BUTTON: u32 = 8985;
const HIT_GENES_PARENT: u32 = 8990;
const HIT_COMPARED: u32 = 8991;

/// The path every species-panel control's tooltip is keyed by.
const SCENE: &str = "UICanvas/MainUI/Layer2/AncestralLineagePanel/";

/// The species panel's text fields and Prune button state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// The species name being typed (`speciesNameEditor`).
    pub name: Option<String>,
    /// The notes field has the keyboard; the text it started with (Escape restores it).
    pub notes: Option<String>,
    /// The Prune button after a favourite toggle (`SetFavorite` recomputes it with its own formula).
    pub prune_override: Option<bool>,
}

impl State {
    /// Whether a text field of the panel has the keyboard.
    #[must_use]
    pub fn typing(&self) -> bool {
        self.name.is_some() || self.notes.is_some()
    }
}

/// The Info panel's rectangles (reference px, `x, y, w, h`).
#[derive(Clone, Debug, PartialEq)]
pub struct InfoLayout {
    /// The panel.
    pub panel: [f32; 4],
    /// The name row (`Name Row` button; the whole row while the name is edited).
    pub name_row: [f32; 4],
    /// The name's text field (while it is edited).
    pub name_field: [f32; 4],
    /// The rename icon at the end of the row.
    pub rename_icon: [f32; 4],
    /// The format warning icon (shown while the typed name is wrong).
    pub warning_icon: [f32; 4],
    /// The "Species ID:" row.
    pub id_row: [f32; 4],
    /// The ID's text (after the label).
    pub id_value: [f32; 4],
    /// The preview frame.
    pub preview: [f32; 4],
    /// "Root species:" row.
    pub root_title: [f32; 4],
    /// "Parent species:" row.
    pub parent_title: [f32; 4],
    /// The root species button.
    pub root_button: [f32; 4],
    /// The parent species button (the "None" text takes its place without a parent).
    pub parent_button: [f32; 4],
    /// Favourite, save, place, select, prune, edit.
    pub buttons: [[f32; 4]; 6],
    /// Time of creation, generation, alive members, alive descendants.
    pub lines: [[f32; 4]; 4],
    /// "Notes:".
    pub notes_label: [f32; 4],
    /// The notes field.
    pub notes: [f32; 4],
}

/// A tab's icon rectangle: `0..=3` the Info, Genes, Brain and Position tabs. `raised` for the selected or hovered one (`TabsButton`: the
/// image goes from `initialY` to `initialY + verticalTravelOnSelection`, 11 higher).
#[must_use]
pub fn tab_rect(index: u8, raised: bool) -> [f32; 4] {
    let centre_y = if raised { TAB_REST - TAB_RISE } else { TAB_REST };
    [
        OVERLAY.x + 30.0 + f32::from(index) * 50.0 - 20.0,
        centre_y - 20.0,
        40.0,
        40.0,
    ]
}

/// The close tab's icon rectangle over a panel `panel_width` wide: `Species Tabs` stretches over the panel (a `HorizontalLayoutGroup` with 10 of
/// padding on each side and a flexible spacer before the last tab), so the tab ends 10 before the panel's right edge. It always sits raised
/// (its image's `localPosition` is (0, 6)).
#[must_use]
pub fn close_tab_rect(panel_width: f32) -> [f32; 4] {
    [
        OVERLAY.x + panel_width - 30.0 - 20.0,
        TAB_REST - TAB_RISE - 20.0,
        40.0,
        40.0,
    ]
}

/// A top bar button: `0` Mesh, `1` Tree, `2` Settings, `3` Exit.
#[must_use]
pub fn bar_button(index: usize) -> [f32; 4] {
    [1745.0 + index as f32 * 50.0 - 20.0, 5.0, 40.0, 40.0]
}

/// The top bar's title: `LiberationSans SDF - Fallback`, 24, bold style (TMP widens the regular atlas), white.
const TITLE_FONT: &str = "LiberationSans SDF - Fallback +bold";

/// `Roboto-Regular SDF` in TMP's bold style (the labels called `Title`, the gene values): synthetic bold on the regular face.
const SEMIBOLD: &str = "Roboto-Regular SDF +bold";

/// A text run in [`SEMIBOLD`].
fn semibold(frame: &mut UiFrame, text: impl Into<String>, pos: Vec2, px: f32, color: Rgba) {
    frame.draws.push(UiDraw::Text {
        font: Some(SEMIBOLD.to_owned()),
        text: text.into(),
        pos,
        px,
        color,
        role: FontRole::Body,
    });
}

fn contains(rect: [f32; 4], point: Vec2) -> bool {
    point.x >= rect[0]
        && point.x < rect[0] + rect[2]
        && point.y >= rect[1]
        && point.y < rect[1] + rect[3]
}

// ------------------------------------------------------------------------------ app side

impl App {
    /// The species whose panels show (`SpeciesTabManager.targetSpecies`).
    #[must_use]
    pub fn species_info_entry(&self) -> Option<&SpeciesListEntry> {
        let id = self.species_info?;
        self.lineage_species.iter().find(|entry| entry.id == id)
    }

    /// The picture of species `id`'s procedural bibite (`BibiteGenePreviewer` of the template's genes), `size`
    /// canvas units across, rendered at `pixels_per_unit` screen pixels per unit; `None` without the game's
    /// sprites or the species.
    #[must_use]
    pub fn species_picture(
        &self,
        id: SpeciesId,
        style: crate::preview::PreviewStyle,
        size: [f32; 2],
        pixels_per_unit: f32,
    ) -> Option<std::sync::Arc<crate::ui::UiTexture>> {
        let assets = self.assets.as_ref()?;
        let record = self.world.species.get(id)?;
        let request = crate::preview::PreviewRequest {
            genome: &record.template.genome,
            size: None,
            style,
            procedural: self.procedural_size,
            picture: size,
            pixels_per_unit,
        };
        self.previews.get(assets, &request)
    }

    /// Makes the pictures of these species ([`App::species_picture`]'s arguments) in parallel, ahead of the frame that draws them.
    pub fn warm_species_pictures(
        &self,
        wanted: &[(SpeciesId, crate::preview::PreviewStyle, [f32; 2], f32)],
    ) {
        let Some(assets) = self.assets.as_ref() else {
            return;
        };
        let requests: Vec<crate::preview::PreviewRequest<'_>> = wanted
            .iter()
            .filter_map(|(id, style, size, pixels_per_unit)| {
                let record = self.world.species.get(*id)?;
                Some(crate::preview::PreviewRequest {
                    genome: &record.template.genome,
                    size: None,
                    style: *style,
                    procedural: self.procedural_size,
                    picture: *size,
                    pixels_per_unit: *pixels_per_unit,
                })
            })
            .collect();
        self.previews.warm(assets, &requests);
    }

    /// `SpeciesPanel.SelectAndFocusSpecies`: the panels show `id` (`None` hides them, with the
    /// text fields), and the graph highlights its node.
    pub fn select_species_info(&mut self, id: Option<SpeciesId>) {
        self.species_commit_edit();
        self.species_ui = State::default();
        self.species_info = id;
        let Some(id) = id else {
            self.species_compared = None;
            self.position = None;
            self.brain = None;
            return;
        };
        // `SpeciesTabManager.SelectSpecies` fills every panel, the Position tab's slider keeping its value.
        self.position_fill(id);
        self.brain_fill(id);
        // `SelectSpecies`: the shown species is compared with its parent, or with itself.
        self.species_compared = Some(
            self.lineage_species
                .iter()
                .find(|entry| entry.id == id)
                .and_then(|entry| entry.parent)
                .unwrap_or(id),
        );
        // `SelectToCompareSpecies` -> `brainPanel.UpdateCompared`: the differences with it are marked.
        self.brain_update_marks();
        if let Some(index) = self.lineage_species.iter().position(|entry| entry.id == id) {
            self.species_selected = index;
        }
        if let Some(tree) = &self.lineage {
            self.lineage_selected = tree.nodes.iter().position(|node| node.id == id);
        }
    }

    /// Ends the text field that has the keyboard (`TMP_InputField.onEndEdit`): a typed name is
    /// applied when it has the game's format and dropped otherwise; the notes were saved as typed.
    pub fn species_commit_edit(&mut self) {
        self.species_ui.notes = None;
        let Some(text) = self.species_ui.name.take() else {
            return;
        };
        if let Some(id) = self.species_info
            && self.world.species.rename(id, &text)
        {
            self.refresh_species_list();
            self.last_action = format!("species renamed to {text}");
        }
    }

    /// Escape in a text field: the name is dropped, the notes go back to what they were
    /// (`TMP_InputField.restoreOriginalTextOnEscape`). `false` when no field had the keyboard.
    pub fn species_cancel_edit(&mut self) -> bool {
        if self.species_ui.name.take().is_some() {
            return true;
        }
        if let Some(original) = self.species_ui.notes.take() {
            if let Some(id) = self.species_info {
                self.world.species.set_description(id, &original);
                self.refresh_species_list();
            }
            return true;
        }
        false
    }

    /// Typing into the name or notes field: characters, Backspace, Enter (submits the name, a new
    /// line in the notes) and Ctrl + V; consumes what it takes.
    pub fn species_edit_typing(&mut self, input: &FrameInput) {
        if !self.species_ui.typing() {
            return;
        }
        let typed = std::mem::take(&mut self.typed);
        let pasted = if input.paste_tag {
            self.clipboard_in.take().unwrap_or_default()
        } else {
            String::new()
        };
        if self.species_ui.name.is_some() {
            let keys = self.edit_keys(input);
            if self.edit_caret_owner != 3 {
                self.edit_caret_owner = 3;
                self.edit_caret = crate::text_field::Caret::at_end(self.species_ui.name.as_deref().unwrap_or(""));
            }
            let typed = format!("{typed}{pasted}");
            if let Some(name) = self.species_ui.name.as_mut() {
                crate::text_field::edit(name, &mut self.edit_caret, &typed, usize::MAX, keys);
            }
            if input.enter {
                self.species_commit_edit();
            }
            return;
        }
        let Some(id) = self.species_info else {
            return;
        };
        let Some(entry) = self.species_info_entry() else {
            return;
        };
        let mut text = entry.description.clone();
        let before = text.clone();
        let keys = self.edit_keys(input);
        if self.edit_caret_owner != 8 {
            self.edit_caret_owner = 8;
            self.edit_caret = crate::text_field::Caret::at_end(&text);
        }
        let typed = format!("{typed}{pasted}");
        crate::text_field::edit(&mut text, &mut self.edit_caret, &typed, usize::MAX, keys);
        if input.enter {
            self.edit_caret.clamp(&text);
            self.edit_caret.insert_newline(&mut text);
        }
        if text != before {
            self.world.species.set_description(id, &text);
            self.refresh_species_list();
        }
    }

    /// The species of the graph by name (`SpeciesPanel.displayedSpecies`), what the compared
    /// dropdown lists.
    #[must_use]
    pub fn species_displayed(&self) -> Vec<(SpeciesId, String)> {
        self.lineage_view
            .displayed()
            .into_iter()
            .map(|id| {
                let name = self
                    .lineage_species
                    .iter()
                    .find(|entry| entry.id == id)
                    .map_or_else(String::new, |entry| entry.name.clone());
                (id, name)
            })
            .collect()
    }

    /// The recorded species as the lineage tree needs them (`LineageTreePanel.BuildTree`'s input).
    #[must_use]
    pub fn lineage_infos(&self) -> Vec<crate::lineage_tree::SpeciesInfo> {
        self.lineage_species
            .iter()
            .map(|entry| crate::lineage_tree::SpeciesInfo {
                id: entry.id,
                parent: entry.parent,
                root: entry.root,
                name: entry.name.clone(),
                favorite: entry.favorite,
                apparition: entry.total_index_of_apparition,
                disappearance: entry.total_index_of_disappearance,
                points_present: entry.n_points_present,
            })
            .collect()
    }

    /// The view that shows is laid out afresh with the current options (`LineageTreePanel.BuildTree` for the tree,
    /// `LineageMeshPanel.ResetState` + `GenerateMesh` for the mesh).
    pub fn rebuild_lineage_view(&mut self) {
        let settings = self.lineage_tree_settings();
        if self.lineage_view.mesh_mode {
            // The shapes start from what the species have now: the members are counted again first (the registry keeps
            // its counts at the tick boundary, and a world that was just loaded or is paused has not had one).
            self.world.reconcile_species_counts();
            let now = self.world.species_points_now();
            self.lineage_view
                .rebuild_mesh(&self.world.species, &now, settings);
        } else {
            let infos = self.lineage_infos();
            self.lineage_view.rebuild(&infos, settings);
        }
    }

    /// `SpeciesPanel.SelectPanel(tree)`: the Mesh and Tree buttons. The view goes back to its first place (`ResetView`),
    /// the Settings panel shows the options of the view, and the species that was selected stays selected.
    pub fn lineage_select_panel(&mut self, tree: bool) {
        // `ResetView` runs before the panels switch: with the view that is leaving (its zoom, its limits and its content).
        self.lineage_view.reset_view();
        self.lineage_view.mesh_mode = !tree;
        self.lineage_settings_scroll = 0.0;
        self.rebuild_lineage_view();
        self.last_action = format!("lineage {}", if tree { "tree" } else { "mesh" });
    }

    /// "Effective Species Span" (`GlobalLineageManager.usedSpeciesSpan`).
    #[must_use]
    pub fn lineage_effective_span(&self) -> f32 {
        self.world.species.current_used_span(&self.world.settings)
    }

    /// A toggle of the Settings panel: the user setting flips and the tree is built again
    /// (`UpdateHideExtinctLineages`, `UpdateShowFavorites`, `UpdateScaleImportantSpecies`); a species that is not
    /// drawn any more leaves the species panels.
    pub fn lineage_toggle_option(&mut self, option: crate::lineage_settings::LineageOption) {
        use crate::lineage_settings::LineageOption;
        // Without a settings file the options are kept for this run.
        if self.user_settings_path.is_none() && self.lineage_options.is_none() {
            self.lineage_options = Some(self.lineage_tree_settings());
        }
        if let Some(options) = self.lineage_options.as_mut() {
            match option {
                LineageOption::HideExtinct => options.hide_extinct = !options.hide_extinct,
                LineageOption::ShowFavorites => options.show_favorites = !options.show_favorites,
                LineageOption::ScaleSpecies => options.scale_species = !options.scale_species,
            }
        } else {
            let mut settings =
                crate::user_settings::UserSettings::load(self.user_settings_path.clone());
            if let Some(index) = crate::user_settings::index_of(option.label()) {
                settings.toggle(index);
            }
        }
        let shown = self.species_info;
        self.rebuild_lineage_view();
        if option != LineageOption::ScaleSpecies
            && let Some(id) = shown
            && !self.lineage_view.shows(id)
        {
            self.select_species_info(None);
        }
        self.last_action = format!("{} toggled", option.label());
    }

    /// The tree's three options (`LineagePanelSettingsManager`'s user settings).
    #[must_use]
    pub fn lineage_tree_settings(&self) -> crate::lineage_tree::TreeSettings {
        if let Some(options) = self.lineage_options {
            return options;
        }
        let settings = crate::user_settings::UserSettings::load(self.user_settings_path.clone());
        crate::lineage_tree::TreeSettings {
            hide_extinct: settings.flag("Hide Extinct Lineages"),
            show_favorites: settings.flag("Show Favorited Lineages"),
            scale_species: settings.flag("Scale Important Species"),
        }
    }

    /// The tree's window takes the wheel (Ctrl zooms, Shift scrolls sideways), the right button (a drag
    /// pans) and the scroll bars (`LineageWindow.Update`, `AncestralLineageScroll`).
    pub fn lineage_input(&mut self, input: &FrameInput) {
        use crate::lineage_view::{
            Drag, VIEW, WHEEL_ZOOM, handle_rect, horizontal_track, vertical_track,
        };
        // The window loop hands the panel what the pointer really did (see `App::lineage_raw`).
        let raw = self.lineage_raw.take();
        let merged;
        let input = match raw {
            Some(raw) => {
                merged = FrameInput {
                    wheel: raw.wheel,
                    primary_down: raw.primary_down,
                    primary_pressed: raw.primary_pressed,
                    primary_released: raw.primary_released,
                    secondary_down: raw.secondary_down,
                    ctrl_down: raw.ctrl_down,
                    right_ctrl_down: raw.right_ctrl_down,
                    shift_down: raw.shift_down,
                    cursor: raw.cursor,
                    ..*input
                };
                &merged
            }
            None => input,
        };
        // `LineageWindow.Update` / `AncestralLineageScroll.OnScroll` test both Control keys (`KeyCode` 305 and 306).
        let ctrl = input.ctrl_down || input.right_ctrl_down;
        // A dialog's screen blocker keeps the wheel and the drags from the view under it (the Save dialog opens over this panel).
        let live = self.panels.is_open(crate::ui::PanelKind::Lineage)
            && self.lineage_view.built()
            && self.confirm_dialog.is_none()
            && self.save_dialog.is_none();
        if !live {
            self.lineage_view.drag = Drag::None;
            return;
        }
        let s = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let cursor = [input.cursor.x / s, input.cursor.y / s];
        let inside = |rect: [f32; 4]| {
            cursor[0] >= rect[0]
                && cursor[1] >= rect[1]
                && cursor[0] < rect[0] + rect[2]
                && cursor[1] < rect[1] + rect[3]
        };
        let g = self.lineage_view.geometry();
        let over_settings = self.lineage_settings_open && inside(crate::lineage_settings::PANEL);
        self.lineage_settings_input(input, cursor, over_settings);
        // The wheel: with Ctrl and no Shift it zooms (anywhere), else it scrolls the content under the pointer.
        if input.wheel != 0.0 && !over_settings {
            if ctrl && !input.shift_down {
                self.lineage_view.zoom_by(input.wheel * WHEEL_ZOOM);
            } else if !ctrl && inside(VIEW) {
                self.lineage_view.scroll(input.wheel, input.shift_down);
            }
        }
        // The right button drags the content (`AncestralLineageScroll` hands it to the ScrollRect as a left one).
        let right = input.secondary_down;
        match self.lineage_view.drag {
            Drag::Pan(last) if right => {
                self.lineage_view
                    .pan(cursor[0] - last[0], cursor[1] - last[1]);
                self.lineage_view.drag = Drag::Pan(cursor);
            }
            Drag::Pan(_) => self.lineage_view.drag = Drag::None,
            Drag::None if right && !self.lineage_right_was_down && inside(VIEW) => {
                self.lineage_view.drag = Drag::Pan(cursor);
            }
            _ => {}
        }
        self.lineage_right_was_down = right;
        // The scroll bars: a press on the handle grabs it, a press on the track pages towards the pointer.
        let (horizontal, vertical) = self.lineage_view.bars();
        let tracks = [
            (false, horizontal_track(g), g.horizontal, horizontal),
            (true, vertical_track(g), g.vertical, vertical),
        ];
        if input.primary_pressed {
            for (is_vertical, track, shown, bar) in tracks {
                if !shown || !inside(track) {
                    continue;
                }
                let handle = handle_rect(track, bar, is_vertical);
                if inside(handle) {
                    let grab = if is_vertical {
                        (cursor[1] - handle[1]) / handle[3].max(1.0)
                    } else {
                        (cursor[0] - handle[0]) / handle[2].max(1.0)
                    };
                    self.lineage_view.drag = if is_vertical {
                        Drag::VerticalBar(grab)
                    } else {
                        Drag::HorizontalBar(grab)
                    };
                } else {
                    // `Scrollbar.ClickRepeat`: a page of the handle's size towards the pointer.
                    let before = if is_vertical {
                        cursor[1] < handle[1]
                    } else {
                        cursor[0] < handle[0]
                    };
                    let towards = if is_vertical == before { 1.0 } else { -1.0 };
                    self.lineage_view
                        .set_bar(is_vertical, bar.value + towards * bar.size);
                }
            }
        }
        let drag = self.lineage_view.drag;
        if let Drag::VerticalBar(grab) | Drag::HorizontalBar(grab) = drag {
            if input.primary_down {
                let is_vertical = matches!(drag, Drag::VerticalBar(_));
                let (track, bar) = if is_vertical {
                    (vertical_track(g), vertical)
                } else {
                    (horizontal_track(g), horizontal)
                };
                let sliding = crate::lineage_view::sliding_area(track);
                let along = if is_vertical { sliding[3] } else { sliding[2] };
                let room = along * (1.0 - bar.size);
                // The handle is 10 longer than the part of the sliding area it covers, at both ends.
                let length = along * bar.size;
                if room > 0.0 {
                    let value = if is_vertical {
                        // The handle's top follows the cursor; the vertical bar's 0 is at the bottom.
                        let top = cursor[1] - grab * (length + 20.0) + 10.0;
                        (sliding[1] + sliding[3] - length - top) / room
                    } else {
                        let left = cursor[0] - grab * (length + 20.0) + 10.0;
                        (left - sliding[0]) / room
                    };
                    self.lineage_view.set_bar(is_vertical, value);
                }
            } else {
                self.lineage_view.drag = Drag::None;
            }
        }
    }

    /// The Settings panel's scroll view: the wheel over it and its bar (`ScrollRect`, vertical, clamped, 35 a notch).
    fn lineage_settings_input(&mut self, input: &FrameInput, cursor: [f32; 2], over: bool) {
        use crate::lineage_settings::{BAR_TRACK, PANEL, layout};
        use crate::lineage_view::{BarState, SCROLL_SENSITIVITY, handle_rect};
        if !self.lineage_settings_open {
            self.lineage_settings_grab = None;
            return;
        }
        // The mesh view has no "Lineage Tree" group (`ShowSettingsOfType`).
        let column = layout(!self.lineage_view.mesh_mode);
        let max = column.max_scroll();
        let _ = PANEL;
        if over && input.wheel != 0.0 && !input.ctrl_down {
            self.lineage_settings_scroll =
                (self.lineage_settings_scroll - input.wheel * SCROLL_SENSITIVITY).clamp(0.0, max);
        }
        self.lineage_settings_scroll = self.lineage_settings_scroll.clamp(0.0, max);
        if max <= 0.0 {
            self.lineage_settings_grab = None;
            return;
        }
        // The bar counts from its top; the shared handle helper from the bottom.
        let size = (PANEL[3] / column.height).clamp(0.0, 1.0);
        let value = (self.lineage_settings_scroll / max).clamp(0.0, 1.0);
        let handle = handle_rect(
            BAR_TRACK,
            BarState {
                value: 1.0 - value,
                size,
            },
            true,
        );
        let inside = |rect: [f32; 4]| {
            cursor[0] >= rect[0]
                && cursor[1] >= rect[1]
                && cursor[0] < rect[0] + rect[2]
                && cursor[1] < rect[1] + rect[3]
        };
        if input.primary_pressed && inside(BAR_TRACK) {
            if inside(handle) {
                self.lineage_settings_grab = Some((cursor[1] - handle[1]) / handle[3].max(1.0));
            } else {
                // A page towards the pointer.
                let towards = if cursor[1] < handle[1] { -1.0 } else { 1.0 };
                self.lineage_settings_scroll =
                    (self.lineage_settings_scroll + towards * size * column.height).clamp(0.0, max);
            }
        }
        if let Some(grab) = self.lineage_settings_grab {
            if input.primary_down {
                let sliding_top = BAR_TRACK[1] + 10.0;
                let sliding_h = BAR_TRACK[3] - 20.0;
                let room = sliding_h * (1.0 - size);
                if room > 0.0 {
                    let top = cursor[1] - grab * (sliding_h * size + 20.0) + 10.0;
                    let value = ((top - sliding_top) / room).clamp(0.0, 1.0);
                    self.lineage_settings_scroll = value * max;
                }
            } else {
                self.lineage_settings_grab = None;
            }
        }
    }

    /// Rebuilds the cached species list after a change of the registry.
    fn refresh_species_list(&mut self) {
        self.lineage_species = self.world.species.species_list();
    }

    /// `SpeciesInfoPanel.SetFavorite`.
    pub fn species_set_favorite(&mut self) {
        let Some(entry) = self.species_info_entry() else {
            return;
        };
        let (id, favorite) = (entry.id, !entry.favorite);
        self.world.species.set_favorite(id, favorite);
        self.refresh_species_list();
        // The handler recomputes the Prune button with its own formula: `!absoluteKeep && there are
        // members && it disappeared once` (`FillPanel` asks for no member instead). The registry
        // keeps no disappearance index and a species that disappeared has no member, so the button
        // ends up disabled until the panel is filled again.
        self.species_ui.prune_override = Some(false);
        self.last_action = format!("species favorite: {favorite}");
    }

    /// Whether the Prune button can be pressed (`SpeciesInfoPanel.FillPanel`).
    #[must_use]
    pub fn species_can_prune(&self) -> bool {
        match (self.species_ui.prune_override, self.species_info) {
            (Some(enabled), _) => enabled,
            (None, Some(id)) => self.world.species.can_prune(id),
            (None, None) => false,
        }
    }

    /// `SpeciesInfoPanel.PruneSpecies`: the warning first (unless it was switched off).
    pub fn species_prune(&mut self) {
        let Some(id) = self.species_info else {
            return;
        };
        if !self.species_can_prune() {
            return;
        }
        if self.warning_off(crate::warnings::ActionWarning::PruneSpecies) {
            self.species_actually_prune(id);
            return;
        }
        self.confirm_do_not_show = false;
        self.confirm_dialog = Some(ConfirmKind::PruneSpecies(id));
        self.last_action = "confirm: prune species?".to_owned();
    }

    /// `SpeciesInfoPanel.ActuallyPruneSpecies`: the species leaves the tree, its data merge into
    /// its parent's, and that one is selected.
    pub fn species_actually_prune(&mut self, id: SpeciesId) {
        // `PruneSpeciesFromTree`: a species with more points than the prune limit stays, and the panel
        // selects whichever species the call returns.
        let merged = self.world.prune_species(id);
        self.lineage = Some(self.world.species.lineage_tree());
        self.refresh_species_list();
        // `SpeciesPanel.OnSpeciesPruned`: the tree is built again (and fits the view again), or the mesh is made again.
        self.rebuild_lineage_view();
        self.lineage_selected = None;
        self.species_selected = self
            .species_selected
            .min(self.lineage_species.len().saturating_sub(1));
        self.select_species_info(merged);
        self.last_action = "species pruned".to_owned();
    }

    /// `SpeciesTabManager.SelectParent`.
    pub fn species_select_parent(&mut self) {
        if let Some(parent) = self.species_info_entry().and_then(|entry| entry.parent) {
            self.select_species_info(Some(parent));
        }
    }

    /// `SpeciesTabManager.SelectRoot`.
    pub fn species_select_root(&mut self) {
        if let Some(root) = self.species_info_entry().map(|entry| entry.root) {
            self.select_species_info(Some(root));
        }
    }

    /// `SpeciesInfoPanel.PlaceSpecies`: the panel closes and the bibite placer starts with the
    /// species' template.
    pub fn species_place(&mut self) {
        let Some(id) = self.species_info else {
            return;
        };
        let Some(source) = self.species_template_source(id) else {
            return;
        };
        self.panels.close(crate::ui::PanelKind::Lineage);
        self.lineage_settings_open = false;
        self.start_bibite_placer_with(source);
    }

    /// `SpeciesInfoPanel.SaveSpecies`: the species' template goes through the Save Bibite dialog (`SaveController.SaveTemplateOfBibite`): it opens on the
    /// species' name and notes with "Save as template" on and locked, and its Save writes `Bibites/Templates/<name>.bb8template` (the folder the Library
    /// lists) with the typed description and the generation of the species' first specimen.
    pub fn species_save(&mut self) {
        let Some(id) = self.species_info else {
            return;
        };
        // A click on a button ends the edit of a text field (`onEndEdit`): the dialog starts from the name and the notes as they stand.
        self.species_commit_edit();
        let Some(record) = self.world.species.get(id) else {
            return;
        };
        // `template.description = species.description`: the dialog's defaults are the template's name (the species') and that description.
        let (name, description) = (record.name(), record.description.clone());
        self.begin_save_dialog(crate::save_dialog::Target::Species(id), name, description, false);
    }

    /// The template the Save dialog writes for species `id`: the species' template with the generation of its first specimen
    /// (`template.generation = generationOfFirstSpecimen`); the dialog's Save gives it the typed name and description.
    pub(crate) fn species_save_source(&self, id: SpeciesId) -> Option<bibites_core::world::TemplateSource> {
        let mut source = self.species_template_source(id)?;
        source.meta.generation = self.world.species.get(id)?.generation_of_first_specimen;
        Some(source)
    }

    /// The template of species `id` as a spawnable source (`Species.template`).
    fn species_template_source(
        &self,
        id: SpeciesId,
    ) -> Option<bibites_core::world::TemplateSource> {
        let record = self.world.species.get(id)?;
        let name = format!("{}.bb8template", record.name());
        let meta = bibites_core::genome::GenomeMeta::default();
        Some(bibites_core::world::TemplateSource::new(
            name,
            record.template.genome.clone(),
            meta,
            record.template.brain.clone(),
        ))
    }
}

// ------------------------------------------------------------------------------ the Genes tab

/// One line of the Genes tab (`GeneLineInfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct GeneRow {
    /// `GeneSetting.Name`.
    pub name: &'static str,
    /// `GeneSetting.HelperText`.
    pub help: &'static str,
    /// The template's value (3 decimals).
    pub value: String,
    /// The signed difference from the compared species, `(+0.123)`; empty when they are equal.
    pub difference: String,
    /// `|difference| / maxValue / span` (0 when equal): the gene's contribution to speciation.
    pub contribution: f32,
    /// The black of the difference text, as an alpha (`UpdateDifferenceColor`); `None` keeps the
    /// prefab's (42, 42, 42).
    pub alpha: Option<f32>,
    /// The difference's tooltip text (empty when they are equal).
    pub breakdown: String,
}

/// `Mathf.Approximately`.
fn approximately(a: f32, b: f32) -> bool {
    (b - a).abs() < (1e-6 * a.abs().max(b.abs())).max(f32::MIN_POSITIVE * 8.0)
}

/// .NET's `{x:F<decimals>}` (the infinities and NaN spelt as the game prints them).
fn fixed(value: f32, decimals: usize) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else {
        format!("{value:.decimals$}")
    }
}

/// The value text of a gene line.
fn value_format() -> FloatValueFormat {
    FloatValueFormat {
        precision: 3,
        si: false,
        ..FloatValueFormat::default()
    }
}

/// The difference text of a gene line: `(+0.123)`.
fn difference_format() -> FloatValueFormat {
    FloatValueFormat {
        prefix: "(",
        units: ")",
        precision: 3,
        si: false,
        always_show_sign: true,
        ..FloatValueFormat::default()
    }
}

/// The distance to the parent as a share of the species span: `(+58.1%)`.
#[must_use]
pub fn parent_distance_text(distance: f32, span: f32) -> String {
    FloatValueFormat {
        prefix: "(",
        units: "%)",
        precision: 1,
        factor: 100.0,
        si: false,
        always_show_sign: true,
        ..FloatValueFormat::default()
    }
    .format(distance / span)
}

/// The gene distance to the compared species: `(0.581)`.
#[must_use]
pub fn compared_distance_text(distance: f32) -> String {
    FloatValueFormat {
        prefix: "(",
        units: ")",
        precision: 3,
        si: false,
        ..FloatValueFormat::default()
    }
    .format(distance)
}

/// The 35 lines for a template's `target` genes against the `compared` ones (both in the core's gene
/// order), given the species span (`GeneLineInfo.SetGene` and `UpdateDifferenceColor`).
#[must_use]
pub fn gene_rows(target: &[f32], compared: &[f32], span: f32) -> Vec<GeneRow> {
    let mut rows: Vec<GeneRow> = crate::gene_info_data::GENES
        .iter()
        .map(|info| {
            let index = Gene::from_name(info.gene).map(Gene::index);
            let value = index.and_then(|i| target.get(i)).copied().unwrap_or(0.0);
            let other = index.and_then(|i| compared.get(i)).copied().unwrap_or(value);
            let mut row = GeneRow {
                name: info.name,
                help: info.help,
                value: value_format().format(value).trim().to_owned(),
                difference: String::new(),
                contribution: 0.0,
                alpha: None,
                breakdown: String::new(),
            };
            if !approximately(value, other) {
                let difference = value - other;
                row.contribution = difference.abs() / info.max / span;
                row.difference = difference_format().format(difference).trim().to_owned();
                row.breakdown = format!(
                    "Flat difference : {}\nRelative difference : {}%\nSpan difference : {}\nContribution to speciation : {}%",
                    fixed(difference, 3),
                    fixed(100.0 * difference / other, 1),
                    fixed(difference / info.max, 3),
                    fixed(100.0 * row.contribution, 1),
                );
            }
            row
        })
        .collect();
    let largest = rows
        .iter()
        .map(|row| row.contribution)
        .fold(0.0_f32, f32::max);
    if largest > 0.0 {
        for row in &mut rows {
            let share = (row.contribution / largest).sqrt();
            row.alpha = Some(0.15 + (1.0 - 0.15) * share);
        }
    }
    rows
}

/// The Genes panel's rectangles (reference px, `x, y, w, h`).
#[derive(Clone, Debug, PartialEq)]
pub struct GenesLayout {
    /// The panel.
    pub panel: [f32; 4],
    /// The species' name.
    pub name: [f32; 4],
    /// "Parent species:".
    pub parent_title: [f32; 4],
    /// The parent button (or "None").
    pub parent_button: [f32; 4],
    /// The distance to the parent.
    pub parent_distance: [f32; 4],
    /// "Compared species:".
    pub compared_title: [f32; 4],
    /// The compared species dropdown.
    pub compared_dropdown: [f32; 4],
    /// The gene distance to the compared species.
    pub compared_distance: [f32; 4],
    /// The top of the first gene line.
    pub rows_top: f32,
}

/// The rows the Genes and the Brain tab put over their content (`header`): the species' name, the parent row and
/// the compared row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HeaderRects {
    /// The species' name.
    pub name: [f32; 4],
    /// "Parent species:".
    pub parent_title: [f32; 4],
    /// The parent button (or "None").
    pub parent_button: [f32; 4],
    /// The distance to the parent.
    pub parent_distance: [f32; 4],
    /// "Compared species:".
    pub compared_title: [f32; 4],
    /// The compared species dropdown.
    pub compared_dropdown: [f32; 4],
    /// The distance to the compared species.
    pub compared_distance: [f32; 4],
}

impl GenesLayout {
    /// The rows of the header.
    pub(crate) fn header(&self) -> HeaderRects {
        HeaderRects {
            name: self.name,
            parent_title: self.parent_title,
            parent_button: self.parent_button,
            parent_distance: self.parent_distance,
            compared_title: self.compared_title,
            compared_dropdown: self.compared_dropdown,
            compared_distance: self.compared_distance,
        }
    }
}

/// How many genes the tab lists.
pub const GENE_COUNT: usize = 35;
/// A gene line's height: one 14-point line.
const GENE_ROW: f32 = 16.41;
/// The distance between two lines: the height and the layout's spacing (5).
const GENE_PITCH: f32 = 21.41;
/// The Genes panel's layout over `inner` (its width less the padding of 10 and 5, [`layout::genes_inner_width`]), for a species with or
/// without a parent (the parent row is 4 shorter without the button): the prefab's, from the overlay's top-left corner.
#[must_use]
pub fn genes_layout(inner: f32, has_parent: bool) -> GenesLayout {
    let (px, py) = (OVERLAY.x, OVERLAY.y);
    // The header: 10 in from the left, 5 down; its rows are 3 apart.
    let (hx, hy) = (px + 10.0, py + 5.0);
    let lines = GENE_COUNT as f32 * GENE_ROW + (GENE_COUNT as f32 - 1.0) * (GENE_PITCH - GENE_ROW);
    let parent_row = if has_parent { 20.41 } else { 16.41 };
    let compared_at = 26.44 + parent_row + 3.0;
    let header = compared_at + 20.41;
    // The buttons share what the title (150), the distance (80) and the padding (5) leave.
    let button = inner - 5.0 - 150.0 - 80.0;
    GenesLayout {
        // 5 + header 70.26 (66.26 without a parent) + 5 + the holder's 5 padding + the lines + 10.
        panel: [px, py, inner + 15.0, 5.0 + header + 5.0 + 5.0 + lines + 10.0],
        parent_title: [hx, hy + 26.44, 150.0, parent_row],
        parent_button: [
            hx + 150.0,
            hy + 26.44,
            if has_parent { button } else { inner - 5.0 - 150.0 },
            parent_row,
        ],
        parent_distance: [hx + inner - 5.0 - 80.0, hy + 26.44, 80.0, 20.41],
        compared_title: [hx, hy + compared_at, 150.0, 20.41],
        compared_dropdown: [hx + 150.0, hy + compared_at, button, 20.41],
        compared_distance: [hx + inner - 5.0 - 80.0, hy + compared_at, 80.0, 20.41],
        name: [hx, hy, inner, 23.44],
        rows_top: hy + header + 5.0 + 5.0,
    }
}

// ----------------------------------------------------------------------------- drawing

/// The text of the Info panel's lines for `entry`: time of creation `{days}d {hours}:{minutes}`,
/// the generation, the members and the descendants.
#[must_use]
pub fn info_values(app: &App, entry: &SpeciesListEntry) -> [String; 4] {
    let seconds = entry.time_creation.max(0.0) as u64;
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    let int = |value: f32| {
        bibites_core::format::FloatValueFormat {
            is_int: true,
            ..bibites_core::format::FloatValueFormat::default()
        }
        .format(value)
    };
    [
        format!("{days}d {hours}:{minutes}"),
        int(entry.generation_of_first_specimen as f32),
        int(entry.count as f32),
        int(app.world.species.alive_descendant_count(entry.id) as f32),
    ]
}

/// Draws the panel when it is open.
pub fn emit(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    width: f32,
    height: f32,
) {
    if !app.panels.is_open(crate::ui::PanelKind::Lineage) {
        return;
    }
    let s = UiFrame::scaler(width, height);
    let draw = Drawer {
        s,
        ui,
        metrics,
        cursor: app.cursor,
    };
    // The panel covers the screen: nothing behind it gets a click.
    frame.hit(
        HIT_BACKDROP,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::Consume,
    );
    frame.rect(Vec2::ZERO, Vec2::new(width, height), PANEL_GREY);
    draw.top_bar(frame, app);
    draw.graph(frame, app);
    draw.lineage_settings(frame, app);
    let Some(entry) = app.species_info_entry() else {
        return;
    };
    // The tabs sit behind the panel; the selected and the hovered one rise.
    let tab = app.lineage_tab.min(TAB_POSITION);
    for index in (0..4).filter(|index| *index != tab) {
        draw.tab(frame, index, false);
    }
    draw.tab(frame, tab, true);
    let panel_width = draw.panel_width(app, entry, tab);
    draw.close_tab(frame, panel_width);
    match tab {
        TAB_INFO => draw.info(frame, app, entry),
        TAB_GENES => draw.genes(frame, app, entry),
        TAB_BRAIN => draw.brain(frame, app, entry),
        _ => draw.position(frame, app, entry),
    }
}

/// The name of species `id` in the panel's list (empty when it is gone).
fn name_of(app: &App, id: SpeciesId) -> String {
    app.lineage_species
        .iter()
        .find(|entry| entry.id == id)
        .map_or_else(String::new, |entry| entry.name.clone())
}

/// What the drawing needs: the scale, the sprites, the text metrics and the pointer.
pub(crate) struct Drawer<'a> {
    pub(crate) s: f32,
    pub(crate) ui: &'a UiAssets,
    pub(crate) metrics: Option<&'a crate::text_sdf::SdfFonts>,
    pub(crate) cursor: Vec2,
}

impl<'a> Drawer<'a> {
    /// A drawer for `scale` (`UiFrame::scaler`), with the UI sprites, the text metrics (estimated without them) and the pointer.
    pub(crate) fn new(
        s: f32,
        ui: &'a UiAssets,
        metrics: Option<&'a crate::text_sdf::SdfFonts>,
        cursor: Vec2,
    ) -> Self {
        Self {
            s,
            ui,
            metrics,
            cursor,
        }
    }
}

impl Drawer<'_> {
    pub(crate) fn at(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(x * self.s, y * self.s)
    }

    fn rect_pos(&self, rect: [f32; 4]) -> Vec2 {
        self.at(rect[0], rect[1])
    }

    fn rect_size(&self, rect: [f32; 4]) -> Vec2 {
        self.at(rect[2], rect[3])
    }

    fn hovered(&self, rect: [f32; 4]) -> bool {
        contains(
            [
                rect[0] * self.s,
                rect[1] * self.s,
                rect[2] * self.s,
                rect[3] * self.s,
            ],
            self.cursor,
        )
    }

    /// The width of `text` in the captured font `name` (a ` +bold` suffix asks for TMP's synthetic bold) at `px`.
    pub(crate) fn measure_named(&self, name: &str, text: &str, px: f32) -> f32 {
        self.metrics.map_or(text.chars().count() as f32 * px * 0.55, |fonts| {
            fonts.measure_named(Some(name), FontRole::Body, text, px)
        })
    }

    pub(crate) fn measure(&self, text: &str, px: f32, bold: bool) -> f32 {
        self.metrics
            .map_or(text.chars().count() as f32 * px * 0.56, |fonts| {
                fonts.measure_named(
                    None,
                    if bold { FontRole::Bold } else { FontRole::Body },
                    text,
                    px,
                )
            })
    }

    /// A 9-sliced sprite, or a flat rectangle when the sprite is missing.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn slice(
        &self,
        frame: &mut UiFrame,
        sprite: &str,
        border: f32,
        dest_scale: f32,
        rect: [f32; 4],
        tint: Rgba,
    ) {
        match self.ui.texture_of_alias(sprite) {
            Some(texture) => frame.draws.push(UiDraw::NineSlice {
                texture,
                border: [border; 4],
                dest_scale: dest_scale * self.s,
                pos: self.rect_pos(rect),
                size: self.rect_size(rect),
                tint,
            }),
            None => frame.rect(self.rect_pos(rect), self.rect_size(rect), tint),
        }
    }

    /// A sprite stretched over `rect`.
    pub(crate) fn icon(&self, frame: &mut UiFrame, sprite: &str, rect: [f32; 4], tint: Rgba) {
        if let Some(texture) = self.ui.texture_of_alias(sprite) {
            frame.draws.push(UiDraw::Image {
                texture,
                pos: self.rect_pos(rect),
                size: self.rect_size(rect),
                tint,
            });
        }
    }

    /// The size `text` is drawn at: `px`, or smaller so it fits `room` px (a name longer than its
    /// button shrinks instead of spilling over its neighbours).
    pub(crate) fn fitted(&self, text: &str, px: f32, bold: bool, room: f32) -> f32 {
        let width = self.measure(text, px, bold);
        if width > room && room > 0.0 {
            px * room / width
        } else {
            px
        }
    }

    /// A tooltip over `rect` (when the scene has one for `path`).
    fn tip(&self, frame: &mut UiFrame, rect: [f32; 4], path: &str) {
        if let Some(tip) = tooltip::find(&format!("{SCENE}{path}")) {
            self.push_tip(frame, rect, tip);
        }
    }

    fn push_tip(&self, frame: &mut UiFrame, rect: [f32; 4], tip: Tip) {
        frame.tips.push(TipRect {
            pos: self.rect_pos(rect),
            size: self.rect_size(rect),
            tip,
        });
    }

    /// The title, the counts and the four buttons of the top bar.
    fn top_bar(&self, frame: &mut UiFrame, app: &App) {
        let px = 24.0 * self.s;
        let recorded = format!("{} Recorded Species", app.lineage_species.len());
        let displayed = format!(" ({} displayed)", app.lineage_view.displayed().len());
        let title = "Ancestral Lineage Graph -";
        let widths = [
            self.measure_named(TITLE_FONT, title, px),
            self.measure(&recorded, px, true),
            self.measure(&displayed, px, true),
        ];
        let gap = 10.0 * self.s;
        let total = widths.iter().sum::<f32>() + 2.0 * gap;
        let top = 25.0 * self.s - px * 0.6;
        let mut x = 960.0 * self.s - total * 0.5;
        frame.draws.push(UiDraw::Text {
            font: Some(TITLE_FONT.to_owned()),
            text: title.to_owned(),
            pos: Vec2::new(x, top),
            px,
            color: WHITE,
            role: FontRole::Body,
        });
        x += widths[0] + gap;
        frame.bold(recorded, Vec2::new(x, top), px, WHITE);
        x += widths[1] + gap;
        let displayed_rect = [x / self.s, 12.0, widths[2] / self.s, 26.0];
        frame.bold(displayed, Vec2::new(x, top), px, WHITE);
        self.tip(frame, displayed_rect, "Top Bar/Middle/displayed");
        // Only the switch to the other view shows (`SpeciesPanel.treeOrMesh`: the Mesh button while the tree is
        // up, the Tree button while the mesh is), and the buttons keep to the right edge, so it takes the
        // second place.
        let sprites = ["mesh_icon", "tree_icon", "settings_icon", "exit_icon"];
        let paths = [
            "Top Bar/TopRight/MeshButton",
            "Top Bar/TopRight/TreeButton",
            "Top Bar/TopRight/SettingsButton",
            "Top Bar/TopRight/ExitButton",
        ];
        let mesh = app.lineage_view.mesh_mode;
        for (index, sprite) in sprites.iter().enumerate() {
            if index == 0 {
                continue;
            }
            // The second place holds the button of the view that is not showing.
            let (sprite, path) = match (index, mesh) {
                (1, false) => (&sprites[0], paths[0]),
                _ => (sprite, paths[index]),
            };
            let rect = bar_button(index);
            self.slice(frame, "base_icon", 5.0, 1.0 / 0.75, rect, WHITE);
            self.icon(frame, sprite, rect, WHITE);
            let action = match index {
                3 => UiAction::ClosePanel,
                2 => UiAction::ToggleLineageSettings,
                // `SelectPanel(tree)`: the Mesh button asks for the mesh, the Tree button for the tree.
                _ => UiAction::SelectLineagePanel(mesh),
            };
            frame.hit(
                HIT_BAR_BUTTON + index as u32,
                self.rect_pos(rect),
                self.rect_size(rect),
                action,
            );
            self.tip(frame, rect, path);
        }
        // RUSTibites Improved: the time tags' two buttons (and name field) between the title and the bar's own buttons (the bar's left holds the species
        // panel's tabs when a species is selected).
        crate::time_tags_ui::Surface { ui: self.ui, metrics: self.metrics, s: self.s }.controls(
            frame,
            app,
            crate::time_tags::TagPanel::Lineage,
            [(x + widths[2]) / self.s + 30.0, 5.0, bar_button(1)[0] - 10.0, 40.0],
            22.0,
        );
        let _ = HIT_BAR;
    }

    /// The graph view: the lineage tree in its viewport with the time axis and the scroll bars
    /// (`lineage_graph.rs`).
    fn graph(&self, frame: &mut UiFrame, app: &App) {
        self.lineage_graph(frame, app);
    }

    /// One tab: its frame, its icon, its hit and its tooltip.
    fn tab(&self, frame: &mut UiFrame, index: u8, selected: bool) {
        let natural = tab_rect(index, false);
        let raised = selected || self.hovered(natural);
        let rect = tab_rect(index, raised);
        if index == TAB_GENES {
            // The Genes tab's image is a finished button (`Genes_button_selected`).
            self.icon(frame, "Genes_button_selected", rect, WHITE);
        } else {
            self.slice(frame, "base_icon", 5.0, 1.0, rect, WHITE);
            let icons = ["stats_icon", "", "brain_icon", "compass"];
            self.icon(frame, icons[usize::from(index)], rect, WHITE);
        }
        // The element (40 x 50) takes the click and the tooltip, at its resting place.
        let element = [natural[0], natural[1] - 10.0, 40.0, 50.0];
        frame.hit(
            HIT_TAB + u32::from(index),
            self.rect_pos(element),
            self.rect_size(element),
            UiAction::SetLineageTab(u32::from(index)),
        );
        let element_paths = [
            "Overlay/SpeciesPanels/Species Tabs/Tab Element",
            "Overlay/SpeciesPanels/Species Tabs/Tab Element (1)",
            "Overlay/SpeciesPanels/Species Tabs/Tab Element (3)",
            "Overlay/SpeciesPanels/Species Tabs/Tab Element (4)",
        ];
        self.tip(frame, element, element_paths[usize::from(index)]);
    }

    /// The close tab (`CloseSpeciesPanels`) at the right end of a panel `panel_width` wide.
    fn close_tab(&self, frame: &mut UiFrame, panel_width: f32) {
        let rect = close_tab_rect(panel_width);
        self.slice(frame, "base_icon", 5.0, 1.0, rect, WHITE);
        self.icon(frame, "exit_icon", rect, WHITE);
        // The element (40 x 50) is centred 10 above the panel, like the other tabs'.
        let element = [rect[0], tab_rect(0, false)[1] - 10.0, 40.0, 50.0];
        frame.hit(
            HIT_CLOSE_TAB,
            self.rect_pos(element),
            self.rect_size(element),
            UiAction::SpeciesClose,
        );
        self.tip(
            frame,
            element,
            "Overlay/SpeciesPanels/Species Tabs/Tab Element (2)",
        );
    }

    /// The width of the panel a tab shows: the close tab sits at its right end.
    fn panel_width(&self, app: &App, entry: &SpeciesListEntry, tab: u8) -> f32 {
        match tab {
            TAB_INFO => self.info_layout(app, entry).panel[2],
            TAB_GENES => self.genes_layout_for(app, entry).panel[2],
            TAB_BRAIN => brain::geometry(app.brain_holder_width, 300.0, 0.0, 0.0).panel[2],
            _ => position::layout(1, 0.0).panel[2],
        }
    }

    /// The Info panel's layout for `entry` (the panel is as wide as its texts ask).
    fn info_layout(&self, app: &App, entry: &SpeciesListEntry) -> InfoLayout {
        let root = name_of(app, entry.root);
        let parent = entry.parent.map(|parent| name_of(app, parent));
        let editing = app
            .species_ui
            .name
            .as_deref()
            .map(|text| !SpeciesRegistry::is_valid_species_name(text));
        let texts = layout::InfoTexts {
            name: &entry.name,
            id: entry.id.value().to_string(),
            root: &root,
            parent: parent.as_deref(),
            values: info_values(app, entry),
            editing,
        };
        layout::info_layout(&texts, &crate::stats_states::text_measure(self.metrics))
    }

    /// The Genes panel's layout for `entry` (the panel is as wide as its widest row asks).
    fn genes_layout_for(&self, app: &App, entry: &SpeciesListEntry) -> GenesLayout {
        let span = app.world.species.current_used_span(&app.world.settings);
        let record = app.world.species.get(entry.id);
        let distance = |other: &bibites_core::species::SpeciesRecord| {
            record.map_or(0.0, |record| {
                record.template.gene_distance(&other.template.genome)
            })
        };
        let parent_distance = entry
            .parent
            .and_then(|parent| app.world.species.get(parent))
            .map(|parent| parent_distance_text(distance(parent), span))
            .unwrap_or_default();
        let compared = app.species_compared.unwrap_or(entry.id);
        let compared_distance = app
            .world
            .species
            .get(compared)
            .map(|other| compared_distance_text(distance(other)))
            .unwrap_or_default();
        let parent = entry.parent.map(|parent| name_of(app, parent));
        let compared_name = name_of(app, compared);
        let texts = layout::GenesTexts {
            name: &entry.name,
            parent: parent.as_deref(),
            parent_distance: &parent_distance,
            compared: &compared_name,
            compared_distance: &compared_distance,
        };
        let inner =
            layout::genes_inner_width(&texts, &crate::stats_states::text_measure(self.metrics));
        genes_layout(inner, entry.parent.is_some())
    }

    /// The name, the parent row and the compared row of a tab's `header`: the species' name, its parent as a
    /// button with how far it is as a percentage of the species span, and a dropdown of the species of the graph
    /// with the distance to the one picked. `distance` is the genetic distance between two templates (the genes'
    /// in the Genes tab, the brains' in the Brain tab) and `scene` the header's path for the distances' tooltips.
    pub(crate) fn species_header(
        &self,
        frame: &mut UiFrame,
        app: &App,
        entry: &SpeciesListEntry,
        layout: &HeaderRects,
        distance: &dyn Fn(
            &bibites_core::species::SpeciesRecord,
            &bibites_core::species::SpeciesRecord,
        ) -> f32,
        scene: &str,
    ) {
        let s = self.s;
        let ink_at = |rect: [f32; 4], dx: f32| self.at(rect[0] + dx, rect[1] + 1.0);
        // The species' name.
        frame.bold(
            entry.name.clone(),
            self.at(layout.name[0], layout.name[1] + 1.0),
            20.0 * s,
            INK,
        );
        let green: Rgba = [2, 147, 2, 255];
        let span = app.world.species.current_used_span(&app.world.settings);
        let record = app.world.species.get(entry.id);
        // The parent row.
        semibold(
            frame,
            "Parent species:",
            ink_at(layout.parent_title, 0.0),
            14.0 * s,
            INK,
        );
        match entry.parent {
            Some(parent) => {
                self.reference_button(
                    frame,
                    layout.parent_button,
                    &name_of(app, parent),
                    HIT_GENES_PARENT,
                    UiAction::SpeciesParent,
                );
                if let (Some(record), Some(parent_record)) = (record, app.world.species.get(parent))
                {
                    self.right_text(
                        frame,
                        layout.parent_distance,
                        &parent_distance_text(distance(record, parent_record), span),
                        16.0,
                        green,
                    );
                    self.tip(
                        frame,
                        layout.parent_distance,
                        &format!("{scene}/parentRow/distToParent"),
                    );
                }
            }
            None => frame.bold("None", ink_at(layout.parent_button, 0.0), 14.0 * s, INK),
        }
        // The compared row: a dropdown of the species of the graph and the distance to the one picked.
        semibold(
            frame,
            "Compared species:",
            ink_at(layout.compared_title, 0.0),
            14.0 * s,
            INK,
        );
        let box_rect = layout.compared_dropdown;
        self.slice(frame, "base_icon", 5.0, 0.5, box_rect, WHITE);
        let compared = app.species_compared.unwrap_or(entry.id);
        let label = name_of(app, compared);
        let label_room = (box_rect[2] - 30.0) * s;
        let label_px = self.fitted(&label, 14.0 * s, true, label_room);
        let label_w = self.measure(&label, label_px, true);
        frame.bold(
            label,
            Vec2::new(
                // The label sits between the dropdown's padding (10 and 20): its middle is half the width less 5.
                (box_rect[0] + box_rect[2] * 0.5 - 5.0) * s - label_w * 0.5,
                (box_rect[1] + 1.5) * s,
            ),
            label_px,
            INK,
        );
        self.icon(
            frame,
            "DropdownArrow",
            [
                box_rect[0] + box_rect[2] - 20.0,
                box_rect[1] + 0.2,
                20.0,
                20.0,
            ],
            WHITE,
        );
        frame.hit(
            HIT_COMPARED,
            self.rect_pos(box_rect),
            self.rect_size(box_rect),
            UiAction::DropdownOpen(DropdownTarget::SpeciesCompared),
        );
        if let (Some(record), Some(other)) = (record, app.world.species.get(compared)) {
            self.right_text(
                frame,
                layout.compared_distance,
                &compared_distance_text(distance(record, other)),
                16.0,
                green,
            );
            self.tip(
                frame,
                layout.compared_distance,
                &format!("{scene}/comparedRow/distToCompared"),
            );
        }
    }

    /// The Genes tab: the header with the parent and the compared species, then the 35 genes.
    fn genes(&self, frame: &mut UiFrame, app: &App, entry: &SpeciesListEntry) {
        let layout = self.genes_layout_for(app, entry);
        let s = self.s;
        self.slice(frame, "base_icon", 5.0, 1.0, layout.panel, FRAME_GREY);
        frame.hit(
            HIT_PANEL,
            self.rect_pos(layout.panel),
            self.rect_size(layout.panel),
            UiAction::Consume,
        );
        let span = app.world.species.current_used_span(&app.world.settings);
        let record = app.world.species.get(entry.id);
        let compared = app.species_compared.unwrap_or(entry.id);
        let compared_record = app.world.species.get(compared);
        self.species_header(
            frame,
            app,
            entry,
            &layout.header(),
            &|target, other| target.template.gene_distance(&other.template.genome),
            "Overlay/SpeciesPanels/Species Genes Panel/header",
        );
        // The genes.
        let Some(record) = record else {
            return;
        };
        let target = record.template.genome.values();
        let other = compared_record.map_or(target, |other| other.template.genome.values());
        let rows = gene_rows(target.as_slice(), other.as_slice(), span);
        // The gene line: the name takes what the 90 value and the 100 difference leave.
        let (hx, value_w, diff_w) = (layout.name[0], 90.0, 100.0);
        let name_w = layout.name[2] - value_w - diff_w;
        for (index, row) in rows.iter().enumerate() {
            let top = layout.rows_top + index as f32 * GENE_PITCH;
            // The name (LiberationSans, which draws its white dark and heavy in the game).
            frame.draws.push(UiDraw::Text {
                font: Some("LiberationSans SDF".to_owned()),
                text: format!("{}: ", row.name),
                pos: self.at(hx, top),
                px: 14.0 * s,
                color: BLACK,
                role: FontRole::Body,
            });
            self.push_tip(
                frame,
                [hx, top, name_w, GENE_ROW],
                Tip {
                    path: "",
                    title: row.name,
                    content: row.help,
                    ui: true,
                    instant: false,
                },
            );
            // The value, flush right in its 90 (`Roboto-Regular SDF`, bold style).
            self.right_semibold(
                frame,
                [hx + name_w, top, value_w, GENE_ROW],
                &row.value,
                14.0,
                BLACK,
            );
            // The difference, centred in its 100, black with the alpha of its weight.
            if !row.difference.is_empty() {
                let color = row.alpha.map_or([42, 42, 42, 255], |alpha| {
                    [0, 0, 0, (alpha * 255.0).round() as u8]
                });
                let width = self.measure(&row.difference, 14.0 * s, false);
                frame.text(
                    row.difference.clone(),
                    Vec2::new(
                        (hx + name_w + value_w + diff_w * 0.5) * s - width * 0.5,
                        top * s,
                    ),
                    14.0 * s,
                    color,
                );
                self.push_tip(
                    frame,
                    [hx + name_w + value_w, top, diff_w, GENE_ROW],
                    Tip {
                        path: "",
                        title: "Difference from target",
                        content: tooltip::intern(row.breakdown.clone()),
                        ui: true,
                        instant: false,
                    },
                );
            }
        }
    }

    /// `text` flush right in `rect`, vertically centred (`px` reference px, Roboto-Bold when it is
    /// 16, Roboto-Regular otherwise).
    fn right_text(&self, frame: &mut UiFrame, rect: [f32; 4], text: &str, px: f32, color: Rgba) {
        let bold = px >= 16.0;
        let size = px * self.s;
        let width = self.measure(text, size, bold);
        let line = size * 1.1719;
        let at = Vec2::new(
            (rect[0] + rect[2]) * self.s - width,
            (rect[1] + rect[3] * 0.5) * self.s - line * 0.5,
        );
        if bold {
            frame.bold(text.to_owned(), at, size, color);
        } else {
            frame.text(text.to_owned(), at, size, color);
        }
    }

    /// `text` flush right in `rect`, vertically centred, in `Roboto-Regular SDF` bold without the extra advance (the gene values: a live capture
    /// gives a "0.816" the regular width, ink 320 to 355 in its 90 wide cell, with the bold weight's strokes).
    fn right_semibold(&self, frame: &mut UiFrame, rect: [f32; 4], text: &str, px: f32, color: Rgba) {
        let size = px * self.s;
        let font = format!("Roboto-Regular SDF{}", crate::text_sdf::BOLD_FLAT_SUFFIX);
        let width = self.measure_named(&font, text, size);
        let line = size * 1.1719;
        let at = Vec2::new(
            (rect[0] + rect[2]) * self.s - width,
            (rect[1] + rect[3] * 0.5) * self.s - line * 0.5,
        );
        frame.draws.push(UiDraw::Text {
            font: Some(font),
            text: text.to_owned(),
            pos: at,
            px: size,
            color,
            role: FontRole::Body,
        });
    }

    /// The Info panel.
    fn info(&self, frame: &mut UiFrame, app: &App, entry: &SpeciesListEntry) {
        let layout = self.info_layout(app, entry);
        self.slice(frame, "base_icon", 5.0, 1.0, layout.panel, FRAME_GREY);
        frame.hit(
            HIT_PANEL,
            self.rect_pos(layout.panel),
            self.rect_size(layout.panel),
            UiAction::Consume,
        );
        let s = self.s;
        // The name: a button with the rename icon, or the text field while it is edited.
        let editing = app.species_ui.name.as_deref();
        let name_px = 18.0 * s;
        match editing {
            Some(text) => {
                let field = layout.name_field;
                self.slice(frame, "InputFieldBackground", 10.0, 0.5, field, WHITE);
                let line_top = field[1] + (field[3] - 14.0 * 1.17) * 0.5;
                frame.text(
                    text.to_owned(),
                    self.at(field[0] + 5.0, line_top),
                    14.0 * s,
                    FIELD_INK,
                );
                let before: String = text.chars().take(app.edit_caret.pos.min(text.chars().count())).collect();
                let caret_x = field[0] + 5.0 + self.measure(&before, 14.0 * s, false) / s;
                frame.rect(
                    self.at(caret_x, field[1] + 4.0),
                    self.at(1.0, 14.0),
                    FIELD_INK,
                );
                if !SpeciesRegistry::is_valid_species_name(text) {
                    self.icon(frame, "bug_icon", layout.warning_icon, [204, 0, 0, 255]);
                    self.tip(
                        frame,
                        layout.warning_icon,
                        "Overlay/SpeciesPanels/Species Info Panel/Top Section/Preview Section/SpeciesNameAndID/NameRow/WarningIcon",
                    );
                }
                frame.hit(
                    HIT_NAME,
                    self.rect_pos(field),
                    self.rect_size(field),
                    UiAction::Consume,
                );
            }
            None => {
                let name_px =
                    self.fitted(&entry.name, name_px, true, (layout.name_row[2] - 32.0) * s);
                frame.bold(
                    entry.name.clone(),
                    self.at(layout.name_row[0], layout.name_row[1]),
                    name_px,
                    INK,
                );
                self.icon(frame, "rename_icon", layout.rename_icon, WHITE);
                frame.hit(
                    HIT_NAME,
                    self.rect_pos(layout.name_row),
                    self.rect_size(layout.name_row),
                    UiAction::SpeciesNameEdit,
                );
            }
        }
        // "Species ID:" and the id.
        let id_px = 14.0 * s;
        semibold(
            frame,
            "Species ID:",
            self.at(layout.id_row[0], layout.id_row[1]),
            id_px,
            INK,
        );
        semibold(
            frame,
            entry.id.value().to_string(),
            self.at(layout.id_value[0], layout.id_value[1]),
            id_px,
            INK,
        );
        // The preview frame, with the template's procedural bibite in it.
        self.slice(frame, "Long_UI", 70.0, 0.1, layout.preview, WHITE);
        if let Some(texture) = app.species_picture(
            entry.id,
            crate::preview::INFO_PANEL,
            [layout.preview[2], layout.preview[3]],
            self.s,
        ) {
            frame.draws.push(UiDraw::Pixels {
                texture,
                pos: self.rect_pos(layout.preview),
                size: self.rect_size(layout.preview),
            });
        }
        // Root and parent.
        semibold(
            frame,
            "Root species:",
            self.at(layout.root_title[0], layout.root_title[1]),
            id_px,
            INK,
        );
        semibold(
            frame,
            "Parent species:",
            self.at(layout.parent_title[0], layout.parent_title[1]),
            id_px,
            INK,
        );
        let name_of = |id: SpeciesId| {
            app.lineage_species
                .iter()
                .find(|entry| entry.id == id)
                .map_or_else(String::new, |entry| entry.name.clone())
        };
        self.reference_button(
            frame,
            layout.root_button,
            &name_of(entry.root),
            HIT_ROOT,
            UiAction::SpeciesRoot,
        );
        match entry.parent {
            Some(parent) => self.reference_button(
                frame,
                layout.parent_button,
                &name_of(parent),
                HIT_PARENT,
                UiAction::SpeciesParent,
            ),
            None => {
                let width = self.measure("None", id_px, true);
                frame.bold(
                    "None",
                    Vec2::new(
                        (layout.parent_button[0] + layout.parent_button[2] * 0.5) * s - width * 0.5,
                        (layout.parent_button[1] + 1.0) * s,
                    ),
                    id_px,
                    INK,
                );
            }
        }
        // The six buttons.
        self.buttons(frame, app, entry, &layout);
        // The body.
        let labels = [
            "Time of creation: ",
            "Gen at creation: ",
            "Number of alive members:",
            "Number of alive descendants:",
        ];
        let line_paths = [
            None,
            None,
            Some("Overlay/SpeciesPanels/Species Info Panel/Body/TextBody/Line (3)/Text (TMP)"),
            Some("Overlay/SpeciesPanels/Species Info Panel/Body/TextBody/Line (2)/Text (TMP)"),
        ];
        let values = info_values(app, entry);
        for (index, rect) in layout.lines.iter().enumerate() {
            let top = rect[1] + 1.0;
            frame.bold(labels[index], self.at(rect[0], top), 13.0 * s, BLACK);
            let width = self.measure(&values[index], 14.0 * s, true);
            frame.bold(
                values[index].clone(),
                Vec2::new((rect[0] + rect[2]) * s - width, top * s),
                14.0 * s,
                BLACK,
            );
            if let Some(path) = line_paths[index] {
                let label_w = self.measure(labels[index], 13.0 * s, true) / s;
                self.tip(frame, [rect[0], rect[1], label_w, rect[3]], path);
            }
        }
        // The notes.
        frame.bold(
            "Notes:",
            self.at(layout.notes_label[0], layout.notes_label[1]),
            12.0 * s,
            BLACK,
        );
        self.notes(frame, app, entry, &layout);
    }

    /// The root or parent species button: `base_icon` (ppu multiplier 2) with the name.
    fn reference_button(
        &self,
        frame: &mut UiFrame,
        rect: [f32; 4],
        name: &str,
        id: u32,
        action: UiAction,
    ) {
        self.slice(frame, "base_icon", 5.0, 0.5, rect, WHITE);
        let px = self.fitted(name, 14.0 * self.s, true, (rect[2] - 20.0) * self.s);
        // `Center` / `Middle` in the button.
        let width = self.measure(name, px, true);
        frame.bold(
            name.to_owned(),
            Vec2::new(
                (rect[0] + rect[2] * 0.5) * self.s - width * 0.5,
                (rect[1] + 1.5) * self.s,
            ),
            px,
            INK,
        );
        frame.hit(id, self.rect_pos(rect), self.rect_size(rect), action);
    }

    /// Favourite toggle, save, place, select, prune and edit.
    fn buttons(
        &self,
        frame: &mut UiFrame,
        app: &App,
        entry: &SpeciesListEntry,
        layout: &InfoLayout,
    ) {
        let has_members = entry.count > 0;
        let enabled = [true, true, true, has_members, app.species_can_prune(), true];
        let icons = [
            "output_want2heal",
            "save_icon",
            "clone_tool",
            "feedback_icon",
            "kill_bibite_tool",
            "credits_icon",
        ];
        let actions = [
            UiAction::Invoke(ControlCommand::SpeciesToggleFavorite),
            UiAction::SpeciesSave,
            UiAction::SpeciesPlace,
            UiAction::Invoke(ControlCommand::SpeciesSelectRandomMember),
            UiAction::SpeciesPrune,
            UiAction::SpeciesEdit,
        ];
        let paths = [
            "Overlay/SpeciesPanels/Species Info Panel/Top Section/Buttons Holder/Favorite Toggle",
            "Overlay/SpeciesPanels/Species Info Panel/Top Section/Buttons Holder/SaveSpeciesButton/Text",
            "Overlay/SpeciesPanels/Species Info Panel/Top Section/Buttons Holder/PlaceSpeciesButton/Text",
            "Overlay/SpeciesPanels/Species Info Panel/Top Section/Buttons Holder/SelectButton",
            "Overlay/SpeciesPanels/Species Info Panel/Top Section/Buttons Holder/PruneButton",
            "Overlay/SpeciesPanels/Species Info Panel/Top Section/Buttons Holder/Edit",
        ];
        for (index, rect) in layout.buttons.iter().enumerate() {
            let tint = if enabled[index] { WHITE } else { DISABLED };
            if index == 0 {
                // The toggle has no frame: its `Background` is a grey heart (scaled 1.5) and its `Checkmark`, a child of it (so as
                // big), an orange one while it is on. `flatMat` paints the sprite's shape in the `Image`'s colour.
                let grow = rect[2] * 0.25;
                let big = [
                    rect[0] - grow,
                    rect[1] - grow,
                    rect[2] + 2.0 * grow,
                    rect[3] + 2.0 * grow,
                ];
                self.icon(frame, "output_want2heal#flat", big, [125, 125, 125, 255]);
                if entry.favorite {
                    self.icon(frame, "output_want2heal#flat", big, [255, 53, 0, 255]);
                }
            } else {
                // `ColorTint` only reaches the button's own frame: the icon (a child `Image`) keeps its colour while the button is disabled.
                self.slice(frame, "base_icon", 5.0, 1.0, *rect, tint);
                self.icon(frame, icons[index], *rect, WHITE);
            }
            if enabled[index] {
                frame.hit(
                    HIT_BUTTON + index as u32,
                    self.rect_pos(*rect),
                    self.rect_size(*rect),
                    actions[index],
                );
            }
            // The select button's tooltip depends on the members (`selectTooltip.UpdateText`).
            if index == 3 {
                self.push_tip(
                    frame,
                    *rect,
                    Tip {
                        path: "",
                        title: "Select Bibite of Species",
                        content: if entry.count > 1 {
                            ""
                        } else {
                            "There are no bibites of this species"
                        },
                        ui: true,
                        instant: false,
                    },
                );
            } else {
                self.tip(frame, *rect, paths[index]);
            }
        }
    }

    /// The notes field: the text wrapped to the field, the last lines that fit.
    fn notes(&self, frame: &mut UiFrame, app: &App, entry: &SpeciesListEntry, layout: &InfoLayout) {
        let field = layout.notes;
        self.slice(frame, "InputFieldBackground", 10.0, 0.5, field, WHITE);
        let px = 12.0 * self.s;
        let line_h = 12.0 * 1.1719;
        let room = (field[2] - 20.0) * self.s;
        let mut lines: Vec<String> = Vec::new();
        // The character where each line starts (the caret is a character index of the whole text).
        let mut starts: Vec<usize> = Vec::new();
        let mut paragraph_start = 0_usize;
        for paragraph in entry.description.split('\n') {
            let mut line = String::new();
            let mut line_start = paragraph_start;
            let mut word_start = paragraph_start;
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if !line.is_empty() && self.measure(&candidate, px, false) > room {
                    lines.push(std::mem::take(&mut line));
                    starts.push(line_start);
                    line_start = word_start;
                    line = word.to_owned();
                } else {
                    line = candidate;
                }
                word_start += word.chars().count() + 1;
            }
            lines.push(line);
            starts.push(line_start);
            paragraph_start += paragraph.chars().count() + 1;
        }
        // The line and column the caret is on: the last line that starts at or before it.
        let caret_at = app.edit_caret.pos.min(entry.description.chars().count());
        let caret_line = starts.iter().rposition(|start| *start <= caret_at).unwrap_or(0);
        let caret_column = caret_at - starts.get(caret_line).copied().unwrap_or(0);
        let capacity = ((field[3] - 13.0) / line_h).floor().max(1.0) as usize;
        let skip = lines.len().saturating_sub(capacity);
        let focused = app.species_ui.notes.is_some();
        if lines.iter().all(String::is_empty) && !focused {
            // The placeholder is italic (`Roboto-Regular SDF`, `FontStyles.Italic`) at half the text's alpha.
            frame.draws.push(UiDraw::Text {
                font: Some(format!("Roboto-Regular SDF{}", crate::text_sdf::ITALIC_SUFFIX)),
                text: "Enter text...".to_owned(),
                pos: self.at(field[0] + 10.0, field[1] + 6.5),
                px,
                color: [50, 50, 50, 128],
                role: FontRole::Body,
            });
        }
        for (row, line) in lines.iter().skip(skip).enumerate() {
            let y = field[1] + 6.5 + row as f32 * line_h;
            frame.text(line.clone(), self.at(field[0] + 10.0, y), px, FIELD_INK);
            if focused && skip + row == caret_line {
                let before: String = line.chars().take(caret_column).collect();
                let caret = field[0] + 10.0 + self.measure(&before, px, false) / self.s;
                frame.rect(self.at(caret, y), self.at(1.0, 12.0), FIELD_INK);
            }
        }
        frame.hit(
            HIT_NOTES,
            self.rect_pos(field),
            self.rect_size(field),
            UiAction::SpeciesNotes,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;

    /// An app with a world that has had its species recorded, the panel open.
    fn app_with_species() -> App {
        let loaded = LoadedWorld::load(None, 41, 10, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        // The tests do not depend on the player's own options: extinct lineages show.
        app.lineage_options = Some(crate::lineage_tree::TreeSettings {
            hide_extinct: false,
            ..crate::lineage_tree::TreeSettings::default()
        });
        app.world.run(2000);
        app.refresh_lineage();
        app.panels.open(crate::ui::PanelKind::Lineage);
        app
    }

    fn frame_of(app: &App) -> UiFrame {
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, app, &UiAssets::empty(), None, 1920.0, 1080.0);
        frame
    }

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

    /// A root species and an extinct child of it, both templates (no members). The child lived for two logs
    /// and has been gone from the last two (`totalIndexOfDisappearance > 0`, two present points: within the
    /// prune point limit), which is what the Prune button asks for.
    fn family(app: &mut App) -> (SpeciesId, SpeciesId) {
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let root = app.world.species.add_template_species(
            "Zzroot zzspecies",
            "root notes",
            &genome,
            &brain,
        );
        let child =
            app.world
                .species
                .add_template_species("Zzchild zzspecies", "", &genome, &brain);
        {
            let record = app.world.species.get_mut(child).expect("child");
            record.parent = Some(root);
            record.root = root;
        }
        app.world
            .species
            .get_mut(root)
            .expect("root")
            .children
            .push(child);
        {
            use bibites_core::species_log::SpeciesDataPoint;
            let log = &mut app.world.species.get_mut(root).expect("root").stats.log;
            for _ in 0..8 {
                log.push(SpeciesDataPoint::from_counts(1, 5.0));
            }
            let log = &mut app.world.species.get_mut(child).expect("child").stats.log;
            for point in [
                SpeciesDataPoint::from_counts(1, 5.0),
                SpeciesDataPoint::from_counts(1, 5.0),
                SpeciesDataPoint::default(),
                SpeciesDataPoint::default(),
            ] {
                log.push(point);
            }
        }
        app.refresh_lineage();
        (root, child)
    }

    fn key(app: &mut App, input: FrameInput) {
        app.handle_input(&FrameInput { dt: 0.016, ..input });
    }

    #[test]
    fn clicking_the_name_opens_a_text_field_and_enter_renames_the_species() {
        let mut app = app_with_species();
        let (root, _) = family(&mut app);
        app.select_species_info(Some(root));
        app.apply_ui_actions(&[UiAction::SpeciesNameEdit]);
        assert_eq!(app.species_ui.name.as_deref(), Some("Zzroot zzspecies"));
        // Keys edit the field and are not hotkeys meanwhile (`R` would select a random bibite).
        for _ in 0.."zzspecies".len() {
            key(
                &mut app,
                FrameInput {
                    backspace: true,
                    ..FrameInput::default()
                },
            );
        }
        app.typed = "orca".to_owned();
        key(
            &mut app,
            FrameInput {
                select_random_bibite: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.species_ui.name.as_deref(), Some("Zzroot orca"));
        assert!(app.selected.is_none(), "the field blocks the hotkeys");
        key(
            &mut app,
            FrameInput {
                enter: true,
                ..FrameInput::default()
            },
        );
        assert!(app.species_ui.name.is_none());
        assert_eq!(app.world.species.get(root).unwrap().name(), "Zzroot orca");
        assert_eq!(app.species_info_entry().unwrap().name, "Zzroot orca");
        // A name of the wrong form is dropped when the field ends, and Escape drops it too.
        app.apply_ui_actions(&[UiAction::SpeciesNameEdit]);
        app.species_ui.name = Some("orca".to_owned());
        key(
            &mut app,
            FrameInput {
                enter: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.world.species.get(root).unwrap().name(), "Zzroot orca");
        app.apply_ui_actions(&[UiAction::SpeciesNameEdit]);
        app.species_ui.name = Some("Valid name".to_owned());
        key(
            &mut app,
            FrameInput {
                quit: true,
                ..FrameInput::default()
            },
        );
        assert!(app.species_ui.name.is_none());
        assert_eq!(app.world.species.get(root).unwrap().name(), "Zzroot orca");
        assert_eq!(app.species_info, Some(root), "Escape ended the field only");
        // Pressing another control ends the field and applies a valid name first.
        app.apply_ui_actions(&[UiAction::SpeciesNameEdit]);
        app.species_ui.name = Some("Valid name".to_owned());
        app.apply_ui_actions(&[UiAction::SetLineageTab(2)]);
        assert!(app.species_ui.name.is_none());
        assert_eq!(app.world.species.get(root).unwrap().name(), "Valid name");
    }

    #[test]
    fn the_notes_field_saves_as_you_type_and_escape_restores_what_it_started_with() {
        let mut app = app_with_species();
        let (root, _) = family(&mut app);
        app.select_species_info(Some(root));
        app.apply_ui_actions(&[UiAction::SpeciesNotes]);
        assert_eq!(app.species_ui.notes.as_deref(), Some("root notes"));
        app.typed = " more".to_owned();
        key(&mut app, FrameInput::default());
        key(
            &mut app,
            FrameInput {
                enter: true,
                ..FrameInput::default()
            },
        );
        app.typed = "line".to_owned();
        key(&mut app, FrameInput::default());
        let notes = |app: &App| app.world.species.get(root).unwrap().description.clone();
        assert_eq!(notes(&app), "root notes more\nline");
        assert_eq!(app.species_info_entry().unwrap().description, notes(&app));
        key(
            &mut app,
            FrameInput {
                backspace: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(notes(&app), "root notes more\nlin");
        // Escape: the field goes back to its text at the start of the edit.
        key(
            &mut app,
            FrameInput {
                quit: true,
                ..FrameInput::default()
            },
        );
        assert!(app.species_ui.notes.is_none());
        assert_eq!(notes(&app), "root notes");
        // Typed text outside a field goes nowhere.
        app.typed = "xyz".to_owned();
        key(&mut app, FrameInput::default());
        assert_eq!(notes(&app), "root notes");
    }

    #[test]
    fn the_favorite_toggle_and_the_prune_button_follow_the_games_rules() {
        let mut app = app_with_species();
        let (root, child) = family(&mut app);
        // A root is always kept: no pruning. An extinct child that is not a favourite can be.
        app.select_species_info(Some(root));
        assert!(!app.species_can_prune());
        app.select_species_info(Some(child));
        assert!(app.species_can_prune());
        let prune_hit = |app: &App| {
            frame_of(app)
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::SpeciesPrune)
        };
        assert!(prune_hit(&app), "the enabled button takes clicks");
        // The favourite toggle flips the flag; the handler leaves Prune disabled (a game quirk).
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SpeciesToggleFavorite)]);
        assert!(app.species_info_entry().unwrap().favorite);
        assert!(!app.species_can_prune());
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SpeciesToggleFavorite)]);
        assert!(!app.species_info_entry().unwrap().favorite);
        assert!(!app.species_can_prune(), "until the panel is filled again");
        assert!(!prune_hit(&app), "a disabled button takes no clicks");
        app.select_species_info(Some(child));
        assert!(app.species_can_prune());
    }

    #[test]
    fn pruning_asks_first_merges_into_the_parent_and_can_skip_the_question() {
        use crate::warnings::ActionWarning;
        let mut app = app_with_species();
        let (root, child) = family(&mut app);
        app.select_species_info(Some(child));
        app.apply_ui_actions(&[UiAction::SpeciesPrune]);
        assert_eq!(app.confirm_dialog, Some(ConfirmKind::PruneSpecies(child)));
        assert_eq!(
            ConfirmKind::PruneSpecies(child).warning(),
            Some(ActionWarning::PruneSpecies)
        );
        // Cancel keeps it.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Cancel)]);
        assert!(app.confirm_dialog.is_none());
        assert!(app.world.species.get(child).is_some());
        // Yes, with "do not show again" ticked, prunes it and selects the parent it merged into.
        app.apply_ui_actions(&[UiAction::SpeciesPrune]);
        app.confirm_do_not_show = true;
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Confirm)]);
        assert!(app.world.species.get(child).is_none());
        assert_eq!(app.species_info, Some(root));
        assert!(app.lineage_species.iter().all(|entry| entry.id != child));
        assert!(app.warning_off(ActionWarning::PruneSpecies));
        // The next one goes at once.
        let (_, second) = family(&mut app);
        app.select_species_info(Some(second));
        app.apply_ui_actions(&[UiAction::SpeciesPrune]);
        assert!(app.confirm_dialog.is_none());
        assert!(app.world.species.get(second).is_none());
        // The root cannot be pruned.
        app.select_species_info(Some(root));
        app.apply_ui_actions(&[UiAction::SpeciesPrune]);
        assert!(app.world.species.get(root).is_some() && app.confirm_dialog.is_none());
    }

    #[test]
    fn the_parent_and_root_buttons_move_the_selection_and_escape_closes_the_species_first() {
        let mut app = app_with_species();
        let (root, child) = family(&mut app);
        app.select_species_info(Some(child));
        app.apply_ui_actions(&[UiAction::SpeciesParent]);
        assert_eq!(app.species_info, Some(root));
        // A root has no parent: the button does nothing.
        app.apply_ui_actions(&[UiAction::SpeciesParent]);
        assert_eq!(app.species_info, Some(root));
        app.select_species_info(Some(child));
        app.apply_ui_actions(&[UiAction::SpeciesRoot]);
        assert_eq!(app.species_info, Some(root));
        // The node of the graph and the close tab.
        let index = app
            .lineage
            .as_ref()
            .and_then(|tree| tree.nodes.iter().position(|node| node.id == child))
            .expect("the child's node");
        app.apply_ui_actions(&[UiAction::SelectLineageNode(child.value())]);
        assert_eq!(app.species_info, Some(child));
        assert_eq!(app.lineage_selected, Some(index));
        app.apply_ui_actions(&[UiAction::SpeciesClose]);
        assert_eq!(app.species_info, None);
        // Escape closes the species panels before the lineage panel.
        app.select_species_info(Some(child));
        key(
            &mut app,
            FrameInput {
                quit: true,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.species_info, None);
        assert!(app.panels.is_open(crate::ui::PanelKind::Lineage));
        // Opening the panel again starts with no species (`SelectSpecies(null)`); it needs five
        // records of the species log first.
        app.select_species_info(Some(child));
        app.panels.close(crate::ui::PanelKind::Lineage);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenLineage)]);
        assert!(
            !app.panels.is_open(crate::ui::PanelKind::Lineage),
            "not enough data"
        );
        app.world.species.set_n_records(5);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenLineage)]);
        assert!(app.panels.is_open(crate::ui::PanelKind::Lineage));
        assert_eq!(app.species_info, None);
    }

    #[test]
    fn place_starts_the_placer_with_the_species_template() {
        let mut app = app_with_species();
        let (root, _) = family(&mut app);
        app.select_species_info(Some(root));
        // Place: the panel closes and the placer holds the template.
        app.apply_ui_actions(&[UiAction::SpeciesPlace]);
        assert!(!app.panels.is_open(crate::ui::PanelKind::Lineage));
        let placer = app.placer.as_ref().expect("the placer starts");
        assert_eq!(placer.template.name, "Zzroot zzspecies.bb8template");
    }

    #[test]
    fn save_opens_the_save_dialog_on_the_species_and_writes_its_template_where_the_library_lists_it() {
        let mut app = app_with_species();
        let (root, _) = family(&mut app);
        let dir = std::env::temp_dir().join(format!("rustibites-species-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the folder");
        app.storage = bibites_io::storage::StoragePaths::new(dir.clone());
        app.user_settings_path = Some(dir.join("settings.json"));
        app.world.species.get_mut(root).expect("the species").generation_of_first_specimen = 12;
        app.select_species_info(Some(root));
        assert!(!app.save_as_template && !app.paused);
        app.apply_ui_actions(&[UiAction::SpeciesSave]);
        // `SaveTemplateOfBibite`: the dialog opens on the species' name and notes with "Save as template" on (and the player's choice stored) and
        // locked, the game paused. Nothing is written yet.
        let dialog = app.save_dialog.as_ref().expect("the Save dialog");
        assert_eq!(dialog.name, "Zzroot zzspecies");
        assert_eq!(dialog.description, "root notes");
        assert!(dialog.as_template && !dialog.allow_saved);
        assert!(app.save_as_template && app.paused);
        assert!(crate::user_settings::UserSettings::load(Some(dir.join("settings.json"))).flag("Save as template"));
        assert!(!dir.join("Bibites").exists());
        // The box cannot be clicked off.
        app.apply_ui_actions(&[UiAction::SaveDialogToggle(0)]);
        assert!(app.save_dialog.as_ref().is_some_and(|dialog| dialog.as_template) && app.save_as_template);
        // The player renames the template and rewrites the notes.
        {
            let dialog = app.save_dialog.as_mut().expect("the dialog");
            dialog.name = "Orca line".to_owned();
            dialog.description = "my notes".to_owned();
        }
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SaveDialogConfirm)]);
        assert!(app.save_dialog.is_none() && !app.paused);
        // `SaveBibiteDialogHandle.GetSavePath`: `Bibites/Templates/<name>.bb8template`; not next to the saved bibites, where no Library looks.
        let file = dir.join("Bibites").join("Templates").join("Orca line.bb8template");
        assert!(file.is_file(), "{}", file.display());
        assert!(!dir.join("Bibites").join("Zzroot zzspecies.bb8template").exists());
        assert!(!dir.join("Bibites").join("Orca line.bb8template").exists());
        let loaded = bibites_io::load_template(&file).expect("a template the game can load");
        assert_eq!(loaded.template.description.as_deref(), Some("my notes"));
        assert_eq!(loaded.template.name.as_deref(), Some("Orca line"));
        assert_eq!(loaded.meta.generation, 12, "the generation of the species' first specimen");
        // The Library lists it.
        let listed = bibites_io::storage::list_library_entries(&app.storage);
        let entry = listed.iter().find(|entry| entry.path == file).expect("the Library lists the saved template");
        assert_eq!(entry.name, "Orca line");
        assert_eq!(entry.kind, bibites_io::storage::TemplateKind::Template);
        // The species keeps its own name.
        assert_eq!(app.world.species.get(root).map(|record| record.name()).as_deref(), Some("Zzroot zzspecies"));
        // Cancel writes nothing and gives the pause back.
        app.apply_ui_actions(&[UiAction::SpeciesSave]);
        assert!(app.save_dialog.is_some() && app.paused);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SaveDialogCancel)]);
        assert!(app.save_dialog.is_none() && !app.paused);
        assert_eq!(std::fs::read_dir(dir.join("Bibites").join("Templates")).expect("the folder").count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_save_dialog_is_over_the_species_panel_and_takes_its_clicks_and_its_wheel() {
        let (mut app, root, _) = app_with_tree();
        app.panels.open(crate::ui::PanelKind::Lineage);
        app.select_species_info(Some(root));
        app.lineage_view.zoom_to(1.0);
        let draw = |app: &App| {
            let mut frame = UiFrame::new(1.0);
            crate::hud::draw_extra_panels(&mut frame, app, &UiAssets::empty(), None, 1920.0, 1080.0, false);
            frame
        };
        let centre_of = |frame: &UiFrame, wanted: UiAction| {
            let id = frame.actions.iter().find(|(_, action)| *action == wanted).map(|(id, _)| *id).expect("the control is drawn");
            let (_, pos, size) = *frame.hits.iter().find(|(hit, _, _)| *hit == id).expect("with a rectangle");
            pos + size * 0.5
        };
        // The panel's own buttons answer while no dialog is up ...
        let before = draw(&app);
        let (place, save) = (centre_of(&before, UiAction::SpeciesPlace), centre_of(&before, UiAction::SpeciesSave));
        assert_eq!(before.action_at(place), Some(UiAction::SpeciesPlace));
        // ... Save opens the dialog over them: its screen blocker takes their clicks, its own buttons answer, and it is drawn after the panel.
        app.apply_ui_actions(&[UiAction::SpeciesSave]);
        let after = draw(&app);
        assert_eq!(after.action_at(place), Some(UiAction::Consume), "the panel under the dialog is blocked");
        assert_eq!(after.action_at(save), Some(UiAction::Consume));
        let confirm = UiAction::Invoke(ControlCommand::SaveDialogConfirm);
        assert_eq!(after.action_at(centre_of(&after, confirm)), Some(confirm));
        let first = |frame: &UiFrame, wanted: &str| {
            frame.draws.iter().position(|draw| matches!(draw, UiDraw::Text { text, .. } if text.trim() == wanted))
        };
        let (panel_label, dialog_title) = (first(&after, "Species ID:").expect("the panel"), first(&after, "Save Bibite").expect("the dialog"));
        assert!(dialog_title > panel_label, "the dialog is drawn over the panel");
        // The tree under it gets neither the wheel nor the zoom of Ctrl + wheel while the dialog is up ...
        let at = Vec2::new(900.0, 500.0);
        let zoom_in = FrameInput { cursor: at, wheel: 1.0, ctrl_down: true, ..FrameInput::default() };
        let (zoom, pos) = (app.lineage_view.zoom, app.lineage_view.pos);
        key(&mut app, zoom_in);
        assert_eq!((app.lineage_view.zoom, app.lineage_view.pos), (zoom, pos), "the view under the dialog stays");
        // ... and gets them again once it is closed.
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::SaveDialogCancel)]);
        key(&mut app, zoom_in);
        assert!(app.lineage_view.zoom != zoom, "Ctrl + wheel zooms the tree without the dialog");
    }

    #[test]
    fn the_info_values_follow_the_games_formats() {
        let mut app = app_with_species();
        let (root, child) = family(&mut app);
        // The panel counts the members again when it opens, so the counts are set (as if the world had them) after.
        app.world.species.get_mut(root).unwrap().stats.count = 4;
        app.world.species.get_mut(child).unwrap().stats.count = 3;
        app.world.species.get_mut(root).unwrap().time_creation = 90_000.0 + 3660.0;
        app.world
            .species
            .get_mut(root)
            .unwrap()
            .generation_of_first_specimen = 12;
        app.refresh_species_list();
        let entry = app
            .lineage_species
            .iter()
            .find(|entry| entry.id == root)
            .expect("root")
            .clone();
        let values = info_values(&app, &entry);
        // 1 day, 2 hours, 1 minute; generation; members; members of the species and its children.
        assert_eq!(values[0], "1d 2:1");
        assert_eq!(values[1].trim(), "12");
        assert_eq!(values[2].trim(), "4");
        assert_eq!(values[3].trim(), "7");
    }

    #[test]
    fn the_gene_lines_follow_the_games_order_formats_and_weights() {
        let same = [0.5_f32; 35];
        let rows = gene_rows(&same, &same, 1.0);
        assert_eq!(rows.len(), GENE_COUNT);
        // `BibiteEditorSettings.geneSettings` order: the colours first, the fat deadband last.
        assert_eq!(rows[0].name, "Red Color");
        assert_eq!(rows[1].name, "Green Color");
        assert_eq!(rows[2].name, "Blue Color");
        assert_eq!(rows[34].name, "Fat Storage Deadband");
        assert!(rows.iter().all(|row| !row.help.is_empty()));
        // Equal genes: no difference text, no tooltip, the prefab's colour.
        assert!(
            rows.iter()
                .all(|row| row.difference.is_empty() && row.breakdown.is_empty())
        );
        assert!(
            rows.iter()
                .all(|row| row.alpha.is_none() && row.contribution == 0.0)
        );
        assert!(
            rows.iter().all(|row| row.value == "0.500"),
            "{:?}",
            rows[0].value
        );
        // One gene differs by 0.25 (red colour: max 1) over a span of 2: it weighs 12.5 %.
        let mut other = same;
        other[Gene::ColorR.index()] = 0.25;
        let rows = gene_rows(&same, &other, 2.0);
        assert_eq!(rows[0].difference, "(+0.250)");
        assert!((rows[0].contribution - 0.125).abs() < 1e-6);
        assert_eq!(
            rows[0].breakdown,
            "Flat difference : 0.250\nRelative difference : 100.0%\nSpan difference : 0.250\nContribution to speciation : 12.5%"
        );
        // The heaviest line is solid black, the equal ones the faintest (`Lerp(0.15, 1, sqrt(share))`).
        assert!((rows[0].alpha.unwrap() - 1.0).abs() < 1e-6);
        assert!((rows[1].alpha.unwrap() - 0.15).abs() < 1e-6);
        // A negative difference is signed, and a gene that grew from zero reads "Infinity".
        other[Gene::ColorR.index()] = 0.75;
        assert_eq!(gene_rows(&same, &other, 1.0)[0].difference, "(-0.250)");
        other[Gene::ColorR.index()] = 0.0;
        assert!(
            gene_rows(&same, &other, 1.0)[0]
                .breakdown
                .contains("Infinity%")
        );
    }

    #[test]
    fn the_header_distances_read_like_the_games_handles() {
        // `(+58.1%)`: the gene distance to the parent over the species span, a percentage with its sign.
        assert_eq!(parent_distance_text(0.581 * 3.0, 3.0), "(+58.1%)");
        assert_eq!(parent_distance_text(0.0, 3.0), "(+0.0%)");
        // `(0.581)`: the gene distance to the compared species.
        assert_eq!(compared_distance_text(0.581), "(0.581)");
        assert_eq!(compared_distance_text(12.0), "(12.000)");
    }

    #[test]
    fn the_genes_panel_fits_the_screen_and_its_header_is_the_prefabs() {
        // The live game's panel for a species with a parent: 405.91 x 839.61 (390.91 inside).
        let layout = genes_layout(390.91, true);
        assert!((layout.panel[2] - 405.91).abs() < 0.01);
        assert!(
            (layout.panel[3] - 839.61).abs() < 0.01,
            "{}",
            layout.panel[3]
        );
        assert!(layout.panel[1] + layout.panel[3] < 1080.0);
        // The header rows are 3 apart (name 23.44, rows 20.41) and end 70.26 below its top.
        assert!((layout.parent_title[1] - layout.name[1] - 26.44).abs() < 0.01);
        assert!((layout.compared_title[1] - layout.parent_title[1] - 23.41).abs() < 0.01);
        assert!((layout.compared_title[1] + 20.41 - (layout.name[1] + 70.26)).abs() < 0.01);
        // Title 150, button 155.91, distance 80: they fill the row but 5.
        assert!((layout.compared_dropdown[2] - 155.91).abs() < 0.01);
        let right = layout.compared_distance[0] + layout.compared_distance[2];
        assert!((right - (layout.name[0] + 390.91 - 5.0)).abs() < 0.01);
        // The last gene line ends 10 above the panel's bottom edge.
        let last = layout.rows_top + 34.0 * GENE_PITCH + GENE_ROW;
        assert!((last + 10.0 - (layout.panel[1] + layout.panel[3])).abs() < 0.01);
        // A root species has no parent button: its row is 16.41 high and the panel 4 shorter (835.61 in the game).
        let root = genes_layout(390.91, false);
        assert!((root.panel[3] - 835.61).abs() < 0.01, "{}", root.panel[3]);
        assert!((root.parent_title[3] - 16.41).abs() < 0.01);
        assert!((root.compared_title[1] - layout.compared_title[1] + 4.0).abs() < 0.01);
    }

    /// The game's fonts, when they have been extracted.
    fn game_fonts() -> Option<crate::text_sdf::SdfFonts> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        crate::text_sdf::SdfFonts::discover(&root)
    }

    #[test]
    fn the_genes_panel_is_as_wide_as_its_widest_row_like_the_live_game() {
        let Some(fonts) = game_fonts() else {
            eprintln!("SDF font assets not present; skipping");
            return;
        };
        let measure = crate::stats_states::text_measure(Some(&fonts));
        // The captured game (`GET /d/ui` after opening each species on the Genes tab): 405.91, 405.91 and 445.75 wide.
        let width = |name: &str, parent: Option<&str>, parent_distance: &str, compared: &str, distance: &str| {
            layout::genes_inner_width(
                &layout::GenesTexts {
                    name,
                    parent,
                    parent_distance,
                    compared,
                    compared_distance: distance,
                },
                &measure,
            ) + 15.0
        };
        // The longest gene name ("Pheromones Sensing Radius:", 185.91 wide) decides.
        let got = width("Rouxkhous memetifius", Some("Neo via"), "(+614.0%)", "Neo via", "(9.209)");
        assert!((got - 405.91).abs() < 1.5, "{got}");
        let got = width("Turny boy", None, "", "Turny boy", "(0.000)");
        assert!((got - 405.91).abs() < 1.5, "{got}");
        // A long compared species' name (the dropdown, 30 of padding) widens the panel.
        let got = width(
            "Ignitomeleagris  deldrisus",
            Some("Rouxkhous pierrotarthurei"),
            "(+196.8%)",
            "Rouxkhous pierrotarthurei",
            "(2.952)",
        );
        assert!((got - 445.75).abs() < 1.5, "{got}");
    }

    #[test]
    fn the_genes_tab_compares_with_the_parent_and_the_dropdown_picks_another_species() {
        let mut app = app_with_species();
        let (root, child) = family(&mut app);
        app.world
            .species
            .get_mut(child)
            .unwrap()
            .template
            .genome
            .set_gene(Gene::LayTime, 40.0);
        app.select_species_info(Some(child));
        // A species is compared with its parent; a root with itself.
        assert_eq!(app.species_compared, Some(root));
        app.select_species_info(Some(root));
        assert_eq!(app.species_compared, Some(root));
        app.select_species_info(Some(child));
        app.apply_ui_actions(&[UiAction::SetLineageTab(u32::from(TAB_GENES))]);
        let frame = frame_of(&app);
        let shown = texts(&frame);
        // The header, the genes and the one difference.
        for label in [
            "Parent species:",
            "Compared species:",
            "Red Color:",
            "Fat Storage Deadband:",
        ] {
            assert!(shown.iter().any(|text| text == label), "{label}");
        }
        assert!(
            shown
                .iter()
                .filter(|text| *text == "Zzroot zzspecies")
                .count()
                >= 2
        );
        assert!(
            shown
                .iter()
                .any(|text| text.starts_with("(+") && text.ends_with("%)"))
        );
        assert_eq!(
            shown
                .iter()
                .filter(|text| text.starts_with("(+")
                    && text.ends_with(')')
                    && !text.ends_with("%)"))
                .count(),
            1,
            "the lay time differs: {shown:?}"
        );
        let breakdown = frame
            .tips
            .iter()
            .find(|rect| rect.tip.title == "Difference from target")
            .expect("the difference's tooltip");
        let root_value = app
            .world
            .species
            .get(root)
            .unwrap()
            .template
            .genome
            .values()[Gene::LayTime.index()];
        let child_value = app
            .world
            .species
            .get(child)
            .unwrap()
            .template
            .genome
            .values()[Gene::LayTime.index()];
        assert!(
            breakdown.tip.content.contains(&format!(
                "Flat difference : {:.3}",
                child_value - root_value
            )),
            "{}",
            breakdown.tip.content
        );
        assert!(frame.tips.iter().any(|rect| rect.tip.title == "Red Color"));
        // The dropdown lists the species of the graph and the picked one is what the genes are compared with.
        let at = frame
            .actions
            .iter()
            .find(|(_, action)| *action == UiAction::DropdownOpen(DropdownTarget::SpeciesCompared))
            .and_then(|(id, _)| frame.hits.iter().find(|hit| hit.0 == *id))
            .map(|hit| (hit.1, hit.2))
            .expect("the compared dropdown");
        app.click_rect = [at.0.x, at.0.y, at.1.x, at.1.y];
        app.apply_ui_actions(&[UiAction::DropdownOpen(DropdownTarget::SpeciesCompared)]);
        let open = app.dropdown_open.as_ref().expect("the list opens");
        let names: Vec<&str> = open.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(names.len(), app.species_displayed().len());
        assert!(names.contains(&"Zzroot zzspecies") && names.contains(&"Zzchild zzspecies"));
        assert_eq!(open.choices[open.selected].label, "Zzroot zzspecies");
        let own = names
            .iter()
            .position(|name| *name == "Zzchild zzspecies")
            .unwrap();
        app.apply_ui_actions(&[UiAction::DropdownPick(own as u16)]);
        assert_eq!(app.species_compared, Some(child));
        let shown = texts(&frame_of(&app));
        assert!(
            !shown
                .iter()
                .any(|text| text.starts_with("(+") && text.ends_with(')') && !text.ends_with("%)")),
            "compared with itself nothing differs"
        );
        // The Genes tab's own image is a finished button.
        assert!(frame_of(&app).hits.iter().any(|hit| hit.0 == HIT_TAB + 1));
    }

    fn info_texts<'a>(
        name: &'a str,
        root: &'a str,
        parent: Option<&'a str>,
    ) -> layout::InfoTexts<'a> {
        layout::InfoTexts {
            name,
            id: "2310".to_owned(),
            root,
            parent,
            values: [
                "3d 9:55".to_owned(),
                "345".to_owned(),
                "0".to_owned(),
                "26".to_owned(),
            ],
            editing: None,
        }
    }

    #[test]
    fn the_info_panel_has_the_prefabs_height_and_its_parts_fit_inside() {
        let fonts = game_fonts();
        let measure = crate::stats_states::text_measure(fonts.as_ref());
        let layout = layout::info_layout(
            &info_texts("Rouxkhous memetifius", "Neo via", Some("Neo via")),
            &measure,
        );
        // The line heights are the fonts' (without them the estimates are close but not exact).
        if fonts.is_some() {
            assert!(
                (layout.panel[3] - 489.71).abs() < 0.05,
                "{}",
                layout.panel[3]
            );
        }
        let (right, bottom) = (
            layout.panel[0] + layout.panel[2],
            layout.panel[1] + layout.panel[3],
        );
        let inside = |rect: [f32; 4]| {
            rect[0] >= layout.panel[0] - 0.01
                && rect[1] >= layout.panel[1] - 0.01
                && rect[0] + rect[2] <= right + 0.01
                && rect[1] + rect[3] <= bottom + 0.01
        };
        for rect in [
            layout.name_row,
            layout.id_row,
            layout.preview,
            layout.root_button,
            layout.parent_button,
            layout.notes_label,
            layout.notes,
        ] {
            assert!(inside(rect), "{rect:?}");
        }
        for rect in layout.buttons.iter().chain(layout.lines.iter()) {
            assert!(inside(*rect), "{rect:?}");
        }
        // The notes field ends 10 above the panel's bottom edge, the buttons are a 44 pitch column.
        assert!((layout.notes[1] + layout.notes[3] + 10.0 - bottom).abs() < 0.01);
        for pair in layout.buttons[..5].windows(2) {
            assert!((pair[1][1] - pair[0][1] - 44.0).abs() < 0.01);
        }
        assert!((right - (layout.buttons[0][0] + 35.0) - 10.0).abs() < 0.01);
        // "Notes:" has 10 of padding over it: the field starts one 12-point line under the label.
        if fonts.is_some() {
            assert!((layout.notes[1] - layout.notes_label[1] - 14.07).abs() < 0.05);
            // The 5 of spacing between the body's rows and the section's 10 of padding.
            assert!((layout.notes_label[1] - (layout.lines[3][1] + 16.41) - 15.0).abs() < 0.05);
        }
    }

    #[test]
    fn the_info_panel_is_as_wide_as_its_texts_ask_like_the_live_game() {
        let Some(fonts) = game_fonts() else {
            eprintln!("SDF font assets not present; skipping");
            return;
        };
        let measure = crate::stats_states::text_measure(Some(&fonts));
        // The captured game (`GET /d/ui` after opening each species), the panel's width and its Preview Section's:
        // the 125 of the titles and the longer of the two reference names decide for a short name.
        let cases = [
            (
                "Rouxkhous memetifius",
                "Neo via",
                Some("Neo via"),
                308.44,
                243.44,
            ),
            ("Turny boy", "Turny boy", None, 273.21, 208.21),
            (
                "Ignitomeleagris  deldrisus",
                "Neo via",
                Some("Rouxkhous pierrotarthurei"),
                375.75,
                310.75,
            ),
            (
                "Enoninjus joeylianus ",
                "Neo via",
                Some("Enoninjus hernandius "),
                347.09,
                282.09,
            ),
        ];
        for (name, root, parent, panel, preview) in cases {
            let layout = layout::info_layout(&info_texts(name, root, parent), &measure);
            assert!(
                (layout.panel[2] - panel).abs() < 1.5,
                "{name}: {} against {panel}",
                layout.panel[2]
            );
            // The preview frame is the Preview Section's width and 151.59 high; the buttons hang 10 from its right edge.
            assert!(
                (layout.preview[2] - preview).abs() < 1.5,
                "{name}: {}",
                layout.preview[2]
            );
            assert!((layout.preview[3] - 151.59).abs() < 0.01);
            assert!(
                (layout.buttons[0][0] - (layout.preview[0] + layout.preview[2] + 10.0)).abs()
                    < 0.01,
                "{name}"
            );
            // The notes field follows the panel's width less its padding.
            assert!(
                (layout.notes[2] - (layout.panel[2] - 20.0)).abs() < 0.01,
                "{name}"
            );
        }
    }

    #[test]
    fn the_close_tab_ends_ten_before_the_panels_right_edge() {
        // Live: the close tab's middle is 30 from the right end of a 308.44, 405.91, 598.46 or 420 wide panel.
        for width in [308.44, 405.91, 598.46, 420.0] {
            let rect = close_tab_rect(width);
            assert!((rect[0] + 20.0 - (OVERLAY.x + width - 30.0)).abs() < 0.01);
            // Raised: its middle is 34 from the top, over the other tabs' 45.
            assert!((rect[1] + 20.0 - 34.0).abs() < 0.01);
        }
    }

    #[test]
    fn the_tabs_sit_in_a_row_and_a_selected_one_rises_eleven() {
        for index in 0..4 {
            let resting = tab_rect(index, false);
            let raised = tab_rect(index, true);
            assert_eq!(resting[0], raised[0]);
            assert!((resting[1] - raised[1] - TAB_RISE).abs() < 0.01);
            assert_eq!((resting[2], resting[3]), (40.0, 40.0));
        }
        let xs: Vec<f32> = (0..4)
            .map(|index| tab_rect(index, false)[0] + 20.0)
            .collect();
        assert_eq!(xs, [45.0, 95.0, 145.0, 195.0]);
        // Exit is the right-most of the top bar's four buttons.
        assert!(bar_button(3)[0] > bar_button(0)[0]);
        assert_eq!(bar_button(3)[0] + 20.0, 1895.0);
    }

    #[test]
    fn nothing_is_drawn_while_the_panel_is_closed_and_a_species_shows_its_tabs_and_info() {
        let mut app = app_with_species();
        app.panels.close(crate::ui::PanelKind::Lineage);
        assert!(frame_of(&app).draws.is_empty());
        app.panels.open(crate::ui::PanelKind::Lineage);
        // Without a selected species: the bar and the graph, no panel (`SelectSpecies(null)`).
        let frame = frame_of(&app);
        assert!(
            texts(&frame)
                .iter()
                .any(|text| text == "Ancestral Lineage Graph -")
        );
        assert!(!texts(&frame).iter().any(|text| text == "Species ID:"));
        assert!(
            !frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::SpeciesNameEdit)
        );
        // A species selected: the info panel with its labels.
        let (first, _) = family(&mut app);
        app.panels.open(crate::ui::PanelKind::Lineage);
        app.select_species_info(Some(first));
        let frame = frame_of(&app);
        let shown = texts(&frame);
        for label in [
            "Species ID:",
            "Root species:",
            "Parent species:",
            "Time of creation:",
            "Gen at creation:",
            "Number of alive members:",
            "Number of alive descendants:",
            "Notes:",
        ] {
            assert!(shown.iter().any(|text| text == label), "{label}: {shown:?}");
        }
        assert!(shown.contains(&"Zzroot zzspecies".to_owned()), "{shown:?}");
        for action in [
            UiAction::SpeciesNameEdit,
            UiAction::SpeciesNotes,
            UiAction::SpeciesSave,
            UiAction::SpeciesPlace,
            UiAction::SpeciesClose,
        ] {
            assert!(
                frame.actions.iter().any(|(_, found)| *found == action),
                "{action:?}"
            );
        }
        assert!(
            frame.tips.len() >= 8,
            "tooltips of the controls: {}",
            frame.tips.len()
        );
    }

    // ---- the lineage tree -------------------------------------------------------------------

    /// The sprites of the tree from the repository (the tests that need them are skipped without).
    fn tree_sprites() -> Option<UiAssets> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game/reference/functional_ui_native");
        let mut ui = UiAssets::empty();
        for name in [
            "lineage-vertical",
            "lineage-horizontal",
            "lineage-elbow-right",
            "lineage-elbow-left",
            "lineage-junction-left",
            "lineage-junction-right",
            "lineage-arrowhead",
            "lineage-template",
            "lineage-termination",
            "lineage-node-small",
        ] {
            ui.add_texture_alias(name, &dir.join("lineage").join(format!("{name}.png")))
                .ok()?;
        }
        for name in [
            "Long_UI",
            "Background",
            "UISprite",
            "base_circle_icon",
            "base_icon",
            "checkmark_icon",
            "Button_UI",
        ] {
            ui.add_texture_alias(name, &dir.join(format!("sprites/{name}.png")))
                .ok()?;
        }
        ui.add_texture_alias(
            "Reset settings icon-1.png",
            &dir.join("sprites/Reset_settings_icon-1.png.png"),
        )
        .ok()?;
        Some(ui)
    }

    /// A world whose family has the root alive for a long time (a tall tree) and the child gone.
    fn app_with_tree() -> (App, SpeciesId, SpeciesId) {
        use bibites_core::species_log::SpeciesDataPoint;
        let mut app = app_with_species();
        let (root, child) = family(&mut app);
        let log = &mut app.world.species.get_mut(root).expect("root").stats.log;
        for _ in 0..120 {
            log.push(SpeciesDataPoint::from_counts(1, 5.0));
        }
        app.refresh_lineage();
        assert!(app.lineage_view.layout.built);
        (app, root, child)
    }

    #[test]
    fn a_sprite_cut_by_the_clip_is_a_region_of_it() {
        let Some(ui) = tree_sprites() else {
            return;
        };
        let texture = ui.texture_of_alias("lineage-vertical").expect("sprite");
        let size = ui
            .texture(texture)
            .map(|t| (t.width as f32, t.height as f32))
            .unwrap();
        let clip = [10.0, 0.0, 100.0, 100.0];
        // Inside: the whole sprite as it is.
        let mut frame = UiFrame::new(1.0);
        crate::lineage_graph::clip_image_for_tests(
            &mut frame,
            &ui,
            texture,
            [20.0, 10.0, 22.0, 22.0],
            clip,
        );
        assert!(matches!(frame.draws[..], [UiDraw::Image { .. }]));
        // Cut on the left: the right part of the sprite over the rest of the rectangle.
        let mut frame = UiFrame::new(1.0);
        crate::lineage_graph::clip_image_for_tests(
            &mut frame,
            &ui,
            texture,
            [0.0, 10.0, 22.0, 22.0],
            clip,
        );
        match frame.draws[..] {
            [
                UiDraw::ImageRegion {
                    region,
                    pos,
                    size: dest,
                    ..
                },
            ] => {
                assert_eq!((pos.x, dest.x), (10.0, 12.0));
                assert!(
                    (region[0] - size.0 * 10.0 / 22.0).abs() < 1e-3,
                    "{region:?}"
                );
                assert!(
                    (region[2] - size.0 * 12.0 / 22.0).abs() < 1e-3,
                    "{region:?}"
                );
                assert_eq!(region[1], 0.0);
                assert!((region[3] - size.1).abs() < 1e-3);
            }
            ref other => panic!("{other:?}"),
        }
        // Outside: nothing.
        let mut frame = UiFrame::new(1.0);
        crate::lineage_graph::clip_image_for_tests(
            &mut frame,
            &ui,
            texture,
            [200.0, 10.0, 22.0, 22.0],
            clip,
        );
        assert!(frame.draws.is_empty());
    }

    /// The lineage panel's tag lines: the viewport-wide rectangles 2 high in a tag's colour (their y), and the chips' labels.
    fn tag_line_ys(frame: &UiFrame, tag: &crate::time_tags::TimeTag) -> Vec<f32> {
        let mut color = tag.color();
        color[3] = 215;
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Rect { pos, size, color: c } if *c == color && size.y == 2.0 && size.x > 500.0 => Some(pos.y + 1.0),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn time_tags_cross_the_lineage_graph_at_the_height_of_their_time_in_the_tree_and_in_the_mesh() {
        use crate::time_tags::{TagPanel, species_point_of_minutes};
        let (mut app, _, _) = app_with_tree();
        app.world.simulated_time = 36_000.0;
        let root = std::env::temp_dir().join(format!("rustibites_lineage_tags_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        app.storage = bibites_io::storage::StoragePaths::new(&root);
        let twenty = app.time_tags.add(36_000.0 - 20.0 * 60.0, "Twenty minutes back").unwrap();
        let now_tag = app.time_tags.add(36_000.0, "Now tag").unwrap();
        let frame = frame_of(&app);
        let shown = texts(&frame);
        assert!(shown.contains(&"Twenty minutes back".to_owned()) && shown.contains(&"Now tag".to_owned()), "{shown:?}");
        assert!(shown.contains(&"Tag time".to_owned()) && shown.contains(&"Hide tags (2)".to_owned()), "{shown:?}");
        for id in [twenty, now_tag] {
            assert!(frame.actions.iter().any(|(_, a)| *a == UiAction::TimeTagRemove(id)), "remove {id}");
        }
        // The line of the tag 20 minutes old is at the axis' height of that place (between the 3rd and the 4th point), the one set now on the bottom line.
        let view = &app.lineage_view;
        let axis = crate::lineage_tree::time_axis();
        let height = axis.height_of_place(species_point_of_minutes(20.0), view.step());
        let wanted = view.to_screen(0.0, height)[1];
        let tag = app.time_tags.get(twenty).unwrap().clone();
        let ys = tag_line_ys(&frame, &tag);
        assert_eq!(ys.len(), 1, "{ys:?}");
        assert!((ys[0] - wanted).abs() < 0.01, "{} vs {wanted}", ys[0]);
        let viewport = view.geometry().viewport;
        let bottom = view.to_screen(0.0, 0.0)[1].clamp(viewport[1] + 1.0, viewport[1] + viewport[3] - 1.0);
        let now_ys = tag_line_ys(&frame, app.time_tags.get(now_tag).unwrap());
        assert_eq!(now_ys.len(), 1, "{now_ys:?}");
        assert!((now_ys[0] - bottom).abs() < 0.01, "{} vs {bottom}", now_ys[0]);
        // The panel's own button hides them; the buttons stay.
        app.apply_ui_actions(&[UiAction::TimeTagsShow(TagPanel::Lineage)]);
        let frame = frame_of(&app);
        let shown = texts(&frame);
        assert!(!shown.contains(&"Twenty minutes back".to_owned()) && tag_line_ys(&frame, &tag).is_empty());
        assert!(shown.contains(&"Show tags (2)".to_owned()) && shown.contains(&"Tag time".to_owned()), "{shown:?}");
        app.apply_ui_actions(&[UiAction::TimeTagsShow(TagPanel::Lineage)]);
        // The mesh view crosses with the same tags (its own default step).
        let (mut mesh_app, _, _) = app_with_mesh();
        mesh_app.storage = app.storage.clone();
        mesh_app.world.simulated_time = 36_000.0;
        mesh_app.time_tags.add(36_000.0 - 20.0 * 60.0, "In the mesh").unwrap();
        let frame = frame_of(&mesh_app);
        assert!(mesh_app.lineage_view.mesh_mode);
        assert!(texts(&frame).contains(&"In the mesh".to_owned()));
        let tag = mesh_app.time_tags.get(1).unwrap();
        let mesh_view = &mesh_app.lineage_view;
        let height = axis.height_of_place(species_point_of_minutes(20.0), mesh_view.step());
        let ys = tag_line_ys(&frame, tag);
        assert_eq!(ys.len(), 1);
        assert!((ys[0] - mesh_view.to_screen(0.0, height)[1]).abs() < 0.01);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_click_on_the_lineage_graph_sets_the_tag_at_the_moment_of_the_axis_under_the_pointer_in_the_tree_and_in_the_mesh() {
        use crate::time_tags::TagPanel;
        let root = std::env::temp_dir().join(format!("rustibites_lineage_tags_pick_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let (tree_app, _, _) = app_with_tree();
        let (mesh_app, _, _) = app_with_mesh();
        for (mut app, mesh) in [(tree_app, false), (mesh_app, true)] {
            app.storage = bibites_io::storage::StoragePaths::new(&root);
            // A world old enough for every height of the view (the log's last scale reaches far back).
            let now = 1.0e9;
            app.world.simulated_time = now;
            assert_eq!(app.lineage_view.mesh_mode, mesh);
            let viewport = app.lineage_view.geometry().viewport;
            let pick = |app: &mut App, y: f32| -> Option<UiAction> {
                app.cursor = Vec2::new(900.0, y);
                frame_of(app).action_at(app.cursor)
            };
            // With no name field open the viewport takes no click for the tags.
            assert!(!matches!(pick(&mut app, viewport[1] + viewport[3] * 0.5), Some(UiAction::TimeTagPlace(..))), "mesh {mesh}");
            app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Lineage)]);
            for fraction in [0.25, 0.5, 0.9] {
                let y = viewport[1] + viewport[3] * fraction;
                let Some(UiAction::TimeTagPlace(TagPanel::Lineage, seconds)) = pick(&mut app, y) else {
                    panic!("the viewport takes the click (mesh {mesh}, {fraction})");
                };
                assert!(seconds < now && seconds >= 0.0, "{seconds}");
                assert!(texts(&frame_of(&app)).iter().any(|text| text.starts_with("Tag here: ")), "the label says how long before now");
                // The tag set there crosses the graph where the pointer was.
                app.apply_ui_actions(&[UiAction::TimeTagPlace(TagPanel::Lineage, seconds)]);
                assert!(app.time_tag_edit.is_none());
                let tag = app.time_tags.all().iter().find(|tag| tag.seconds == seconds).expect("the tag").clone();
                let ys = tag_line_ys(&frame_of(&app), &tag);
                assert_eq!(ys.len(), 1, "mesh {mesh}, {fraction}: {ys:?}");
                assert!((ys[0] - y).abs() < 0.1, "mesh {mesh}, {fraction}: {} vs {y}", ys[0]);
                app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Lineage)]);
            }
            assert_eq!(app.time_tags.len(), 3);
            // The pointer at the viewport's bottom edge still has a moment (never after now).
            let Some(UiAction::TimeTagPlace(_, newest)) = pick(&mut app, viewport[1] + viewport[3] - 0.5) else { panic!("the viewport takes the click") };
            assert!(newest <= now, "{newest}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_name_field_of_the_lineage_panel_sits_between_the_title_and_the_bars_buttons_and_the_tag_set_there_is_the_apps_one() {
        use crate::time_tags::TagPanel;
        let (mut app, _, _) = app_with_tree();
        let root = std::env::temp_dir().join(format!("rustibites_lineage_tags_field_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        app.storage = bibites_io::storage::StoragePaths::new(&root);
        app.apply_ui_actions(&[UiAction::TimeTagStart(TagPanel::Lineage)]);
        let shown = texts(&frame_of(&app));
        assert!(shown.contains(&"Tag now".to_owned()) && shown.iter().any(|text| text.starts_with("Name")), "{shown:?}");
        app.typed = "Mass extinction".to_owned();
        key(&mut app, FrameInput::default());
        key(&mut app, FrameInput { enter: true, ..FrameInput::default() });
        assert_eq!(app.time_tags.len(), 1);
        // The same tag is the one the graphs panel's frame lists (the list is the app's, not a panel's).
        let label = app.time_tags.all()[0].label.clone();
        assert_eq!(label, "Mass extinction");
        assert!(texts(&frame_of(&app)).contains(&label));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_tree_draws_its_species_with_hits_and_tooltips() {
        let Some(ui) = tree_sprites() else {
            return;
        };
        let (mut app, root, child) = app_with_tree();
        app.select_species_info(Some(root));
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, &app, &ui, None, 1920.0, 1080.0);
        // Every drawn species can be pressed.
        for id in [root, child] {
            assert!(
                frame
                    .actions
                    .iter()
                    .any(|(_, action)| *action == UiAction::SelectLineageNode(id.value())),
                "species {} has no hit",
                id.value()
            );
        }
        // The game's tooltips of the parts: the child's birth, its extinction, the template's introduction.
        let titles: Vec<&str> = frame.tips.iter().map(|rect| rect.tip.title).collect();
        for wanted in ["Species' Birth", "Extinction", "Species' Introduction"] {
            assert!(titles.contains(&wanted), "{wanted} in {titles:?}");
        }
        let birth = frame
            .tips
            .iter()
            .find(|rect| rect.tip.title == "Species' Birth")
            .unwrap();
        assert!(birth.tip.instant, "`parentLink`'s trigger is instant");
        assert!(
            birth
                .tip
                .content
                .starts_with("Zzchild zzspecies diverged from Zzroot zzspecies approximately "),
            "{}",
            birth.tip.content
        );
        assert!(birth.tip.content.ends_with(" ago"));
        // The axis: "now" at the bottom, in both corners.
        let shown = texts(&frame);
        assert!(
            shown.iter().filter(|text| *text == "now").count() >= 2,
            "{shown:?}"
        );
        // The selected species is outlined: its parts are drawn again in the highlight colour.
        let yellow = frame
            .draws
            .iter()
            .filter(|draw| {
                matches!(
                    draw,
                    UiDraw::Image {
                        tint: [251, 255, 0, 255],
                        ..
                    } | UiDraw::ImageRegion {
                        tint: [251, 255, 0, 255],
                        ..
                    }
                )
            })
            .count();
        assert!(yellow > 0, "the selected species is outlined");
        // Nothing of the tree is drawn or pressable outside its viewport.
        let viewport = app.lineage_view.geometry().viewport;
        for (id, pos, size) in &frame.hits {
            if *id >= crate::lineage_graph::HIT_PART {
                assert!(
                    pos.x >= viewport[0] - 0.5
                        && pos.y >= viewport[1] - 0.5
                        && pos.x + size.x <= viewport[0] + viewport[2] + 0.5
                        && pos.y + size.y <= viewport[1] + viewport[3] + 0.5,
                    "hit {id} at {pos:?} {size:?} is outside {viewport:?}"
                );
            }
        }
    }

    #[test]
    fn the_wheel_scrolls_ctrl_zooms_and_the_right_button_drags_the_tree() {
        let (mut app, _, _) = app_with_tree();
        // The tree opens fitted to the view; zoomed in it is taller than the viewport.
        app.lineage_view.zoom_to(1.0);
        let view = |app: &App| (app.lineage_view.zoom, app.lineage_view.pos);
        let at = Vec2::new(900.0, 500.0);
        let (zoom, pos) = view(&app);
        let g = app.lineage_view.geometry();
        let content = app.lineage_view.content_size();
        assert!(content[1] * zoom > g.viewport[3], "a tall tree");
        // The wheel scrolls up 35 a notch (content moves down).
        key(
            &mut app,
            FrameInput {
                cursor: at,
                wheel: 2.0,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.lineage_view.zoom, zoom);
        assert!(
            (app.lineage_view.pos[1] - (pos[1] - 70.0)).abs() < 1e-3,
            "{:?}",
            app.lineage_view.pos
        );
        // Ctrl zooms by a sixth a notch and does not scroll.
        let scrolled = app.lineage_view.pos;
        key(
            &mut app,
            FrameInput {
                cursor: at,
                wheel: 1.0,
                ctrl_down: true,
                ..FrameInput::default()
            },
        );
        assert!(
            app.lineage_view.zoom > zoom * 1.1 - 1e-4
                || app.lineage_view.zoom >= crate::lineage_tree::MAX_ZOOM - 1e-4
        );
        let _ = scrolled;
        // A right-button drag moves the content with the cursor.
        app.lineage_view.zoom_to(zoom);
        app.lineage_view.pos = [0.0, -200.0];
        key(
            &mut app,
            FrameInput {
                cursor: at,
                secondary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(matches!(
            app.lineage_view.drag,
            crate::lineage_view::Drag::Pan(_)
        ));
        key(
            &mut app,
            FrameInput {
                cursor: Vec2::new(at.x, at.y + 50.0),
                secondary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(
            (app.lineage_view.pos[1] - (-250.0)).abs() < 1e-3,
            "{:?}",
            app.lineage_view.pos
        );
        key(
            &mut app,
            FrameInput {
                cursor: Vec2::new(at.x, at.y + 50.0),
                ..FrameInput::default()
            },
        );
        assert_eq!(app.lineage_view.drag, crate::lineage_view::Drag::None);
        // Dragging the vertical bar's handle sets the position: the handle's bottom end is "now".
        let (_, vertical) = app.lineage_view.bars();
        let track = crate::lineage_view::vertical_track(app.lineage_view.geometry());
        let handle = crate::lineage_view::handle_rect(track, vertical, true);
        let grab = Vec2::new(handle[0] + handle[2] / 2.0, handle[1] + handle[3] / 2.0);
        key(
            &mut app,
            FrameInput {
                cursor: grab,
                primary_down: true,
                primary_pressed: true,
                ..FrameInput::default()
            },
        );
        key(
            &mut app,
            FrameInput {
                cursor: Vec2::new(grab.x, grab.y + 5000.0),
                primary_down: true,
                ..FrameInput::default()
            },
        );
        assert!(
            app.lineage_view.pos[1].abs() < 0.5,
            "dragged to the bottom: {:?}",
            app.lineage_view.pos
        );
        key(
            &mut app,
            FrameInput {
                cursor: Vec2::new(grab.x, grab.y - 5000.0),
                primary_down: true,
                ..FrameInput::default()
            },
        );
        let (_, now) = app.lineage_view.bars();
        assert!(now.value > 0.99, "dragged to the top: {now:?}");
        key(&mut app, FrameInput::default());
        assert_eq!(app.lineage_view.drag, crate::lineage_view::Drag::None);
    }

    /// What the window loop does with a frame (`route_ui_frame`): the panel's full-screen backdrop is a widget everywhere, so the loop takes the
    /// wheel and the buttons from everything but the UI. The Lineage panel still has to get them (it had none of them in the window).
    #[test]
    fn the_window_loop_leaves_the_lineage_panel_its_wheel_and_its_scroll_bars() {
        let (mut app, _, _) = app_with_tree();
        app.lineage_view.zoom_to(1.0);
        let at = Vec2::new(900.0, 500.0);
        let route = |app: &mut App, raw: FrameInput| -> FrameInput {
            let mut frame = UiFrame::new(1.0);
            emit(&mut frame, app, &UiAssets::empty(), None, 1920.0, 1080.0);
            assert!(frame.widget_at(at).is_some(), "the backdrop is a widget");
            crate::route_ui_frame(app, &frame, raw, None, 1920.0, 1080.0)
        };
        // Ctrl (either one) and the wheel zoom the graph; the camera behind it gets no wheel.
        for (left, right) in [(true, false), (false, true)] {
            let before = app.lineage_view.zoom;
            let routed = route(
                &mut app,
                FrameInput {
                    cursor: at,
                    wheel: -1.0,
                    ctrl_down: left,
                    right_ctrl_down: right,
                    ..FrameInput::default()
                },
            );
            assert_eq!(routed.wheel, 0.0, "the camera does not zoom");
            app.handle_input(&routed);
            assert!(
                app.lineage_view.zoom < before - 1e-4 || app.lineage_view.zoom <= app.lineage_view.min_zoom() + 1e-4,
                "zoomed out: {before} -> {}",
                app.lineage_view.zoom
            );
            let before = app.lineage_view.zoom;
            let routed = route(
                &mut app,
                FrameInput {
                    cursor: at,
                    wheel: 1.0,
                    ctrl_down: left,
                    right_ctrl_down: right,
                    ..FrameInput::default()
                },
            );
            app.handle_input(&routed);
            assert!(
                app.lineage_view.zoom > before + 1e-4 || app.lineage_view.zoom >= crate::lineage_tree::MAX_ZOOM - 1e-4,
                "zoomed in: {before} -> {}",
                app.lineage_view.zoom
            );
        }
        // The plain wheel scrolls.
        app.lineage_view.zoom_to(1.0);
        let pos = app.lineage_view.pos;
        let routed = route(&mut app, FrameInput { cursor: at, wheel: 2.0, ..FrameInput::default() });
        app.handle_input(&routed);
        assert!((app.lineage_view.pos[1] - (pos[1] - 70.0)).abs() < 1e-3, "{pos:?} -> {:?}", app.lineage_view.pos);
        // The scroll bar's handle takes a press (the loop clears the primary button over the UI).
        let (_, vertical) = app.lineage_view.bars();
        let track = crate::lineage_view::vertical_track(app.lineage_view.geometry());
        let handle = crate::lineage_view::handle_rect(track, vertical, true);
        let grab = Vec2::new(handle[0] + handle[2] / 2.0, handle[1] + handle[3] / 2.0);
        let press = FrameInput { cursor: grab, primary_down: true, primary_pressed: true, ..FrameInput::default() };
        let routed = route(&mut app, press);
        app.handle_input(&routed);
        assert!(matches!(app.lineage_view.drag, crate::lineage_view::Drag::VerticalBar(_)), "{:?}", app.lineage_view.drag);
    }

    // ---- the Settings panel -----------------------------------------------------------------

    #[test]
    fn the_gear_button_opens_the_settings_panel_with_its_toggles_and_sliders() {
        use crate::lineage_settings::LineageOption;
        let Some(ui) = tree_sprites() else {
            return;
        };
        let (mut app, _, _) = app_with_tree();
        assert!(!app.lineage_settings_open);
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, &app, &ui, None, 1920.0, 1080.0);
        // The gear is the third button of the top bar.
        let gear = frame
            .actions
            .iter()
            .find(|(_, action)| *action == UiAction::ToggleLineageSettings)
            .expect("the gear button");
        let hit = frame.hits.iter().find(|hit| hit.0 == gear.0).unwrap();
        assert_eq!((hit.1.x, hit.1.y), (bar_button(2)[0], bar_button(2)[1]));
        app.apply_ui_actions(&[UiAction::ToggleLineageSettings]);
        assert!(app.lineage_settings_open);
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, &app, &ui, None, 1920.0, 1080.0);
        // Its three toggles and its three sliders with their reset buttons.
        for option in [
            LineageOption::HideExtinct,
            LineageOption::ShowFavorites,
            LineageOption::ScaleSpecies,
        ] {
            assert!(
                frame
                    .actions
                    .iter()
                    .any(|(_, action)| *action == UiAction::LineageOption(option.index())),
                "{}",
                option.label()
            );
        }
        let slides = frame
            .actions
            .iter()
            .filter(|(_, action)| matches!(action, UiAction::DynSlide(..)))
            .count();
        let resets = frame
            .actions
            .iter()
            .filter(|(_, action)| matches!(action, UiAction::DynReset(..)))
            .count();
        assert!(
            slides >= 1 && resets >= 1,
            "{slides} sliders, {resets} resets in view"
        );
        let shown = texts(&frame);
        for wanted in [
            "Lineage Panel Settings",
            "Effective Species Span:",
            "Hide Extinct Lineages",
        ] {
            assert!(
                shown.iter().any(|text| text == wanted),
                "{wanted} in {shown:?}"
            );
        }
        // The names have their tooltips; "Effective Species Span" shows the value the registry uses.
        let titles: Vec<&str> = frame.tips.iter().map(|rect| rect.tip.title).collect();
        assert!(titles.contains(&"Hide Extinct Lineages"), "{titles:?}");
        let span = format!("{:.3}", app.lineage_effective_span());
        assert!(shown.contains(&span), "{span} in {shown:?}");
        // Pressing the gear again closes it, and so does Escape.
        app.apply_ui_actions(&[UiAction::ToggleLineageSettings]);
        assert!(!app.lineage_settings_open);
        app.apply_ui_actions(&[UiAction::ToggleLineageSettings]);
        key(
            &mut app,
            FrameInput {
                quit: true,
                ..FrameInput::default()
            },
        );
        assert!(!app.lineage_settings_open);
        assert!(
            app.panels.is_open(crate::ui::PanelKind::Lineage),
            "the panel stays"
        );
    }

    #[test]
    fn the_info_panel_and_the_tree_nodes_draw_the_templates_procedural_bibite() {
        let Some(ui) = tree_sprites() else {
            return;
        };
        let Some(assets) = crate::assets::GameAssets::discover().ok().flatten() else {
            return;
        };
        let (mut app, root, _) = app_with_tree();
        app.select_species_info(Some(root));
        let pictures = |app: &App| -> (Vec<(f32, f32)>, usize) {
            let mut frame = UiFrame::new(1.0);
            emit(&mut frame, app, &ui, None, 1920.0, 1080.0);
            let whole = frame
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    UiDraw::Pixels { size, .. } => Some((size.x, size.y)),
                    _ => None,
                })
                .collect();
            let cut = frame
                .draws
                .iter()
                .filter(|draw| matches!(draw, UiDraw::PixelsRegion { .. }))
                .count();
            (whole, cut)
        };
        // Without the game's sprites there is nothing to draw.
        assert_eq!(pictures(&app), (Vec::new(), 0));
        app.assets = Some(assets);
        let (whole, cut) = pictures(&app);
        // The info panel's `BibitePreview` is at least 200 wide (as wide as the panel's widest row, which the test's estimated text widths
        // decide) and about 151.59 high (what the 255 high top section leaves), and shows the bibite whole.
        assert_eq!(whole.len(), 1, "{whole:?}");
        assert!(
            whole[0].0 >= 200.0 && (whole[0].1 - 151.59).abs() < 1.0,
            "{whole:?}"
        );
        assert!(cut >= 1, "a node of the tree shows its bibite ({cut})");
        // The pictures are made once per size: drawing again adds none.
        let made = app.previews.len();
        let _ = pictures(&app);
        assert_eq!(app.previews.len(), made);
        // Another species shows another picture of its own genes.
        let other = app
            .species_picture(root, crate::preview::INFO_PANEL, [267.0, 151.59], 1.0)
            .unwrap();
        let again = app
            .species_picture(root, crate::preview::INFO_PANEL, [267.0, 151.59], 1.0)
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&other, &again));
    }

    #[test]
    fn a_settings_slider_rows_value_text_opens_its_edit_field_in_place_of_the_slider() {
        use crate::dyn_panel::FieldRef;
        use crate::value_edit::ValueEdit;
        let Some(ui) = tree_sprites() else {
            return;
        };
        let (mut app, _, _) = app_with_tree();
        app.lineage_settings_open = true;
        let (index, row) = crate::dyn_panel::ui_of("speciesGeneticSpan").expect("the row");
        let field = FieldRef::Setting(index as u16);
        let acts = |app: &App| -> Vec<UiAction> {
            let mut frame = UiFrame::new(1.0);
            emit(&mut frame, app, &ui, None, 1920.0, 1080.0);
            frame.actions.iter().map(|(_, action)| *action).collect()
        };
        let before = acts(&app);
        assert!(
            before.contains(&UiAction::DynEditValue(field)),
            "the value text is a button"
        );
        assert!(
            before
                .iter()
                .any(|a| matches!(a, UiAction::DynSlide(at, ..) if usize::from(*at) == index))
        );
        assert!(before.contains(&UiAction::DynReset(index as u16)));
        // Open, the field takes the slider's and the reset button's place.
        let shown = crate::dyn_panel::read_value(&app.world.settings, row) * row.factor;
        app.value_edit = Some(ValueEdit::open(
            field,
            shown,
            crate::value_edit::Content::Decimal,
        ));
        let open = acts(&app);
        assert!(open.contains(&UiAction::ValueEditInside));
        assert!(open.contains(&UiAction::DynEditValue(field)));
        assert!(
            !open
                .iter()
                .any(|a| matches!(a, UiAction::DynSlide(at, ..) if usize::from(*at) == index))
        );
        assert!(!open.contains(&UiAction::DynReset(index as u16)));
        // The other rows keep theirs.
        assert!(open.iter().any(|a| matches!(a, UiAction::DynSlide(..))));
        // The typed text is drawn, and the value text follows it.
        app.value_edit.as_mut().unwrap().type_char('4');
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, &app, &ui, None, 1920.0, 1080.0);
        let shown = texts(&frame);
        assert!(shown.iter().any(|text| text == "4"), "{shown:?}");
        let preview = crate::dyn_panel::format_of(row).format(4.0);
        assert!(shown.contains(&preview), "{preview:?} in {shown:?}");
    }

    #[test]
    fn toggling_an_option_builds_the_tree_again_and_drops_a_species_that_is_not_drawn() {
        use crate::lineage_settings::LineageOption;
        let (mut app, root, child) = app_with_tree();
        // Extinct lineages show in this fixture; the child is extinct and selected.
        assert!(app.lineage_view.layout.node_of(child).is_some());
        app.select_species_info(Some(child));
        assert_eq!(app.species_info, Some(child));
        app.apply_ui_actions(&[UiAction::LineageOption(LineageOption::HideExtinct.index())]);
        assert!(app.lineage_options.unwrap().hide_extinct);
        assert!(
            app.lineage_view.layout.node_of(child).is_none(),
            "an extinct lineage is hidden now"
        );
        assert!(app.lineage_view.layout.node_of(root).is_some());
        assert_eq!(
            app.species_info, None,
            "the hidden species leaves the panels"
        );
        // Showing favourites keeps a favourite's lineage; scaling only builds the tree again.
        app.world.species.set_favorite(child, true);
        app.refresh_lineage();
        app.apply_ui_actions(&[UiAction::LineageOption(
            LineageOption::ShowFavorites.index(),
        )]);
        assert!(
            app.lineage_view.layout.node_of(child).is_some(),
            "a favourite stays"
        );
        app.select_species_info(Some(child));
        app.apply_ui_actions(&[UiAction::LineageOption(LineageOption::ScaleSpecies.index())]);
        assert!(!app.lineage_options.unwrap().scale_species);
        assert_eq!(app.species_info, Some(child), "only the layout changed");
    }

    #[test]
    fn the_sliders_change_the_species_span_at_once_and_the_column_scrolls() {
        let (mut app, _, _) = app_with_tree();
        app.lineage_settings_open = true;
        let before = app.lineage_effective_span();
        let (index, row) = crate::dyn_panel::ui_of("speciesGeneticSpan").expect("the row");
        // The slider's track runs 100 px; a press at 60 puts the value 60 % of the way.
        app.dyn_slide(index, (200.0, 300.0), 260.0);
        let after = app.lineage_effective_span();
        assert!(after != before, "{before} -> {after}");
        assert_eq!(
            app.world.settings.independents.species_genetic_span,
            crate::dyn_panel::read_value(&app.world.settings, row)
        );
        // The registry uses the new span for its next assignments.
        let span = app.world.settings.independents.species_genetic_span;
        assert!((app.world.species.current_used_span(&app.world.settings) - after).abs() < 1e-6);
        assert!(span > 0.0);
        // The wheel over the panel scrolls the column 35 a notch; elsewhere it does not.
        let max = crate::lineage_settings::layout(true).max_scroll();
        assert!(max > 30.0);
        let over = Vec2::new(1500.0, 400.0);
        key(
            &mut app,
            FrameInput {
                cursor: over,
                wheel: -1.0,
                ..FrameInput::default()
            },
        );
        assert!(
            (app.lineage_settings_scroll - 35.0_f32.min(max)).abs() < 1e-3,
            "{}",
            app.lineage_settings_scroll
        );
        key(
            &mut app,
            FrameInput {
                cursor: over,
                wheel: -5.0,
                ..FrameInput::default()
            },
        );
        assert!((app.lineage_settings_scroll - max).abs() < 1e-3, "clamped");
        key(
            &mut app,
            FrameInput {
                cursor: over,
                wheel: 9.0,
                ..FrameInput::default()
            },
        );
        assert_eq!(app.lineage_settings_scroll, 0.0);
        // The tree under it did not move.
        let pos = app.lineage_view.pos;
        key(
            &mut app,
            FrameInput {
                cursor: over,
                wheel: 2.0,
                ..FrameInput::default()
            },
        );
        assert_eq!(
            app.lineage_view.pos, pos,
            "the wheel belongs to the panel while it is over it"
        );
    }

    /// A world with two species of known histories (twelve logs: the root with 6 bibites and 900 energy, its child with
    /// 4 and 600), the Lineage panel open and the mesh view showing.
    fn app_with_mesh() -> (App, SpeciesId, SpeciesId) {
        let mut app = app_with_species();
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let root = app
            .world
            .species
            .add_template_species("Zzroot zzspecies", "", &genome, &brain);
        let child =
            app.world
                .species
                .add_template_species("Zzchild zzspecies", "", &genome, &brain);
        {
            let record = app.world.species.get_mut(child).expect("child");
            record.parent = Some(root);
            record.root = root;
        }
        app.world
            .species
            .get_mut(root)
            .expect("root")
            .children
            .push(child);
        for _ in 0..12 {
            for (id, count, energy) in [(root, 6, 900.0), (child, 4, 600.0)] {
                let record = app.world.species.get_mut(id).expect("species");
                record.stats.count = count;
                record.stats.energy = energy;
            }
            app.world.species.log_stats_with(&[]);
        }
        app.refresh_lineage();
        app.lineage_select_panel(false);
        (app, root, child)
    }

    /// The actions of the top bar's second button.
    fn second_button(frame: &UiFrame) -> Option<UiAction> {
        frame
            .actions
            .iter()
            .find(|(id, _)| *id == HIT_BAR_BUTTON + 1)
            .map(|(_, action)| *action)
    }

    #[test]
    fn the_mesh_and_tree_buttons_switch_the_view_and_the_settings_follow() {
        let (mut app, root, child) = app_with_mesh();
        assert!(app.lineage_view.mesh_mode && app.lineage_view.mesh.built);
        assert!(app.lineage_view.shows(root) && app.lineage_view.shows(child));
        // While the mesh shows, the button asks for the tree; while the tree shows, for the mesh.
        assert_eq!(
            second_button(&frame_of(&app)),
            Some(UiAction::SelectLineagePanel(true))
        );
        // The count in the title follows the view that shows.
        let displayed = app.lineage_view.displayed().len();
        assert!(
            texts(&frame_of(&app)).contains(&format!("({displayed} displayed)")),
            "{:?}",
            texts(&frame_of(&app))
        );
        app.select_species_info(Some(root));
        app.apply_ui_actions(&[UiAction::SelectLineagePanel(true)]);
        assert!(!app.lineage_view.mesh_mode && app.lineage_view.layout.built);
        assert_eq!(
            second_button(&frame_of(&app)),
            Some(UiAction::SelectLineagePanel(false))
        );
        // The species that was selected stays selected through the switch.
        assert_eq!(app.species_info, Some(root));
        app.apply_ui_actions(&[UiAction::SelectLineagePanel(false)]);
        assert!(app.lineage_view.mesh_mode && app.lineage_view.mesh.built);
        assert_eq!(app.species_info, Some(root));
        assert_eq!(
            (app.lineage_view.zoom, app.lineage_view.pos),
            (1.0, [0.0, 0.0])
        );
        // The view keeps its mode when the options change and the graph is made again.
        let before = app.lineage_view.mesh.serial;
        app.lineage_toggle_option(crate::lineage_settings::LineageOption::HideExtinct);
        assert!(app.lineage_view.mesh_mode);
        assert_ne!(app.lineage_view.mesh.serial, before, "made again");
    }

    #[test]
    fn the_settings_panel_has_no_tree_group_while_the_mesh_shows() {
        let Some(ui) = tree_sprites() else {
            return;
        };
        let (mut app, _, _) = app_with_mesh();
        app.lineage_settings_open = true;
        let toggles = |app: &App| {
            let mut frame = UiFrame::new(1.0);
            emit(&mut frame, app, &ui, None, 1920.0, 1080.0);
            let has = |option: crate::lineage_settings::LineageOption| {
                frame
                    .actions
                    .iter()
                    .any(|(_, action)| *action == UiAction::LineageOption(option.index()))
            };
            use crate::lineage_settings::LineageOption::{
                HideExtinct, ScaleSpecies, ShowFavorites,
            };
            (has(HideExtinct), has(ShowFavorites), has(ScaleSpecies))
        };
        assert_eq!(toggles(&app), (true, true, false), "the mesh view");
        app.apply_ui_actions(&[UiAction::SelectLineagePanel(true)]);
        assert_eq!(toggles(&app), (true, true, true), "the tree view");
    }

    #[test]
    fn the_shape_under_the_pointer_is_pressable_and_tells_its_rung() {
        let (mut app, root, child) = app_with_mesh();
        let (mesh, height) = {
            let mesh = app.lineage_view.mesh.mesh_of(root).expect("root shape");
            // Between the lines of the first and the second point.
            (mesh, mesh.bottom_height() + 80.0)
        };
        let profile = mesh.profile_at(height).expect("inside");
        let x = (profile.left + profile.right) / 2.0;
        let at = app
            .lineage_view
            .to_screen(crate::lineage_tree::SIDE_PADDING + x, height);
        // Nothing is pressable while the pointer is elsewhere (the shapes register no rectangles of their own).
        app.cursor = Vec2::new(5.0, 5.0);
        let idle = frame_of(&app);
        assert!(
            !idle
                .actions
                .iter()
                .any(|(_, action)| matches!(action, UiAction::SelectLineageNode(_)))
        );
        app.cursor = Vec2::new(at[0], at[1]);
        let frame = frame_of(&app);
        let pressed: Vec<UiAction> = frame
            .actions
            .iter()
            .map(|(_, action)| *action)
            .filter(|action| matches!(action, UiAction::SelectLineageNode(_)))
            .collect();
        assert_eq!(pressed, vec![UiAction::SelectLineageNode(root.value())]);
        let tip = frame
            .tips
            .iter()
            .rev()
            .find(|rect| rect.contains(app.cursor) && rect.tip.path.starts_with('@'))
            .expect("the shape's tooltip");
        assert_eq!(tip.tip.title, "Zzroot zzspecies");
        assert_eq!(tip.tip.path, format!("@lineage-mesh/{}", root.value()));
        assert!(
            tip.tip.content.contains("Count: 6") && tip.tip.content.contains("Energy: 900 E"),
            "{:?}",
            tip.tip.content
        );
        // The click selects the species.
        app.apply_ui_actions(&pressed);
        assert_eq!(app.species_info, Some(root));
        // The child's shape is the other one: a point of it presses the child.
        let other = app.lineage_view.mesh.mesh_of(child).expect("child shape");
        let profile = other.profile_at(height).expect("inside");
        let x = (profile.left + profile.right) / 2.0;
        let at = app
            .lineage_view
            .to_screen(crate::lineage_tree::SIDE_PADDING + x, height);
        app.cursor = Vec2::new(at[0], at[1]);
        let frame = frame_of(&app);
        assert!(
            frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::SelectLineageNode(child.value()))
        );
    }

    #[test]
    fn each_shape_has_its_own_hit_and_a_panel_over_it_takes_its_tooltip() {
        use crate::tooltip::Target;
        let (mut app, _, _) = app_with_mesh();
        let view = app.lineage_view.clone();
        // A point of some shape under the Settings panel (it hangs from x 1134.5 and y 50 to 790).
        let mut found = None;
        'search: for height in [300.0, 350.0, 400.0] {
            for step in 0..60 {
                let screen = [1140.0 + 10.0 * step as f32, 0.0];
                let y = view.to_screen(0.0, height)[1];
                let at = view.to_content(screen[0], y);
                if let Some(mesh) = view
                    .mesh
                    .mesh_at(at[0] - crate::lineage_tree::SIDE_PADDING, height)
                {
                    found = Some((Vec2::new(screen[0], y), mesh.species));
                    break 'search;
                }
            }
        }
        let (cursor, species) = found.expect("a shape under the panel's place");
        app.cursor = cursor;
        // With nothing over it the shape's tooltip shows, and its hit is its own.
        let frame = frame_of(&app);
        let hit = frame
            .actions
            .iter()
            .find(|(_, action)| *action == UiAction::SelectLineageNode(species.value()))
            .map(|(id, _)| *id)
            .expect("the hovered shape is pressable");
        assert_eq!(
            hit,
            crate::lineage_graph::HIT_PART + species.value() % 9_999
        );
        app.update_tooltip(&frame, cursor, false, 0.6);
        match app.tooltip.shown() {
            Some((Target::Control(tip), _)) => assert!(tip.path.starts_with("@lineage-mesh/")),
            other => panic!("{other:?}"),
        }
        // The Settings panel is drawn over it afterwards: the pointer is not on the shape any more.
        app.lineage_settings_open = true;
        let frame = frame_of(&app);
        app.update_tooltip(&frame, cursor, true, 0.6);
        assert!(app.tooltip.shown().is_none(), "{:?}", app.tooltip.shown());
    }

    #[test]
    fn the_mesh_is_drawn_as_pictures_inside_the_viewport() {
        let (mut app, root, _) = app_with_mesh();
        app.select_species_info(Some(root));
        let frame = frame_of(&app);
        let viewport = app.lineage_view.geometry().viewport;
        let pictures: Vec<(Vec2, Vec2)> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::PixelsRegion { pos, size, .. } => Some((*pos, *size)),
                _ => None,
            })
            .collect();
        assert!(!pictures.is_empty(), "the shapes are painted into tiles");
        for (pos, size) in &pictures {
            assert!(
                pos.x >= viewport[0] - 0.5
                    && pos.y >= viewport[1] - 0.5
                    && pos.x + size.x <= viewport[0] + viewport[2] + 0.5
                    && pos.y + size.y <= viewport[1] + viewport[3] + 0.5,
                "{pos:?} {size:?} is outside {viewport:?}"
            );
        }
        // Scrolling does not paint again: the tiles that stay in view are the same pictures.
        let key = |frame: &UiFrame| -> Vec<usize> {
            frame
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    UiDraw::PixelsRegion { texture, .. } => {
                        Some(std::sync::Arc::as_ptr(texture) as usize)
                    }
                    _ => None,
                })
                .collect()
        };
        // Zoomed in the graph is wider than the view, so it scrolls sideways.
        app.lineage_view.zoom_to(crate::lineage_tree::MAX_ZOOM);
        let first = key(&frame_of(&app));
        assert!(!first.is_empty());
        app.lineage_view.scroll(1.0, true);
        let second = key(&frame_of(&app));
        let kept = second.iter().filter(|tile| first.contains(tile)).count();
        assert!(kept > 0, "{first:?} then {second:?}");
    }
}

/// Probe (ignored): how long the Lineage panel's frames take while the wheel scrolls and zooms it, on a real save
/// (`RUSTIBITES_PROBE_SAVE=<zip> cargo test -p bibites-view --release --bins lineage_scroll_probe -- --ignored --nocapture`).
#[cfg(test)]
mod scroll_probe {
    use super::*;
    use crate::app::{FrameInput, LoadedWorld};

    #[test]
    #[ignore]
    fn lineage_scroll_probe() {
        let Ok(path) = std::env::var("RUSTIBITES_PROBE_SAVE") else {
            return;
        };
        let loaded = LoadedWorld::load(Some(std::path::Path::new(&path)), 1, 0, 1).expect("load");
        let assets = if std::env::var("PROBE_NO_ASSETS").is_ok() { None } else { crate::assets::GameAssets::discover().ok().flatten() };
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), assets);
        app.world.species.set_n_records(app.world.species.n_records().max(5));
        let ui = crate::ui::UiAssets::discover().ok().flatten();
        let empty = UiAssets::empty();
        for mesh in [true, false] {
            app.apply_ui_actions(&[UiAction::Invoke(crate::ui::ControlCommand::OpenLineage)]);
            app.apply_ui_actions(&[UiAction::SelectLineagePanel(!mesh)]);
            let mut times: Vec<(&str, f64)> = Vec::new();
            let at = Vec2::new(900.0, 500.0);
            let mut frame_ms = |app: &mut App, label: &'static str, input: FrameInput| {
                let start = std::time::Instant::now();
                app.handle_input(&input);
                let input_ms = start.elapsed().as_secs_f64() * 1000.0;
                let mut frame = UiFrame::new(1.0);
                emit(&mut frame, app, ui.as_ref().unwrap_or(&empty), None, 1920.0, 1080.0);
                let total = start.elapsed().as_secs_f64() * 1000.0;
                if total > 10.0 {
                    eprintln!("slow frame {label}: input {input_ms:.1} ms, emit {:.1} ms", total - input_ms);
                }
                times.push((label, total));
            };
            frame_ms(&mut app, "first", FrameInput { cursor: at, ..FrameInput::default() });
            for _ in 0..40 {
                frame_ms(&mut app, "scroll", FrameInput { cursor: at, wheel: -1.0, ..FrameInput::default() });
            }
            for _ in 0..30 {
                frame_ms(&mut app, "zoom", FrameInput { cursor: at, wheel: 1.0, ctrl_down: true, ..FrameInput::default() });
            }
            for _ in 0..30 {
                frame_ms(&mut app, "zoomout", FrameInput { cursor: at, wheel: -1.0, ctrl_down: true, ..FrameInput::default() });
            }
            for _ in 0..40 {
                frame_ms(&mut app, "idle", FrameInput { cursor: at, ..FrameInput::default() });
            }
            for label in ["first", "scroll", "zoom", "zoomout", "idle"] {
                let mut v: Vec<f64> = times.iter().filter(|(l, _)| *l == label).map(|(_, t)| *t).collect();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                eprintln!(
                    "{} {label:>8}: n {:>3} median {:7.2} ms  p90 {:7.2}  max {:7.2}",
                    if mesh { "mesh" } else { "tree" },
                    v.len(),
                    v[v.len() / 2],
                    v[(v.len() * 9 / 10).min(v.len() - 1)],
                    v[v.len() - 1]
                );
            }
        }
    }
}
