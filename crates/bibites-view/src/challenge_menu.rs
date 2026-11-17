//! The challenge list's champion section of the New Game screen (`ScenarioSelectorPanel`, the `ChampionSelectionPanel` under the scenario's
//! information): "Champion Selection", the "Select Champion" button until a champion is picked and then the champion's picture and name
//! with "Select Other", and at the bottom "Start Challenge" (`StartChallenge(true)`, not interactable without a champion) and the small
//! button that launches the challenge as a scenario (`StartChallenge(false)`).
//!
//! In the challenge list the `ParametersPanel` (the scenario modifiers) is hidden (`showWithScenarios`) and this panel takes its place: both
//! have a flexible height of 1, so the section is exactly as big as the modifiers' panel. The panel was captured inactive, so its layout is
//! solved here from the prefab's layout groups (`Content`: a vertical group with 25 around it, `TopLine`, a spacer, the `SelectChampion`
//! button or the `Body`, a spacer and `Bottom`; the two spacers share what is left).

use crate::menu::MenuState;
use crate::species_panel::Drawer;
use crate::ui::{MenuCommand, Rgba, UiAction, UiDraw, UiFrame, UiId};

/// The panel's padding (`Content` of the `VerticalLayoutGroup`).
const PAD: f32 = 25.0;
/// `TopLine`'s height: the title's Roboto-Bold 42 line.
const TITLE_H: f32 = 49.22;
/// The title's width (the stored `TopLine` is as wide as the text).
const TITLE_W: f32 = 428.11;
/// `SelectChampion`'s preferred size.
const SELECT: [f32; 2] = [410.3, 57.5];
/// `Body`'s height: the champion box (12 + the 110 picture + 12).
const BODY_H: f32 = 134.0;
/// `Bottom`'s height (`StartButton` is 400 x 80).
const BOTTOM_H: f32 = 80.0;
/// `StartButton`'s width.
const START_W: f32 = 400.0;
/// The two small 80 x 80 buttons / icons of the bottom row.
const ICON: f32 = 80.0;
/// `Select Other`'s size.
const OTHER: f32 = 70.0;
/// The picture's box.
const PICTURE: f32 = 110.0;
/// `ChampionPreview` (`HorizontalLayoutGroup` padding 20 / 35 / 12 / 12, spacing 25) and `Body` (spacing 50).
const BOX_PAD: [f32; 4] = [20.0, 35.0, 12.0, 12.0];
const BOX_SPACING: f32 = 25.0;
const BODY_SPACING: f32 = 50.0;
/// The ink of the panel's texts.
const INK: Rgba = [42, 42, 42, 255];
const BUTTON_INK: Rgba = [38, 38, 38, 255];
const WHITE: Rgba = [255, 255, 255, 255];
/// The dark box of the champion (`Background`, (0.110, 0.114, 0.137)).
const BOX_BACK: Rgba = [28, 29, 35, 255];
/// A button that is not interactable: `Button.disabledColor` (0.784 grey, 0.502 alpha) over its white image.
const DISABLED: Rgba = [200, 200, 200, 128];
/// The name's size (Roboto-Regular 40) and the buttons' texts' (Boston Black 40 / 42.65).
const NAME_PX: f32 = 40.0;
const TITLE_PX: f32 = 42.0;
const START_PX: f32 = 40.0;
const SELECT_PX: f32 = 42.65;

/// The rectangles (`[x, y, w, h]`, reference px, from the panel's top left corner) of the section's parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// "Champion Selection".
    pub title: [f32; 4],
    /// "Select Champion" (until a champion is picked).
    pub select: Option<[f32; 4]>,
    /// The champion's box, picture, name and "Select Other" (once picked).
    pub body: Option<Body>,
    /// The `Warner` (`bug_icon`: "You cannot save a challenge").
    pub warner: [f32; 4],
    /// "Start Challenge".
    pub start: [f32; 4],
    /// The small button that launches the challenge as a scenario.
    pub scenario: [f32; 4],
}

/// The champion row of the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    /// `ChampionPreview` (the framed dark box).
    pub boxed: [f32; 4],
    /// `BibitePreview`.
    pub picture: [f32; 4],
    /// `Champion Name`.
    pub name: [f32; 4],
    /// `Select Other`.
    pub other: [f32; 4],
}

/// Solves the section for a panel of `size` (reference px); `name_width` is the champion's name's width (`None`: nothing is picked).
#[must_use]
pub fn layout(size: [f32; 2], name_width: Option<f32>) -> Layout {
    let inner_w = size[0] - 2.0 * PAD;
    let inner_h = size[1] - 2.0 * PAD;
    let middle_h = if name_width.is_some() {
        BODY_H
    } else {
        SELECT[1]
    };
    // The two spacers (`LayoutElement` flexible height 1) share what the others leave.
    let spacer = ((inner_h - TITLE_H - middle_h - BOTTOM_H) / 2.0).max(0.0);
    let title = [PAD + (inner_w - TITLE_W) / 2.0, PAD, TITLE_W, TITLE_H];
    let middle_y = PAD + TITLE_H + spacer;
    let (select, body) = match name_width {
        None => (
            Some([
                PAD + (inner_w - SELECT[0]) / 2.0,
                middle_y,
                SELECT[0],
                SELECT[1],
            ]),
            None,
        ),
        Some(name_w) => {
            let box_w = BOX_PAD[0] + PICTURE + BOX_SPACING + name_w + BOX_PAD[1];
            let total = box_w + BODY_SPACING + OTHER;
            let x = PAD + (inner_w - total) / 2.0;
            let boxed = [x, middle_y, box_w, BODY_H];
            let picture = [x + BOX_PAD[0], middle_y + BOX_PAD[2], PICTURE, PICTURE];
            let name_h = 46.88;
            let name = [
                x + BOX_PAD[0] + PICTURE + BOX_SPACING,
                middle_y + (BODY_H - name_h) / 2.0,
                name_w,
                name_h,
            ];
            let other = [
                x + box_w + BODY_SPACING,
                middle_y + (BODY_H - OTHER) / 2.0,
                OTHER,
                OTHER,
            ];
            (
                None,
                Some(Body {
                    boxed,
                    picture,
                    name,
                    other,
                }),
            )
        }
    };
    // `Bottom`: a spacer, `StartButton`, a spacer (25 between them); the spacers are flexible.
    let bottom_y = size[1] - PAD - BOTTOM_H;
    let side = ((inner_w - START_W - 2.0 * 25.0) / 2.0).max(0.0);
    let start_x = PAD + side + 25.0;
    let start = [start_x, bottom_y, START_W, BOTTOM_H];
    let warner = [PAD + side / 2.0 - ICON / 2.0, bottom_y, ICON, ICON];
    let scenario_x = start_x + START_W + 25.0 + side - ICON;
    let scenario = [scenario_x, bottom_y, ICON, ICON];
    Layout {
        title,
        select,
        body,
        warner,
        start,
        scenario,
    }
}

/// The tooltip trigger of a control of the section (`tooltip_data`).
fn tip_path(tail: &str) -> String {
    format!(
        "UI/NewGameScenarios/ScenarioSelector/Holder/RightPanels/RightGroup/ChampionSelectionPanel/Content/{tail}"
    )
}

/// Draws the section into the panel at `pos` / `size` (reference px) and gives its buttons their actions. `scale` is the
/// reference-to-screen factor; `next_hit` the next free hit id.
pub fn emit(
    frame: &mut UiFrame,
    d: &Drawer<'_>,
    menu: &MenuState,
    pos: [f32; 2],
    size: [f32; 2],
    next_hit: &mut UiId,
) {
    let s = d.s;
    let champion = menu.champion();
    let name = champion.map(|champion| champion.name.clone());
    let name_width = name
        .as_deref()
        .map(|text| d.measure(text, NAME_PX * s, false) / s);
    let layout = layout(size, name_width);
    let abs = |rect: [f32; 4]| [pos[0] + rect[0], pos[1] + rect[1], rect[2], rect[3]];
    // The panel (`base_icon`, sliced at 0.5 pixels per unit).
    d.slice(
        frame,
        "base_icon",
        5.0,
        2.0,
        [pos[0], pos[1], size[0], size[1]],
        WHITE,
    );
    // Title.
    let title = abs(layout.title);
    let title_px = TITLE_PX * s;
    let title_width = d.measure("Champion Selection", title_px, true);
    frame.bold(
        "Champion Selection",
        d.at(
            title[0] + title[2] / 2.0 - title_width / s / 2.0,
            title[1] + title[3] / 2.0 - TITLE_PX * 0.585,
        ),
        title_px,
        INK,
    );
    let mut button = |frame: &mut UiFrame,
                      rect: [f32; 4],
                      label: Option<(&str, f32)>,
                      icon: Option<&str>,
                      tint: Rgba,
                      action: Option<UiAction>,
                      tip: Option<String>| {
        d.slice(frame, "base_icon", 5.0, 2.0, rect, tint);
        if let Some(sprite) = icon {
            d.icon(frame, sprite, rect, WHITE);
        }
        if let Some((text, px)) = label {
            let fitted = d.fitted(text, px * s, true, rect[2] * s * 0.95);
            let width = d.measure(text, fitted, true);
            frame.draws.push(UiDraw::Text {
                font: Some("Boston Black SDF".to_owned()),
                text: text.to_owned(),
                pos: d.at(
                    rect[0] + rect[2] / 2.0 - width / s / 2.0,
                    rect[1] + rect[3] / 2.0 - fitted / s * 0.585,
                ),
                px: fitted,
                color: BUTTON_INK,
                role: crate::ui::FontRole::Bold,
            });
        }
        if let Some(action) = action {
            let id = *next_hit;
            *next_hit += 1;
            frame.hit(id, d.at(rect[0], rect[1]), d.at(rect[2], rect[3]), action);
        }
        if let Some(path) = tip
            && let Some(tip) = crate::tooltip::find(&path)
        {
            frame.tips.push(crate::tooltip::TipRect {
                pos: d.at(rect[0], rect[1]),
                size: d.at(rect[2], rect[3]),
                tip,
            });
        }
    };
    if let Some(select) = layout.select {
        button(
            frame,
            abs(select),
            Some(("Select Champion", SELECT_PX)),
            None,
            WHITE,
            Some(UiAction::Menu(MenuCommand::SelectChampion)),
            None,
        );
    }
    if let Some(body) = layout.body {
        // `ChampionPreview`: the framed box with its dark background (9 px in), the bibite and the name.
        let boxed = abs(body.boxed);
        d.slice(frame, "gray_panel", 10.0, 1.0, boxed, WHITE);
        frame.rect(
            d.at(boxed[0] + 9.0, boxed[1] + 9.0),
            d.at(boxed[2] - 18.0, boxed[3] - 18.0),
            BOX_BACK,
        );
        if let Some(texture) = menu.champion_picture() {
            let picture = abs(body.picture);
            frame.draws.push(UiDraw::Pixels {
                texture,
                pos: d.at(picture[0], picture[1]),
                size: d.at(picture[2], picture[3]),
            });
        }
        if let Some(text) = name.as_deref() {
            let rect = abs(body.name);
            frame.text(
                text.to_owned(),
                d.at(rect[0], rect[1] + rect[3] / 2.0 - NAME_PX * 0.585),
                NAME_PX * s,
                WHITE,
            );
        }
        button(
            frame,
            abs(body.other),
            None,
            Some("reset_icon"),
            WHITE,
            Some(UiAction::Menu(MenuCommand::SelectChampion)),
            Some(tip_path("Body/Select Other")),
        );
    }
    // `Bottom`: the warner, "Start Challenge" (not interactable without a champion) and the scenario launcher.
    let warner = abs(layout.warner);
    d.icon(frame, "bug_icon", warner, WHITE);
    if let Some(tip) = crate::tooltip::find(&tip_path("Bottom/spacer (1)/Warner")) {
        frame.tips.push(crate::tooltip::TipRect {
            pos: d.at(warner[0], warner[1]),
            size: d.at(warner[2], warner[3]),
            tip,
        });
    }
    let ready = champion.is_some();
    button(
        frame,
        abs(layout.start),
        Some(("Start Challenge", START_PX)),
        None,
        if ready { WHITE } else { DISABLED },
        ready.then_some(UiAction::Menu(MenuCommand::StartChallenge(true))),
        None,
    );
    button(
        frame,
        abs(layout.scenario),
        None,
        Some("simulation_icon"),
        WHITE,
        Some(UiAction::Menu(MenuCommand::StartChallenge(false))),
        Some(tip_path("Bottom/spacer (2)/StartButton (1)")),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spacers_share_what_the_rows_leave() {
        // A panel as big as the scenario modifiers' (1124 x 480.5): nothing picked, then a champion.
        let none = layout([1124.0, 480.5], None);
        let select = none.select.expect("Select Champion");
        assert!(none.body.is_none());
        // Centred, between the title and the bottom row with equal gaps.
        assert!((select[0] + select[2] / 2.0 - 562.0).abs() < 0.01);
        let above = select[1] - (none.title[1] + none.title[3]);
        let below = none.start[1] - (select[1] + select[3]);
        assert!(
            (above - below).abs() < 0.01 && above > 100.0,
            "{above} {below}"
        );
        assert!((none.start[1] + none.start[3] - (480.5 - 25.0)).abs() < 0.01);
        let some = layout([1124.0, 480.5], Some(268.37));
        assert!(some.select.is_none());
        let body = some.body.expect("the champion row");
        // The box is 20 + 110 + 25 + name + 35 wide and 134 high; the picture sits 12 below its top; "Select Other" 50 right of it.
        assert!((body.boxed[2] - (20.0 + 110.0 + 25.0 + 268.37 + 35.0)).abs() < 0.01);
        assert_eq!(body.boxed[3], 134.0);
        assert_eq!(body.picture[1] - body.boxed[1], 12.0);
        assert!((body.other[0] - (body.boxed[0] + body.boxed[2] + 50.0)).abs() < 0.01);
        // The row as a whole is centred.
        let left = body.boxed[0];
        let right = body.other[0] + body.other[2];
        assert!(((left + right) / 2.0 - 562.0).abs() < 0.01);
    }

    #[test]
    fn the_bottom_row_is_a_flexible_spacer_the_start_button_and_another_spacer() {
        let layout = layout([1124.0, 480.5], None);
        // `StartButton` 400 wide in the middle; the warner centred in the left spacer, the launcher at the right end of the right one.
        assert!((layout.start[0] + 200.0 - 562.0).abs() < 0.01);
        assert_eq!(layout.start[2..], [400.0, 80.0]);
        let left_spacer = (25.0, layout.start[0] - 25.0 - 25.0);
        assert!(
            (layout.warner[0] + 40.0 - (left_spacer.0 + left_spacer.1 / 2.0)).abs() < 0.01,
            "{:?}",
            layout.warner
        );
        assert!((layout.scenario[0] + 80.0 - (1124.0 - 25.0)).abs() < 0.01);
    }
}
