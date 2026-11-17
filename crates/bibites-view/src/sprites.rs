//! Game-faithful sprite composition.
//!
//! This module recreates `BibiteProceduralSpriter` + `ProceduralSpriteManager`
//! behaviour on top of [`GameAssets`]: it picks the same size/inflate/diet/
//! strength/view/defence sprite indices as the game, applies the same pivots and
//! layer order, scales everything by the body's `d1Size`, and computes the same
//! skin/eye colours (`BibiteGenes.GenesToColor` / `GetEyeColor`) plus the corpse
//! decay desaturation from `BibiteProceduralSpriter.SetCorpseDecay`.
//!
//! The output is a list of [`SpriteInstance`]s in painter order; [`crate::render`]
//! turns them into platform draw calls (GPU texture batch for the window,
//! nearest-neighbour sampling for the headless rasteriser — the game uses
//! `FilterMode.Point`, so nearest is also the faithful choice).

use bibites_core::body::{BibiteRecord, EggRecord};
use bibites_core::math::Vec2;
use bibites_core::pellet::{MaterialKind, PelletView};

use crate::assets::{
    BibitePart, GameAssets, SpriteRef, body_color, eye_color, hsv_to_rgb, rgb_to_hsv,
    total_organ_wag,
};
use crate::render::{Rgba, TailWarp};
use crate::tail_mesh::TailState;

/// Debug-only layer filter for the headless closeup diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnlyLayer {
    /// Body sprite only.
    Body,
    /// Mouth only.
    Mouth,
    /// Eyes only.
    Eyes,
    /// Both arms.
    Arms,
    /// Right arm (first arms layer, z=0).
    RightArm,
    /// Left arm (second arms layer, z=-1).
    LeftArm,
    /// Exoskeleton only.
    Exoskeleton,
}

impl OnlyLayer {
    /// Parses the `--only-layer` CLI value.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "body" => Ok(Self::Body),
            "mouth" => Ok(Self::Mouth),
            "eyes" => Ok(Self::Eyes),
            "arms" => Ok(Self::Arms),
            "right-arm" | "rightarm" | "right" => Ok(Self::RightArm),
            "left-arm" | "leftarm" | "left" => Ok(Self::LeftArm),
            "exoskeleton" | "exo" => Ok(Self::Exoskeleton),
            other => Err(format!(
                "invalid --only-layer {other}: expected body|mouth|eyes|arms|right-arm|left-arm|exoskeleton"
            )),
        }
    }
}

static ONLY_LAYER: std::sync::OnceLock<Option<OnlyLayer>> = std::sync::OnceLock::new();

/// Sets the debug layer filter for subsequent sprite composition.
pub fn set_only_layer(layer: Option<OnlyLayer>) {
    let _ = ONLY_LAYER.set(layer);
}

fn only_layer() -> Option<OnlyLayer> {
    *ONLY_LAYER.get_or_init(|| None)
}

/// One sprite draw: atlas/rect plus world placement and tint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteInstance {
    /// Atlas sprite to draw.
    pub sprite: SpriteRef,
    /// World position where the sprite pivot lands.
    pub anchor: Vec2,
    /// World size of the sprite at its current scale.
    pub size: Vec2,
    /// Normalised pivot inside the sprite, `(0,0)` = top-left, `(1,1)` =
    /// bottom-right.
    pub pivot: Vec2,
    /// Sprite rotation in world radians (positive = counter-clockwise).
    pub rotation: f32,
    /// RGBA tint multiplied over the sprite.
    pub tint: Rgba,
    /// Mirror the sprite horizontally.
    pub flip_x: bool,
    /// Mirror the sprite vertically (prefab renderer `flipY`).
    pub flip_y: bool,
    /// Recolour by replacing the art hue with the tint hue (game skin shader).
    pub hue_shift: bool,
    /// Optional procedural tail warp (Body layer only).
    pub warp: Option<TailWarp>,
    /// The skin shader's `[_HurtAmount, _IsKiller]` (zero for what does not have them).
    pub effects: crate::skin::SkinEffects,
}

static PROCEDURAL_SIZE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);

/// `UserSettings.ProceduralSize` for the world's sprites (the Settings screens and the start-up read of the player's settings set it).
pub fn set_procedural_size(choice: crate::preview::ProceduralSize) {
    let value = match choice {
        crate::preview::ProceduralSize::Genetic => 0,
        crate::preview::ProceduralSize::Maturity => 1,
        crate::preview::ProceduralSize::Both => 2,
    };
    PROCEDURAL_SIZE.store(value, std::sync::atomic::Ordering::Relaxed);
}

fn procedural_size() -> crate::preview::ProceduralSize {
    crate::preview::ProceduralSize::of_choice(usize::from(PROCEDURAL_SIZE.load(std::sync::atomic::Ordering::Relaxed)))
}

/// The debug environment switches of the sprite composition, read once. Every bibite asks for them in every frame, and an
/// environment lookup (a system call, a lock and two allocations) takes about a microsecond: three of them per bibite were a
/// few milliseconds of every frame in a world with thousands of bibites in view.
struct DebugSwitches {
    /// `RUSTIBITES_FORCE_SKIN=rad,rain,source` (any of them) puts those patterns on every bibite, to look at them in the window.
    force_rad: bool,
    force_rain: bool,
    force_source: bool,
    /// `RUSTIBITES_NO_WARP` (any value) draws the bodies without the tail's bend.
    no_warp: bool,
    /// `RUSTIBITES_FORCE_TAIL=<degrees>` bends every tail by that tail angle (and a third of it at the segment).
    force_tail_degrees: Option<f32>,
}

fn debug_switches() -> &'static DebugSwitches {
    static SWITCHES: std::sync::OnceLock<DebugSwitches> = std::sync::OnceLock::new();
    SWITCHES.get_or_init(|| {
        let forced = std::env::var("RUSTIBITES_FORCE_SKIN").unwrap_or_default();
        DebugSwitches {
            force_rad: forced.contains("rad"),
            force_rain: forced.contains("rain"),
            force_source: forced.contains("source"),
            no_warp: std::env::var("RUSTIBITES_NO_WARP").is_ok(),
            force_tail_degrees: std::env::var("RUSTIBITES_FORCE_TAIL").ok().and_then(|value| value.parse::<f32>().ok()),
        }
    })
}

/// Composes every visible layer of a bibite, back to front.
///
/// Returns an empty vector when the asset set lacks the needed sheets (caller
/// falls back to the procedural capsule).
#[must_use]
pub fn bibite_sprites(
    assets: &GameAssets,
    record: &BibiteRecord,
    tail: Option<&TailState>,
) -> Vec<SpriteInstance> {
    bibite_sprites_of(assets, record, tail, false)
}

/// [`bibite_sprites`] with the body alone (no mouth, eyes, arms or exoskeleton): a bibite a few pixels long, whose parts are a pixel across and cost a
/// draw each.
pub fn bibite_body_sprite(
    assets: &GameAssets,
    record: &BibiteRecord,
    tail: Option<&TailState>,
) -> Vec<SpriteInstance> {
    bibite_sprites_of(assets, record, tail, true)
}

fn bibite_sprites_of(
    assets: &GameAssets,
    record: &BibiteRecord,
    tail: Option<&TailState>,
    body_only: bool,
) -> Vec<SpriteInstance> {
    let genes = record.genome.values();
    let d1 = record.body.geometry.d1_size;
    if !d1.is_finite() || d1 <= 0.0 {
        return Vec::new();
    }
    // `BibiteProceduralSpriter.SpriteChangeFromGrowth`: the tier follows the size gene (Genetic), the growth (Maturity) or both (their product is `d1`).
    let tier_size = match procedural_size() {
        crate::preview::ProceduralSize::Genetic => genes[3],
        crate::preview::ProceduralSize::Maturity => record.growth.growth.max(0.0).sqrt(),
        crate::preview::ProceduralSize::Both => d1,
    };
    let size_index = assets.closest_part_size_index(tier_size);
    let inflate_index = assets.closest_inflate_index(size_index, record.fat.inflate_factor);
    let body_sprite = match assets.body_sprite(size_index, inflate_index) {
        Some(sprite) => sprite,
        None => return Vec::new(),
    };

    let total_wag = total_organ_wag(genes);
    let strength = if total_wag > 0.0 {
        (genes[31].max(0.0) / total_wag).sqrt()
    } else {
        0.0
    };
    let defence = if total_wag > 0.0 {
        genes[29].max(0.0) / total_wag
    } else {
        0.0
    };

    let hue_shift = assets.tint_mode == crate::assets::TintMode::HueShift;
    let skin = skin_tint(record);
    let eye = eye_tint(record);
    let dead = record.is_corpse;
    let rotation = record.rot;
    // `BibiteBody.BecomeReaper` sets `_IsKiller`; `damagedFlash` is `_HurtAmount`.
    // Debug aid: `RUSTIBITES_FORCE_SKIN=rad,rain,source` (any of them) puts those patterns on every bibite, to look at them in the window.
    let switches = debug_switches();
    let (rad, rain, source) = (
        record.meta.is_rad || switches.force_rad,
        record.meta.is_rain || switches.force_rain,
        record.meta.is_source || switches.force_source,
    );
    let up = Vec2::Y.rotate(record.rot);
    let arm_height = assets.arm_sprite(size_index, genes[4]).map_or(0.0, |arm| arm.rect[3] as f32);
    let effects = crate::skin::SkinEffects {
        hurt: tail.map_or(0.0, |tail| tail.hurt),
        killer: if record.meta.is_killer { 1.0 } else { 0.0 },
        // `RadioZone.color` (176, 255, 0): the virus of a radioactive bibite (`_InfectedRatio` 1.25).
        infected: if rad { 1.25 } else { 0.0 },
        virus: [176.0 / 255.0, 1.0, 0.0],
        rainbow: rain,
        producer: source,
        root: [record.pos.x, record.pos.y],
        dir: [up.x, up.y],
        upp: d1 / body_sprite.ppu.max(1e-6),
        body_width: body_sprite.rect[3] as f32 + 2.0 * arm_height,
    };

    // A body drawn alone is far away: the virus, rainbow and producer patterns (each a software picture of the sprite, per frame) are not drawn then, only the colour.
    let effects = if body_only { crate::skin::SkinEffects { infected: 0.0, rainbow: false, producer: false, ..effects } } else { effects };
    let filter = only_layer();
    let mut arms_seen = 0_usize;
    let mut sprites = Vec::with_capacity(7);
    for (part, layout) in &assets.bibite_layout.layers {
        let selected = match filter {
            None => true,
            Some(OnlyLayer::Body) => *part == BibitePart::Body,
            Some(OnlyLayer::Mouth) => *part == BibitePart::Mouth,
            Some(OnlyLayer::Eyes) => matches!(part, BibitePart::Eyes | BibitePart::EyesDead),
            Some(OnlyLayer::Arms) => *part == BibitePart::Arms,
            Some(OnlyLayer::RightArm) => *part == BibitePart::Arms && arms_seen == 0,
            Some(OnlyLayer::LeftArm) => *part == BibitePart::Arms && arms_seen == 1,
            Some(OnlyLayer::Exoskeleton) => *part == BibitePart::Exoskeleton,
        };
        if *part == BibitePart::Arms {
            arms_seen += 1;
        }
        if !selected || (body_only && *part != BibitePart::Body) {
            continue;
        }
        let sprite = match part {
            BibitePart::Body => Some(body_sprite),
            BibitePart::Mouth => assets.mouth_sprite(size_index, genes[16], strength),
            BibitePart::Eyes => assets
                .eye_sprite(size_index, genes[13], genes[12], dead)
                .or_else(|| assets.eye_sprite(size_index, genes[13], genes[12], false)),
            BibitePart::EyesDead => assets.eye_sprite(size_index, genes[13], genes[12], true),
            BibitePart::Arms => assets.arm_sprite(size_index, genes[4]),
            BibitePart::Exoskeleton => {
                assets.exoskeleton_sprite(size_index, inflate_index, defence)
            }
        };
        let Some(sprite) = sprite else {
            continue;
        };
        let tint = match part {
            BibitePart::Eyes | BibitePart::EyesDead => eye,
            _ => skin,
        };
        let warp = if *part == BibitePart::Body && !switches.no_warp {
            tail.map(|tail| {
                let (mut angle0, mut angle1) = tail.warp_angles();
                // Debug aid: `RUSTIBITES_FORCE_TAIL=<degrees>` bends every tail by that tail angle (and a third of it at the segment).
                if let Some(degrees) = switches.force_tail_degrees {
                    angle1 = degrees.to_radians();
                    angle0 = angle1 / 3.0;
                }
                TailWarp {
                    angle0,
                    angle1,
                    bend_row: 0.55,
                    size_scale: ((d1 * 12.5).round() as u32)
                        .saturating_mul(2)
                        .saturating_add(1)
                        .max(13),
                }
            })
        } else {
            None
        };
        // The prefab's own placement and nothing else: the game has no extra offset on the Left Arm (the renderer bounds of the live dump's 11
        // bibites are the prefab's transforms, the sprite's pivot and Unity's flips, to 0.0001). What looked like one was the pivot: see
        // [`drawn_pivot`].
        sprites.push(SpriteInstance {
            sprite,
            anchor: record.pos + layout.local_position.rotate(rotation) * d1,
            size: sprite.world_size() * layout.local_scale * d1,
            pivot: drawn_pivot(sprite.pivot, layout.flip_x, layout.flip_y),
            rotation: rotation + layout.local_rotation,
            tint: multiply_tint(tint, layout.color),
            flip_x: layout.flip_x,
            flip_y: layout.flip_y,
            hue_shift,
            warp,
            effects: if hue_shift && matches!(part, BibitePart::Body | BibitePart::Arms | BibitePart::Mouth | BibitePart::Exoskeleton) { effects } else { crate::skin::SkinEffects::NONE },
        });
    }
    sprites
}

/// The mean colour of the opaque texels of a sprite's cell (straight RGB in `0..=1`, weighted by alpha), kept for the life of the thread: what a sprite
/// looks like from far away. `None` when the cell is not inside the software atlas (no software atlases, or a cell outside them).
fn mean_texel(assets: &GameAssets, sprite: SpriteRef) -> Option<[f32; 3]> {
    /// The means made so far, by atlas and cell.
    type Means = std::collections::HashMap<(u16, [u32; 4]), Option<[f32; 3]>>;
    thread_local! {
        static MEANS: std::cell::RefCell<Means> = std::cell::RefCell::new(Means::new());
    }
    let key = (sprite.atlas, sprite.rect);
    if let Some(known) = MEANS.with(|means| means.borrow().get(&key).copied()) {
        return known;
    }
    let mean = (|| {
        let atlas = assets.atlases.get(usize::from(sprite.atlas))?;
        let [x0, y0, width, height] = sprite.rect;
        if width == 0 || height == 0 || x0.checked_add(width)? > atlas.width || y0.checked_add(height)? > atlas.height {
            return None;
        }
        let (mut sum, mut weight) = ([0.0_f64; 3], 0.0_f64);
        for y in y0..y0 + height {
            let start = ((y * atlas.width + x0) * 4) as usize;
            for texel in atlas.rgba.get(start..start + (width * 4) as usize)?.chunks_exact(4) {
                let alpha = f64::from(texel[3]);
                for channel in 0..3 {
                    sum[channel] += f64::from(texel[channel]) * alpha;
                }
                weight += alpha;
            }
        }
        (weight > 0.0).then(|| [0, 1, 2].map(|channel| (sum[channel] / weight / 255.0) as f32))
    })();
    MEANS.with(|means| means.borrow_mut().insert(key, mean));
    mean
}

/// The colour a bibite is drawn in when it is too small to draw in parts: its body sprite's mean texel through the skin shader with the bibite's colour
/// (and the reaper's darkening and the hurt flash), as [`crate::render`]'s recoloured cells have it. Without a software atlas to take the mean from, the
/// bibite's own colour. `None` for a bibite with no body sprite (the caller draws it in the ordinary way).
#[must_use]
pub fn bibite_dot_colour(assets: &GameAssets, record: &BibiteRecord, tail: Option<&TailState>) -> Option<Rgba> {
    let genes = record.genome.values();
    let d1 = record.body.geometry.d1_size;
    if !d1.is_finite() || d1 <= 0.0 {
        return None;
    }
    let tier_size = match procedural_size() {
        crate::preview::ProceduralSize::Genetic => genes[3],
        crate::preview::ProceduralSize::Maturity => record.growth.growth.max(0.0).sqrt(),
        crate::preview::ProceduralSize::Both => d1,
    };
    let size_index = assets.closest_part_size_index(tier_size);
    let inflate_index = assets.closest_inflate_index(size_index, record.fat.inflate_factor);
    let body_sprite = assets.body_sprite(size_index, inflate_index)?;
    let tint = skin_tint(record);
    let (killer, hurt) = (if record.meta.is_killer { 1.0_f32 } else { 0.0 }, tail.map_or(0.0, |tail| tail.hurt).clamp(0.0, 1.0));
    let rgb = match mean_texel(assets, body_sprite) {
        Some(texel) if assets.tint_mode == crate::assets::TintMode::HueShift => {
            let skin = crate::skin::SkinParams::from(&assets.parameters);
            let shaded = crate::skin::SkinTint::new(tint.map(|channel| channel * (1.0 - killer)), skin).apply(texel);
            let flash = 0.75 * hurt;
            [shaded[0] + (1.0 - shaded[0]) * flash, shaded[1] + (0.1011 - shaded[1]) * flash, shaded[2] * (1.0 - flash)]
        }
        Some(texel) => [texel[0] * tint[0], texel[1] * tint[1], texel[2] * tint[2]],
        None => tint,
    };
    Some(color_to_rgba(rgb, 1.0))
}

/// The colour of a pellet too small to draw: the mean texel of its material's sprite (the art is pre-coloured). `None` for the other materials, and without
/// the art.
#[must_use]
pub fn pellet_dot_colour(assets: &GameAssets, material: MaterialKind) -> Option<Rgba> {
    let sprite = match material {
        MaterialKind::Plant => assets.plant_sprite(1.0),
        MaterialKind::Meat => assets.meat_sprite(1.0),
        _ => None,
    }?;
    let texel = mean_texel(assets, sprite)?;
    Some(color_to_rgba(texel, 1.0))
}

/// The pivot the rasteriser (and the window's `draw_texture_ex`) draws a layer round, from the sprite's own: Unity's, normalised with y UP
/// (`(0, 0)` is the bottom left corner; the arms' `(1, 0)` is their bottom right one).
///
/// The renderers count a pivot from the TOP of the picture, and their flips turn the picture inside the sprite's own rectangle, where a Unity
/// `SpriteRenderer` turns the whole sprite, rectangle included, about its pivot. So an unflipped sprite is drawn round `1 - y` (it stands on its
/// pivot: Unity's `y` counted from the other end), and a flipped one hangs from its pivot as it is (turned about itself the rectangle runs the
/// other way, which is the renderers' own rectangle for the pivot of the other edge; their flip then mirrors the picture inside it). Only the
/// arms have a pivot that is not on the middle line, and the prefab flips only the right arm: with `y` taken as it is for both, the right arm
/// was right and the left arm sat one arm's thickness too far in. A constant (3 units, measured on a normal bibite) moved it back for that tier
/// alone, so the large tier's left arm hovered off the body and the huge tier's lay on it.
#[must_use]
pub fn drawn_pivot(pivot: Vec2, flip_x: bool, flip_y: bool) -> Vec2 {
    Vec2::new(
        if flip_x { 1.0 - pivot.x } else { pivot.x },
        if flip_y { pivot.y } else { 1.0 - pivot.y },
    )
}

/// Egg sprite: `ClosestSizeIndex(sizeGene)`, scaled by `sizeGene`, tinted by the
/// child's body colour (mirrors `EggHatching`).
#[must_use]
pub fn egg_sprite(assets: &GameAssets, egg: &EggRecord) -> Option<SpriteInstance> {
    let size_gene = egg.size_gene.max(0.0);
    let sprite = assets.egg_sprite(size_gene)?;
    let tint = color_to_rgba(egg_tint(egg.genome.values()), 1.0);
    Some(SpriteInstance {
        sprite,
        anchor: egg.pos,
        size: sprite.world_size() * size_gene,
        pivot: sprite.pivot,
        rotation: egg.rot,
        tint,
        flip_x: false,
        flip_y: false,
        hue_shift: assets.tint_mode == crate::assets::TintMode::HueShift,
        warp: None,
        effects: crate::skin::SkinEffects::NONE,
    })
}

/// Pellet sprite local scale from amount: `sqrt(amount / NormalAmountAtScale1)`
/// (`NormalAmountAtScale1 = 20`), matching the game's `Matter` prefab radius
/// `2.5·sqrt(amount/20)` at authored scale 1.
#[must_use]
pub fn pellet_render_scale(amount: f32) -> f32 {
    (amount.max(0.0) / 20.0).sqrt()
}

/// Pellet sprite: `ClosestSizeIndex(sqrt(sizeFactor))` with transform scale
/// `sqrt(sizeFactor)`, where `sizeFactor = amount/20` in the game
/// (`NormalAmountAtScale1`; prefab radius `2.5·sqrt(amount/20)` at scale 1).
/// The art is pre-coloured (plant/meat), so no tint is applied.
#[must_use]
pub fn pellet_sprite(
    assets: &GameAssets,
    pellet: &PelletView,
    rotation: f32,
) -> Option<SpriteInstance> {
    let scale = pellet_render_scale(pellet.amount);
    let sprite = match pellet.material {
        MaterialKind::Plant => assets.plant_sprite(scale),
        MaterialKind::Meat => assets.meat_sprite(scale),
        _ => None,
    }?;
    Some(SpriteInstance {
        sprite,
        anchor: pellet.pos,
        size: sprite.world_size() * scale,
        pivot: sprite.pivot,
        rotation,
        tint: [255, 255, 255, 255],
        flip_x: false,
        flip_y: false,
        hue_shift: false,
        warp: None,
        effects: crate::skin::SkinEffects::NONE,
    })
}

/// Skin colour including corpse decay (`BibiteProceduralSpriter.SetCorpseDecay`).
#[must_use]
pub fn skin_tint(record: &BibiteRecord) -> [f32; 3] {
    let genes = record.genome.values();
    if !record.is_corpse {
        return body_color(genes);
    }
    let decay = record.body.corpse_decay.clamp(0.0, 1.0);
    let (hue, saturation, value) = rgb_to_hsv(body_color(genes));
    let target_value = if value > 0.5 { 0.5 } else { 0.85 };
    hsv_to_rgb(
        hue,
        lerp(saturation, 0.0, decay),
        lerp(value, target_value, decay),
    )
}

/// The colour of the eye material, including the corpse's decay.
///
/// A living bibite's eyes are `GetEyeColor` (the body hue moved by `EyeOffset`, fully saturated and bright).
/// `BibiteProceduralSpriter.SetCorpseDecay` gives a corpse's eye material the body hue at saturation 0.01 and the body's value
/// and sets its `_saturationOverride` to `1 - decay`; the eye shader moves the hue by `EyeOffset` and lifts saturation and
/// value towards 1 by that override ([`crate::skin::eye_shader_color`]), so the eyes fade to the body's own colour.
#[must_use]
pub fn eye_tint(record: &BibiteRecord) -> [f32; 3] {
    let genes = record.genome.values();
    if !record.is_corpse {
        return eye_color(genes);
    }
    let decay = record.body.corpse_decay.clamp(0.0, 1.0);
    let (hue, _saturation, value) = rgb_to_hsv(body_color(genes));
    crate::skin::eye_shader_color(hsv_to_rgb(hue, 0.01, value), genes[25], 1.0 - decay)
}

/// Egg sprite tint from a child's genes.
#[must_use]
pub fn egg_tint(genes: &[f32; bibites_core::genome::N_GENES]) -> [f32; 3] {
    body_color(genes)
}

fn multiply_tint(color: [f32; 3], layer: [f32; 4]) -> Rgba {
    color_to_rgba(
        [
            color[0] * layer[0],
            color[1] * layer[1],
            color[2] * layer[2],
        ],
        layer[3],
    )
}

fn color_to_rgba(color: [f32; 3], alpha: f32) -> Rgba {
    [
        (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_helpers_are_bounded() {
        let tint = color_to_rgba([1.5, -0.5, 0.5], 2.0);
        assert_eq!(tint, [255, 0, 128, 255]);
        let multiplied = multiply_tint([1.0, 0.5, 0.0], [0.5, 1.0, 1.0, 0.5]);
        assert_eq!(multiplied, [128, 128, 0, 128]);
    }

    #[test]
    fn a_unity_pivot_becomes_the_rasterisers_by_the_sprites_flips() {
        // The arms' pivot is their bottom right corner (Unity, y up): the unflipped arm stands on it, the flipped one hangs from it.
        let arm = Vec2::new(1.0, 0.0);
        assert_eq!(drawn_pivot(arm, false, false), Vec2::new(1.0, 1.0));
        assert_eq!(drawn_pivot(arm, false, true), Vec2::new(1.0, 0.0));
        // A pivot on the middle line (the body's, the mouth's, the eyes') is the same whichever way the sprite is turned.
        for flip in [false, true] {
            assert_eq!(
                drawn_pivot(Vec2::new(1.0, 0.5), false, flip),
                Vec2::new(1.0, 0.5)
            );
        }
        // Flipped in x the rectangle runs the other way.
        assert_eq!(
            drawn_pivot(Vec2::new(0.25, 0.5), true, false),
            Vec2::new(0.75, 0.5)
        );
    }

    #[test]
    fn pellet_scale_matches_matter_material() {
        // 31.1189 u² at world (-6.628, 837.456) in the reference closeup:
        // radius = 2.5 * sqrt(31.1189 / 20) = 3.118 world units.
        let scale = pellet_render_scale(31.118_916);
        assert!((scale - 1.247_375).abs() < 1e-4, "{scale}");
        let radius = 2.5 * scale;
        assert!((radius - 3.118_438).abs() < 1e-4, "{radius}");
        assert!((pellet_render_scale(20.0) - 1.0).abs() < 1e-6);
        assert_eq!(pellet_render_scale(-1.0), 0.0);
    }
}
