//! The Bibite Editor's "Save sprite" button (`BibiteEditorPanel.SaveSpriteOfBibite` -> `SpriteGenerator.SaveSpriteOfPreview` -> `BuildAndSaveSprite`): the
//! picture of the previewer's five sprites (body, mouth, arm, exoskeleton, eyes) laid out on one sheet and magnified by the `Output Pixel Ratio` (10).
//!
//! The sheet is `body width + mouth width` wide and `body height + 2 arm heights` tall with no padding: the body at the left with the arm above it and the
//! arm turned upside down below it (both ending at the body's right edge), the mouth to the right of the body in the middle, the exoskeleton over the body's
//! place and the eyes at the right edge, both blended over the rest. Each sprite goes through the skin / eye shader with the bibite's colours first and keeps
//! its own alpha; the empty parts of the first three are the background colour (transparent for the editor's button). The game asks for the file with the
//! system's save dialog; this port writes `bibite_yyyyMMddHHmmss.png` to the sprite folder of the settings or to the data folder.

use bibites_core::genome::Genome;
use bibites_core::math::Vec2;

use crate::assets::{GameAssets, SpriteRef};
use crate::render::{Camera, Primitive, Scene, render_scene};

/// `SpriteGeneratorSettings.ImageRatio`'s default.
pub const PIXEL_RATIO: u32 = 10;

/// One sprite's pixels (RGBA, top row first) after the skin shader.
struct Part {
    width: usize,
    height: usize,
    rgba: Vec<[u8; 4]>,
}

fn render_part(assets: &GameAssets, sprite: SpriteRef, tint: [u8; 4], hue_shift: bool) -> Part {
    let size = sprite.world_size();
    let (width, height) = (sprite.rect[2] as usize, sprite.rect[3] as usize);
    // The camera looks at the middle of the sprite's rectangle: with the pivot at `pivot`, that is (0.5 - pivot.x, pivot.y - 0.5) sprite sizes from the
    // anchor, and the sprite is drawn at one pixel per sprite pixel.
    let anchor = Vec2::ZERO;
    let center = Vec2::new((0.5 - sprite.pivot.x) * size.x, (sprite.pivot.y - 0.5) * size.y);
    let camera = Camera { center, zoom: sprite.ppu, screen: Vec2::new(width as f32, height as f32) };
    let scene = Scene {
        background: [0, 0, 0, 0],
        primitives: vec![Primitive::Sprite {
            sprite,
            anchor,
            size,
            pivot: sprite.pivot,
            rotation: 0.0,
            tint,
            flip_x: false,
            flip_y: false,
            hue_shift,
            effects: crate::skin::SkinEffects::NONE,
            warp: None,
        }],
        ..Scene::new()
    };
    let image = render_scene(&scene, &camera, width as u32, height as u32, 1, &assets.atlases);
    Part {
        width,
        height,
        rgba: image.rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect(),
    }
}

fn rgba(color: [f32; 3]) -> [u8; 4] {
    [
        (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        255,
    ]
}

/// Blends `src` over `dst` (straight alpha).
fn over(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let a = f32::from(src[3]) / 255.0;
    let b = f32::from(dst[3]) / 255.0;
    let out = a + b * (1.0 - a);
    if out <= 0.0 {
        return [0, 0, 0, 0];
    }
    let mix = |s: u8, d: u8| ((f32::from(s) * a + f32::from(d) * b * (1.0 - a)) / out).round() as u8;
    [mix(src[0], dst[0]), mix(src[1], dst[1]), mix(src[2], dst[2]), (out * 255.0).round() as u8]
}

/// The sheet of a genome as `(width, height, RGBA)`, at one pixel per sprite pixel, for the sprite tier `size_index`.
#[must_use]
pub fn sheet(assets: &GameAssets, genome: &Genome, size_index: usize) -> Option<(usize, usize, Vec<[u8; 4]>)> {
    let genes = genome.values();
    let total = crate::assets::total_organ_wag(genes);
    let share = |gene: f32| if total > 0.0 { gene.max(0.0) / total } else { 0.0 };
    let strength = share(genes[31]).sqrt();
    let defence = share(genes[29]);
    let hue_shift = assets.tint_mode == crate::assets::TintMode::HueShift;
    let skin = rgba(crate::assets::body_color(genes));
    let eye = rgba(crate::assets::eye_color(genes));
    let body = render_part(assets, assets.body_sprite(size_index, 0)?, skin, hue_shift);
    let mouth = render_part(assets, assets.mouth_sprite(size_index, genes[16], strength)?, skin, hue_shift);
    let arm = render_part(assets, assets.arm_sprite(size_index, genes[4])?, skin, hue_shift);
    let exo = render_part(assets, assets.exoskeleton_sprite(size_index, 0, defence)?, skin, hue_shift);
    let eyes = render_part(assets, assets.eye_sprite(size_index, genes[13], genes[12], false)?, eye, hue_shift);
    let width = body.width + mouth.width;
    let height = body.height + 2 * arm.height;
    let mut canvas = vec![[0u8, 0, 0, 0]; width * height];
    // Unity's rows count from the bottom; here from the top, so a part whose bottom is at `y` has its top row at `height - y - part.height`.
    let put = |canvas: &mut Vec<[u8; 4]>, part: &Part, x: usize, bottom: usize, flip: bool, blend: bool| {
        let top = height.saturating_sub(bottom + part.height);
        for py in 0..part.height {
            let source_row = if flip { part.height - 1 - py } else { py };
            for px in 0..part.width {
                let pixel = part.rgba[source_row * part.width + px];
                let at = (top + py) * width + x + px;
                if at >= canvas.len() || x + px >= width {
                    continue;
                }
                if blend {
                    canvas[at] = over(canvas[at], pixel);
                } else if pixel[3] >= 3 {
                    canvas[at] = pixel;
                }
            }
        }
    };
    put(&mut canvas, &body, 0, arm.height, false, false);
    put(&mut canvas, &mouth, body.width, (height - mouth.height) / 2, false, false);
    put(&mut canvas, &arm, body.width - arm.width, body.height + arm.height, false, false);
    put(&mut canvas, &arm, body.width - arm.width, 0, true, false);
    put(&mut canvas, &exo, 0, (height - exo.height) / 2, false, true);
    put(&mut canvas, &eyes, width - eyes.width, (height - eyes.height) / 2, false, true);
    Some((width, height, canvas))
}

/// The sheet magnified `ratio` times (`ScaleTexture2D`: every pixel becomes a `ratio` square) as a PNG.
#[must_use]
pub fn png(assets: &GameAssets, genome: &Genome, size_index: usize, ratio: u32) -> Option<Vec<u8>> {
    let (width, height, pixels) = sheet(assets, genome, size_index)?;
    let ratio = ratio.max(1) as usize;
    let (out_w, out_h) = (width * ratio, height * ratio);
    let mut rgba = Vec::with_capacity(out_w * out_h * 4);
    for y in 0..out_h {
        for x in 0..out_w {
            rgba.extend_from_slice(&pixels[(y / ratio) * width + x / ratio]);
        }
    }
    crate::render::Image { width: out_w as u32, height: out_h as u32, rgba }.to_png_bytes().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_lays_the_five_sprites_out_as_the_game_does() {
        let Some(assets) = GameAssets::discover().ok().flatten() else {
            eprintln!("skipping: no game assets");
            return;
        };
        let genome = Genome::defaults();
        let index = assets.closest_part_size_index(1.0);
        let (width, height, pixels) = sheet(&assets, &genome, index).expect("a sheet");
        let body = assets.body_sprite(index, 0).expect("body").rect;
        let mouth = assets.mouth_sprite(index, 0.5, 0.0).expect("mouth").rect;
        let arm = assets.arm_sprite(index, 0.5).expect("arm").rect;
        assert_eq!(width as u32, body[2] + mouth[2]);
        assert_eq!(height as u32, body[3] + 2 * arm[3]);
        assert!(pixels.iter().any(|p| p[3] == 255), "something is drawn");
        // The top and bottom strips are the two arms: the same picture, one upside down, so the sheet is not empty there.
        let strip = |rows: std::ops::Range<usize>| pixels[rows.start * width..rows.end * width].iter().filter(|p| p[3] > 0).count();
        let arm_rows = arm[3] as usize;
        assert!(strip(0..arm_rows) > 0 && strip(height - arm_rows..height) > 0);
        if let Ok(dir) = std::env::var("SPRITE_DUMP") {
            let bytes = png(&assets, &genome, index, PIXEL_RATIO).expect("png");
            std::fs::write(std::path::Path::new(&dir).join("sprite_sheet.png"), bytes).expect("write");
        }
    }

    #[test]
    fn blending_keeps_the_destination_under_a_transparent_source() {
        assert_eq!(over([10, 20, 30, 255], [200, 0, 0, 0]), [10, 20, 30, 255]);
        assert_eq!(over([0, 0, 0, 0], [200, 100, 50, 255]), [200, 100, 50, 255]);
        let half = over([0, 0, 255, 255], [255, 0, 0, 128]);
        assert!(half[0] > 120 && half[2] > 100 && half[3] == 255, "{half:?}");
    }
}
