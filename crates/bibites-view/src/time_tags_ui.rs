//! RUSTibites Improved: drawing the time tags (`time_tags.rs` is the model): each panel's two buttons and name field, a tag's label chip with its remove
//! button, and the tag lines of the data graphs. The lineage graph draws its own lines (`lineage_graph.rs`) with the same chip.
//!
//! Everything is in reference px (the 1920 x 1080 canvas) and scaled by `s` on its way into the frame, like the panels it sits in.

use bibites_core::genes_log::SERIAL_HEAVY;
use bibites_core::math::Vec2;

use crate::app::App;
use crate::species_panel::position::handle_x;
use crate::hud::{closest_index, icon_image, sliced_image, tmp_label_aligned};
use crate::text_sdf::SdfFonts;
use crate::time_format::{TimeFormat, Timescale};
use crate::time_tags::{self, TagPanel, TimeTag};
use crate::tooltip::{self, Tip, TipRect};
use crate::ui::{FontRole, Rgba, UiAction, UiAssets, UiFrame};

/// The prefab's label font of the Stats and Analysis panel (the same in every panel the tags show in).
pub const FONT: &str = "Roboto-Bold SDF NoOutline";
const INK: Rgba = [38, 38, 38, 255];
const WHITE: Rgba = [255, 255, 255, 255];
/// A label chip's body: light like the panels' buttons, so that the dark text and the game's dark cross read on the dark graphs and the navy lineage.
const CHIP_BG: Rgba = [236, 236, 236, 245];
/// The height of a label chip and of its text.
pub const CHIP_HEIGHT: f32 = 22.0;
const CHIP_PX: f32 = 15.0;
/// The width of a chip's colour bar and of its remove button.
const BAR: f32 = 5.0;
const REMOVE: f32 = 22.0;
/// The width of the name field.
const FIELD_WIDTH: f32 = 330.0;
/// What the empty name field says, the longest first (the first that fits the field is shown): the name is optional, and a click on the panel's graph sets
/// the tag at the moment clicked (the button sets it now).
pub const NAME_HINTS: [&str; 3] = ["Name, then click a time", "Name, then click", "Name (optional)"];
const FIELD_PX: f32 = 18.0;

/// The width of `text` in reference px at size `px`.
#[must_use]
pub fn width_of(metrics: Option<&SdfFonts>, text: &str, px: f32) -> f32 {
    metrics.map_or(text.chars().count() as f32 * px * 0.55, |fonts| {
        fonts.measure_named(Some(FONT), FontRole::Body, text, px)
    })
}

/// Drawing context of one panel: the sprites, the text metrics and the scale of the canvas.
#[derive(Clone, Copy)]
pub struct Surface<'a> {
    /// The UI sprites.
    pub ui: &'a UiAssets,
    /// The text metrics (none: an estimate).
    pub metrics: Option<&'a SdfFonts>,
    /// Screen px per reference px.
    pub s: f32,
}

impl Surface<'_> {
    fn at(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(x * self.s, y * self.s)
    }

    fn size(&self, w: f32, h: f32) -> Vec2 {
        Vec2::new(w * self.s, h * self.s)
    }

    fn label(&self, frame: &mut UiFrame, text: &str, px: f32, color: Rgba, rect: [f32; 4], align: f32) {
        tmp_label_aligned(frame, self.metrics, self.s, FONT, text, px, color, rect, align);
    }

    /// A button in the look of the panels' page buttons (`base_icon`, the label in Roboto-Bold).
    fn button(&self, frame: &mut UiFrame, label: &str, rect: [f32; 4], px: f32, id: u32, action: UiAction) {
        sliced_image(frame, self.ui, self.s, "base_icon", 5.0, 1.0 / 0.75, rect, WHITE);
        self.label(frame, label, px, INK, rect, 0.5);
        frame.hit(id, self.at(rect[0], rect[1]), self.size(rect[2], rect[3]), action);
    }

    /// The name field: the sliced input background, the text (or what to type while it is empty), the selection and the blinking caret.
    fn field(&self, frame: &mut UiFrame, edit: &crate::app::TimeTagEdit, rect: [f32; 4], id: u32) {
        sliced_image(frame, self.ui, self.s, "InputFieldBackground", 8.0, 1.0, rect, WHITE);
        let text_x = rect[0] + 10.0;
        let before = |chars: usize| width_of(self.metrics, &edit.text.chars().take(chars).collect::<String>(), FIELD_PX);
        if let Some((from, to)) = edit.caret.selection() {
            frame.rect(
                self.at(text_x + before(from), rect[1] + 5.0),
                self.size(before(to) - before(from), rect[3] - 10.0),
                [168, 206, 255, 192],
            );
        }
        let (shown, ink) = if edit.text.is_empty() {
            // What to do next, as much of it as the field has room for.
            let room = rect[2] - 20.0;
            let hint = NAME_HINTS.into_iter().find(|hint| width_of(self.metrics, hint, FIELD_PX) <= room).unwrap_or("Name");
            (hint, [50, 50, 50, 110])
        } else {
            (edit.text.as_str(), [50, 50, 50, 255])
        };
        self.label(frame, shown, FIELD_PX, ink, [text_x, rect[1], rect[2] - 20.0, rect[3]], 0.0);
        if edit.caret_visible() {
            frame.rect(
                self.at(text_x + before(edit.caret.pos), rect[1] + 6.0),
                self.size(1.5, rect[3] - 12.0),
                [50, 50, 50, 255],
            );
        }
        // A press on the field keeps it open (anything else closes it).
        frame.hit(id, self.at(rect[0], rect[1]), self.size(rect[2], rect[3]), UiAction::TimeTagField);
    }

    /// The two controls of `panel` in the strip `bar` (reference px: its left, top, the right limit and the height): "Tag time" (which becomes "Tag now" with
    /// the name field next to it while the player types a name) and the show / hide button of the panel's own switch (the field takes its place while it is
    /// open). Nothing goes past the right limit: the name field gets what is left (at most its full width, at least a short one).
    pub fn controls(&self, frame: &mut UiFrame, app: &App, panel: TagPanel, bar: [f32; 4], px: f32) {
        let [left, top, max_right, height] = bar;
        let base = panel.hit_base();
        let editing = app.time_tag_edit.as_ref().filter(|edit| edit.panel == panel);
        let first = if editing.is_some() { "Tag now" } else { "Tag time" };
        let first_w = width_of(self.metrics, first, px) + 30.0;
        self.button(frame, first, [left, top, first_w, height], px, base, UiAction::TimeTagStart(panel));
        let next = left + first_w + 10.0;
        if let Some(edit) = editing {
            let width = (max_right - next).clamp(120.0, FIELD_WIDTH);
            self.field(frame, edit, [next, top, width, height], base + 2);
            return;
        }
        let shown = app.time_tags_shown(panel);
        let count = app.time_tags.len();
        let label = match (shown, count) {
            (true, 0) => "Hide tags".to_owned(),
            (true, n) => format!("Hide tags ({n})"),
            (false, 0) => "Show tags".to_owned(),
            (false, n) => format!("Show tags ({n})"),
        };
        let w = width_of(self.metrics, &label, px) + 30.0;
        self.button(frame, &label, [next, top, w, height], px, base + 1, UiAction::TimeTagsShow(panel));
    }

    /// The hint of the moment the pointer stands on while a tag is being placed (`text`, see [`pick_text`]): a label whose left is `left`, hung to the left of
    /// `left` instead when it would pass `right_limit`.
    pub fn readout(&self, frame: &mut UiFrame, text: &str, left: f32, top: f32, right_limit: f32) {
        let width = width_of(self.metrics, text, CHIP_PX) + 16.0;
        let left = if left + width > right_limit { (left - width - 16.0).max(0.0) } else { left };
        frame.rect(self.at(left, top), self.size(width, CHIP_HEIGHT), CHIP_BG);
        self.label(frame, text, CHIP_PX, INK, [left + 8.0, top, width - 16.0, CHIP_HEIGHT], 0.0);
    }

    /// A tag's label chip with its colour bar and remove button, its top left at `(left, top)`; the remove button is hit `remove_hit`. Returns the chip's
    /// rectangle.
    pub fn chip(&self, frame: &mut UiFrame, tag: &TimeTag, left: f32, top: f32, remove_hit: u32) -> [f32; 4] {
        let text_w = width_of(self.metrics, &tag.label, CHIP_PX);
        let width = BAR + 6.0 + text_w + 6.0 + REMOVE;
        let rect = [left, top, width, CHIP_HEIGHT];
        frame.rect(self.at(rect[0], rect[1]), self.size(rect[2], rect[3]), CHIP_BG);
        frame.rect(self.at(rect[0], rect[1]), self.size(BAR, CHIP_HEIGHT), tag.color());
        self.label(frame, &tag.label, CHIP_PX, INK, [left + BAR + 6.0, top, text_w + 4.0, CHIP_HEIGHT], 0.0);
        let remove = [left + width - REMOVE, top, REMOVE, CHIP_HEIGHT];
        if self.ui.texture_of_alias("exit_icon").is_some() {
            icon_image(frame, self.ui, self.s, "exit_icon", [remove[0] + 3.0, remove[1] + 3.0, REMOVE - 6.0, CHIP_HEIGHT - 6.0]);
        } else {
            self.label(frame, "x", CHIP_PX, INK, remove, 0.5);
        }
        frame.hit(remove_hit, self.at(remove[0], remove[1]), self.size(remove[2], remove[3]), UiAction::TimeTagRemove(tag.id));
        let when = TimeFormat::new(tag.seconds as f32, Timescale::Seconds).formatted(2, " ", false, true, Timescale::Minutes);
        frame.tips.push(TipRect {
            pos: self.at(rect[0], rect[1]),
            size: self.size(rect[2] - REMOVE, rect[3]),
            tip: Tip {
                path: "",
                title: "Time tag",
                content: tooltip::intern(format!("{}\nSet at {when} of world time. Press the cross to remove the tag.", tag.label)),
                ui: true,
                instant: false,
            },
        });
        rect
    }
}

/// The width a chip for `tag` takes (reference px).
#[must_use]
pub fn chip_width(metrics: Option<&SdfFonts>, tag: &TimeTag) -> f32 {
    BAR + 6.0 + width_of(metrics, &tag.label, CHIP_PX) + 6.0 + REMOVE
}

/// Puts spans (`start`, `end` along one axis, in the order given) into rows so that spans of one row do not overlap (`gap` apart): the row of each, at
/// most `rows` of them (past that the least full one is shared).
#[must_use]
pub fn assign_rows(spans: &[(f32, f32)], rows: usize, gap: f32) -> Vec<usize> {
    let mut ends = vec![f32::MIN; rows.max(1)];
    spans
        .iter()
        .map(|&(start, end)| {
            let row = ends.iter().position(|&last| last + gap <= start).unwrap_or_else(|| {
                ends.iter().enumerate().min_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(row, _)| row)
            });
            ends[row] = end;
            row
        })
        .collect()
}

/// Whether `panel`'s name field is open: its graph takes the click that sets the tag at the moment under the pointer.
#[must_use]
pub fn is_picking(app: &App, panel: TagPanel) -> bool {
    app.time_tag_edit.as_ref().is_some_and(|edit| edit.panel == panel)
}

/// What the pointer's moment says while a tag is being placed: how long before now it is ("Tag here: 12m 30s ago"), or that it is not in the world's time.
#[must_use]
pub fn pick_text(age_seconds: f64, now_seconds: f64) -> String {
    if age_seconds > now_seconds + 0.5 {
        return "Before the world began".to_owned();
    }
    if age_seconds < 1.0 {
        return "Tag here: now".to_owned();
    }
    let ago = TimeFormat::new(age_seconds as f32, Timescale::Seconds).formatted(1, " ", true, false, Timescale::Seconds);
    // "26m 0s" reads "26m": a smaller unit that is zero is left off.
    let mut parts: Vec<&str> = ago.split(' ').collect();
    while parts.len() > 1 && parts.last().is_some_and(|part| part.trim_end_matches(char::is_alphabetic) == "0") {
        parts.pop();
    }
    format!("Tag here: {} ago", parts.join(" "))
}

/// The data graphs while their name field is open: the plot (`plot`, reference px, `n_points` points over `span_minutes`, now at its right edge) takes the
/// click, and sets the tag at the logged point nearest to the pointer, the one the graph's own white hover line stands on. A label at the line tells how long
/// before now that is.
pub fn draw_graph_pick(frame: &mut UiFrame, surface: &Surface, app: &App, plot: [f32; 4], span_minutes: f64, n_points: usize) {
    if !is_picking(app, TagPanel::Graphs) || n_points < 2 || app.dropdown_open.is_some() {
        return;
    }
    let cursor = [app.cursor.x / surface.s, app.cursor.y / surface.s];
    let xs: Vec<f32> = (0..n_points).map(|k| plot[2] * k as f32 / (n_points - 1) as f32).collect();
    let k = closest_index(&xs, cursor[0] - plot[0]).min(n_points - 1);
    let now = app.simulated_time();
    let age = span_minutes * 60.0 * (1.0 - k as f64 / (n_points - 1) as f64);
    frame.hit(
        TagPanel::Graphs.hit_base() + 3,
        surface.at(plot[0], plot[1]),
        surface.size(plot[2], plot[3]),
        UiAction::TimeTagPlace(TagPanel::Graphs, now - age),
    );
    let over = cursor[0] >= plot[0] && cursor[0] < plot[0] + plot[2] && cursor[1] >= plot[1] && cursor[1] < plot[1] + plot[3];
    if over {
        let top = plot[1] + plot[3] - CHIP_HEIGHT - 6.0;
        surface.readout(frame, &pick_text(age, now), plot[0] + xs[k] + 10.0, top, plot[0] + plot[2]);
    }
}

/// The tag lines of the data graphs: a coloured line down the plot at each tag's time (the plot is `span_minutes` wide, now at its right edge) and its label
/// chip at the top. `plot` is the plot's rectangle in reference px (`x, y, w, h`). Nothing when the panel's switch is off.
pub fn draw_graph_tags(frame: &mut UiFrame, surface: &Surface, app: &App, plot: [f32; 4], span_minutes: f64) {
    if !app.time_tags_shown(TagPanel::Graphs) {
        return;
    }
    let now = app.simulated_time();
    let mut shown: Vec<(&TimeTag, f32)> = Vec::new();
    for tag in app.time_tags.up_to(now) {
        if let Some(fraction) = time_tags::graph_fraction(time_tags::minutes_ago(now, tag.seconds), span_minutes) {
            shown.push((tag, plot[0] + plot[2] * fraction));
        }
    }
    for (tag, x) in &shown {
        let mut color = tag.color();
        color[3] = 215;
        frame.rect(surface.at(x - 1.0, plot[1]), surface.size(2.0, plot[3]), color);
    }
    let spans: Vec<(f32, f32)> = shown.iter().map(|(tag, x)| (*x, x + chip_width(surface.metrics, tag))).collect();
    let rows = assign_rows(&spans, 4, 4.0);
    for (index, (tag, x)) in shown.iter().enumerate() {
        let width = chip_width(surface.metrics, tag);
        // A chip that would run off the plot's right edge hangs to the line's left instead.
        let left = if x + 2.0 + width > plot[0] + plot[2] { x - 2.0 - width } else { x + 2.0 };
        let top = plot[1] + 4.0 + rows[index] as f32 * (CHIP_HEIGHT + 4.0);
        surface.chip(frame, tag, left, top, TagPanel::Graphs.remove_base() + index as u32);
    }
}

/// Where a moment at `place` (`time_tags::serial_place`: `-1.0` now, whole numbers the genes log's points) stands along the gene analysis page's time
/// slider `slider` whose values run over `range` (right to left); `None` when it is not on the slider.
#[must_use]
pub fn slider_x(slider: [f32; 4], range: (i32, i32), place: f64) -> Option<f32> {
    let (min, max) = range;
    if max <= min || place < f64::from(min) || place > f64::from(max) {
        return None;
    }
    let low = place.floor() as i32;
    let high = (low + 1).min(max);
    let fraction = (place - f64::from(low)) as f32;
    let (a, b) = (handle_x(slider, min, max, low), handle_x(slider, min, max, high));
    Some(a + (b - a) * fraction)
}

/// The inverse of [`slider_x`]: the place on the genes log's point scale (`-1.0` is now) under `x` along the time `slider` whose values run over `range`
/// (right to left); the ends of the track hold for a pointer past them. `None` for a slider without a track.
#[must_use]
pub fn slider_place(slider: [f32; 4], range: (i32, i32), x: f32) -> Option<f64> {
    let (min, max) = range;
    if max <= min || slider[2] <= 0.0 {
        return None;
    }
    let along = f64::from(((x - slider[0]) / slider[2]).clamp(0.0, 1.0));
    Some(f64::from(min) + (1.0 - along) * f64::from(max - min))
}

/// The gene analysis page while its name field is open: the time slider's band takes the click and sets the tag at the moment of the slider under the
/// pointer (the log's points are an hour apart at first and further apart as they get older; a click between two is in proportion).
fn draw_genes_pick(frame: &mut UiFrame, surface: &Surface, app: &App, slider: [f32; 4], range: (i32, i32), chips_top: f32) {
    if !is_picking(app, TagPanel::Genes) {
        return;
    }
    let cursor = [app.cursor.x / surface.s, app.cursor.y / surface.s];
    let Some(place) = slider_place(slider, range, cursor[0]) else {
        return;
    };
    let Some(hours) = time_tags::serial_ago(&SERIAL_HEAVY, 1.0, place) else {
        return;
    };
    let now = app.simulated_time();
    let age = hours * 3600.0;
    let band = [slider[0] - 5.0, chips_top, slider[2] + 5.0, slider[1] + slider[3] - chips_top];
    frame.hit(
        TagPanel::Genes.hit_base() + 3,
        surface.at(band[0], band[1]),
        surface.size(band[2], band[3]),
        UiAction::TimeTagPlace(TagPanel::Genes, now - age),
    );
    let over = cursor[0] >= band[0] && cursor[0] < band[0] + band[2] && cursor[1] >= band[1] && cursor[1] < band[1] + band[3];
    if over && let Some(x) = slider_x(slider, range, place) {
        frame.rect(surface.at(x - 1.0, band[1]), surface.size(2.0, band[3]), WHITE);
        surface.readout(frame, &pick_text(age, now), x + 8.0, band[1] - CHIP_HEIGHT - 4.0, slider[0] + slider[2] + 40.0);
    }
}

/// The tag marks of the gene analysis page: a coloured line across the time slider at each tag's time and, in the band above it (`chips_top`), the label
/// chips of the tags that do not cover each other (a tag whose chip would is still marked by its line, which tells its name when the pointer rests on it).
pub fn draw_genes_tags(frame: &mut UiFrame, surface: &Surface, app: &App, slider: [f32; 4], range: (i32, i32), chips_top: f32) {
    draw_genes_pick(frame, surface, app, slider, range, chips_top);
    if !app.time_tags_shown(TagPanel::Genes) {
        return;
    }
    let now = app.simulated_time();
    let mut shown: Vec<(&TimeTag, f32)> = Vec::new();
    for tag in app.time_tags.up_to(now) {
        let hours = time_tags::minutes_ago(now, tag.seconds) / 60.0;
        if let Some(x) = slider_x(slider, range, time_tags::serial_place(&SERIAL_HEAVY, 1.0, hours)) {
            shown.push((tag, x));
        }
    }
    let bottom = slider[1] + slider[3];
    for (tag, x) in &shown {
        let mut color = tag.color();
        color[3] = 215;
        frame.rect(surface.at(x - 1.0, chips_top), surface.size(2.0, bottom - chips_top), color);
    }
    // Chips from the left: one that would cover the one before it is left out.
    let mut order: Vec<usize> = (0..shown.len()).collect();
    order.sort_by(|a, b| shown[*a].1.total_cmp(&shown[*b].1));
    let mut end = f32::MIN;
    for index in order {
        let (tag, x) = shown[index];
        let width = chip_width(surface.metrics, tag);
        let left = if x + 2.0 + width > slider[0] + slider[2] + 40.0 { x - 2.0 - width } else { x + 2.0 };
        if left < end + 4.0 {
            frame.tips.push(TipRect {
                pos: surface.at(x - 4.0, chips_top),
                size: surface.size(8.0, bottom - chips_top),
                tip: Tip {
                    path: "",
                    title: "Time tag",
                    content: tooltip::intern(tag.label.clone()),
                    ui: true,
                    instant: false,
                },
            });
            continue;
        }
        surface.chip(frame, tag, left, chips_top, TagPanel::Genes.remove_base() + index as u32);
        end = left + width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_on_the_gene_slider_is_between_the_points_and_now_is_at_the_right_end() {
        let slider = [250.0, 1002.0, 1000.0, 20.0];
        let range = (-1, 9);
        assert_eq!(slider_x(slider, range, -1.0), Some(1250.0), "now, at the right end of the track");
        assert_eq!(slider_x(slider, range, 9.0), Some(250.0), "the oldest point, at the left end");
        let (a, b) = (slider_x(slider, range, 3.0).unwrap(), slider_x(slider, range, 4.0).unwrap());
        let middle = slider_x(slider, range, 3.5).unwrap();
        assert!(a > b && (middle - (a + b) / 2.0).abs() < 1e-3);
        assert_eq!(slider_x(slider, range, 9.5), None, "older than the log's oldest point");
        assert_eq!(slider_x(slider, (-1, -1), -1.0), None, "a log without points has no slider");
        assert_eq!(slider_x(slider, range, f64::INFINITY), None);
    }

    #[test]
    fn a_pointer_on_the_gene_slider_is_a_place_between_its_points_the_other_way_round_from_slider_x() {
        let slider = [250.0, 1002.0, 1000.0, 20.0];
        let range = (-1, 9);
        assert_eq!(slider_place(slider, range, 1250.0), Some(-1.0), "the right end is now");
        assert_eq!(slider_place(slider, range, 250.0), Some(9.0), "the left end is the oldest point");
        assert_eq!(slider_place(slider, range, 5000.0), Some(-1.0), "past the track the end holds");
        assert_eq!(slider_place(slider, range, -50.0), Some(9.0));
        for place in [-1.0, -0.25, 0.0, 3.5, 8.75, 9.0] {
            let x = slider_x(slider, range, place).unwrap();
            assert!((slider_place(slider, range, x).unwrap() - place).abs() < 1e-3, "place {place} at {x}");
        }
        assert_eq!(slider_place(slider, (-1, -1), 600.0), None, "a log without points has no slider");
    }

    #[test]
    fn the_hint_of_a_moment_says_how_long_ago_it_was_and_that_a_time_before_the_world_is_none() {
        assert_eq!(pick_text(0.0, 5000.0), "Tag here: now");
        assert_eq!(pick_text(0.4, 5000.0), "Tag here: now");
        assert_eq!(pick_text(45.0, 5000.0), "Tag here: 45s ago");
        assert_eq!(pick_text(750.0, 5000.0), "Tag here: 12m 30s ago");
        assert_eq!(pick_text(1_560.0, 5000.0), "Tag here: 26m ago", "a zero smaller unit is left off");
        assert_eq!(pick_text(3_600.0, 5000.0), "Tag here: 1h ago");
        assert_eq!(pick_text(4_500.0, 9000.0), "Tag here: 1h 15m ago");
        assert_eq!(pick_text(6_000.0, 5000.0), "Before the world began");
    }

    #[test]
    fn spans_that_overlap_go_to_different_rows_and_the_row_is_free_again_after_its_span() {
        // Three chips side by side: the second starts before the first ends, the third after.
        let rows = assign_rows(&[(0.0, 100.0), (50.0, 150.0), (120.0, 200.0)], 3, 4.0);
        assert_eq!(rows, [0, 1, 0]);
        // More overlapping spans than rows: the rows are shared (the least full one), never past the last row.
        let crowded = assign_rows(&[(0.0, 100.0); 6], 2, 4.0);
        assert!(crowded.iter().all(|&row| row < 2));
        // A gap counts: touching spans share a row only when the gap fits.
        assert_eq!(assign_rows(&[(0.0, 100.0), (102.0, 150.0)], 2, 4.0), [0, 1]);
        assert_eq!(assign_rows(&[(0.0, 100.0), (104.0, 150.0)], 2, 4.0), [0, 0]);
    }
}
