//! The challenge's end popup (`ChallengeEndPopup`, `TopLayer/Popuholder/ChallengeEndPopup`): "Congratulation!" with the stars and what the
//! result was, or "Better luck next time...", over a dimming screen blocker, with "Continue as a Simulation" and "Return To Menu".
//!
//! Geometry and fonts are the prefab's (level2): a `Button_UI` slice (`pixelsPerUnitMultiplier` 8) with a vertical layout (padding 10,
//! spacing 10) and a `ContentSizeFitter`: the title line (Roboto-Bold 36, its object scaled 0.74 so that the holder's 1.35 leaves it at 36),
//! the body (padding 10 left and right, spacing 15: the three stars, the middle one 15 higher, then the lost text or the win section: its lines
//! of Roboto 20, 23.44 high, 15 apart) and the button row (25 above it; "Continue as a Simulation" 275 x 35, a flexible spacer of at least 150,
//! "Return To Menu" 195.4 x 35). The scene's `Popuholder` scales the popup 1.35 in the world. What the popup says is
//! `ChallengeEndPopup.Show`.

use bibites_core::world::ChallengeSession;
use bibites_io::storage::ChallengeRecordOutcome;

use crate::app::App;
use crate::text_sdf::SdfFonts;
use crate::ui::{ControlCommand, FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame};

/// Hit ids: the blocker, the popup, "Continue as a Simulation", "Return To Menu".
const HIT_BLOCKER: u32 = 9450;
const HIT_PANEL: u32 = 9451;
const HIT_CONTINUE: u32 = 9452;
const HIT_RETURN: u32 = 9453;

/// The `Popuholder`'s scale in the world.
const SCALE: f32 = 1.35;
const PAD: f32 = 10.0;
const SPACING: f32 = 10.0;
const BODY_PAD: f32 = 10.0;
const BODY_SPACING: f32 = 15.0;
const LINE_K: f32 = 1.172;
const TEXT_PX: f32 = 20.0;
const TITLE_PX: f32 = 36.0;
/// The title line's object is scaled 0.74 (`1 / 1.35`) about its middle.
const TITLE_SCALE: f32 = 0.740_741;
const STAR: f32 = 40.0;
const STARS_H: f32 = 55.0;
const STAR_SPACING: f32 = 10.0;
/// The middle star is 15 higher than the others (its holder's bottom padding).
const STAR_RAISE: f32 = 15.0;
/// The row of two buttons: 25 above it.
const BUTTON_ROW_TOP: f32 = 25.0;
const CONTINUE: [f32; 2] = [275.0, 35.0];
const RETURN: [f32; 2] = [195.4, 35.0];
const SPACER_MIN: f32 = 150.0;
const BUTTON_PX: f32 = 22.35;
/// `Button_UI`: the slice border and the multiplier of its images.
const BORDER: f32 = 30.0;
const MULTIPLIER: f32 = 8.0;

const INK: Rgba = [42, 42, 42, 255];
const BLACK: Rgba = [0, 0, 0, 255];
const UNLOCKED: Rgba = [231, 179, 0, 255];
const LOCKED: Rgba = [153, 153, 153, 255];
/// The `screenBlocker`: black at 0.4 alpha.
const BLOCKER: Rgba = [0, 0, 0, 102];
/// `Continue As Sim` is drawn at 0.878 alpha, its text (0, 0, 0, 175); `Return To Menu` is opaque with (50, 50, 50).
const CONTINUE_TINT: Rgba = [255, 255, 255, 224];
const CONTINUE_INK: Rgba = [0, 0, 0, 175];
const RETURN_INK: Rgba = [50, 50, 50, 255];

/// A piece of a line: a text (bold or not) or a flexible spacer.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// Text and whether it is Roboto-Bold.
    Text(String, bool),
    /// A `LayoutElement` with a flexible width of 1.
    Spacer,
}

/// What the popup says.
#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    /// "Congratulation!" or "Better luck next time...".
    pub title: &'static str,
    /// Stars attained (they light up).
    pub stars: u8,
    /// The lost text's paragraphs, or the win section's lines.
    pub body: Body,
}

/// The body under the stars.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// `lostText`.
    Lost(String),
    /// `WinSection`'s active lines.
    Win(Vec<Vec<Part>>),
}

/// `ChallengeEndPopup.Show`: what the result is told as. `record` is what storing it found (`TrySetHighScoreOfChallenge`).
#[must_use]
pub fn content(session: &ChallengeSession, record: Option<&ChallengeRecordOutcome>) -> Content {
    let stars = session.stars;
    if stars == 0 {
        return Content {
            title: "Better luck next time...",
            stars,
            body: Body::Lost(
                "You failed....\nBut don't give up! This champion just wasn't up to the task\n\nTry observing what failed and how you can improve your champion"
                    .to_owned(),
            ),
        };
    }
    let text = |text: &str| Part::Text(text.to_owned(), false);
    let bold = |text: &str| Part::Text(text.to_owned(), true);
    let mut lines = vec![
        vec![
            text("You completed the"),
            bold(&session.challenge_name),
            text(" Challenge!"),
            Part::Spacer,
        ],
        vec![
            text("With a score of "),
            bold(&format!("{:.0}", session.score)),
            Part::Spacer,
        ],
    ];
    if stars < 3 {
        let next = &session.star_descriptions[usize::from(stars.max(1))];
        lines.push(vec![
            text("Try to"),
            bold(&next.to_lowercase()),
            text("next time"),
            Part::Spacer,
        ]);
    }
    let improved = record.is_none_or(|record| record.improved);
    if improved {
        lines.push(vec![
            text("This is a new Highscore for you and "),
            bold(&session.champion_name),
            text("!"),
            Part::Spacer,
        ]);
    } else if let Some(previous) = record
        .and_then(|record| record.previous.as_ref())
        .filter(|previous| previous.stars > 0)
    {
        lines.push(vec![
            text("Highscore:"),
            bold(&format!("{:.0}", previous.score.unwrap_or(0.0))),
            Part::Spacer,
            text("With Champion:"),
            bold(&previous.champion_name),
            Part::Spacer,
        ]);
    }
    Content {
        title: "Congratulation!",
        stars,
        body: Body::Win(lines),
    }
}

/// The popup's rectangles in canvas units of a popup whose top-left corner is the origin (the holder's scale applies when drawing).
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    /// The popup's size.
    pub size: [f32; 2],
    /// The title line.
    pub title: [f32; 4],
    /// The three stars.
    pub stars: [[f32; 4]; 3],
    /// The text lines of the body: the piece's text, whether it is bold, its x and the line's top.
    pub pieces: Vec<(String, bool, f32, f32)>,
    /// "Continue as a Simulation".
    pub continue_button: [f32; 4],
    /// "Return To Menu".
    pub return_button: [f32; 4],
}

/// Lays the popup out for `content`; `measure(text, px, bold)` is the width of a string in Roboto.
#[must_use]
pub fn lay_out(content: &Content, measure: &dyn Fn(&str, f32, bool) -> f32) -> Laid {
    let line_h = TEXT_PX * LINE_K;
    // The widest line decides the width (`ContentSizeFitter`), the buttons' row being the least.
    let button_row = CONTINUE[0] + SPACER_MIN + RETURN[0];
    let mut widest = button_row;
    let line_width = |line: &[Part]| -> f32 {
        let texts: Vec<f32> = line
            .iter()
            .filter_map(|part| match part {
                Part::Text(text, bold) => Some(measure(text, TEXT_PX, *bold)),
                Part::Spacer => None,
            })
            .collect();
        texts.iter().sum::<f32>() + 5.0 * (line.len().saturating_sub(1)) as f32
    };
    if let Body::Win(lines) = &content.body {
        for line in lines {
            widest = widest.max(line_width(line) + 2.0 * BODY_PAD);
        }
    }
    let width = widest + 2.0 * PAD;
    let body_w = width - 2.0 * PAD - 2.0 * BODY_PAD;
    let title_h = TITLE_PX * LINE_K;
    let mut y = PAD;
    let title = [PAD, y, width - 2.0 * PAD, title_h];
    y += title_h + SPACING;
    let body_x = PAD + BODY_PAD;
    // The stars: a centred row of three 40-wide holders, 10 apart.
    let row_w = 3.0 * STAR + 2.0 * STAR_SPACING;
    let row_x = body_x + (body_w - row_w) / 2.0;
    let mut stars = [[0.0; 4]; 3];
    for (index, star) in stars.iter_mut().enumerate() {
        let lift = if index == 1 { 0.0 } else { STAR_RAISE };
        *star = [
            row_x + index as f32 * (STAR + STAR_SPACING),
            y + lift,
            STAR,
            STAR,
        ];
    }
    y += STARS_H + BODY_SPACING;
    let mut pieces = Vec::new();
    match &content.body {
        Body::Lost(text) => {
            let lines =
                crate::ui_runtime::wrap_words(text, body_w, &|line| measure(line, TEXT_PX, false));
            for (index, line) in lines.split('\n').enumerate() {
                if !line.is_empty() {
                    pieces.push((line.to_owned(), false, body_x, y + index as f32 * line_h));
                }
            }
            y += lines.split('\n').count() as f32 * line_h;
        }
        Body::Win(lines) => {
            for (row, line) in lines.iter().enumerate() {
                // The flexible spacers share what the texts and the 5-unit spacings leave.
                let spacers = line.iter().filter(|part| **part == Part::Spacer).count();
                let fixed = line_width(line);
                let share = if spacers == 0 {
                    0.0
                } else {
                    ((body_w - fixed).max(0.0)) / spacers as f32
                };
                let mut x = body_x;
                for (index, part) in line.iter().enumerate() {
                    match part {
                        Part::Text(text, bold) => {
                            pieces.push((text.clone(), *bold, x, y));
                            x += measure(text, TEXT_PX, *bold);
                        }
                        Part::Spacer => x += share,
                    }
                    if index + 1 < line.len() {
                        x += 5.0;
                    }
                }
                y += line_h;
                if row + 1 < lines.len() {
                    y += BODY_SPACING;
                }
            }
        }
    }
    y += SPACING;
    let row_y = y + BUTTON_ROW_TOP;
    let continue_button = [PAD, row_y, CONTINUE[0], CONTINUE[1]];
    let return_button = [width - PAD - RETURN[0], row_y, RETURN[0], RETURN[1]];
    y = row_y + CONTINUE[1] + PAD;
    Laid {
        size: [width, y],
        title,
        stars,
        pieces,
        continue_button,
        return_button,
    }
}

/// Draws the popup (over its blocker) when a challenge has ended.
pub fn draw(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&SdfFonts>,
    width: f32,
    height: f32,
) {
    let Some(session) = app.challenge.as_ref().filter(|_| app.challenge_end_shown) else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let content = content(session, app.challenge_record.as_ref());
    let measure = |text: &str, px: f32, bold: bool| -> f32 {
        metrics.map_or(text.chars().count() as f32 * px * 0.5, |fonts| {
            fonts.measure_named(
                None,
                if bold { FontRole::Bold } else { FontRole::Body },
                text,
                px,
            )
        })
    };
    let laid = lay_out(&content, &measure);
    // Popup units -> screen px: the holder's scale and the UI scaler; centred on the screen.
    let k = SCALE * s;
    let origin = (
        width / 2.0 - laid.size[0] * k / 2.0,
        height / 2.0 - laid.size[1] * k / 2.0,
    );
    let at = |x: f32, y: f32| bibites_core::math::Vec2::new(origin.0 + x * k, origin.1 + y * k);
    let extent = |w: f32, h: f32| bibites_core::math::Vec2::new(w * k, h * k);
    frame.rect(
        bibites_core::math::Vec2::ZERO,
        bibites_core::math::Vec2::new(width, height),
        BLOCKER,
    );
    frame.hit(
        HIT_BLOCKER,
        bibites_core::math::Vec2::ZERO,
        bibites_core::math::Vec2::new(width, height),
        UiAction::Consume,
    );
    let slice =
        |frame: &mut UiFrame, rect: [f32; 4], tint: Rgba| match ui.texture_of_alias("Button_UI") {
            Some(texture) => frame.draws.push(UiDraw::NineSlice {
                texture,
                border: [BORDER; 4],
                dest_scale: SCALE / MULTIPLIER * s,
                pos: at(rect[0], rect[1]),
                size: extent(rect[2], rect[3]),
                tint,
            }),
            None => {
                frame.rect(
                    at(rect[0], rect[1]),
                    extent(rect[2], rect[3]),
                    [30, 31, 30, 255],
                );
                let edge = 2.0;
                frame.rect(
                    at(rect[0] + edge, rect[1] + edge),
                    extent(rect[2] - 2.0 * edge, rect[3] - 2.0 * edge),
                    [224, 224, 224, tint[3]],
                );
            }
        };
    slice(
        frame,
        [0.0, 0.0, laid.size[0], laid.size[1]],
        [255, 255, 255, 255],
    );
    frame.hit(
        HIT_PANEL,
        at(0.0, 0.0),
        extent(laid.size[0], laid.size[1]),
        UiAction::Consume,
    );
    // The title: Roboto-Bold 36, black, centred, its object scaled 0.74.
    let title_px = TITLE_PX * TITLE_SCALE * k;
    let title_width = measure(content.title, TITLE_PX, true) * TITLE_SCALE;
    frame.draws.push(UiDraw::Text {
        font: Some("Roboto-Bold SDF NoOutline".to_owned()),
        text: content.title.to_owned(),
        pos: at(
            laid.title[0] + (laid.title[2] - title_width) / 2.0,
            laid.title[1] + laid.title[3] / 2.0 - TITLE_PX * TITLE_SCALE * 0.585,
        ),
        px: title_px,
        color: BLACK,
        role: FontRole::Bold,
    });
    // The stars (`ChallengesProgress.unlockedStarColor` / `lockedStarColor`).
    for (index, star) in laid.stars.iter().enumerate() {
        if let Some(texture) = ui.texture_of_alias("star_0") {
            frame.draws.push(UiDraw::Image {
                texture,
                pos: at(star[0], star[1]),
                size: extent(star[2], star[3]),
                tint: if usize::from(content.stars) > index {
                    UNLOCKED
                } else {
                    LOCKED
                },
            });
        }
    }
    // The text of the body.
    let line_h = TEXT_PX * LINE_K;
    for (text, bold, x, y) in &laid.pieces {
        frame.draws.push(UiDraw::Text {
            font: Some(
                if *bold {
                    "Roboto-Bold SDF NoOutline"
                } else {
                    "Roboto-Regular SDF"
                }
                .to_owned(),
            ),
            text: text.clone(),
            pos: at(*x, y + line_h / 2.0 - TEXT_PX * 0.585),
            px: TEXT_PX * k,
            color: INK,
            role: if *bold {
                FontRole::Bold
            } else {
                FontRole::Body
            },
        });
    }
    // The two buttons (LiberationSans 22.35 centred).
    let buttons = [
        (
            laid.continue_button,
            "Continue as a Simulation",
            CONTINUE_TINT,
            CONTINUE_INK,
            UiAction::Invoke(ControlCommand::ChallengeContinue),
            HIT_CONTINUE,
            "LiberationSans SDF - Fallback",
        ),
        (
            laid.return_button,
            "Return To Menu",
            [255, 255, 255, 255],
            RETURN_INK,
            UiAction::Invoke(ControlCommand::ChallengeReturnToMenu),
            HIT_RETURN,
            "LiberationSans SDF",
        ),
    ];
    for (rect, label, tint, ink, action, id, font) in buttons {
        slice(frame, rect, tint);
        let label_width = measure(label, BUTTON_PX, false);
        frame.draws.push(UiDraw::Text {
            font: Some(font.to_owned()),
            text: label.to_owned(),
            pos: at(
                rect[0] + (rect[2] - label_width) / 2.0,
                rect[1] + rect[3] / 2.0 - BUTTON_PX * 0.585,
            ),
            px: BUTTON_PX * k,
            color: ink,
            role: FontRole::Body,
        });
        frame.hit(id, at(rect[0], rect[1]), extent(rect[2], rect[3]), action);
    }
    if let Some(tip) = crate::tooltip::find(
        "UI/MainUI/TopLayer/Popuholder/ChallengeEndPopup/buttonHolder/Continue As Sim",
    ) {
        frame.tips.push(crate::tooltip::TipRect {
            pos: at(laid.continue_button[0], laid.continue_button[1]),
            size: extent(laid.continue_button[2], laid.continue_button[3]),
            tip,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::settings::ChallengeParameters;

    /// Every glyph is half an em wide.
    fn mono(text: &str, px: f32, _bold: bool) -> f32 {
        text.chars().count() as f32 * px * 0.5
    }

    fn session(stars: u8, score: f32) -> ChallengeSession {
        let mut session = ChallengeSession::new(
            ChallengeParameters::game_defaults(),
            "Like Rabbits",
            "desc",
            "Basic bibite",
            "path",
            0.0,
        );
        session.star_descriptions = [
            "Reach 50 population".to_owned(),
            "Reach 100 population".to_owned(),
            "Reach 200 population".to_owned(),
        ];
        session.stars = stars;
        session.score = score;
        session
    }

    fn record(improved: bool, previous_stars: u8) -> ChallengeRecordOutcome {
        ChallengeRecordOutcome {
            previous: (previous_stars > 0).then(|| bibites_io::storage::StoredChallengeScore {
                stars: previous_stars,
                score: Some(188.0),
                champion_name: "Old Champion".to_owned(),
                champion_path: String::new(),
                time: String::new(),
            }),
            improved,
        }
    }

    fn texts(content: &Content) -> Vec<String> {
        match &content.body {
            Body::Win(lines) => lines
                .iter()
                .map(|line| {
                    line.iter()
                        .filter_map(|part| match part {
                            Part::Text(text, _) => Some(text.as_str()),
                            Part::Spacer => None,
                        })
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect(),
            Body::Lost(text) => vec![text.clone()],
        }
    }

    #[test]
    fn a_failed_run_says_better_luck_and_a_win_tells_the_result() {
        let lost = content(&session(0, 12.0), Some(&record(false, 0)));
        assert_eq!(lost.title, "Better luck next time...");
        assert!(matches!(lost.body, Body::Lost(ref text) if text.starts_with("You failed....")));
        // Two stars, a new best: what it was, what to try next (star 3, lower case) and the new highscore.
        let won = content(&session(2, 120.0), Some(&record(true, 1)));
        assert_eq!(won.title, "Congratulation!");
        assert_eq!(
            texts(&won),
            [
                "You completed the|Like Rabbits| Challenge!",
                "With a score of |120",
                "Try to|reach 200 population|next time",
                "This is a new Highscore for you and |Basic bibite|!",
            ]
        );
        // One star: the next one is the second; not a new best: the stored one is told instead.
        let kept = content(&session(1, 60.0), Some(&record(false, 3)));
        assert_eq!(
            texts(&kept),
            [
                "You completed the|Like Rabbits| Challenge!",
                "With a score of |60",
                "Try to|reach 100 population|next time",
                "Highscore:|188|With Champion:|Old Champion",
            ]
        );
        // Three stars: nothing left to try; a best that is not beaten and had no stars says nothing about it.
        let three = content(&session(3, 400.0), Some(&record(false, 0)));
        assert_eq!(texts(&three).len(), 2);
    }

    #[test]
    fn the_popup_is_the_prefabs_size_for_its_stored_text() {
        // The stored state shows every line (five); its text is as wide as the prefab's and the buttons' row decides the width: 640.4 x 389.39.
        let content = Content {
            title: "Congratulation!",
            stars: 3,
            body: Body::Win(
                (0..5)
                    .map(|_| vec![Part::Text("x".to_owned(), false), Part::Spacer])
                    .collect(),
            ),
        };
        let laid = lay_out(&content, &mono);
        assert!((laid.size[0] - 640.4).abs() < 0.01, "{:?}", laid.size);
        assert!((laid.size[1] - 389.39).abs() < 0.05, "{:?}", laid.size);
        // The middle star is 15 above the other two.
        assert_eq!(laid.stars[0][1] - laid.stars[1][1], 15.0);
        assert_eq!(laid.stars[2][1], laid.stars[0][1]);
        // The buttons: "Continue" at the left, "Return" at the right, 25 below the body.
        assert_eq!(laid.continue_button[0], 10.0);
        assert!((laid.return_button[0] + laid.return_button[2] - (640.4 - 10.0)).abs() < 0.01);
    }

    #[test]
    fn a_wide_line_widens_the_popup_and_a_lost_run_is_the_text_over_the_buttons() {
        let wide = Content {
            title: "Congratulation!",
            stars: 1,
            body: Body::Win(vec![vec![Part::Text("w".repeat(80), false), Part::Spacer]]),
        };
        let laid = lay_out(&wide, &mono);
        assert!(laid.size[0] > 640.4 + 100.0, "{:?}", laid.size);
        // A narrower face than the half-em test one (Roboto's average glyph is about 0.42 em).
        let narrow = |text: &str, px: f32, _bold: bool| text.chars().count() as f32 * px * 0.4;
        let lost = lay_out(&content(&session(0, 0.0), None), &narrow);
        // Three paragraphs and an empty line: four lines of the lost text, none of them wrapped at 600 wide.
        let first_line_y = lost.pieces[0].3;
        let last_line_y = lost.pieces.last().expect("lines").3;
        assert!((last_line_y - first_line_y - 3.0 * 23.44).abs() < 0.05);
    }
}
