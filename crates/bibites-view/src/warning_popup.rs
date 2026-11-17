//! The game's warning popup (`PopupManager.DisplayActionWarning` -> the `WarningPopup` prefab):
//! a title, a wrapped text, a "do not show again" checkbox and the Cancel / proceed buttons,
//! centred on the screen over a dimming screen blocker.
//!
//! Prefab values (`sharedassets0`, `WarningPopup`): a 400-wide `Button_UI` slice (border 30,
//! `pixelsPerUnitMultiplier` 8) with a vertical layout (padding 10 / 10 / 5 / 10, spacing 10, the
//! height fits the content): the title (Roboto-Bold 24, 28 high), the text (Roboto-Regular 18,
//! wrapped at 380), the "Additionnal Line" (a horizontal layout, right-aligned, spacing 5: the label
//! "do not show again " in 16 pt Liberation Sans and the 20 x 20 `Toggle`, a `base_icon` slice with
//! a `checkmark_icon`), and the button holder (30 high: Cancel at the left, the proceed button at
//! the right, 80 x 30 each, Roboto-Bold 20). The scene's `Popuholder` scales the popup: 1.35 in the
//! world, 1.0 in the title menu. The `screenBlocker` behind it is a black 0.4 alpha quad.
//!
//! This prefab was never captured in a running game (the other popups were), so the geometry is
//! derived from those values and not measured on a screenshot.

use bibites_core::math::Vec2;

use crate::ui::{FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiId};

/// The popup's width in canvas units (the root's `size_delta.x`).
pub const WIDTH: f32 = 400.0;
const PADDING_X: f32 = 10.0;
const PADDING_TOP: f32 = 5.0;
const PADDING_BOTTOM: f32 = 10.0;
const SPACING: f32 = 10.0;
const TITLE_HEIGHT: f32 = 28.0;
const TITLE_PX: f32 = 24.0;
const TEXT_PX: f32 = 18.0;
const LABEL_PX: f32 = 16.0;
const BUTTON_PX: f32 = 20.0;
/// The checkbox row (the label's preferred height and the toggle's size).
const ROW_HEIGHT: f32 = 20.0;
const ROW_SPACING: f32 = 5.0;
const BUTTON_SIZE: [f32; 2] = [80.0, 30.0];
/// Roboto's line height over its size (`face.lineHeight / pointSize`).
const ROBOTO_LINE: f32 = crate::tooltip::LINE_HEIGHT;
/// The label of the checkbox (with the prefab's trailing space).
pub const LABEL: &str = "do not show again ";
/// `Button_UI`: slice border, and the `Image.pixelsPerUnitMultiplier` of the popup's images.
const BUTTON_BORDER: f32 = 30.0;
const BUTTON_MULTIPLIER: f32 = 8.0;
/// `base_icon`: slice border of the checkbox.
const TOGGLE_BORDER: f32 = 5.0;
/// The text colour of the prefab's TMP objects (`(30, 31, 30)`).
const INK: Rgba = [30, 31, 30, 255];
/// The Cancel button's text colour (`(35, 36, 35, 212)`).
const CANCEL_INK: Rgba = [35, 36, 35, 212];
/// `LiberationSans SDF` white labels draw dark and heavy in the game (the outline material).
const LABEL_INK: Rgba = [33, 34, 33, 255];
/// The `screenBlocker`: black at 0.4 alpha.
pub const BLOCKER: Rgba = [0, 0, 0, 102];

/// What a warning popup shows.
#[derive(Clone, Copy, Debug)]
pub struct Spec<'a> {
    /// The title.
    pub title: &'a str,
    /// The text (newlines kept).
    pub text: &'a str,
    /// The proceed button's label.
    pub proceed: &'a str,
    /// The "do not show again" checkbox is ticked.
    pub checked: bool,
    /// The scene's `Popuholder` scale: 1.35 in the world, 1.0 in the title menu.
    pub scale: f32,
}

/// What the three controls do.
#[derive(Clone, Copy, Debug)]
pub struct Actions {
    /// The Cancel button.
    pub cancel: UiAction,
    /// The proceed button.
    pub proceed: UiAction,
    /// The checkbox.
    pub toggle: UiAction,
}

/// Which face a string is measured in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// Roboto-Regular (the text).
    Regular,
    /// Roboto-Bold (the title, the button labels).
    Bold,
    /// Liberation Sans (the checkbox label).
    Liberation,
}

/// The popup's geometry in screen units of a popup whose top-left is the origin (canvas units
/// times the popup scale; the UI scaler is applied when drawing).
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    /// The popup's size.
    pub size: Vec2,
    /// The wrapped text lines.
    pub lines: Vec<String>,
    /// Top of the title's line.
    pub title_top: f32,
    /// Top of the first text line.
    pub text_top: f32,
    /// Height of one text line.
    pub line_height: f32,
    /// Top of the checkbox row.
    pub row_top: f32,
    /// Left edge of the checkbox label (it ends one spacing, 5 units, left of the checkbox).
    pub label_left: f32,
    /// The checkbox: `x, y, w, h`.
    pub toggle: [f32; 4],
    /// The Cancel button: `x, y, w, h`.
    pub cancel: [f32; 4],
    /// The proceed button: `x, y, w, h`.
    pub proceed: [f32; 4],
}

/// Lays the popup out for `spec`; `measure(text, px, face)` gives the width of a string.
#[must_use]
pub fn lay_out(spec: &Spec<'_>, measure: &dyn Fn(&str, f32, Face) -> f32) -> Laid {
    let k = spec.scale;
    let lines = crate::tooltip::wrap(spec.text, (WIDTH - 2.0 * PADDING_X) * k, &|line| {
        measure(line, TEXT_PX * k, Face::Regular)
    });
    let line_height = TEXT_PX * k * ROBOTO_LINE;
    let text_height = line_height * lines.len().max(1) as f32;
    let title_top = PADDING_TOP * k;
    let text_top = title_top + (TITLE_HEIGHT + SPACING) * k;
    let row_top = text_top + text_height + SPACING * k;
    let holder_top = row_top + (ROW_HEIGHT + SPACING) * k;
    let height = holder_top + (BUTTON_SIZE[1] + PADDING_BOTTOM) * k;
    let inner_right = (WIDTH - PADDING_X) * k;
    // The row is right-aligned: the toggle first from the right, then the spacing, then the label.
    let toggle_size = ROW_HEIGHT * k;
    let toggle_left = inner_right - toggle_size;
    let label_width = measure(LABEL, LABEL_PX * k, Face::Liberation);
    let label_left = toggle_left - ROW_SPACING * k - label_width;
    let button = [BUTTON_SIZE[0] * k, BUTTON_SIZE[1] * k];
    Laid {
        size: Vec2::new(WIDTH * k, height),
        lines,
        title_top,
        text_top,
        line_height,
        row_top,
        label_left,
        toggle: [toggle_left, row_top, toggle_size, toggle_size],
        cancel: [PADDING_X * k, holder_top, button[0], button[1]],
        proceed: [inner_right - button[0], holder_top, button[0], button[1]],
    }
}

/// The text measure of the popup: the loaded SDF fonts, or half an em per character.
fn measurer(metrics: Option<&crate::text_sdf::SdfFonts>) -> impl Fn(&str, f32, Face) -> f32 + '_ {
    move |text: &str, px: f32, face: Face| -> f32 {
        let (name, role) = match face {
            Face::Regular => (None, FontRole::Body),
            Face::Bold => (None, FontRole::Bold),
            Face::Liberation => (Some("LiberationSans SDF"), FontRole::Body),
        };
        metrics.map_or(text.chars().count() as f32 * px * 0.5, |fonts| {
            fonts.measure_named(name, role, text, px)
        })
    }
}

/// Draws the popup over a full-screen blocker and registers its hits (`first_id` .. `first_id + 4`):
/// the blocker and the panel swallow the clicks, then the checkbox and the two buttons.
#[allow(clippy::too_many_arguments)]
pub fn emit(
    frame: &mut UiFrame,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    width: f32,
    height: f32,
    spec: &Spec<'_>,
    actions: Actions,
    first_id: UiId,
) {
    let s = UiFrame::scaler(width, height);
    let k = spec.scale;
    let measure = measurer(metrics);
    let laid = lay_out(spec, &measure);
    // The popup is centred on the screen (the holder fills it; the popup's anchor is its middle).
    let origin = Vec2::new(
        width * 0.5 - laid.size.x * s * 0.5,
        height * 0.5 - laid.size.y * s * 0.5,
    );
    let at = |x: f32, y: f32| Vec2::new(origin.x + x * s, origin.y + y * s);
    let extent = |w: f32, h: f32| Vec2::new(w * s, h * s);
    // The screen blocker dims the world and takes every click.
    frame.rect(Vec2::ZERO, Vec2::new(width, height), BLOCKER);
    frame.hit(
        first_id,
        Vec2::ZERO,
        Vec2::new(width, height),
        UiAction::Consume,
    );
    // The popup itself.
    let slice = |frame: &mut UiFrame, rect: [f32; 4]| match ui.texture_of_alias("Button_UI") {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [BUTTON_BORDER; 4],
            dest_scale: k / BUTTON_MULTIPLIER,
            pos: at(rect[0], rect[1]),
            size: extent(rect[2], rect[3]),
            tint: [255, 255, 255, 255],
        }),
        None => {
            frame.rect(
                at(rect[0], rect[1]),
                extent(rect[2], rect[3]),
                [30, 31, 30, 255],
            );
            let edge = 2.0 * k;
            frame.rect(
                at(rect[0] + edge, rect[1] + edge),
                extent(rect[2] - 2.0 * edge, rect[3] - 2.0 * edge),
                [224, 224, 224, 255],
            );
        }
    };
    slice(frame, [0.0, 0.0, laid.size.x, laid.size.y]);
    frame.hit(
        first_id + 1,
        at(0.0, 0.0),
        extent(laid.size.x, laid.size.y),
        UiAction::Consume,
    );
    frame.bold(
        spec.title,
        at(PADDING_X * k, laid.title_top),
        TITLE_PX * k * s,
        INK,
    );
    for (index, line) in laid.lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        frame.text(
            line.clone(),
            at(
                PADDING_X * k,
                laid.text_top + laid.line_height * index as f32,
            ),
            TEXT_PX * k * s,
            INK,
        );
    }
    // The checkbox row: the label (a 16 pt Liberation Sans line centred in the 20 high row), then
    // the toggle.
    let label_line = LABEL_PX * k * 1.149;
    frame.bold(
        LABEL,
        at(
            laid.label_left,
            laid.row_top + (ROW_HEIGHT * k - label_line) * 0.5,
        ),
        LABEL_PX * k * s,
        LABEL_INK,
    );
    let [tx, ty, tw, th] = laid.toggle;
    match ui.texture_of_alias("base_icon") {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [TOGGLE_BORDER; 4],
            dest_scale: k,
            pos: at(tx, ty),
            size: extent(tw, th),
            tint: [255, 255, 255, 255],
        }),
        None => {
            frame.rect(at(tx, ty), extent(tw, th), [30, 31, 30, 255]);
            frame.rect(
                at(tx + 2.0 * k, ty + 2.0 * k),
                extent(tw - 4.0 * k, th - 4.0 * k),
                [255, 255, 255, 255],
            );
        }
    }
    if spec.checked {
        match ui.texture_of_alias("checkmark_icon") {
            Some(texture) => frame.draws.push(UiDraw::Image {
                texture,
                pos: at(tx, ty),
                size: extent(tw, th),
                tint: [255, 255, 255, 255],
            }),
            None => frame.bold("x", at(tx + 5.0 * k, ty), LABEL_PX * k * s, INK),
        }
    }
    frame.hit(first_id + 4, at(tx, ty), extent(tw, th), actions.toggle);
    // The buttons: Cancel on the left, proceed on the right; Roboto-Bold 20 centred in each.
    let buttons = [
        (
            laid.cancel,
            "Cancel",
            CANCEL_INK,
            actions.cancel,
            first_id + 2,
        ),
        (
            laid.proceed,
            spec.proceed,
            INK,
            actions.proceed,
            first_id + 3,
        ),
    ];
    let label_height = BUTTON_PX * k * ROBOTO_LINE;
    for (rect, label, ink, action, id) in buttons {
        slice(frame, rect);
        let label_width = measure(label, BUTTON_PX * k, Face::Bold);
        frame.bold(
            label,
            at(
                rect[0] + (rect[2] - label_width) * 0.5,
                rect[1] + (rect[3] - label_height) * 0.5,
            ),
            BUTTON_PX * k * s,
            ink,
        );
        frame.hit(id, at(rect[0], rect[1]), extent(rect[2], rect[3]), action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph is half an em wide.
    fn mono(text: &str, px: f32, _face: Face) -> f32 {
        text.chars().count() as f32 * px * 0.5
    }

    fn spec(text: &str, scale: f32) -> Spec<'_> {
        Spec {
            title: "Deleting Save File",
            text,
            proceed: "Yes",
            checked: false,
            scale,
        }
    }

    #[test]
    fn the_prefabs_vertical_layout_gives_the_height() {
        // One line of text: 5 + 28 + 10 + 18 * 1.1719 + 10 + 20 + 10 + 30 + 10 canvas units.
        let laid = lay_out(&spec("Are you sure?", 1.0), &mono);
        assert_eq!(laid.lines.len(), 1);
        let expected = 123.0 + 18.0 * ROBOTO_LINE;
        assert!((laid.size.y - expected).abs() < 1e-3, "{}", laid.size.y);
        assert_eq!(laid.size.x, 400.0);
        // Two extra lines make it two text lines taller.
        let three = lay_out(&spec("a\nb\nc", 1.0), &mono);
        assert_eq!(three.lines.len(), 3);
        assert!((three.size.y - laid.size.y - 2.0 * 18.0 * ROBOTO_LINE).abs() < 1e-3);
    }

    #[test]
    fn the_world_scale_is_the_holders_one_and_a_half_over() {
        let menu = lay_out(&spec("Are you sure?", 1.0), &mono);
        let world = lay_out(&spec("Are you sure?", 1.35), &mono);
        assert!((world.size.x - 540.0).abs() < 1e-3, "400 units at 1.35");
        assert!((world.size.y - menu.size.y * 1.35).abs() < 1e-2);
        // The measured dialog (the same layout without the checkbox row) is 125.55 + the text at
        // 1.35: this one is 30 units (40.5 px) taller.
        assert!((world.proceed[3] - 40.5).abs() < 1e-3, "30 units at 1.35");
    }

    #[test]
    fn the_checkbox_row_is_right_aligned_above_the_buttons() {
        let laid = lay_out(&spec("Are you sure?", 1.0), &mono);
        let [x, y, w, h] = laid.toggle;
        assert_eq!((w, h), (20.0, 20.0), "the toggle is 20 x 20");
        assert!(
            (x + w - 390.0).abs() < 1e-3,
            "its right edge is the inner right edge"
        );
        assert_eq!(y, laid.row_top);
        // The label ends one spacing (5) left of the toggle.
        let label_width = mono(LABEL, LABEL_PX, Face::Liberation);
        assert!((laid.label_left + label_width + 5.0 - x).abs() < 1e-3);
        // The buttons come 10 below the row; Cancel is at the left, proceed at the right.
        assert!((laid.cancel[1] - (y + 20.0 + 10.0)).abs() < 1e-3);
        assert_eq!(laid.cancel[0], 10.0);
        assert!((laid.proceed[0] + laid.proceed[2] - 390.0).abs() < 1e-3);
        assert_eq!((laid.cancel[2], laid.cancel[3]), (80.0, 30.0));
        // 10 of padding under the buttons.
        assert!((laid.size.y - (laid.cancel[1] + 30.0 + 10.0)).abs() < 1e-3);
    }

    #[test]
    fn long_text_wraps_at_the_inner_width() {
        let long = "word ".repeat(60);
        let laid = lay_out(&spec(&long, 1.0), &mono);
        assert!(laid.lines.len() > 1);
        for line in &laid.lines {
            assert!(
                mono(line, TEXT_PX, Face::Regular) <= 380.0 + 1e-3,
                "{line:?}"
            );
        }
        // A blank paragraph keeps its line (the scenario warning has one).
        let blank = lay_out(&spec("a\n\nb", 1.0), &mono);
        assert_eq!(blank.lines, vec!["a", "", "b"]);
    }

    fn drawn(spec: &Spec<'_>) -> UiFrame {
        let ui = UiAssets::empty();
        let mut frame = UiFrame::new(1.0);
        let actions = Actions {
            cancel: UiAction::ClosePanel,
            proceed: UiAction::Consume,
            toggle: UiAction::ToggleHelp,
        };
        emit(&mut frame, &ui, None, 1920.0, 1080.0, spec, actions, 100);
        frame
    }

    #[test]
    fn the_popup_blocks_the_screen_and_has_three_controls() {
        let frame = drawn(&spec("Are you sure?", 1.35));
        // The blocker (full screen), the panel, the two buttons and the checkbox are hit areas.
        assert_eq!(frame.hits.len(), 5);
        let [blocker, panel, cancel, proceed, toggle] = [100, 101, 102, 103, 104];
        let find = |id: UiId| frame.hits.iter().find(|hit| hit.0 == id).expect("hit");
        assert_eq!(find(blocker).2, Vec2::new(1920.0, 1080.0));
        // The popup is centred.
        let panel = find(panel);
        assert!(((panel.1.x + panel.2.x * 0.5) - 960.0).abs() < 1e-2);
        assert!(((panel.1.y + panel.2.y * 0.5) - 540.0).abs() < 1e-2);
        // The click on a control gets its action, the click beside them is swallowed.
        let centre = |id: UiId| {
            let hit = find(id);
            hit.1 + hit.2 * 0.5
        };
        assert_eq!(frame.action_at(centre(cancel)), Some(UiAction::ClosePanel));
        assert_eq!(frame.action_at(centre(proceed)), Some(UiAction::Consume));
        assert_eq!(frame.action_at(centre(toggle)), Some(UiAction::ToggleHelp));
        assert_eq!(
            frame.action_at(Vec2::new(5.0, 5.0)),
            Some(UiAction::Consume)
        );
    }

    #[test]
    fn the_blocker_is_the_scene_screen_blocker() {
        let frame = drawn(&spec("x", 1.0));
        assert!(matches!(
            frame.draws.first(),
            Some(UiDraw::Rect { color, .. }) if *color == [0, 0, 0, 102]
        ));
    }

    #[test]
    fn a_ticked_box_draws_a_checkmark_and_an_unticked_one_does_not() {
        let count = |checked: bool| {
            let mut s = spec("x", 1.0);
            s.checked = checked;
            drawn(&s)
                .draws
                .iter()
                .filter(|draw| matches!(draw, UiDraw::Text { text, .. } if text == "x"))
                .count()
        };
        // Without textures the check is a drawn "x" (the same text as the popup's own "x" line).
        assert_eq!(count(true), count(false) + 1);
    }
}
