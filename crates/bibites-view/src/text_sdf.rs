//! Game-exact TMP SDF text rendering.
//!
//! The game draws every UI label with TextMeshPro distance-field atlas text.
//! Parity-dev's runtime reflection dump (`assets/game/ui_fonts/`) provides the
//! per-font face info, character/glyph tables, kerning and material parameters
//! plus the extracted SDF atlas PNGs. This module parses those assets and
//! exposes a small glyph sampler used by the software rasteriser.
//!
//! Coordinates follow Unity's atlas convention (bottom-left origin); glyph
//! metrics are in pixels at `pointSize`, scaled to the requested px size with
//! `px / pointSize`.

use std::collections::HashMap;
use std::path::Path;

use crate::ui::FontRole;

/// One glyph in an SDF font.
#[derive(Clone, Debug)]
pub struct SdfGlyph {
    /// Atlas rectangle (top-left origin after Unity y-flip).
    pub rect: [u32; 4],
    /// Horizontal bearing (px at point size).
    pub bearing_x: f32,
    /// Vertical bearing / glyph top above baseline (px at point size).
    pub bearing_y: f32,
    /// Horizontal advance (px at point size).
    pub advance: f32,
}

/// One SDF font with its atlas.
pub struct SdfFont {
    /// Atlas width in pixels.
    pub atlas_width: u32,
    /// Atlas height in pixels.
    pub atlas_height: u32,
    /// Atlas RGBA pixels.
    pub atlas: Vec<u8>,
    /// Face point size.
    pub point_size: f32,
    /// Face line height (px at point size).
    #[allow(dead_code)] // used once real-advance layout lands
    pub line_height: f32,
    /// Face ascent (px at point size).
    pub ascent: f32,
    /// Face descent (`descentLine`, px at point size, negative below the baseline).
    pub descent: f32,
    /// `_FaceDilate`.
    pub face_dilate: f32,
    /// `_OutlineWidth`.
    pub outline_width: f32,
    /// `_OutlineColor`.
    pub outline_color: [u8; 4],
    /// `_OutlineSoftness`.
    pub outline_softness: f32,
    /// `_GradientScale`.
    pub gradient_scale: f32,
    /// `_ScaleRatioA` (the material's gradient scale less the padding, over the thickest of weight / dilate / outline / softness).
    pub scale_ratio_a: f32,
    /// How far TMP's synthetic bold moves the face edge, in this module's dilation units (`threshold = 0.5 - dilate / 2`): see [`bold_dilate_of`].
    pub bold_dilate: f32,
    /// Glyphs by character.
    pub glyphs: HashMap<char, SdfGlyph>,
    /// The font's pair kerning in ems (`kerning.rs`): added to the first character's advance. Empty for the assets the game gave no pair table
    /// (`Roboto-Regular SDF`, the body face, among them).
    pub kerning: HashMap<(char, char), f32>,
}

impl SdfFont {
    /// Renders one line with a per-pixel callback.
    ///
    /// `pos` is the top-left text origin; `px` the requested pixel size; the
    /// callback receives `(x, y, rgba)` for each covered pixel.
    pub fn draw_line(
        &self,
        text: &str,
        pos: (f32, f32),
        px: f32,
        color: [u8; 4],
        blend: impl FnMut(i64, i64, [u8; 4]),
    ) -> f32 {
        self.draw_line_styled(text, pos, px, color, (false, 0.0), false, blend)
    }

    /// [`SdfFont::draw_line`] with TextMeshPro's synthetic bold when `bold` (a font style on an asset that has no bold face of its own, as
    /// `LiberationSans SDF`): the glyphs are widened by the shader's bold weight and every character advances `BOLD_SPACING` of an em more;
    /// and its italic when `italic`: each glyph's quad is sheared by `ITALIC_SLOPE` around half the cap line (the advances stay).
    #[allow(clippy::too_many_arguments)] // the raster inputs stay explicit
    pub fn draw_line_styled(
        &self,
        text: &str,
        pos: (f32, f32),
        px: f32,
        color: [u8; 4],
        (bold, spacing): (bool, f32),
        italic: bool,
        mut blend: impl FnMut(i64, i64, [u8; 4]),
    ) -> f32 {
        let factor = px / self.point_size.max(1.0);
        let extra_advance = spacing * px;
        let dilate = if bold { self.bold_dilate } else { 0.0 };
        let shear = if italic { ITALIC_SLOPE } else { 0.0 };
        let half_cap = CAP_HEIGHT * px * 0.5;
        let baseline = pos.1 + self.ascent * factor;
        let mut pen = pos.0;
        let mut previous: Option<char> = None;
        for character in text.chars() {
            if let Some(before) = previous {
                pen += self.kerning.get(&(before, character)).copied().unwrap_or(0.0) * px;
            }
            previous = Some(character);
            let Some(glyph) = self.glyphs.get(&character) else {
                pen += px * 0.25;
                continue;
            };
            // TMP's quad is the glyph's metrics box widened by the atlas padding, and an outline's ring (half of it lies outside the glyph) needs that
            // margin. Plain text is drawn inside the box only: widening it for every face measured slightly worse against the game's screens
            // (sum of the 39 menu references' mean differences 341.7 -> 342.9, whatever half-texel shift went with it).
            let pad = if self.outline_width > 0.0 || self.outline_softness > 0.0 {
                (self.gradient_scale - 1.0).max(0.0)
            } else {
                0.0
            };
            let glyph_w = (glyph.rect[2] as f32 + 2.0 * pad) * factor;
            let glyph_h = (glyph.rect[3] as f32 + 2.0 * pad) * factor;
            let gx = pen + (glyph.bearing_x - pad) * factor;
            let gy = baseline - (glyph.bearing_y + pad) * factor;
            let (rect_x, rect_y, rect_w, rect_h) = (
                glyph.rect[0] as f32 - pad,
                glyph.rect[1] as f32 - pad,
                glyph.rect[2] as f32 + 2.0 * pad,
                glyph.rect[3] as f32 + 2.0 * pad,
            );
            if glyph.rect[2] > 0 && glyph.rect[3] > 0 {
                // The shear moves a row by `shear * (height over the baseline - half the cap line)`: the top of the quad right, the bottom left.
                let top_shift = shear * ((glyph.bearing_y + pad) * factor - half_cap);
                let bottom_shift =
                    shear * ((glyph.bearing_y - glyph.rect[3] as f32 - pad) * factor - half_cap);
                let x0 = (gx + top_shift.min(bottom_shift).min(0.0)).floor() as i64;
                let y0 = gy.floor() as i64;
                let x1 = (gx + glyph_w + top_shift.max(bottom_shift).max(0.0)).ceil() as i64;
                let y1 = (gy + glyph_h).ceil() as i64;
                for y in y0..y1 {
                    let v = (y as f32 + 0.5 - gy) / glyph_h;
                    if !(0.0..1.0).contains(&v) {
                        continue;
                    }
                    let sample_y = rect_y + v * rect_h;
                    let row_shift = shear * (baseline - (y as f32 + 0.5) - half_cap);
                    for x in x0..x1 {
                        let u = (x as f32 + 0.5 - gx - row_shift) / glyph_w;
                        if !(0.0..1.0).contains(&u) {
                            continue;
                        }
                        let sample_x = rect_x + u * rect_w;
                        let distance = self.sample_bilinear(sample_x, sample_y);
                        if self.outline_width > 0.0 || self.outline_softness > 0.0 {
                            if let Some(pixel) = self.outlined_pixel(distance, px, dilate, color) {
                                blend(x, y, pixel);
                            }
                            continue;
                        }
                        let alpha = self.coverage_dilated(distance, px, dilate);
                        if alpha <= 0.0 {
                            continue;
                        }
                        let a = (alpha * f32::from(color[3])).round().clamp(0.0, 255.0) as u8;
                        blend(x, y, [color[0], color[1], color[2], a]);
                    }
                }
            }
            pen += glyph.advance * factor + extra_advance;
        }
        pen - pos.0
    }

    /// Bilinear alpha sample from the atlas (clamped).
    fn sample_bilinear(&self, x: f32, y: f32) -> f32 {
        let width = self.atlas_width as i32;
        let height = self.atlas_height as i32;
        if width <= 0 || height <= 0 {
            return 0.0;
        }
        let x = x.clamp(0.0, (width - 1) as f32);
        let y = y.clamp(0.0, (height - 1) as f32);
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = (x0 + 1).min(width - 1);
        let y1 = (y0 + 1).min(height - 1);
        let fx = x - x0 as f32;
        let fy = y - y0 as f32;
        let sample = |sx: i32, sy: i32| -> f32 {
            // Extracted Alpha8 atlases are stored as RGBA with the SDF value in
            // the alpha channel (RGB is zero).
            let index = ((sy * width + sx) * 4 + 3) as usize;
            self.atlas
                .get(index)
                .copied()
                .map_or(0.0, |alpha| f32::from(alpha) / 255.0)
        };
        let top = sample(x0, y0) * (1.0 - fx) + sample(x1, y0) * fx;
        let bottom = sample(x0, y1) * (1.0 - fx) + sample(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// Half the width of the antialiasing ramp, in distance-field units.
    ///
    /// `TextMeshPro/Distance Field` ramps the alpha over `1 / scale` in distance-field units, where `scale = sqrt 2 * gradientScale * (screen
    /// pixels per atlas texel)` (the vertex shader's `scale`) and one texel is `1 / (2 * gradientScale)` of the field: the ramp is
    /// `sqrt 2` pixels wide whatever the size. A ramp with a part that does not shrink as the text grows (the earlier `+ 0.3`) is several
    /// pixels wide on big text and made the title and every large label of Boston Black blurry; and the first version of this width
    /// (`sqrt 2 / 2` pixels each side, twice this) left the small text soft: a live capture of the stem of a 30 px "D" has its edge in one
    /// partial pixel (`[47, 190, 255]` / `[255, 133, 47]` over a 47 background), which this width gives to within a few levels.
    fn ramp_half_width(&self, factor: f32) -> f32 {
        std::f32::consts::FRAC_1_SQRT_2 / (2.0 * self.gradient_scale.max(1.0) * factor)
    }

    /// One pixel of a glyph whose material has an outline or softness (`TextMeshPro/Distance Field`'s `GetColor`), as straight RGBA, or `None`
    /// when nothing is drawn there. With `t` the sample's distance inside the face edge (the field less the weight-moved threshold) and `scale`
    /// the shader's field-to-pixel factor (the ramp is `1 / scale` wide): the face covers `t > -outline / 2`, the outline colour fills `t < outline / 2`
    /// (a band `_OutlineWidth * _ScaleRatioA` of the field wide, centred on the glyph's edge), the face colour what is inside it; the two are
    /// mixed premultiplied and the whole is cut by the face's silhouette.
    fn outlined_pixel(&self, distance: f32, px: f32, extra_dilate: f32, face: [u8; 4]) -> Option<[u8; 4]> {
        let factor = (px / self.point_size.max(1.0)).max(0.01);
        let scale = 1.0 / (2.0 * self.ramp_half_width(factor));
        let ratio = self.scale_ratio_a;
        let weight = self.face_dilate * ratio * 0.5 + extra_dilate * 0.5;
        let t = distance - (0.5 - weight);
        let outline = self.outline_width * ratio * scale;
        let softness = self.outline_softness * ratio * scale;
        let d = 0.5 - t * scale;
        let face_alpha = 1.0 - ((d - outline * 0.5 + softness * 0.5) / (1.0 + softness)).clamp(0.0, 1.0);
        if face_alpha <= 0.0 {
            return None;
        }
        let outline_alpha = (d + outline * 0.5).clamp(0.0, 1.0) * outline.min(1.0).sqrt();
        let premultiplied = |color: [u8; 4]| -> [f32; 4] {
            let a = f32::from(color[3]) / 255.0;
            [
                f32::from(color[0]) / 255.0 * a,
                f32::from(color[1]) / 255.0 * a,
                f32::from(color[2]) / 255.0 * a,
                a,
            ]
        };
        let (face, outline_color) = (premultiplied(face), premultiplied(self.outline_color));
        let mut mixed = [0.0_f32; 4];
        for channel in 0..4 {
            mixed[channel] =
                (face[channel] + (outline_color[channel] - face[channel]) * outline_alpha) * face_alpha;
        }
        if mixed[3] <= 0.0 {
            return None;
        }
        let straight = |value: f32| (value / mixed[3] * 255.0).round().clamp(0.0, 255.0) as u8;
        Some([
            straight(mixed[0]),
            straight(mixed[1]),
            straight(mixed[2]),
            (mixed[3] * 255.0).round().clamp(0.0, 255.0) as u8,
        ])
    }

    /// TMP-style coverage from a distance sample.
    #[allow(dead_code)] // the unstyled path of the tests
    fn coverage(&self, distance: f32, px: f32) -> f32 {
        self.coverage_dilated(distance, px, 0.0)
    }

    /// [`SdfFont::coverage`] with `extra_dilate` more face dilation (the bold weight).
    fn coverage_dilated(&self, distance: f32, px: f32, extra_dilate: f32) -> f32 {
        let factor = (px / self.point_size.max(1.0)).max(0.01);
        // The face dilate (and the bold weight) moves the edge outwards; a material with an outline or softness goes through `outlined_pixel`.
        let threshold = 0.5 - (self.face_dilate + extra_dilate) * 0.5;
        ramp(distance, threshold, self.ramp_half_width(factor))
    }
}

/// A font name with this suffix asks for TextMeshPro's synthetic bold (`FontStyles.Bold` on an asset without a bold face).
pub const BOLD_SUFFIX: &str = " +bold";
/// `TMP_FontAsset.boldSpacing` (7, in hundredths of an em: the advance of every bold character grows by 0.07 em).
pub const BOLD_SPACING: f32 = 0.07;
/// The game's materials all have `_WeightBold` 0.75 (`assets/game/materials/*.json`).
const WEIGHT_BOLD: f32 = 0.75;

/// The bold weight of an asset in the face dilation's units. `TextMeshPro/Distance Field` moves the face threshold by
/// `(_WeightBold / 4 + _FaceDilate) * _ScaleRatioA * 0.5` of the distance field, twice that in this module's units: `0.1875 * _ScaleRatioA`, 0.156 for Boston Black and
/// Roboto-Regular, whose ink then matches the game's (title screen's version label at 28 px: the best of a sweep from 0.05 to 0.25 was 0.15 to 0.16).
/// The `Mobile/Distance Field` shader of `LiberationSans SDF` has the same formula (0.169) but the Biology panel's bold labels, black on the light
/// panel, only reach the game's ink at 0.2 to 0.22 (0.169 leaves them 6 % light; 0.25, the value used before, 5 % heavy): measured, used as is.
fn bold_dilate_of(face_dilate: f32, scale_ratio_a: f32, mobile_shader: bool) -> f32 {
    if mobile_shader {
        return 0.21 + face_dilate;
    }
    (WEIGHT_BOLD / 4.0 + face_dilate) * scale_ratio_a - face_dilate
}

/// A font name with this suffix asks for TextMeshPro's bold weight without its extra advance: the glyphs are widened, the characters keep their
/// regular advances (the gene values of the species panel: a live capture gives a "0.816" the regular 35.27 wide at 14, not 39.19).
pub const BOLD_FLAT_SUFFIX: &str = " +bold0";
/// A font name with this mark and a whole number (thousandths of an em, signed) after it, before the italic suffix, adds that much to the advance of
/// every character: TextMeshPro's character spacing (the inspector's gene values are in the Bold style, +0.07 em, with a spacing of -0.05 em).
pub const SPACING_MARK: &str = " +sp";
/// A font name with this suffix (after the bold one, when both) asks for TextMeshPro's italic style: the glyph quads are sheared.
pub const ITALIC_SUFFIX: &str = " +italic";
/// `TMP_FontAsset.italicStyle` (35, in hundredths): how far a glyph's top moves over its bottom, per unit of height.
pub const ITALIC_SLOPE: f32 = 0.35;
/// Roboto's cap height, in ems (1456 of 2048): TMP shears a glyph around half of the cap line.
const CAP_HEIGHT: f32 = 0.711;

/// A captured font name and whether it carries the bold request.
#[must_use]
pub fn split_bold(name: &str) -> (&str, bool) {
    match name.strip_suffix(BOLD_SUFFIX) {
        Some(plain) => (plain, true),
        None => (name, false),
    }
}

/// A captured font name and the styles it carries: `(font, bold, italic)`.
#[must_use]
pub fn split_style(name: &str) -> (&str, bool, bool) {
    let (plain, bold, italic, _) = split_style_full(name);
    (plain, bold, italic)
}

/// [`split_style`] and what the styles add to every character's advance, in ems: `(font, bold, italic, spacing)` (0.07 for the synthetic bold,
/// whatever a `+sp` mark says on top).
#[must_use]
pub fn split_style_full(name: &str) -> (&str, bool, bool, f32) {
    let (rest, italic) = match name.strip_suffix(ITALIC_SUFFIX) {
        Some(rest) => (rest, true),
        None => (name, false),
    };
    let (rest, spaced) = match rest.rfind(SPACING_MARK) {
        Some(at) => match rest[at + SPACING_MARK.len()..].parse::<i32>() {
            Ok(thousandths) => (&rest[..at], thousandths as f32 / 1000.0),
            Err(_) => (rest, 0.0),
        },
        None => (rest, 0.0),
    };
    if let Some(plain) = rest.strip_suffix(BOLD_FLAT_SUFFIX) {
        return (plain, true, italic, spaced);
    }
    let (plain, bold) = split_bold(rest);
    (plain, bold, italic, spaced + if bold { BOLD_SPACING } else { 0.0 })
}

/// Alpha of a distance sample: 0 below `threshold - half`, 1 above `threshold + half`, linear between
/// (TMP's `1 - saturate(sd)`).
fn ramp(distance: f32, threshold: f32, half: f32) -> f32 {
    if half <= 0.0 {
        return if distance >= threshold { 1.0 } else { 0.0 };
    }
    ((distance - threshold) / (2.0 * half) + 0.5).clamp(0.0, 1.0)
}

/// The fonts the window loaded, for code that builds text before it has any at hand (a label sized to its box when a panel is generated).
static SHARED: std::sync::OnceLock<std::sync::Arc<SdfFonts>> = std::sync::OnceLock::new();

/// Makes `fonts` available to [`shared`] (the first call wins).
pub fn share(fonts: &std::sync::Arc<SdfFonts>) {
    let _ = SHARED.set(std::sync::Arc::clone(fonts));
}

/// The fonts handed to [`share`], if a font set has been loaded.
#[must_use]
pub fn shared() -> Option<&'static SdfFonts> {
    SHARED.get().map(|fonts| &**fonts)
}

/// `TMP_Text`'s auto-sizing (`enableAutoSizing`) of a one-line label: the largest size from `min` to `max` at which the line (`width_at(size)`)
/// fits `available`. TMP halves the distance between a size that fits and one that does not and keeps sizes in steps of 0.05 until the two are within
/// 0.051 of each other; the same search here ends at that size (the game's own labels, 24.6 and 25 for the Balance tab's two longest, are
/// what it gives).
#[must_use]
pub fn fit_size(min: f32, max: f32, available: f32, width_at: impl Fn(f32) -> f32) -> f32 {
    if width_at(max) <= available {
        return max;
    }
    if width_at(min) > available {
        return min;
    }
    let (mut fits, mut overflows) = (min, max);
    while overflows - fits > 0.051 {
        let middle = ((fits + overflows) / 2.0 * 20.0).round() / 20.0;
        if middle <= fits || middle >= overflows {
            break;
        }
        if width_at(middle) <= available {
            fits = middle;
        } else {
            overflows = middle;
        }
    }
    fits
}

/// All SDF fonts loaded from `assets/game/ui_fonts`.
pub struct SdfFonts {
    body: SdfFont,
    bold: SdfFont,
    mono: Option<SdfFont>,
    /// All loaded fonts by captured TMP asset name.
    by_name: std::collections::HashMap<String, SdfFont>,
}

impl SdfFonts {
    /// Discovers and loads the font set relative to the repository root.
    #[must_use]
    pub fn discover(root: &Path) -> Option<Self> {
        let dir = root.join("assets/game/ui_fonts");
        let manifest_path = dir.join("fonts_manifest.json");
        if !manifest_path.is_file() {
            return None;
        }
        let text = std::fs::read_to_string(&manifest_path).ok()?;
        let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
        let fonts = manifest.get("fonts")?.as_array()?;
        let entry = |names: &[&str]| -> Option<&serde_json::Value> {
            fonts.iter().find(|font| {
                names.iter().any(|name| {
                    font.get("name")
                        .and_then(|value| value.as_str())
                        .is_some_and(|value| value == *name)
                })
            })
        };
        let load = |font: &serde_json::Value| -> Option<SdfFont> {
            let json_file = font.get("json")?.as_str()?;
            let atlas_file = font.get("atlas_file")?.as_str()?;
            load_font(&dir.join(json_file), &dir.join(atlas_file))
        };
        let mut by_name = std::collections::HashMap::new();
        for font in fonts {
            if let (Some(name), Some(loaded)) = (
                font.get("name").and_then(serde_json::Value::as_str),
                load(font),
            ) {
                by_name.insert(name.to_owned(), loaded);
            }
        }
        let body = entry(&["Roboto-Regular SDF"]).and_then(load)?;
        let bold =
            entry(&["Roboto-Bold SDF NoOutline", "Roboto-Bold SDF WhiteOutline"]).and_then(load)?;
        let mono = entry(&["Inconsolata-SemiBold SDF", "Inconsolata SDF"])
            .and_then(load)
            .or_else(|| entry(&["LiberationSans SDF"]).and_then(load));
        Some(Self {
            body,
            bold,
            mono,
            by_name,
        })
    }

    /// Font for a UI role.
    #[must_use]
    pub fn font(&self, role: FontRole) -> &SdfFont {
        match role {
            FontRole::Body => &self.body,
            FontRole::Bold => &self.bold,
            FontRole::Mono => self.mono.as_ref().unwrap_or(&self.body),
        }
    }

    /// Font by captured TMP asset name (exact variant).
    #[must_use]
    pub fn font_named(&self, name: &str) -> Option<&SdfFont> {
        self.by_name.get(split_style(name).0)
    }

    /// Line height in pixels for a role.
    #[must_use]
    pub fn line_height(&self, role: FontRole, px: f32) -> f32 {
        let font = self.font(role);
        font.line_height * (px / font.point_size.max(1.0))
    }

    /// Line height in pixels of a captured font name, with role fallback (a font asset has its own: Boston Black's is 1.3 of its size).
    #[must_use]
    pub fn line_height_named(&self, name: Option<&str>, role: FontRole, px: f32) -> f32 {
        let font = name
            .and_then(|name| self.font_named(name))
            .unwrap_or_else(|| self.font(role));
        font.line_height * (px / font.point_size.max(1.0))
    }

    /// What a font's line box has beyond its glyphs' ascent-to-descent extent, in pixels (TMP centres a line on the extent, not on the line height).
    #[must_use]
    pub fn line_gap_named(&self, name: Option<&str>, role: FontRole, px: f32) -> f32 {
        let font = name
            .and_then(|name| self.font_named(name))
            .unwrap_or_else(|| self.font(role));
        (font.line_height - (font.ascent - font.descent)) * (px / font.point_size.max(1.0))
    }

    /// Measures a line using a captured font name with role fallback.
    #[must_use]
    pub fn measure_named(&self, name: Option<&str>, role: FontRole, text: &str, px: f32) -> f32 {
        let font = name
            .and_then(|name| self.font_named(name))
            .unwrap_or_else(|| self.font(role));
        let factor = px / font.point_size.max(1.0);
        let extra = name.map_or(0.0, |name| split_style_full(name).3 * px);
        let mut previous: Option<char> = None;
        text.chars()
            .map(|character| {
                let kern = previous.map_or(0.0, |before| {
                    font.kerning.get(&(before, character)).copied().unwrap_or(0.0) * px
                });
                previous = Some(character);
                kern + font
                    .glyphs
                    .get(&character)
                    .map_or(px * 0.25, |glyph| glyph.advance * factor + extra)
            })
            .sum()
    }

    /// Measures a line with real glyph advances.
    #[allow(dead_code)] // wired into RuntimeUi layout next
    #[must_use]
    pub fn measure(&self, role: FontRole, text: &str, px: f32) -> f32 {
        let font = self.font(role);
        let factor = px / font.point_size.max(1.0);
        let mut previous: Option<char> = None;
        text.chars()
            .map(|character| {
                let kern = previous.map_or(0.0, |before| {
                    font.kerning.get(&(before, character)).copied().unwrap_or(0.0) * px
                });
                previous = Some(character);
                kern + font
                    .glyphs
                    .get(&character)
                    .map_or(px * 0.25, |glyph| glyph.advance * factor)
            })
            .sum()
    }
}

/// Loads one font JSON plus its atlas PNG.
fn load_font(json_path: &Path, atlas_path: &Path) -> Option<SdfFont> {
    let text = std::fs::read_to_string(json_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let face = value.get("faceInfo")?;
    let point_size = face
        .get("pointSize")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(36.0) as f32;
    let line_height = face
        .get("lineHeight")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(point_size as f64 * 1.2) as f32;
    let ascent = face
        .get("ascentLine")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(point_size as f64 * 0.8) as f32;
    let descent = face
        .get("descentLine")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(point_size as f64 * -0.2) as f32;
    let material = value.get("material").cloned().unwrap_or_default();
    let material_f32 = |key: &str, fallback: f32| -> f32 {
        material
            .get(key)
            .and_then(serde_json::Value::as_f64)
            .map_or(fallback, |value| value as f32)
    };
    let atlas = decode_png(atlas_path)?;
    let outline_color = material
        .get("_OutlineColor")
        .map_or([255, 255, 255, 255], |color| {
            let channel = |key: &str| -> u8 {
                (color
                    .get(key)
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(1.0)
                    * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            [channel("r"), channel("g"), channel("b"), channel("a")]
        });
    // Character table: unicode -> glyph index; glyph table: glyph index -> metrics.
    let mut glyph_by_index: HashMap<String, serde_json::Value> = HashMap::new();
    for glyph in value.get("glyphTable")?.as_array()? {
        if let Some(index) = glyph.get("glyphIndex").and_then(serde_json::Value::as_str) {
            glyph_by_index.insert(index.to_owned(), glyph.clone());
        }
    }
    let mut glyphs = HashMap::new();
    for entry in value.get("characterTable")?.as_array()? {
        let Some(unicode) = entry
            .get("unicode")
            .and_then(|value| value.as_str().or_else(|| value.as_u64().map(|_| "")))
        else {
            continue;
        };
        // `unicode` is a decimal string in the dump.
        let Ok(code) = unicode.parse::<u32>() else {
            continue;
        };
        let Some(character) = char::from_u32(code) else {
            continue;
        };
        let Some(glyph_index) = entry.get("glyphIndex").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(glyph) = glyph_by_index.get(glyph_index) else {
            continue;
        };
        let Some(rect) = glyph.get("atlasRect") else {
            continue;
        };
        let Some(metrics) = glyph.get("metrics") else {
            continue;
        };
        let rect_w = rect
            .get("w")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let rect_h = rect
            .get("h")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let rect_x = rect
            .get("x")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        let rect_y = rect
            .get("y")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u32;
        // Unity atlas coordinates are bottom-left; convert to top-left.
        let top = atlas.1.saturating_sub(rect_y + rect_h);
        let metric = |key: &str| -> f32 {
            metrics
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0) as f32
        };
        glyphs.insert(
            character,
            SdfGlyph {
                rect: [rect_x, top, rect_w, rect_h],
                bearing_x: metric("bearingX"),
                bearing_y: metric("bearingY"),
                advance: metric("advance"),
            },
        );
    }
    if glyphs.is_empty() {
        return None;
    }
    let (width, height, rgba) = atlas;
    let kerning = crate::kerning::pairs_for_dump(json_path);
    Some(SdfFont {
        atlas_width: width,
        atlas_height: height,
        atlas: rgba,
        point_size,
        line_height,
        ascent,
        descent,
        face_dilate: material_f32("_FaceDilate", 0.0),
        outline_width: material_f32("_OutlineWidth", 0.0),
        outline_color,
        outline_softness: material_f32("_OutlineSoftness", 0.0),
        gradient_scale: material_f32("_GradientScale", 6.0),
        scale_ratio_a: material_f32("_ScaleRatioA", 1.0),
        bold_dilate: bold_dilate_of(
            material_f32("_FaceDilate", 0.0),
            material_f32("_ScaleRatioA", 1.0),
            material
                .get("shader")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|shader| shader.contains("Mobile")),
        ),
        glyphs,
        kerning,
    })
}

/// Decodes an SDF atlas PNG into `(width, height, RGBA)`.
fn decode_png(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0_u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    let raw = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => raw
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => raw
            .iter()
            .flat_map(|&value| [value, value, value, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => raw
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some((info.width, info.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font with the title font's numbers (Boston Black: 68 points, gradient scale 6).
    fn boston() -> SdfFont {
        SdfFont {
            atlas_width: 0,
            atlas_height: 0,
            atlas: Vec::new(),
            point_size: 68.0,
            line_height: 88.4,
            ascent: 68.0,
            descent: -16.0,
            face_dilate: 0.0,
            outline_width: 0.0,
            outline_color: [0, 0, 0, 0],
            outline_softness: 0.0,
            gradient_scale: 6.0,
            scale_ratio_a: 0.8333,
            bold_dilate: bold_dilate_of(0.0, 0.8333, false),
            glyphs: HashMap::new(),
            kerning: HashMap::new(),
        }
    }

    /// Screen pixels the antialiased edge spans at text size `px` (`TMP_SDF.shader`: the field changes
    /// by `1 / (2 * gradientScale)` per texel, as the atlases show, so a ramp `w` wide covers
    /// `w * 2 * gradientScale` texels of `factor` pixels each): `sqrt 2` at every size.
    fn edge_pixels(font: &SdfFont, px: f32) -> f32 {
        let factor = px / font.point_size;
        2.0 * font.ramp_half_width(factor) * 2.0 * font.gradient_scale * factor
    }

    #[test]
    fn an_outlined_material_has_its_ring_on_the_edge_and_the_face_inside() {
        // `Roboto-Bold SDF WhiteOutline`: outline 0.3, white; a dark face.
        let mut font = boston();
        font.outline_width = 0.3;
        font.outline_color = [255, 255, 255, 255];
        let (px, face) = (24.0, [40, 40, 40, 255]);
        // On the glyph's edge (field 0.5) the ring's colour dominates (TMP scales the ring's opacity by `sqrt(min(1, width))`, here 0.75).
        let edge = font.outlined_pixel(0.5, px, 0.0, face).expect("the edge");
        assert!(edge[0] > 180 && edge[3] > 200, "{edge:?}");
        // Well inside, the face colour; well outside, nothing.
        assert_eq!(font.outlined_pixel(0.9, px, 0.0, face), Some([40, 40, 40, 255]));
        assert_eq!(font.outlined_pixel(0.1, px, 0.0, face), None);
        // The ring is half outside the edge: a sample a little outside (field 0.4) is still the outline's, partly covered.
        let outside = font.outlined_pixel(0.42, px, 0.0, face).expect("ring outside the edge");
        assert!(outside[0] > 200 && outside[3] < 255, "{outside:?}");
    }

    #[test]
    fn the_bold_weight_follows_the_materials_scale_ratio() {
        // `_WeightBold` 0.75, `_ScaleRatioA` 0.8333 (gradient scale 6): TMP moves the threshold by 0.75 / 4 * 0.8333 * 0.5 of the field.
        assert!((bold_dilate_of(0.0, 0.8333, false) - 0.15625).abs() < 1e-3);
        // Gradient scale 10 (ratio 0.9) would be 0.169; the Liberation (mobile shader) face is measured at 0.21.
        assert!((bold_dilate_of(0.0, 0.9, false) - 0.16875).abs() < 1e-3);
        assert!((bold_dilate_of(0.0, 0.9, true) - 0.21).abs() < 1e-6);
        assert!((boston().bold_dilate - 0.15625).abs() < 1e-3);
    }

    #[test]
    fn the_edge_is_about_a_pixel_wide_at_every_size_and_the_title_is_not_blurry() {
        let font = boston();
        for px in [15.0, 28.0, 40.0, 81.0, 98.7, 200.0] {
            let width = edge_pixels(&font, px);
            assert!(
                (1.0..=2.5).contains(&width),
                "{px} px text has a {width} px edge"
            );
        }
        // Between the same two samples a big title goes from nothing to full ink in as few
        // pixels as a small label does (it used to spread over several).
        assert!((edge_pixels(&font, 81.0) - edge_pixels(&font, 15.0)).abs() < 0.01);
    }

    #[test]
    fn the_coverage_is_half_at_the_edge_and_ramps_from_nothing_to_full() {
        let font = boston();
        let px = 40.0;
        assert!((font.coverage(0.5, px) - 0.5).abs() < 1e-4);
        assert_eq!(font.coverage(0.0, px), 0.0);
        assert_eq!(font.coverage(1.0, px), 1.0);
        let (a, b, c) = (
            font.coverage(0.45, px),
            font.coverage(0.5, px),
            font.coverage(0.55, px),
        );
        assert!(a < b && b < c, "{a} {b} {c}");
        // The ramp is linear: the middle of the ramp is the edge.
        let half = font.ramp_half_width(px / font.point_size);
        assert!((font.coverage(0.5 + half, px) - 1.0).abs() < 1e-4);
        assert!(font.coverage(0.5 - half, px).abs() < 1e-4);
        // A glyph dilated outwards covers more at the same sample.
        let mut bold = boston();
        bold.face_dilate = 0.2;
        assert!(bold.coverage(0.45, px) > font.coverage(0.45, px));
    }

    #[test]
    fn the_character_spacing_mark_adds_to_the_styles_advance() {
        let (name, bold, italic, spacing) = split_style_full("Roboto-Regular SDF +bold +sp-50");
        assert_eq!((name, bold, italic), ("Roboto-Regular SDF", true, false));
        assert!((spacing - 0.02).abs() < 1e-6, "{spacing}");
        let (name, bold, _, spacing) = split_style_full("Roboto-Regular SDF +sp-50");
        assert_eq!((name, bold), ("Roboto-Regular SDF", false));
        assert!((spacing + 0.05).abs() < 1e-6, "{spacing}");
        assert_eq!(split_style_full("Roboto-Regular SDF +bold0"), ("Roboto-Regular SDF", true, false, 0.0));
        assert_eq!(
            split_style_full("LiberationSans SDF +bold +italic"),
            ("LiberationSans SDF", true, true, BOLD_SPACING)
        );
        // A name that merely contains the mark text is not spaced.
        assert_eq!(split_style_full("Roboto +spx"), ("Roboto +spx", false, false, 0.0));
    }

    #[test]
    fn sdf_fonts_load_when_extracted() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        let Some(fonts) = SdfFonts::discover(&root) else {
            eprintln!("SDF font assets not present; skipping");
            return;
        };
        let width = fonts.measure(FontRole::Body, "The Bibites", 20.0);
        assert!(width > 10.0, "measured {width}");
        let font = fonts.font(FontRole::Body);
        assert!(font.glyphs.len() > 20);
        assert!(!font.atlas.is_empty());
    }

    #[test]
    fn auto_sized_labels_get_the_sizes_the_game_chose() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let Some(fonts) = SdfFonts::discover(&root) else {
            eprintln!("SDF font assets not present; skipping");
            return;
        };
        // The Balance tab's row labels (`SettingSlider/Name`: `Roboto-Bold SDF NoOutline`, 25 at most, a 343.5 wide box with a margin of 10 on the left):
        // the game's own `font_size` of the two longest, read from its live UI dump.
        let font = Some("Roboto-Bold SDF NoOutline");
        for (text, game) in [("Muscles Sizing Turning Power", 24.6_f32), ("Biting Repulsion Force Factor", 25.0), ("Arm Muscles Force", 25.0)] {
            let size = fit_size(10.0, 25.0, 333.5, |size| fonts.measure_named(font, FontRole::Bold, text, size));
            assert!((size - game).abs() <= 0.1, "{text}: {size}, the game's is {game}");
        }
        // The line TMP lays out at the game's size is what its `line0_length` says (332.85 and 330.16).
        let width = fonts.measure_named(font, FontRole::Bold, "Muscles Sizing Turning Power", 24.6);
        assert!((width - 332.85).abs() < 1.0, "{width}");
    }

    #[test]
    fn a_bold_font_name_adds_the_bold_spacing_to_every_character() {
        assert_eq!(split_bold("LiberationSans SDF +bold"), ("LiberationSans SDF", true));
        assert_eq!(split_bold("LiberationSans SDF"), ("LiberationSans SDF", false));
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
        let Some(fonts) = SdfFonts::discover(&root) else {
            eprintln!("SDF font assets not present; skipping");
            return;
        };
        let (plain, bold) = ("LiberationSans SDF", "LiberationSans SDF +bold");
        let regular = fonts.measure_named(Some(plain), FontRole::Body, "Species:", 26.0);
        let heavy = fonts.measure_named(Some(bold), FontRole::Body, "Species:", 26.0);
        // 8 characters, 0.07 em each: the capture's "Species:" at 26 is 99.77 regular and 112.51 bold (the last character's spacing is not part
        // of the width TMP lays out with, which the Stats layout measures separately).
        assert!((heavy - regular - 8.0 * BOLD_SPACING * 26.0).abs() < 0.01, "regular {regular}, bold {heavy}");
        assert!((regular - 99.77).abs() < 0.5, "regular {regular}");
        // The named font is found through the suffix.
        assert!(fonts.font_named(bold).is_some());
    }
}
