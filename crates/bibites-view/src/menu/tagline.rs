//! The port's own line on the title screen: "RUSTibites Faithful".
//!
//! A title card in the top left corner, made of what the title screen itself uses: `Boston Black SDF` in the Bold style the version label
//! takes (TMP's synthetic bold, see `text_sdf`), in the white of the version label and the Patreon text. It is not part of the game's
//! capture (the game has no such line), so it is added over the captured frame and takes no clicks.

use bibites_core::math::Vec2;

use crate::ui::{FontRole, Rgba, UiDraw, UiFrame};

/// The card's one line.
const NAME: &str = "RUSTibites Faithful";
/// `Boston Black SDF` in the Bold style (the version label's face).
const FONT: &str = "Boston Black SDF +bold";
/// The version label's and the Patreon text's colour.
const WHITE: Rgba = [255, 255, 255, 255];

/// Text sizes, and the distance of the card from the screen's top left corner, in reference pixels (1920 x 1080).
const NAME_PX: f32 = 66.0;
const LEFT: f32 = 56.0;
const TOP: f32 = 44.0;

/// Adds the card to the title frame (drawn over everything else of it).
pub fn add(frame: &mut UiFrame, width: f32, height: f32) {
    let scale = UiFrame::scaler(width, height);
    push(frame, NAME, NAME_PX * scale, Vec2::new(LEFT * scale, TOP * scale));
}

fn push(frame: &mut UiFrame, text: &str, px: f32, pos: Vec2) {
    frame.draws.push(UiDraw::Text {
        font: Some(FONT.to_owned()),
        text: text.to_owned(),
        pos,
        px,
        color: WHITE,
        role: FontRole::Body,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(frame: &UiFrame) -> Vec<(String, Vec2, f32, Option<String>)> {
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, pos, px, font, .. } => Some((text.clone(), *pos, *px, font.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_card_is_one_bold_boston_line_in_the_top_left_corner_and_takes_no_clicks() {
        let mut frame = UiFrame::new(1.0);
        add(&mut frame, 1920.0, 1080.0);
        let lines = texts(&frame);
        assert_eq!(lines.len(), 1, "the edition's name and nothing else (no author line)");
        assert_eq!(lines[0].0, "RUSTibites Faithful");
        for (_, pos, _, font) in &lines {
            assert_eq!(font.as_deref(), Some("Boston Black SDF +bold"));
            assert!(pos.x >= 0.0 && pos.y >= 0.0 && pos.x < 200.0 && pos.y < 200.0, "{pos:?}");
        }
        assert!(!lines[0].0.contains("Storme"));
        assert!(frame.hits.is_empty() && frame.actions.is_empty(), "the card is not a control");
        // Every draw is white.
        assert!(frame
            .draws
            .iter()
            .all(|draw| !matches!(draw, UiDraw::Text { color, .. } if *color != [255, 255, 255, 255])));
    }

    #[test]
    fn the_card_follows_the_ui_scale_of_a_smaller_window() {
        let mut full = UiFrame::new(1.0);
        add(&mut full, 1920.0, 1080.0);
        let mut half = UiFrame::new(1.0);
        add(&mut half, 960.0, 540.0);
        let (full, half) = (texts(&full), texts(&half));
        for (big, small) in full.iter().zip(&half) {
            assert!((small.2 * 2.0 - big.2).abs() < 0.01, "{} vs {}", small.2, big.2);
            assert!((small.1.x * 2.0 - big.1.x).abs() < 0.01 && (small.1.y * 2.0 - big.1.y).abs() < 0.01);
        }
    }
}
