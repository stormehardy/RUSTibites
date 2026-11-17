//! The game's skin and eye shader (`Shader Graphs/proceduralBibiteShader` and `proceduralEyeShader`), as far as colour goes.
//!
//! The extracted part art is one blue hue (0.549) whose shading is in its saturation and value; the shaders recolour it with
//! the bibite's colour. The maths below was read from the shaders' compiled DXBC in the installed game (`tools/unity/shader_asm.py`
//! dumps and disassembles it) and checked against the game's own captures (`assets/game/reference/default/bibite_closeup.png`,
//! the lol save's `bibite_closeup.png`): every colour of those pictures is reproduced to one level of 255.
//!
//! For a texel `(h_t, s_t, v_t)` and the material colour `(h_c, s_c, v_c)` (`_Color`, the body colour of the genes):
//!
//! ```text
//! hue        = h_t - FromHue + h_c
//! saturation = s_c * s_t
//! widened    = VmaxAtS0 + s_c * (1 - VmaxAtS0)
//! shade      = clamp(v_c * (widened - VminAtS0) + VminAtS0, min(widened, VminAtS0), max(widened, VminAtS0))
//! value      = v_t * (1 - s_t * (1 - shade))
//! ```
//!
//! `FromHue`, `VminAtS0` and `VmaxAtS0` are the shader globals `ProceduralSpriteManager.LoadParametersCSV` sets from
//! `parameters.csv` (0.55, 0 and 0.75 in the shipped game). A grey body colour (`s_c = 0`, a basic bibite's 0.9 grey) gives a
//! grey picture whose dark texels are darker than the art's value (`shade` is 0.675 for 0.9 grey): that is the contrast of the
//! game's bibites, which a plain hue replacement misses (the darkest body level is 100, not 142).
//!
//! The same shader also draws the virus pattern of a radioactive bibite (`_InfectedRatio`, `_VirusColor`), the rainbow of an
//! `isRain` bibite, the producer pattern, the reaper's black colour (`_IsKiller`) and the red flash of a hurt bibite
//! (`_HurtAmount`); none of those is modelled here.

use crate::assets::{Parameters, hsv_to_rgb, rgb_to_hsv};

/// The shader globals of the skin and eye shaders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinParams {
    /// `_FromHue` (`InitialHue`): the hue of the art, subtracted from the texel's hue.
    pub from_hue: f32,
    /// `_VminAtS0` (`VminAtS0`).
    pub vmin_at_s0: f32,
    /// `_VmaxAtS0` (`VmaxAtS0`).
    pub vmax_at_s0: f32,
    /// The shader's `_Time` of the frame (game time in seconds: the rainbow and the virus move with it).
    pub time: f32,
}

impl Default for SkinParams {
    /// The shipped `parameters.csv` (`InitialHue 0,55`, `VminAtS0 0`, `VmaxAtS0 0,75`).
    fn default() -> Self {
        Self {
            from_hue: 0.55,
            vmin_at_s0: 0.0,
            vmax_at_s0: 0.75,
            time: 0.0,
        }
    }
}

impl From<&Parameters> for SkinParams {
    fn from(parameters: &Parameters) -> Self {
        Self {
            from_hue: parameters.initial_hue,
            vmin_at_s0: parameters.vmin_at_s0,
            vmax_at_s0: parameters.vmax_at_s0,
            time: 0.0,
        }
    }
}

/// A material colour prepared for shading many texels (the colour's HSV and its `shade` factor are the same for a whole sprite).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinTint {
    hue: f32,
    saturation: f32,
    shade: f32,
    from_hue: f32,
}

impl SkinTint {
    /// Prepares the material colour `color` (RGB in `0..=1`).
    #[must_use]
    pub fn new(color: [f32; 3], params: SkinParams) -> Self {
        let (hue, saturation, value) = rgb_to_hsv(color);
        let widened = params.vmax_at_s0 + saturation * (1.0 - params.vmax_at_s0);
        let lifted = value * (widened - params.vmin_at_s0) + params.vmin_at_s0;
        let low = widened.min(params.vmin_at_s0);
        let high = widened.max(params.vmin_at_s0);
        Self {
            hue,
            saturation,
            shade: lifted.max(low).min(high),
            from_hue: params.from_hue,
        }
    }

    /// One texel of the art (RGB in `0..=1`) through the shader.
    #[must_use]
    pub fn apply(&self, texel: [f32; 3]) -> [f32; 3] {
        let (texel_hue, texel_saturation, texel_value) = rgb_to_hsv(texel);
        hsv_to_rgb(
            texel_hue - self.from_hue + self.hue,
            self.saturation * texel_saturation,
            texel_value * (1.0 - texel_saturation * (1.0 - self.shade)),
        )
    }
}

/// One texel through the shader with the material colour `color` (see the module documentation).
#[cfg(test)]
#[must_use]
pub fn shade(texel: [f32; 3], color: [f32; 3], params: SkinParams) -> [f32; 3] {
    SkinTint::new(color, params).apply(texel)
}

/// The colour the eye shader works with (`proceduralEyeShader` before the texel is read): the hue of the material colour
/// `body` moved by `eye_offset` (`_hueShift`, the `EyeOffset` gene), and its saturation and value pushed towards 1 by
/// `saturation_override` (`_saturationOverride`: 1 in the material asset, so a living bibite's eyes are fully saturated and
/// bright, `GetEyeColor`; a corpse lowers it to `1 - decay`).
#[must_use]
pub fn eye_shader_color(body: [f32; 3], eye_offset: f32, saturation_override: f32) -> [f32; 3] {
    let (hue, saturation, value) = rgb_to_hsv(body);
    hsv_to_rgb(
        hue - eye_offset,
        saturation + saturation_override * (1.0 - saturation),
        value + saturation_override * (1.0 - value),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(color: [f32; 3]) -> [i32; 3] {
        color.map(|channel| (channel * 255.0).round() as i32)
    }

    fn unit(color: [u8; 3]) -> [f32; 3] {
        color.map(|channel| f32::from(channel) / 255.0)
    }

    fn within(actual: [i32; 3], expected: [i32; 3], tolerance: i32) -> bool {
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() <= tolerance)
    }

    /// The eight colours of the extracted `normal_body` sheet.
    const BODY_ART: [[u8; 3]; 8] = [
        [14, 104, 142],
        [24, 127, 170],
        [138, 174, 189],
        [70, 160, 198],
        [95, 181, 217],
        [162, 201, 217],
        [188, 215, 227],
        [126, 204, 236],
    ];

    #[test]
    fn a_basic_bibites_body_has_the_grey_levels_of_the_games_capture() {
        // `assets/game/reference/default/bibite_closeup.png` (the game, a new Default world): the body's grey levels.
        let basic = crate::assets::genes_to_color(0.1, 0.1, 0.1);
        let expected = [100, 123, 172, 156, 177, 199, 214, 200];
        for (art, expected) in BODY_ART.iter().zip(expected) {
            let grey = bytes(shade(unit(*art), basic, SkinParams::default()));
            assert!(
                within(grey, [expected; 3], 1),
                "art {art:?}: {grey:?}, the game shows {expected}"
            );
        }
    }

    #[test]
    fn a_basic_bibites_eyes_have_the_colours_of_the_games_capture() {
        let basic = crate::assets::genes_to_color(0.1, 0.1, 0.1);
        let eye = eye_shader_color(basic, 0.5, 1.0);
        assert_eq!(bytes(eye), [0, 255, 255], "cyan, the body hue minus 0.5");
        // The eye art `normal_eyes` cell (16, 9, 8, 9) and the game's picture of it.
        for (art, expected) in [
            ([70, 160, 198], [70, 198, 198]),
            ([126, 204, 236], [126, 236, 235]),
            ([24, 127, 170], [24, 170, 169]),
        ] {
            let shown = bytes(shade(unit(art), eye, SkinParams::default()));
            assert!(within(shown, expected, 1), "art {art:?}: {shown:?}");
        }
    }

    #[test]
    fn a_coloured_bibite_has_the_colours_of_the_games_capture() {
        // The lol save's first bibite (genes 0.483 / 0.029 / 0.921, EyeOffset 0.194) in the game's closeup.
        let body = crate::assets::genes_to_color(0.483, 0.029, 0.921);
        for (art, expected) in [
            ([14, 104, 142], [80, 23, 136]),
            ([24, 127, 170], [99, 34, 163]),
            ([138, 174, 189], [164, 140, 186]),
            ([70, 160, 198], [136, 78, 192]),
            ([95, 181, 217], [157, 102, 211]),
            ([162, 201, 217], [189, 164, 214]),
            ([188, 215, 227], [208, 190, 225]),
        ] {
            let shown = bytes(shade(unit(art), body, SkinParams::default()));
            assert!(within(shown, expected, 1), "body {art:?}: {shown:?}");
        }
        let eye = eye_shader_color(body, 0.194, 1.0);
        for (art, expected) in [
            ([95, 181, 217], [95, 176, 217]),
            ([126, 204, 236], [126, 199, 236]),
            ([70, 160, 198], [70, 154, 198]),
            ([24, 127, 170], [24, 121, 170]),
        ] {
            let shown = bytes(shade(unit(art), eye, SkinParams::default()));
            assert!(within(shown, expected, 1), "eye {art:?}: {shown:?}");
        }
    }

    #[test]
    fn a_fully_saturated_bright_colour_keeps_the_arts_saturation_and_value() {
        // The eye colour: `shade` is 1, so only the hue changes (the old hue replacement was exact for eyes).
        let eye = eye_shader_color([0.3, 0.2, 0.9], 0.1, 1.0);
        let (_, saturation, value) = rgb_to_hsv(eye);
        assert!((saturation - 1.0).abs() < 1e-5 && (value - 1.0).abs() < 1e-5);
        let art = unit([70, 160, 198]);
        let (_, art_saturation, art_value) = rgb_to_hsv(art);
        let (_, out_saturation, out_value) = rgb_to_hsv(shade(art, eye, SkinParams::default()));
        assert!((out_saturation - art_saturation).abs() < 2e-3);
        assert!((out_value - art_value).abs() < 2e-3);
    }

    #[test]
    fn a_corpses_eyes_fade_to_the_body_colour() {
        // `SetCorpseDecay`: the eye colour is the body's hue at saturation 0.01 and its value, the override is `1 - decay`.
        let body = crate::assets::genes_to_color(0.483, 0.029, 0.921);
        let (hue, _, value) = rgb_to_hsv(body);
        let flat = hsv_to_rgb(hue, 0.01, value);
        let fresh = eye_shader_color(flat, 0.194, 1.0);
        let (_, s, v) = rgb_to_hsv(fresh);
        assert!((s - 1.0).abs() < 1e-5 && (v - 1.0).abs() < 1e-5, "{s} {v}");
        let rotten = eye_shader_color(flat, 0.194, 0.0);
        let (rotten_hue, s, v) = rgb_to_hsv(rotten);
        assert!((s - 0.01).abs() < 1e-3, "{s}");
        assert!((v - value).abs() < 1e-3, "{v}");
        let (expected_hue, _, _) = rgb_to_hsv(hsv_to_rgb(hue - 0.194, 0.01, value));
        assert!((rotten_hue - expected_hue).abs() < 1e-3);
    }
}

/// What the skin shader draws on top of the plain recolour, per bibite (`_HurtAmount`, `_IsKiller`, `_InfectedRatio` + `_VirusColor`, `_IsRainbow`,
/// `_IsProducer` and the `_RootPos` / `_Dir` / `_upp` / `_BodyWidth` its patterns are laid out with).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinEffects {
    /// `_HurtAmount` (`damagedFlash`).
    pub hurt: f32,
    /// `_IsKiller` (a reaper).
    pub killer: f32,
    /// `_InfectedRatio` (1.25 for a radioactive bibite, else 0).
    pub infected: f32,
    /// `_VirusColor` (`RadioZone.color`).
    pub virus: [f32; 3],
    /// `_IsRainbow`.
    pub rainbow: bool,
    /// `_IsProducer`.
    pub producer: bool,
    /// `_RootPos` (the bibite's place) and `_Dir` (its up vector).
    pub root: [f32; 2],
    /// See `root`.
    pub dir: [f32; 2],
    /// `_upp` (`d1Size / ppu`: world units per art pixel).
    pub upp: f32,
    /// `_BodyWidth` (the body sprite's height plus two arm sprites').
    pub body_width: f32,
}

impl SkinEffects {
    /// Nothing but the plain recolour.
    pub const NONE: Self = Self {
        hurt: 0.0,
        killer: 0.0,
        infected: 0.0,
        virus: [0.0; 3],
        rainbow: false,
        producer: false,
        root: [0.0; 2],
        dir: [0.0, 1.0],
        upp: 1.0,
        body_width: 1.0,
    };

    /// Whether the patterns that need the whole pixel program (virus, rainbow, producer) are on.
    #[must_use]
    pub fn needs_program(&self) -> bool {
        self.infected != 0.0 || self.rainbow || self.producer
    }
}

impl Default for SkinEffects {
    fn default() -> Self {
        Self::NONE
    }
}
