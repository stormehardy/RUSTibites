//! The Save Bibite dialog (`SaveBibiteDialogHandle`, opened by `SaveController.SaveBibiteOrEgg` from
//! the Stats panel's Save button): a name, a description, "Save as template" and "Format JSON",
//! warnings for a name with invalid characters and for a file that already exists, and Cancel /
//! Save. While it is open the game is paused and every control is blocked (`LockControls`,
//! `AllowControl = false`, the screen blocker); Escape closes it.
//!
//! Geometry and texts come from the prefab (`tools/unity/dump_ui_tree.py`, scene `level2`, path id
//! 692): a 320-wide `Button_UI` frame (multiplier 8) in the middle of the screen, 10 units of padding
//! and 10 between its rows (a hidden warning takes no room): "Save Bibite", "Bibite Name:" and a
//! 300 x 28.65 field, the "already exists" warning, "Description:" and a 300 x 190 field, the
//! "invalid characters" warning, the two checkboxes and the 60 x 30 buttons.
//!
//! Not matched: the text fields are drawn in the software text style (no caret blinking, no
//! selection), and the screen blocker is a plain dimming.

use std::path::{Path, PathBuf};

use bibites_core::ids::{BibiteId, EggId, SpeciesId};
use bibites_core::math::Vec2;

use crate::app::App;
use crate::ui::{ControlCommand, Rgba, UiAction, UiAssets, UiDraw, UiFrame};

/// The default name the game's dialog shows when it has none (`OpenDialog(defaultText)`).
pub const DEFAULT_NAME: &str = "Bibite personalis";

/// What the dialog saves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The selected bibite.
    Bibite(BibiteId),
    /// The selected egg.
    Egg(EggId),
    /// The template of a species (`SpeciesInfoPanel.SaveSpecies` -> `SaveController.SaveTemplateOfBibite`): the dialog opens with "Save as template" on and
    /// not interactable (`OpenDialog(allowSaved: false)`).
    Species(SpeciesId),
}

/// The text field with the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// "Bibite Name:".
    Name,
    /// "Description:".
    Description,
}

/// The state of an open dialog.
#[derive(Clone, Debug)]
pub struct SaveDialog {
    /// What is saved.
    pub target: Target,
    /// The file name (`bibiteName`).
    pub name: String,
    /// The player's description (`description`).
    pub description: String,
    /// `saveAsTemplate`: a `.bb8template` in the templates folder instead of a `.bb8`.
    pub as_template: bool,
    /// `OpenDialog(allowSaved)`: false for a species' template, whose "Save as template" box is on and cannot be changed (`saveAsTemplate.interactable`).
    pub allow_saved: bool,
    /// `formatJSON`: indented JSON.
    pub format_json: bool,
    /// The field typing goes to.
    pub focus: Field,
    /// The game's pause before the dialog locked it (`TogglePauseGame("SaveSystem")`).
    pub previous_pause: bool,
    /// The caret of the focused field.
    pub caret: crate::text_field::Caret,
}

/// `Path.GetInvalidFileNameChars()` on Windows.
#[must_use]
pub fn is_invalid_file_char(character: char) -> bool {
    matches!(
        character,
        '"' | '<' | '>' | '|' | ':' | '*' | '?' | '\\' | '/'
    ) || character < ' '
}

impl SaveDialog {
    /// `containsABadCharacter.IsMatch(bibiteName.text)`.
    #[must_use]
    pub fn name_is_invalid(&self) -> bool {
        self.name.chars().any(is_invalid_file_char)
    }

    /// `submitButton.interactable`: a valid, non-empty name.
    #[must_use]
    pub fn can_save(&self) -> bool {
        !self.name_is_invalid() && !self.name.is_empty()
    }

    /// `GetSavePath`: the templates folder for a template, the saved bibites folder otherwise.
    #[must_use]
    pub fn path(&self, bibites_dir: &Path, templates_dir: &Path) -> PathBuf {
        if self.as_template {
            templates_dir.join(format!("{}.bb8template", self.name))
        } else {
            bibites_dir.join(format!("{}.bb8", self.name))
        }
    }

    /// `warningExistingText.SetActive(File.Exists(GetSavePath()))`.
    #[must_use]
    pub fn would_overwrite(&self, bibites_dir: &Path, templates_dir: &Path) -> bool {
        self.can_save() && self.path(bibites_dir, templates_dir).is_file()
    }

    fn focused_text(&mut self) -> &mut String {
        match self.focus {
            Field::Name => &mut self.name,
            Field::Description => &mut self.description,
        }
    }

    /// Typed characters go to the focused field (`TMP_InputField`; control characters are dropped).
    pub fn type_text(&mut self, text: &str) {
        let mut caret = self.caret;
        caret.clamp(self.focused_text());
        caret.insert(self.focused_text(), text);
        self.caret = caret;
    }

    /// Backspace in the focused field.
    pub fn backspace(&mut self) {
        let mut caret = self.caret;
        caret.clamp(self.focused_text());
        caret.backspace(self.focused_text());
        self.caret = caret;
    }

    /// The caret keys (and Delete / Ctrl + A) in the focused field.
    pub fn keys(&mut self, keys: crate::text_field::EditKeys) {
        let mut caret = self.caret;
        crate::text_field::edit(self.focused_text(), &mut caret, "", usize::MAX, keys);
        self.caret = caret;
    }

    /// Puts the caret at the end of the field that has the focus (the focus moved).
    pub fn focus_field(&mut self, field: Field) {
        self.focus = field;
        let end = crate::text_field::Caret::at_end(self.focused_text());
        self.caret = end;
    }

    /// Enter: a new line in the multi-line description; the name field has none.
    pub fn enter(&mut self) {
        if self.focus == Field::Description {
            let mut caret = self.caret;
            caret.clamp(&self.description);
            caret.insert_newline(&mut self.description);
            self.caret = caret;
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

const WIDTH: f32 = 320.0;
const PAD: f32 = 10.0;
const GAP: f32 = 10.0;
const DARK: Rgba = [28, 28, 28, 255];
const WARNING: Rgba = [170, 40, 30, 255];
/// A disabled `Selectable`: `disabledColor` (0.784, 0.784, 0.784, 0.502).
const DISABLED: Rgba = [200, 200, 200, 128];

// Click-area ids of the dialog: the block 9010..=9017 is this dialog's. It sits over the Information panel (8800..) and the Biology popups (9000..) and
// `UiFrame::interact` answers a click with every action registered under the id it hit, so it may share none of theirs (it was on 8800..8807 and a click
// on its panel or its Name field also closed the Information panel or swapped its count / energy section).
const HIT_BLOCKER: u32 = 9010;
const HIT_PANEL: u32 = 9011;
const HIT_NAME: u32 = 9012;
const HIT_DESCRIPTION: u32 = 9013;
const HIT_TEMPLATE: u32 = 9014;
const HIT_JSON: u32 = 9015;
const HIT_CANCEL: u32 = 9016;
const HIT_SAVE: u32 = 9017;

/// Rough advance of a run of text at `px` (the build step has no font metrics).
fn estimate_width(text: &str, px: f32) -> f32 {
    text.chars().count() as f32 * px * 0.56
}

/// The dialog's rows from the top, with a hidden warning taking no room.
struct Layout {
    top: f32,
    height: f32,
    name_label: f32,
    name_field: f32,
    existing: Option<f32>,
    description_label: f32,
    description_field: f32,
    invalid: Option<f32>,
    options: f32,
    buttons: f32,
}

fn layout(overwrites: bool, invalid: bool) -> Layout {
    const TITLE_H: f32 = 28.13;
    const NAME_H: f32 = 47.41;
    const EXISTING_H: f32 = 28.13;
    const DESCRIPTION_H: f32 = 208.76;
    const INVALID_H: f32 = 15.83;
    const OPTIONS_H: f32 = 15.0;
    const BUTTONS_H: f32 = 30.0;
    let mut y = PAD + TITLE_H + GAP;
    let name_top = y;
    y += NAME_H + GAP;
    let existing = overwrites.then(|| {
        let at = y;
        y += EXISTING_H + GAP;
        at
    });
    let description_top = y;
    y += DESCRIPTION_H + GAP;
    let invalid_at = invalid.then(|| {
        let at = y;
        y += INVALID_H + GAP;
        at
    });
    let options = y;
    y += OPTIONS_H + GAP;
    let buttons = y;
    y += BUTTONS_H + PAD;
    Layout {
        top: 0.0,
        height: y,
        name_label: name_top,
        name_field: name_top + 18.76 + 4.0,
        existing,
        description_label: description_top,
        description_field: description_top + 18.76 + 0.0,
        invalid: invalid_at,
        options,
        buttons,
    }
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

/// Draws the dialog while it is open.
pub fn draw(frame: &mut UiFrame, app: &App, ui: &UiAssets, width: f32, height: f32) {
    let Some(dialog) = app.save_dialog.as_ref() else {
        return;
    };
    let overwrites =
        dialog.would_overwrite(&app.storage.bibites_dir(), &app.storage.templates_dir());
    draw_dialog(frame, dialog, overwrites, ui, width, height);
}

/// Draws `dialog`; `overwrites` is whether its file exists already.
pub fn draw_dialog(
    frame: &mut UiFrame,
    dialog: &SaveDialog,
    overwrites: bool,
    ui: &UiAssets,
    width: f32,
    height: f32,
) {
    let s = UiFrame::scaler(width, height);
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let dim = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let invalid = dialog.name_is_invalid();
    let rows = layout(overwrites, invalid);
    let left = (1920.0 - WIDTH) * 0.5;
    let top = (1080.0 - rows.height) * 0.5 + rows.top;
    let inner = left + PAD;
    let inner_w = WIDTH - 2.0 * PAD;

    // The screen blocker: dims the world and takes every click.
    frame.rect(Vec2::ZERO, Vec2::new(width, height), [0, 0, 0, 110]);
    frame.hit(
        HIT_BLOCKER,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::Consume,
    );
    slice(
        frame,
        ui,
        "Button_UI",
        30.0,
        0.125,
        at(left, top),
        dim(WIDTH, rows.height),
        [255, 255, 255, 255],
    );
    frame.hit(
        HIT_PANEL,
        at(left, top),
        dim(WIDTH, rows.height),
        UiAction::Consume,
    );

    let title = "Save Bibite";
    frame.bold(
        title,
        at(
            left + (WIDTH - estimate_width(title, 24.0)) * 0.5,
            top + PAD,
        ),
        24.0 * s,
        DARK,
    );

    let field = |frame: &mut UiFrame, x: f32, y: f32, w: f32, h: f32, focused: bool| {
        frame.rect(
            at(x, y),
            dim(w, h),
            if focused {
                [90, 140, 220, 255]
            } else {
                [90, 90, 90, 255]
            },
        );
        frame.rect(
            at(x + 1.0, y + 1.0),
            dim(w - 2.0, h - 2.0),
            [255, 255, 255, 255],
        );
    };

    // "Bibite Name:" and its single-line field.
    frame.text(
        "Bibite Name:",
        at(inner, top + rows.name_label),
        18.0 * s,
        DARK,
    );
    let name_y = top + rows.name_field;
    field(
        frame,
        inner,
        name_y,
        inner_w,
        28.65,
        dialog.focus == Field::Name,
    );
    let name_text = if dialog.focus == Field::Name {
        crate::text_field::with_caret_at(&dialog.name, dialog.caret.pos)
    } else {
        dialog.name.clone()
    };
    frame.text(name_text, at(inner + 6.0, name_y + 5.0), 18.0 * s, DARK);
    frame.hit(
        HIT_NAME,
        at(inner, name_y),
        dim(inner_w, 28.65),
        UiAction::SaveDialogFocus(0),
    );

    if let Some(y) = rows.existing {
        for (index, line) in [
            "*A Bibite file with this name already exists*",
            "*The existing file will be overwritten*",
        ]
        .into_iter()
        .enumerate()
        {
            let px = 13.0;
            frame.text(
                line,
                at(
                    left + (WIDTH - estimate_width(line, px)) * 0.5,
                    top + y + index as f32 * 14.0,
                ),
                px * s,
                WARNING,
            );
        }
    }

    // "Description:" and its multi-line field.
    frame.text(
        "Description:",
        at(inner, top + rows.description_label),
        18.0 * s,
        DARK,
    );
    let description_y = top + rows.description_field;
    field(
        frame,
        inner,
        description_y,
        inner_w,
        190.0,
        dialog.focus == Field::Description,
    );
    let mut text = dialog.description.clone();
    if dialog.focus == Field::Description {
        text = crate::text_field::with_caret_at(&dialog.description, dialog.caret.pos);
    }
    // A simple wrap at the field's width, line by line.
    let line_chars = ((inner_w - 12.0) / (16.0 * 0.56)).floor().max(8.0) as usize;
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let characters: Vec<char> = paragraph.chars().collect();
        if characters.is_empty() {
            lines.push(String::new());
        }
        for chunk in characters.chunks(line_chars) {
            lines.push(chunk.iter().collect());
        }
    }
    for (index, line) in lines.iter().take(9).enumerate() {
        frame.text(
            line,
            at(inner + 6.0, description_y + 5.0 + index as f32 * 19.0),
            16.0 * s,
            DARK,
        );
    }
    frame.hit(
        HIT_DESCRIPTION,
        at(inner, description_y),
        dim(inner_w, 190.0),
        UiAction::SaveDialogFocus(1),
    );

    if let Some(y) = rows.invalid {
        let line = "*The name contains invalid characters*";
        frame.text(
            line,
            at(left + (WIDTH - estimate_width(line, 13.0)) * 0.5, top + y),
            13.0 * s,
            WARNING,
        );
    }

    // The two checkboxes.
    let options_y = top + rows.options;
    for (index, (label, on, x, id)) in [
        ("Save as template", dialog.as_template, inner, HIT_TEMPLATE),
        (
            "Format JSON",
            dialog.format_json,
            left + WIDTH - PAD - 90.35,
            HIT_JSON,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        // A species' template is always saved as one: its box is on and not interactable (`OpenDialog(allowSaved: false)`), drawn with the disabled tint.
        let locked = index == 0 && !dialog.allow_saved;
        slice(
            frame,
            ui,
            "base_icon",
            5.0,
            0.5,
            at(x, options_y),
            dim(15.0, 15.0),
            if locked { DISABLED } else { [255, 255, 255, 255] },
        );
        if on {
            if let Some(texture) = ui.texture_of_alias("checkmark_icon") {
                frame.draws.push(UiDraw::Image {
                    texture,
                    pos: at(x, options_y),
                    size: dim(15.0, 15.0),
                    tint: [255, 255, 255, 255],
                });
            } else {
                frame.rect(at(x + 3.0, options_y + 3.0), dim(9.0, 9.0), DARK);
            }
        }
        frame.text(label, at(x + 19.0, options_y - 1.0), 13.0 * s, DARK);
        if !locked {
            frame.hit(
                id,
                at(x, options_y),
                dim(if index == 0 { 113.42 } else { 90.35 }, 15.0),
                UiAction::SaveDialogToggle(index as u8),
            );
        }
    }

    // Cancel (left) and Save (right), 60 x 30.
    let buttons_y = top + rows.buttons;
    for (label, x, enabled, id, command) in [
        (
            "Cancel",
            inner,
            true,
            HIT_CANCEL,
            ControlCommand::SaveDialogCancel,
        ),
        (
            "Save",
            left + WIDTH - PAD - 60.0,
            dialog.can_save(),
            HIT_SAVE,
            ControlCommand::SaveDialogConfirm,
        ),
    ] {
        slice(
            frame,
            ui,
            "Button_UI",
            30.0,
            0.125,
            at(x, buttons_y),
            dim(60.0, 30.0),
            if enabled {
                [255, 255, 255, 255]
            } else {
                [255, 255, 255, 110]
            },
        );
        frame.text(
            label,
            at(
                x + (60.0 - estimate_width(label, 18.0)) * 0.5,
                buttons_y + 5.0,
            ),
            18.0 * s,
            if enabled { DARK } else { [120, 120, 120, 255] },
        );
        if enabled {
            frame.hit(
                id,
                at(x, buttons_y),
                dim(60.0, 30.0),
                UiAction::Invoke(command),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> SaveDialog {
        SaveDialog {
            target: Target::Bibite(BibiteId::from_handle(bibites_core::arena::Handle::new(
                1, 0,
            ))),
            name: "Wolf".to_owned(),
            description: "a hunter".to_owned(),
            as_template: false,
            allow_saved: true,
            format_json: false,
            focus: Field::Name,
            previous_pause: false,
            caret: crate::text_field::Caret { pos: usize::MAX, anchor: None },
        }
    }

    #[test]
    fn invalid_names_are_the_windows_file_name_characters() {
        for bad in [
            "a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", "a\u{1}b",
        ] {
            let mut dialog = dialog();
            dialog.name = bad.to_owned();
            assert!(dialog.name_is_invalid(), "{bad:?}");
            assert!(!dialog.can_save());
        }
        let mut dialog = dialog();
        assert!(!dialog.name_is_invalid() && dialog.can_save());
        // An empty name cannot be saved either (`!string.IsNullOrEmpty`).
        dialog.name.clear();
        assert!(!dialog.can_save());
        // Spaces, dots and accents are fine.
        dialog.name = "Bibite personalis v1.2 \u{e9}".to_owned();
        assert!(dialog.can_save());
    }

    #[test]
    fn the_path_follows_the_template_checkbox() {
        let mut dialog = dialog();
        let (bibites, templates) = (Path::new("B"), Path::new("T"));
        assert_eq!(
            dialog.path(bibites, templates),
            Path::new("B").join("Wolf.bb8")
        );
        dialog.as_template = true;
        assert_eq!(
            dialog.path(bibites, templates),
            Path::new("T").join("Wolf.bb8template")
        );
    }

    #[test]
    fn an_existing_file_is_a_warning_not_a_block() {
        let dir = std::env::temp_dir().join(format!("rustibites-savedlg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let mut dialog = dialog();
        assert!(!dialog.would_overwrite(&dir, &dir));
        std::fs::write(dir.join("Wolf.bb8"), b"{}").expect("file");
        assert!(dialog.would_overwrite(&dir, &dir));
        assert!(dialog.can_save(), "saving over it is allowed");
        // An invalid name never reports an overwrite.
        dialog.name = "a/b".to_owned();
        assert!(!dialog.would_overwrite(&dir, &dir));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typing_goes_to_the_focused_field_and_enter_only_breaks_the_description() {
        let mut dialog = dialog();
        dialog.type_text("x\ty");
        assert_eq!(dialog.name, "Wolfxy", "control characters are dropped");
        dialog.backspace();
        assert_eq!(dialog.name, "Wolfx");
        dialog.enter();
        assert_eq!(dialog.name, "Wolfx", "the name field is single-line");
        dialog.focus_field(Field::Description);
        dialog.type_text("!");
        dialog.enter();
        dialog.type_text("two");
        assert_eq!(dialog.description, "a hunter!\ntwo");
    }

    #[test]
    fn hidden_warnings_take_no_room_and_the_controls_are_hittable() {
        let calm = layout(false, false);
        let with_both = layout(true, true);
        assert!((with_both.height - calm.height - (28.13 + 15.83 + 2.0 * GAP)).abs() < 1e-3);
        // The measured prefab height with both warnings: 453.26.
        assert!(
            (with_both.height - 453.26).abs() < 0.01,
            "{}",
            with_both.height
        );

        let dialog = dialog();
        let mut frame = UiFrame::default();
        draw_dialog(
            &mut frame,
            &dialog,
            false,
            &UiAssets::empty(),
            1920.0,
            1080.0,
        );
        let rows = layout(false, false);
        let left = (1920.0 - WIDTH) * 0.5;
        let top = (1080.0 - rows.height) * 0.5;
        // Everything outside the dialog is blocked, inside it the fields and buttons answer.
        assert_eq!(
            frame.action_at(Vec2::new(5.0, 5.0)),
            Some(UiAction::Consume)
        );
        assert_eq!(
            frame.action_at(Vec2::new(left + 100.0, top + rows.name_field + 10.0)),
            Some(UiAction::SaveDialogFocus(0))
        );
        assert_eq!(
            frame.action_at(Vec2::new(left + 100.0, top + rows.description_field + 50.0)),
            Some(UiAction::SaveDialogFocus(1))
        );
        assert_eq!(
            frame.action_at(Vec2::new(left + 15.0, top + rows.options + 7.0)),
            Some(UiAction::SaveDialogToggle(0))
        );
        assert_eq!(
            frame.action_at(Vec2::new(left + WIDTH - 20.0, top + rows.options + 7.0)),
            Some(UiAction::SaveDialogToggle(1))
        );
        assert_eq!(
            frame.action_at(Vec2::new(left + 20.0, top + rows.buttons + 15.0)),
            Some(UiAction::Invoke(ControlCommand::SaveDialogCancel))
        );
        assert_eq!(
            frame.action_at(Vec2::new(left + WIDTH - 20.0, top + rows.buttons + 15.0)),
            Some(UiAction::Invoke(ControlCommand::SaveDialogConfirm))
        );
        // An invalid name disables Save: its area only blocks the click.
        let mut bad = dialog.clone();
        bad.name = "a/b".to_owned();
        let mut frame = UiFrame::default();
        draw_dialog(&mut frame, &bad, false, &UiAssets::empty(), 1920.0, 1080.0);
        let rows = layout(false, true);
        let top = (1080.0 - rows.height) * 0.5;
        assert_eq!(
            frame.action_at(Vec2::new(left + WIDTH - 20.0, top + rows.buttons + 15.0)),
            Some(UiAction::Consume)
        );
    }

    #[test]
    fn a_species_template_is_saved_as_a_template_and_its_box_is_locked() {
        // `SaveController.SaveTemplateOfBibite` opens the dialog with `allowSaved: false`: "Save as template" is on and cannot be clicked, "Format JSON" can.
        let mut species = dialog();
        species.target = Target::Species(SpeciesId::new(3));
        species.as_template = true;
        species.allow_saved = false;
        let mut frame = UiFrame::default();
        draw_dialog(&mut frame, &species, false, &UiAssets::empty(), 1920.0, 1080.0);
        let rows = layout(false, false);
        let left = (1920.0 - WIDTH) * 0.5;
        let top = (1080.0 - rows.height) * 0.5;
        assert_eq!(
            frame.action_at(Vec2::new(left + 15.0, top + rows.options + 7.0)),
            Some(UiAction::Consume),
            "the locked box only blocks the click"
        );
        assert_eq!(
            frame.action_at(Vec2::new(left + WIDTH - 20.0, top + rows.options + 7.0)),
            Some(UiAction::SaveDialogToggle(1))
        );
        // The file goes to the templates folder.
        assert_eq!(species.path(Path::new("B"), Path::new("T")), Path::new("T").join("Wolf.bb8template"));
    }
}
