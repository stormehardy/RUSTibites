//! The vertical `Scrollbar` of the game's lists (`Scrollbar Vertical` of a `ScrollRect` with `AutoHideAndExpandViewport`): a track as wide as the bar with its
//! `Sliding Area` inset by half that width, and a handle whose `sizeDelta` is (width, width): it reaches half a width over each end of its share of the sliding
//! area. The handle is as long as the share of the content the view shows (`Scrollbar.size = view / content`); pressing it starts a drag that keeps the pointer
//! where it grabbed the handle; pressing the track outside it moves the content by a page toward the pointer. The lists' bars differ in width and sprites only
//! (the Load Game list's: 20, `Background` and `UISprite`; the scenario list's: 16, `base_icon_thin` at multiplier 0.5; the Library's: 12, `scroll` and `base_icon`;
//! the Credits and Additional Tools tabs': 20, the Load Game list's sprites with a faint track).
//!
//! Only the geometry and the arithmetic live here (reference px, y down); the owner draws it with [`VBar::draw`] and feeds it the pointer.

use bibites_core::math::Vec2;

use crate::species_panel::Drawer;
use crate::ui::{Rgba, UiFrame};

/// What a press on the bar hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Press {
    /// The handle: a drag starts, `grab` is the pointer's distance under the handle's top.
    Handle {
        /// The pointer's distance under the handle's top.
        grab: f32,
    },
    /// The track above (`-1`) or under (`1`) the handle: the content moves a page that way.
    Page(f32),
}

/// A 9-sliced sprite of the bar: its name, the border of the sprite (px) and the scale it is drawn at (`100 / ppu / multiplier`), and its tint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skin {
    /// The sprite's name.
    pub sprite: &'static str,
    /// The sprite's border.
    pub border: f32,
    /// The border's scale on the canvas.
    pub scale: f32,
    /// The `Image.color`.
    pub tint: Rgba,
}

/// What the track and the handle are made of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// The track (`Scrollbar Vertical`'s own image).
    pub track: Skin,
    /// The handle.
    pub handle: Skin,
}

impl Style {
    /// The Load Game and Save Game lists' bar: white `Background` and `UISprite` (border 10 at ppu 200).
    pub const SAVES: Self = Self {
        track: Skin {
            sprite: "Background",
            border: 10.0,
            scale: 0.5,
            tint: [255, 255, 255, 255],
        },
        handle: Skin {
            sprite: "UISprite",
            border: 10.0,
            scale: 0.5,
            tint: [255, 255, 255, 255],
        },
    };

    /// The Credits and Additional Tools tabs' bar (20 wide, at the tab's right edge): the sprites of [`Style::SAVES`], the track at 0.255 alpha.
    pub const TABS: Self = Self {
        track: Skin {
            sprite: "Background",
            border: 10.0,
            scale: 0.5,
            tint: [255, 255, 255, 65],
        },
        handle: Skin {
            sprite: "UISprite",
            border: 10.0,
            scale: 0.5,
            tint: [255, 255, 255, 255],
        },
    };

    /// The scenario list's bar: `base_icon_thin` (border 3) at multiplier 0.5, the track grey at 0.58 alpha.
    pub const SCENARIOS: Self = Self {
        track: Skin {
            sprite: "base_icon_thin",
            border: 3.0,
            scale: 2.0,
            tint: [145, 145, 145, 148],
        },
        handle: Skin {
            sprite: "base_icon_thin",
            border: 3.0,
            scale: 2.0,
            tint: [255, 255, 255, 255],
        },
    };

    /// The Library lists' bar: the dark `scroll` track (border 3) and the `base_icon` handle (border 5).
    pub const LIBRARY: Self = Self {
        track: Skin {
            sprite: "scroll",
            border: 3.0,
            scale: 1.0,
            tint: [82, 82, 82, 255],
        },
        handle: Skin {
            sprite: "base_icon",
            border: 5.0,
            scale: 1.0,
            tint: [255, 255, 255, 255],
        },
    };
}

/// A vertical scrollbar over a list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VBar {
    /// The track `[x, y, w, h]`.
    pub track: [f32; 4],
    /// The height of the whole content.
    pub content: f32,
    /// The height of the view.
    pub view: f32,
    /// How far the content is scrolled (0 at the top).
    pub scroll: f32,
    /// The sprites.
    pub style: Style,
}

impl VBar {
    /// A bar on `track` for `content` high content in a `view` high view scrolled `scroll`.
    #[must_use]
    pub fn new(track: [f32; 4], content: f32, view: f32, scroll: f32, style: Style) -> Self {
        Self {
            track,
            content,
            view,
            scroll,
            style,
        }
    }

    /// The farthest the content scrolls.
    #[must_use]
    pub fn max_scroll(&self) -> f32 {
        (self.content - self.view).max(0.0)
    }

    /// `AutoHideAndExpandViewport`: the bar is there only when the content does not fit.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.max_scroll() > 0.0
    }

    /// The room the handle moves in: the track less the half width of the sliding area's inset at each end.
    fn room(&self) -> f32 {
        (self.track[3] - self.track[2]).max(0.0)
    }

    /// The handle's length without its half width over each end.
    fn length(&self) -> f32 {
        let size = if self.content > 0.0 {
            (self.view / self.content).clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.room() * size
    }

    /// The handle `[x, y, w, h]`.
    #[must_use]
    pub fn handle(&self) -> [f32; 4] {
        let max = self.max_scroll();
        let along = if max > 0.0 { self.scroll / max } else { 0.0 };
        let length = self.length();
        [
            self.track[0],
            self.track[1] + along * (self.room() - length),
            self.track[2],
            length + self.track[2],
        ]
    }

    /// A press at `at`: the handle (a drag), the track (a page), or nothing.
    #[must_use]
    pub fn press(&self, at: Vec2) -> Option<Press> {
        if !self.visible()
            || at.x < self.track[0]
            || at.x >= self.track[0] + self.track[2]
            || at.y < self.track[1]
            || at.y >= self.track[1] + self.track[3]
        {
            return None;
        }
        let handle = self.handle();
        if at.y >= handle[1] && at.y < handle[1] + handle[3] {
            Some(Press::Handle {
                grab: at.y - handle[1],
            })
        } else if at.y < handle[1] {
            Some(Press::Page(-1.0))
        } else {
            Some(Press::Page(1.0))
        }
    }

    /// The scroll that puts the handle's top at `top` (clamped to the ends).
    #[must_use]
    pub fn scroll_for_handle_top(&self, top: f32) -> f32 {
        let travel = self.room() - self.length();
        if travel <= 0.0 {
            return 0.0;
        }
        ((top - self.track[1]) / travel).clamp(0.0, 1.0) * self.max_scroll()
    }

    /// The scroll after a page that way (`-1` up, `1` down).
    #[must_use]
    pub fn paged(&self, direction: f32) -> f32 {
        (self.scroll + direction * self.view).clamp(0.0, self.max_scroll())
    }

    /// Draws the track and the handle; `handle_tint` is the handle's `Selectable` state (white, 245 hovered, 200 pressed) over its own tint.
    pub fn draw(&self, frame: &mut UiFrame, d: &Drawer<'_>, handle_tint: Rgba) {
        if !self.visible() {
            return;
        }
        let Style { track, handle } = self.style;
        d.slice(
            frame,
            track.sprite,
            track.border,
            track.scale,
            self.track,
            track.tint,
        );
        let mix = |own: Rgba, state: Rgba| -> Rgba {
            [
                (u16::from(own[0]) * u16::from(state[0]) / 255) as u8,
                (u16::from(own[1]) * u16::from(state[1]) / 255) as u8,
                (u16::from(own[2]) * u16::from(state[2]) / 255) as u8,
                (u16::from(own[3]) * u16::from(state[3]) / 255) as u8,
            ]
        };
        d.slice(
            frame,
            handle.sprite,
            handle.border,
            handle.scale,
            self.handle(),
            mix(handle.tint, handle_tint),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(scroll: f32) -> VBar {
        // The Save panel's: the track 913 high at (890, 75), 1160 of content in a view 930 high.
        VBar::new(
            [890.0, 75.0, 20.0, 913.0],
            1160.0,
            930.0,
            scroll,
            Style::SAVES,
        )
    }

    #[test]
    fn the_bar_is_there_only_when_the_content_does_not_fit() {
        assert!(bar(0.0).visible());
        let fits = VBar {
            content: 900.0,
            ..bar(0.0)
        };
        assert!(!fits.visible());
        assert_eq!(fits.press(Vec2::new(900.0, 100.0)), None);
        assert_eq!(fits.max_scroll(), 0.0);
    }

    #[test]
    fn the_handle_is_as_long_as_the_share_of_the_content_the_view_shows() {
        let bar = bar(0.0);
        // room 893; size 930 / 1160; its two half widths added.
        let handle = bar.handle();
        assert!(
            (handle[3] - (893.0 * 930.0 / 1160.0 + 20.0)).abs() < 0.01,
            "{handle:?}"
        );
        // At the top it starts at the track's top, at the end it ends at the track's bottom.
        assert_eq!(handle[1], 75.0);
        let end = VBar {
            scroll: 230.0,
            ..bar
        }
        .handle();
        assert!((end[1] + end[3] - (75.0 + 913.0)).abs() < 0.01, "{end:?}");
    }

    #[test]
    fn a_narrower_bar_insets_by_half_its_own_width() {
        // The scenario list's: 16 wide at (733.2, 108), 946 high.
        let bar = VBar::new(
            [733.2, 108.0, 16.0, 946.0],
            2800.0,
            946.0,
            0.0,
            Style::SCENARIOS,
        );
        // room 930; the handle reaches 8 over each end of its share.
        assert!(
            (bar.handle()[3] - (930.0 * 946.0 / 2800.0 + 16.0)).abs() < 0.01,
            "{:?}",
            bar.handle()
        );
        let end = VBar {
            scroll: bar.max_scroll(),
            ..bar
        }
        .handle();
        assert!((end[1] + end[3] - (108.0 + 946.0)).abs() < 0.01, "{end:?}");
    }

    #[test]
    fn a_press_on_the_handle_drags_and_one_on_the_track_pages() {
        let bar = bar(100.0);
        let handle = bar.handle();
        let inside = Vec2::new(900.0, handle[1] + 30.0);
        assert_eq!(bar.press(inside), Some(Press::Handle { grab: 30.0 }));
        assert_eq!(bar.press(Vec2::new(900.0, 76.0)), Some(Press::Page(-1.0)));
        assert_eq!(
            bar.press(Vec2::new(900.0, 75.0 + 913.0 - 2.0)),
            Some(Press::Page(1.0))
        );
        // Outside the track nothing is hit.
        assert_eq!(bar.press(Vec2::new(500.0, 200.0)), None);
        // A page is the view's height, clamped to the ends.
        assert_eq!(bar.paged(1.0), 230.0);
        assert_eq!(bar.paged(-1.0), 0.0);
        assert_eq!(VBar { scroll: 0.0, ..bar }.paged(1.0), 230.0);
    }

    #[test]
    fn dragging_the_handle_follows_the_pointer_and_stops_at_the_ends() {
        let bar = bar(0.0);
        let travel = 893.0 - 893.0 * 930.0 / 1160.0;
        // The handle's top at the track's top: scrolled 0; half way down: half of 230; past the end: 230.
        assert_eq!(bar.scroll_for_handle_top(75.0), 0.0);
        assert!((bar.scroll_for_handle_top(75.0 + travel / 2.0) - 115.0).abs() < 0.01);
        assert_eq!(bar.scroll_for_handle_top(75.0 + travel), 230.0);
        assert_eq!(bar.scroll_for_handle_top(5000.0), 230.0);
        assert_eq!(bar.scroll_for_handle_top(-5000.0), 0.0);
        // A very long list: the handle is as small as its overhang lets it be, and still reaches the end.
        let long = VBar {
            content: 1_000_000.0,
            ..bar
        };
        assert!(long.handle()[3] < 21.0, "{:?}", long.handle());
        assert!((long.scroll_for_handle_top(5000.0) - long.max_scroll()).abs() < 0.01);
    }
}
