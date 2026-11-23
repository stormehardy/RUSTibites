//! The Gene Analysis page of the Graphs panel (`GraphsPanel.genesGraphPage`: `BarGraph`, `GraphDataBar`, the gene dropdown and the
//! `LogLikeTimeSpanSlider`): how one gene was distributed over the bibites and the eggs, now or at an hour the genes stream logged.
//!
//! Prefab values (level2, `StatsPanel/GenesPage`): a 1820 x 854.68 grey (0.33) graph body, 70.16 under the page's top (which is 60 down),
//! its plot (`Bars`, the `graphRect`) 1745 x 778.68 at 50 from the body's left, 25 + 1 from its top, and 26 x 11 grid lines like the
//! historic graph's; the 20 bars sit in a `HorizontalLayoutGroup` (padding 1, lower left aligned) between two flexible spacers that put
//! them where their values belong in the gene's whole range; the dropdown is 250 x 35 (caption 18.75, items 22 high, label 12.8) beside
//! the 30 pt "Gene Distribution:"; the time slider (a 1440 x 30 panel with a 1420 x 20 slider, whole numbers, right to left) sits at the
//! bottom with "Time:" before it and "Now" / "3h ago" after it.
//!
//! The "Now" histogram is made when the page is drawn: the game is paused while the panel is open (`TogglePauseGame("GraphsPanel")`), so
//! it is the same as the game's, which makes it when the slider, the gene or the panel's size change.

use bibites_core::format::FloatValueFormat;
use bibites_core::genes_log::{GenesBucketPoint, N_BUCKETS, SERIAL_HEAVY};
use bibites_core::genome::{GENE_DEFS, N_GENES};
use bibites_core::math::Vec2;

use crate::app::App;
use crate::gene_info_data::{GENES, GeneInfo};
use crate::species_panel::position::{Drag, clamp_value, handle_x, value_at};
use crate::time_format::{TimeFormat, Timescale};
use crate::ui::{DropdownTarget, Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiId};

/// `genesDropdown.value = 16` when the panel starts: the 17th gene of the enum, Diet.
pub const DEFAULT_GENE: usize = 16;

/// The graph body (`GraphBody`): x, y, width, height in canvas units.
const BODY: [f32; 4] = [50.0, 130.16, 1820.0, 854.68];
/// The plot (`Bars`, `BarGraph.graphRect`).
const PLOT: [f32; 4] = [100.0, 156.16, 1745.0, 778.68];
/// `BaseGraph.nX` and `nY`: the grid lines.
const N_X: usize = 26;
const N_Y: usize = 11;
/// The bars' `Holder` pads each side by this.
const HOLDER_PADDING: f32 = 1.0;
/// The slider (`Slider`, inside the `SliderPanel`, whose image is off).
const SLIDER: [f32; 4] = [250.0, 1002.42, 1420.0, 20.0];

/// Hit ids of the page.
const HIT_DROPDOWN: UiId = 8210;
const HIT_HANDLE: UiId = 8212;
const HIT_TRACK: UiId = 8213;

/// The game's setting of gene `gene` (`BibiteEditorSettings.SettingOfGene`): its name, help text and number format.
#[must_use]
pub fn gene_info(gene: usize) -> &'static GeneInfo {
    let name = GENE_DEFS[gene].name;
    GENES
        .iter()
        .find(|info| info.gene == name)
        .expect("every gene has a setting")
}

/// `FloatSetting.formatting`: how the setting writes a value of the gene.
#[must_use]
pub fn gene_format(info: &GeneInfo) -> FloatValueFormat {
    FloatValueFormat {
        prefix: info.prefix,
        units: info.units,
        precision: info.precision,
        factor: info.factor,
        si: info.si,
        always_show_sign: info.always_show_sign,
        precision_is_si: false,
        is_int: false,
    }
}

/// The y axis' labels (the `yLine` prefab's `FloatValueTextHandle`): whole numbers, SI prefixes.
const COUNT_FORMAT: FloatValueFormat = FloatValueFormat {
    prefix: "",
    units: "",
    precision: 0,
    factor: 1.0,
    si: true,
    always_show_sign: false,
    precision_is_si: false,
    is_int: true,
};

/// One bar of the histogram, in canvas units from the plot's left edge and the bottom of its holder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// Left edge of the bar's slot.
    pub x: f32,
    /// Width of the slot (the drawn bar is 2 narrower).
    pub width: f32,
    /// Height of the bar.
    pub height: f32,
    /// How many bibites it counts.
    pub count: u16,
    /// The values it covers.
    pub range: (f32, f32),
}

/// What `BarGraph.UpdateGraphData` makes of one point.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    /// The value of the top grid line: the largest count rounded up to a decade's multiple.
    pub top: u32,
    /// All the bibites counted.
    pub total: u32,
    /// The 20 bars.
    pub bars: Vec<Bar>,
}

/// The y axis' top (`BarGraph.UpdateGraphData`): the largest count, at least 1, rounded up to a multiple of its decade (at least 10).
#[must_use]
pub fn axis_top(largest: u16) -> u32 {
    let count = if largest == 0 { 1 } else { u32::from(largest) };
    let exponent = (count as f32).log10().floor().max(1.0);
    let decade = 10.0_f32.powf(exponent);
    (decade as u32) * (count as f32 / decade).ceil() as u32
}

/// `BarGraph.UpdateGraphData`: the bars of `point`, laid out the way the layout groups do. The point's buckets are `bucket_width`
/// wide and start at its `min`; the left and right spacers (flexible widths) fill the rest of the gene's range, so a bar is where its
/// values are on the axis.
#[must_use]
pub fn histogram(point: &GenesBucketPoint, gene: usize) -> Histogram {
    let def = &GENE_DEFS[gene];
    let width = point.bucket_width(gene);
    // A negative flexible width is no flexible width (`LayoutElement`: below 0 is unset).
    let left = ((point.min - def.min) / width).max(0.0);
    let right = ((def.max - (point.min + width * N_BUCKETS as f32)) / width).max(0.0);
    let unit = (PLOT[2] - 2.0 * HOLDER_PADDING) / (left + N_BUCKETS as f32 + right);
    let largest = point.data.iter().copied().max().unwrap_or(0);
    let top = axis_top(largest);
    let total = point.data.iter().map(|count| u32::from(*count)).sum();
    // The holder is 2 shorter than the plot, and a bar never gets taller than it (`controlChildHeight`).
    let tallest = PLOT[3] - 2.0 * HOLDER_PADDING;
    let bars = (0..N_BUCKETS)
        .map(|bucket| {
            let count = point.data[bucket];
            Bar {
                x: HOLDER_PADDING + unit * (left + bucket as f32),
                width: unit,
                height: (f32::from(count) / top as f32 * PLOT[3]).min(tallest),
                count,
                range: (
                    point.min + width * bucket as f32,
                    point.min + width * (bucket + 1) as f32,
                ),
            }
        })
        .collect();
    Histogram { top, total, bars }
}

/// `LogLikeTimeSpanSlider.UpdateTime`'s text: "Now", or how long ago the point was logged.
#[must_use]
pub fn time_text(value: i32) -> String {
    if value < 0 {
        return "Now".to_owned();
    }
    let time = TimeFormat::of_serial_point(&SERIAL_HEAVY, 1, Timescale::Hours, value);
    format!(
        "{} ago",
        time.formatted(1, "", true, false, Timescale::Hours)
    )
}

/// The histogram the page shows: the population as it is, or a point of the genes stream.
fn shown_point(app: &App, gene: usize) -> GenesBucketPoint {
    let time = app.graphs_gene_time_value();
    if time < 0 {
        app.world.current_gene_point(gene)
    } else {
        app.world
            .data_logger
            .genes
            .get(time as usize)
            .map_or(GenesBucketPoint::EMPTY, |buckets| buckets[gene])
    }
}

/// The text a bar's tooltip shows (`GraphDataBar.UpdateValue`): the values it covers, then its count and share.
#[must_use]
pub fn bar_tip(bar: &Bar, total: u32, format: &FloatValueFormat) -> (String, String) {
    let share = f32::from(bar.count) / total as f32;
    (
        format!(
            "[{} : {}]",
            format.format(bar.range.0),
            format.format(bar.range.1)
        ),
        format!("count: {} ({:.1}%)", bar.count, share * 100.0),
    )
}

impl App {
    /// The time slider's range (`SetMinMax(-1, loggedGenes - 1)`): "Now", then the points of the genes stream.
    #[must_use]
    pub fn graphs_gene_range(&self) -> (i32, i32) {
        (-1, self.world.data_logger.genes.n_data() as i32 - 1)
    }

    /// The slider's value, inside its range (setting the range clamps the value, `Slider.minValue`).
    #[must_use]
    pub fn graphs_gene_time_value(&self) -> i32 {
        let (min, max) = self.graphs_gene_range();
        clamp_value(self.graphs_gene_time, min, max)
    }

    /// The slider moved to `value` (`LogLikeTimeSpanSlider.UpdateTime` -> `BarGraph.UpdateTime`).
    pub fn graphs_gene_set_time(&mut self, value: i32) {
        let (min, max) = self.graphs_gene_range();
        self.graphs_gene_time = clamp_value(value, min, max);
    }

    /// The slider was pressed: on its handle (the grab offset keeps the value where it is) or on its track (the value jumps to the
    /// cursor, `OnPointerDown`).
    pub fn graphs_gene_press(&mut self, on_handle: bool, track: (f32, f32), cursor_x: f32) {
        let (min, max) = self.graphs_gene_range();
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let grab = if on_handle {
            let slider = [track.0, 0.0, track.1 - track.0, 0.0];
            cursor_x / scale - handle_x(slider, min, max, self.graphs_gene_time_value())
        } else {
            0.0
        };
        self.graphs_gene_drag = Some(Drag {
            x0: track.0,
            x1: track.1,
            grab,
        });
        self.graphs_gene_slide(cursor_x);
    }

    /// The held slider follows the cursor (`Slider.UpdateDrag`).
    pub fn graphs_gene_slide(&mut self, cursor_x: f32) {
        let Some(drag) = self.graphs_gene_drag else {
            return;
        };
        let (min, max) = self.graphs_gene_range();
        let scale = UiFrame::scaler(self.camera.screen.x, self.camera.screen.y);
        let value = value_at(drag.x0, drag.x1, min, max, cursor_x / scale, drag.grab);
        self.graphs_gene_set_time(value);
    }
}

/// Draws the page below the Graphs panel's top bar (`s`: screen pixels per canvas unit). Every rectangle and font is from a live dump of
/// `StatsPanel/GenesPage` at 1920 x 1080: the labels are `Roboto-Bold SDF NoOutline`, the grid is the historic graph's (2 across: (10, 10, 10) rows
/// 77.87 apart from the plot's top, black columns 69.72 apart from x 101, from y 181 for 781), the slider's track is a `Background` sprite
/// (alpha 0.255, ppu 200) with a `base_circle_icon` handle.
pub fn draw(
    frame: &mut UiFrame,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    app: &App,
    s: f32,
) {
    use crate::hud::{icon_image, sliced_image, tmp_label_aligned};
    const ROBOTO: &str = "Roboto-Bold SDF NoOutline";
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let sz = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let white: Rgba = [255, 255, 255, 255];
    let label_ink: Rgba = [38, 38, 38, 255];
    let label = |frame: &mut UiFrame, text: &str, px: f32, color: Rgba, rect: [f32; 4], align: f32| {
        tmp_label_aligned(frame, metrics, s, ROBOTO, text, px, color, rect, align);
    };
    let gene = app.graphs_gene.min(N_GENES - 1);
    let def = &GENE_DEFS[gene];
    let info = gene_info(gene);
    let format = gene_format(info);

    // The dropdown (`GraphsPanel.genesDropdown`): "Gene Distribution:" (Roboto-Bold 30, 10 of margin), then the 250 x 35 box (a `UISprite`, ppu 200)
    // with the gene's name (18.75) and the arrow in its right 40, which stands 2.5 above and below it.
    label(frame, "Gene Distribution:", 30.0, white, [60.0, 85.0, 290.0, 35.2], 0.0);
    let boxed = [365.0, 85.1, 250.0, 35.0];
    sliced_image(frame, ui, s, "UISprite", 10.0, 0.5, boxed, white);
    label(frame, info.name, 18.75, label_ink, [375.0, 92.1, 215.0, 22.0], 0.0);
    icon_image(frame, ui, s, "DropdownArrow", [575.0, 82.6, 40.0, 40.0]);
    let (box_pos, box_size) = (at(boxed[0], boxed[1]), sz(boxed[2], boxed[3]));
    frame.hit(
        HIT_DROPDOWN,
        box_pos,
        box_size,
        UiAction::DropdownOpen(DropdownTarget::GraphsGene),
    );

    // The graph body with its grid.
    frame.rect(
        at(BODY[0], BODY[1]),
        sz(BODY[2], BODY[3]),
        [84, 84, 84, 255],
    );
    let plot = at(PLOT[0], PLOT[1]);
    let histogram = histogram(&shown_point(app, gene), gene);
    let line_w = 1.98;
    for k in 0..N_Y {
        // From the top: `yLines` (11 of them) are spread over the plot's height.
        let c = PLOT[1] + PLOT[3] * k as f32 / (N_Y - 1) as f32;
        frame.rect(at(101.0, c - line_w / 2.0), sz(1745.0, line_w), [10, 10, 10, 255]);
        // `yLines[k].UpdateValue(k / (nY - 1) * top)`, whole numbers; the label is centred in the 50 wide axis (Roboto-Bold 11.05, white).
        let along = 1.0 - k as f32 / (N_Y - 1) as f32;
        label(
            frame,
            &COUNT_FORMAT.format(along * histogram.top as f32),
            11.05,
            white,
            [50.0, c - 6.5, 50.0, 13.0],
            0.5,
        );
    }
    for k in 0..N_X {
        let c = 101.0 + 69.72 * k as f32;
        frame.rect(at(c - line_w / 2.0, 181.0), sz(line_w, 781.0), [0, 0, 0, 255]);
        // `xLines[k].UpdateValue(minValue + span * k / (nX - 1))`, in the gene's own format (Roboto-Bold 12 in a 50 x 15 box under the plot).
        label(
            frame,
            &format.format(def.min + def.span() * k as f32 / (N_X - 1) as f32),
            12.0,
            white,
            [c - 25.0, 942.3, 50.0, 15.0],
            0.5,
        );
    }

    // The bars, white, standing on the bottom of the holder (1 above the plot's bottom); the one under the pointer has its tooltip.
    let bottom = plot.y + (PLOT[3] - HOLDER_PADDING) * s;
    for bar in &histogram.bars {
        if bar.height <= 0.0 {
            continue;
        }
        let slot = Vec2::new(plot.x + bar.x * s, bottom - bar.height * s);
        let slot_size = Vec2::new(bar.width * s, bar.height * s);
        frame.rect(
            Vec2::new(slot.x + s, slot.y),
            Vec2::new((slot_size.x - 2.0 * s).max(1.0), slot_size.y),
            white,
        );
        let inside = app.cursor.x >= slot.x
            && app.cursor.x < slot.x + slot_size.x
            && app.cursor.y >= slot.y
            && app.cursor.y < slot.y + slot_size.y;
        if inside && app.dropdown_open.is_none() {
            let (title, content) = bar_tip(bar, histogram.total, &format);
            frame.tips.push(crate::tooltip::TipRect {
                pos: slot,
                size: slot_size,
                tip: crate::tooltip::Tip {
                    path: "",
                    title: crate::tooltip::intern(title),
                    content: crate::tooltip::intern(content),
                    ui: true,
                    instant: false,
                },
            });
        }
    }

    // The time slider (`GenesPage/TimeSlider`): "Time:" (Roboto-Bold 30, 10 of margin), the slider (its panel's image is off), and how long ago the
    // point was logged (Roboto-Bold 24, right aligned, 10 of margin).
    label(frame, "Time:", 30.0, white, [60.0, 994.8, 165.0, 35.2], 0.0);
    // The track and the fill's area reach 5 to the left of the slider.
    sliced_image(
        frame,
        ui,
        s,
        "Background",
        10.0,
        0.5,
        [SLIDER[0] - 5.0, SLIDER[1], SLIDER[2] + 5.0, SLIDER[3]],
        [255, 255, 255, 65],
    );
    let (min, max) = app.graphs_gene_range();
    let value = app.graphs_gene_time_value();
    let centre = handle_x(SLIDER, min, max, value);
    if ui.texture_of_alias("base_circle_icon").is_some() {
        icon_image(frame, ui, s, "base_circle_icon", [centre - 10.0, SLIDER[1], 20.0, 20.0]);
    } else {
        let diameter = (20.0 * s).round().max(2.0) as u32;
        frame.draws.push(UiDraw::Pixels {
            texture: std::sync::Arc::new(crate::hud::disc_texture(diameter, white)),
            pos: Vec2::new(centre * s - diameter as f32 / 2.0, SLIDER[1] * s),
            size: Vec2::splat(diameter as f32),
        });
    }
    let ends = (
        SLIDER[0].round() as u16,
        (SLIDER[0] + SLIDER[2]).round() as u16,
    );
    // The handle sticks out of the track at the ends, so it has its own hit, on top.
    frame.hit(
        HIT_HANDLE,
        at(centre - 10.0, SLIDER[1]),
        sz(20.0, SLIDER[3]),
        UiAction::GenesTimeSlideHandle(ends.0, ends.1),
    );
    frame.hit(
        HIT_TRACK,
        at(SLIDER[0] - 5.0, SLIDER[1]),
        sz(SLIDER[2] + 5.0, SLIDER[3]),
        UiAction::GenesTimeSlide(ends.0, ends.1),
    );
    label(frame, &time_text(value), 24.0, white, [1705.0, 998.4, 165.0, 28.1], 1.0);
    // RUSTibites Improved: the time tags on the slider (the chips in the band between the plot's x labels and the slider).
    crate::time_tags_ui::draw_genes_tags(frame, &crate::time_tags_ui::Surface { ui, metrics, s }, app, SLIDER, (min, max), 961.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_axis_tops_at_a_multiple_of_the_counts_decade() {
        // An empty histogram counts as 1; below 100 the decade is 10, as the game's `Max(1, floor(log10))` says.
        assert_eq!(axis_top(0), 10);
        assert_eq!(axis_top(1), 10);
        assert_eq!(axis_top(9), 10);
        assert_eq!(axis_top(10), 10);
        assert_eq!(axis_top(22), 30);
        assert_eq!(axis_top(45), 50);
        assert_eq!(axis_top(51), 60);
        assert_eq!(axis_top(99), 100);
        assert_eq!(axis_top(100), 100);
        assert_eq!(axis_top(129), 200);
        assert_eq!(axis_top(1000), 1000);
        assert_eq!(axis_top(1001), 2000);
    }

    #[test]
    fn the_bars_stand_where_their_values_are_on_the_genes_axis() {
        // The real save's newest lay time histogram: 45 bibites in the buckets 0, 3, 6 and 13 of 4 to 14 (0.5 each), of the gene's 1 to
        // 51: 6 buckets' room to the left, 74 to the right.
        let mut data = [0_u16; N_BUCKETS];
        data[0] = 7;
        data[3] = 1;
        data[6] = 22;
        data[13] = 15;
        let point = GenesBucketPoint {
            min: 4.0,
            max: 11.0,
            data,
        };
        let histogram = histogram(&point, 0);
        assert_eq!((histogram.top, histogram.total), (30, 45));
        let unit = 1743.0 / 100.0;
        for (bucket, bar) in histogram.bars.iter().enumerate() {
            assert!((bar.x - (1.0 + unit * (6.0 + bucket as f32))).abs() < 1e-3);
            assert!((bar.width - unit).abs() < 1e-3);
            assert_eq!(bar.count, data[bucket]);
        }
        assert_eq!(histogram.bars[0].range, (4.0, 4.5));
        assert_eq!(histogram.bars[19].range, (13.5, 14.0));
        // The highest bar is 22 of the axis' 30 high; the empty ones have no height.
        assert!((histogram.bars[6].height - 22.0 / 30.0 * 778.68).abs() < 1e-2);
        assert_eq!(histogram.bars[1].height, 0.0);
        // The bars end 74 buckets from the right: the last bar's right edge is where the value 14 is on the axis, (14 - 1) / 50 of
        // the way along the plot's inner width.
        let end = histogram.bars[19].x + histogram.bars[19].width;
        assert!((end - (1.0 + 1743.0 * 13.0 / 50.0)).abs() < 1e-2, "{end}");
        // A full-height bar stops at the holder's inner height.
        let mut full = [0_u16; N_BUCKETS];
        full[0] = 50;
        let point = GenesBucketPoint {
            min: 4.0,
            max: 11.0,
            data: full,
        };
        assert_eq!(histogram_of(&point).bars[0].height, 776.68);
    }

    fn histogram_of(point: &GenesBucketPoint) -> Histogram {
        histogram(point, 0)
    }

    #[test]
    fn a_range_that_fills_the_genes_whole_range_has_no_spacers() {
        // The merged points of the real save can span the whole range: the bars then fill the plot's inner width.
        let point = GenesBucketPoint {
            min: 1.0,
            max: 51.0,
            data: [1; N_BUCKETS],
        };
        let histogram = histogram(&point, 0);
        assert_eq!(histogram.total, 20);
        assert!((histogram.bars[0].x - 1.0).abs() < 1e-4);
        let end = histogram.bars[19].x + histogram.bars[19].width;
        assert!((end - 1744.0).abs() < 1e-2, "{end}");
        // No count at all: every bar is flat and the axis is 10.
        let flat = histogram_of(&GenesBucketPoint::EMPTY);
        assert_eq!(flat.top, 10);
        assert!(flat.bars.iter().all(|bar| bar.height == 0.0));
    }

    #[test]
    fn a_bars_tooltip_has_its_values_its_count_and_its_share() {
        let format = gene_format(gene_info(0));
        let bar = Bar {
            x: 0.0,
            width: 10.0,
            height: 10.0,
            count: 22,
            range: (7.0, 7.5),
        };
        let (title, content) = bar_tip(&bar, 45, &format);
        assert_eq!(title, "[7.0 s : 7.5 s]");
        assert_eq!(content, "count: 22 (48.9%)");
        // An empty population has no share to give.
        let (_, content) = bar_tip(&Bar { count: 0, ..bar }, 0, &format);
        assert_eq!(content, "count: 0 (NaN%)");
    }

    #[test]
    fn every_gene_has_a_setting_whose_range_is_the_cores() {
        for (index, def) in GENE_DEFS.iter().enumerate() {
            let info = gene_info(index);
            assert_eq!(info.gene, def.name);
            assert_eq!(info.min, def.min, "{}", def.name);
            assert_eq!(info.max, def.max, "{}", def.name);
        }
        assert_eq!(gene_info(DEFAULT_GENE).name, "Diet");
    }

    #[test]
    fn the_slider_says_how_long_ago_the_point_was_logged() {
        assert_eq!(time_text(-1), "Now");
        // The six newest points are 1 to 6 hours old; the next scale's points stand for 3 hours each.
        assert_eq!(time_text(0), "1h ago");
        assert_eq!(time_text(5), "6h ago");
        assert_eq!(time_text(6), "9h ago");
        assert_eq!(time_text(7), "12h ago");
    }
}
