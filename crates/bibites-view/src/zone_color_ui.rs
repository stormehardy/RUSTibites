//! RUSTibites Improved: the colour pop-up of a zone (`zone_colors.rs` has the colour maths). A click on a zone in the Zones tab's Preview opens it at the pointer:
//! the zone's name, a **colour wheel** (hue round it, saturation from the middle out), a bar for the brightness, a box that shows the colour from before next to the
//! colour being made, a **hex field** the colour can be typed into (`#RRGGBB` or `RGB`), "Fertility Color" (the zone goes back to the colour all the zones have by
//! default), "All different" (every zone of the world gets a colour of its own, spread round the colour wheel) and "Done". The zone takes the colour as it is made, so
//! the world and the Preview show it at once; a click outside the pop-up or Done keeps it, Escape puts the colour from before back. Everything is in reference px
//! (the 1920 x 1080 canvas) and scaled on its way into the frame.

use std::sync::{Arc, Mutex};

use bibites_core::math::Vec2;

use crate::app::App;
use crate::hud::{sliced_image, tmp_label_aligned};
use crate::text_field::Caret;
use crate::text_sdf::SdfFonts;
use crate::time_tags_ui::{FONT, width_of};
use crate::ui::{Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiTexture};
use crate::zone_colors::{hex_of, hs_to_wheel, hsv_to_rgb};

/// Which part of the pop-up the pointer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupDrag {
    /// The colour wheel: hue and saturation follow the pointer.
    Wheel,
    /// The brightness bar.
    Value,
}

/// `TMP_InputField.caretBlinkRate` of the game's input fields (the hex field's caret blinks like theirs).
const BLINK_RATE: f32 = 0.85;

/// The pop-up's state while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneColorPopup {
    /// The zone's number in the world's zone list.
    pub zone: usize,
    /// The pointer's place (screen px) when the pop-up opened: the pop-up hangs from there.
    pub at: Vec2,
    /// The zone's own colour when the pop-up opened (`None`: it was in the Fertility Color); Escape puts it back.
    pub original: Option<[u8; 3]>,
    /// The colour being made: hue, saturation and brightness, each 0..1 (the zone has it from the first touch on).
    pub hsv: [f32; 3],
    /// The hex field's text: up to six hex digits, upper case, without the `#`.
    pub hex: String,
    /// The caret and the selection of the hex field.
    pub caret: Caret,
    /// Whether the keyboard is in the hex field (a click on it).
    pub hex_focus: bool,
    /// Seconds since the last key (the caret blinks from there).
    pub idle: f32,
    /// What the pointer holds.
    pub drag: Option<PopupDrag>,
}

impl ZoneColorPopup {
    /// A pop-up for zone `zone` opened with the pointer at `at`; the colour being made starts as `start` (the zone's own colour, or the colour it is drawn in).
    #[must_use]
    pub fn new(zone: usize, at: Vec2, original: Option<[u8; 3]>, start: [u8; 3]) -> Self {
        let (hue, saturation, value) = crate::zone_colors::rgb_to_hsv(start);
        let hex = hex_of(start);
        Self {
            zone,
            at,
            original,
            hsv: [hue, saturation, value],
            caret: Caret::at_end(&hex),
            hex,
            hex_focus: false,
            idle: 0.0,
            drag: None,
        }
    }

    /// The colour being made.
    #[must_use]
    pub fn rgb(&self) -> [u8; 3] {
        hsv_to_rgb(self.hsv[0], self.hsv[1], self.hsv[2])
    }

    /// Whether the hex field's caret is drawn now: no selection, and the first half of the blink period.
    #[must_use]
    pub fn caret_visible(&self) -> bool {
        let period = 1.0 / BLINK_RATE;
        self.caret.selection().is_none() && self.idle % period <= period / 2.0
    }
}

/// Ids of the pop-up's click areas (the block 9300.. of `ui::UiId`'s table): the backdrop that closes it, the panel that keeps the clicks on it, the buttons, the
/// wheel, the brightness bar and the hex field.
const HIT_BACKDROP: u32 = 9300;
const HIT_PANEL: u32 = 9302;
const HIT_CLEAR: u32 = 9303;
const HIT_SPREAD: u32 = 9304;
const HIT_WHEEL: u32 = 9305;
const HIT_VALUE: u32 = 9306;
const HIT_HEX: u32 = 9307;
const HIT_DONE: u32 = 9308;

const PAD: f32 = 12.0;
const TITLE_H: f32 = 30.0;
/// The wheel's diameter.
const WHEEL: f32 = 190.0;
/// The right column (the colours' box, the hex field, Done).
const COLUMN: f32 = 112.0;
const BAR_H: f32 = 22.0;
const BUTTON_H: f32 = 30.0;
const WHITE: Rgba = [255, 255, 255, 255];
const BLACK: Rgba = [0, 0, 0, 255];
const INK: Rgba = [38, 38, 38, 255];
const PANEL_BG: Rgba = [40, 40, 40, 250];

/// Where the parts of the pop-up are (reference px, `[x, y, w, h]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The whole pop-up.
    pub panel: [f32; 4],
    /// The square the wheel is in.
    pub wheel: [f32; 4],
    /// The brightness bar under the wheel.
    pub bar: [f32; 4],
    /// The box that shows the colour from before and the colour being made.
    pub colors: [f32; 4],
    /// The hex field.
    pub hex: [f32; 4],
    /// "Done".
    pub done: [f32; 4],
    /// "Fertility Color" and "All different".
    pub buttons: [[f32; 4]; 2],
}

/// The size of the pop-up.
#[must_use]
pub fn size() -> [f32; 2] {
    [2.0 * PAD + WHEEL + 14.0 + COLUMN, PAD + TITLE_H + 4.0 + WHEEL + 10.0 + BAR_H + 12.0 + BUTTON_H + PAD]
}

/// The pop-up's parts for a pointer at `at` (reference px) on a canvas `canvas` big: it hangs from the pointer and keeps inside the canvas.
#[must_use]
pub fn layout(at: [f32; 2], canvas: [f32; 2]) -> Layout {
    let [w, h] = size();
    let x = (at[0] + 10.0).min(canvas[0] - w - 6.0).max(6.0);
    let y = (at[1] + 10.0).min(canvas[1] - h - 6.0).max(6.0);
    let wheel = [x + PAD, y + PAD + TITLE_H + 4.0, WHEEL, WHEEL];
    let column = wheel[0] + WHEEL + 14.0;
    let bar = [wheel[0], wheel[1] + WHEEL + 10.0, WHEEL, BAR_H];
    let buttons_top = bar[1] + BAR_H + 12.0;
    let half = (w - 2.0 * PAD - 10.0) / 2.0;
    Layout {
        panel: [x, y, w, h],
        wheel,
        bar,
        colors: [column, wheel[1], COLUMN, 62.0],
        hex: [column, wheel[1] + 84.0, COLUMN, 30.0],
        done: [column, wheel[1] + WHEEL - BUTTON_H, COLUMN, BUTTON_H],
        buttons: [[x + PAD, buttons_top, half, BUTTON_H], [x + PAD + half + 10.0, buttons_top, half, BUTTON_H]],
    }
}

/// The wheel's middle and radius (reference px).
#[must_use]
pub fn wheel_circle(layout: &Layout) -> ([f32; 2], f32) {
    ([layout.wheel[0] + WHEEL / 2.0, layout.wheel[1] + WHEEL / 2.0], WHEEL / 2.0)
}

/// The wheel as a picture `diameter` px across (the hues round it, the saturation from the middle out, all at full brightness; the rim one pixel soft). Made once
/// per size and handed out again, as the Position tab's map is: the window uploads a picture once per image.
fn wheel_texture(diameter: u32) -> Arc<UiTexture> {
    static CACHE: Mutex<Option<(u32, Arc<UiTexture>)>> = Mutex::new(None);
    if let Ok(cache) = CACHE.lock()
        && let Some((size, texture)) = cache.as_ref()
        && *size == diameter
    {
        return Arc::clone(texture);
    }
    let radius = diameter as f32 / 2.0;
    let mut rgba = Vec::with_capacity((diameter * diameter * 4) as usize);
    for row in 0..diameter {
        for column in 0..diameter {
            let (dx, dy) = (column as f32 + 0.5 - radius, row as f32 + 0.5 - radius);
            let (hue, saturation) = crate::zone_colors::wheel_to_hs(dx, dy, radius);
            let [r, g, b] = hsv_to_rgb(hue, saturation, 1.0);
            let coverage = (radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[r, g, b, (coverage * 255.0).round() as u8]);
        }
    }
    let texture = Arc::new(UiTexture { width: diameter, height: diameter, rgba });
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((diameter, Arc::clone(&texture)));
    }
    texture
}

/// Draws the pop-up when it is open.
pub fn emit(frame: &mut UiFrame, app: &App, ui: &UiAssets, metrics: Option<&SdfFonts>, width: f32, height: f32) {
    let Some(popup) = app.zone_color_popup.as_ref() else {
        return;
    };
    let Some(zone) = app.world.zones.zones().get(popup.zone) else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    let sz = |w: f32, h: f32| Vec2::new(w * s, h * s);
    let canvas = [width / s, height / s];
    let l = layout([popup.at.x / s, popup.at.y / s], canvas);
    let rect = |frame: &mut UiFrame, r: [f32; 4], color: Rgba| frame.rect(at(r[0], r[1]), sz(r[2], r[3]), color);
    let outline = |frame: &mut UiFrame, r: [f32; 4], color: Rgba| {
        for edge in [[r[0], r[1], r[2], 1.0], [r[0], r[1] + r[3] - 1.0, r[2], 1.0], [r[0], r[1], 1.0, r[3]], [r[0] + r[2] - 1.0, r[1], 1.0, r[3]]] {
            frame.rect(at(edge[0], edge[1]), sz(edge[2], edge[3]), color);
        }
    };
    // Whatever is behind keeps out of it: a click outside the panel closes it.
    frame.hit(HIT_BACKDROP, Vec2::ZERO, Vec2::new(width, height), UiAction::ZoneColorClose);
    rect(frame, [l.panel[0] - 1.0, l.panel[1] - 1.0, l.panel[2] + 2.0, l.panel[3] + 2.0], [150, 150, 150, 255]);
    rect(frame, l.panel, PANEL_BG);
    frame.hit(HIT_PANEL, at(l.panel[0], l.panel[1]), sz(l.panel[2], l.panel[3]), UiAction::Consume);
    let title = format!("Colour of {}", zone.config.name);
    tmp_label_aligned(frame, metrics, s, FONT, &title, 19.0, WHITE, [l.panel[0] + PAD, l.panel[1] + PAD - 2.0, l.panel[2] - 2.0 * PAD, TITLE_H], 0.0);

    // The wheel, with a mark where the colour being made is.
    let diameter = (WHEEL * s).round().max(2.0) as u32;
    frame.draws.push(UiDraw::Pixels { texture: wheel_texture(diameter), pos: at(l.wheel[0], l.wheel[1]), size: Vec2::splat(diameter as f32) });
    frame.hit(HIT_WHEEL, at(l.wheel[0], l.wheel[1]), sz(l.wheel[2], l.wheel[3]), UiAction::ZoneColorWheel);
    let (centre, radius) = wheel_circle(&l);
    let (dx, dy) = hs_to_wheel(popup.hsv[0], popup.hsv[1], radius);
    let current = popup.rgb();
    let mark = [centre[0] + dx - 6.0, centre[1] + dy - 6.0, 12.0, 12.0];
    rect(frame, mark, BLACK);
    rect(frame, [mark[0] + 1.0, mark[1] + 1.0, 10.0, 10.0], WHITE);
    rect(frame, [mark[0] + 3.0, mark[1] + 3.0, 6.0, 6.0], [current[0], current[1], current[2], 255]);

    // The brightness bar: black to the colour at full brightness, with a handle where the colour's brightness is.
    let full = hsv_to_rgb(popup.hsv[0], popup.hsv[1], 1.0);
    let strips = 40;
    for strip in 0..strips {
        let value = (strip as f32 + 0.5) / strips as f32;
        let color = [(f32::from(full[0]) * value).round() as u8, (f32::from(full[1]) * value).round() as u8, (f32::from(full[2]) * value).round() as u8, 255];
        let (x0, x1) = (l.bar[0] + l.bar[2] * strip as f32 / strips as f32, l.bar[0] + l.bar[2] * (strip + 1) as f32 / strips as f32);
        rect(frame, [x0, l.bar[1], x1 - x0, l.bar[3]], color);
    }
    outline(frame, l.bar, [150, 150, 150, 255]);
    let handle = l.bar[0] + l.bar[2] * popup.hsv[2];
    rect(frame, [handle - 4.0, l.bar[1] - 3.0, 8.0, l.bar[3] + 6.0], BLACK);
    rect(frame, [handle - 3.0, l.bar[1] - 2.0, 6.0, l.bar[3] + 4.0], WHITE);
    frame.hit(HIT_VALUE, at(l.bar[0] - 4.0, l.bar[1] - 3.0), sz(l.bar[2] + 8.0, l.bar[3] + 6.0), UiAction::ZoneColorValueBar);

    // The colour from before (left) and the colour being made (right).
    let before = popup.original.unwrap_or_else(|| app.assets.as_ref().map_or([231, 154, 0], |assets| assets.background.zone_base));
    let half = l.colors[2] / 2.0;
    rect(frame, [l.colors[0], l.colors[1], half, l.colors[3]], [before[0], before[1], before[2], 255]);
    rect(frame, [l.colors[0] + half, l.colors[1], half, l.colors[3]], [current[0], current[1], current[2], 255]);
    outline(frame, l.colors, [150, 150, 150, 255]);
    tmp_label_aligned(frame, metrics, s, FONT, "before", 12.0, WHITE, [l.colors[0], l.colors[1] + l.colors[3] + 1.0, half, 14.0], 0.5);
    tmp_label_aligned(frame, metrics, s, FONT, "now", 12.0, WHITE, [l.colors[0] + half, l.colors[1] + l.colors[3] + 1.0, half, 14.0], 0.5);

    // The hex field: `#` and up to six digits, the caret and the selection while the keyboard is in it.
    sliced_image(frame, ui, s, "InputFieldBackground", 8.0, 1.0, l.hex, WHITE);
    let text_x = l.hex[0] + 10.0;
    let shown = format!("#{}", popup.hex);
    let before_caret = |chars: usize| width_of(metrics, &shown.chars().take(chars + 1).collect::<String>(), 18.0);
    if popup.hex_focus
        && let Some((from, to)) = popup.caret.selection()
    {
        rect(frame, [text_x + before_caret(from), l.hex[1] + 5.0, before_caret(to) - before_caret(from), l.hex[3] - 10.0], [168, 206, 255, 192]);
    }
    tmp_label_aligned(frame, metrics, s, FONT, &shown, 18.0, [50, 50, 50, 255], [text_x, l.hex[1], l.hex[2] - 16.0, l.hex[3]], 0.0);
    if popup.hex_focus && popup.caret_visible() {
        rect(frame, [text_x + before_caret(popup.caret.pos), l.hex[1] + 6.0, 1.5, l.hex[3] - 12.0], [50, 50, 50, 255]);
    }
    frame.hit(HIT_HEX, at(l.hex[0], l.hex[1]), sz(l.hex[2], l.hex[3]), UiAction::ZoneColorHexField);

    // The buttons.
    for (r, text, id, action) in [
        (l.done, "Done", HIT_DONE, UiAction::ZoneColorClose),
        (l.buttons[0], "Fertility Color", HIT_CLEAR, UiAction::ZoneColorClear(popup.zone as u16)),
        (l.buttons[1], "All different", HIT_SPREAD, UiAction::ZoneColorSpread),
    ] {
        sliced_image(frame, ui, s, "base_icon", 5.0, 1.0 / 0.75, r, WHITE);
        tmp_label_aligned(frame, metrics, s, FONT, text, 18.0, INK, r, 0.5);
        frame.hit(id, at(r[0], r[1]), sz(r[2], r[3]), action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;

    fn app() -> App {
        let loaded = LoadedWorld::load(None, 5, 3, 1).expect("starter world");
        App::new(loaded, Vec2::new(1920.0, 1080.0), None)
    }

    fn inside(outer: [f32; 4], inner: [f32; 4]) -> bool {
        inner[0] >= outer[0] && inner[1] >= outer[1] && inner[0] + inner[2] <= outer[0] + outer[2] && inner[1] + inner[3] <= outer[1] + outer[3]
    }

    fn apart(a: [f32; 4], b: [f32; 4]) -> bool {
        a[0] + a[2] <= b[0] || b[0] + b[2] <= a[0] || a[1] + a[3] <= b[1] || b[1] + b[3] <= a[1]
    }

    #[test]
    fn the_popup_hangs_from_the_pointer_keeps_inside_the_canvas_and_its_parts_do_not_cover_each_other() {
        let [w, h] = size();
        let canvas = [1920.0, 1080.0];
        let near = layout([300.0, 200.0], canvas);
        assert_eq!([near.panel[0], near.panel[1]], [310.0, 210.0]);
        let far = layout([1900.0, 1070.0], canvas);
        assert!(far.panel[0] + w <= canvas[0] && far.panel[1] + h <= canvas[1], "{far:?}");
        let corner = layout([-50.0, -50.0], canvas);
        assert!(corner.panel[0] >= 0.0 && corner.panel[1] >= 0.0);
        let parts = [near.wheel, near.bar, near.colors, near.hex, near.done, near.buttons[0], near.buttons[1]];
        for (i, a) in parts.iter().enumerate() {
            assert!(inside(near.panel, *a), "part {i} is inside the panel");
            for b in &parts[i + 1..] {
                assert!(apart(*a, *b), "{a:?} {b:?}");
            }
        }
        let (centre, radius) = wheel_circle(&near);
        assert_eq!((centre, radius), ([near.wheel[0] + 95.0, near.wheel[1] + 95.0], 95.0));
    }

    #[test]
    fn the_wheel_picture_has_the_hues_round_it_the_saturation_outwards_and_is_made_once_per_size() {
        let texture = wheel_texture(100);
        assert!(Arc::ptr_eq(&texture, &wheel_texture(100)), "the same picture is handed out again");
        assert!(!Arc::ptr_eq(&texture, &wheel_texture(120)), "another size is another picture");
        let pixel = |x: u32, y: u32| {
            let at = ((y * texture.width + x) * 4) as usize;
            [texture.rgba[at], texture.rgba[at + 1], texture.rgba[at + 2], texture.rgba[at + 3]]
        };
        // Far right, at the rim: red (hue 0) at full saturation; the middle is white; a corner outside the wheel is clear.
        let right = pixel(98, 50);
        assert!(right[0] > 240 && right[1] < 30 && right[2] < 30 && right[3] > 200, "{right:?}");
        let middle = pixel(50, 50);
        assert!(middle[0] > 230 && middle[1] > 230 && middle[2] > 230 && middle[3] == 255, "{middle:?}");
        assert_eq!(pixel(1, 1)[3], 0);
        // Up (a quarter turn counter-clockwise from the right) is a yellow-green (hue 0.25), left is cyan (0.5).
        let up = pixel(50, 2);
        assert!(up[1] > 240 && up[2] < 30 && up[0] > 100 && up[0] < 150, "{up:?}");
        let left = pixel(1, 50);
        assert!(left[0] < 30 && left[1] > 240 && left[2] > 240, "{left:?}");
    }

    #[test]
    fn the_open_popup_draws_its_parts_with_click_areas_and_the_backdrop_closes_it() {
        let mut app = app();
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, &app, &UiAssets::empty(), None, 1920.0, 1080.0);
        assert!(frame.hits.is_empty(), "nothing is drawn while the pop-up is shut");
        app.zone_color_popup = Some(ZoneColorPopup::new(0, Vec2::new(600.0, 400.0), None, [231, 154, 0]));
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, &app, &UiAssets::empty(), None, 1920.0, 1080.0);
        let l = layout([600.0, 400.0], [1920.0, 1080.0]);
        let middle = |r: [f32; 4]| Vec2::new(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        assert_eq!(frame.action_at(middle(l.wheel)), Some(UiAction::ZoneColorWheel));
        assert_eq!(frame.action_at(middle(l.bar)), Some(UiAction::ZoneColorValueBar));
        assert_eq!(frame.action_at(middle(l.hex)), Some(UiAction::ZoneColorHexField));
        assert_eq!(frame.action_at(middle(l.done)), Some(UiAction::ZoneColorClose));
        assert_eq!(frame.action_at(middle(l.buttons[0])), Some(UiAction::ZoneColorClear(0)));
        assert_eq!(frame.action_at(middle(l.buttons[1])), Some(UiAction::ZoneColorSpread));
        // A click on the panel's padding is kept; a click far from it reaches the backdrop, which closes the pop-up.
        assert_eq!(frame.action_at(Vec2::new(l.panel[0] + 2.0, l.panel[1] + l.panel[3] - 2.0)), Some(UiAction::Consume));
        assert_eq!(frame.action_at(Vec2::new(5.0, 5.0)), Some(UiAction::ZoneColorClose));
        // The wheel is one picture; the hex field shows the colour with its `#`.
        let pictures = frame.draws.iter().filter(|draw| matches!(draw, UiDraw::Pixels { .. })).count();
        assert_eq!(pictures, 1);
        let texts: Vec<String> = frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.trim().to_owned()),
                _ => None,
            })
            .collect();
        for wanted in ["#E79A00", "before", "now", "Done", "Fertility Color", "All different"] {
            assert!(texts.contains(&wanted.to_owned()), "{wanted}: {texts:?}");
        }
        assert!(texts.iter().any(|text| text.starts_with("Colour of ")), "{texts:?}");
        // The caret and the selection show only while the keyboard is in the field.
        let count = |app: &App| {
            let mut frame = UiFrame::new(1.0);
            emit(&mut frame, app, &UiAssets::empty(), None, 1920.0, 1080.0);
            frame.draws.len()
        };
        let before = count(&app);
        app.zone_color_popup.as_mut().unwrap().hex_focus = true;
        assert_eq!(count(&app), before + 1, "the blinking caret");
        app.zone_color_popup.as_mut().unwrap().caret.select_all("E79A00");
        assert_eq!(count(&app), before + 1, "the selection instead of the caret");
        // A zone that is not there is not drawn.
        app.zone_color_popup = Some(ZoneColorPopup::new(99, Vec2::ZERO, None, [0, 0, 0]));
        let mut frame = UiFrame::new(1.0);
        emit(&mut frame, &app, &UiAssets::empty(), None, 1920.0, 1080.0);
        assert!(frame.hits.is_empty());
    }
}
