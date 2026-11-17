//! The Save Game panel (`SaveGamePanel`: `UICanvas/MainUI/TopLayer/Save Game Panel`, level2 path id 1637), opened by the pause menu's "Save Game".
//!
//! What it does (`SaveGamePanel.cs`, `SaveItemReference.cs`): opening it pauses the game and blocks the controls (`TogglePauseGame("SavePanel")`,
//! `AllowControl = false`), lists the saves (`SaveController.GetSaves()`: `Savefiles/*.zip`, the autosaves left out, newest first), shows the
//! simulated time, the size, the number of bibites (and eggs) and pellets, takes a 400 x 400 picture of the whole world
//! (`SceneScreenShotHandler`), and starts with an empty name and a disabled Save button. A row sets the name to its file's (without `.zip`); the
//! name field (`TMP_InputField`) enables the button when it holds no `Path.GetInvalidFileNameChars()` character and is not empty, and shows the
//! bug icon ("This will replace an existing savefile") when `<name>.zip` exists. Save asks first when it does (`DisplayChoiceDialog("Are you
//! SURE???", ...)`), then writes the world and the picture (`img.png`) and closes the panel; Delete asks (`deleteSaveFileActionWarning`), removes
//! the file and selects the row that takes its place (its name goes into the field; with no row left the field keeps its text); the X and Escape
//! close it.
//!
//! Geometry, fonts and colours are the prefab's (read with `tools/unity/dump_ui_tree.py`, `dump_ui_components.py`, `dump_fonts.py` and the scratch
//! `button_colors.py`): a full-screen navy `Background` (5 over the canvas on every side) with a horizontal layout group (padding 50, spacing 50)
//! of two columns, the list (`gray_panel`, 895 at 1920) and the selected panel (`base_icon`, 885): the widths are the two panels' sprites' minimum sizes
//! (20 and 10) plus half of what is left, so they only depend on the screen's width. The list has a 30 inset `ScrollRect` (sensitivity 50, a scrollbar
//! when the rows do not fit) of 50 high `SaveItem` rows (`base_icon`, padding 15 / 5, spacing 10: the name in Roboto-Bold 25, the date in
//! Roboto-Bold 20, 200 wide) 5 apart. The selected panel is a vertical group (padding 50 / 50 / 45 / 20, spacing 40) with the delete (60 x 60) and
//! close (50 x 50) buttons out of the flow, the picture (a square as high as what is left, `base_icon` with the raw image inset 10), the 253.92 high
//! list of lines (title, name field, "Informations", the time and size line, the numbers line) and the 300 x 100 Save button with the bug icon 80 to
//! its right.
//!
//! Not matched: the field has no caret movement or selection beyond the select-all of a click that focuses it (the caret blinks at the end), a
//! scrolled list draws only the rows that fit entirely (the game masks them), the scrollbar cannot be dragged (the wheel and the scroll
//! wheel's 50 per notch work, without the elastic overshoot), and a save that cannot be read does not raise the "Corrupted Save" popup.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bibites_core::format::FloatValueFormat;
use bibites_core::math::Vec2;

use crate::app::{App, ConfirmKind, FrameInput};
use crate::render::{Camera, Scene, SceneOptions};
use crate::save_dialog::is_invalid_file_char;
use crate::scrollbar::{Press, Style, VBar};
use crate::species_panel::Drawer;
use crate::text_sdf::SdfFonts;
use crate::ui::{FontRole, PanelKind, Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiTexture};

/// Hit ids of the panel (the block 9500..9549).
const HIT_BACK: u32 = 9500;
const HIT_NAME: u32 = 9501;
const HIT_SAVE: u32 = 9502;
const HIT_DELETE: u32 = 9503;
const HIT_CLOSE: u32 = 9504;
const HIT_INDICATOR: u32 = 9505;
const HIT_ROWS: u32 = 9510;

/// The root reaches 5 past the canvas on each side; with the padding of 50 the columns start 45 in.
const SIDE: f32 = 45.0;
/// The two columns are as high as the canvas less 90.
const COLUMN_H: f32 = 990.0;
/// The two panels' minimum widths (`Image.preferredWidth` of a sliced sprite: the borders): `gray_panel` 20, `base_icon` 10.
const LIST_MIN: f32 = 20.0;
const SELECTED_MIN: f32 = 10.0;
/// A TMP line of Roboto: `size * 1.172`.
const LINE_K: f32 = 1.172;
/// The vertical group's spacing, and its padding.
const SPACING: f32 = 40.0;
/// The size of the world's picture (`SceneScreenShotHandler`).
pub const PICTURE: u32 = 400;
/// A row of the list: height, spacing, and the content's padding (left, right, top, bottom).
const ROW_H: f32 = 50.0;
const ROW_SPACING: f32 = 5.0;
const CONTENT_PAD: [f32; 4] = [15.0, 15.0, 15.0, 50.0];
/// The scroll view's inset in the list and its height.
const VIEW_INSET: f32 = 30.0;
const VIEW_H: f32 = COLUMN_H - 2.0 * VIEW_INSET;
/// `ScrollRect.scrollSensitivity` of the prefab, per wheel notch.
const WHEEL_STEP: f32 = 50.0;
/// `ScrollRect.elasticity` (Unity's default; the prefab's value was not read) and how far one frame's wheel steps can pull the content past its end.
const ELASTICITY: f32 = 0.1;
/// `ScrollRect.decelerationRate`.
const DECELERATION: f32 = 0.135;
/// How far the pointer must move before a press on a row becomes a drag of the list (`EventSystem.pixelDragThreshold`).
const DRAG_THRESHOLD: f32 = 10.0;
const ELASTIC_LIMIT: f32 = 150.0;
/// The scrollbar: 20 wide, 17 shorter than the view; the viewport gives up 17 for it (20 less the spacing of -3).
const BAR_W: f32 = 20.0;
const BAR_SHORT: f32 = 17.0;

/// Text sizes of the prefab.
const TITLE_PX: f32 = 30.0;
const LINE_PX: f32 = 25.0;
const FIELD_PX: f32 = 24.0;
const NAME_PX: f32 = 25.0;
const DATE_PX: f32 = 20.0;
const SAVE_PX: f32 = 60.0;
/// `TMP`'s centre of a line sits this far (in em) under the origin the software text is positioned by.
const ORIGIN: f32 = 0.585;

const INK: Rgba = [38, 38, 38, 255];
const FIELD_INK: Rgba = [50, 50, 50, 255];
const PLACEHOLDER: Rgba = [50, 50, 50, 128];
const WHITE: Rgba = [255, 255, 255, 255];
const NAVY: Rgba = [0, 0, 31, 255];
/// `Selectable.colors` of every button of the panel: highlighted and selected 0.961, pressed 0.784, disabled 0.784 at half alpha.
const HIGHLIGHT: Rgba = [245, 245, 245, 255];
const PRESSED: Rgba = [200, 200, 200, 255];
const DISABLED: Rgba = [200, 200, 200, 128];
/// The delete button's red cross and the replacement warning's bug.
const DELETE_RED: Rgba = [137, 7, 7, 255];
const BUG_RED: Rgba = [137, 0, 0, 255];
/// `TMP_InputField` defaults: the selection colour.
const SELECTION: Rgba = [168, 206, 255, 192];

/// The tint an image takes from the state of its button (`Selectable.colors` of the panels' buttons are Unity's defaults).
#[must_use]
pub(crate) fn button_tint(enabled: bool, pressed: bool, highlighted: bool) -> Rgba {
    if !enabled {
        DISABLED
    } else if pressed {
        PRESSED
    } else if highlighted {
        HIGHLIGHT
    } else {
        WHITE
    }
}

/// A `SaveItem` row of a list of saves (the Save Game panel's and the Load Game screen's): `base_icon` (multiplier 0.5) in the tint of its button,
/// the name on the left (Roboto-Bold 25, shrunk to the room the 200 wide date, the paddings of 15 and the spacing of 10 leave) and the date on
/// the right (Roboto-Bold 20).
pub(crate) fn draw_row(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    rect: [f32; 4],
    name: &str,
    date: &str,
    tint: Rgba,
) {
    let s = d.s;
    d.slice(frame, "base_icon", 5.0, 2.0, rect, tint);
    let mid = rect[1] + ROW_H / 2.0;
    let text = |frame: &mut UiFrame, line: &str, x: f32, size: f32| {
        frame.draws.push(UiDraw::Text {
            font: Some("Roboto-Bold SDF NoOutline".to_owned()),
            text: line.to_owned(),
            pos: d.at(x, mid - size * ORIGIN),
            px: size * s,
            color: INK,
            role: FontRole::Bold,
        });
    };
    let room = rect[2] - 30.0 - 10.0 - 200.0;
    let size = d.fitted(name, NAME_PX * s, true, room * s) / s;
    text(frame, name, rect[0] + 15.0, size);
    let date_w = d.measure(date, DATE_PX * s, true) / s;
    text(frame, date, rect[0] + rect[2] - 15.0 - date_w, DATE_PX);
}

/// What a control of the panel does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveAction {
    /// A click on the panel's background: whatever had the focus loses it (a click on a button or a row does that too).
    Background,
    /// The name field.
    Name,
    /// A row of the list.
    Row(u16),
    /// The Save button (`AskOverwritePreviousSave`).
    Save,
    /// The delete button (`AskDeleteSelectedSave`).
    Delete,
    /// The X (`ClosePanel`).
    Close,
}

/// A row of the list: a save of `Savefiles/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveRow {
    /// The file.
    pub path: PathBuf,
    /// `Path.GetFileNameWithoutExtension`.
    pub name: String,
    /// `FileInfo.LastWriteTime.ToString("dd/MM/yyyy HH:mm")`.
    pub date: String,
}

/// The open panel.
#[derive(Clone, Debug)]
pub struct SavePanel {
    /// `saveName.text`.
    pub name: String,
    /// The field is selected: typing goes to it.
    pub focused: bool,
    /// The caret and the selection (focusing the field selects all of it: the next character replaces it).
    pub caret: crate::text_field::Caret,
    /// A press on the focused field is being dragged (the selection follows the pointer).
    pub dragging: bool,
    /// Where the field's text was drawn last: the screen x of its start and of each character boundary after it (for placing the caret by a click).
    pub edges: std::cell::RefCell<(f32, Vec<f32>)>,
    /// Seconds since the last edit (the caret blinks from there).
    pub blink: f32,
    /// The list.
    pub rows: Vec<SaveRow>,
    /// `selectedSaveItem`.
    pub selected: Option<usize>,
    /// How far the list is scrolled (content units); below 0 or past `max_scroll` while the elastic list is overshooting.
    pub scroll: f32,
    /// The elastic pull-back's speed (`SmoothDamp`'s velocity).
    pub scroll_speed: f32,
    /// A press on the list that may become a drag of its content (`ScrollRect.OnBeginDrag`).
    pub drag: Option<ContentDrag>,
    /// The speed the content keeps after a drag (`ScrollRect.inertia`, `decelerationRate` 0.135 per second).
    pub inertia: f32,
    /// The scrollbar's handle is being dragged: where the pointer took hold of it (under the handle's top).
    pub bar_drag: Option<f32>,
    /// "hh:mm:ss" of the simulated time.
    pub time: String,
    /// The simulation's size as `FloatValueTextHandle` writes it ("3000 u").
    pub size: String,
    /// Bibites and eggs.
    pub bibites: usize,
    /// Pellets (`InformationPanel.pelletNumber`).
    pub pellets: usize,
    /// The world's picture, as it is drawn.
    pub preview: Option<Arc<UiTexture>>,
    /// The same as PNG bytes: the `img.png` of the save.
    pub preview_png: Option<Vec<u8>>,
    /// `Savefiles/`.
    dir: PathBuf,
}

/// `Savefiles/` of the app's storage and its rows.
fn read_rows(app: &App) -> Vec<SaveRow> {
    bibites_io::storage::list_saves(&app.storage, false)
        .unwrap_or_default()
        .into_iter()
        .map(|file| SaveRow {
            name: file
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_owned(),
            date: file
                .modified
                .map(crate::menu::format_save_date)
                .unwrap_or_default(),
            path: file.path,
        })
        .collect()
}

impl SavePanel {
    /// `SaveGamePanel.OpenPanel` (the game's state at this moment is read here).
    #[must_use]
    pub fn open(app: &App) -> Self {
        let [hours, minutes, seconds] = crate::time_keeper::parse_time(app.simulated_time());
        let counts = app.world.pellets.counts();
        let image = app.world_screenshot();
        Self {
            name: String::new(),
            focused: false,
            caret: crate::text_field::Caret::default(),
            dragging: false,
            edges: std::cell::RefCell::new((0.0, Vec::new())),
            blink: 0.0,
            rows: read_rows(app),
            selected: None,
            scroll: 0.0,
            scroll_speed: 0.0,
            drag: None,
            inertia: 0.0,
            bar_drag: None,
            time: format!("{hours:02}:{minutes:02}:{seconds:02}"),
            size: FloatValueFormat {
                prefix: "",
                units: "u",
                precision: 0,
                factor: 1.0,
                si: false,
                always_show_sign: false,
                precision_is_si: false,
                is_int: false,
            }
            .format(app.world.settings.independents.simulation_size),
            bibites: app.world.bibites.len() + app.world.eggs.len(),
            pellets: counts.plant_count as usize + counts.meat_count as usize,
            preview_png: image.as_ref().and_then(|image| image.to_png_bytes().ok()),
            preview: image.map(|image| {
                Arc::new(UiTexture {
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba,
                })
            }),
            dir: app.storage.saves_dir(),
        }
    }

    /// `containsABadCharacter.IsMatch(saveName.text)`.
    #[must_use]
    pub fn name_is_bad(&self) -> bool {
        self.name.chars().any(is_invalid_file_char)
    }

    /// `saveButton.interactable`: a name that is not empty and has no bad character.
    #[must_use]
    pub fn can_save(&self) -> bool {
        !self.name.is_empty() && !self.name_is_bad()
    }

    /// The file the world would be saved to (`SavePath + name + ".zip"`).
    #[must_use]
    pub fn save_path(&self) -> PathBuf {
        self.dir.join(format!("{}.zip", self.name))
    }

    /// `existIndicator.SetActive(File.Exists(path))`.
    #[must_use]
    pub fn replaces_a_save(&self) -> bool {
        !self.name_is_bad() && self.save_path().is_file()
    }

    /// `SaveGamePanel.SelectSaveItem`: the name becomes the row's file name. The button the player pressed has the selection, not the field.
    pub fn select(&mut self, row: usize) {
        let Some(entry) = self.rows.get(row) else {
            return;
        };
        self.selected = Some(row);
        self.name = entry.name.clone();
        self.focused = false;
        self.caret = crate::text_field::Caret::at_end(&self.name);
    }

    /// The field takes the focus (`TMP_InputField` with `onFocusSelectAll`): the whole text is selected. A press on a focused field puts the caret
    /// under the pointer instead ([`Self::press_at`]).
    pub fn focus(&mut self) {
        if !self.focused {
            self.caret.select_all(&self.name);
        }
        self.focused = true;
        self.blink = 0.0;
    }

    /// A press on the focused field: the caret goes to the boundary nearest to the screen x `x` (Shift keeps the selection's anchor) and a drag may follow.
    pub fn press_at(&mut self, x: f32, extend: bool) {
        let place = self.place_of(x);
        self.caret.press(place, extend);
        self.dragging = true;
        self.blink = 0.0;
    }

    fn place_of(&self, x: f32) -> usize {
        let (start, edges) = &*self.edges.borrow();
        if edges.is_empty() {
            return self.name.chars().count();
        }
        crate::text_field::nearest_edge(edges, x - start)
    }

    /// The pointer moved with the button held after [`Self::press_at`].
    pub fn drag_to(&mut self, x: f32) {
        if self.dragging {
            let place = self.place_of(x);
            self.caret.drag(place);
            self.blink = 0.0;
        }
    }

    /// Typed characters replace the selection (or go in at the caret); control characters are dropped.
    pub fn type_text(&mut self, text: &str) {
        let before = self.name.clone();
        self.caret.clamp(&self.name);
        self.caret.insert(&mut self.name, text);
        if self.name != before {
            self.blink = 0.0;
        }
    }

    /// Backspace deletes the selection, or the character before the caret.
    pub fn backspace(&mut self) {
        self.caret.clamp(&self.name);
        self.caret.backspace(&mut self.name);
        self.blink = 0.0;
    }

    /// Delete removes the selection, or the character after the caret.
    pub fn delete_forward(&mut self) {
        self.caret.clamp(&self.name);
        self.caret.delete(&mut self.name);
        self.blink = 0.0;
    }

    /// The height of the list's content: the padding, the rows 5 apart.
    #[must_use]
    pub fn content_height(&self) -> f32 {
        let rows = self.rows.len() as f32;
        let between = if self.rows.is_empty() {
            0.0
        } else {
            (rows - 1.0) * ROW_SPACING
        };
        CONTENT_PAD[2] + rows * ROW_H + between + CONTENT_PAD[3]
    }

    /// The farthest the list scrolls.
    #[must_use]
    pub fn max_scroll(&self) -> f32 {
        (self.content_height() - VIEW_H).max(0.0)
    }

    /// Whether the rows do not fit and the scrollbar is there.
    #[must_use]
    pub fn has_scrollbar(&self) -> bool {
        self.max_scroll() > 0.0
    }

    /// The list's scrollbar in `layout` (`Scrollbar Vertical`: 20 wide at the view's right edge, 17 short of its height).
    #[must_use]
    pub fn bar(&self, layout: &Layout) -> VBar {
        VBar::new(
            [
                layout.view[0] + layout.view[2] - BAR_W,
                layout.view[1],
                BAR_W,
                VIEW_H - BAR_SHORT,
            ],
            self.content_height(),
            VIEW_H,
            self.scroll.clamp(0.0, self.max_scroll()),
            Style::SAVES,
        )
    }

    /// The wheel moves the content by the scroll sensitivity per notch (a positive wheel is up). The prefab's `ScrollRect` is elastic: the content
    /// may go past its ends (`OnScroll` does not clamp it) and [`Self::spring`] pulls it back.
    pub fn wheel(&mut self, notches: f32) {
        if !self.has_scrollbar() {
            return;
        }
        let overshoot = ELASTIC_LIMIT;
        self.scroll = (self.scroll - notches * WHEEL_STEP).clamp(-overshoot, self.max_scroll() + overshoot);
    }

    /// `ScrollRect.LateUpdate` for the elastic movement: a content past its ends is pulled back by `SmoothDamp` with the `elasticity` as its smooth time.
    pub fn spring(&mut self, dt: f32) {
        // After a drag the content keeps moving and slows down (`inertia`, `decelerationRate` 0.135).
        if self.drag.is_none() && self.inertia.abs() > 1.0 {
            let inside = self.scroll >= 0.0 && self.scroll <= self.max_scroll();
            if inside {
                self.scroll += self.inertia * dt;
                self.inertia *= DECELERATION.powf(dt);
                if self.scroll < 0.0 || self.scroll > self.max_scroll() {
                    // Past an end the elastic spring takes the speed over.
                    self.scroll_speed = self.inertia;
                    self.inertia = 0.0;
                }
                return;
            }
        }
        if self.drag.is_none() {
            self.inertia = 0.0;
        } else {
            return;
        }
        let target = self.scroll.clamp(0.0, self.max_scroll());
        if (self.scroll - target).abs() < 0.01 {
            self.scroll = target;
            self.scroll_speed = 0.0;
            return;
        }
        // `Mathf.SmoothDamp` (critically damped spring).
        let smooth = ELASTICITY.max(0.0001);
        let omega = 2.0 / smooth;
        let x = omega * dt;
        let exp = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
        let change = self.scroll - target;
        let temp = (self.scroll_speed + omega * change) * dt;
        self.scroll_speed = (self.scroll_speed - omega * temp) * exp;
        self.scroll = target + (change + temp) * exp;
    }

    /// `DeleteSelectedSave` once the file is gone: the list is read again and the row at the same place (the last one when it was the last)
    /// is selected, its name into the field; with no row left nothing is selected and the field keeps its text.
    pub fn after_delete(&mut self, rows: Vec<SaveRow>, index: usize) {
        self.rows = rows;
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        self.selected = None;
        if let Some(last) = self.rows.len().checked_sub(1) {
            self.select(index.min(last));
        }
    }
}

/// The rectangles (`[x, y, w, h]`, reference px) of the panel's parts, which only depend on the width of the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The list's panel.
    pub list: [f32; 4],
    /// The scroll view inside it.
    pub view: [f32; 4],
    /// The selected panel.
    pub selected: [f32; 4],
    /// The delete button (60 x 60 at 20, 20 of the panel's corner).
    pub delete: [f32; 4],
    /// The close button (50 x 50 at 20, 20 of the panel's top right corner).
    pub close: [f32; 4],
    /// The holder of the picture.
    pub holder: [f32; 4],
    /// The picture's square (`base_icon`).
    pub picture: [f32; 4],
    /// The raw image inside it, inset 10.
    pub image: [f32; 4],
    /// "New Save".
    pub title: [f32; 4],
    /// The name field.
    pub input: [f32; 4],
    /// "Informations".
    pub stats_title: [f32; 4],
    /// The line of the time and the size.
    pub sim_line: [f32; 4],
    /// The line of the numbers.
    pub number_line: [f32; 4],
    /// The Save button.
    pub save: [f32; 4],
    /// The bug icon beside it.
    pub indicator: [f32; 4],
}

/// The prefab's layout at a canvas `ref_width` units wide (1920 at 16:9).
#[must_use]
pub fn layout(ref_width: f32) -> Layout {
    // The two columns share what the minimum widths leave: padding 50 each side of a root 10 wider than the canvas, spacing 50.
    let free = ref_width + 10.0 - 100.0 - 50.0 - LIST_MIN - SELECTED_MIN;
    let list_w = LIST_MIN + free / 2.0;
    let selected_w = SELECTED_MIN + free / 2.0;
    let list = [SIDE, SIDE, list_w, COLUMN_H];
    let selected_x = SIDE + list_w + 50.0;
    let selected = [selected_x, SIDE, selected_w, COLUMN_H];
    let view = [
        list[0] + VIEW_INSET,
        list[1] + VIEW_INSET,
        list_w - 2.0 * VIEW_INSET,
        VIEW_H,
    ];
    // The vertical group: padding 45 on top and 20 below, the picture's holder takes what the lines and the button leave.
    let inner_x = selected_x + 50.0;
    let inner_w = selected_w - 100.0;
    let line30 = TITLE_PX * LINE_K;
    let line25 = LINE_PX * LINE_K;
    let list_h = line30 + 20.0 + 45.0 + 20.0 + line30 + 20.0 + line25 + 20.0 + line25;
    let button_h = 100.0;
    let top = SIDE + 45.0;
    let bottom = SIDE + COLUMN_H - 20.0;
    let holder_h = bottom - top - 2.0 * SPACING - list_h - button_h;
    let holder = [inner_x, top, inner_w, holder_h];
    let picture = [
        inner_x + (inner_w - holder_h) / 2.0,
        top,
        holder_h,
        holder_h,
    ];
    let image = [
        picture[0] + 10.0,
        picture[1] + 10.0,
        holder_h - 20.0,
        holder_h - 20.0,
    ];
    // The list of lines: padding 50 left and right, spacing 20.
    let lines_x = inner_x + 50.0;
    let lines_w = inner_w - 100.0;
    let lines_top = top + holder_h + SPACING;
    let title = [lines_x, lines_top, lines_w, line30];
    let input = [lines_x, title[1] + line30 + 20.0, lines_w, 45.0];
    let stats_title = [lines_x, input[1] + 45.0 + 20.0, lines_w, line30];
    let sim_line = [lines_x, stats_title[1] + line30 + 20.0, lines_w, line25];
    let number_line = [lines_x, sim_line[1] + line25 + 20.0, lines_w, line25];
    let save = [
        inner_x + (inner_w - 300.0) / 2.0,
        bottom - button_h,
        300.0,
        button_h,
    ];
    Layout {
        list,
        view,
        selected,
        delete: [selected_x + 20.0, SIDE + 20.0, 60.0, 60.0],
        close: [
            selected_x + selected_w - 20.0 - 50.0,
            SIDE + 20.0,
            50.0,
            50.0,
        ],
        holder,
        picture,
        image,
        title,
        input,
        stats_title,
        sim_line,
        number_line,
        save,
        indicator: [
            save[0] + save[2] + 40.0,
            save[1] + (save[3] - 80.0) / 2.0,
            80.0,
            80.0,
        ],
    }
}

impl App {
    /// `SceneScreenShotHandler`: the whole simulation square from above, 400 x 400 (the camera's orthographic size is the simulation's).
    #[must_use]
    pub(crate) fn world_screenshot(&self) -> Option<crate::render::Image> {
        if self.world_away() {
            // The world is on the simulation thread: what the app holds is a husk.
            return None;
        }
        let size = self.world.settings.independents.simulation_size;
        let side = PICTURE as f32;
        let camera = Camera {
            center: Vec2::ZERO,
            zoom: crate::render::ortho_to_zoom(size, side),
            screen: Vec2::new(side, side),
        };
        let options = SceneOptions {
            grid: false,
            fov: false,
            pheromones: false,
            ..SceneOptions::default()
        };
        let mut scene = Scene::new();
        crate::render::build_scene(
            &mut scene,
            &self.world,
            &camera,
            None,
            &options,
            self.assets.as_ref(),
            None,
        );
        let atlases: &[crate::assets::Atlas] = self
            .assets
            .as_ref()
            .map_or(&[], |assets| assets.atlases.as_slice());
        Some(crate::render::render_scene(
            &scene, &camera, PICTURE, PICTURE, 2, atlases,
        ))
    }

    /// The `img.png` a save made now carries: the picture the open panel took, else a new one.
    #[must_use]
    pub(crate) fn save_picture(&self) -> Option<Vec<u8>> {
        self.save_panel
            .as_ref()
            .and_then(|panel| panel.preview_png.clone())
            .or_else(|| self.world_screenshot()?.to_png_bytes().ok())
    }

    /// `SaveGamePanel.OpenPanel`.
    pub(crate) fn open_save_panel(&mut self) {
        let panel = SavePanel::open(self);
        // `SaveGamePanel.FillList`: a save that cannot be read raises the "Corrupted Save" error popup.
        for row in &panel.rows {
            // A save that has not changed since it was last read keeps its verdict (a 700 MB one takes a second to open).
            if let Some(reason) = crate::menu::checked_save_problem(&row.path) {
                self.error_popups.push(("Corrupted Save".to_owned(), crate::error_popup::corrupted_text(&row.path, &reason)));
            }
        }
        self.save_panel = Some(panel);
        self.panels.open(PanelKind::SaveGame);
    }

    /// The panel's state goes with the panel (Escape, the X and a save close it through the stack).
    pub(crate) fn sync_save_panel(&mut self) {
        if self.save_panel.is_some() && !self.panels.is_open(PanelKind::SaveGame) {
            self.save_panel = None;
        }
    }

    /// A control of the panel was pressed.
    pub(crate) fn save_panel_action(&mut self, action: SaveAction) {
        let Some(panel) = self.save_panel.as_mut() else {
            return;
        };
        match action {
            SaveAction::Background => panel.focused = false,
            SaveAction::Name => {
                if panel.focused {
                    panel.press_at(self.cursor.x, self.shift_held);
                } else {
                    panel.focus();
                }
            }
            SaveAction::Row(row) => panel.select(usize::from(row)),
            SaveAction::Close => {
                self.panels.close(PanelKind::SaveGame);
                self.save_panel = None;
            }
            SaveAction::Save => {
                panel.focused = false;
                self.save_panel_save();
            }
            SaveAction::Delete => {
                panel.focused = false;
                self.save_panel_delete();
            }
        }
    }

    /// `AskOverwritePreviousSave`: a name that is taken asks first; a free one is saved at once. A button that is not interactable does nothing.
    pub(crate) fn save_panel_save(&mut self) {
        let Some(panel) = self.save_panel.as_ref() else {
            return;
        };
        if !panel.can_save() {
            return;
        }
        let name = panel.name.clone();
        if panel.replaces_a_save() {
            self.confirm_do_not_show = false;
            self.confirm_dialog = Some(ConfirmKind::OverwriteSave(name));
            self.last_action = "confirm: overwrite save?".to_owned();
        } else {
            self.save_as_to(&name);
        }
    }

    /// `AskDeleteSelectedSave`: the warning (unless it was switched off), then the file goes. With no row selected the game's `DeleteSelectedSave`
    /// has nothing to delete (a null selection), so nothing happens here either.
    pub(crate) fn save_panel_delete(&mut self) {
        let Some(path) = self
            .save_panel
            .as_ref()
            .and_then(|panel| panel.rows.get(panel.selected?))
            .map(|row| row.path.clone())
        else {
            return;
        };
        if self.warning_off(crate::warnings::ActionWarning::DeleteSaveFile) {
            self.delete_save_path(&path);
            return;
        }
        self.confirm_do_not_show = false;
        self.confirm_dialog = Some(ConfirmKind::DeleteSave(path));
        self.last_action = "confirm: delete save?".to_owned();
    }

    /// The file of a row is gone: the list follows (`DeleteSelectedSave`).
    pub(crate) fn save_panel_after_delete(&mut self, path: &Path) {
        let rows = match self.save_panel.as_ref() {
            Some(_) => read_rows(self),
            None => return,
        };
        let Some(panel) = self.save_panel.as_mut() else {
            return;
        };
        let index = panel
            .rows
            .iter()
            .position(|row| row.path == path)
            .or(panel.selected)
            .unwrap_or(0);
        panel.after_delete(rows, index);
    }

    /// Typing into the name field: characters and Ctrl + V go in, Backspace deletes, Enter ends the editing.
    pub(crate) fn edit_save_name(&mut self, input: &FrameInput) {
        let Some(panel) = self.save_panel.as_mut() else {
            return;
        };
        panel.blink += input.dt;
        panel.spring(input.dt);
        if !panel.focused {
            return;
        }
        let typed = std::mem::take(&mut self.typed);
        panel.type_text(&typed);
        if input.paste_tag {
            let pasted = self.clipboard_in.take().unwrap_or_default();
            panel.type_text(&pasted);
        }
        if input.backspace {
            panel.backspace();
        }
        if input.key_delete {
            panel.delete_forward();
        }
        let extend = input.shift_down;
        if input.key_left {
            panel.caret.left(extend);
            panel.blink = 0.0;
        }
        if input.key_right {
            panel.caret.right(&panel.name, extend);
            panel.blink = 0.0;
        }
        if input.key_home {
            panel.caret.home(extend);
            panel.blink = 0.0;
        }
        if input.key_end {
            panel.caret.end(&panel.name, extend);
            panel.blink = 0.0;
        }
        if input.select_all_key {
            panel.caret.select_all(&panel.name);
            panel.blink = 0.0;
        }
        if input.primary_down {
            panel.drag_to(input.cursor.x);
        } else {
            panel.dragging = false;
        }
        if input.enter {
            panel.focused = false;
            panel.caret = crate::text_field::Caret::at_end(&panel.name);
        }
    }

    /// Test helper: the Save Game panel opened (when it is not), `name` put in its field and the Save button pressed.
    #[cfg(test)]
    pub(crate) fn save_named(&mut self, name: &str) {
        if self.save_panel.is_none() {
            self.open_save_panel();
        }
        if let Some(panel) = self.save_panel.as_mut() {
            panel.name = name.to_owned();
        }
        self.save_panel_save();
    }

    /// The pointer on the list's scrollbar (the window loop hands over the raw pointer): a press on the handle takes hold of it, one on the track
    /// moves the list a page toward the pointer, and the held button drags the handle.
    pub(crate) fn save_panel_pointer(&mut self, cursor: Vec2, pressed: bool, down: bool, dt: f32) -> Option<SaveAction> {
        let panel = self.save_panel.as_mut()?;
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let at = cursor / scale;
        let bar = panel.bar(&layout(self.camera.screen.x / scale));
        let mut clicked = None;
        // The content follows the pointer once it has moved past the threshold; a press that did not move is the click it was.
        if let Some(mut drag) = panel.drag {
            if down {
                if !drag.moved && (at.y - drag.start_y).abs() > DRAG_THRESHOLD {
                    drag.moved = true;
                }
                if drag.moved {
                    let wanted = drag.start_scroll - (at.y - drag.start_y);
                    let max = panel.max_scroll();
                    let over = if wanted < 0.0 { wanted } else if wanted > max { wanted - max } else { 0.0 };
                    let base = wanted.clamp(0.0, max);
                    let before = panel.scroll;
                    panel.scroll = if over == 0.0 { base } else { base + rubber(over, VIEW_H) };
                    let dt = dt.max(1e-4);
                    panel.inertia = panel.inertia * 0.5 + (panel.scroll - before) / dt * 0.5;
                    drag.last_y = at.y;
                }
                panel.drag = Some(drag);
            } else {
                panel.drag = None;
                if !drag.moved {
                    clicked = Some(drag.click);
                    panel.inertia = 0.0;
                }
            }
        }
        if pressed {
            match bar.press(at) {
                Some(Press::Handle { grab }) => panel.bar_drag = Some(grab),
                Some(Press::Page(direction)) => panel.scroll = bar.paged(direction),
                None => {}
            }
        }
        if let Some(grab) = panel.bar_drag {
            if down {
                panel.scroll = bar.scroll_for_handle_top(at.y - grab);
            } else {
                panel.bar_drag = None;
            }
        }
        clicked
    }

    /// A press on a row of the list is held back: it becomes a click when released without a drag (or the start of a drag).
    pub(crate) fn save_panel_hold_press(&mut self, cursor: Vec2, click: SaveAction) {
        let Some(panel) = self.save_panel.as_mut() else {
            return;
        };
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let at = cursor / scale;
        panel.inertia = 0.0;
        panel.drag = Some(ContentDrag { start_y: at.y, start_scroll: panel.scroll, last_y: at.y, moved: false, click });
    }

    /// The wheel over the list scrolls it (the window loop hands it over, and the camera does not zoom).
    pub(crate) fn save_panel_wheel(&mut self, cursor: Vec2, wheel: f32) {
        if wheel == 0.0 {
            return;
        }
        let Some(panel) = self.save_panel.as_mut() else {
            return;
        };
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let at = cursor / scale;
        let list = layout(self.camera.screen.x / scale).list;
        if at.x >= list[0]
            && at.x < list[0] + list[2]
            && at.y >= list[1]
            && at.y < list[1] + list[3]
        {
            panel.wheel(wheel);
        }
    }
}

/// A press on the list: where it began and what it would click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentDrag {
    start_y: f32,
    start_scroll: f32,
    last_y: f32,
    moved: bool,
    click: SaveAction,
}

/// `ScrollRect.RubberDelta`: how far the content follows the pointer past an end.
fn rubber(over: f32, view: f32) -> f32 {
    (1.0 - 1.0 / (over.abs() * 0.55 / view + 1.0)) * view * over.signum()
}

/// Draws the panel when it is the newest one (over the pause menu it was opened from).
pub fn draw(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&SdfFonts>,
    width: f32,
    height: f32,
) {
    let Some(panel) = app
        .save_panel
        .as_ref()
        .filter(|_| app.panels.top() == Some(PanelKind::SaveGame))
    else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let d = Drawer::new(s, ui, metrics, app.cursor);
    let ref_width = width / s;
    let layout = layout(ref_width);
    let pressed = app.pointer_down;
    let over = |rect: [f32; 4]| {
        let at = app.cursor / s;
        at.x >= rect[0] && at.x < rect[0] + rect[2] && at.y >= rect[1] && at.y < rect[1] + rect[3]
    };
    let state_tint = |rect: [f32; 4], enabled: bool, selected: bool| -> Rgba {
        button_tint(enabled, over(rect) && pressed, over(rect) || selected)
    };
    let hit = |frame: &mut UiFrame, id: u32, rect: [f32; 4], action: SaveAction| {
        frame.hit(
            id,
            d.at(rect[0], rect[1]),
            d.at(rect[2], rect[3]),
            UiAction::SavePanel(action),
        );
    };
    // The opaque root: nothing under it shows or takes a click.
    frame.rect(Vec2::ZERO, Vec2::new(width, height), NAVY);
    frame.hit(
        HIT_BACK,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::SavePanel(SaveAction::Background),
    );
    let put = |frame: &mut UiFrame,
               line: &str,
               x: f32,
               mid: f32,
               size: f32,
               font: &str,
               role: FontRole,
               color: Rgba| {
        frame.draws.push(UiDraw::Text {
            font: Some(font.to_owned()),
            text: line.to_owned(),
            pos: d.at(x, mid - size * ORIGIN),
            px: size * s,
            color,
            role,
        });
    };
    let bold = |frame: &mut UiFrame, line: &str, x: f32, mid: f32, size: f32, color: Rgba| {
        put(
            frame,
            line,
            x,
            mid,
            size,
            "Roboto-Bold SDF NoOutline",
            FontRole::Bold,
            color,
        );
    };
    let wide = |line: &str, size: f32, bold: bool| d.measure(line, size * s, bold) / s;

    // --- The list --------------------------------------------------------------------------------------------------------------------
    d.slice(frame, "gray_panel", 10.0, 4.0, layout.list, WHITE);
    let bar = panel.has_scrollbar();
    let row_w =
        layout.view[2] - CONTENT_PAD[0] - CONTENT_PAD[1] - if bar { BAR_W - 3.0 } else { 0.0 };
    let top = layout.view[1];
    let mut masked = false;
    for (index, row) in panel.rows.iter().enumerate() {
        let y = top + CONTENT_PAD[2] + index as f32 * (ROW_H + ROW_SPACING) - panel.scroll;
        // The rows at the edges of the view are cut by its mask (`Mask` of the viewport).
        if y + ROW_H <= top || y >= top + VIEW_H {
            continue;
        }
        if !masked {
            let clip = [layout.view[0], top, layout.view[2], VIEW_H];
            frame.draws.push(UiDraw::Clip(Some([d.at(clip[0], clip[1]).x, d.at(clip[0], clip[1]).y, clip[2] * s, clip[3] * s])));
            masked = true;
        }
        let rect = [layout.view[0] + CONTENT_PAD[0], y, row_w, ROW_H];
        let selected = panel.selected == Some(index) && !panel.focused;
        draw_row(
            frame,
            &d,
            rect,
            &row.name,
            &row.date,
            state_tint(rect, true, selected),
        );
        // Only the part inside the view takes the pointer.
        let (hit_top, hit_bottom) = (rect[1].max(top), (rect[1] + rect[3]).min(top + VIEW_H));
        hit(
            frame,
            HIT_ROWS + index as u32,
            [rect[0], hit_top, rect[2], hit_bottom - hit_top],
            SaveAction::Row(index as u16),
        );
    }
    if masked {
        frame.draws.push(UiDraw::Clip(None));
    }
    if bar {
        let scrollbar = panel.bar(&layout);
        let handle = scrollbar.handle();
        let tint = state_tint(handle, true, false);
        let dragging = panel.bar_drag.is_some();
        scrollbar.draw(frame, &d, if dragging { PRESSED } else { tint });
    }

    // --- The selected panel --------------------------------------------------------------------------------------------------------------
    d.slice(frame, "base_icon", 5.0, 4.0, layout.selected, WHITE);
    // The delete button: `base_icon` (multiplier 0.65) with a red `exit_icon`.
    let delete_enabled = true;
    d.slice(
        frame,
        "base_icon",
        5.0,
        1.0 / 0.65,
        layout.delete,
        state_tint(layout.delete, delete_enabled, false),
    );
    d.icon(frame, "exit_icon", layout.delete, DELETE_RED);
    hit(frame, HIT_DELETE, layout.delete, SaveAction::Delete);
    d.icon(
        frame,
        "1727490",
        layout.close,
        state_tint(layout.close, true, false),
    );
    hit(frame, HIT_CLOSE, layout.close, SaveAction::Close);
    // The picture of the world.
    d.slice(frame, "base_icon", 5.0, 4.0, layout.picture, WHITE);
    if let Some(texture) = panel.preview.as_ref() {
        frame.draws.push(UiDraw::Pixels {
            texture: Arc::clone(texture),
            pos: d.at(layout.image[0], layout.image[1]),
            size: d.at(layout.image[2], layout.image[3]),
        });
    }
    // The lines.
    let line30 = TITLE_PX * LINE_K;
    let line25 = LINE_PX * LINE_K;
    bold(
        frame,
        "New Save",
        layout.title[0],
        layout.title[1] + line30 / 2.0,
        TITLE_PX,
        INK,
    );
    draw_input(
        frame,
        &d,
        panel,
        layout.input,
        over(layout.input),
        pressed,
        s,
    );
    hit(frame, HIT_NAME, layout.input, SaveAction::Name);
    let heading = "Informations";
    bold(
        frame,
        heading,
        layout.stats_title[0] + (layout.stats_title[2] - wide(heading, TITLE_PX, true)) / 2.0,
        layout.stats_title[1] + line30 / 2.0,
        TITLE_PX,
        INK,
    );
    // The time and the size: the time group is as wide as its two texts, the size group takes the rest (its label is the flexible one, its
    // value is at least 65 wide), 25 apart, the groups' texts right aligned in their boxes.
    let mid = layout.sim_line[1] + line25 / 2.0;
    let (time_label, size_label) = ("Sim Time:", "Sim Size:");
    let time_w = wide(time_label, LINE_PX, true) + wide(&panel.time, LINE_PX, true);
    bold(frame, time_label, layout.sim_line[0], mid, LINE_PX, INK);
    bold(
        frame,
        &panel.time,
        layout.sim_line[0] + wide(time_label, LINE_PX, true),
        mid,
        LINE_PX,
        INK,
    );
    let right = layout.sim_line[0] + layout.sim_line[2];
    let value_w = wide(&panel.size, LINE_PX, true).max(65.0);
    bold(
        frame,
        &panel.size,
        right - wide(&panel.size, LINE_PX, true),
        mid,
        LINE_PX,
        [0, 0, 0, 255],
    );
    let label_right = right - value_w - 10.0;
    bold(
        frame,
        size_label,
        (label_right - wide(size_label, LINE_PX, true)).max(layout.sim_line[0] + time_w + 25.0),
        mid,
        LINE_PX,
        INK,
    );
    // The numbers: bibites (label and count) and pellets (flexible label, count).
    let mid = layout.number_line[1] + line25 / 2.0;
    let (bibites_label, pellets_label) = ("Number of Bibites: ", "Number of Pellets: ");
    let (bibites, pellets) = (panel.bibites.to_string(), panel.pellets.to_string());
    let bibites_w = wide(bibites_label, LINE_PX, true) + wide(&bibites, LINE_PX, true);
    bold(
        frame,
        bibites_label,
        layout.number_line[0],
        mid,
        LINE_PX,
        INK,
    );
    bold(
        frame,
        &bibites,
        layout.number_line[0] + wide(bibites_label, LINE_PX, true),
        mid,
        LINE_PX,
        INK,
    );
    let right = layout.number_line[0] + layout.number_line[2];
    let count_w = wide(&pellets, LINE_PX, true);
    bold(frame, &pellets, right - count_w, mid, LINE_PX, INK);
    bold(
        frame,
        pellets_label,
        (right - count_w - wide(pellets_label, LINE_PX, true))
            .max(layout.number_line[0] + bibites_w + 25.0),
        mid,
        LINE_PX,
        INK,
    );
    // The Save button and, when the name is taken, the bug icon of `replacementIndicator`.
    let can_save = panel.can_save();
    d.slice(
        frame,
        "base_icon",
        5.0,
        4.0,
        layout.save,
        state_tint(layout.save, can_save, false),
    );
    let label = "Save";
    let size = d.fitted(label, SAVE_PX * s, true, layout.save[2] * s * 0.95) / s;
    put(
        frame,
        label,
        layout.save[0] + (layout.save[2] - wide(label, size, true)) / 2.0,
        layout.save[1] + layout.save[3] / 2.0,
        size,
        "Boston Black SDF",
        FontRole::Bold,
        INK,
    );
    hit(
        frame,
        HIT_SAVE,
        layout.save,
        if can_save {
            SaveAction::Save
        } else {
            SaveAction::Background
        },
    );
    if panel.replaces_a_save() {
        d.icon(frame, "bug_icon", layout.indicator, BUG_RED);
        hit(
            frame,
            HIT_INDICATOR,
            layout.indicator,
            if can_save {
                SaveAction::Save
            } else {
                SaveAction::Background
            },
        );
    }
}

/// The name field: a `base_icon` (multiplier 1) with the text (Roboto-Regular 24) in a text area 10 in from the sides, the placeholder while
/// it is empty, the select-all highlight and the blinking caret while it has the focus.
fn draw_input(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    panel: &SavePanel,
    rect: [f32; 4],
    hovered: bool,
    pressed: bool,
    s: f32,
) {
    let tint = if hovered && pressed {
        PRESSED
    } else if hovered || panel.focused {
        HIGHLIGHT
    } else {
        WHITE
    };
    d.slice(frame, "base_icon", 5.0, 1.0, rect, tint);
    let (x, mid) = (rect[0] + 10.0, rect[1] + rect[3] / 2.0);
    let wide = |line: &str| d.measure(line, FIELD_PX * s, false) / s;
    let put = |frame: &mut UiFrame, line: &str, color: Rgba| {
        frame.draws.push(UiDraw::Text {
            font: Some("Roboto-Regular SDF".to_owned()),
            text: line.to_owned(),
            pos: d.at(x, mid - FIELD_PX * ORIGIN),
            px: FIELD_PX * s,
            color,
            role: FontRole::Body,
        });
    };
    // The boundaries of the characters, kept for placing the caret with a click.
    let mut edges = vec![0.0f32];
    let mut prefix = String::new();
    for character in panel.name.chars() {
        prefix.push(character);
        edges.push(wide(&prefix));
    }
    *panel.edges.borrow_mut() = (d.at(x, 0.0).x, edges.iter().map(|edge| edge * s).collect());
    let at_edge = |place: usize| edges.get(place).copied().unwrap_or_else(|| edges.last().copied().unwrap_or(0.0));
    if panel.name.is_empty() {
        put(frame, "Enter name...", PLACEHOLDER);
    } else {
        if panel.focused
            && let Some((from, to)) = panel.caret.selection()
        {
            frame.rect(
                d.at(x + at_edge(from), mid - FIELD_PX * LINE_K / 2.0),
                d.at(at_edge(to) - at_edge(from), FIELD_PX * LINE_K),
                SELECTION,
            );
        }
        put(frame, &panel.name, FIELD_INK);
    }
    // `caretBlinkRate` 0.85: the caret shows for the first half of each blink and restarts with every edit.
    if panel.focused && (panel.blink * 0.85) % 1.0 < 0.5 {
        frame.rect(
            d.at(x + at_edge(panel.caret.pos), mid - FIELD_PX * LINE_K / 2.0),
            d.at(1.5, FIELD_PX * LINE_K),
            FIELD_INK,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prefab's stored sizes at 1920 x 1080.
    #[test]
    fn the_layout_is_the_prefabs_at_full_hd() {
        let layout = layout(1920.0);
        // `SavesList` 895 x 990 and `SelectedPanel` 885 x 990, 50 apart, 45 from the corner.
        assert_eq!(layout.list, [45.0, 45.0, 895.0, 990.0]);
        assert_eq!(layout.selected, [990.0, 45.0, 885.0, 990.0]);
        // The scroll view is inset 30.
        assert_eq!(layout.view, [75.0, 75.0, 835.0, 930.0]);
        // Delete and close at 20 from the corners of the panel.
        assert_eq!(layout.delete, [1010.0, 65.0, 60.0, 60.0]);
        assert_eq!(layout.close, [1805.0, 65.0, 50.0, 50.0]);
        // The holder of the picture 785 x 491.08, the picture its height square, the raw image inset 10.
        assert_eq!(layout.holder[2], 785.0);
        assert!(
            (layout.holder[3] - 491.08).abs() < 0.01,
            "{:?}",
            layout.holder
        );
        assert!(
            (layout.picture[0] - 1186.96).abs() < 0.01,
            "{:?}",
            layout.picture
        );
        assert!((layout.picture[2] - 491.08).abs() < 0.01);
        assert!((layout.image[2] - 471.08).abs() < 0.01);
        // The list of lines is 253.92 high, under the holder and 40 from the button.
        let lines_bottom = layout.number_line[1] + layout.number_line[3];
        assert!(
            (lines_bottom - (layout.holder[1] + layout.holder[3] + 40.0 + 253.92)).abs() < 0.01
        );
        assert!((layout.save[1] - (lines_bottom + 40.0)).abs() < 0.01);
        assert_eq!(layout.save[1] + layout.save[3], 1015.0);
        // The title, the name field and the others, at the stored places.
        assert!(
            (layout.title[1] - 621.08).abs() < 0.01,
            "{:?}",
            layout.title
        );
        assert!(
            (layout.input[1] - 676.24).abs() < 0.01,
            "{:?}",
            layout.input
        );
        assert_eq!(layout.input[2], 685.0);
        assert!((layout.stats_title[1] - 741.24).abs() < 0.01);
        assert!((layout.sim_line[1] - 796.4).abs() < 0.01);
        assert!((layout.number_line[1] - 845.7).abs() < 0.01);
        assert_eq!(layout.save[2..], [300.0, 100.0]);
        // The bug icon 80 x 80 (the stored size), 40 to the right of the button.
        assert_eq!(layout.indicator[2..], [80.0, 80.0]);
        assert_eq!(layout.indicator[0], layout.save[0] + 300.0 + 40.0);
    }

    #[test]
    fn the_columns_follow_the_width_of_the_screen_not_the_texts() {
        // 21:9: 2520 wide.
        let wide = layout(2520.0);
        assert_eq!(
            wide.list[2] + wide.selected[2] + 50.0,
            2520.0 + 10.0 - 100.0
        );
        assert_eq!(wide.list[2] - wide.selected[2], 10.0);
        // The right column's contents stay as tall.
        assert!((wide.holder[3] - 491.08).abs() < 0.01);
    }

    fn panel() -> SavePanel {
        SavePanel {
            name: String::new(),
            focused: false,
            caret: crate::text_field::Caret::default(),
            dragging: false,
            edges: std::cell::RefCell::new((0.0, Vec::new())),
            blink: 0.0,
            rows: ["alpha", "beta", "gamma"]
                .into_iter()
                .map(|name| SaveRow {
                    path: PathBuf::from(format!("{name}.zip")),
                    name: name.to_owned(),
                    date: "01/01/2026 10:00".to_owned(),
                })
                .collect(),
            selected: None,
            scroll: 0.0,
            scroll_speed: 0.0,
            drag: None,
            inertia: 0.0,
            bar_drag: None,
            time: "00:00:00".to_owned(),
            size: "3000 u".to_owned(),
            bibites: 3,
            pellets: 4,
            preview: None,
            preview_png: None,
            dir: PathBuf::from("no-such-folder"),
        }
    }

    #[test]
    fn the_save_button_needs_a_name_without_bad_characters() {
        let mut panel = panel();
        // Opening: an empty name and a disabled button.
        assert!(!panel.can_save());
        panel.name = "good name".to_owned();
        assert!(panel.can_save());
        // `Path.GetInvalidFileNameChars()`.
        for bad in [
            '"', '<', '>', '|', ':', '*', '?', '\\', '/', '\u{1}', '\u{1f}',
        ] {
            panel.name = format!("a{bad}b");
            assert!(!panel.can_save(), "{bad:?}");
            assert!(panel.name_is_bad());
        }
        // Spaces and dots are fine; the game does not trim.
        panel.name = " . ".to_owned();
        assert!(panel.can_save());
    }

    #[test]
    fn a_row_puts_its_name_in_the_field_and_takes_the_selection_from_the_field() {
        let mut panel = panel();
        panel.focus();
        assert!(panel.focused);
        panel.select(1);
        assert_eq!(panel.name, "beta");
        assert_eq!(panel.selected, Some(1));
        assert!(!panel.focused, "the button has the selection now");
        // A row that is not there does nothing.
        panel.select(9);
        assert_eq!(panel.name, "beta");
    }

    #[test]
    fn typing_replaces_what_the_focus_selected_and_backspace_deletes_it() {
        let mut panel = panel();
        panel.select(0);
        assert_eq!(panel.name, "alpha");
        // A click on the field selects all: the first character replaces the text, the next ones follow.
        panel.focus();
        assert!(panel.caret.selection().is_some());
        panel.type_text("ne");
        panel.type_text("w\n\t");
        assert_eq!(panel.name, "new", "control characters are dropped");
        // A second click puts the caret in: nothing is selected, characters go to the end.
        panel.focus();
        assert!(panel.caret.selection().is_none());
        panel.type_text("er");
        assert_eq!(panel.name, "newer");
        panel.backspace();
        assert_eq!(panel.name, "newe");
        // Select all, then Backspace: the whole text goes.
        panel.focused = false;
        panel.focus();
        panel.backspace();
        assert_eq!(panel.name, "");
        assert!(panel.caret.selection().is_none());
        // Every edit restarts the caret.
        panel.blink = 3.0;
        panel.type_text("x");
        assert_eq!(panel.blink, 0.0);
    }

    #[test]
    fn the_caret_keys_edit_inside_the_name_and_a_press_on_the_focused_field_places_the_caret() {
        let (mut app, dir) = app_in("caret");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Name)]);
        app.typed = "abcdef".to_owned();
        let key = |f: &dyn Fn(&mut FrameInput)| {
            let mut input = FrameInput { dt: 0.016, ..FrameInput::default() };
            f(&mut input);
            input
        };
        app.handle_input(&key(&|_| {}));
        assert_eq!(app.save_panel.as_ref().expect("panel").name, "abcdef");
        app.handle_input(&key(&|i| i.key_left = true));
        app.handle_input(&key(&|i| i.key_left = true));
        app.typed = "X".to_owned();
        app.handle_input(&key(&|_| {}));
        assert_eq!(app.save_panel.as_ref().expect("panel").name, "abcdXef");
        app.handle_input(&key(&|i| i.key_delete = true));
        assert_eq!(app.save_panel.as_ref().expect("panel").name, "abcdXf");
        app.handle_input(&key(&|i| i.key_home = true));
        app.handle_input(&key(&|i| {
            i.key_right = true;
            i.shift_down = true;
        }));
        app.handle_input(&key(&|i| {
            i.key_right = true;
            i.shift_down = true;
        }));
        app.handle_input(&key(&|i| i.backspace = true));
        assert_eq!(app.save_panel.as_ref().expect("panel").name, "cdXf");
        app.handle_input(&key(&|i| i.select_all_key = true));
        assert_eq!(app.save_panel.as_ref().expect("panel").caret.selection(), Some((0, 4)));
        // A press on the focused field puts the caret under the pointer: the characters' edges are known once it was drawn.
        {
            let panel = app.save_panel.as_ref().expect("panel");
            *panel.edges.borrow_mut() = (100.0, vec![0.0, 10.0, 20.0, 30.0, 40.0]);
        }
        app.cursor = Vec2::new(121.0, 0.0);
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Name)]);
        let panel = app.save_panel.as_ref().expect("panel");
        assert_eq!((panel.caret.pos, panel.caret.selection()), (2, None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dragging_the_rows_moves_the_list_keeps_moving_and_a_still_press_is_a_click() {
        let (mut app, dir) = app_in("drag");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        let panel = app.save_panel.as_mut().expect("panel");
        panel.rows = (0..40)
            .map(|index| SaveRow { path: PathBuf::from(format!("{index}.zip")), name: index.to_string(), date: String::new() })
            .collect();
        panel.scroll = 500.0;
        let scale = UiFrame::scaler(1280.0, 800.0);
        let at = |y: f32| Vec2::new(300.0 * scale, y * scale);
        // A press that does not move gives its click back on release.
        app.save_panel_hold_press(at(300.0), SaveAction::Row(3));
        assert_eq!(app.save_panel_pointer(at(302.0), false, true, 0.016), None);
        assert_eq!(app.save_panel_pointer(at(302.0), false, false, 0.016), Some(SaveAction::Row(3)));
        // A press that moves drags the content with the pointer (up: the list goes further down), gives no click and keeps some speed.
        app.save_panel_hold_press(at(400.0), SaveAction::Row(3));
        for step in 1..=10 {
            assert_eq!(app.save_panel_pointer(at(400.0 - 20.0 * step as f32), false, true, 0.016), None);
        }
        let dragged = app.save_panel.as_ref().expect("panel").scroll;
        assert!((dragged - 700.0).abs() < 1e-3, "{dragged}");
        assert_eq!(app.save_panel_pointer(at(200.0), false, false, 0.016), None, "a drag is not a click");
        let speed = app.save_panel.as_ref().expect("panel").inertia;
        assert!(speed > 100.0, "{speed}");
        for _ in 0..10 {
            app.save_panel.as_mut().expect("panel").spring(0.016);
        }
        let later = app.save_panel.as_ref().expect("panel").scroll;
        assert!(later > dragged, "it keeps moving ({later})");
        for _ in 0..600 {
            app.save_panel.as_mut().expect("panel").spring(0.016);
        }
        assert!(app.save_panel.as_ref().expect("panel").inertia.abs() <= 1.0 || app.save_panel.as_ref().expect("panel").scroll >= 0.0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_scrollbar_drags_pages_and_follows_the_wheel() {
        let (mut app, dir) = app_in("bar");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        let panel = app.save_panel.as_mut().expect("panel");
        panel.rows = (0..40)
            .map(|index| SaveRow {
                path: PathBuf::from(format!("{index}.zip")),
                name: index.to_string(),
                date: String::new(),
            })
            .collect();
        // 40 rows: 15 + 2195 + 50 = 2260 in a view 930 high.
        assert_eq!(panel.max_scroll(), 1330.0);
        let scale = UiFrame::scaler(1280.0, 800.0);
        let ref_width = 1280.0 / scale;
        let layout = layout(ref_width);
        let at = |y: f32, x: f32| Vec2::new(x * scale, y * scale);
        let bar = app.save_panel.as_ref().expect("panel").bar(&layout);
        let handle = bar.handle();
        let x = handle[0] + 10.0;
        // A press under the handle moves a page (the view's height) down.
        app.save_panel_pointer(at(handle[1] + handle[3] + 40.0, x), true, true, 0.016);
        assert_eq!(app.save_panel.as_ref().expect("panel").scroll, 930.0);
        app.save_panel_pointer(at(handle[1] + handle[3] + 40.0, x), false, false, 0.016);
        // A press on the handle takes hold of it; moving the held pointer scrolls; the release lets go.
        let bar = app.save_panel.as_ref().expect("panel").bar(&layout);
        let handle = bar.handle();
        let start = handle[1] + 25.0;
        app.save_panel_pointer(at(start, x), true, true, 0.016);
        assert!(app.save_panel.as_ref().expect("panel").bar_drag.is_some());
        app.save_panel_pointer(at(start - 100.0, x), false, true, 0.016);
        let moved = app.save_panel.as_ref().expect("panel").scroll;
        assert!(moved < 930.0 && moved > 0.0, "{moved}");
        app.save_panel_pointer(at(-500.0, x), false, true, 0.016);
        assert_eq!(
            app.save_panel.as_ref().expect("panel").scroll,
            0.0,
            "stops at the top"
        );
        app.save_panel_pointer(at(5000.0, x), false, true, 0.016);
        assert_eq!(
            app.save_panel.as_ref().expect("panel").scroll,
            1330.0,
            "and at the end"
        );
        app.save_panel_pointer(at(100.0, x), false, false, 0.016);
        assert!(app.save_panel.as_ref().expect("panel").bar_drag.is_none());
        app.save_panel_pointer(at(5.0, x), false, true, 0.016);
        assert_eq!(
            app.save_panel.as_ref().expect("panel").scroll,
            1330.0,
            "a moved pointer without a grip drags nothing"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_list_scrolls_in_steps_of_fifty_between_its_ends() {
        let mut panel = panel();
        assert!(!panel.has_scrollbar(), "three rows fit");
        panel.wheel(-1.0);
        assert_eq!(panel.scroll, 0.0);
        // 20 rows: 15 + 20 * 50 + 19 * 5 + 50 = 1160 against 930.
        panel.rows = (0..20)
            .map(|index| SaveRow {
                path: PathBuf::from(format!("{index}.zip")),
                name: index.to_string(),
                date: String::new(),
            })
            .collect();
        assert_eq!(panel.content_height(), 1160.0);
        assert_eq!(panel.max_scroll(), 230.0);
        assert!(panel.has_scrollbar());
        panel.wheel(-1.0);
        assert_eq!(
            panel.scroll, 50.0,
            "a notch down moves the content up by 50"
        );
        panel.wheel(-10.0);
        assert!(panel.scroll > 230.0, "the elastic list goes past its end");
        for _ in 0..60 {
            panel.spring(0.016);
        }
        assert_eq!(panel.scroll, 230.0, "and springs back to it");
        panel.wheel(1.0);
        assert_eq!(panel.scroll, 180.0);
        // 16 rows are the first that need the bar (940 against 930); 15 do not.
        panel.rows.truncate(15);
        assert!(!panel.has_scrollbar());
        panel.rows = (0..16)
            .map(|index| SaveRow {
                path: PathBuf::from(format!("{index}.zip")),
                name: index.to_string(),
                date: String::new(),
            })
            .collect();
        assert!(panel.has_scrollbar());
    }

    use crate::app::LoadedWorld;
    use crate::ui::ControlCommand;
    use bibites_io::storage::StoragePaths;

    /// An app on a world of a dozen bibites whose storage is a folder of its own.
    fn app_in(name: &str) -> (App, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "rustibites-savepanel-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let world = LoadedWorld::load(None, 7, 12, 1).expect("starter world");
        let mut app = App::new(world, Vec2::new(1280.0, 800.0), None);
        app.storage = StoragePaths::new(dir.clone());
        app.user_settings_path = Some(dir.join("settings.json"));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::ResetWarnings)]);
        (app, dir)
    }

    /// A frame of nothing in particular (the characters travel through `App::typed`).
    fn idle() -> FrameInput {
        FrameInput {
            dt: 0.016,
            ..FrameInput::default()
        }
    }

    fn names(app: &App) -> Vec<String> {
        app.save_panel
            .as_ref()
            .expect("panel")
            .rows
            .iter()
            .map(|row| row.name.clone())
            .collect()
    }

    #[test]
    fn opening_the_panel_reads_the_world_and_starts_empty() {
        let (mut app, dir) = app_in("open");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        assert!(app.panels.is_open(PanelKind::SaveGame));
        let panel = app.save_panel.as_ref().expect("the panel");
        assert_eq!(panel.name, "");
        assert!(!panel.can_save(), "`saveButton.interactable = false`");
        assert!(panel.rows.is_empty());
        assert_eq!(panel.time, "00:00:00");
        assert_eq!(
            panel.bibites,
            app.world.bibites.len() + app.world.eggs.len()
        );
        let counts = app.world.pellets.counts();
        assert_eq!(
            panel.pellets,
            (counts.plant_count + counts.meat_count) as usize
        );
        assert!(panel.size.ends_with('u'), "{}", panel.size);
        // The picture of the world: 400 x 400.
        let picture = panel.preview.as_ref().expect("the world's picture");
        assert_eq!((picture.width, picture.height), (400, 400));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_picture_shows_the_whole_world_and_is_the_img_png_of_the_save() {
        let (mut app, dir) = app_in("picture");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        let picture = app
            .save_panel
            .as_ref()
            .expect("panel")
            .preview
            .clone()
            .expect("picture");
        let first = &picture.rgba[..4];
        assert!(
            picture.rgba.chunks_exact(4).any(|pixel| pixel != first),
            "the world is not one colour"
        );
        app.save_named("with-picture");
        // The file carries it as `img.png`: a 400 x 400 PNG, which the Load screen shows.
        let path = dir.join("Savefiles").join("with-picture.zip");
        assert!(path.is_file(), "{}", app.last_action);
        let card = bibites_io::storage::read_save_card(&path);
        let png = card.preview_png.expect("img.png");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        let side = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().expect("four bytes"));
        assert_eq!((side(16), side(20)), (400, 400));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_save_raises_the_corrupted_save_popup_when_the_panel_opens() {
        let (mut app, dir) = app_in("corrupt");
        std::fs::create_dir_all(dir.join("Savefiles")).expect("folder");
        std::fs::write(dir.join("Savefiles").join("broken.zip"), b"not a zip").expect("file");
        assert!(app.error_popups.is_empty());
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        assert_eq!(app.error_popups.len(), 1);
        assert_eq!(app.error_popups[0].0, "Corrupted Save");
        assert!(app.error_popups[0].1.contains("broken.zip"));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::DismissError)]);
        assert!(app.error_popups.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_save_carries_the_templates_its_spawn_rows_use_and_loading_restores_the_missing_ones() {
        let (mut app, dir) = app_in("templates");
        let templates = app.storage.templates_dir();
        std::fs::create_dir_all(&templates).expect("folder");
        std::fs::write(templates.join("mine.bb8template"), b"{}").expect("template");
        let mut row = bibites_core::settings::BibiteSpawn::game_defaults();
        row.path = "mine.bb8template".to_owned();
        app.world.bibite_spawns = vec![row];
        app.save_named("with-template");
        let save = dir.join("Savefiles").join("with-template.zip");
        // Another install without it gets it back.
        std::fs::remove_file(templates.join("mine.bb8template")).expect("remove");
        let browser = bibites_io::storage::SaveBrowser::new(app.storage.clone());
        assert_eq!(browser.restore_missing_templates(&save), vec!["mine.bb8template".to_owned()]);
        assert!(templates.join("mine.bb8template").is_file());
        assert!(browser.restore_missing_templates(&save).is_empty(), "nothing is overwritten");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_taken_name_shows_the_warning_and_saving_over_it_asks_first() {
        let (mut app, dir) = app_in("taken");
        app.save_named("taken");
        assert!(
            !app.panels.is_open(PanelKind::SaveGame),
            "a save closes the panel"
        );
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        // The list shows the file, a row puts its name in the field, and that name is taken.
        assert_eq!(names(&app), ["taken"]);
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Row(0))]);
        let panel = app.save_panel.as_ref().expect("panel");
        assert_eq!(panel.name, "taken");
        assert!(panel.replaces_a_save(), "`existIndicator`");
        // A name that is free has no warning.
        app.save_panel.as_mut().expect("panel").name = "free".to_owned();
        assert!(!app.save_panel.as_ref().expect("panel").replaces_a_save());
        // Save over the taken one: the question first, and cancelling leaves the file alone.
        app.save_panel.as_mut().expect("panel").name = "taken".to_owned();
        let file = dir.join("Savefiles").join("taken.zip");
        let before = std::fs::metadata(&file).expect("file").modified().ok();
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Save)]);
        assert!(
            matches!(app.confirm_dialog, Some(ConfirmKind::OverwriteSave(ref name)) if name == "taken")
        );
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Cancel)]);
        assert!(app.confirm_dialog.is_none() && app.panels.is_open(PanelKind::SaveGame));
        assert_eq!(
            std::fs::metadata(&file).expect("file").modified().ok(),
            before
        );
        // OK writes it and closes the panel.
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Save)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Confirm)]);
        assert!(!app.panels.is_open(PanelKind::SaveGame));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deleting_asks_then_removes_the_file_and_selects_the_next_row() {
        let (mut app, dir) = app_in("delete");
        for name in ["one", "two", "three"] {
            app.save_named(name);
            // Distinct modification times: the list is newest first.
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        assert_eq!(names(&app), ["three", "two", "one"], "newest first");
        // With no row selected Delete does nothing.
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Delete)]);
        assert!(app.confirm_dialog.is_none());
        // `two` selected: the warning, cancelled, then confirmed.
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Row(1))]);
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Delete)]);
        assert!(matches!(
            app.confirm_dialog,
            Some(ConfirmKind::DeleteSave(_))
        ));
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Cancel)]);
        assert_eq!(names(&app).len(), 3, "cancel keeps the file");
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Delete)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::Confirm)]);
        assert_eq!(names(&app), ["three", "one"]);
        // The row that took its place (`one`) is selected and named.
        let panel = app.save_panel.as_ref().expect("panel");
        assert_eq!((panel.selected, panel.name.as_str()), (Some(1), "one"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typing_needs_the_focus_and_the_game_keys_are_blocked_while_the_panel_is_open() {
        let (mut app, dir) = app_in("typing");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenEscapeMenu)]);
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        // Not focused: the characters are left alone and nothing is typed.
        app.typed = "ignored".to_owned();
        app.handle_input(&idle());
        assert_eq!(app.save_panel.as_ref().expect("panel").name, "");
        app.typed.clear();
        // A click on the field focuses it; the characters go in, Backspace takes one out, Enter ends the editing.
        app.apply_ui_actions(&[UiAction::SavePanel(SaveAction::Name)]);
        app.typed = "my save".to_owned();
        app.handle_input(&idle());
        assert_eq!(app.save_panel.as_ref().expect("panel").name, "my save");
        app.handle_input(&FrameInput {
            backspace: true,
            ..idle()
        });
        assert_eq!(app.save_panel.as_ref().expect("panel").name, "my sav");
        // Space does not pause or resume while the panel is open (`AllowControl = false`).
        let paused = app.paused;
        app.handle_input(&FrameInput {
            toggle_pause: true,
            ..idle()
        });
        assert_eq!(app.paused, paused);
        app.handle_input(&FrameInput {
            enter: true,
            ..idle()
        });
        assert!(!app.save_panel.as_ref().expect("panel").focused);
        // Escape closes the panel and the state goes with it; the pause menu shows again.
        app.handle_input(&FrameInput {
            quit: true,
            ..idle()
        });
        assert!(!app.panels.is_open(PanelKind::SaveGame));
        assert_eq!(app.panels.top(), Some(PanelKind::Escape));
        app.handle_input(&idle());
        assert!(app.save_panel.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_wheel_scrolls_the_list_only_over_it() {
        let (mut app, dir) = app_in("wheel");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        let panel = app.save_panel.as_mut().expect("panel");
        panel.rows = (0..30)
            .map(|index| SaveRow {
                path: PathBuf::from(format!("{index}.zip")),
                name: index.to_string(),
                date: String::new(),
            })
            .collect();
        let scale = UiFrame::scaler(1280.0, 800.0);
        // Over the selected panel nothing moves; over the list it scrolls by a notch.
        let over = |x: f32, y: f32| Vec2::new(x * scale, y * scale);
        app.save_panel_wheel(over(1200.0, 500.0), -1.0);
        assert_eq!(app.save_panel.as_ref().expect("panel").scroll, 0.0);
        app.save_panel_wheel(over(400.0, 500.0), -1.0);
        assert_eq!(app.save_panel.as_ref().expect("panel").scroll, 50.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_drawn_panel_has_the_buttons_the_prefab_has_and_a_disabled_save() {
        let (mut app, dir) = app_in("draw");
        app.apply_ui_actions(&[UiAction::Invoke(ControlCommand::OpenSavePanel)]);
        let ui = UiAssets::empty();
        let frame_of = |app: &App| {
            let mut frame = UiFrame::new(UiFrame::scaler(1920.0, 1080.0));
            draw(&mut frame, app, &ui, None, 1920.0, 1080.0);
            frame
        };
        let actions = |frame: &UiFrame| -> Vec<UiAction> {
            frame.actions.iter().map(|(_, action)| *action).collect()
        };
        let empty = actions(&frame_of(&app));
        for wanted in [SaveAction::Name, SaveAction::Delete, SaveAction::Close] {
            assert!(empty.contains(&UiAction::SavePanel(wanted)), "{wanted:?}");
        }
        assert!(
            !empty.contains(&UiAction::SavePanel(SaveAction::Save)),
            "with an empty name the button takes the click and does nothing"
        );
        // A valid name: Save is a control.
        app.save_panel.as_mut().expect("panel").name = "ok".to_owned();
        assert!(actions(&frame_of(&app)).contains(&UiAction::SavePanel(SaveAction::Save)));
        // A name with a bad character: not again.
        app.save_panel.as_mut().expect("panel").name = "a/b".to_owned();
        assert!(!actions(&frame_of(&app)).contains(&UiAction::SavePanel(SaveAction::Save)));
        // Nothing is drawn while another panel is the newest (the pause menu shows through when it is on top).
        app.panels.open(PanelKind::Escape);
        assert!(frame_of(&app).draws.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deleting_a_row_selects_the_one_that_takes_its_place() {
        let mut panel = panel();
        panel.select(1);
        // `beta` is gone: the row at the same place (now `gamma`) is selected and named.
        let rows: Vec<SaveRow> = panel
            .rows
            .iter()
            .filter(|row| row.name != "beta")
            .cloned()
            .collect();
        panel.after_delete(rows, 1);
        assert_eq!(panel.selected, Some(1));
        assert_eq!(panel.name, "gamma");
        // The last one goes: the one before it.
        let rows: Vec<SaveRow> = panel
            .rows
            .iter()
            .filter(|row| row.name != "gamma")
            .cloned()
            .collect();
        panel.after_delete(rows, 1);
        assert_eq!(panel.selected, Some(0));
        assert_eq!(panel.name, "alpha");
        // The only one goes: nothing is selected and the field keeps its text.
        panel.after_delete(Vec::new(), 0);
        assert_eq!(panel.selected, None);
        assert_eq!(panel.name, "alpha");
    }
}
