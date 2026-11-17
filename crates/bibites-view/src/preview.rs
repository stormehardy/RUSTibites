//! Procedural bibite previews (`BibiteGenePreviewer`, `BibiteTemplateGenePreviewer`).
//!
//! A previewer composes a bibite from a template's genes with the same procedural sprites the world
//! uses (`ProceduralSpriteManager`: the size tier, the inflate-free body, the mouth, the eyes, the
//! arms and the exoskeleton the genes pick), scales it by `Clamp(baseScale * sqrt(size), 0.05,
//! maxScale)` canvas units per sprite pixel, hangs it at its prefab's offset from the middle of the
//! picture and (with `rotateModelWithDiet`) turns it towards the plant or the meat. The parts are the
//! prefab's children in hierarchy order: mouth, body, left arm, right arm (mirrored), exoskeleton,
//! eyes. The picture is rendered once per genome with the software rasteriser at the size it is shown
//! at (the game samples its sprites with `FilterMode.Point`, so every sprite pixel is a block) and
//! drawn as a [`crate::ui::UiDraw::Pixels`] image.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bibites_core::genome::Genome;
use bibites_core::math::Vec2;

use crate::assets::{GameAssets, body_color, eye_color, total_organ_wag};
use crate::render::{Camera, Primitive, Scene, render_scene};
use crate::ui::UiTexture;

/// `UserSettings.ProceduralSize`: which size the previews pick their sprite tier by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProceduralSize {
    /// The genetic size gene's tier.
    Genetic,
    /// Always the adult-size tier (size 1).
    Maturity,
    /// The product of both (a preview has the genes only, so as `Genetic`).
    Both,
}

impl ProceduralSize {
    /// The choice at index `index` of the setting's list (`Genetic`, `Maturity`, `Both`).
    #[must_use]
    pub fn of_choice(index: usize) -> Self {
        match index {
            0 => Self::Genetic,
            1 => Self::Maturity,
            _ => Self::Both,
        }
    }
}

/// The parameters of one previewer prefab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewStyle {
    /// `baseScale`.
    pub base_scale: f32,
    /// `maxScale`.
    pub max_scale: f32,
    /// `sqrtSizing`.
    pub sqrt_sizing: bool,
    /// `rotateModelWithDiet`.
    pub rotate_with_diet: bool,
    /// `adjustOffset` (the UI previewer moves the bibite's origin with its scale).
    pub adjust_offset: bool,
    /// Canvas units from the middle of the picture to the bibite's origin (the body / mouth joint), y up,
    /// at `base_scale`.
    pub offset: [f32; 2],
    /// Where the prefab's part objects sit in the bibite's own frame, in the frame's units (forward, left): the editor's parts
    /// are 3 forward (`anchoredPosition (0, 3)` under the 90 degree turn of `proceduralBibite`), the other previewers' are 0.
    pub part_shift: [f32; 2],
    /// `FoodPreviewHolder`: the plant and meat icons (`PlantImage`, `MeatImage`) as `[x of the holder from the
    /// middle, y of the icons from the holder's middle, the icons' size]`.
    pub food: Option<[f32; 3]>,
}

/// The species tree node's previewer (`SpeciesTreeNode/SpeciesPreview/proceduralBibite`).
pub const TREE_NODE: PreviewStyle = PreviewStyle {
    base_scale: 5.0,
    max_scale: 9.0,
    sqrt_sizing: true,
    // `BibiteGenePreviewer.UpdateTemplate` turns the model only inside its `PlantImage` / `MeatImage` block: a tree node has no food icons
    // (a live capture: all 25 nodes' `proceduralBibite` are at 0 degrees whatever the diet).
    rotate_with_diet: false,
    adjust_offset: true,
    offset: [15.0, 9.0],
    part_shift: [0.0, 0.0],
    food: None,
};

/// The species info panel's picture (`Preview Item/BibitePreview/proceduralBibite`).
pub const INFO_PANEL: PreviewStyle = PreviewStyle {
    base_scale: 5.0,
    max_scale: 10.0,
    sqrt_sizing: true,
    rotate_with_diet: false,
    adjust_offset: false,
    offset: [0.0, 0.0],
    // The parts' `anchoredPosition` is (0, 4) (the arms (+-4.5, 4)) under `proceduralBibite`'s 90 degree turn: 4 units ahead of the frame's middle
    // (a live capture of the game: the whole bibite sits 25 px to the right of where the unshifted parts put it).
    part_shift: [4.0, 0.0],
    food: None,
};

/// The lineage mesh's previewer (`BodyPreview/previewBibiteUI`, in a 400 x 400 disc).
#[allow(dead_code)] // the mesh view draws it
pub const MESH: PreviewStyle = PreviewStyle {
    base_scale: 10.0,
    max_scale: 20.0,
    sqrt_sizing: true,
    // The game's mesh pictures all lie along the axis whatever the species' diet (a live capture of lol.zip's mesh view): the previewer
    // has no food icons, so `UpdateTemplate` does not turn the model.
    rotate_with_diet: false,
    adjust_offset: true,
    offset: [30.0, 0.0],
    part_shift: [0.0, 0.0],
    food: None,
};

/// The champion previewer of the New Game screen's challenge list (`ChampionPreview/BibitePreview/previewBibiteUI`: scale 6, largest 9,
/// the bibite 25 right of the middle of its 110 x 110 box, turning with its diet).
pub const CHAMPION: PreviewStyle = PreviewStyle {
    base_scale: 6.0,
    max_scale: 9.0,
    sqrt_sizing: true,
    rotate_with_diet: true,
    adjust_offset: true,
    offset: [25.0, 0.0],
    part_shift: [0.0, 0.0],
    food: None,
};

/// The Bibite Library's / the template selector's preview (`BibitePreview/proceduralBibite`).
pub const SELECTOR: PreviewStyle = PreviewStyle {
    base_scale: 10.0,
    max_scale: 24.0,
    sqrt_sizing: true,
    rotate_with_diet: true,
    adjust_offset: false,
    offset: [-24.0, 0.0],
    part_shift: [0.0, 0.0],
    // `FoodPreviewHolder` at 200 right, its icons 40 above and below, 40 x 40.
    food: Some([200.0, 40.0, 40.0]),
};

/// The Bibite Editor's previewer (`BibitePreview/Top/GenePreviewer`, a `ProceduralGenePreviewer`: scale 9, largest 22, the bibite at the
/// middle of its 548 x 392 frame turning with its diet, the food icons in a holder 40 from the right edge). Its part objects
/// (`Mouth`, `Body`, the arms, `Exoskelton`, `Eyes`) have `anchoredPosition (0, 3)` (the arms `(+-4.5, 3)`) under `proceduralBibite`'s
/// 90 degree turn, so the whole bibite sits 3 units ahead of the frame's middle: the reference screenshot has it centred.
pub const EDITOR: PreviewStyle = PreviewStyle {
    base_scale: 9.0,
    max_scale: 22.0,
    sqrt_sizing: true,
    rotate_with_diet: true,
    adjust_offset: false,
    offset: [0.0, 0.0],
    part_shift: [3.0, 0.0],
    // `FoodPreviewHolder` (40 x 120, pivot at its right edge): 214 from the middle; its icons 40 above and below, 40 x 40.
    food: Some([214.0, 40.0, 40.0]),
};

/// What `BibiteGenePreviewer.UpdateTemplate` works out from the genes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewScale {
    /// `size = genes[3] * sqrt(GrowthAtMature)`.
    pub size: f32,
    /// Canvas units per sprite pixel.
    pub scale: f32,
    /// The sprite tier index.
    pub size_index: usize,
    /// How far the bibite turns towards its food, radians (counter-clockwise).
    pub rotation: f32,
    /// Canvas units from the middle of the picture to the bibite's origin (y up).
    pub origin: Vec2,
    /// Where the parts sit in the bibite's own frame (`PreviewStyle::part_shift`), turned and scaled with it.
    pub shift: Vec2,
    /// The food icons' layout and the holder's scale (`1 / sqrt(size)` with `sqrtSizing`).
    pub food: Option<([f32; 3], f32)>,
}

/// What a previewer shows when no template is selected (`UpdateTemplate(null)`): size 1, white pigment, an
/// even diet, no weapons or armour, `viewRadius` 80, `viewAngle` 120 and `speed` 80.
#[must_use]
pub fn no_template() -> Genome {
    let mut genes = *Genome::defaults().values();
    for gene in [5, 6, 7, 29, 31] {
        genes[gene] = 0.0;
    }
    genes[4] = 80.0;
    genes[12] = 120.0;
    genes[13] = 80.0;
    genes[16] = 0.5;
    genes[25] = 0.5;
    Genome::from_values(genes)
}

/// The `size` of a template: `genes[3] * sqrt(GrowthAtMature)` (1 when there is none).
#[must_use]
pub fn template_size(genome: &Genome) -> f32 {
    genome.values()[3] * genome.growth_at_mature().sqrt()
}

/// The `size` the Bibite Editor's previewer works from (`ProceduralGenePreviewer.UpdatePreview`): not the template previewers'
/// `genes[3] * sqrt(GrowthAtMature)`, but the size gene (`Genetic`), the growth at maturity (`Maturity`) or their product (`Both`).
#[must_use]
pub fn editor_size(genome: &Genome, procedural: ProceduralSize) -> f32 {
    let ratio = genome.values()[3];
    let growth = genome.growth_at_mature();
    match procedural {
        ProceduralSize::Genetic => ratio,
        ProceduralSize::Maturity => growth,
        ProceduralSize::Both => ratio * growth,
    }
}

/// `UpdateTemplate`'s numbers for a template of this `size` and genes.
#[must_use]
pub fn scale_of(
    assets: &GameAssets,
    genome: &Genome,
    size: f32,
    style: &PreviewStyle,
    procedural: ProceduralSize,
) -> PreviewScale {
    let genes = genome.values();
    let size_index = match procedural {
        ProceduralSize::Maturity => assets.closest_part_size_index(1.0),
        ProceduralSize::Genetic | ProceduralSize::Both => assets.closest_part_size_index(size),
    };
    let sized = if style.sqrt_sizing { size.sqrt() } else { size };
    let scale = (style.base_scale * sized).clamp(0.05, style.max_scale);
    let carn = genes[16];
    let rotation = if style.rotate_with_diet {
        -2.0 * (carn - 0.5) * (40.0 / (112.0 * size.sqrt())).atan()
    } else {
        0.0
    };
    let x = if style.adjust_offset {
        style.offset[0] * scale / style.base_scale
    } else {
        style.offset[0]
    };
    let holder_scale = if style.sqrt_sizing {
        1.0 / size.sqrt().max(1e-3)
    } else {
        1.0
    };
    PreviewScale {
        size,
        scale,
        size_index,
        rotation,
        origin: Vec2::new(x, style.offset[1]),
        shift: Vec2::new(style.part_shift[0], style.part_shift[1]),
        food: style.food.map(|food| (food, holder_scale)),
    }
}

/// The sprites of the bibite, back to front, with its origin at `origin` (canvas units, y up).
#[must_use]
pub fn compose(
    assets: &GameAssets,
    genome: &Genome,
    numbers: &PreviewScale,
    origin: Vec2,
) -> Vec<Primitive> {
    let genes = genome.values();
    let total = total_organ_wag(genes);
    let share = |gene: f32| {
        if total > 0.0 {
            gene.max(0.0) / total
        } else {
            0.0
        }
    };
    let strength = share(genes[31]).sqrt();
    let defence = share(genes[29]);
    let size_index = numbers.size_index;
    let scale = numbers.scale;
    let hue_shift = assets.tint_mode == crate::assets::TintMode::HueShift;
    let skin = rgba(body_color(genes));
    let eye = rgba(eye_color(genes));
    let body = assets.body_sprite(size_index, 0);
    // The arms hang from half the body's height above and below its axis.
    let half_body = body.map_or(0.0, |sprite| sprite.world_size().y / 2.0);
    let turn = |offset: Vec2| origin + (offset + numbers.shift).rotate(numbers.rotation) * scale;
    let mut sprites = Vec::with_capacity(6);
    let push = |sprite: Option<crate::assets::SpriteRef>,
                offset: Vec2,
                tint: [u8; 4],
                mirrored: bool,
                sprites: &mut Vec<Primitive>| {
        let Some(sprite) = sprite else {
            return;
        };
        // The sprite's pivot is Unity's (y up: the arms' (1, 0) is their bottom right corner), the rasteriser's counts from the top: an arm
        // stands on its anchor and reaches up and back. A transform scale of -1 turns the sprite about its pivot, so the mirrored arm hangs
        // from it and reaches down; its picture is mirrored, which the rasteriser's flip does not do alone (inside the sprite's own rectangle).
        let pivot = crate::sprites::drawn_pivot(sprite.pivot, false, mirrored);
        sprites.push(Primitive::Sprite {
            sprite,
            anchor: turn(offset),
            size: sprite.world_size() * scale,
            pivot,
            rotation: numbers.rotation,
            tint,
            flip_x: false,
            flip_y: mirrored,
            hue_shift,
            effects: crate::skin::SkinEffects::NONE,
            warp: None,
        });
    };
    push(
        assets.mouth_sprite(size_index, genes[16], strength),
        Vec2::ZERO,
        skin,
        false,
        &mut sprites,
    );
    push(body, Vec2::ZERO, skin, false, &mut sprites);
    let arm = assets.arm_sprite(size_index, genes[4]);
    push(arm, Vec2::new(0.0, half_body), skin, false, &mut sprites);
    push(arm, Vec2::new(0.0, -half_body), skin, true, &mut sprites);
    push(
        assets.exoskeleton_sprite(size_index, 0, defence),
        Vec2::ZERO,
        skin,
        false,
        &mut sprites,
    );
    let eyes = assets.eye_sprite(size_index, genes[13], genes[12], false);
    push(eyes, Vec2::ZERO, eye, false, &mut sprites);
    // `FoodPreviewHolder`: the plant icon fades as the diet turns to meat and the meat icon shows (alpha
    // `4 (1 - carn)^2` and `4 carn^2`), each scaled by `sqrt(share + 0.5)` on the holder's own `1 / sqrt(size)`.
    if let Some(([holder_x, spacing, icon], holder_scale)) = numbers.food {
        let carn = genes[16];
        let mut icon_of = |sprite: Option<crate::assets::SpriteRef>, y: f32, share: f32| {
            let Some(sprite) = sprite else {
                return;
            };
            let alpha = (4.0 * share * share).clamp(0.0, 1.0);
            let grown = icon * holder_scale * (share + 0.5).max(0.0).sqrt();
            sprites.push(Primitive::Sprite {
                sprite,
                // The picture's middle is the bibite frame's `-origin`.
                anchor: Vec2::new(
                    holder_x - numbers.origin.x,
                    y * holder_scale - numbers.origin.y,
                ),
                size: Vec2::new(grown, grown),
                pivot: Vec2::new(0.5, 0.5),
                rotation: 0.0,
                tint: [255, 255, 255, (alpha * 255.0).round() as u8],
                flip_x: false,
                flip_y: false,
                hue_shift: false,
                effects: crate::skin::SkinEffects::NONE,
                warp: None,
            });
        };
        icon_of(assets.plant_sprite(1.0), spacing, 1.0 - carn);
        icon_of(assets.meat_sprite(1.0), -spacing, carn);
    }
    sprites
}

fn rgba(color: [f32; 3]) -> [u8; 4] {
    [
        (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        255,
    ]
}

/// A preview to draw: the genes, the prefab's parameters and the picture's size.
#[derive(Clone, Copy, Debug)]
pub struct PreviewRequest<'a> {
    /// The template's genome.
    pub genome: &'a Genome,
    /// The template's `size` (`None`: worked out from the genes; a previewer with no template has size 1).
    pub size: Option<f32>,
    /// The previewer prefab.
    pub style: PreviewStyle,
    /// `UserSettings.ProceduralSize`.
    pub procedural: ProceduralSize,
    /// The picture's size in canvas units.
    pub picture: [f32; 2],
    /// Screen pixels per canvas unit (the picture is rendered at the size it is shown at).
    pub pixels_per_unit: f32,
}

impl PreviewRequest<'_> {
    /// A cache key: the genes and everything the picture depends on.
    #[must_use]
    pub fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for gene in self.genome.values() {
            gene.to_bits().hash(&mut hasher);
        }
        let style = &self.style;
        for value in [
            style.base_scale,
            style.max_scale,
            style.offset[0],
            style.offset[1],
            style.part_shift[0],
            style.part_shift[1],
            self.picture[0],
            self.picture[1],
            self.pixels_per_unit,
            self.size.unwrap_or(f32::NAN),
        ] {
            value.to_bits().hash(&mut hasher);
        }
        (
            style.sqrt_sizing,
            style.rotate_with_diet,
            style.adjust_offset,
            self.procedural,
        )
            .hash(&mut hasher);
        hasher.finish()
    }
}

/// Renders the preview: a transparent picture `size * pixels_per_unit` pixels across with the bibite
/// at its place.
#[must_use]
pub fn render(assets: &GameAssets, request: &PreviewRequest<'_>) -> Option<UiTexture> {
    let width = (request.picture[0] * request.pixels_per_unit)
        .round()
        .max(1.0) as u32;
    let height = (request.picture[1] * request.pixels_per_unit)
        .round()
        .max(1.0) as u32;
    let size = request
        .size
        .unwrap_or_else(|| template_size(request.genome));
    let numbers = scale_of(
        assets,
        request.genome,
        size,
        &request.style,
        request.procedural,
    );
    let primitives = compose(assets, request.genome, &numbers, Vec2::ZERO);
    if primitives.is_empty() {
        return None;
    }
    // The camera looks at the point of the bibite's frame that sits in the middle of the picture.
    let camera = Camera::new(
        Vec2::new(-numbers.origin.x, -numbers.origin.y),
        request.pixels_per_unit,
        Vec2::new(width as f32, height as f32),
    );
    let scene = Scene {
        background: [0, 0, 0, 0],
        primitives,
        ..Scene::new()
    };
    let image = render_scene(&scene, &camera, width, height, 1, &assets.atlases);
    Some(UiTexture {
        width: image.width,
        height: image.height,
        rgba: image.rgba,
    })
}

/// The pictures made so far, by [`PreviewRequest::key`]; a window resize or a different genome makes
/// new ones, so it is emptied when it grows past [`CACHE_MAX`].
#[derive(Debug, Default)]
pub struct PreviewCache {
    pictures: Mutex<HashMap<u64, Option<Arc<UiTexture>>>>,
}

/// The most pictures kept.
pub const CACHE_MAX: usize = 256;

impl PreviewCache {
    /// The picture of `request` (made on the first ask, `None` when the assets cannot make it).
    #[must_use]
    pub fn get(&self, assets: &GameAssets, request: &PreviewRequest<'_>) -> Option<Arc<UiTexture>> {
        let key = request.key();
        let Ok(mut pictures) = self.pictures.lock() else {
            return None;
        };
        if let Some(found) = pictures.get(&key) {
            return found.clone();
        }
        if pictures.len() >= CACHE_MAX {
            pictures.clear();
        }
        let made = render(assets, request).map(Arc::new);
        pictures.insert(key, made.clone());
        made
    }

    /// Makes the pictures of `requests` that are not kept yet, in parallel (a frame that needs many at a new scale - a zoom step over a whole
    /// tree - made them one after another on the UI thread, 35 to 60 ms), so that the `get`s of the frame find them.
    pub fn warm(&self, assets: &GameAssets, requests: &[PreviewRequest<'_>]) {
        let missing: Vec<(u64, &PreviewRequest<'_>)> = {
            let Ok(pictures) = self.pictures.lock() else {
                return;
            };
            let mut seen = std::collections::HashSet::new();
            requests
                .iter()
                .map(|request| (request.key(), request))
                .filter(|(key, _)| !pictures.contains_key(key) && seen.insert(*key))
                .collect()
        };
        if missing.len() < 2 {
            return; // one is made by its own `get`, with no threads to start
        }
        let workers = std::thread::available_parallelism().map_or(2, usize::from).min(8).min(missing.len());
        let made: Vec<(u64, Option<Arc<UiTexture>>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|worker| {
                    let missing = &missing;
                    scope.spawn(move || {
                        missing
                            .iter()
                            .skip(worker)
                            .step_by(workers)
                            .map(|(key, request)| (*key, render(assets, request).map(Arc::new)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|handle| handle.join().unwrap_or_default()).collect()
        });
        if let Ok(mut pictures) = self.pictures.lock() {
            if pictures.len() + made.len() > CACHE_MAX {
                pictures.clear();
            }
            for (key, picture) in made {
                pictures.insert(key, picture);
            }
        }
    }

    /// How many pictures are kept.
    #[cfg(test)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.pictures.lock().map_or(0, |pictures| pictures.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> Option<GameAssets> {
        GameAssets::discover().ok().flatten()
    }

    fn genome_with(edit: impl FnOnce(&mut [f32; bibites_core::genome::N_GENES])) -> Genome {
        let mut values = *Genome::defaults().values();
        edit(&mut values);
        Genome::from_values(values)
    }

    #[test]
    fn the_editors_previewer_has_its_own_size_and_sits_three_units_ahead() {
        // `ProceduralGenePreviewer`: the size gene, the growth at maturity or their product, not the template previewers'
        // `sizeRatio * sqrt(GrowthAtMature)`.
        let genome = genome_with(|g| g[3] = 1.5);
        let growth = genome.growth_at_mature();
        assert!(growth > 1.0, "the default genes grow: {growth}");
        assert_eq!(editor_size(&genome, ProceduralSize::Genetic), 1.5);
        assert_eq!(editor_size(&genome, ProceduralSize::Maturity), growth);
        assert_eq!(editor_size(&genome, ProceduralSize::Both), 1.5 * growth);
        assert_ne!(
            editor_size(&genome, ProceduralSize::Both),
            template_size(&genome)
        );
        let Some(assets) = assets() else {
            return;
        };
        // The parts are 3 units forward of the bibite's origin, turned and scaled with it; the other previewers have no such shift.
        let size = editor_size(&genome, ProceduralSize::Both);
        let numbers = scale_of(&assets, &genome, size, &EDITOR, ProceduralSize::Both);
        assert_eq!(numbers.shift, Vec2::new(3.0, 0.0));
        let plain = scale_of(&assets, &genome, size, &SELECTOR, ProceduralSize::Both);
        assert_eq!(plain.shift, Vec2::ZERO);
        let body_at = |numbers: &PreviewScale| {
            compose(&assets, &genome, numbers, Vec2::ZERO)
                .into_iter()
                .find_map(|primitive| match primitive {
                    Primitive::Sprite { anchor, .. } => Some(anchor),
                    _ => None,
                })
                .expect("a sprite")
        };
        let moved = body_at(&numbers);
        let unshifted = body_at(&PreviewScale {
            shift: Vec2::ZERO,
            ..numbers
        });
        let expected = Vec2::new(3.0, 0.0).rotate(numbers.rotation) * numbers.scale;
        assert!(
            ((moved - unshifted) - expected).length() < 1e-3,
            "{moved:?} {unshifted:?} {expected:?}"
        );
    }

    #[test]
    fn the_scale_follows_the_size_gene_with_the_prefabs_limits() {
        let Some(assets) = assets() else {
            return;
        };
        let small = genome_with(|g| g[3] = 0.25);
        let large = genome_with(|g| g[3] = 40.0);
        let tree = scale_of(
            &assets,
            &small,
            template_size(&small),
            &TREE_NODE,
            ProceduralSize::Both,
        );
        // `baseScale * sqrt(size)`, kept between 0.05 and `maxScale`.
        let expected = TREE_NODE.base_scale * tree.size.sqrt();
        assert!((tree.scale - expected).abs() < 1e-4, "{tree:?}");
        let big = scale_of(
            &assets,
            &large,
            template_size(&large),
            &TREE_NODE,
            ProceduralSize::Both,
        );
        assert_eq!(big.scale, TREE_NODE.max_scale, "{big:?}");
        // The UI previewer moves its origin with the scale; the template previewer does not.
        assert!((tree.origin.x - 15.0 * tree.scale / 5.0).abs() < 1e-4);
        assert_eq!(tree.origin.y, 9.0);
        let info = scale_of(
            &assets,
            &small,
            template_size(&small),
            &INFO_PANEL,
            ProceduralSize::Both,
        );
        assert_eq!(info.origin, Vec2::ZERO);
        assert_eq!(
            info.rotation, 0.0,
            "the info panel does not turn the bibite"
        );
    }

    #[test]
    fn the_bibite_turns_towards_its_food() {
        let Some(assets) = assets() else {
            return;
        };
        let plant = genome_with(|g| g[16] = 0.0);
        let meat = genome_with(|g| g[16] = 1.0);
        let even = genome_with(|g| g[16] = 0.5);
        let up = scale_of(
            &assets,
            &plant,
            template_size(&plant),
            &SELECTOR,
            ProceduralSize::Both,
        )
        .rotation;
        let down = scale_of(
            &assets,
            &meat,
            template_size(&meat),
            &SELECTOR,
            ProceduralSize::Both,
        )
        .rotation;
        let level = scale_of(
            &assets,
            &even,
            template_size(&even),
            &SELECTOR,
            ProceduralSize::Both,
        )
        .rotation;
        assert!(
            up > 0.0 && down < 0.0 && level == 0.0,
            "{up} {down} {level}"
        );
        assert!((up + down).abs() < 1e-6, "symmetric about the even diet");
    }

    #[test]
    fn the_maturity_size_choice_ignores_the_genetic_size() {
        let Some(assets) = assets() else {
            return;
        };
        let tiny = genome_with(|g| g[3] = 0.2);
        let genetic = scale_of(
            &assets,
            &tiny,
            template_size(&tiny),
            &SELECTOR,
            ProceduralSize::Genetic,
        );
        let maturity = scale_of(
            &assets,
            &tiny,
            template_size(&tiny),
            &SELECTOR,
            ProceduralSize::Maturity,
        );
        assert_eq!(maturity.size_index, assets.closest_part_size_index(1.0));
        assert_eq!(
            genetic.size_index,
            assets.closest_part_size_index(genetic.size)
        );
        // The scale itself does not depend on the choice.
        assert_eq!(genetic.scale, maturity.scale);
    }

    #[test]
    fn a_bibite_is_six_sprites_in_the_prefabs_order() {
        let Some(assets) = assets() else {
            return;
        };
        let genome = Genome::defaults();
        let numbers = scale_of(
            &assets,
            &genome,
            template_size(&genome),
            &INFO_PANEL,
            ProceduralSize::Both,
        );
        let sprites = compose(&assets, &genome, &numbers, Vec2::ZERO);
        assert_eq!(sprites.len(), 6, "mouth, body, two arms, exoskeleton, eyes");
        let flips: Vec<bool> = sprites
            .iter()
            .map(|p| matches!(p, Primitive::Sprite { flip_y: true, .. }))
            .collect();
        assert_eq!(
            flips,
            [false, false, false, true, false, false],
            "the right arm is mirrored"
        );
        // The body's right edge and the mouth's left edge are at the origin.
        let Primitive::Sprite {
            anchor,
            size,
            pivot,
            ..
        } = sprites[1]
        else {
            unreachable!()
        };
        // The prefab's parts sit 4 units ahead of the origin (`anchoredPosition (0, 4)` under the 90 degree turn).
        assert!(
            (anchor - Vec2::new(4.0 * numbers.scale, 0.0)).length() < 1e-3,
            "{anchor:?}"
        );
        assert!(
            (pivot.x - 1.0).abs() < 1e-4,
            "the body hangs left of the origin"
        );
        assert!(size.x > 0.0 && size.y > 0.0);
        let Primitive::Sprite { pivot: mouth, .. } = sprites[0] else {
            unreachable!()
        };
        assert!(mouth.x.abs() < 1e-4, "the mouth points right of the origin");
        // The arms hang from half the body's height above and below its axis.
        let Primitive::Sprite { anchor: left, .. } = sprites[2] else {
            unreachable!()
        };
        let Primitive::Sprite { anchor: right, .. } = sprites[3] else {
            unreachable!()
        };
        assert!(
            (left.y - size.y / 2.0).abs() < 1e-3,
            "{left:?} {size:?}"
        );
        assert!((left.y + right.y).abs() < 1e-5);
        // The upper arm stands on its anchor (a Unity pivot of (1, 0) is the bottom right), the mirrored one hangs from it.
        let Primitive::Sprite { pivot: upper, .. } = sprites[2] else {
            unreachable!()
        };
        let Primitive::Sprite { pivot: lower, .. } = sprites[3] else {
            unreachable!()
        };
        assert!(upper.y > 0.99 && lower.y < 0.01, "{upper:?} {lower:?}");
    }

    #[test]
    fn with_no_template_the_previewer_shows_a_white_even_bibite_of_size_one() {
        let Some(assets) = assets() else {
            return;
        };
        let genome = no_template();
        let numbers = scale_of(&assets, &genome, 1.0, &SELECTOR, ProceduralSize::Both);
        // `Clamp(10 * sqrt(1), 0.05, 24)`, hung 24 left of the middle (the library's `Bibite` position).
        assert_eq!(numbers.scale, 10.0);
        assert_eq!(numbers.rotation, 0.0, "an even diet does not turn it");
        assert_eq!(numbers.origin, Vec2::new(-24.0, 0.0));
        assert_eq!(numbers.size_index, assets.closest_part_size_index(1.0));
        let request = PreviewRequest {
            genome: &genome,
            size: Some(1.0),
            style: SELECTOR,
            procedural: ProceduralSize::Both,
            picture: [671.2, 376.0],
            pixels_per_unit: 1.0,
        };
        let picture = render(&assets, &request).expect("a picture");
        assert_eq!((picture.width, picture.height), (671, 376));
        // The body hangs 110 px left of the joint at 24 left of the middle, the mouth 50 right of it:
        // 1508.4 - 1196.8 = 311.6 from the left edge in the game's capture.
        let column = |x: u32| {
            (0..picture.height)
                .any(|y| picture.rgba[((y * picture.width + x) * 4 + 3) as usize] > 0)
        };
        assert!(
            column(311 - 40) && column(311 + 30),
            "the bibite spans the joint"
        );
        assert!(
            column(311 - 108),
            "the body's tail tip is 110 px (11 sprite pixels) left of the joint"
        );
        assert!(
            !column(311 - 130) && !column(311 + 70),
            "and no more than body and mouth"
        );
        // The plant and the meat icons sit 200 right of the middle, 40 above and below it, both whole for an
        // even diet; a carnivore's plant icon is gone.
        let pixel = |x: u32, y: u32| picture.rgba[((y * picture.width + x) * 4 + 3) as usize];
        let (icon_x, plant_y, meat_y) = (335 + 200, 188 - 40, 188 + 40);
        assert!(pixel(icon_x, plant_y) > 200 && pixel(icon_x, meat_y) > 200);
        let carnivore = genome_with(|g| g[16] = 1.0);
        let request = PreviewRequest {
            genome: &carnivore,
            size: Some(1.0),
            ..request
        };
        let picture = render(&assets, &request).expect("a picture");
        let pixel = |x: u32, y: u32| picture.rgba[((y * picture.width + x) * 4 + 3) as usize];
        assert_eq!(pixel(icon_x, plant_y), 0, "the plant fades away");
        assert!(pixel(icon_x, meat_y) > 200);
    }

    #[test]
    fn the_picture_is_rendered_at_the_size_it_is_shown_at_and_cached() {
        let Some(assets) = assets() else {
            return;
        };
        let genome = Genome::defaults();
        let request = PreviewRequest {
            genome: &genome,
            size: None,
            style: INFO_PANEL,
            procedural: ProceduralSize::Both,
            picture: [267.0, 151.59],
            pixels_per_unit: 1.0,
        };
        let picture = render(&assets, &request).expect("a picture");
        assert_eq!((picture.width, picture.height), (267, 152));
        let opaque = picture.rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(opaque > 500, "the bibite is drawn: {opaque} pixels");
        let transparent = picture.rgba.chunks_exact(4).filter(|p| p[3] == 0).count();
        assert!(transparent > opaque, "on a transparent picture");
        // The same request is made once; another size is another picture.
        let cache = PreviewCache::default();
        let first = cache.get(&assets, &request).expect("cached");
        let again = cache.get(&assets, &request).expect("cached");
        assert!(Arc::ptr_eq(&first, &again));
        let larger = PreviewRequest {
            pixels_per_unit: 2.0,
            ..request
        };
        let double = cache.get(&assets, &larger).expect("made");
        assert_eq!((double.width, double.height), (534, 303));
        assert_eq!(cache.len(), 2);
    }
}
