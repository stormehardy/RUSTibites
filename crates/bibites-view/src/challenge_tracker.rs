//! The challenge tracker (`ChallengeTrackerPanel`, the `ChallengeInfo` panel of `Right Panels/QuickStatsHolder`) and the "Challenge" tab
//! (`OpenChallengeTracker`) that opens it, under the Information tab.
//!
//! Geometry, fonts and layout rules are the prefab's (level2, read with `tools/unity/dump_ui_tree.py` / `dump_ui_components.py`): a
//! 500-wide `base_icon` panel (alpha 0.949) hanging from the top-right corner of the `Right Panels` rectangle (10 in, 80 down: the Information
//! panel's place) with a `VerticalLayoutGroup` (padding 20 / 20 / 10 / 20, spacing 20) and a `ContentSizeFitter`: the title line, the body (a
//! vertical group, spacing 15: the description, the exit condition's "remaining" text, the score line and the three star lines, spacing 5), the
//! retry button (40 x 40 at 5, 5) and the close button (30 x 30 at 10, 10 from the top-right corner). A TMP line is `size * 1.172` high.
//!
//! What it does (`ChallengeTrackerPanel.cs`, `SimConditionText.cs`): the description and the stars' texts are the scenario's, the stars light up
//! with the stars attained (`unlockedStarColor` / `lockedStarColor`), the score is `scoringMetric.Evaluate()`, "(High is better)" /
//! "(Low is better)", the remaining text counts what is left of the exit condition's first condition, Retry asks first and starts the same
//! challenge again. While the panel is open the tab buttons are hidden (`onOpen` / `onClose` of the panel: `OpenButtons.SetActive`).

use bibites_core::settings::{Comparator, ConditionGroup, ConditionNode, SimCondition, SimMetric};
use bibites_core::world::World;

use crate::app::App;
use crate::species_panel::Drawer;
use crate::text_sdf::SdfFonts;
use crate::ui::{ControlCommand, FontRole, PanelKind, Rgba, UiAction, UiAssets, UiDraw, UiFrame};

/// Hit ids of the tracker (the block 9400..9449) and of its tab.
const HIT_BODY: u32 = 9400;
const HIT_CLOSE: u32 = 9401;
const HIT_RETRY: u32 = 9402;
const HIT_TAB: u32 = 9403;

/// The panel hangs 10 in from the right and 80 down (as the Information panel does).
const RIGHT_MARGIN: f32 = 10.0;
const TOP: f32 = 80.0;
const WIDTH: f32 = 500.0;
/// Padding left, right, top, bottom, and the spacing of the panel's vertical group.
const PAD: [f32; 4] = [20.0, 20.0, 10.0, 20.0];
const SPACING: f32 = 20.0;
/// The body's spacing and the star lines'.
const BODY_SPACING: f32 = 15.0;
const STAR_SPACING: f32 = 5.0;
/// A TMP line of Roboto: `size * 1.172` (23.44 at 20, 30.47 at 26).
const LINE_K: f32 = 1.172;
const TEXT_PX: f32 = 20.0;
const TITLE_PX: f32 = 26.0;
/// A star line: the 20 x 20 star, 15 to its text.
const STAR: f32 = 20.0;
const STAR_GAP: f32 = 15.0;

const INK: Rgba = [42, 42, 42, 255];
const BETTER_INK: Rgba = [56, 56, 56, 255];
const BLACK: Rgba = [0, 0, 0, 255];
const WHITE: Rgba = [255, 255, 255, 255];
/// `ChallengesProgress.unlockedStarColor` (0.906, 0.702, 0) and `lockedStarColor` (0.6).
const UNLOCKED: Rgba = [231, 179, 0, 255];
const LOCKED: Rgba = [153, 153, 153, 255];
/// The tab's text (38, 38, 38).
const TAB_INK: Rgba = [38, 38, 38, 255];
/// The Information tab is 30 high and the Challenge tab 40 under it (`pos (0, -40)`).
const TAB_H: f32 = 30.0;
const TAB_STEP: f32 = 40.0;
const TAB_PAD: [f32; 4] = [7.0, 7.0, 2.0, 2.0];
const TAB_SPACING: f32 = 5.0;
const TAB_ICON: f32 = 20.0;
const TAB_PX: f32 = 24.0;

/// `TimeSpan.ToString("g")` of a whole number of seconds: `[d:]h:mm:ss`, the hours not padded.
#[must_use]
pub fn time_span_g(seconds: f32) -> String {
    let total = seconds.round().max(0.0) as u64;
    let (days, hours, minutes, secs) = (
        total / 86_400,
        total % 86_400 / 3600,
        total % 3600 / 60,
        total % 60,
    );
    if days > 0 {
        format!("{days}:{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{hours}:{minutes:02}:{secs:02}")
    }
}

/// The first leaf of a condition group (`ConditionGroup.first`).
#[must_use]
pub fn first_condition(group: &ConditionGroup) -> Option<&SimCondition> {
    group.sub_conditions.iter().find_map(|node| match node {
        ConditionNode::Sim(condition) => Some(condition),
        ConditionNode::Group(inner) => first_condition(inner),
    })
}

/// `SimConditionText.UpdateText`: what is left of the condition, in the words of its metrics. `elapsed` is the challenge's time (the
/// `Time` metric of a challenge counts from its start). Other pairings leave the label as the prefab has it.
#[must_use]
pub fn remaining_text(condition: &SimCondition, world: &World, elapsed: f32) -> String {
    let value = |metric: &bibites_core::settings::SimulationMetric| match metric.metric {
        SimMetric::Time => elapsed,
        _ => world.metric_value(metric.metric, &metric.argument),
    };
    let mut num = (value(&condition.right) - value(&condition.left)).abs();
    if matches!(
        condition.comparator,
        Comparator::GreaterThan | Comparator::LessThan
    ) {
        num += 1.0;
    }
    let pair = |a: SimMetric, b: SimMetric| {
        (condition.left.metric == a && condition.right.metric == b)
            || (condition.left.metric == b && condition.right.metric == a)
    };
    let argument = &condition.left.argument;
    let plural = if num < 1.5 { "" } else { "s" };
    if pair(SimMetric::Time, SimMetric::Constant) {
        format!("{} remaining", time_span_g(num))
    } else if pair(SimMetric::MaterialCount, SimMetric::Constant) {
        format!("{num:.0} {argument}{plural} remaining")
    } else if pair(SimMetric::TagCount, SimMetric::Constant) {
        format!("{num:.0} {argument} tag{plural} remaining")
    } else if pair(SimMetric::SpeciesCount, SimMetric::Constant) {
        format!("{num:.0} {argument}{plural} remaining")
    } else if pair(SimMetric::MaterialBiomass, SimMetric::Constant) {
        format!("{num:.0} {argument} Energy remaining")
    } else if pair(SimMetric::TagBiomass, SimMetric::Constant) {
        format!("{num:.0} {argument} tag Energy remaining")
    } else if pair(SimMetric::SpeciesBiomass, SimMetric::Constant) {
        format!("{num:.0} {argument} Energy remaining")
    } else {
        "01:30 remaining".to_owned()
    }
}

/// The wrapped lines of what a text box of `width` holds (TMP's word wrap).
fn wrap(text: &str, width: f32, measure: &dyn Fn(&str) -> f32) -> Vec<String> {
    crate::ui_runtime::wrap_words(text, width, measure)
        .split('\n')
        .map(str::to_owned)
        .collect()
}

/// The panel's rectangles (`[x, y, w, h]`, reference px) for a description of `description` lines and star texts of `stars` lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The panel.
    pub panel: [f32; 4],
    /// The title line.
    pub title: [f32; 4],
    /// The close button.
    pub close: [f32; 4],
    /// The retry button.
    pub retry: [f32; 4],
    /// The description (its lines' tops follow the box's).
    pub description: [f32; 4],
    /// The exit condition's line.
    pub end: [f32; 4],
    /// The score line.
    pub score: [f32; 4],
    /// The star lines: the line, its star and its text.
    pub stars: [StarLine; 3],
}

/// One of the three star lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarLine {
    /// The line.
    pub line: [f32; 4],
    /// The star.
    pub star: [f32; 4],
    /// The text's box.
    pub text: [f32; 4],
}

/// Lays the panel out for `width` (the screen's width in reference px), a description of `description` lines and star texts of
/// `stars` lines.
#[must_use]
pub fn layout(width: f32, description: usize, stars: [usize; 3]) -> Layout {
    let line_h = TEXT_PX * LINE_K;
    let title_h = TITLE_PX * LINE_K;
    let x1 = width - RIGHT_MARGIN;
    let x0 = x1 - WIDTH;
    let inner_x = x0 + PAD[0];
    let inner_w = WIDTH - PAD[0] - PAD[1];
    let mut y = TOP + PAD[2];
    let title = [inner_x, y, inner_w, title_h];
    y += title_h + SPACING;
    let description_h = description.max(1) as f32 * line_h;
    let description = [inner_x, y, inner_w, description_h];
    y += description_h + BODY_SPACING;
    let end = [inner_x, y, inner_w, line_h];
    y += line_h + BODY_SPACING;
    let score = [inner_x, y, inner_w, line_h];
    y += line_h + BODY_SPACING;
    let mut star_lines = [StarLine {
        line: [0.0; 4],
        star: [0.0; 4],
        text: [0.0; 4],
    }; 3];
    for (index, lines) in stars.into_iter().enumerate() {
        let height = (lines.max(1) as f32 * line_h).max(STAR);
        star_lines[index] = StarLine {
            line: [inner_x, y, inner_w, height],
            star: [inner_x, y + (height - STAR) / 2.0, STAR, STAR],
            text: [
                inner_x + STAR + STAR_GAP,
                y,
                inner_w - STAR - STAR_GAP,
                height,
            ],
        };
        y += height + if index < 2 { STAR_SPACING } else { 0.0 };
    }
    let panel = [x0, TOP, WIDTH, y + PAD[3] - TOP];
    Layout {
        panel,
        title,
        close: [x1 - 10.0 - 30.0, TOP + 10.0, 30.0, 30.0],
        retry: [x0 + 5.0, TOP + 5.0, 40.0, 40.0],
        description,
        end,
        score,
        stars: star_lines,
    }
}

/// Draws the tracker when it is open, and the Challenge tab when it is not (and the Information panel is not either).
pub fn draw(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&SdfFonts>,
    width: f32,
    height: f32,
) {
    let Some(session) = app.challenge.as_ref() else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let d = Drawer::new(s, ui, metrics, app.cursor);
    let ref_width = width / s;
    if !app.panels.is_open(PanelKind::ChallengeTracker) {
        if !app.panels.is_open(PanelKind::Information) {
            draw_tab(frame, &d, ref_width);
        }
        return;
    }
    let px = TEXT_PX * s;
    let measure_ref = |text: &str| d.measure(text, px, false) / s;
    let inner_w = WIDTH - PAD[0] - PAD[1];
    let description_lines = wrap(&session.description, inner_w, &measure_ref);
    let star_text_w = inner_w - STAR - STAR_GAP;
    let star_lines: [Vec<String>; 3] =
        [0, 1, 2].map(|index| wrap(&session.star_descriptions[index], star_text_w, &measure_ref));
    let layout = layout(
        ref_width,
        description_lines.len(),
        [
            star_lines[0].len(),
            star_lines[1].len(),
            star_lines[2].len(),
        ],
    );
    d.slice(
        frame,
        "base_icon",
        5.0,
        1.0,
        layout.panel,
        [255, 255, 255, 242],
    );
    // The panel takes the clicks on it.
    frame.hit(
        HIT_BODY,
        d.at(layout.panel[0], layout.panel[1]),
        d.at(layout.panel[2], layout.panel[3]),
        UiAction::Consume,
    );
    let line_h = TEXT_PX * LINE_K;
    let text = |frame: &mut UiFrame, line: &str, x: f32, top: f32, bold: bool, color: Rgba| {
        frame.draws.push(UiDraw::Text {
            font: Some(
                if bold {
                    "Roboto-Bold SDF NoOutline"
                } else {
                    "Roboto-Regular SDF"
                }
                .to_owned(),
            ),
            text: line.to_owned(),
            pos: d.at(x, top + line_h / 2.0 - TEXT_PX * 0.585),
            px,
            color,
            role: if bold { FontRole::Bold } else { FontRole::Body },
        });
    };
    let centred = |frame: &mut UiFrame, line: &str, rect: [f32; 4], bold: bool, color: Rgba| {
        let width = d.measure(line, px, bold) / s;
        text(
            frame,
            line,
            rect[0] + (rect[2] - width) / 2.0,
            rect[1],
            bold,
            color,
        );
    };
    // The title ("Challenge Informations", Roboto-Bold 26, black, centred).
    let title_px = TITLE_PX * s;
    let title_text = "Challenge Informations";
    let title_width = d.measure(title_text, title_px, true) / s;
    frame.draws.push(UiDraw::Text {
        font: Some("Roboto-Bold SDF NoOutline".to_owned()),
        text: title_text.to_owned(),
        pos: d.at(
            layout.title[0] + (layout.title[2] - title_width) / 2.0,
            layout.title[1] + layout.title[3] / 2.0 - TITLE_PX * 0.585,
        ),
        px: title_px,
        color: BLACK,
        role: FontRole::Bold,
    });
    // Close and retry.
    d.icon(frame, "exit_icon", layout.close, WHITE);
    frame.hit(
        HIT_CLOSE,
        d.at(layout.close[0], layout.close[1]),
        d.at(layout.close[2], layout.close[3]),
        UiAction::Invoke(ControlCommand::ChallengeTrackerClose),
    );
    d.slice(frame, "base_icon", 5.0, 1.0, layout.retry, WHITE);
    d.icon(frame, "reset_icon", layout.retry, WHITE);
    frame.hit(
        HIT_RETRY,
        d.at(layout.retry[0], layout.retry[1]),
        d.at(layout.retry[2], layout.retry[3]),
        UiAction::Invoke(ControlCommand::ChallengeRestart),
    );
    if let Some(tip) = crate::tooltip::find(
        "UI/MainUI/Layer1/Panels/Right Panels/QuickStatsHolder/ChallengeInfo/ResetChallenge",
    ) {
        frame.tips.push(crate::tooltip::TipRect {
            pos: d.at(layout.retry[0], layout.retry[1]),
            size: d.at(layout.retry[2], layout.retry[3]),
            tip,
        });
    }
    // The description, left aligned.
    for (index, line) in description_lines.iter().enumerate() {
        text(
            frame,
            line,
            layout.description[0],
            layout.description[1] + index as f32 * line_h,
            false,
            INK,
        );
    }
    // The exit condition's first condition: what is left of it (Roboto-Bold, centred).
    let remaining = first_condition(&session.parameters.exit_condition).map_or_else(
        || "01:30 remaining".to_owned(),
        |condition| remaining_text(condition, &app.world, session.elapsed_s as f32),
    );
    centred(frame, &remaining, layout.end, true, INK);
    // The score line: "Current score:" and the score (bold, the score centred in what is left) and "(High is better)".
    let label = "Current score:";
    let better = if session.parameters.high_score_is_better {
        "(High is better)"
    } else {
        "(Low is better)"
    };
    let label_w = d.measure(label, px, true) / s;
    let better_w = d.measure(better, px, false) / s;
    text(frame, label, layout.score[0], layout.score[1], true, INK);
    let score_rect = [
        layout.score[0] + label_w,
        layout.score[1],
        layout.score[2] - label_w - better_w,
        layout.score[3],
    ];
    centred(
        frame,
        &format!("{:.0}", session.score),
        score_rect,
        true,
        INK,
    );
    text(
        frame,
        better,
        layout.score[0] + layout.score[2] - better_w,
        layout.score[1],
        false,
        BETTER_INK,
    );
    // The three stars: lit with the stars attained.
    for (index, star) in layout.stars.iter().enumerate() {
        let lit = usize::from(session.stars) > index;
        d.icon(
            frame,
            "star_0",
            star.star,
            if lit { UNLOCKED } else { LOCKED },
        );
        let lines = &star_lines[index];
        let top = star.text[1] + (star.text[3] - lines.len() as f32 * line_h) / 2.0;
        for (row, line) in lines.iter().enumerate() {
            text(
                frame,
                line,
                star.text[0],
                top + row as f32 * line_h,
                false,
                INK,
            );
        }
    }
}

/// The "Challenge" tab (`OpenChallengeTracker`): a `base_icon` tab like the Information one, 40 under it, as wide as its text and its small
/// "+" box need (`HorizontalLayoutGroup` padding 7 / 7 / 2 / 2, spacing 5, `ContentSizeFitter`), hanging from the same right edge.
fn draw_tab(frame: &mut UiFrame, d: &Drawer<'_>, ref_width: f32) {
    let s = d.s;
    let label = "Challenge";
    let label_w = d.measure(label, TAB_PX * s, true) / s;
    let w = TAB_PAD[0] + label_w + TAB_SPACING + TAB_ICON + TAB_PAD[1];
    let x1 = ref_width - RIGHT_MARGIN;
    let rect = [x1 - w, TOP + TAB_STEP, w, TAB_H];
    d.slice(frame, "base_icon", 5.0, 1.0, rect, WHITE);
    let mid = rect[1] + rect[3] / 2.0;
    frame.draws.push(UiDraw::Text {
        font: Some("Roboto-Bold SDF NoOutline".to_owned()),
        text: label.to_owned(),
        pos: d.at(rect[0] + TAB_PAD[0], mid - TAB_PX * 0.585),
        px: TAB_PX * s,
        color: TAB_INK,
        role: FontRole::Bold,
    });
    let icon = [
        x1 - TAB_PAD[1] - TAB_ICON,
        mid - TAB_ICON / 2.0,
        TAB_ICON,
        TAB_ICON,
    ];
    d.slice(frame, "base_icon", 5.0, 1.0, icon, WHITE);
    let plus = "+";
    let plus_w = d.measure(plus, TAB_PX * s, true) / s;
    frame.draws.push(UiDraw::Text {
        font: Some("Roboto-Bold SDF NoOutline".to_owned()),
        text: plus.to_owned(),
        pos: d.at(icon[0] + (TAB_ICON - plus_w) / 2.0, mid - TAB_PX * 0.585),
        px: TAB_PX * s,
        color: TAB_INK,
        role: FontRole::Bold,
    });
    frame.hit(
        HIT_TAB,
        d.at(rect[0], rect[1]),
        d.at(rect[2], rect[3]),
        UiAction::OpenPanel(PanelKind::ChallengeTracker),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::settings::SimCondition;

    #[test]
    fn a_time_span_is_dotnets_general_short_format() {
        assert_eq!(time_span_g(3600.0), "1:00:00");
        assert_eq!(time_span_g(90.0), "0:01:30");
        assert_eq!(time_span_g(59.6), "0:01:00");
        assert_eq!(time_span_g(90_061.0), "1:1:01:01");
        assert_eq!(time_span_g(0.0), "0:00:00");
    }

    #[test]
    fn the_remaining_text_says_what_is_left_in_the_words_of_the_metric() {
        let world = crate::app::LoadedWorld::load(None, 5, 3, 1)
            .expect("starter world")
            .world;
        let time = SimCondition::new(
            SimMetric::Time,
            "",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "3600",
        );
        assert_eq!(remaining_text(&time, &world, 0.0), "1:00:00 remaining");
        assert_eq!(remaining_text(&time, &world, 3510.0), "0:01:30 remaining");
        // `>` and `<` count one more (the condition is met one step later).
        let after = SimCondition::new(
            SimMetric::Time,
            "",
            Comparator::GreaterThan,
            SimMetric::Constant,
            "10",
        );
        assert_eq!(remaining_text(&after, &world, 5.0), "0:00:06 remaining");
        // A tag count: three bibites carry no tag, so 50 are missing.
        let tags = SimCondition::new(
            SimMetric::TagCount,
            "Champion",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "50",
        );
        assert_eq!(
            remaining_text(&tags, &world, 0.0),
            "50 Champion tags remaining"
        );
        let one = SimCondition::new(
            SimMetric::TagCount,
            "Champion",
            Comparator::GreaterThanOrEqual,
            SimMetric::Constant,
            "1",
        );
        assert_eq!(
            remaining_text(&one, &world, 0.0),
            "1 Champion tag remaining"
        );
    }

    #[test]
    fn the_first_condition_is_the_first_leaf_of_the_tree() {
        use bibites_core::settings::ConditionGroupLogic;
        let leaf = |metric, argument: &str, value: &str| {
            SimCondition::new(
                metric,
                argument,
                Comparator::GreaterThanOrEqual,
                SimMetric::Constant,
                value,
            )
        };
        let mut inner = ConditionGroup::new(ConditionGroupLogic::And);
        inner.sub_conditions.push(ConditionNode::Sim(leaf(
            SimMetric::TagCount,
            "Champion",
            "0",
        )));
        let mut outer = ConditionGroup::new(ConditionGroupLogic::Or);
        outer.sub_conditions.push(ConditionNode::Group(inner));
        outer
            .sub_conditions
            .push(ConditionNode::Sim(leaf(SimMetric::Time, "", "3600")));
        let first = first_condition(&outer).expect("a leaf");
        assert_eq!(first.left.metric, SimMetric::TagCount);
        assert!(first_condition(&ConditionGroup::new(ConditionGroupLogic::And)).is_none());
    }

    #[test]
    fn the_panel_is_the_prefabs_size_for_its_stored_text_and_grows_with_wrapped_lines() {
        // The prefab's stored state: a two-line description and three one-line stars: 500 x 299.55 (the stored RectTransform).
        let layout = layout(1920.0, 2, [1, 1, 1]);
        assert!((layout.panel[2] - 500.0).abs() < 1e-3);
        assert!(
            (layout.panel[3] - 299.55).abs() < 0.05,
            "{}",
            layout.panel[3]
        );
        assert!((layout.panel[0] + layout.panel[2] - 1910.0).abs() < 1e-3);
        assert_eq!(layout.panel[1], 80.0);
        // The body is 219.08 high as stored: 46.88 + 15 + 23.44 + 15 + 23.44 + 15 + 80.32.
        let body = layout.stars[2].line[1] + layout.stars[2].line[3] - layout.description[1];
        assert!((body - 219.08).abs() < 0.05, "{body}");
        // A third description line pushes everything under it down by one line.
        let taller = layout_with(3);
        assert!((taller.end[1] - layout.end[1] - 23.44).abs() < 0.01);
        assert!((taller.panel[3] - layout.panel[3] - 23.44).abs() < 0.01);
        // The buttons sit in the panel's corners.
        assert_eq!(layout.retry[..2], [layout.panel[0] + 5.0, 85.0]);
        assert!((layout.close[0] + 30.0 + 10.0 - 1910.0).abs() < 1e-3);
    }

    fn layout_with(description: usize) -> Layout {
        layout(1920.0, description, [1, 1, 1])
    }
}
