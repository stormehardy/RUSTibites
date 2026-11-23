//! RUSTibites Improved: individually coloured zones. The game draws every zone in the one colour of the player's "Fertility Color" setting. Here the player can
//! give each zone a colour of its own (a click on the zone in the Zones tab's Preview opens the colour pop-up, `zone_color_ui.rs`); a zone without one is drawn in
//! the Fertility Color as before. The colour is `ZoneData::tint` (view only: not simulated, not in the game's save format, not hashed) and is kept beside the save by
//! the zone's name (`zone_colors/<save name>.json`, see `app/zone_color_actions.rs`). A check box on the View panel of the toolbar ("Individual Zone Colors") shows or
//! hides the colours the zones were given without taking them away.
//!
//! This module holds the colour maths of the pop-up (a colour wheel for hue and saturation, a bar for brightness, a hex field) and the colours "All different"
//! gives.

use std::f32::consts::TAU;

/// The colour a string of hex digits stands for: six digits (`FF8000`) or three (`F80`, each doubled), with or without a leading `#`, in either case. `None` for
/// anything else.
#[must_use]
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.trim().trim_start_matches('#');
    if !digits.is_ascii() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = |at: usize, len: usize| u8::from_str_radix(&digits[at..at + len], 16).ok();
    match digits.len() {
        6 => Some([value(0, 2)?, value(2, 2)?, value(4, 2)?]),
        3 => {
            let double = |at: usize| value(at, 1).map(|nibble| nibble * 17);
            Some([double(0)?, double(1)?, double(2)?])
        }
        _ => None,
    }
}

/// A colour as six upper case hex digits (`FF8000`).
#[must_use]
pub fn hex_of(rgb: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// The hue and saturation (each 0..1) of the point `(dx, dy)` of a colour wheel of `radius` (screen directions: `y` down): the hue is the angle from the right,
/// counter-clockwise, the saturation the distance from the middle (a point outside the wheel is taken to its rim).
#[must_use]
pub fn wheel_to_hs(dx: f32, dy: f32, radius: f32) -> (f32, f32) {
    let saturation = if radius > 0.0 { (dx.hypot(dy) / radius).clamp(0.0, 1.0) } else { 0.0 };
    let hue = (-dy).atan2(dx) / TAU;
    (hue.rem_euclid(1.0), saturation)
}

/// The place `(dx, dy)` of a hue and saturation on a colour wheel of `radius`: the other way round from [`wheel_to_hs`].
#[must_use]
pub fn hs_to_wheel(hue: f32, saturation: f32, radius: f32) -> (f32, f32) {
    let angle = hue * TAU;
    (angle.cos() * saturation * radius, -angle.sin() * saturation * radius)
}

/// The turn of the hue from one zone to the next in [`spread`]: `1 - 1/phi` of the wheel (the golden angle, 137.5 degrees).
const GOLDEN_TURN: f32 = 0.381_966;

/// The colour zone number `index` gets from "All different": the first zone keeps `base` (the Fertility Color) and every later one turns its hue a golden angle
/// further round the colour wheel, so that however many zones there are the next one is as far from the ones before it as it can be. The colour's saturation and
/// brightness are kept; a grey or black `base` has no hue to turn, so the zones after the first take a bright one instead.
#[must_use]
pub fn spread(base: [u8; 3], index: usize) -> [u8; 3] {
    if index == 0 {
        return base;
    }
    let (hue, saturation, value) = rgb_to_hsv(base);
    let (saturation, value) = if saturation < 0.25 { (0.85, value.max(0.9)) } else { (saturation, value) };
    let turned = (hue + GOLDEN_TURN * (index % 4096) as f32).fract();
    hsv_to_rgb(turned, saturation, value)
}

/// `(hue 0..1, saturation 0..1, value 0..1)` of an sRGB colour (the zone shader multiplies the colour as it is, so the colour wheel is the plain one).
pub fn rgb_to_hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = rgb.map(|channel| f32::from(channel) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let sixths = if delta <= f32::EPSILON {
        0.0
    } else if (max - r).abs() <= f32::EPSILON {
        ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() <= f32::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let hue = sixths / 6.0;
    let saturation = if max <= f32::EPSILON { 0.0 } else { delta / max };
    (hue, saturation, max)
}

/// The sRGB colour of a hue, saturation and value (each 0..1).
pub fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let sector = (hue.rem_euclid(1.0) * 6.0).min(5.999_999);
    let whole = sector.floor();
    let fraction = sector - whole;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    let (r, g, b) = match whole as u32 {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    [r, g, b].map(|channel| (channel * 255.0).round().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shorter way round the wheel between two hues, in turns.
    fn hue_gap(a: [u8; 3], b: [u8; 3]) -> f32 {
        let (ha, hb) = (rgb_to_hsv(a).0, rgb_to_hsv(b).0);
        let gap = (ha - hb).abs();
        gap.min(1.0 - gap)
    }

    #[test]
    fn hex_text_is_read_with_or_without_the_hash_in_either_case_and_in_three_or_six_digits() {
        assert_eq!(parse_hex("FF8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex("#ff8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex("  #0a1B2c "), Some([10, 27, 44]));
        assert_eq!(parse_hex("f80"), Some([255, 136, 0]), "three digits are doubled");
        assert_eq!(parse_hex("#FFF"), Some([255, 255, 255]));
        for bad in ["", "#", "12", "12345", "1234567", "GG0000", "#12 345", "ff80zz", "ffff\u{e9}0"] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
        assert_eq!(hex_of([255, 128, 0]), "FF8000");
        assert_eq!(hex_of([0, 5, 171]), "0005AB");
        // What is written comes back.
        for colour in [[0, 0, 0], [255, 255, 255], [1, 2, 3], [231, 154, 0]] {
            assert_eq!(parse_hex(&hex_of(colour)), Some(colour));
        }
    }

    #[test]
    fn a_point_of_the_wheel_is_a_hue_by_its_angle_and_a_saturation_by_its_distance_and_back() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        // Right is red (hue 0), up (screen y negative) a quarter turn on, left half a turn, down three quarters.
        let (hue, sat) = wheel_to_hs(50.0, 0.0, 100.0);
        assert!(close(hue, 0.0) && close(sat, 0.5), "{hue} {sat}");
        assert!(close(wheel_to_hs(0.0, -100.0, 100.0).0, 0.25));
        assert!(close(wheel_to_hs(-30.0, 0.0, 100.0).0, 0.5));
        assert!(close(wheel_to_hs(0.0, 80.0, 100.0).0, 0.75));
        // Past the rim the saturation stops at 1; the middle is no saturation; a wheel without a radius gives none.
        assert!(close(wheel_to_hs(300.0, 0.0, 100.0).1, 1.0));
        assert_eq!(wheel_to_hs(0.0, 0.0, 100.0).1, 0.0);
        assert_eq!(wheel_to_hs(5.0, 5.0, 0.0).1, 0.0);
        // The way back: every hue and saturation comes back from its place.
        for hue in [0.0, 0.1, 0.25, 0.5, 0.77, 0.99] {
            for saturation in [0.0, 0.3, 1.0] {
                let (dx, dy) = hs_to_wheel(hue, saturation, 90.0);
                let (h, s) = wheel_to_hs(dx, dy, 90.0);
                assert!(close(s, saturation), "{s} vs {saturation}");
                if saturation > 0.0 {
                    assert!(close(h, hue) || close((h - hue).abs(), 1.0), "{h} vs {hue}");
                }
            }
        }
    }

    #[test]
    fn all_different_keeps_the_first_zones_colour_and_gives_every_later_one_a_colour_of_its_own() {
        let fertility = [231, 154, 0];
        assert_eq!(spread(fertility, 0), fertility);
        let colours: Vec<[u8; 3]> = (0..12).map(|index| spread(fertility, index)).collect();
        for (i, a) in colours.iter().enumerate() {
            for b in &colours[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // A golden angle turns the hue: gold becomes cyan-ish, and each zone is well apart from the one before it and from the first.
        for index in 1..12 {
            assert!(hue_gap(colours[index], colours[index - 1]) > 0.08, "zone {index}");
            assert!(hue_gap(colours[index], colours[0]) > 0.05, "zone {index}");
        }
        // The colour's own strength is kept: the zones are as bright and as saturated as the Fertility Color.
        let (_, s0, v0) = rgb_to_hsv(fertility);
        for colour in &colours[1..] {
            let (_, s, v) = rgb_to_hsv(*colour);
            assert!((s - s0).abs() < 0.02 && (v - v0).abs() < 0.02, "{colour:?}");
        }
    }

    #[test]
    fn a_grey_fertility_colour_gives_the_later_zones_bright_colours_and_the_wheel_round_trips() {
        let grey = [90, 90, 90];
        assert_eq!(spread(grey, 0), grey);
        let second = spread(grey, 1);
        assert!(rgb_to_hsv(second).1 > 0.8 && rgb_to_hsv(second).2 >= 0.9, "{second:?}");
        assert_ne!(spread(grey, 2), second);
        // Black has no brightness to keep: the later zones are bright all the same.
        assert!(rgb_to_hsv(spread([0, 0, 0], 1)).2 >= 0.9);
        // The colour wheel round trips on its primaries and on a mixed colour.
        for colour in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [12, 200, 130], [231, 154, 0]] {
            let (h, s, v) = rgb_to_hsv(colour);
            let back = hsv_to_rgb(h, s, v);
            for (a, b) in colour.iter().zip(back) {
                assert!(i32::from(*a).abs_diff(i32::from(b)) <= 1, "{colour:?} came back as {back:?}");
            }
        }
        // A very high zone number still gives a colour (no overflow, no NaN).
        let far = spread([231, 154, 0], usize::MAX);
        assert!(far.iter().any(|channel| *channel > 0));
    }
}
