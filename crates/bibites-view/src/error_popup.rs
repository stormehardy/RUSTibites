//! The game's error popup (`PopupManager.DisplayError` -> the `ErrorPopup` prefab of `sharedassets0`): a 400 wide `Button_UI` slice with a vertical layout
//! (padding left 10, top 5, bottom 10, spacing 10): "A Problem Occured" in red (24), the source (24), the text (18, wrapped at 380) and an OK button (80 x 30,
//! right-aligned, Roboto-Bold 20), centred on the screen. The first two lines are 28 high like the warning popup's title.
//!
//! The prefab was never captured in a running game; the geometry comes from its serialized values.

use bibites_core::math::Vec2;

use crate::ui::{FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiId};

const WIDTH: f32 = 400.0;
const PAD_LEFT: f32 = 10.0;
const PAD_TOP: f32 = 5.0;
const PAD_BOTTOM: f32 = 10.0;
const SPACING: f32 = 10.0;
const LINE_PX: f32 = 24.0;
const LINE_HEIGHT: f32 = 28.0;
const TEXT_PX: f32 = 18.0;
const TEXT_WIDTH: f32 = 380.0;
const BUTTON: [f32; 2] = [80.0, 30.0];
const BUTTON_PX: f32 = 20.0;
const BORDER: f32 = 30.0;
const MULTIPLIER: f32 = 8.0;
const INK: Rgba = [30, 31, 30, 255];
const RED: Rgba = [255, 0, 0, 255];

/// The text of the "Corrupted Save" popup (`SaveItemReference.InitSaveItem`, `SaveGamePanel.FillList`).
#[must_use]
pub fn corrupted_text(path: &std::path::Path, reason: &str) -> String {
    format!(
        "A corrupted save was found and couldn't ne displayed:
{}
Delete the file to stop this problem, or even better: Share it with the dev team

{}",
        path.display(),
        reason.lines().next().unwrap_or("")
    )
}

/// The world's error popup, if one waits (the world's `Popuholder` scales it by 1.35).
pub fn emit_world(frame: &mut UiFrame, app: &crate::app::App, ui: &UiAssets, metrics: Option<&crate::text_sdf::SdfFonts>, width: f32, height: f32) {
    if let Some((source, text)) = app.error_popups.first() {
        emit(frame, ui, metrics, width, height, source, text, 1.35, UiAction::Invoke(crate::ui::ControlCommand::DismissError), 44_500);
    }
}

/// Where things sit in a popup whose top-left is the origin (canvas units times the scale).
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    /// The popup's size.
    pub size: Vec2,
    /// The wrapped text lines.
    pub lines: Vec<String>,
    /// Tops of the title line, the source line and the first text line.
    pub tops: [f32; 3],
    /// Height of one text line.
    pub line_height: f32,
    /// The OK button: x, y, w, h.
    pub ok: [f32; 4],
}

/// Puts a line break inside every word that is wider than `width`.
fn break_long_words(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> String {
    let mut out = String::new();
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        for (word_index, word) in line.split(' ').enumerate() {
            if word_index > 0 {
                out.push(' ');
            }
            if measure(word) <= width {
                out.push_str(word);
                continue;
            }
            let mut piece = String::new();
            for character in word.chars() {
                piece.push(character);
                if measure(&piece) > width && piece.chars().count() > 1 {
                    piece.pop();
                    out.push_str(&piece);
                    out.push('\n');
                    piece.clear();
                    piece.push(character);
                }
            }
            out.push_str(&piece);
        }
    }
    out
}

/// Lays the popup out; `measure(text, px, bold)` is the width of a string.
#[must_use]
pub fn lay_out(text: &str, scale: f32, measure: &dyn Fn(&str, f32, bool) -> f32) -> Laid {
    let k = scale;
    // A word wider than the box (a long path) is broken where it no longer fits, as TextMeshPro does.
    let text = break_long_words(text, TEXT_WIDTH * k, &|line| measure(line, TEXT_PX * k, false));
    let lines = crate::tooltip::wrap(&text, TEXT_WIDTH * k, &|line| measure(line, TEXT_PX * k, false));
    let line_height = TEXT_PX * k * crate::tooltip::LINE_HEIGHT;
    let title = PAD_TOP * k;
    let source = title + (LINE_HEIGHT + SPACING) * k;
    let body = source + (LINE_HEIGHT + SPACING) * k;
    let holder = body + line_height * lines.len().max(1) as f32 + SPACING * k;
    let height = holder + (BUTTON[1] + PAD_BOTTOM) * k;
    // The OK button is anchored to the right of the 380 wide holder (its centre 40 in).
    let right = (PAD_LEFT + TEXT_WIDTH) * k;
    Laid {
        size: Vec2::new(WIDTH * k, height),
        lines,
        tops: [title, source, body],
        line_height,
        ok: [right - BUTTON[0] * k, holder, BUTTON[0] * k, BUTTON[1] * k],
    }
}

/// Draws the popup over the dimming screen blocker and registers its hits (`first_id` .. `first_id + 2`): the blocker and the panel swallow clicks, OK
/// does `dismiss`.
#[allow(clippy::too_many_arguments)]
pub fn emit(
    frame: &mut UiFrame,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    width: f32,
    height: f32,
    source: &str,
    text: &str,
    scale: f32,
    dismiss: UiAction,
    first_id: UiId,
) {
    let s = UiFrame::scaler(width, height);
    let measure = |text: &str, px: f32, bold: bool| -> f32 {
        let role = if bold { FontRole::Bold } else { FontRole::Body };
        metrics.map_or(text.chars().count() as f32 * px * 0.5, |fonts| fonts.measure_named(None, role, text, px))
    };
    let laid = lay_out(text, scale, &measure);
    let origin = Vec2::new(width * 0.5 - laid.size.x * s * 0.5, height * 0.5 - laid.size.y * s * 0.5);
    let at = |x: f32, y: f32| Vec2::new(origin.x + x * s, origin.y + y * s);
    let extent = |w: f32, h: f32| Vec2::new(w * s, h * s);
    frame.rect(Vec2::ZERO, Vec2::new(width, height), crate::warning_popup::BLOCKER);
    frame.hit(first_id, Vec2::ZERO, Vec2::new(width, height), UiAction::Consume);
    let slice = |frame: &mut UiFrame, rect: [f32; 4]| match ui.texture_of_alias("Button_UI") {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [BORDER; 4],
            dest_scale: scale / MULTIPLIER,
            pos: at(rect[0], rect[1]),
            size: extent(rect[2], rect[3]),
            tint: [255, 255, 255, 255],
        }),
        None => {
            frame.rect(at(rect[0], rect[1]), extent(rect[2], rect[3]), [30, 31, 30, 255]);
            frame.rect(at(rect[0] + 2.0, rect[1] + 2.0), extent(rect[2] - 4.0, rect[3] - 4.0), [224, 224, 224, 255]);
        }
    };
    slice(frame, [0.0, 0.0, laid.size.x, laid.size.y]);
    frame.hit(first_id + 1, at(0.0, 0.0), extent(laid.size.x, laid.size.y), UiAction::Consume);
    frame.bold("A Problem Occured", at(PAD_LEFT * scale, laid.tops[0]), LINE_PX * scale * s, RED);
    frame.bold(source, at(PAD_LEFT * scale, laid.tops[1]), LINE_PX * scale * s, INK);
    for (index, line) in laid.lines.iter().enumerate() {
        if !line.is_empty() {
            frame.text(
                line.clone(),
                at(PAD_LEFT * scale, laid.tops[2] + laid.line_height * index as f32),
                TEXT_PX * scale * s,
                INK,
            );
        }
    }
    slice(frame, laid.ok);
    let label_width = measure("OK", BUTTON_PX * scale, true);
    let label_height = BUTTON_PX * scale * crate::tooltip::LINE_HEIGHT;
    frame.bold(
        "OK",
        at(laid.ok[0] + (laid.ok[2] - label_width) * 0.5, laid.ok[1] + (laid.ok[3] - label_height) * 0.5),
        BUTTON_PX * scale * s,
        INK,
    );
    frame.hit(first_id + 2, at(laid.ok[0], laid.ok[1]), extent(laid.ok[2], laid.ok[3]), dismiss);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half_em(text: &str, px: f32, _bold: bool) -> f32 {
        text.chars().count() as f32 * px * 0.5
    }

    #[test]
    fn the_popup_stacks_title_source_text_and_the_ok_button() {
        let laid = lay_out("one\ntwo", 1.0, &half_em);
        assert_eq!(laid.lines.len(), 2);
        assert!((laid.tops[0] - 5.0).abs() < 1e-5 && (laid.tops[1] - 43.0).abs() < 1e-5 && (laid.tops[2] - 81.0).abs() < 1e-5);
        // The button sits under the text, right-aligned in the 380 wide holder.
        assert!((laid.ok[0] + laid.ok[2] - 390.0).abs() < 1e-5);
        assert!((laid.ok[1] - (81.0 + laid.line_height * 2.0 + 10.0)).abs() < 1e-3);
        assert!((laid.size.y - (laid.ok[1] + 30.0 + 10.0)).abs() < 1e-3);
    }

    #[test]
    fn the_popup_is_centred_blocks_the_screen_and_ok_dismisses() {
        let ui = UiAssets::empty();
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, &ui, None, 1920.0, 1080.0, "Corrupted Save", "text", 1.0, UiAction::ClosePanel, 100);
        assert_eq!(frame.hits.len(), 3);
        let find = |id: UiId| frame.hits.iter().find(|hit| hit.0 == id).expect("hit");
        assert_eq!(find(100).2, Vec2::new(1920.0, 1080.0));
        let panel = find(101);
        assert!(((panel.1.x + panel.2.x * 0.5) - 960.0).abs() < 1e-2);
        let ok = find(102);
        assert_eq!(frame.action_at(ok.1 + ok.2 * 0.5), Some(UiAction::ClosePanel));
        assert_eq!(frame.action_at(Vec2::new(5.0, 5.0)), Some(UiAction::Consume));
    }

    #[test]
    fn a_word_wider_than_the_box_is_broken_inside() {
        let laid = lay_out(&"x".repeat(200), 1.0, &half_em);
        assert!(laid.lines.len() >= 3, "{:?}", laid.lines);
        assert!(laid.lines.iter().all(|line| half_em(line, TEXT_PX, false) <= TEXT_WIDTH + 1e-3));
        assert_eq!(laid.lines.concat().len(), 200);
    }

    #[test]
    fn a_long_text_wraps_at_380() {
        let long = "word ".repeat(60);
        let laid = lay_out(&long, 1.0, &half_em);
        assert!(laid.lines.len() > 2);
        assert!(laid.lines.iter().all(|line| half_em(line, TEXT_PX, false) <= TEXT_WIDTH + 1e-3));
    }
}
