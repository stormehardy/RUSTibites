//! The target labels of "Show Targets" (`UserSettings.ShowBibiteTargets`, the View panel's switch): while a living bibite is selected and its field of view
//! is shown, every object it sees that weighs more than 5 % of the heaviest of its channel gets a label (`FieldOfViewDisplay`, `WeightDisplay`).
//!
//! A label is a world-space canvas: it hangs at `target + (-m, m)` with `m = max(scale x 12.5, 10)` for a bibite or an egg and `max(scale x 5, 10)` for a
//! pellet (`scale` = the object's `localScale.x`: a bibite's size, a pellet's `sqrt(amount / 20)`), joined to the object by a white 0.5 wide pointer, and
//! has one row (a corpse has two: the bibite and the meat of it, 3.52 above): an underline, the 3 x 3 icon of the kind of target, the weight with two
//! decimals and, in brackets, its share of the channel's weights (`FloatValueTextHandle`: factor 100, one decimal, `%`), all Roboto-Bold 3 high and white.
//! The whole label fades in with the square root of the share of the channel's heaviest weight (`CanvasGroup.alpha`) and is hidden under 0.05 of it.
//!
//! The pointers are scene primitives (a rotated line); the rest is drawn by the HUD under every panel.

use bibites_core::math::Vec2;
use bibites_core::senses::{FovTargets, TargetKind};

use crate::render::{Camera, Primitive};
use crate::text_sdf::SdfFonts;
use crate::ui::{FontRole, UiAssets, UiDraw, UiFrame};

/// `WeightDisplay.MinThresholdForVisibility`: a label shows only above this share of its channel's heaviest weight.
pub const MIN_VISIBLE: f32 = 0.05;
/// The font size of every text of a label, in world units (the TMP size of the prefab).
const TEXT_SIZE: f32 = 3.0;
/// The icon's size (`LayoutElement.preferred` 3 x 3).
const ICON: f32 = 3.0;
/// The spacing of the row's layout group.
const SPACING: f32 = 0.5;
/// How far the second row (`Container (1)`) is above the first.
const ROW_STEP: f32 = 3.52;
/// The thickness of the underline (`underline`, 0.5) and of the pointer (`Pointer`, 0.5).
const LINE: f32 = 0.5;
/// The font asset of the label's texts.
const FONT: &str = "Roboto-Bold SDF NoOutline";

/// One row of a label.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    /// The kind of target the row's icon shows.
    pub kind: TargetKind,
    /// The target's FOV weight.
    pub weight: f32,
    /// Its share of the weights of the channel (`weight / sum`).
    pub share: f32,
}

impl Row {
    /// The weight's text (`FloatValueTextHandle`, two decimals).
    #[must_use]
    pub fn weight_text(&self) -> String {
        format!("{:.2}", self.weight)
    }

    /// The share's text (`FloatValueTextHandle`, factor 100, one decimal, `%`).
    #[must_use]
    pub fn share_text(&self) -> String {
        format!("{:.1}%", self.share * 100.0)
    }
}

/// A label: where its object is, where it hangs and what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// The object's world position.
    pub target: Vec2,
    /// `m`: the label hangs `m` left of the object and `m` above it.
    pub offset: f32,
    /// `CanvasGroup.alpha`.
    pub alpha: f32,
    /// One row, or two for a corpse.
    pub rows: Vec<Row>,
}

impl Label {
    /// Where the label hangs (its bottom middle, the pointer's start).
    #[must_use]
    pub fn origin(&self) -> Vec2 {
        self.target + Vec2::new(-self.offset, self.offset)
    }

    /// The pointer's primitive: a white line from the label to the object.
    #[must_use]
    pub fn pointer(&self) -> Primitive {
        Primitive::Line {
            a: self.origin(),
            b: self.target,
            thickness: LINE,
            color: [255, 255, 255, (self.alpha.clamp(0.0, 1.0) * 255.0).round() as u8],
        }
    }
}

/// The labels of what a bibite's field of view sees, in the order the game makes them (bibites, plants, meat; each from its last to its first).
#[must_use]
pub fn labels(targets: &FovTargets) -> Vec<Label> {
    struct Entry {
        key: u64,
        kind: TargetKind,
        target: Vec2,
        scale: f32,
        alpha: f32,
        visible: bool,
        rows: Vec<Row>,
    }
    let mut entries: Vec<Entry> = Vec::new();
    for kind in [TargetKind::Bibite, TargetKind::Plant] {
        let (list, sum, max) = targets.channel(kind);
        if max <= 0.0 {
            continue;
        }
        for seen in list.iter().rev() {
            let ratio = seen.weight / max;
            entries.push(Entry {
                key: seen.key,
                kind,
                target: seen.pos,
                scale: seen.scale,
                alpha: ratio.sqrt(),
                visible: ratio > MIN_VISIBLE,
                rows: vec![Row {
                    kind,
                    weight: seen.weight,
                    share: seen.weight / sum,
                }],
            });
        }
    }
    let (meat, sum, max) = targets.channel(TargetKind::Meat);
    if max > 0.0 {
        for seen in meat.iter().rev() {
            let ratio = seen.weight / max;
            let row = Row {
                kind: TargetKind::Meat,
                weight: seen.weight,
                share: seen.weight / sum,
            };
            // A corpse is a bibite and meat (`UpdateCorpseWeight`): the label of the bibite gets a second row, and a corpse whose bibite is too light to
            // show is shown by its meat alone.
            match entries.iter_mut().find(|entry| entry.key == seen.key) {
                Some(entry) if entry.kind == TargetKind::Bibite => {
                    if entry.visible || ratio > MIN_VISIBLE {
                        if !entry.visible {
                            entry.visible = true;
                            entry.alpha = ratio.sqrt();
                        }
                        entry.rows.push(row);
                    }
                }
                Some(_) => {}
                None => entries.push(Entry {
                    key: seen.key,
                    kind: TargetKind::Meat,
                    target: seen.pos,
                    scale: seen.scale,
                    alpha: ratio.sqrt(),
                    visible: ratio > MIN_VISIBLE,
                    rows: vec![row],
                }),
            }
        }
    }
    entries
        .into_iter()
        .filter(|entry| entry.visible)
        .map(|entry| {
            // `WeightDisplay.SetOffset`: a bibite's or an egg's `scale x 2.5 x 5`, a pellet's `scale x 2 x 2.5`, at least 10.
            let reach = match entry.kind {
                TargetKind::Bibite => entry.scale * 2.5 * 5.0,
                TargetKind::Plant | TargetKind::Meat => entry.scale * 2.0 * 2.5,
            };
            Label {
                target: entry.target,
                offset: reach.max(10.0),
                alpha: entry.alpha,
                rows: entry.rows,
            }
        })
        .collect()
}

/// The sprite of a kind of target (`WeightDisplay.targetIndicator`).
fn icon_alias(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Bibite => "bibite",
        TargetKind::Plant => "medium_plant",
        TargetKind::Meat => "medium_meat",
    }
}

/// Draws the labels (under every panel: the caller puts them first in the frame).
pub fn draw(
    frame: &mut UiFrame,
    ui: &UiAssets,
    metrics: Option<&SdfFonts>,
    camera: &Camera,
    labels: &[Label],
) {
    let zoom = camera.zoom;
    let px = TEXT_SIZE * zoom;
    if !(px.is_finite() && px >= 4.0) {
        return;
    }
    // The widths and the height of the texts in world units (a text's preferred size; every row is as high as one line).
    let width_of = |text: &str| -> f32 {
        metrics.map_or(text.chars().count() as f32 * TEXT_SIZE * 0.55, |metrics| {
            metrics.measure_named(Some(FONT), FontRole::Bold, text, TEXT_SIZE)
        })
    };
    let height = metrics.map_or(TEXT_SIZE * 1.17, |metrics| {
        metrics.line_height_named(Some(FONT), FontRole::Bold, TEXT_SIZE)
    });
    for label in labels {
        let origin = label.origin();
        let tint = |a: f32| [255, 255, 255, (a.clamp(0.0, 1.0) * 255.0).round() as u8];
        let color = tint(label.alpha);
        for (index, row) in label.rows.iter().enumerate() {
            let weight = row.weight_text();
            let share = row.share_text();
            let (open, close) = ("(", ")");
            let widths = [
                width_of(&weight),
                width_of(open),
                width_of(&share),
                width_of(close),
            ];
            let total = ICON + SPACING + widths[0] + SPACING + widths[1] + widths[2] + widths[3];
            let left = origin.x - total / 2.0;
            let bottom = origin.y + ROW_STEP * index as f32;
            let top = bottom + height;
            let corner = |x: f32, y: f32| camera.world_to_screen(Vec2::new(x, y));
            // The underline: as wide as the row, centred on its bottom edge.
            frame.rect(
                corner(left, bottom + LINE / 2.0),
                Vec2::new(total * zoom, LINE * zoom),
                color,
            );
            // The icon, in the middle of the row's height.
            if let Some(texture) = ui.texture_of_alias(icon_alias(row.kind)) {
                frame.draws.push(UiDraw::Image {
                    texture,
                    pos: corner(left, bottom + (height + ICON) / 2.0),
                    size: Vec2::new(ICON * zoom, ICON * zoom),
                    tint: color,
                });
            }
            // The weight, then the share in brackets.
            let mut x = left + ICON + SPACING;
            for (text, width, gap) in [
                (weight.as_str(), widths[0], SPACING),
                (open, widths[1], 0.0),
                (share.as_str(), widths[2], 0.0),
                (close, widths[3], 0.0),
            ] {
                frame.draws.push(UiDraw::Text {
                    font: Some(FONT.to_owned()),
                    text: text.to_owned(),
                    pos: corner(x, top),
                    px,
                    color,
                    role: FontRole::Bold,
                });
                x += width + gap;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::senses::SeenTarget;

    fn seen(key: u64, x: f32, scale: f32, weight: f32) -> SeenTarget {
        SeenTarget {
            key,
            pos: Vec2::new(x, 0.0),
            scale,
            weight,
        }
    }

    #[test]
    fn a_label_shows_above_a_twentieth_of_the_heaviest_and_fades_with_its_square_root() {
        let targets = FovTargets {
            plants: vec![
                seen(1, 0.0, 1.0, 4.0),
                seen(2, 10.0, 1.0, 1.0),
                seen(3, 20.0, 1.0, 0.1),
            ],
            ..FovTargets::default()
        };
        let labels = labels(&targets);
        // 0.1 / 4 = 0.025 is under 0.05: no label; the others are from the last seen to the first, as the game's loop is.
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].target, Vec2::new(10.0, 0.0));
        assert!((labels[0].alpha - 0.5).abs() < 1e-6, "sqrt(1 / 4)");
        assert_eq!(labels[1].target, Vec2::ZERO);
        assert_eq!(labels[1].alpha, 1.0);
        // The share of the channel's weights (4 + 1 + 0.1) and the formats of the two numbers.
        let heavy = labels[1].rows[0];
        assert!((heavy.share - 4.0 / 5.1).abs() < 1e-6);
        assert_eq!(heavy.weight_text(), "4.00");
        assert_eq!(heavy.share_text(), "78.4%");
    }

    #[test]
    fn a_label_hangs_up_left_of_its_object_by_the_size_of_the_object() {
        let targets = FovTargets {
            bibites: vec![seen(1, 100.0, 1.6, 1.0)],
            plants: vec![seen(2, 50.0, 1.0, 1.0), seen(3, 60.0, 3.0, 1.0)],
            ..FovTargets::default()
        };
        let labels = labels(&targets);
        let offset = |x: f32| {
            labels
                .iter()
                .find(|label| label.target.x == x)
                .expect("label")
                .offset
        };
        // A bibite: scale x 2.5 x 5 = 20; a small pellet: scale x 5 under 10 -> 10; a big one: 15.
        assert!((offset(100.0) - 20.0).abs() < 1e-5);
        assert_eq!(offset(50.0), 10.0);
        assert!((offset(60.0) - 15.0).abs() < 1e-5);
        let label = labels
            .iter()
            .find(|label| label.target.x == 100.0)
            .expect("label");
        assert_eq!(label.origin(), Vec2::new(80.0, 20.0));
        // The pointer runs from the label to the object, white at the label's alpha, 0.5 wide.
        match label.pointer() {
            Primitive::Line {
                a,
                b,
                thickness,
                color,
            } => {
                assert_eq!((a, b), (Vec2::new(80.0, 20.0), Vec2::new(100.0, 0.0)));
                assert_eq!(thickness, 0.5);
                assert_eq!(color, [255, 255, 255, 255]);
            }
            other => panic!("not a line: {other:?}"),
        }
    }

    #[test]
    fn a_corpse_is_one_label_with_a_row_for_the_bibite_and_one_for_its_meat() {
        let targets = FovTargets {
            bibites: vec![seen(7, 0.0, 1.0, 2.0), seen(8, 30.0, 1.0, 1.0)],
            meat: vec![seen(7, 0.0, 1.0, 3.0), seen(9, 60.0, 1.0, 0.1)],
            ..FovTargets::default()
        };
        let labels = labels(&targets);
        let corpse = labels
            .iter()
            .find(|label| label.target.x == 0.0)
            .expect("the corpse");
        assert_eq!(corpse.rows.len(), 2);
        assert_eq!(corpse.rows[0].kind, TargetKind::Bibite);
        assert_eq!(corpse.rows[1].kind, TargetKind::Meat);
        // The bibite's own label fades with its weight over the bibites' heaviest (2 of 2), not the meat's.
        assert_eq!(corpse.alpha, 1.0);
        // A live bibite has one row; a pellet of meat too light for a label has none.
        assert_eq!(labels.len(), 2);
        assert!(labels.iter().all(|label| label.target.x != 60.0));
        // A corpse whose bibite row is too light to show is shown by its meat.
        let light = FovTargets {
            bibites: vec![seen(1, 0.0, 1.0, 10.0), seen(2, 10.0, 1.0, 0.1)],
            meat: vec![seen(2, 10.0, 1.0, 5.0)],
            ..FovTargets::default()
        };
        let shown = super::labels(&light);
        let corpse = shown
            .iter()
            .find(|label| label.target.x == 10.0)
            .expect("shown by its meat");
        assert_eq!(corpse.alpha, 1.0);
        assert_eq!(corpse.rows.len(), 2);
    }

    #[test]
    fn a_label_is_an_underline_an_icon_and_the_two_numbers_in_a_row() {
        let ui = UiAssets::empty();
        let camera = Camera::new(Vec2::ZERO, 10.0, Vec2::new(1920.0, 1080.0));
        let label = Label {
            target: Vec2::new(0.0, 0.0),
            offset: 10.0,
            alpha: 1.0,
            rows: vec![Row {
                kind: TargetKind::Plant,
                weight: 0.35,
                share: 0.089,
            }],
        };
        let mut frame = UiFrame::new(1.0);
        draw(&mut frame, &ui, None, &camera, &[label]);
        let texts: Vec<&str> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, ["0.35", "(", "8.9%", ")"]);
        // The texts are 3 world units high: 30 px at this zoom, one after the other left to right.
        let mut last = f32::MIN;
        for draw in &frame.draws {
            if let UiDraw::Text { px, pos, .. } = draw {
                assert!((px - 30.0).abs() < 1e-4);
                assert!(pos.x > last);
                last = pos.x;
            }
        }
        // The underline is one rectangle as wide as the row, 0.5 world units (5 px) high.
        let underline = frame
            .draws
            .iter()
            .find_map(|draw| match draw {
                UiDraw::Rect { size, .. } => Some(*size),
                _ => None,
            })
            .expect("underline");
        assert!((underline.y - 5.0).abs() < 1e-4 && underline.x > 50.0);
        // Too far out to read: nothing is drawn under 4 px.
        let far = Camera::new(Vec2::ZERO, 1.0, Vec2::new(1920.0, 1080.0));
        let mut frame = UiFrame::new(1.0);
        draw(&mut frame, &ui, None, &far, &[Label {
            target: Vec2::ZERO,
            offset: 10.0,
            alpha: 1.0,
            rows: Vec::new(),
        }]);
        assert!(frame.draws.is_empty());
    }
}
