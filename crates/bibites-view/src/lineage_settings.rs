//! The Lineage panel's Settings panel (`LineagePanelSettingsManager`, `SettingsGroupHandle`, `SettingToggle`,
//! `FloatSettingSlider`, `IntSettingSlider`): the gear button of the top bar opens it at the overlay's top-right corner.
//!
//! Prefab values (level2 `LineageSettingsPanel`, sharedassets0 `SettingGroupPrefab` / `SettingToggle` / `SettingSlider`): a
//! 745.5 x 740 `Long_UI` panel whose scroll view (vertical only, clamped, 35 a notch, the bar 20 wide and 25 away) holds a column
//! 700.5 wide with 20 / 20 / 20 / 35 padding and 20 between its children: the title (Roboto-Bold 36), "Effective Species Span:" with its
//! value, the "Lineage Options" group (Hide Extinct Lineages, Show Favorited Lineages, and in the tree view the "Lineage Tree"
//! group with Scale Important Species) and the "Species Options" group (the span, its scaling and the prune point limit as
//! sliders). A group is a 50-high title (Boston Black 36) and a holder (10 above, 20 below, 20 between the rows); a toggle row is
//! 40 high with the name flexible and a 350-wide toggle whose 40 x 40 box sits in its middle; a slider row is 40 high with the
//! name (125 + 2 shares), the value (80 + 1 share), the slider (200 + 1 share) and the 40 x 40 reset button, 15 apart. Unity's
//! layout groups share what is left of the row by those flexible weights, which is computed here.

use bibites_core::math::Vec2;

use crate::app::App;
use crate::lineage_graph::{clip_image, clip_nine_slice, clip_rect};
use crate::species_panel::Drawer;
use crate::tooltip::{self, Tip, TipRect};
use crate::ui::{FontRole, Rgba, UiAction, UiDraw, UiFrame};

/// `LineageSettingsPanel`: `x, y, w, h` (reference px), on the 1920 x 1080 canvas the prefab was captured on (see [`panel`]).
pub const PANEL: [f32; 4] = [1134.5, 50.0, 745.5, 740.0];

/// `LineageSettingsPanel` on a canvas `canvas_width` reference px wide: the prefab hangs it from the top right corner of the `Overlay` (anchors (1, 1), pivot
/// (1, 1), 40 in from the canvas' right edge), so it keeps to the right edge as the window gets wider or narrower.
#[must_use]
pub const fn panel(canvas_width: f32) -> [f32; 4] {
    [canvas_width - 40.0 - PANEL[2], PANEL[1], PANEL[2], PANEL[3]]
}
/// The viewport: the panel without the 45 the scroll bar and its distance take.
const VIEWPORT_W: f32 = 700.5;
/// The column's padding: left, right, top, bottom.
const PADDING: [f32; 4] = [20.0, 20.0, 20.0, 35.0];
/// The column's spacing.
const SPACING: f32 = 20.0;
/// The room a row has.
const INNER_W: f32 = VIEWPORT_W - PADDING[0] - PADDING[1];
/// The scroll bar's track (`Scrollbar Vertical`: 20 wide, 20 from the right edge, 40 shorter than the panel).
pub const BAR_TRACK: [f32; 4] = bar_track(PANEL);

/// The scroll bar's track of a panel placed at `panel` (see [`panel`]).
#[must_use]
pub const fn bar_track(panel: [f32; 4]) -> [f32; 4] {
    [panel[0] + panel[2] - 40.0, panel[1] + 20.0, 20.0, panel[3] - 40.0]
}
/// Hit ids of the panel (one for the panel, then one per control).
const HIT_PANEL: u32 = 210_000;

const WHITE: Rgba = [255, 255, 255, 255];
/// The default value pointer's ticks (0.961).
const TICK: Rgba = [245, 245, 245, 255];

/// The tree's three options (`LineagePanelSettingsManager`'s `BoolUserSetting`s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineageOption {
    /// `hideExtinctLineages`.
    HideExtinct,
    /// `showLineagesOfFavorite`.
    ShowFavorites,
    /// `scaleImportantSpecies`.
    ScaleSpecies,
}

impl LineageOption {
    /// The index the action carries.
    #[must_use]
    pub fn index(self) -> u8 {
        match self {
            Self::HideExtinct => 0,
            Self::ShowFavorites => 1,
            Self::ScaleSpecies => 2,
        }
    }

    /// The option an action's index names.
    #[must_use]
    pub fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::HideExtinct),
            1 => Some(Self::ShowFavorites),
            2 => Some(Self::ScaleSpecies),
            _ => None,
        }
    }

    /// The setting's name (`Name`; also the user setting's label).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::HideExtinct => "Hide Extinct Lineages",
            Self::ShowFavorites => "Show Favorited Lineages",
            Self::ScaleSpecies => "Scale Important Species",
        }
    }

    /// The setting's `HelperText`, the tooltip of its name.
    #[must_use]
    pub fn help(self) -> &'static str {
        match self {
            Self::HideExtinct => {
                "If enabled, the species of extinct lineages (with no alive descendants) won't be displayed."
            }
            Self::ShowFavorites => {
                "If enabled, the lineage of favorited species will always be displayed, no matter the other settings"
            }
            Self::ScaleSpecies => {
                "If enabled, the tree will emphasize (make bigger) species considered important (around for longer, more children, etc.) and scale down others"
            }
        }
    }
}

/// The value of one of the tree's options.
#[must_use]
pub fn option_value(options: &crate::lineage_tree::TreeSettings, option: LineageOption) -> bool {
    match option {
        LineageOption::HideExtinct => options.hide_extinct,
        LineageOption::ShowFavorites => options.show_favorites,
        LineageOption::ScaleSpecies => options.scale_species,
    }
}

/// The sliders of "Species Options" (`ScenarioIndependentSettings`).
pub const SLIDERS: [&str; 3] = [
    "speciesGeneticSpan",
    "speciesSpanScalingFactor",
    "speciesPrunePointLimit",
];

/// What a placed rectangle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// "Lineage Panel Settings".
    Title,
    /// "Effective Species Span: 1.599".
    Span,
    /// A group's title.
    GroupTitle(&'static str),
    /// A toggle row.
    Toggle(LineageOption),
    /// A slider row (an index of [`SLIDERS`]).
    Slider(usize),
}

/// A part and its rectangle in the column (`x, y, w, h`, y down from the column's top).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// What it is.
    pub part: Part,
    /// Where.
    pub rect: [f32; 4],
}

/// The column laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The parts, top to bottom.
    pub parts: Vec<Placed>,
    /// The column's height.
    pub height: f32,
}

impl Layout {
    /// How far the column can scroll (`ScrollRect` clamped).
    #[must_use]
    pub fn max_scroll(&self) -> f32 {
        (self.height - PANEL[3]).max(0.0)
    }
}

/// The column of the panel: the tree view has the "Lineage Tree" group, the mesh view does not
/// (`ShowSettingsOfType`).
#[must_use]
pub fn layout(tree_view: bool) -> Layout {
    let mut parts = Vec::new();
    let x = PADDING[0];
    let mut y = PADDING[2];
    let mut place = |part: Part, rect: [f32; 4]| parts.push(Placed { part, rect });
    // The title and the line under it keep to their own widths, centred.
    let title = [371.16, 42.19];
    place(
        Part::Title,
        [x + (INNER_W - title[0]) / 2.0, y, title[0], title[1]],
    );
    y += title[1] + SPACING;
    let span = [347.97, 29.3];
    place(
        Part::Span,
        [x + (INNER_W - span[0]) / 2.0, y, span[0], span[1]],
    );
    y += span[1] + SPACING;
    // "Lineage Options": the title, then the holder (10 above, 20 between and below).
    place(Part::GroupTitle("Lineage Options"), [x, y, INNER_W, 50.0]);
    y += 50.0 + 10.0 + 10.0;
    for option in [LineageOption::HideExtinct, LineageOption::ShowFavorites] {
        place(Part::Toggle(option), [x, y, INNER_W, 40.0]);
        y += 40.0 + 20.0;
    }
    if tree_view {
        place(Part::GroupTitle("Lineage Tree"), [x, y, INNER_W, 50.0]);
        y += 50.0 + 10.0 + 10.0;
        place(
            Part::Toggle(LineageOption::ScaleSpecies),
            [x, y, INNER_W, 40.0],
        );
        // The inner holder's bottom padding, then the outer one's.
        y += 40.0 + 20.0 + 20.0;
    }
    y += SPACING;
    place(Part::GroupTitle("Species Options"), [x, y, INNER_W, 50.0]);
    y += 50.0 + 10.0 + 10.0;
    for index in 0..SLIDERS.len() {
        place(Part::Slider(index), [x, y, INNER_W, 40.0]);
        y += 40.0 + 20.0;
    }
    Layout {
        parts,
        height: y + PADDING[3],
    }
}

/// The parts of a toggle row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToggleRow {
    /// The name.
    pub name: [f32; 4],
    /// The 40 x 40 box (in the middle of the 350-wide toggle).
    pub check: [f32; 4],
}

/// The parts of a toggle row: the name takes what the 350-wide toggle and the 15 before it leave.
#[must_use]
pub fn toggle_row(row: [f32; 4]) -> ToggleRow {
    let toggle_x = row[0] + row[2] - 350.0;
    ToggleRow {
        name: [row[0], row[1], row[2] - 350.0 - 15.0, row[3]],
        check: [toggle_x + 175.0 - 20.0, row[1], 40.0, 40.0],
    }
}

/// The parts of a slider row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderRow {
    /// The setting's name.
    pub name: [f32; 4],
    /// The value.
    pub value: [f32; 4],
    /// The slider's panel (its dim backing).
    pub panel: [f32; 4],
    /// The slider itself: the panel inset by 10 (the handle slides over its full width).
    pub slider: [f32; 4],
    /// The reset button.
    pub reset: [f32; 4],
}

/// Unity's horizontal layout of the row: the preferred widths (name 125, value 80, slider 200 + 15 + 40) and the 15s between
/// the three children are placed, and what is left goes to the children by their flexible weights 2, 1 and 1.
#[must_use]
pub fn slider_row(row: [f32; 4]) -> SliderRow {
    let extra = row[2] - (125.0 + 80.0 + 255.0 + 30.0);
    let name_w = 125.0 + extra * 2.0 / 4.0;
    let value_w = 80.0 + extra / 4.0;
    let panel_w = 200.0 + extra / 4.0;
    let value_x = row[0] + name_w + 15.0;
    let panel_x = value_x + value_w + 15.0;
    let reset_x = panel_x + panel_w + 15.0;
    SliderRow {
        name: [row[0], row[1], name_w, row[3]],
        value: [value_x, row[1], value_w, row[3]],
        panel: [panel_x, row[1], panel_w, row[3]],
        slider: [panel_x + 10.0, row[1] + 10.0, panel_w - 20.0, row[3] - 20.0],
        reset: [reset_x, row[1], 40.0, 40.0],
    }
}

impl Drawer<'_> {
    /// The Settings panel, when the gear button opened it.
    pub(crate) fn lineage_settings(&self, frame: &mut UiFrame, app: &App) {
        if !app.lineage_settings_open {
            return;
        }
        let s = self.s;
        let panel = panel(self.canvas[0]);
        // The panel takes the clicks over it.
        frame.hit(
            HIT_PANEL,
            self.at(panel[0], panel[1]),
            self.at(panel[2], panel[3]),
            UiAction::Consume,
        );
        self.slice(frame, "Long_UI", 70.0, 0.2, panel, WHITE);
        // The mesh view has no "Lineage Tree" group (`ShowSettingsOfType`).
        let column = layout(!app.lineage_view.mesh_mode);
        let options = app.lineage_tree_settings();
        let scroll = app.lineage_settings_scroll.clamp(0.0, column.max_scroll());
        // The viewport's mask.
        let clip = [panel[0] * s, panel[1] * s, VIEWPORT_W * s, panel[3] * s];
        let place = |rect: [f32; 4]| -> [f32; 4] {
            [
                (panel[0] + rect[0]) * s,
                (panel[1] + rect[1] - scroll) * s,
                rect[2] * s,
                rect[3] * s,
            ]
        };
        let mut next_hit = HIT_PANEL + 1;
        for placed in &column.parts {
            let rect = place(placed.rect);
            if rect[1] > clip[1] + clip[3] || rect[1] + rect[3] < clip[1] {
                continue;
            }
            match placed.part {
                Part::Title => self.text_cell(
                    frame,
                    "Lineage Panel Settings",
                    rect,
                    36.0,
                    true,
                    TextAlign::Center,
                    clip,
                ),
                Part::Span => {
                    let value = format!("{:.3}", app.lineage_effective_span());
                    let label = [rect[0], rect[1], 262.67 * s, rect[3]];
                    let number = [rect[0] + (262.67 + 20.0) * s, rect[1], 65.3 * s, rect[3]];
                    self.text_cell(
                        frame,
                        "Effective Species Span: ",
                        label,
                        25.0,
                        true,
                        TextAlign::Left,
                        clip,
                    );
                    self.text_cell(frame, &value, number, 25.0, true, TextAlign::Right, clip);
                }
                Part::GroupTitle(title) => {
                    let size = 36.0 * s;
                    let width = self.measure(title, size, true);
                    let at = [
                        rect[0] + (rect[2] - width) / 2.0,
                        rect[1] + (rect[3] - size * 1.17) / 2.0,
                        width,
                        size * 1.17,
                    ];
                    if inside_rect(at, clip) {
                        frame.draws.push(UiDraw::Text {
                            font: Some("Boston Black SDF".to_owned()),
                            text: title.to_owned(),
                            pos: Vec2::new(at[0], at[1]),
                            px: size,
                            color: WHITE,
                            role: FontRole::Bold,
                        });
                    }
                }
                Part::Toggle(option) => {
                    let row = toggle_row(placed.rect);
                    let (name, check) = (place(row.name), place(row.check));
                    self.text_cell(
                        frame,
                        option.label(),
                        name,
                        25.0,
                        true,
                        TextAlign::Left,
                        clip,
                    );
                    self.tip_rect(frame, name, clip, option.label(), option.help());
                    if let Some(texture) = self.ui.texture_of_alias("base_icon") {
                        clip_nine_slice(frame, self.ui, texture, 5.0, 5.0 * s, check, WHITE, clip);
                    }
                    if option_value(&options, option)
                        && let Some(texture) = self.ui.texture_of_alias("checkmark_icon")
                    {
                        clip_image(frame, self.ui, texture, None, check, WHITE, clip);
                    }
                    self.hit_rect(
                        frame,
                        check,
                        clip,
                        UiAction::LineageOption(option.index()),
                        &mut next_hit,
                    );
                }
                Part::Slider(index) => {
                    self.slider(frame, app, index, placed.rect, &place, clip, &mut next_hit);
                }
            }
        }
        if column.max_scroll() > 0.0 {
            self.settings_bar(frame, scroll, &column);
        }
    }

    /// The edit field of a slider row (`InputField (TMP)`): the sliced `InputFieldBackground`, the
    /// text in `LiberationSans SDF` 20 (dark grey, 12 in from the left), its selection highlight or
    /// the blinking caret after it, the placeholder while it is empty; a press on it keeps it open.
    pub(crate) fn edit_field(
        &self,
        frame: &mut UiFrame,
        edit: &crate::value_edit::ValueEdit,
        rect: [f32; 4],
        clip: [f32; 4],
        next_hit: &mut u32,
    ) {
        use crate::value_edit::text_width;
        let s = self.s;
        match self.ui.texture_of_alias("InputFieldBackground") {
            Some(texture) => {
                clip_nine_slice(frame, self.ui, texture, 8.0, 8.0 * s, rect, WHITE, clip)
            }
            None => clip_rect(frame, rect, WHITE, clip),
        }
        let size = 20.0 * s;
        let text_x = rect[0] + 12.0 * s;
        let middle = rect[1] + rect[3] / 2.0 + 0.5 * s;
        let line = 23.0 * s;
        let before = |chars: usize| text_width(&edit.text.chars().take(chars).collect::<String>(), 20.0) * s;
        let width = before(edit.caret.pos);
        if let Some((from, to)) = edit.caret.selection() {
            clip_rect(
                frame,
                [text_x + before(from), middle - line / 2.0, before(to) - before(from), line],
                [168, 206, 255, 192],
                clip,
            );
        }
        let (shown, ink) = if edit.text.is_empty() {
            ("Enter value...", [50, 50, 50, 128])
        } else {
            (edit.text.as_str(), [50, 50, 50, 255])
        };
        let height = size * 1.149;
        let at = [
            text_x,
            middle - height / 2.0,
            text_width(shown, 20.0) * s,
            height,
        ];
        if inside_rect(at, clip) {
            frame.draws.push(UiDraw::Text {
                text: shown.to_owned(),
                pos: Vec2::new(at[0], at[1]),
                px: size,
                color: ink,
                role: FontRole::Body,
                font: Some("LiberationSans SDF".to_owned()),
            });
        }
        if edit.caret_visible() {
            clip_rect(
                frame,
                [text_x + width, middle - line / 2.0, s.max(1.0), line],
                [50, 50, 50, 255],
                clip,
            );
        }
        self.hit_rect(frame, rect, clip, UiAction::ValueEditInside, next_hit);
    }

    /// One slider row of the panel (a setting of the world): the name, the value, the slider with its handle and default marker, the reset
    /// button.
    #[allow(clippy::too_many_arguments)]
    fn slider(
        &self,
        frame: &mut UiFrame,
        app: &App,
        index: usize,
        local: [f32; 4],
        place: &dyn Fn([f32; 4]) -> [f32; 4],
        clip: [f32; 4],
        next_hit: &mut u32,
    ) {
        let Some((row_index, row)) = crate::dyn_panel::ui_of(SLIDERS[index]) else {
            return;
        };
        let (mode, _) = crate::dyn_panel::slider_info_or_default(row);
        let field = crate::dyn_panel::FieldRef::Setting(row_index as u16);
        let value = crate::dyn_panel::read_value(&app.world.settings, row);
        let default = crate::dyn_panel::default_of(row, app.tps_defaults);
        let spec = SliderSpec {
            name: row.name,
            help: row.helper,
            field,
            value,
            format: crate::dyn_panel::format_of(row),
            rules: crate::dyn_panel::setting_rules(row, mode),
            fraction: crate::dyn_panel::fraction_of(mode, row, default, value).clamp(0.0, 1.0),
            default_fraction: crate::dyn_panel::fraction_of(mode, row, default, default)
                .clamp(0.0, 1.0),
            reset: UiAction::DynReset(row_index as u16),
            lineage: true,
        };
        let editing = app.value_edit.as_ref().filter(|edit| edit.field == field);
        self.slider_cells(
            frame,
            &spec,
            editing,
            local,
            self::panel(self.canvas[0])[0],
            &|from, to| UiAction::DynSlide(row_index as u16, from, to),
            place,
            clip,
            next_hit,
        );
    }

    /// The cells of a slider row (`SettingSlider`): `local` is the row in its column, `base_x` where the column starts (the track's
    /// ends in the action are in reference px), `place` puts a rectangle of the column on the screen.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn slider_cells(
        &self,
        frame: &mut UiFrame,
        spec: &SliderSpec<'_>,
        editing: Option<&crate::value_edit::ValueEdit>,
        local: [f32; 4],
        base_x: f32,
        slide: &dyn Fn(u16, u16) -> UiAction,
        place: &dyn Fn([f32; 4]) -> [f32; 4],
        clip: [f32; 4],
        next_hit: &mut u32,
    ) {
        let s = self.s;
        let field = spec.field;
        let parts = slider_row(local);
        let (name, value_cell, panel, slider, reset) = match editing {
            // The edit field takes the slider's place: the name and the value text share the row
            // with it by Unity's flexible sizes again.
            Some(_) => {
                let shares = crate::value_edit::row_parts(local[2], true);
                (
                    place([
                        local[0] + shares.name[0],
                        local[1],
                        shares.name[1] - shares.name[0],
                        local[3],
                    ]),
                    place([
                        local[0] + shares.value[0],
                        local[1],
                        shares.value[1] - shares.value[0],
                        local[3],
                    ]),
                    [0.0; 4],
                    [0.0; 4],
                    [0.0; 4],
                )
            }
            None => (
                place(parts.name),
                place(parts.value),
                place(parts.panel),
                place(parts.slider),
                place(parts.reset),
            ),
        };
        let (fraction, default_fraction) = (spec.fraction, spec.default_fraction);
        let format = &spec.format;
        // `UpdateValueDuringEdit`: while typing, the value text shows the value the text means.
        let text = editing
            .and_then(|edit| spec.rules.preview(&edit.text))
            .map_or_else(|| format.format(spec.value), |typed| format.format(typed));
        if spec.lineage {
            // `Name`: Roboto-Bold 20.6, 10 of left margin, wrapped to its cell and centred on the row.
            let cell = [name[0] + 10.0 * s, name[1], name[2] - 10.0 * s, name[3]];
            self.wrapped_cell(frame, spec.name, cell, 20.6, clip);
        } else {
            self.text_cell(frame, spec.name, name, 17.05, true, TextAlign::Left, clip);
        }
        self.tip_rect(frame, name, clip, spec.name, spec.help);
        self.text_cell(
            frame,
            &text,
            value_cell,
            25.33,
            true,
            TextAlign::Center,
            clip,
        );
        // The value text is the row's `editButton`.
        self.hit_rect(
            frame,
            value_cell,
            clip,
            UiAction::DynEditValue(field),
            next_hit,
        );
        if let Some(edit) = editing {
            let shares = crate::value_edit::row_parts(local[2], true);
            let height = crate::value_edit::FIELD_HEIGHT;
            let rect = place([
                local[0] + shares.body[0],
                local[1] + (local[3] - height) / 2.0,
                shares.body[1] - shares.body[0],
                height,
            ]);
            self.edit_field(frame, edit, rect, clip, next_hit);
            return;
        }
        // The slider's panel: a dim `Background` (border 5; the Lineage Panel Settings' has none), the track, the fill and the handle.
        if let Some(texture) = self.ui.texture_of_alias("Background") {
            if !spec.lineage {
                clip_nine_slice(
                    frame,
                    self.ui,
                    texture,
                    10.0,
                    5.0 * s,
                    panel,
                    [255, 255, 255, 100],
                    clip,
                );
            }
            // The track is 5 longer than the slider and its border 10 (ppum 0.5).
            let track = [slider[0], slider[1], slider[2] + 5.0 * s, slider[3]];
            clip_nine_slice(
                frame,
                self.ui,
                texture,
                10.0,
                10.0 * s,
                track,
                [255, 255, 255, 65],
                clip,
            );
        }
        let centre = slider[0] + fraction * slider[2];
        if let Some(texture) = self.ui.texture_of_alias("UISprite") {
            let left = slider[0] - 5.0 * s;
            let fill = [left, slider[1], centre + 5.0 * s - left, slider[3]];
            clip_nine_slice(frame, self.ui, texture, 10.0, 10.0 * s, fill, WHITE, clip);
        }
        // The default value's pointer: a 4 x 10 tick above and below the track.
        let mark = slider[0] + default_fraction * slider[2];
        clip_rect(
            frame,
            [mark - 2.0 * s, slider[1] - 11.0 * s, 4.0 * s, 10.0 * s],
            TICK,
            clip,
        );
        clip_rect(
            frame,
            [mark - 2.0 * s, slider[1] + slider[3] + s, 4.0 * s, 10.0 * s],
            TICK,
            clip,
        );
        if let Some(texture) = self.ui.texture_of_alias("base_circle_icon") {
            let handle = [centre - 15.0 * s, slider[1] - 5.0 * s, 30.0 * s, 30.0 * s];
            clip_image(frame, self.ui, texture, None, handle, WHITE, clip);
        }
        // The reset button: `Button_UI` at pixels-per-unit 16, its icon 4 larger.
        if let Some(texture) = self.ui.texture_of_alias("Button_UI") {
            clip_nine_slice(
                frame,
                self.ui,
                texture,
                30.0,
                30.0 / 16.0 * s,
                reset,
                WHITE,
                clip,
            );
        }
        if let Some(texture) = self
            .ui
            .texture_of_alias("Reset settings icon-1.png")
            .or_else(|| self.ui.texture_of_alias("reset_icon"))
        {
            let icon = [
                reset[0] - 2.0 * s,
                reset[1] - 2.0 * s,
                reset[2] + 4.0 * s,
                reset[3] + 4.0 * s,
            ];
            clip_image(frame, self.ui, texture, None, icon, WHITE, clip);
        }
        // Presses: the slider follows the cursor over its panel (the value is read from the track's ends in reference px),
        // the reset button restores the default.
        let ends = (
            (base_x + parts.slider[0]).round() as u16,
            (base_x + parts.slider[0] + parts.slider[2]).round() as u16,
        );
        self.hit_rect(frame, panel, clip, slide(ends.0, ends.1), next_hit);
        self.hit_rect(frame, reset, clip, spec.reset, next_hit);
    }

    /// The scroll bar: the `Background` track at 0.255 and the `UISprite` handle.
    fn settings_bar(&self, frame: &mut UiFrame, scroll: f32, column: &Layout) {
        let s = self.s;
        let max = column.max_scroll();
        let bar = crate::lineage_view::BarState {
            value: (scroll / max).clamp(0.0, 1.0),
            size: (PANEL[3] / column.height).clamp(0.0, 1.0),
        };
        // The bar's 0 is its top: the vertical helper counts from the bottom.
        let flipped = crate::lineage_view::BarState {
            value: 1.0 - bar.value,
            ..bar
        };
        let everything = [0.0, 0.0, 100_000.0, 100_000.0];
        if let Some(texture) = self.ui.texture_of_alias("Background") {
            let bar = bar_track(panel(self.canvas[0]));
            let track = [bar[0] * s, bar[1] * s, bar[2] * s, bar[3] * s];
            clip_nine_slice(
                frame,
                self.ui,
                texture,
                10.0,
                10.0 * s,
                track,
                [255, 255, 255, 65],
                everything,
            );
        }
        let handle = crate::lineage_view::handle_rect(bar_track(panel(self.canvas[0])), flipped, true);
        if let Some(texture) = self.ui.texture_of_alias("UISprite") {
            let rect = [handle[0] * s, handle[1] * s, handle[2] * s, handle[3] * s];
            clip_nine_slice(
                frame,
                self.ui,
                texture,
                10.0,
                10.0 * s,
                rect,
                WHITE,
                everything,
            );
        }
    }

    /// Text in a cell, kept to it: shrunk to the width, aligned, vertically centred; left out when the mask would cut it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text_cell(
        &self,
        frame: &mut UiFrame,
        text: &str,
        cell: [f32; 4],
        px: f32,
        bold: bool,
        align: TextAlign,
        clip: [f32; 4],
    ) {
        let size = self.fitted(text, px * self.s, bold, cell[2]);
        let width = self.measure(text, size, bold);
        let x = match align {
            TextAlign::Left => cell[0],
            TextAlign::Center => cell[0] + (cell[2] - width) / 2.0,
            TextAlign::Right => cell[0] + cell[2] - width,
        };
        let at = [
            x,
            cell[1] + (cell[3] - size * 1.17) / 2.0,
            width,
            size * 1.17,
        ];
        if inside_rect(at, clip) {
            if bold {
                frame.bold(text.to_owned(), Vec2::new(at[0], at[1]), size, WHITE);
            } else {
                frame.text(text.to_owned(), Vec2::new(at[0], at[1]), size, WHITE);
            }
        }
    }

    /// `text` in the bold face, auto-sized like TMP (`enableAutoSizing`: the largest size up to `px` reference px at which the text wrapped at the
    /// spaces to the width of `cell` fits its height; a long name takes two lines at 17.05), left aligned and centred vertically in `cell`.
    pub(crate) fn wrapped_cell(&self, frame: &mut UiFrame, text: &str, cell: [f32; 4], px: f32, clip: [f32; 4]) {
        let wrap = |size: f32| -> Vec<String> {
            let mut lines: Vec<String> = Vec::new();
            let mut line = String::new();
            for word in text.split(' ') {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if !line.is_empty() && self.measure(&candidate, size, true) > cell[2] {
                    lines.push(std::mem::take(&mut line));
                    line = word.to_owned();
                } else {
                    line = candidate;
                }
            }
            lines.push(line);
            lines
        };
        let s = self.s;
        let mut size = px * s;
        let mut lines = wrap(size);
        // A line is `1.17` of its size high; shrink in 0.05 steps until all of them fit.
        while lines.len() as f32 * size * 1.17 > cell[3] + 0.01 * s && size > 6.0 * s {
            size -= 0.05 * s;
            lines = wrap(size);
        }
        let line_height = size * 1.17;
        let top = cell[1] + (cell[3] - line_height * lines.len() as f32) / 2.0;
        for (index, line) in lines.into_iter().enumerate() {
            let at = [cell[0], top + index as f32 * line_height, cell[2], line_height];
            if inside_rect(at, clip) {
                frame.bold(line, Vec2::new(at[0], at[1]), size, WHITE);
            }
        }
    }

    /// A press target over a rectangle (cut to the mask).
    pub(crate) fn hit_rect(
        &self,
        frame: &mut UiFrame,
        rect: [f32; 4],
        clip: [f32; 4],
        action: UiAction,
        next_hit: &mut u32,
    ) {
        if let Some(r) = intersect_rect(rect, clip) {
            frame.hit(
                *next_hit,
                Vec2::new(r[0], r[1]),
                Vec2::new(r[2], r[3]),
                action,
            );
            *next_hit += 1;
        }
    }

    /// A tooltip over a rectangle (cut to the mask).
    pub(crate) fn tip_rect(
        &self,
        frame: &mut UiFrame,
        rect: [f32; 4],
        clip: [f32; 4],
        title: &str,
        content: &str,
    ) {
        if let Some(r) = intersect_rect(rect, clip) {
            frame.tips.push(TipRect {
                pos: Vec2::new(r[0], r[1]),
                size: Vec2::new(r[2], r[3]),
                tip: Tip {
                    path: "",
                    title: tooltip::intern(title.to_owned()),
                    content: tooltip::intern(content.to_owned()),
                    ui: true,
                    instant: false,
                },
            });
        }
    }
}

/// What a slider row shows and does: its name and help, the setting it edits, its value and where the handle and the default mark sit.
pub(crate) struct SliderSpec<'a> {
    /// The setting's name.
    pub name: &'a str,
    /// Its `HelperText`, the tooltip of the name.
    pub help: &'a str,
    /// What typing a value into the row edits.
    pub field: crate::dyn_panel::FieldRef,
    /// The setting's value.
    pub value: f32,
    /// How the value text reads.
    pub format: bibites_core::format::FloatValueFormat,
    /// What typing a value does (the factor, the ends, ...).
    pub rules: crate::value_edit::Rules,
    /// Where the value sits on the track, `0..=1`.
    pub fraction: f32,
    /// Where the default value sits.
    pub default_fraction: f32,
    /// The press on the reset button.
    pub reset: UiAction,
    /// The Lineage Panel Settings' row: its name is 20.6 (not 17.05) with a margin of 10 and wraps in its cell, and its `SliderPanel` has no
    /// backing (the `Image` is disabled).
    pub lineage: bool,
}

/// How a text sits in its cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextAlign {
    Left,
    Center,
    Right,
}

pub(crate) fn intersect_rect(a: [f32; 4], b: [f32; 4]) -> Option<[f32; 4]> {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

pub(crate) fn inside_rect(a: [f32; 4], clip: [f32; 4]) -> bool {
    a[0] >= clip[0] - 0.01
        && a[1] >= clip[1] - 0.01
        && a[0] + a[2] <= clip[0] + clip[2] + 0.01
        && a[1] + a[3] <= clip[1] + clip[3] + 0.01
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_is_as_tall_as_the_layout_groups_make_it() {
        // Padding 20 + title 42.19 + 20 + line 29.3 + 20, "Lineage Options" 340 (50 + 10, a holder of 10 + 2 x (40 + 20) + the
        // 130-high "Lineage Tree" group + 20), 20, "Species Options" 250, padding 35.
        let tree = layout(true);
        assert!(
            (tree.height - (20.0 + 42.19 + 20.0 + 29.3 + 20.0 + 340.0 + 20.0 + 250.0 + 35.0)).abs()
                < 1e-3
        );
        assert!(
            tree.max_scroll() > 36.0 && tree.max_scroll() < 37.0,
            "{}",
            tree.max_scroll()
        );
        // Without the tree group (the mesh view) it is 150 shorter and fits the panel.
        let mesh = layout(false);
        assert!((tree.height - mesh.height - 150.0).abs() < 1e-3);
        assert_eq!(mesh.max_scroll(), 0.0);
        // The parts come in order and never overlap.
        let mut bottom = 0.0;
        for placed in &tree.parts {
            assert!(
                placed.rect[1] >= bottom - 1e-3,
                "{placed:?} starts above {bottom}"
            );
            bottom = placed.rect[1] + placed.rect[3];
        }
        assert!(bottom <= tree.height);
        assert_eq!(
            tree.parts
                .iter()
                .filter(|p| matches!(p.part, Part::Toggle(_)))
                .count(),
            3
        );
        assert_eq!(
            mesh.parts
                .iter()
                .filter(|p| matches!(p.part, Part::Toggle(_)))
                .count(),
            2
        );
    }

    #[test]
    fn a_slider_row_shares_what_is_left_by_the_weights_two_one_one() {
        let row = slider_row([20.0, 100.0, INNER_W, 40.0]);
        // 660.5 - (125 + 80 + 255 + 30) = 170.5 over the weights 2 + 1 + 1.
        assert!((row.name[2] - (125.0 + 85.25)).abs() < 1e-3);
        assert!((row.value[2] - (80.0 + 42.625)).abs() < 1e-3);
        assert!((row.panel[2] - (200.0 + 42.625)).abs() < 1e-3);
        // 15 apart, the reset button 40 wide at the end, the slider inset by 10.
        assert!((row.value[0] - (row.name[0] + row.name[2] + 15.0)).abs() < 1e-3);
        assert!((row.panel[0] - (row.value[0] + row.value[2] + 15.0)).abs() < 1e-3);
        assert!((row.reset[0] + 40.0 - (20.0 + INNER_W)).abs() < 1e-3);
        assert_eq!(row.slider[1], 110.0);
        assert_eq!(row.slider[3], 20.0);
        assert!((row.slider[2] - (row.panel[2] - 20.0)).abs() < 1e-3);
    }

    #[test]
    fn a_toggle_row_has_its_box_in_the_middle_of_the_350_wide_toggle() {
        let row = toggle_row([20.0, 100.0, INNER_W, 40.0]);
        assert!((row.name[2] - (INNER_W - 365.0)).abs() < 1e-3);
        assert!((row.check[0] + 20.0 - (20.0 + INNER_W - 175.0)).abs() < 1e-3);
        assert_eq!((row.check[2], row.check[3]), (40.0, 40.0));
    }

    #[test]
    fn the_options_have_the_games_names_and_help() {
        for option in [
            LineageOption::HideExtinct,
            LineageOption::ShowFavorites,
            LineageOption::ScaleSpecies,
        ] {
            assert_eq!(LineageOption::from_index(option.index()), Some(option));
            assert!(
                crate::user_settings::index_of(option.label()).is_some(),
                "{}",
                option.label()
            );
            assert!(option.help().starts_with("If enabled"));
        }
        assert!(LineageOption::from_index(3).is_none());
        for key in SLIDERS {
            assert!(crate::dyn_panel::ui_of(key).is_some(), "{key}");
            // Every one is a plain slider (the species settings are in no in-world tab).
            let row = crate::dyn_panel::ui_of(key).unwrap().1;
            let (mode, fixed) = crate::dyn_panel::slider_info_or_default(row);
            assert!(fixed.is_none(), "{key}");
            assert!(
                matches!(
                    mode,
                    crate::dyn_panel::SliderMode::Linear | crate::dyn_panel::SliderMode::Int
                ),
                "{key}"
            );
        }
    }
}
