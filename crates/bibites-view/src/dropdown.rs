//! The open list of a dropdown (`TMP_Dropdown.Show`): what the game's dropdown shows when it is
//! clicked, used by every dropdown of the port (the Settings rows, the New Game modifiers, the zone
//! and default-bibite items, the placers, the towers).
//!
//! Prefab values (`TMP_Dropdown` template of the scenes): a list (the `Template`, an `UISprite`
//! slice) as wide as the dropdown, hung under it (2 px of overlap) and at most 150 high, with the
//! items stacked in its `Content` (an item is 20 to 30 high, `Item Background` (0.961 grey) tinted by
//! the item's `Toggle`: lighter on hover, darker when pressed), a checkmark on the current choice
//! and the choice's label (dark grey, Roboto-Bold 16 or Liberation Sans 12-14); a 20-wide scrollbar
//! shows when the choices do not fit, and the list flips above the dropdown when it would leave
//! the screen. A blocker behind it closes the list on any other click.
//!
//! The Graphs panel's two dropdowns are the game's own `MyDropdown` family instead
//! (`RealDropdown`, `TogglesDropdown`): a template as tall as its items (35 high, no slack, no
//! scrolling), checkmarks, items with a tooltip of their own, and a list that closes when the pointer
//! leaves the dropdown and its list. The curves dropdown is a multi-select: a pick toggles the curve
//! and the list stays open.
//!
//! No screenshot of an open list exists in the reference captures: the geometry is derived from
//! those serialized values.

use bibites_core::math::Vec2;

use crate::tooltip::{Tip, TipRect};
use crate::ui::{DropdownTarget, FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiId};

/// The template's height: the list is never taller (`Template.sizeDelta.y`).
pub const TEMPLATE_HEIGHT: f32 = 150.0;
/// `Content.sizeDelta.y` of the `TMP_Dropdown` prefabs; the content is `n * item + (CONTENT - item)`
/// high (a style's `slack`).
const CONTENT_HEIGHT: f32 = 28.0;
/// The template sits 2 above the dropdown's bottom edge (`anchoredPosition.y`).
const GAP: f32 = 2.0;
/// `Scrollbar.sizeDelta.x`, and the viewport's inset while it shows (`Viewport.sizeDelta.x`).
const SCROLLBAR: f32 = 20.0;
/// `Item Background`: (0.961, 0.961, 0.961).
const ITEM: Rgba = [245, 245, 245, 255];
/// The item toggle's `ColorBlock`: highlighted and selected are 0.961 of the normal.
const HOVER: Rgba = [235, 235, 235, 255];
/// The label colour, (0.2, 0.2, 0.2).
const INK: Rgba = [51, 51, 51, 255];
/// The Graphs panel's label colour, (0.149, 0.149, 0.149).
const GRAPHS_INK: Rgba = [38, 38, 38, 255];
/// Line heights over the size: Roboto and Liberation Sans.
const ROBOTO_LINE: f32 = 1.1719;
const LIBERATION_LINE: f32 = 1.149;
/// Hit ids: the blocker, the list, then the rows.
const HIT_BLOCKER: UiId = 46_000;
const HIT_LIST: UiId = 46_001;
const HIT_ANCHOR: UiId = 46_002;
const HIT_ROW: UiId = 46_010;

/// How one family of dropdowns draws its items (canvas units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// `Item` height.
    pub item: f32,
    /// Label font size.
    pub label_px: f32,
    /// Roboto-Bold (`true`) or Liberation Sans.
    pub bold: bool,
    /// Checkmark size and the x of its centre.
    pub check: f32,
    /// x of the checkmark's centre.
    pub check_x: f32,
    /// The checkmark's sprite: the default `Checkmark`, or the Graphs panel's bolder `checkmark_icon`.
    pub check_sprite: &'static str,
    /// Left edge of the label.
    pub label_x: f32,
    /// The content's height beyond `n * item` (the template's slack: `28 - item` for `TMP_Dropdown`,
    /// none for a layout-sized template).
    pub slack: f32,
    /// How tall the list gets before it scrolls.
    pub max_height: f32,
    /// The label colour.
    pub ink: Rgba,
}

/// The standard dropdown: 30-high items, Roboto-Bold 16 (the placer, the settings, the zone items).
pub const ROBOTO: Style = Style {
    item: 30.0,
    label_px: 16.0,
    bold: true,
    check: 20.0,
    check_x: 15.0,
    check_sprite: "Checkmark",
    label_x: 31.3,
    slack: CONTENT_HEIGHT - 30.0,
    max_height: TEMPLATE_HEIGHT,
    ink: INK,
};
/// The New Game screen's dropdowns: Roboto-Bold 20 and a 25-unit checkmark.
pub const ROBOTO_LARGE: Style = Style {
    item: 30.0,
    label_px: 20.0,
    bold: true,
    check: 25.0,
    check_x: 15.0,
    check_sprite: "Checkmark",
    label_x: 31.3,
    slack: CONTENT_HEIGHT - 30.0,
    max_height: TEMPLATE_HEIGHT,
    ink: INK,
};
/// The towers' dropdowns: 20-high items in Liberation Sans 12.
pub const LIBERATION: Style = Style {
    item: 20.0,
    label_px: 12.0,
    bold: false,
    check: 20.0,
    check_x: 10.0,
    check_sprite: "Checkmark",
    label_x: 20.0,
    slack: CONTENT_HEIGHT - 20.0,
    max_height: TEMPLATE_HEIGHT,
    ink: INK,
};
/// The species panels' compared-species dropdown: 18-high items (a 15-unit checkmark at x 8, the
/// label from x 15) in Roboto-Bold 12, the template 100 high at most.
pub const COMPARED: Style = Style {
    item: 18.0,
    label_px: 12.0,
    bold: true,
    check: 15.0,
    check_x: 8.0,
    check_sprite: "Checkmark",
    label_x: 15.0,
    slack: CONTENT_HEIGHT - 18.0,
    max_height: 100.0,
    ink: [50, 50, 50, 255],
};
/// The Graphs panel's dropdowns: 35-high items (a 30-unit checkmark, the label from x 40) in
/// Roboto-Bold 22, listed in full (`ContentSizeFitter`) in the label's darker grey.
pub const GRAPHS: Style = Style {
    item: 35.0,
    label_px: 22.0,
    bold: true,
    check: 30.0,
    check_x: 15.0,
    check_sprite: "checkmark_icon",
    label_x: 40.0,
    slack: 0.0,
    max_height: f32::MAX,
    ink: GRAPHS_INK,
};

/// The Gene Analysis dropdown (`GraphsPanel.genesDropdown`): the same family with 22-high items (a 20-unit checkmark, the label from x
/// 30) in Roboto-Bold 12.8, the 35 genes listed in full.
pub const GENES: Style = Style {
    item: 22.0,
    label_px: 12.8,
    bold: true,
    check: 20.0,
    check_x: 15.0,
    check_sprite: "checkmark_icon",
    label_x: 30.0,
    slack: 0.0,
    max_height: f32::MAX,
    ink: GRAPHS_INK,
};

/// One option of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The option's text.
    pub label: String,
    /// The option's tooltip (`SettingChoice.tooltipText`; empty when it has none).
    pub tip: &'static str,
    /// The header of the tooltip the item shows while the pointer rests on it (empty: none; an item
    /// with a `DropdownItem` tooltip shows its label as the header).
    pub title: &'static str,
}

impl Choice {
    /// An option without a tooltip.
    #[must_use]
    pub fn plain(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            tip: "",
            title: "",
        }
    }

    /// An option whose item shows `title` and `tip` as its tooltip (`DropdownItemData`).
    #[must_use]
    pub fn described(title: &'static str, tip: &'static str) -> Self {
        Self {
            label: title.to_owned(),
            tip,
            title,
        }
    }

    /// An option of a setting's dropdown: while the pointer rests on its item the tooltip reads
    /// "`setting`: label" over `tip` (`ChoiceSettingDropdown.OnItemHovered`, which shows
    /// `SettingChoice.tooltipText`; `TooltipSystem.Show` has no delay).
    #[must_use]
    pub fn of_setting(setting: &str, label: impl Into<String>, tip: &'static str) -> Self {
        let label = label.into();
        Self {
            title: crate::tooltip::intern(format!("{setting}: {label}")),
            tip,
            label,
        }
    }

    /// An option of a `FloatSettingDropdown` (`NumericSettingDropdown.OnItemHovered`): the item's
    /// tooltip reads "`setting`: label" over the landmark's own text when it has one, else over its
    /// value formatted with the setting's `precision` and `units` (`FormattedValueOfLandmark`).
    #[must_use]
    pub fn of_landmark(
        setting: &str,
        label: impl Into<String>,
        value: f32,
        precision: i32,
        units: &str,
        text: Option<&'static str>,
    ) -> Self {
        let tip = text.unwrap_or_else(|| {
            crate::tooltip::intern(bibites_core::format::scientific_format(
                value,
                precision,
                bibites_core::format::ScientificOptions {
                    units,
                    ..bibites_core::format::ScientificOptions::default()
                },
            ))
        });
        Self::of_setting(setting, label, tip)
    }
}

/// An open dropdown list.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    /// The control the list belongs to (what a pick does).
    pub target: DropdownTarget,
    /// The dropdown control (reference px: `x, y, w, h`).
    pub anchor: [f32; 4],
    /// The options.
    pub choices: Vec<Choice>,
    /// The current choice (it carries the checkmark).
    pub selected: usize,
    /// How the items look.
    pub style: Style,
    /// How far the content is scrolled (canvas units).
    pub scroll: f32,
    /// The item drawn as the event system's selected one (`TMP_Dropdown.Show` selects the current
    /// choice; the custom dropdowns select the item last clicked).
    pub tint: Option<usize>,
    /// A multi-select list (`TogglesDropdown`): whether each choice is on, in place of `selected`.
    pub checked: Option<Vec<bool>>,
    /// The list closes when the pointer leaves the dropdown and the list (`MyDropdown.OnPointerExit`).
    pub leave_closes: bool,
}

/// The list's geometry (screen px).
#[derive(Clone, Debug, PartialEq)]
pub struct Laid {
    /// The list: `x, y, w, h`.
    pub list: [f32; 4],
    /// The visible rows: the choice index and its rectangle (clipped to the viewport).
    pub rows: Vec<(usize, [f32; 4])>,
    /// The scrollbar's track and handle, when the choices do not fit.
    pub scrollbar: Option<([f32; 4], [f32; 4])>,
    /// The largest scroll offset (canvas units).
    pub max_scroll: f32,
}

impl Open {
    /// A list for `choices`, opened on the dropdown at `anchor`.
    #[must_use]
    pub fn new(
        target: DropdownTarget,
        anchor: [f32; 4],
        choices: Vec<Choice>,
        selected: usize,
        style: Style,
    ) -> Self {
        Self {
            target,
            anchor,
            choices,
            selected,
            style,
            scroll: 0.0,
            tint: Some(selected),
            checked: None,
            leave_closes: false,
        }
    }

    /// The list of a `MyDropdown`: it closes when the pointer leaves, and nothing is tinted until
    /// an item is clicked.
    #[must_use]
    pub fn leaving(mut self) -> Self {
        self.leave_closes = true;
        self.tint = None;
        self
    }

    /// The list of a `TogglesDropdown`: one on/off mark per choice.
    #[must_use]
    pub fn with_checks(mut self, checked: Vec<bool>) -> Self {
        self.checked = Some(checked);
        self.leaving()
    }

    /// Whether choice `index` carries the checkmark.
    #[must_use]
    pub fn is_checked(&self, index: usize) -> bool {
        match &self.checked {
            Some(marks) => marks.get(index).copied().unwrap_or(false),
            None => index == self.selected,
        }
    }

    /// Whether `cursor` is over the dropdown or its list, on a `screen` of the given size at UI
    /// scale `s` (`OnPointerExit` fires when it is over neither: the list is the dropdown's child).
    #[must_use]
    pub fn contains(&self, cursor: Vec2, screen: Vec2, s: f32) -> bool {
        let inside = |rect: [f32; 4]| {
            cursor.x >= rect[0]
                && cursor.x < rect[0] + rect[2]
                && cursor.y >= rect[1]
                && cursor.y < rect[1] + rect[3]
        };
        let [ax, ay, aw, ah] = self.anchor;
        inside([ax * s, ay * s, aw * s, ah * s]) || inside(self.lay_out(screen, s).list)
    }

    /// The height of the content: `n` items plus the template's slack (`Show` resizes `Content`).
    #[must_use]
    pub fn content_height(&self) -> f32 {
        self.choices.len() as f32 * self.style.item + self.style.slack
    }

    /// The list's height: the content, or the template's when that is shorter.
    #[must_use]
    pub fn list_height(&self) -> f32 {
        self.content_height().min(self.style.max_height)
    }

    /// The largest scroll offset.
    #[must_use]
    pub fn max_scroll(&self) -> f32 {
        (self.content_height() - self.list_height()).max(0.0)
    }

    /// Scrolls by `notches` of the mouse wheel (up is positive); an item is one notch.
    pub fn wheel(&mut self, notches: f32) {
        self.scroll = (self.scroll - notches * self.style.item).clamp(0.0, self.max_scroll());
    }

    /// Lays the list out on a `screen` of the given size; `s` is the UI scale (canvas unit -> px).
    #[must_use]
    pub fn lay_out(&self, screen: Vec2, s: f32) -> Laid {
        let [ax, ay, aw, ah] = self.anchor;
        let (x, below_top) = (ax * s, (ay + ah - GAP) * s);
        let (width, height) = (aw * s, self.list_height() * s);
        // Under the dropdown, or above it when that would leave the screen.
        let y = if below_top + height > screen.y && ay * s + GAP * s - height >= 0.0 {
            ay * s + GAP * s - height
        } else {
            below_top
        };
        // And inside the screen sideways.
        let x = x.min(screen.x - width).max(0.0);
        let max_scroll = self.max_scroll();
        let scrolls = max_scroll > 0.0;
        let viewport_w = width - if scrolls { SCROLLBAR * s } else { 0.0 };
        let item = self.style.item * s;
        // The first item's top is half the slack below the content's top (negative for tall items).
        let pad = self.style.slack * 0.5 * s;
        let scroll = self.scroll.clamp(0.0, max_scroll) * s;
        let mut rows = Vec::new();
        for index in 0..self.choices.len() {
            let top = y + pad + index as f32 * item - scroll;
            let (clipped_top, clipped_bottom) = (top.max(y), (top + item).min(y + height));
            if clipped_bottom - clipped_top > 0.5 {
                rows.push((
                    index,
                    [x, clipped_top, viewport_w, clipped_bottom - clipped_top],
                ));
            }
        }
        let scrollbar = scrolls.then(|| {
            let track = [x + width - SCROLLBAR * s, y, SCROLLBAR * s, height];
            let visible = (self.list_height() / self.content_height()).clamp(0.05, 1.0);
            let handle_h = (height * visible).max(SCROLLBAR * s);
            let free = height - handle_h;
            let handle_y = y + free * (self.scroll.clamp(0.0, max_scroll) / max_scroll);
            (track, [track[0], handle_y, track[2], handle_h])
        });
        Laid {
            list: [x, y, width, height],
            rows,
            scrollbar,
            max_scroll,
        }
    }
}

/// Draws the open list over everything and registers its hits: a blocker that closes the list, the
/// list itself, and one `DropdownPick` per visible row. `cursor` (screen px) lights the row under it.
pub fn emit(
    frame: &mut UiFrame,
    ui: &UiAssets,
    metrics: Option<&crate::text_sdf::SdfFonts>,
    open: &Open,
    screen: Vec2,
    cursor: Vec2,
) {
    let s = frame.scale;
    let laid = open.lay_out(screen, s);
    // The list covers the screen: the controls under it show no tooltips, only its items do.
    frame.tips.clear();
    // Any other click closes the list, and does nothing else.
    frame.hit(HIT_BLOCKER, Vec2::ZERO, screen, UiAction::DropdownClose);
    if open.leave_closes {
        // Clicking the dropdown again only (re)activates its template (`MyDropdown.OnPointerClick`).
        let [ax, ay, aw, ah] = open.anchor;
        frame.hit(
            HIT_ANCHOR,
            Vec2::new(ax * s, ay * s),
            Vec2::new(aw * s, ah * s),
            UiAction::Consume,
        );
    }
    let [lx, ly, lw, lh] = laid.list;
    // The template: an `UISprite` slice (border 10 at 200 pixels per unit).
    match ui.texture_of_alias("UISprite") {
        Some(texture) => frame.draws.push(UiDraw::NineSlice {
            texture,
            border: [10.0; 4],
            dest_scale: 0.5,
            pos: Vec2::new(lx, ly),
            size: Vec2::new(lw, lh),
            tint: [255, 255, 255, 255],
        }),
        None => {
            frame.rect(Vec2::new(lx, ly), Vec2::new(lw, lh), [90, 90, 90, 255]);
            frame.rect(
                Vec2::new(lx + s, ly + s),
                Vec2::new(lw - 2.0 * s, lh - 2.0 * s),
                ITEM,
            );
        }
    }
    frame.hit(
        HIT_LIST,
        Vec2::new(lx, ly),
        Vec2::new(lw, lh),
        UiAction::Consume,
    );
    let style = open.style;
    let measure = |text: &str, px: f32| -> f32 {
        metrics.map_or(text.chars().count() as f32 * px * 0.5, |fonts| {
            let (name, role) = if style.bold {
                (None, FontRole::Bold)
            } else {
                (Some("LiberationSans SDF"), FontRole::Body)
            };
            fonts.measure_named(name, role, text, px)
        })
    };
    let px = style.label_px * s;
    let line = px
        * if style.bold {
            ROBOTO_LINE
        } else {
            LIBERATION_LINE
        };
    let item = style.item * s;
    for (index, rect) in &laid.rows {
        let inside = cursor.x >= rect[0]
            && cursor.x < rect[0] + rect[2]
            && cursor.y >= rect[1]
            && cursor.y < rect[1] + rect[3];
        // `Item Background` tinted by the toggle: highlighted under the pointer and on the selected
        // item, the normal grey otherwise.
        let color = if inside || open.tint == Some(*index) {
            HOVER
        } else {
            ITEM
        };
        frame.rect(
            Vec2::new(rect[0], rect[1]),
            Vec2::new(rect[2], rect[3]),
            color,
        );
        // The row's unclipped top, for the glyphs (a half row at the edge keeps its text in place).
        let full_top = if rect[3] >= item - 0.5 {
            rect[1]
        } else if rect[1] <= ly + 0.5 {
            rect[1] + rect[3] - item
        } else {
            rect[1]
        };
        if open.is_checked(*index)
            && let Some(texture) = ui.texture_of_alias(style.check_sprite)
        {
            let size = style.check * s;
            frame.draws.push(UiDraw::Image {
                texture,
                pos: Vec2::new(
                    rect[0] + style.check_x * s - size * 0.5,
                    full_top + (item - size) * 0.5,
                ),
                size: Vec2::new(size, size),
                tint: [255, 255, 255, 255],
            });
        }
        let label = &open.choices[*index].label;
        let text_x = rect[0] + style.label_x * s;
        let room = rect[0] + rect[2] - 2.7 * s - text_x;
        // A label wider than its room shrinks to fit (TMP's wrapped label would clip).
        let width = measure(label, px);
        let fitted = if width > room && room > 0.0 {
            px * room / width
        } else {
            px
        };
        let at = Vec2::new(text_x, full_top + (item - line) * 0.5);
        frame.draws.push(UiDraw::Text {
            font: if style.bold {
                None
            } else {
                Some("LiberationSans SDF".to_owned())
            },
            text: label.clone(),
            pos: at,
            px: fitted,
            color: style.ink,
            role: if style.bold {
                FontRole::Bold
            } else {
                FontRole::Body
            },
        });
        frame.hit(
            HIT_ROW + *index as UiId,
            Vec2::new(rect[0], rect[1]),
            Vec2::new(rect[2], rect[3]),
            UiAction::DropdownPick(*index as u16),
        );
        // An item with a `TooltipTrigger` of its own shows it at once (`TooltipSystem.Show`).
        let choice = &open.choices[*index];
        if !choice.title.is_empty() && !choice.tip.is_empty() {
            frame.tips.push(TipRect {
                pos: Vec2::new(rect[0], rect[1]),
                size: Vec2::new(rect[2], rect[3]),
                tip: Tip {
                    path: "",
                    title: choice.title,
                    content: choice.tip,
                    ui: true,
                    instant: true,
                },
            });
        }
    }
    if let Some((track, handle)) = laid.scrollbar {
        frame.rect(
            Vec2::new(track[0], track[1]),
            Vec2::new(track[2], track[3]),
            [255, 255, 255, 255],
        );
        match ui.texture_of_alias("UISprite") {
            Some(texture) => frame.draws.push(UiDraw::NineSlice {
                texture,
                border: [10.0; 4],
                dest_scale: 0.5,
                pos: Vec2::new(handle[0], handle[1]),
                size: Vec2::new(handle[2], handle[3]),
                tint: [200, 200, 200, 255],
            }),
            None => frame.rect(
                Vec2::new(handle[0], handle[1]),
                Vec2::new(handle[2], handle[3]),
                [200, 200, 200, 255],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(count: usize, style: Style) -> Open {
        Open::new(
            DropdownTarget::PelletMaterial,
            [100.0, 100.0, 148.0, 30.0],
            (0..count)
                .map(|n| Choice::plain(format!("Option {n}")))
                .collect(),
            1,
            style,
        )
    }

    #[test]
    fn the_list_is_as_tall_as_its_content_up_to_the_templates_150() {
        // `Show` resizes the content to `n * item + (28 - item)` and trims the template to it.
        assert!((open(3, ROBOTO).list_height() - (3.0 * 30.0 - 2.0)).abs() < 1e-3);
        assert!((open(3, LIBERATION).list_height() - (3.0 * 20.0 + 8.0)).abs() < 1e-3);
        assert_eq!(open(8, ROBOTO).list_height(), TEMPLATE_HEIGHT);
        assert!(open(8, ROBOTO).max_scroll() > 0.0);
        assert_eq!(open(4, ROBOTO).max_scroll(), 0.0, "118 fits in 150");
    }

    #[test]
    fn the_list_hangs_under_the_dropdown_and_flips_above_at_the_screen_edge() {
        let screen = Vec2::new(1920.0, 1080.0);
        let below = open(3, ROBOTO).lay_out(screen, 1.0);
        // As wide as the dropdown, two px above its bottom edge.
        assert_eq!(below.list[0], 100.0);
        assert_eq!(below.list[2], 148.0);
        assert!((below.list[1] - (100.0 + 30.0 - 2.0)).abs() < 1e-3);
        // Near the bottom of the screen it goes above (two px below the dropdown's top edge).
        let mut low = open(3, ROBOTO);
        low.anchor[1] = 1040.0;
        let above = low.lay_out(screen, 1.0);
        assert!((above.list[1] + above.list[3] - (1040.0 + 2.0)).abs() < 1e-3);
        // The UI scale multiplies everything.
        let big = open(3, ROBOTO).lay_out(screen, 1.5);
        assert!((big.list[2] - 148.0 * 1.5).abs() < 1e-3);
        assert!((big.list[3] - 88.0 * 1.5).abs() < 1e-3);
    }

    #[test]
    fn the_rows_stack_inside_the_list_and_a_long_list_scrolls_with_a_scrollbar() {
        let screen = Vec2::new(1920.0, 1080.0);
        let short = open(3, ROBOTO);
        let laid = short.lay_out(screen, 1.0);
        assert_eq!(laid.rows.len(), 3);
        assert!(laid.scrollbar.is_none());
        // The rows are stacked without gaps, as wide as the list; the middle one shows in full (30
        // high) while the first and last lose their 1 px of slack to the list's edges.
        let rect = |index: usize| laid.rows[index].1;
        assert!((rect(0)[1] + rect(0)[3] - rect(1)[1]).abs() < 1e-3);
        assert!((rect(1)[1] + rect(1)[3] - rect(2)[1]).abs() < 1e-3);
        assert!((rect(1)[3] - 30.0).abs() < 1e-3);
        assert_eq!(rect(0)[2], 148.0);

        let mut long = open(10, ROBOTO);
        let laid = long.lay_out(screen, 1.0);
        let (track, handle) = laid.scrollbar.expect("ten items do not fit in 150");
        assert_eq!(track[2], SCROLLBAR);
        assert!(handle[3] < track[3], "the handle is shorter than the track");
        // The viewport leaves room for the scrollbar; only the rows in view are listed.
        assert_eq!(laid.rows[0].1[2], 148.0 - SCROLLBAR);
        assert!(laid.rows.len() < 10);
        // The wheel scrolls (an item per notch) and stops at the ends.
        long.wheel(-1.0);
        assert_eq!(long.scroll, 30.0);
        long.wheel(-100.0);
        assert_eq!(long.scroll, long.max_scroll());
        long.wheel(100.0);
        assert_eq!(long.scroll, 0.0);
        let scrolled = {
            long.scroll = long.max_scroll();
            long.lay_out(screen, 1.0)
        };
        assert_eq!(
            scrolled.rows.last().map(|row| row.0),
            Some(9),
            "the last choice is reachable"
        );
    }

    fn drawn(open: &Open, cursor: Vec2) -> UiFrame {
        let ui = UiAssets::empty();
        let mut frame = UiFrame::new(1.0);
        emit(
            &mut frame,
            &ui,
            None,
            open,
            Vec2::new(1920.0, 1080.0),
            cursor,
        );
        frame
    }

    #[test]
    fn a_click_on_a_row_picks_it_and_any_other_click_closes_the_list() {
        let list = open(3, ROBOTO);
        let frame = drawn(&list, Vec2::ZERO);
        let laid = list.lay_out(Vec2::new(1920.0, 1080.0), 1.0);
        for (index, rect) in &laid.rows {
            let centre = Vec2::new(rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5);
            assert_eq!(
                frame.action_at(centre),
                Some(UiAction::DropdownPick(*index as u16))
            );
        }
        // Outside the list: the blocker; on the dropdown control itself too (it is under the blocker).
        assert_eq!(
            frame.action_at(Vec2::new(500.0, 500.0)),
            Some(UiAction::DropdownClose)
        );
        assert_eq!(
            frame.action_at(Vec2::new(150.0, 110.0)),
            Some(UiAction::DropdownClose),
            "the dropdown's own box is covered by the blocker"
        );
    }

    fn graphs_list(count: usize) -> Open {
        Open::new(
            DropdownTarget::GraphsCurves,
            [316.0, 87.0, 397.0, 37.0],
            (0..count)
                .map(|n| {
                    Choice::described(
                        Box::leak(format!("Curve {n}").into_boxed_str()),
                        "A curve of the graph",
                    )
                })
                .collect(),
            0,
            GRAPHS,
        )
    }

    #[test]
    fn the_graphs_lists_are_as_tall_as_their_items_and_never_scroll() {
        // Seven 35-high items, no slack, no scrollbar (`ContentSizeFitter`).
        let list = graphs_list(7);
        assert_eq!(list.list_height(), 245.0);
        assert_eq!(list.max_scroll(), 0.0);
        let laid = list.lay_out(Vec2::new(1920.0, 1080.0), 1.0);
        assert!(laid.scrollbar.is_none());
        assert_eq!(laid.rows.len(), 7);
        for (_, rect) in &laid.rows {
            assert_eq!(rect[3], 35.0);
        }
        // Two px under the dropdown's bottom edge, as wide as the dropdown.
        assert_eq!(laid.list[0], 316.0);
        assert_eq!(laid.list[2], 397.0);
        assert_eq!(laid.list[1], 87.0 + 37.0 - 2.0);
    }

    #[test]
    fn a_leaving_list_covers_the_dropdown_and_closes_when_the_pointer_is_over_neither() {
        let screen = Vec2::new(1920.0, 1080.0);
        let list = graphs_list(3).leaving();
        assert!(list.leave_closes);
        // On the dropdown, on the list (which overlaps its bottom edge), both open.
        assert!(list.contains(Vec2::new(400.0, 100.0), screen, 1.0));
        assert!(list.contains(Vec2::new(400.0, 150.0), screen, 1.0));
        // Beside, above and far below it, closed.
        assert!(!list.contains(Vec2::new(100.0, 100.0), screen, 1.0));
        assert!(!list.contains(Vec2::new(400.0, 50.0), screen, 1.0));
        assert!(!list.contains(Vec2::new(400.0, 400.0), screen, 1.0));
        // The UI scale moves the whole thing.
        assert!(list.contains(Vec2::new(400.0 * 1.5, 100.0 * 1.5), screen, 1.5));
        // Clicking the dropdown again does not close the list (it is covered by a hit that
        // swallows the click), while the standard lists do close that way.
        let frame = drawn(&list, Vec2::ZERO);
        assert_eq!(
            frame.action_at(Vec2::new(350.0, 100.0)),
            Some(UiAction::Consume)
        );
        let standard = drawn(&open(3, ROBOTO), Vec2::ZERO);
        assert_eq!(
            standard.action_at(Vec2::new(150.0, 110.0)),
            Some(UiAction::DropdownClose)
        );
    }

    #[test]
    fn a_multi_select_list_marks_every_item_that_is_on() {
        let list = graphs_list(4).with_checks(vec![true, false, true, false]);
        assert!(list.leave_closes && list.tint.is_none());
        let marks: Vec<bool> = (0..4).map(|index| list.is_checked(index)).collect();
        assert_eq!(marks, [true, false, true, false]);
        // A single-select list marks its current choice only.
        let single = open(3, ROBOTO);
        let marks: Vec<bool> = (0..3).map(|index| single.is_checked(index)).collect();
        assert_eq!(marks, [false, true, false]);
        // Only the item under the pointer or last clicked is tinted: not every item that is on.
        let laid = list.lay_out(Vec2::new(1920.0, 1080.0), 1.0);
        let frame = drawn(&list, Vec2::ZERO);
        let tinted = frame
            .draws
            .iter()
            .filter(|draw| matches!(draw, UiDraw::Rect { color, .. } if *color == HOVER))
            .count();
        assert_eq!(tinted, 0, "{} rows, none tinted", laid.rows.len());
        let mut clicked = list.clone();
        clicked.tint = Some(2);
        let frame = drawn(&clicked, Vec2::ZERO);
        let tinted = frame
            .draws
            .iter()
            .filter(|draw| matches!(draw, UiDraw::Rect { color, .. } if *color == HOVER))
            .count();
        assert_eq!(tinted, 1);
    }

    #[test]
    fn an_item_with_a_description_shows_it_as_its_tooltip_and_the_controls_below_show_none() {
        let list = graphs_list(3).leaving();
        let ui = UiAssets::empty();
        let mut frame = UiFrame::new(1.0);
        // A tooltip of a control under the list.
        frame.tips.push(TipRect {
            pos: Vec2::ZERO,
            size: Vec2::new(1920.0, 1080.0),
            tip: Tip {
                path: "x",
                title: "Below",
                content: "",
                ui: true,
                instant: false,
            },
        });
        emit(
            &mut frame,
            &ui,
            None,
            &list,
            Vec2::new(1920.0, 1080.0),
            Vec2::ZERO,
        );
        assert_eq!(frame.tips.len(), 3, "only the three items' tips remain");
        let laid = list.lay_out(Vec2::new(1920.0, 1080.0), 1.0);
        let on_second = Vec2::new(laid.rows[1].1[0] + 5.0, laid.rows[1].1[1] + 5.0);
        let rect = frame
            .tips
            .iter()
            .rev()
            .find(|rect| rect.contains(on_second))
            .expect("the second item's tip");
        assert_eq!(rect.tip.title, "Curve 1");
        assert_eq!(rect.tip.content, "A curve of the graph");
        assert!(rect.tip.ui && rect.tip.instant);
        // A plain choice has no tooltip.
        let plain = drawn(&open(3, ROBOTO), Vec2::ZERO);
        assert!(plain.tips.is_empty());
    }

    #[test]
    fn a_settings_item_shows_the_setting_and_the_choice_as_the_header_and_keeps_its_own_label() {
        // `ChoiceSettingDropdown.OnItemHovered`: the header is "Setting: Choice", the row says "Choice".
        let list = Open::new(
            DropdownTarget::Zone(0, 0),
            [316.0, 87.0, 200.0, 37.0],
            vec![
                Choice::of_setting("Movement", "None", "The Zone will be fixed and not move."),
                Choice::of_setting("Movement", "Free", "The Zone will freely move around"),
                Choice::plain("No tooltip"),
            ],
            0,
            ROBOTO,
        );
        let ui = UiAssets::empty();
        let mut frame = UiFrame::new(1.0);
        emit(
            &mut frame,
            &ui,
            None,
            &list,
            Vec2::new(1920.0, 1080.0),
            Vec2::ZERO,
        );
        assert_eq!(frame.tips.len(), 2, "the plain choice has none");
        let laid = list.lay_out(Vec2::new(1920.0, 1080.0), 1.0);
        let on = |row: usize| Vec2::new(laid.rows[row].1[0] + 5.0, laid.rows[row].1[1] + 5.0);
        let first = frame.tips.iter().find(|rect| rect.contains(on(0))).unwrap();
        assert_eq!(first.tip.title, "Movement: None");
        assert_eq!(first.tip.content, "The Zone will be fixed and not move.");
        assert!(
            first.tip.ui && first.tip.instant,
            "`TooltipSystem.Show` has no delay"
        );
        let second = frame.tips.iter().find(|rect| rect.contains(on(1))).unwrap();
        assert_eq!(second.tip.title, "Movement: Free");
        assert!(frame.tips.iter().all(|rect| !rect.contains(on(2))));
        // The row's own text stays the choice.
        let texts: Vec<&str> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            texts.contains(&"None") && texts.contains(&"Free"),
            "{texts:?}"
        );
        assert!(
            !texts.iter().any(|text| text.contains("Movement")),
            "{texts:?}"
        );
    }

    #[test]
    fn the_current_choice_and_the_hovered_row_are_highlighted_and_labelled() {
        let list = open(3, ROBOTO);
        let laid = list.lay_out(Vec2::new(1920.0, 1080.0), 1.0);
        let hover = &laid.rows[2].1;
        let frame = drawn(&list, Vec2::new(hover[0] + 5.0, hover[1] + 5.0));
        let fills: Vec<Rgba> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Rect { size, color, .. }
                    if (size.x - 148.0).abs() < 0.5 && size.y < 31.0 =>
                {
                    Some(*color)
                }
                _ => None,
            })
            .collect();
        // Row 0 normal, row 1 the current choice, row 2 under the pointer.
        assert_eq!(fills, vec![ITEM, HOVER, HOVER]);
        let labels: Vec<&str> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(labels, ["Option 0", "Option 1", "Option 2"]);
    }
}
