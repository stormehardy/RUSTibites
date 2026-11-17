//! Game-asset loading for the game-faithful renderer.
//!
//! The viewer can render The Bibites' *actual* procedural sprites when the
//! user supplies extracted game assets in `assets/game/` (see
//! `docs/VIEWER.md` and `assets/game/LICENSE-ASSETS.txt`). No game artwork is
//! part of the MIT-licensed source tree: the folder is optional and every
//! accessor here degrades to the built-in procedural renderer when it is
//! missing or malformed.
//!
//! The asset layout mirrors the game's `Resources/ProceduralSprites/` tree:
//!
//! * CSV text assets: `Bibites/sizes`, `Bibites/parts`,
//!   `Objects/egg_sizes`, `Objects/pellet_sizes`, `parameters`
//! * sprite sheets: `Bibites/Parts/<size>_body.png`, `_mouth.png`,
//!   `_eyes.png`, `_eyes_dead.png`, `_arms.png`, `_exoskeleton.png`
//! * object sprites: `Objects/Sprites/<size>_egg.png`, `_plant.png`, `_meat.png`
//!
//! `manifest.json` maps logical names to files on disk (and optionally carries
//! prefab/renderer metadata such as child transforms and sorting order). The
//! loader is deliberately tolerant: both a nested `resources` layout and a
//! flat directory of decoded PNGs are accepted, and CSV text assets are read
//! from the manifest entries *or* discovered by name.
//!
//! Slicing follows `ProceduralSpriteManager.cs`: a sheet is a `rows × cols`
//! grid of equally sized cells (`parts` CSV), the sprite's pixels-per-unit is
//! `cellPixelWidth / 9` for bibite parts (`/11` eggs, `/5` pellets), so one
//! sprite is 9 world units wide at local scale 1. The bibite root transform is
//! scaled by `d1Size`, i.e. every part's world size is `9 * d1Size`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bibites_core::genome::{Gene, N_GENES};
use bibites_core::math::Vec2;

/// Pixels-per-unit divisor for bibite part sheets (`partSizes[k].pixelWidth/9`).
pub const PART_PPU_DIVISOR: f32 = 9.0;
/// Pixels-per-unit divisor for egg sprites (`/11`).
pub const EGG_PPU_DIVISOR: f32 = 11.0;
/// Pixels-per-unit divisor for pellet sprites (`/5`).
pub const PELLET_PPU_DIVISOR: f32 = 5.0;

/// Default game camera half-height in world units, used by `--ortho-size` and
/// reference-frame matching when the manifest does not record it.
pub const DEFAULT_ORTHOGRAPHIC_SIZE: f32 = 500.0;

/// One size tier from `sizes.csv` (`name;pixelWidth`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SizeEntry {
    /// Tier name as used in the sprite-sheet file names.
    pub name: String,
    /// Native pixel width of the tier (also its resolution ratio).
    pub pixel_width: u32,
}

/// Grid description from `parts.csv` (`name;nRows;nColumns`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartGrid {
    /// Number of sprite rows in the sheet.
    pub rows: u32,
    /// Number of sprite columns in the sheet.
    pub columns: u32,
}

/// A decoded RGBA8 texture atlas (top row first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atlas {
    /// Logical name from the manifest.
    pub name: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA rows, top to bottom.
    pub rgba: Vec<u8>,
}

/// A sprite reference: an atlas plus a pixel rectangle in **top-left** origin
/// coordinates (Unity rects are bottom-left; the loader converts).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteRef {
    /// Index into [`GameAssets::atlases`].
    pub atlas: u16,
    /// `[x, y, width, height]` in atlas pixels (top-left origin).
    pub rect: [u32; 4],
    /// Pixels per world unit for this sprite.
    pub ppu: f32,
    /// Normalised sprite pivot as Unity has it: y UP, `(0, 0)` is the bottom left corner (the arms' `(1, 0)` is their bottom right one). The renderers
    /// count from the top: [`crate::sprites::drawn_pivot`] turns it into theirs.
    pub pivot: Vec2,
}

impl SpriteRef {
    /// World-space size of the sprite at local scale 1.
    #[must_use]
    pub fn world_size(&self) -> Vec2 {
        Vec2::new(
            self.rect[2] as f32 / self.ppu,
            self.rect[3] as f32 / self.ppu,
        )
    }
}

/// The six procedural body-part sheets, mirroring `ProceduralSpriteManager`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BibitePart {
    /// Body (inflate rows).
    Body,
    /// Mouth (diet columns × strength rows).
    Mouth,
    /// Eyes (view-angle columns × view-radius rows).
    Eyes,
    /// Dead eyes (same grid as [`BibitePart::Eyes`]).
    EyesDead,
    /// Arms (speed columns).
    Arms,
    /// Exoskeleton (defence columns × inflate rows).
    Exoskeleton,
}

impl BibitePart {
    /// Sheet-name suffix in `parts.csv` / file names (`body`, `eyes_dead`, …).
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Mouth => "mouth",
            Self::Eyes => "eyes",
            Self::EyesDead => "eyes_dead",
            Self::Arms => "arms",
            Self::Exoskeleton => "exoskeleton",
        }
    }
}

/// One sliced bibite part sheet for one size tier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sheet {
    /// Atlas index.
    pub atlas: u16,
    /// Cell size in pixels.
    pub cell: [u32; 2],
    /// Grid size.
    pub grid: PartGrid,
    /// Pixels per world unit (`size pixelWidth / 9`).
    pub ppu: f32,
    /// Normalised pivot, `(0,0)` = top-left (converted from Unity).
    pub pivot: Vec2,
    /// `true` when the game's rows are stored bottom-up (Unity sprite rects);
    /// the PNG export is top-down, so the row index is mirrored.
    pub bottom_origin_rows: bool,
}

impl Sheet {
    /// Sprite at `(column, row)`, clipped to the grid.
    #[must_use]
    pub fn sprite(&self, column: u32, row: u32) -> SpriteRef {
        let column = column.min(self.grid.columns.saturating_sub(1));
        let row = row.min(self.grid.rows.saturating_sub(1));
        let row = if self.bottom_origin_rows {
            self.grid.rows - 1 - row
        } else {
            row
        };
        SpriteRef {
            atlas: self.atlas,
            rect: [
                column * self.cell[0],
                row * self.cell[1],
                self.cell[0],
                self.cell[1],
            ],
            ppu: self.ppu,
            pivot: self.pivot,
        }
    }
}

/// Per-layer placement from the game's bibite prefab.
///
/// When the manifest does not provide prefab metadata (or for a hand-made
/// fixture), the defaults below place every layer at the rig origin; the game's
/// artist-authored pivots (`bodyPivot`, `mouthPivot`, `eyesPivot`, …) then do
/// the alignment, which is exactly how the Unity prefab is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerLayout {
    /// Local position in parent units (multiplied by `d1Size`).
    pub local_position: Vec2,
    /// Local rotation in radians (additive to the body rotation).
    pub local_rotation: f32,
    /// Local scale multiplier.
    pub local_scale: f32,
    /// Unity sorting order (higher draws later).
    pub sorting_order: i32,
    /// Static sprite tint multiplied with the skin/eye colour.
    pub color: [f32; 4],
    /// Mirror horizontally.
    pub flip_x: bool,
    /// Mirror vertically (prefab renderer `flipY`).
    pub flip_y: bool,
}

impl Default for LayerLayout {
    fn default() -> Self {
        Self {
            local_position: Vec2::ZERO,
            local_rotation: 0.0,
            local_scale: 1.0,
            sorting_order: 0,
            color: [1.0, 1.0, 1.0, 1.0],
            flip_x: false,
            flip_y: false,
        }
    }
}

/// Sprite-layer metadata for the bibite prefab (`Left Arm`, `RightArm`, …).
#[derive(Clone, Debug, PartialEq)]
pub struct BibiteLayout {
    /// The six part layers, in the game prefab's child order.
    pub layers: Vec<(BibitePart, LayerLayout)>,
}

impl Default for BibiteLayout {
    fn default() -> Self {
        // Observed draw roles (see BibiteProceduralSpriter): both arms use the
        // same sprite; one sits behind the body, one in front; the exoskeleton
        // overlays the body; the mouth and then the eyes sit on top. Sorting
        // orders are refreshed from the prefab manifest when available.
        let layer = |sorting_order| LayerLayout {
            sorting_order,
            ..LayerLayout::default()
        };
        Self {
            layers: vec![
                (BibitePart::Arms, layer(-2)), // Left Arm (behind)
                (BibitePart::Body, layer(-1)),
                (BibitePart::Arms, layer(0)), // RightArm (front arm)
                (BibitePart::Exoskeleton, layer(1)),
                (BibitePart::Mouth, layer(2)),
                (BibitePart::Eyes, layer(3)),
            ],
        }
    }
}

/// Camera/reference metadata (used by `--ortho-size` and comparison shots).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraDefaults {
    /// `Camera.main.orthographicSize` (half viewport height in world units).
    pub orthographic_size: f32,
    /// `Camera.main.backgroundColor`.
    pub background_color: [f32; 4],
}

impl Default for CameraDefaults {
    fn default() -> Self {
        // `UserSettings.BackgroundColor` default: (0, 0, 0.19, 1).
        Self {
            orthographic_size: DEFAULT_ORTHOGRAPHIC_SIZE,
            background_color: [0.0, 0.0, 0.19, 1.0],
        }
    }
}

/// `parameters.csv` shader constants (`InitialHue`, `VminAtS0`, `VmaxAtS0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameters {
    /// Base hue used by the skin shader.
    pub initial_hue: f32,
    /// Value where saturation starts at 0.
    pub vmin_at_s0: f32,
    /// Value where saturation reaches 1.
    pub vmax_at_s0: f32,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            initial_hue: 0.55,
            vmin_at_s0: 0.0,
            vmax_at_s0: 1.0,
        }
    }
}

/// The sprites of the towers and the colour killer (`sharedassets2.assets`: the `Radiation Tower`,
/// `PheroTower` and `ColorKiller` prefabs, 1 pixel = 1 world unit).
#[derive(Clone, Copy, Debug, Default)]
pub struct ToolSprites {
    /// `laser_circle` (35 x 35): the round base of the radiation tower and the colour killer.
    pub base: Option<SpriteRef>,
    /// `radioactive` (35 x 35): the icon on the radiation tower.
    pub rad_icon: Option<SpriteRef>,
    /// `pheroTower` (45 x 45).
    pub phero: Option<SpriteRef>,
    /// `laser_crystal` (15 x 15): the colour killer's crystal (magenta art).
    pub crystal: Option<SpriteRef>,
    /// The crystal recoloured to every whole degree of hue by `Shader Graphs/LaserCrystal` (`crystal_hue_cell` picks one): the first cell of an atlas of 360.
    pub crystal_hues: Option<SpriteRef>,
}

impl ToolSprites {
    /// The crystal of a colour killer of hue `hue` (0 to 1: `ColorKiller.UpdateHue`'s `HSVToRGB(hue, 1, 1)` is the shader's `_color`).
    #[must_use]
    pub fn crystal_hue_cell(&self, hue: f32) -> Option<SpriteRef> {
        let first = self.crystal_hues?;
        let degree = (hue.rem_euclid(1.0) * 360.0).round() as u32 % CRYSTAL_HUES;
        let height = first.rect[3];
        Some(SpriteRef {
            rect: [first.rect[0], first.rect[1] + degree * height, first.rect[2], height],
            ..first
        })
    }
}

/// How many hues of the colour killer's crystal are baked (one per degree).
pub const CRYSTAL_HUES: u32 = 360;

/// `Color.HSVToRGB(h, 1, 1)`.
#[must_use]
pub fn hsv_full(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let f = h - h.floor();
    match h.floor() as u32 {
        0 => [1.0, f, 0.0],
        1 => [1.0 - f, 1.0, 0.0],
        2 => [0.0, 1.0, f],
        3 => [0.0, 1.0 - f, 1.0],
        4 => [f, 0.0, 1.0],
        _ => [1.0, 0.0, 1.0 - f],
    }
}

/// The crystal's pixels for one hue: the game's `LaserCrystal` pixel program over every texel of the art, `_color` = `HSVToRGB(hue, 1, 1)`.
#[must_use]
pub fn crystal_pixels(art: &Atlas, hue: f32) -> Vec<u8> {
    let (w, h) = (art.width as usize, art.height as usize);
    let color = hsv_full(hue);
    let cb0 = [[0.0f32; 4]; 5];
    let cb1 = [[0.0f32; 4], [color[0], color[1], color[2], 0.0]];
    let fetch = |uv: [f32; 2]| -> [f32; 4] {
        let x = ((uv[0] * w as f32).floor() as i64).clamp(0, w as i64 - 1) as usize;
        let y = (((1.0 - uv[1]) * h as f32).floor() as i64).clamp(0, h as i64 - 1) as usize;
        let at = (y * w + x) * 4;
        let c = |k: usize| f32::from(art.rgba[at + k]) / 255.0;
        [c(0), c(1), c(2), c(3)]
    };
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let uv = [(x as f32 + 0.5) / w as f32, 1.0 - (y as f32 + 0.5) / h as f32];
            let Some(rgba) = crate::internals_shaders::laser_pixel(uv, [1.0; 4], &cb0, &cb1, &fetch) else {
                continue;
            };
            for k in 0..4 {
                out[(y * w + x) * 4 + k] = (rgba[k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    out
}

/// Everything the sprite renderer needs, loaded from `assets/game/`.
pub struct GameAssets {
    /// Directory the manifest was loaded from.
    pub root: PathBuf,
    /// Bibite size tiers (`Bibites/sizes`).
    pub part_sizes: Vec<SizeEntry>,
    /// Egg size tiers (`Objects/egg_sizes`).
    pub egg_sizes: Vec<SizeEntry>,
    /// Pellet size tiers (`Objects/pellet_sizes`).
    pub pellet_sizes: Vec<SizeEntry>,
    /// Decoded atlases, indexed by [`SpriteRef::atlas`].
    pub atlases: Vec<Atlas>,
    /// Part sheets keyed by `(size index, part)`.
    sheets: HashMap<(usize, BibitePart), Sheet>,
    /// Egg sprites by egg size index.
    egg_sprites: Vec<Option<SpriteRef>>,
    /// Plant pellet sprites by pellet size index.
    plant_sprites: Vec<Option<SpriteRef>>,
    /// Meat pellet sprites by pellet size index.
    meat_sprites: Vec<Option<SpriteRef>>,
    /// Tower and colour killer sprites.
    pub tool_sprites: ToolSprites,
    /// Bibite prefab layer layout.
    pub bibite_layout: BibiteLayout,
    /// Camera/reference defaults.
    pub camera: CameraDefaults,
    /// Skin-shader parameters.
    pub parameters: Parameters,
    /// Tint approximation mode (`multiply` or `hsv`) until the shader is known.
    pub tint_mode: TintMode,
    /// Game background style (black + the user's grid palette).
    pub background: BackgroundStyle,
    /// The cell a sheet's part resolves to, by sheet, formula cell and preferred cell ([`GameAssets::sheet_sprite_with_alpha_fallback`]).
    /// The resolution looks at the pixels of the art (and sorts the cells), which is far too much to do for every eye and arm of every
    /// bibite in every frame; its answer never changes for one set of assets.
    fallback_cache: std::sync::RwLock<HashMap<FallbackKey, SpriteRef>>,
}

/// A sheet (by atlas, cell size and grid), a formula cell and the preferred cell of [`GameAssets::sheet_sprite_with_alpha_fallback`].
type FallbackKey = ([u32; 6], u32, u32, Option<(u32, u32)>);

/// Visual constants of the game's `Shader Graphs/background` layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundStyle {
    /// Clear colour behind everything (`Camera.main.backgroundColor`).
    pub color: [u8; 4],
    /// Fine grid step in world units (`_LineStep`, default 100).
    pub fine_step: f32,
    /// Coarse grid step in world units (`_LineStep2`, default 1000).
    pub coarse_step: f32,
    /// Fine grid line colour (RGBA).
    pub fine_color: [u8; 4],
    /// Coarse grid line colour (RGBA).
    pub coarse_color: [u8; 4],
    /// The fine line's width in world units (`GridLineWidth1`, default 1).
    pub fine_width: f32,
    /// The coarse line's width in world units (the fine width times `GridLineWidth2`, default 5).
    pub coarse_width: f32,
    /// Zone-shader base colour (measured golden; `docs/design/ZONE_COLOUR.md`).
    pub zone_base: [u8; 3],
    /// The experimental background's cell colour (`UserSettings.TextureColor`) while `ShowTexture` is on.
    pub texture: Option<[u8; 4]>,
}

impl Default for BackgroundStyle {
    fn default() -> Self {
        // Values from assets/game/materials/backgroundMat.json plus the default
        // user grid settings (`GridStep1` 100, `GridStep2` 10, `GridColor1`
        // white @ 0.25, `GridColor2` white @ 0.4, `GridLineWidth1` 1 u, `GridLineWidth2` 5 x).
        Self {
            color: [0, 0, 0, 255],
            fine_step: 100.0,
            coarse_step: 1000.0,
            fine_color: [255, 255, 255, 64],
            coarse_color: [255, 255, 255, 102],
            fine_width: 1.0,
            coarse_width: 5.0,
            // Measured base_srgb from docs/design/ZONE_COLOUR.md; zone opacity
            // is derived per zone from fertility/biomass density and multiplied
            // over black.
            zone_base: [231, 154, 0],
            texture: None,
        }
    }
}

/// How sprite pixels are combined with the genome colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TintMode {
    /// Multiply sprite RGB by the colour (works for white/grayscale art).
    Multiply,
    /// Replace sprite hue with the genome colour hue, keeping sprite
    /// saturation/value (approximates a hue-shift skin shader).
    HueShift,
}

impl std::fmt::Debug for GameAssets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GameAssets")
            .field("root", &self.root)
            .field("part_sizes", &self.part_sizes.len())
            .field("egg_sizes", &self.egg_sizes.len())
            .field("pellet_sizes", &self.pellet_sizes.len())
            .field("atlases", &self.atlases.len())
            .field("sheets", &self.sheets.len())
            .field("tint_mode", &self.tint_mode)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

impl GameAssets {
    /// Default asset locations, relative to the working directory then the
    /// crate: `assets/game/` (the in-repo, gitignored/asset-license location).
    #[must_use]
    pub fn default_candidates() -> Vec<PathBuf> {
        let mut candidates = vec![PathBuf::from("assets/game")];
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("assets")
                .join("game"),
        );
        candidates
    }

    /// Finds and loads the first usable `assets/game` directory.
    ///
    /// Returns `Ok(None)` when no manifest is present (the caller should then
    /// use the built-in procedural renderer), and `Err` when a manifest exists
    /// but the asset set is incomplete/malformed (so problems are visible).
    pub fn discover() -> Result<Option<Self>, String> {
        let mut last_error = None;
        for candidate in Self::default_candidates() {
            if !candidate.join("manifest.json").is_file() {
                continue;
            }
            match Self::load(&candidate) {
                Ok(assets) => return Ok(Some(assets)),
                Err(error) => last_error = Some(format!("{}: {error}", candidate.display())),
            }
        }
        match last_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    /// Loads a manifest and all assets it references from `root`.
    pub fn load(root: &Path) -> Result<Self, String> {
        let manifest_path = root.join("manifest.json");
        let manifest_text = std::fs::read_to_string(&manifest_path)
            .map_err(|error| format!("could not read {}: {error}", manifest_path.display()))?;
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text)
            .map_err(|error| format!("invalid manifest JSON: {error}"))?;
        let base = root.to_path_buf();
        let schema = manifest
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if schema.starts_with("rustibites.assets.manifest.v") {
            return Self::load_v1(&manifest, &base);
        }

        // Text assets: manifest-provided paths first, then discovery by name.
        let text_paths = collect_text_paths(&manifest, &base);
        let part_sizes = parse_sizes(&read_text_asset(&text_paths, &["bibites/sizes", "sizes"])?)?;
        let egg_sizes = parse_sizes(&read_text_asset(
            &text_paths,
            &["objects/egg_sizes", "egg_sizes"],
        )?)?;
        let pellet_sizes = parse_sizes(&read_text_asset(
            &text_paths,
            &["objects/pellet_sizes", "pellet_sizes"],
        )?)?;
        let parts: HashMap<String, PartGrid> = parse_parts(&read_text_asset(
            &text_paths,
            &["bibites/parts", "parts", "parts.csv"],
        )?)?;
        let parameters =
            parse_parameters(&read_text_asset(&text_paths, &["parameters"]).unwrap_or_default());

        // Textures: logical name -> path. Resolve through the manifest, falling
        // back to conventional relative locations.
        let texture_paths = collect_texture_paths(&manifest, &base);
        let mut atlases: Vec<Atlas> = Vec::new();
        let mut atlas_index: HashMap<String, u16> = HashMap::new();
        let mut sheets = HashMap::new();
        let mut sheet_size_entries = Vec::new();

        for (size_index, size) in part_sizes.iter().enumerate() {
            for part in [
                BibitePart::Body,
                BibitePart::Mouth,
                BibitePart::Eyes,
                BibitePart::EyesDead,
                BibitePart::Arms,
                BibitePart::Exoskeleton,
            ] {
                let sheet_name = format!("{}_{}", size.name, part.suffix());
                let Some(grid) = parts.get(&sheet_name).copied() else {
                    continue;
                };
                let Some(atlas) = load_atlas_for(
                    &mut atlases,
                    &mut atlas_index,
                    &texture_paths,
                    &base,
                    &sheet_name,
                    &["Bibites/Parts", "bibites/parts", "sprites"],
                )?
                else {
                    continue;
                };
                let cell_width = atlas.width / grid.columns.max(1);
                let cell_height = atlas.height / grid.rows.max(1);
                if cell_width == 0 || cell_height == 0 {
                    continue;
                }
                sheets.insert(
                    (size_index, part),
                    Sheet {
                        atlas: atlas_index[&sheet_name],
                        cell: [cell_width, cell_height],
                        grid,
                        ppu: size.pixel_width as f32 / PART_PPU_DIVISOR,
                        pivot: legacy_pivot(part, None, [cell_width, cell_height]),
                        bottom_origin_rows: false,
                    },
                );
            }
            sheet_size_entries.push(size.name.clone());
        }

        let mut egg_sprites = Vec::new();
        for size in &egg_sizes {
            let name = format!("{}_egg", size.name);
            egg_sprites.push(Some(load_single_sprite(
                &mut atlases,
                &mut atlas_index,
                &texture_paths,
                &base,
                &name,
                &["Objects/Sprites", "objects/sprites", "sprites"],
                size.pixel_width as f32 / EGG_PPU_DIVISOR,
            )?));
        }
        let mut plant_sprites = Vec::new();
        let mut meat_sprites = Vec::new();
        for size in &pellet_sizes {
            let ppu = size.pixel_width as f32 / PELLET_PPU_DIVISOR;
            plant_sprites.push(Some(load_single_sprite(
                &mut atlases,
                &mut atlas_index,
                &texture_paths,
                &base,
                &format!("{}_plant", size.name),
                &["Objects/Sprites", "objects/sprites", "sprites"],
                ppu,
            )?));
            meat_sprites.push(Some(load_single_sprite(
                &mut atlases,
                &mut atlas_index,
                &texture_paths,
                &base,
                &format!("{}_meat", size.name),
                &["Objects/Sprites", "objects/sprites", "sprites"],
                ppu,
            )?));
        }

        if sheets.is_empty() {
            return Err(format!(
                "manifest found but no bibite part sheets could be resolved (tiers: {:?})",
                sheet_size_entries
            ));
        }

        let bibite_layout = parse_bibite_layout(&manifest);
        let camera = parse_camera(&manifest);
        let tint_mode = match manifest
            .get("tint_mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("multiply")
        {
            "hsv" | "hue_shift" | "hue-shift" => TintMode::HueShift,
            _ => TintMode::Multiply,
        };

        Ok(Self {
            root: base,
            part_sizes,
            egg_sizes,
            pellet_sizes,
            atlases,
            sheets,
            egg_sprites,
            plant_sprites,
            meat_sprites,
            tool_sprites: ToolSprites::default(),
            bibite_layout,
            camera,
            parameters,
            tint_mode,
            background: BackgroundStyle::default(),
            fallback_cache: std::sync::RwLock::default(),
        })
    }

    // -- procedural-sprite queries (mirroring ProceduralSpriteManager) ------

    /// `ClosestSizeIndex` for bibite parts: closest `pixelWidth[i]/pixelWidth[0]`.
    #[must_use]
    pub fn closest_part_size_index(&self, size: f32) -> usize {
        closest_size_index(&self.part_sizes, size)
    }

    /// `ClosestSizeIndex` for eggs.
    #[must_use]
    pub fn closest_egg_size_index(&self, size: f32) -> usize {
        closest_size_index(&self.egg_sizes, size)
    }

    /// `ClosestSizeIndex` for pellets.
    #[must_use]
    pub fn closest_pellet_size_index(&self, size: f32) -> usize {
        closest_size_index(&self.pellet_sizes, size)
    }

    /// `ClosestInflateIndex`.
    #[must_use]
    pub fn closest_inflate_index(&self, size_index: usize, inflate: f32) -> usize {
        let rows = self
            .sheets
            .get(&(size_index, BibitePart::Body))
            .map_or(1, |sheet| sheet.grid.rows);
        repartition_index(inflate, 0.0, 1.0, rows) as usize
    }

    /// A bibite part sheet, if extracted for this tier.
    #[must_use]
    pub fn sheet(&self, size_index: usize, part: BibitePart) -> Option<&Sheet> {
        self.sheets.get(&(size_index, part))
    }

    /// Number of atlas pixels with alpha greater than 8 inside `sprite.rect`.
    ///
    /// Runtime-named game sprites (`normal_eyes_21`, `normal_arms_10`, …) have
    /// no extracted `Sprite`/manifest entry, and some formula-selected sheet
    /// cells are almost transparent. This probe lets the viewer fall back to the
    /// nearest visually non-empty cell instead of drawing an empty quad.
    #[must_use]
    pub fn sprite_alpha_nonzero(&self, sprite: &SpriteRef) -> usize {
        let Some(atlas) = self.atlases.get(sprite.atlas as usize) else {
            return 0;
        };
        alpha_nonzero_in_rect(atlas, sprite.rect)
    }

    /// First non-empty candidate cell for a sheet: preferred cells (the live
    /// captured normal-tier cells), then the formula cell, then all cells
    /// ordered by Manhattan distance from the formula cell.
    fn sheet_sprite_with_alpha_fallback(
        &self,
        sheet: &Sheet,
        column: u32,
        row: u32,
        preferred: &[(u32, u32)],
    ) -> SpriteRef {
        // At most one preferred cell is part of the key (the only one in use is the normal tier's live eyes).
        let key = (preferred.len() <= 1).then(|| {
            (
                [
                    u32::from(sheet.atlas),
                    sheet.cell[0],
                    sheet.cell[1],
                    sheet.grid.columns,
                    sheet.grid.rows,
                    u32::from(sheet.bottom_origin_rows),
                ],
                column,
                row,
                preferred.first().copied(),
            )
        });
        if let Some(key) = key
            && let Some(hit) = self
                .fallback_cache
                .read()
                .ok()
                .and_then(|cache| cache.get(&key).copied())
        {
            return hit;
        }
        let sprite = alpha_fallback_sprite(sheet, column, row, preferred, |sprite| {
            self.sprite_alpha_nonzero(&sprite)
        });
        if let Some(key) = key
            && let Ok(mut cache) = self.fallback_cache.write()
        {
            cache.insert(key, sprite);
        }
        sprite
    }

    /// `RequestBodySprite`.
    #[must_use]
    pub fn body_sprite(&self, size_index: usize, inflate_index: usize) -> Option<SpriteRef> {
        self.sheet(size_index, BibitePart::Body)
            .map(|sheet| sheet.sprite(0, inflate_index as u32))
    }

    /// `RequestMouthSprite` (diet × strength).
    #[must_use]
    pub fn mouth_sprite(&self, size_index: usize, diet: f32, strength: f32) -> Option<SpriteRef> {
        let sheet = self.sheet(size_index, BibitePart::Mouth)?;
        let column = repartition_index(
            diet,
            Gene::Diet.def().min,
            Gene::Diet.def().max,
            sheet.grid.columns,
        );
        let row = repartition_index(strength, 0.0, 0.6, sheet.grid.rows);
        Some(sheet.sprite(column, row))
    }

    /// `RequestEyeSprite` (view angle × view radius).
    #[must_use]
    pub fn eye_sprite(
        &self,
        size_index: usize,
        view_radius: f32,
        view_angle: f32,
        dead: bool,
    ) -> Option<SpriteRef> {
        let part = if dead {
            BibitePart::EyesDead
        } else {
            BibitePart::Eyes
        };
        let sheet = self.sheet(size_index, part)?;
        let column = repartition_index(
            view_angle,
            Gene::ViewAngle.def().min,
            Gene::ViewAngle.def().max,
            sheet.grid.columns,
        );
        let row = repartition_index(
            view_radius,
            Gene::ViewRadius.def().min,
            Gene::ViewRadius.def().max,
            sheet.grid.rows,
        );
        let preferred: &[(u32, u32)] = if !dead
            && self
                .part_sizes
                .get(size_index)
                .is_some_and(|size| size.name == "normal")
        {
            // Live layer capture: `normal_eyes_21` is rect {16,9,8,9}, i.e.
            // column 2 / row 1 from the bottom (top-left PNG row 1).
            &[(2, 1)]
        } else {
            &[]
        };
        Some(self.sheet_sprite_with_alpha_fallback(sheet, column, row, preferred))
    }

    /// `RequestArmSprite` (speed ratio).
    #[must_use]
    pub fn arm_sprite(&self, size_index: usize, speed_ratio: f32) -> Option<SpriteRef> {
        let sheet = self.sheet(size_index, BibitePart::Arms)?;
        let column = repartition_index(
            speed_ratio,
            Gene::SpeedRatio.def().min,
            Gene::SpeedRatio.def().max,
            sheet.grid.columns,
        );
        // `Arms[sizeIndex][index][0]`, `index` from the speed ratio: the cell the formula gives (a normal bibite at speed ratio 1 gets column 1, `normal_arms_10`;
        // one at 2.5 gets the widest arch, column 3). Only an empty cell is replaced by its neighbour.
        Some(self.sheet_sprite_with_alpha_fallback(sheet, column, 0, &[]))
    }

    /// `RequestExoskeletonSprite` (defence proportion × inflate).
    #[must_use]
    pub fn exoskeleton_sprite(
        &self,
        size_index: usize,
        inflate_index: usize,
        defence_proportion: f32,
    ) -> Option<SpriteRef> {
        let sheet = self.sheet(size_index, BibitePart::Exoskeleton)?;
        let column = repartition_index(defence_proportion, 0.0, 0.3, sheet.grid.columns);
        Some(sheet.sprite(column, inflate_index as u32))
    }

    /// `RequestEggSprite` for a child `SizeRatio` gene.
    #[must_use]
    pub fn egg_sprite(&self, size_gene: f32) -> Option<SpriteRef> {
        let index = self.closest_egg_size_index(size_gene);
        self.egg_sprites.get(index).copied().flatten()
    }

    /// `RequestPelletSpriteOfMaterial`.
    #[must_use]
    pub fn plant_sprite(&self, size_factor_sqrt: f32) -> Option<SpriteRef> {
        let index = self.closest_pellet_size_index(size_factor_sqrt);
        self.plant_sprites.get(index).copied().flatten()
    }

    /// Meat pellet sprite.
    #[must_use]
    pub fn meat_sprite(&self, size_factor_sqrt: f32) -> Option<SpriteRef> {
        let index = self.closest_pellet_size_index(size_factor_sqrt);
        self.meat_sprites.get(index).copied().flatten()
    }
}

/// Loads the real extraction-pipeline manifest
/// (`rustibites.assets.manifest.v1`, produced by `tools/assets/extract.py`).
impl GameAssets {
    fn load_v1(manifest: &serde_json::Value, root: &Path) -> Result<Self, String> {
        let procedural = manifest
            .get("procedural")
            .ok_or_else(|| "manifest has no `procedural` section".to_owned())?;
        let text_asset = |logical: &str| -> Option<String> {
            let path = procedural
                .get("text_assets")?
                .get(logical)?
                .as_str()
                .map(|value| resolve_asset_path(root, value))?;
            std::fs::read_to_string(path).ok()
        };
        let sizes_text = text_asset("ProceduralSprites/Bibites/sizes")
            .ok_or_else(|| "missing Bibites/sizes text asset".to_owned())?;
        let egg_text = text_asset("ProceduralSprites/Objects/egg_sizes")
            .ok_or_else(|| "missing Objects/egg_sizes text asset".to_owned())?;
        let pellet_text = text_asset("ProceduralSprites/Objects/pellet_sizes")
            .ok_or_else(|| "missing Objects/pellet_sizes text asset".to_owned())?;
        let parts_text = text_asset("ProceduralSprites/Bibites/parts").unwrap_or_default();
        let parameters_text = text_asset("ProceduralSprites/parameters").unwrap_or_default();

        let part_sizes = parse_sizes(&sizes_text)?;
        let egg_sizes = parse_sizes(&egg_text)?;
        let pellet_sizes = parse_sizes(&pellet_text)?;
        let parts = procedural
            .get("parts")
            .and_then(parse_parts_json)
            .or_else(|| parse_parts(&parts_text).ok())
            .ok_or_else(|| "missing or malformed parts table".to_owned())?;
        let mut parameters = parse_parameters(&parameters_text);
        if let Some(values) = procedural.get("parameters") {
            parameters.initial_hue =
                json_f32(values, "InitialHue").unwrap_or(parameters.initial_hue);
            parameters.vmin_at_s0 = json_f32(values, "VminAtS0").unwrap_or(parameters.vmin_at_s0);
            parameters.vmax_at_s0 = json_f32(values, "VmaxAtS0").unwrap_or(parameters.vmax_at_s0);
        }

        let mut atlases: Vec<Atlas> = Vec::new();
        let mut atlas_index: HashMap<String, u16> = HashMap::new();
        let mut sheets = HashMap::new();
        let mut mouth_cells: HashMap<usize, [u32; 2]> = HashMap::new();

        for part in [
            BibitePart::Body,
            BibitePart::Mouth,
            BibitePart::Eyes,
            BibitePart::EyesDead,
            BibitePart::Arms,
            BibitePart::Exoskeleton,
        ] {
            for (size_index, size) in part_sizes.iter().enumerate() {
                let sheet_name = format!("{}_{}", size.name, part.suffix());
                let Some(grid) = parts.get(&sheet_name).copied() else {
                    continue;
                };
                let Some(atlas_index) = load_atlas_v1(
                    &mut atlases,
                    &mut atlas_index,
                    root,
                    manifest,
                    &sheet_name,
                    false,
                )?
                else {
                    continue;
                };
                let atlas = &atlases[atlas_index as usize];
                let cell = [
                    atlas.width / grid.columns.max(1),
                    atlas.height / grid.rows.max(1),
                ];
                if cell[0] == 0 || cell[1] == 0 {
                    continue;
                }
                if part == BibitePart::Mouth {
                    mouth_cells.insert(size_index, cell);
                }
                let pivot = sheet_pivot_v1(
                    manifest,
                    &sheet_name,
                    part,
                    mouth_cells.get(&size_index).copied(),
                    cell,
                );
                sheets.insert(
                    (size_index, part),
                    Sheet {
                        atlas: atlas_index,
                        cell,
                        grid,
                        ppu: size.pixel_width as f32 / PART_PPU_DIVISOR,
                        pivot,
                        bottom_origin_rows: true,
                    },
                );
            }
        }
        if sheets.is_empty() {
            return Err("no bibite part sheets resolved from the manifest".to_owned());
        }

        let mut egg_sprites = Vec::with_capacity(egg_sizes.len());
        for size in &egg_sizes {
            let name = format!("{}_egg", size.name);
            egg_sprites.push(load_object_sprite_v1(
                &mut atlases,
                &mut atlas_index,
                root,
                manifest,
                &name,
                size.pixel_width as f32 / EGG_PPU_DIVISOR,
                false,
            )?);
        }
        let mut plant_sprites = Vec::with_capacity(pellet_sizes.len());
        let mut meat_sprites = Vec::with_capacity(pellet_sizes.len());
        for size in &pellet_sizes {
            let ppu = size.pixel_width as f32 / PELLET_PPU_DIVISOR;
            plant_sprites.push(load_object_sprite_v1(
                &mut atlases,
                &mut atlas_index,
                root,
                manifest,
                &format!("{}_plant", size.name),
                ppu,
                false,
            )?);
            meat_sprites.push(load_object_sprite_v1(
                &mut atlases,
                &mut atlas_index,
                root,
                manifest,
                &format!("{}_meat", size.name),
                ppu,
                false,
            )?);
        }

        // The prefabs' sprites are 1 pixel per unit (35 or 45 units across).
        let mut tool_sprite = |name: &str| -> Result<Option<SpriteRef>, String> {
            load_object_sprite_v1(
                &mut atlases,
                &mut atlas_index,
                root,
                manifest,
                name,
                1.0,
                false,
            )
        };
        let mut tool_sprites = ToolSprites {
            base: tool_sprite("laser_circle")?,
            rad_icon: tool_sprite("radioactive")?,
            phero: tool_sprite("pheroTower")?,
            crystal: tool_sprite("laser_crystal")?,
            crystal_hues: None,
        };
        // The colour killer's crystal at every hue, one cell per degree stacked in one atlas (the shader recolours the art; its `_color` is the killer's hue).
        if let Some(crystal) = tool_sprites.crystal {
            let art = &atlases[crystal.atlas as usize];
            let (w, h) = (art.width, art.height);
            let mut rgba = Vec::with_capacity((w * h * 4 * CRYSTAL_HUES) as usize);
            for degree in 0..CRYSTAL_HUES {
                rgba.extend(crystal_pixels(art, degree as f32 / CRYSTAL_HUES as f32));
            }
            atlases.push(Atlas {
                name: "laser_crystal_hues".to_owned(),
                width: w,
                height: h * CRYSTAL_HUES,
                rgba,
            });
            tool_sprites.crystal_hues = Some(SpriteRef {
                atlas: (atlases.len() - 1) as u16,
                rect: [0, 0, w, h],
                ..crystal
            });
        }
        // The radiation tower's icon shows as its tint, (127, 148, 0): the sprite's own colour (a dark grey, 38) does not multiply into it, so the icon
        // is a white mask here and the tint colours it.
        if let Some(icon) = tool_sprites.rad_icon {
            for pixel in atlases[icon.atlas as usize].rgba.chunks_exact_mut(4) {
                pixel[..3].fill(255);
            }
        }

        Ok(Self {
            root: root.to_path_buf(),
            part_sizes,
            egg_sizes,
            pellet_sizes,
            atlases,
            sheets,
            egg_sprites,
            plant_sprites,
            meat_sprites,
            tool_sprites,
            bibite_layout: load_bibite_prefab(root),
            camera: CameraDefaults {
                orthographic_size: DEFAULT_ORTHOGRAPHIC_SIZE,
                background_color: [0.0, 0.0, 0.0, 1.0],
            },
            parameters,
            // The game's skin/eye shaders replace the sprite hue with the
            // genome colour hue while keeping the art's saturation/value
            // (verified against assets/game/reference/bibite_closeup.png).
            tint_mode: TintMode::HueShift,
            background: BackgroundStyle::default(),
            fallback_cache: std::sync::RwLock::default(),
        })
    }
}

/// Loads a whole sheet texture (grayscale-converted when `grayscale`) and
/// returns its atlas index, or `None` when the texture was not extracted.
fn load_atlas_v1(
    atlases: &mut Vec<Atlas>,
    index: &mut HashMap<String, u16>,
    root: &Path,
    manifest: &serde_json::Value,
    logical_name: &str,
    grayscale: bool,
) -> Result<Option<u16>, String> {
    let key = normalize_key(logical_name);
    if let Some(existing) = index.get(&key) {
        return Ok(Some(*existing));
    }
    let Some(path) = find_texture_v1(root, manifest, logical_name) else {
        return Ok(None);
    };
    let mut atlas = decode_png(&path)?.with_name(logical_name);
    if grayscale {
        grayscale_atlas(&mut atlas);
    }
    atlases.push(atlas);
    let slot = (atlases.len() - 1) as u16;
    index.insert(key, slot);
    Ok(Some(slot))
}

/// Loads a single-frame object sprite (egg/plant/meat) from the manifest.
fn load_object_sprite_v1(
    atlases: &mut Vec<Atlas>,
    index: &mut HashMap<String, u16>,
    root: &Path,
    manifest: &serde_json::Value,
    logical_name: &str,
    ppu: f32,
    grayscale: bool,
) -> Result<Option<SpriteRef>, String> {
    let Some(atlas_index) = load_atlas_v1(atlases, index, root, manifest, logical_name, grayscale)?
    else {
        return Ok(None);
    };
    let atlas = &atlases[atlas_index as usize];
    Ok(Some(SpriteRef {
        atlas: atlas_index,
        rect: [0, 0, atlas.width, atlas.height],
        ppu,
        pivot: Vec2::splat(0.5),
    }))
}

/// Resolves a path from the manifest (`assets/game/...`) against the asset root.
fn resolve_asset_path(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if let Ok(rest) = path.strip_prefix("assets/game") {
        return root.join(rest);
    }
    let candidate = root.join(path);
    if candidate.exists() {
        return candidate;
    }
    path.to_path_buf()
}

/// Finds an extracted texture by logical name, checking the manifest, the
/// standard `sheets/`+`sprites/` folders, the per-asset UI folders and finally
/// a recursive search (some shared UI textures live under `ui/...`).
fn find_texture_v1(
    root: &Path,
    manifest: &serde_json::Value,
    logical_name: &str,
) -> Option<PathBuf> {
    if let Some(file) = manifest
        .get("textures")
        .and_then(|textures| textures.get(logical_name))
        .and_then(|entry| entry.get("file"))
        .and_then(|file| file.as_str())
    {
        let path = resolve_asset_path(root, file);
        if path.is_file() {
            return Some(path);
        }
    }
    for subdirectory in ["sheets", "sprites"] {
        let path = root.join(subdirectory).join(format!("{logical_name}.png"));
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(ui) = manifest.get("ui").and_then(|ui| ui.as_object()) {
        for asset in ui.keys() {
            for subdirectory in ["sheets", "sprites"] {
                let path = root
                    .join("ui")
                    .join(asset)
                    .join(subdirectory)
                    .join(format!("{logical_name}.png"));
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    find_file_recursive(root, &format!("{logical_name}.png"))
}

/// Sprite pivot from the manifest's per-sprite metadata (Unity's: bottom-left
/// origin, y up), with the game's documented defaults as a fallback.
fn sheet_pivot_v1(
    manifest: &serde_json::Value,
    sheet_name: &str,
    part: BibitePart,
    mouth_cell: Option<[u32; 2]>,
    eyes_cell: [u32; 2],
) -> Vec2 {
    if let Some(pivot) = manifest
        .get("sheets")
        .and_then(|sheets| sheets.get(sheet_name))
        .and_then(|entries| entries.as_array())
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("pivot"))
        .and_then(json_vec2)
    {
        // The extraction manifest stores the SpriteRenderer pivot as Unity has
        // it, normalised (`x/width`, `y/height`, y counted from the BOTTOM): the
        // arm pivot is (1.0, 0.0), its bottom right corner (the live
        // `normal_arms_10` is (9, 0) px). It is kept as it is here; the
        // renderers count from the top, and `sprites::drawn_pivot` converts
        // it per layer by the layer's flips.
        return Vec2::new(pivot.x, pivot.y);
    }
    legacy_pivot(part, mouth_cell, eyes_cell)
}

/// Loads the game's bibite prefab hierarchy from `assets/game/prefabs.json`.
///
/// Gives exact child local positions/rotations/scales, renderer flips and the
/// z-based layering (Unity passes: farther sprites first). Falls back to
/// [`BibiteLayout::default`] when the file is missing.
fn load_bibite_prefab(root: &Path) -> BibiteLayout {
    let path = root.join("prefabs.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return BibiteLayout::default();
    };
    let Ok(document) = serde_json::from_str::<serde_json::Value>(&text) else {
        return BibiteLayout::default();
    };
    let Some(roots) = document.get("roots").and_then(|roots| roots.as_array()) else {
        return BibiteLayout::default();
    };
    let Some(bibite) = roots
        .iter()
        .find(|root| root.get("name").and_then(|name| name.as_str()) == Some("bibite"))
    else {
        return BibiteLayout::default();
    };
    let Some(tree) = bibite.get("tree").and_then(|tree| tree.as_array()) else {
        return BibiteLayout::default();
    };
    let mut layers = Vec::new();
    for (index, node) in tree.iter().enumerate() {
        if node
            .get("depth")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            != 1
        {
            continue;
        }
        let Some(name) = node.get("name").and_then(|name| name.as_str()) else {
            continue;
        };
        // The prefab has separate Left Arm / RightArm renderers.
        let part = if name.eq_ignore_ascii_case("left arm") || name.eq_ignore_ascii_case("rightarm")
        {
            Some(BibitePart::Arms)
        } else {
            part_from_layer_name(name)
        };
        let Some(part) = part else {
            continue;
        };
        let position = node
            .get("local_position")
            .and_then(|value| value.as_array())
            .map_or(Vec2::ZERO, |array| {
                Vec2::new(
                    array
                        .first()
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0) as f32,
                    array
                        .get(1)
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0) as f32,
                )
            });
        let z = node
            .get("local_position")
            .and_then(|value| value.as_array())
            .and_then(|array| array.get(2))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;
        let rotation = node
            .get("local_rotation")
            .and_then(|value| value.as_array())
            .and_then(|array| quaternion_z_to_angle(array))
            .unwrap_or(0.0);
        let scale = node
            .get("local_scale")
            .and_then(|value| value.as_array())
            .and_then(|array| array.first())
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0) as f32;
        let renderer = node.get("sprite_renderer");
        let flip_x = renderer
            .and_then(|renderer| renderer.get("flip_x"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let flip_y = renderer
            .and_then(|renderer| renderer.get("flip_y"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let color = renderer
            .and_then(|renderer| renderer.get("color"))
            .and_then(json_rgba)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        layers.push((
            index,
            part,
            LayerLayout {
                local_position: position,
                local_rotation: rotation,
                local_scale: scale,
                sorting_order: 0,
                color,
                flip_x,
                flip_y,
            },
            z,
        ));
    }
    // Unity painter order: larger z is farther from the camera (camera is at
    // negative z), so draw by descending z; equal z keeps hierarchy order.
    layers.sort_by(|a, b| {
        b.3.partial_cmp(&a.3)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    BibiteLayout {
        layers: layers
            .into_iter()
            .map(|(_, part, mut layer, z)| {
                layer.sorting_order = (z * 100.0) as i32;
                (part, layer)
            })
            .collect(),
    }
}

/// Unity serialises 2D rotations as a quaternion; the dump keeps `(x, y, z)`
/// and drops `w` (reconstructible for unit quaternions). Returns radians.
fn quaternion_z_to_angle(value: &[serde_json::Value]) -> Option<f32> {
    let x = value
        .first()
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let y = value
        .get(1)
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let z = value
        .get(2)
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let remainder = 1.0 - x * x - y * y - z * z;
    if remainder < -1e-3 {
        return None;
    }
    let w = remainder.max(0.0).sqrt();
    Some((2.0 * z.atan2(w)) as f32)
}
/// The game's default pivots (`ProceduralSpriteManager`) in top-left space.
/// Counts pixels with alpha greater than 8 inside a top-left-origin rect.
fn alpha_nonzero_in_rect(atlas: &Atlas, rect: [u32; 4]) -> usize {
    let [x, y, width, height] = rect;
    let x0 = x as usize;
    let y0 = y as usize;
    let x1 = (x.saturating_add(width) as usize).min(atlas.width as usize);
    let y1 = (y.saturating_add(height) as usize).min(atlas.height as usize);
    if x0 >= x1 || y0 >= y1 {
        return 0;
    }
    let mut count = 0;
    for row in y0..y1 {
        for column in x0..x1 {
            let offset = (row * atlas.width as usize + column) * 4;
            if atlas.rgba.get(offset + 3).is_some_and(|&alpha| alpha > 8) {
                count += 1;
            }
        }
    }
    count
}

/// Picks the first non-empty candidate cell: preferred live-captured cells,
/// then the formula cell, then cells by Manhattan distance from it.
fn alpha_fallback_sprite(
    sheet: &Sheet,
    column: u32,
    row: u32,
    preferred: &[(u32, u32)],
    alpha_nonzero: impl Fn(SpriteRef) -> usize,
) -> SpriteRef {
    let mut cells: Vec<(u32, u32)> =
        Vec::with_capacity((sheet.grid.rows * sheet.grid.columns) as usize + preferred.len());
    for &(candidate_column, candidate_row) in preferred {
        if candidate_column < sheet.grid.columns && candidate_row < sheet.grid.rows {
            cells.push((candidate_column, candidate_row));
        }
    }
    cells.push((column, row));
    let mut rest: Vec<(u32, u32)> = Vec::new();
    for candidate_row in 0..sheet.grid.rows {
        for candidate_column in 0..sheet.grid.columns {
            let cell = (candidate_column, candidate_row);
            if !cells.contains(&cell) {
                rest.push(cell);
            }
        }
    }
    rest.sort_by_key(|&(candidate_column, candidate_row)| {
        (
            candidate_column.abs_diff(column) + candidate_row.abs_diff(row),
            candidate_row,
            candidate_column,
        )
    });
    cells.extend(rest);
    for (candidate_column, candidate_row) in cells {
        let candidate = sheet.sprite(candidate_column, candidate_row);
        if alpha_nonzero(candidate) > 0 {
            return candidate;
        }
    }
    sheet.sprite(column, row)
}

fn legacy_pivot(part: BibitePart, mouth_cell: Option<[u32; 2]>, eyes_cell: [u32; 2]) -> Vec2 {
    match part {
        BibitePart::Body => Vec2::new(1.0, 0.5),
        BibitePart::Mouth => Vec2::new(0.0, 0.5),
        BibitePart::Arms => Vec2::new(1.0, 0.0),
        BibitePart::Eyes | BibitePart::EyesDead | BibitePart::Exoskeleton => {
            let eyes_width = eyes_cell[0].max(1) as f32;
            let mouth_width = mouth_cell.map_or(eyes_width * 0.625, |cell| cell[0] as f32);
            Vec2::new((1.0 - mouth_width / eyes_width).clamp(0.0, 1.0), 0.5)
        }
    }
}

/// `procedural.parts` JSON map (`name -> {rows, columns}`).
fn parse_parts_json(value: &serde_json::Value) -> Option<HashMap<String, PartGrid>> {
    let object = value.as_object()?;
    let mut parts = HashMap::new();
    for (name, entry) in object {
        let rows = entry.get("rows").and_then(serde_json::Value::as_u64)? as u32;
        let columns = entry.get("columns").and_then(serde_json::Value::as_u64)? as u32;
        parts.insert(
            name.clone(),
            PartGrid {
                rows: rows.max(1),
                columns: columns.max(1),
            },
        );
    }
    (!parts.is_empty()).then_some(parts)
}

/// Converts a blue-tinted procedural sprite into a luminance mask so the
/// per-entity tint controls hue while the art supplies shading.
fn grayscale_atlas(atlas: &mut Atlas) {
    for pixel in atlas.rgba.chunks_exact_mut(4) {
        let luma = (0.299 * f32::from(pixel[0])
            + 0.587 * f32::from(pixel[1])
            + 0.114 * f32::from(pixel[2]))
        .round()
        .clamp(0.0, 255.0) as u8;
        pixel[0] = luma;
        pixel[1] = luma;
        pixel[2] = luma;
    }
}

fn json_f32(value: &serde_json::Value, key: &str) -> Option<f32> {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .map(|v| v as f32)
}

// ---------------------------------------------------------------------------
// Manifest parsing helpers
// ---------------------------------------------------------------------------

fn collect_text_paths(manifest: &serde_json::Value, base: &Path) -> HashMap<String, PathBuf> {
    let mut paths = HashMap::new();
    let Some(text_files) = manifest
        .get("text_files")
        .and_then(|value| value.as_object())
    else {
        return paths;
    };
    for (logical, value) in text_files {
        let Some(path) = value.as_str() else {
            continue;
        };
        paths.insert(normalize_key(logical), base.join(path));
    }
    paths
}

fn collect_texture_paths(manifest: &serde_json::Value, base: &Path) -> HashMap<String, PathBuf> {
    let mut paths = HashMap::new();
    // Object form: {"name": "path" | {"path": "..."}}
    if let Some(textures) = manifest.get("textures").and_then(|value| value.as_object()) {
        for (logical, value) in textures {
            let path = value
                .as_str()
                .or_else(|| value.get("path").and_then(|path| path.as_str()));
            if let Some(path) = path {
                paths.insert(normalize_key(logical), base.join(path));
            }
        }
    }
    // Array form: [{"name": "...", "path": "..."}]
    if let Some(textures) = manifest.get("textures").and_then(|value| value.as_array()) {
        for entry in textures {
            let name = entry.get("name").and_then(|name| name.as_str());
            let path = entry
                .get("path")
                .or_else(|| entry.get("file"))
                .and_then(|path| path.as_str());
            if let (Some(name), Some(path)) = (name, path) {
                paths.insert(normalize_key(name), base.join(path));
            }
        }
    }
    // Alternate top-level names.
    for key in ["sprites", "images", "files"] {
        if let Some(textures) = manifest.get(key).and_then(|value| value.as_object()) {
            for (logical, value) in textures {
                let path = value
                    .as_str()
                    .or_else(|| value.get("path").and_then(|path| path.as_str()));
                if let Some(path) = path {
                    paths
                        .entry(normalize_key(logical))
                        .or_insert_with(|| base.join(path));
                }
            }
        }
    }
    paths
}

fn read_text_asset(
    text_paths: &HashMap<String, PathBuf>,
    candidates: &[&str],
) -> Result<String, String> {
    for candidate in candidates {
        let key = normalize_key(candidate);
        if let Some(path) = text_paths.get(&key) {
            return std::fs::read_to_string(path)
                .map_err(|error| format!("could not read {}: {error}", path.display()));
        }
        // Also accept any logical name ending in the candidate (e.g.
        // "ProceduralSprites/Bibites/sizes" for "bibites/sizes").
        for (logical, path) in text_paths {
            if logical.ends_with(&format!("/{key}")) || logical == &key {
                return std::fs::read_to_string(path)
                    .map_err(|error| format!("could not read {}: {error}", path.display()));
            }
        }
    }
    Err(format!(
        "missing text asset (looked for {candidates:?}); manifest text_files has: {:?}",
        text_paths.keys().collect::<Vec<_>>()
    ))
}

fn load_atlas_for(
    atlases: &mut Vec<Atlas>,
    index: &mut HashMap<String, u16>,
    texture_paths: &HashMap<String, PathBuf>,
    base: &Path,
    logical_name: &str,
    subdirs: &[&str],
) -> Result<Option<Atlas>, String> {
    let key = normalize_key(logical_name);
    if let Some(&existing) = index.get(&key) {
        return Ok(Some(atlases[existing as usize].clone()));
    }
    let path = resolve_texture(texture_paths, base, logical_name, subdirs);
    let Some(path) = path else {
        return Ok(None);
    };
    let atlas = decode_png(&path)?.with_name(logical_name);
    atlases.push(atlas.clone());
    index.insert(key, (atlases.len() - 1) as u16);
    Ok(Some(atlas))
}

fn load_single_sprite(
    atlases: &mut Vec<Atlas>,
    index: &mut HashMap<String, u16>,
    texture_paths: &HashMap<String, PathBuf>,
    base: &Path,
    logical_name: &str,
    subdirs: &[&str],
    ppu: f32,
) -> Result<SpriteRef, String> {
    let Some(atlas) = load_atlas_for(atlases, index, texture_paths, base, logical_name, subdirs)?
    else {
        return Err(format!(
            "missing sprite '{}' (looked in {:?} and manifest textures)",
            logical_name, subdirs
        ));
    };
    Ok(SpriteRef {
        atlas: index[&normalize_key(logical_name)],
        rect: [0, 0, atlas.width, atlas.height],
        ppu,
        pivot: Vec2::splat(0.5),
    })
}

fn resolve_texture(
    texture_paths: &HashMap<String, PathBuf>,
    base: &Path,
    logical_name: &str,
    subdirs: &[&str],
) -> Option<PathBuf> {
    let key = normalize_key(logical_name);
    if let Some(path) = texture_paths.get(&key) {
        if path.is_file() {
            return Some(path.clone());
        }
    }
    // Accept names that include the directory prefix in the manifest.
    for (logical, path) in texture_paths {
        if (logical == &key || logical.ends_with(&format!("/{key}"))) && path.is_file() {
            return Some(path.clone());
        }
    }
    // Conventional layout discovery: <root>/<subdir>/<logical_name>.png.
    for subdir in subdirs {
        let path = base.join(subdir).join(format!("{logical_name}.png"));
        if path.is_file() {
            return Some(path);
        }
    }
    // Last resort: walk the asset root for the file name.
    find_file_recursive(base, &format!("{logical_name}.png"))
}

fn find_file_recursive(root: &Path, file_name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, file_name) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(file_name))
        {
            return Some(path);
        }
    }
    None
}

impl Atlas {
    fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_owned();
        self
    }
}

fn decode_png(path: &Path) -> Result<Atlas, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("PNG header error in {}: {error}", path.display()))?;
    let mut buffer = vec![0_u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("PNG decode error in {}: {error}", path.display()))?;
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
        png::ColorType::Indexed => {
            return Err(format!(
                "indexed PNG {} is not supported (re-export as RGBA)",
                path.display()
            ));
        }
    };
    if rgba.len() != (info.width * info.height * 4) as usize {
        return Err(format!(
            "unexpected PNG layout in {} ({:?} {}x{})",
            path.display(),
            info.color_type,
            info.width,
            info.height
        ));
    }
    Ok(Atlas {
        name: String::new(),
        width: info.width,
        height: info.height,
        rgba,
    })
}

// ---------------------------------------------------------------------------
// CSV parsing (semicolon-separated, as the game ships them)
// ---------------------------------------------------------------------------

/// Parses `sizes.csv` (`name;pixelWidth`, first line is a header).
pub fn parse_sizes(text: &str) -> Result<Vec<SizeEntry>, String> {
    let mut sizes = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line_index == 0 {
            continue;
        }
        let fields = split_csv_line(line);
        if fields.len() < 2 || fields[0].is_empty() {
            continue;
        }
        let pixel_width: u32 = fields[1].trim().parse().map_err(|error| {
            format!("invalid pixelWidth in sizes.csv line {line_index}: {error}")
        })?;
        sizes.push(SizeEntry {
            name: fields[0].trim().to_owned(),
            pixel_width,
        });
    }
    if sizes.is_empty() {
        return Err("sizes CSV had no rows".to_owned());
    }
    Ok(sizes)
}

/// Parses `parts.csv` (`name;nRows;nColumns`, first line is a header).
pub fn parse_parts(text: &str) -> Result<HashMap<String, PartGrid>, String> {
    let mut parts = HashMap::new();
    for (line_index, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line_index == 0 {
            continue;
        }
        let fields = split_csv_line(line);
        if fields.len() < 3 || fields[0].is_empty() {
            continue;
        }
        let rows: u32 = fields[1]
            .trim()
            .parse()
            .map_err(|error| format!("invalid nRows in parts.csv line {line_index}: {error}"))?;
        let columns: u32 = fields[2]
            .trim()
            .parse()
            .map_err(|error| format!("invalid nColumns in parts.csv line {line_index}: {error}"))?;
        parts.insert(
            fields[0].trim().to_owned(),
            PartGrid {
                rows: rows.max(1),
                columns: columns.max(1),
            },
        );
    }
    if parts.is_empty() {
        return Err("parts CSV had no rows".to_owned());
    }
    Ok(parts)
}

/// Parses `parameters.csv` (`name;value`, first line is a header).
pub fn parse_parameters(text: &str) -> Parameters {
    let mut parameters = Parameters::default();
    for line in text.lines().skip(1) {
        let fields = split_csv_line(line.trim());
        if fields.len() < 2 {
            continue;
        }
        let value: f32 = fields[1].trim().replace(',', ".").parse().unwrap_or(0.0);
        match fields[0].trim() {
            "InitialHue" => parameters.initial_hue = value,
            "VminAtS0" => parameters.vmin_at_s0 = value,
            "VmaxAtS0" => parameters.vmax_at_s0 = value,
            _ => {}
        }
    }
    parameters
}

fn split_csv_line(line: &str) -> Vec<&str> {
    let separator = if line.contains(';') {
        ';'
    } else if line.contains(',') {
        ','
    } else {
        ';'
    };
    line.split(separator).collect()
}

// ---------------------------------------------------------------------------
// Prefab / camera metadata
// ---------------------------------------------------------------------------

fn parse_bibite_layout(manifest: &serde_json::Value) -> BibiteLayout {
    let Some(layers) = manifest
        .pointer("/prefabs/bibite/layers")
        .or_else(|| manifest.pointer("/prefabs/bibite/children"))
        .and_then(|value| value.as_array())
    else {
        return BibiteLayout::default();
    };
    let mut layout = BibiteLayout::default();
    let mut parsed = Vec::new();
    for layer in layers {
        let Some(name) = layer.get("name").and_then(|name| name.as_str()) else {
            continue;
        };
        let Some(part) = part_from_layer_name(name) else {
            continue;
        };
        let local_position = layer
            .get("local_position")
            .or_else(|| layer.get("localPosition"))
            .and_then(json_vec2)
            .unwrap_or(Vec2::ZERO);
        let local_rotation = layer
            .get("local_rotation_deg")
            .or_else(|| layer.get("localRotationDeg"))
            .and_then(serde_json::Value::as_f64)
            .map_or(0.0, |degrees| degrees.to_radians() as f32);
        let local_scale = layer
            .get("local_scale")
            .or_else(|| layer.get("localScale"))
            .and_then(serde_json::Value::as_f64)
            .map_or(1.0, |scale| scale as f32);
        let sorting_order = layer
            .get("sorting_order")
            .or_else(|| layer.get("sortingOrder"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32;
        let color = layer
            .get("color")
            .and_then(json_rgba)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let flip_x = layer
            .get("flip_x")
            .or_else(|| layer.get("flipX"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let flip_y = layer
            .get("flip_y")
            .or_else(|| layer.get("flipY"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        parsed.push((
            part,
            LayerLayout {
                local_position,
                local_rotation,
                local_scale,
                sorting_order,
                color,
                flip_x,
                flip_y,
            },
        ));
    }
    if !parsed.is_empty() {
        parsed.sort_by_key(|(_, layer)| layer.sorting_order);
        layout.layers = parsed;
    }
    layout
}

fn part_from_layer_name(name: &str) -> Option<BibitePart> {
    let name = name.to_ascii_lowercase().replace([' ', '_', '-'], "");
    match name.as_str() {
        "leftarm" | "rightarm" | "arm" | "arms" => Some(BibitePart::Arms),
        "body" => Some(BibitePart::Body),
        "mouth" => Some(BibitePart::Mouth),
        "eyes" => Some(BibitePart::Eyes),
        "eyesdead" => Some(BibitePart::EyesDead),
        "exoskelton" | "exoskeleton" => Some(BibitePart::Exoskeleton),
        _ => None,
    }
}

fn parse_camera(manifest: &serde_json::Value) -> CameraDefaults {
    let mut camera = CameraDefaults::default();
    if let Some(size) = manifest
        .pointer("/camera/orthographic_size")
        .or_else(|| manifest.pointer("/camera/orthographicSize"))
        .and_then(serde_json::Value::as_f64)
    {
        if size > 0.0 {
            camera.orthographic_size = size as f32;
        }
    }
    if let Some(color) = manifest
        .pointer("/camera/background_color")
        .and_then(json_rgba)
    {
        camera.background_color = color;
    }
    camera
}

fn json_vec2(value: &serde_json::Value) -> Option<Vec2> {
    let array = value.as_array()?;
    if array.len() < 2 {
        return None;
    }
    Some(Vec2::new(
        array[0].as_f64()? as f32,
        array[1].as_f64()? as f32,
    ))
}

fn json_rgba(value: &serde_json::Value) -> Option<[f32; 4]> {
    let array = value.as_array()?;
    if array.len() < 3 {
        return None;
    }
    Some([
        array[0].as_f64()? as f32,
        array[1].as_f64()? as f32,
        array[2].as_f64()? as f32,
        array
            .get(3)
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0) as f32,
    ])
}

fn normalize_key(key: &str) -> String {
    key.trim()
        .trim_start_matches("./")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// Game formula helpers (mirroring ProceduralSpriteManager)
// ---------------------------------------------------------------------------

/// `FindRepartitionIndex`: `floor(n * (v-min)/(max-min))` clamped to `0..n-1`.
#[must_use]
pub fn repartition_index(value: f32, min: f32, max: f32, n: u32) -> u32 {
    if n <= 1 || max.is_nan() || min.is_nan() || max <= min {
        return 0;
    }
    let normalized = (value - min) / (max - min);
    let index = (n as f32 * normalized).floor();
    index.clamp(0.0, (n - 1) as f32) as u32
}

/// `ProceduralSpriteManager.ClosestSizeIndex`: closest pixel-width ratio.
fn closest_size_index(sizes: &[SizeEntry], size: f32) -> usize {
    let Some(first) = sizes.first() else {
        return 0;
    };
    let base = first.pixel_width as f32;
    let mut best = 0;
    let mut best_distance = f32::INFINITY;
    for (index, entry) in sizes.iter().enumerate() {
        let distance = (size - entry.pixel_width as f32 / base).abs();
        if distance < best_distance {
            best_distance = distance;
            best = index;
        }
    }
    best
}

/// `BibiteGenes.GenesToColor`: maps the three pigment genes to the skin colour
/// the game's material `_Color` receives (the simulation's own function: the colour killer uses it too).
#[must_use]
pub fn genes_to_color(red: f32, green: f32, blue: f32) -> [f32; 3] {
    bibites_core::genome::genes_to_color(red, green, blue)
}

/// `BibiteGenes.GetBodyColor`: pigment genes (indices 5/6/7) → skin colour.
#[must_use]
pub fn body_color(genes: &[f32; N_GENES]) -> [f32; 3] {
    genes_to_color(genes[5], genes[6], genes[7])
}

/// `BibiteGenes.GetEyeColor`: body hue shifted by the `EyeOffset` gene, fully
/// saturated/bright (the eye shader's colour with `_saturationOverride` 1).
#[must_use]
pub fn eye_color(genes: &[f32; N_GENES]) -> [f32; 3] {
    crate::skin::eye_shader_color(body_color(genes), genes[25], 1.0)
}

/// `TotalOrganWAG`: sum of non-negative WAG genes 26..=32.
#[must_use]
pub fn total_organ_wag(genes: &[f32; N_GENES]) -> f32 {
    genes[26..=32].iter().map(|value| value.max(0.0)).sum()
}

/// RGB in `0..=1` → `(hue, saturation, value)` (Unity `Color.RGBToHSV`).
#[must_use]
pub fn rgb_to_hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if (max - r).abs() <= f32::EPSILON {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if (max - g).abs() <= f32::EPSILON {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    let saturation = if max <= f32::EPSILON {
        0.0
    } else {
        delta / max
    };
    (hue, saturation, max)
}

/// `(hue, saturation, value)` → RGB in `0..=1` (Unity `Color.HSVToRGB`).
#[must_use]
pub fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let h = (hue.rem_euclid(1.0)) * 6.0;
    let sector = h.floor() as i32;
    let fraction = h - h.floor();
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    match sector {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_game_csv_assets() {
        let sizes = parse_sizes("name;pixelWidth\nsmall;100\nmedium;200\nbig;300\n").unwrap();
        assert_eq!(sizes.len(), 3);
        assert_eq!(
            sizes[1],
            SizeEntry {
                name: "medium".into(),
                pixel_width: 200
            }
        );
        let parts = parse_parts("name;nRows;nColumns\nsmall_body;3;1\nsmall_arms;1;5\n").unwrap();
        assert_eq!(
            parts["small_body"],
            PartGrid {
                rows: 3,
                columns: 1
            }
        );
        assert_eq!(parts["small_arms"].columns, 5);
        let parameters = parse_parameters("name;value\nInitialHue;0,55\nVminAtS0;0\nVmaxAtS0;1\n");
        assert!((parameters.initial_hue - 0.55).abs() < 1e-6);
    }

    #[test]
    fn size_index_matches_game_formula() {
        let sizes = vec![
            SizeEntry {
                name: "s".into(),
                pixel_width: 100,
            },
            SizeEntry {
                name: "m".into(),
                pixel_width: 200,
            },
            SizeEntry {
                name: "l".into(),
                pixel_width: 300,
            },
        ];
        assert_eq!(closest_size_index(&sizes, 1.0), 0);
        assert_eq!(closest_size_index(&sizes, 1.9), 1);
        assert_eq!(closest_size_index(&sizes, 3.0), 2);
        assert_eq!(repartition_index(0.0, 0.0, 1.0, 5), 0);
        assert_eq!(repartition_index(1.0, 0.0, 1.0, 5), 4);
        assert_eq!(repartition_index(0.5, 0.0, 1.0, 5), 2);
    }

    #[test]
    fn body_colour_matches_game_genes_to_color() {
        // `GenesToColor` of equal pigments is a light grey (`1 - minimum`), not the dark raw pigment: the basic
        // bibite (0.1, 0.1, 0.1) is 0.9 grey.
        assert_eq!(genes_to_color(0.1, 0.1, 0.1), [0.9, 0.9, 0.9]);
        // A pure red pigment dominates red.
        let red = genes_to_color(1.0, 0.1, 0.1);
        assert!(red[0] > 0.5 && red[1] < 0.5 && red[2] < 0.5, "{red:?}");
        let green = genes_to_color(0.1, 1.0, 0.1);
        assert!(green[1] > 0.5, "{green:?}");
    }

    #[test]
    fn hsv_round_trips_primary_colours() {
        for (rgb, expected_hue) in [
            ([1.0, 0.0, 0.0], 0.0),
            ([0.0, 1.0, 0.0], 1.0 / 3.0),
            ([0.0, 0.0, 1.0], 2.0 / 3.0),
        ] {
            let (hue, saturation, value) = rgb_to_hsv(rgb);
            assert!((hue - expected_hue).abs() < 1e-5, "{rgb:?}: hue {hue}");
            assert!((saturation - 1.0).abs() < 1e-5);
            assert!((value - 1.0).abs() < 1e-5);
            let back = hsv_to_rgb(hue, saturation, value);
            for channel in 0..3 {
                assert!((back[channel] - rgb[channel]).abs() < 1e-5, "{back:?}");
            }
        }
    }

    #[test]
    fn organ_wag_sums_positive_genes() {
        let mut genes = [0.0_f32; N_GENES];
        genes[26] = 0.1;
        genes[27] = -0.2;
        genes[31] = 0.4;
        assert!((total_organ_wag(&genes) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn slab_slicing_converts_unity_rects_to_top_left() {
        let sheet = Sheet {
            atlas: 3,
            cell: [10, 20],
            grid: PartGrid {
                rows: 4,
                columns: 2,
            },
            ppu: 100.0 / PART_PPU_DIVISOR,
            pivot: Vec2::splat(0.5),
            bottom_origin_rows: false,
        };
        let sprite = sheet.sprite(1, 2);
        assert_eq!(sprite.rect, [10, 40, 10, 20]);
        assert_eq!(sprite.atlas, 3);
        assert!((sprite.ppu - 100.0 / 9.0).abs() < 1e-6);
    }

    #[test]
    fn arm_sheet_pivot_matches_top_left_renderer_convention() {
        // Live `normal_arms_10` pivot is (9,0) px; the renderer uses the same
        // top-left normalised pivot, i.e. (1,0), not the old `1 - y` (1,1).
        assert_eq!(
            legacy_pivot(BibitePart::Arms, None, [9, 3]),
            Vec2::new(1.0, 0.0)
        );
        let manifest = serde_json::json!({
            "sheets": {
                "normal_arms": [{"pivot": {"x": 1.0, "y": 0.0}}]
            }
        });
        assert_eq!(
            sheet_pivot_v1(&manifest, "normal_arms", BibitePart::Arms, None, [9, 3]),
            Vec2::new(1.0, 0.0)
        );
    }

    #[test]
    fn alpha_fallback_picks_nearest_nonempty_cell() {
        let mut rgba = vec![0_u8; 16 * 8 * 4];
        // Fill the right-hand cell only.
        for y in 0..8 {
            for x in 8..16 {
                let offset = (y * 16 + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&[200, 200, 200, 255]);
            }
        }
        let atlas = Atlas {
            name: "synthetic_eyes".into(),
            width: 16,
            height: 8,
            rgba,
        };
        let sheet = Sheet {
            atlas: 0,
            cell: [8, 8],
            grid: PartGrid {
                rows: 1,
                columns: 2,
            },
            ppu: 1.0,
            pivot: Vec2::new(0.5, 0.5),
            bottom_origin_rows: false,
        };
        // Formula cell 0 is empty, so the nearest non-empty cell 1 wins.
        let alpha = |sprite: SpriteRef| alpha_nonzero_in_rect(&atlas, sprite.rect);
        let sprite = alpha_fallback_sprite(&sheet, 0, 0, &[], alpha);
        assert_eq!(sprite.rect, [8, 0, 8, 8]);
        // A preferred non-empty cell is used before distance search.
        let sprite = alpha_fallback_sprite(&sheet, 0, 0, &[(1, 0)], alpha);
        assert_eq!(sprite.rect, [8, 0, 8, 8]);
        // Empty preferred candidates do not override a non-empty formula cell.
        let sprite = alpha_fallback_sprite(&sheet, 1, 0, &[(0, 0)], alpha);
        assert_eq!(sprite.rect, [8, 0, 8, 8]);
    }

    /// True when the pictures of the game (extracted on this machine by `tools/release/extract_art.py`) are under `root`: the manifest alone is
    /// metadata, which the public repository holds without any picture.
    fn game_pictures_present(root: &Path) -> bool {
        fn any_png(dir: &Path) -> bool {
            let Ok(entries) = std::fs::read_dir(dir) else { return false };
            entries.flatten().any(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    any_png(&path)
                } else {
                    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
                }
            })
        }
        any_png(root)
    }

    #[test]
    fn normal_captured_eye_and_arm_cells_are_nonempty() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("assets")
            .join("game");
        if !root.join("manifest.json").is_file() || !game_pictures_present(&root) {
            return; // the extracted game art is local-only (the repository holds the metadata, not the pictures)
        }
        let assets = GameAssets::load(&root).expect("load game assets");
        let eyes = assets
            .eye_sprite(0, 75.0, 180.0, false)
            .expect("normal eyes sprite");
        assert_eq!(eyes.rect, [16, 9, 8, 9]);
        assert!(assets.sprite_alpha_nonzero(&eyes) > 0);
        let arms = assets.arm_sprite(0, 1.0).expect("normal arms sprite");
        assert_eq!(arms.rect, [9, 0, 9, 3]);
        assert!(assets.sprite_alpha_nonzero(&arms) > 0);
    }

    #[test]
    fn the_remembered_cell_of_a_part_is_the_one_the_full_search_picks() {
        // The search looks at the pixels of the art and sorts the cells, so the answer is kept; kept or not it must be the same
        // for every sheet, every formula cell and both preferred cells in use (none, and the normal tier's live eye cell).
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("assets")
            .join("game");
        if !root.join("manifest.json").is_file() || !game_pictures_present(&root) {
            return; // the extracted game art is local-only (the repository holds the metadata, not the pictures)
        }
        let assets = GameAssets::load(&root).expect("load game assets");
        let mut checked = 0;
        for sheet in assets.sheets.values() {
            for preferred in [&[][..], &[(2, 1)][..]] {
                for row in 0..sheet.grid.rows {
                    for column in 0..sheet.grid.columns {
                        let direct = alpha_fallback_sprite(sheet, column, row, preferred, |sprite| {
                            assets.sprite_alpha_nonzero(&sprite)
                        });
                        // The first call works it out, the second one is remembered: both are the full search's answer.
                        for _ in 0..2 {
                            let kept = assets.sheet_sprite_with_alpha_fallback(sheet, column, row, preferred);
                            assert_eq!(kept, direct, "cell ({column}, {row}) preferred {preferred:?} of the sheet in atlas {}", sheet.atlas);
                        }
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 100, "only {checked} cells");
        assert!(assets.fallback_cache.read().expect("cache").len() >= checked / 2);
    }
}

#[cfg(test)]
mod crystal_tests {
    use super::*;

    #[test]
    fn the_colour_killers_crystal_has_the_colours_the_game_shows_at_a_hue() {
        // The art of `laser_crystal` (magenta, seven colours) as the game's texture has it, 15 x 15.
        let art_colours: [[u8; 3]; 7] = [[255, 105, 253], [251, 174, 255], [143, 18, 170], [191, 24, 189], [234, 51, 232], [77, 21, 120], [253, 213, 255]];
        let mut rgba = Vec::new();
        for index in 0..225 {
            let c = art_colours[index % 7];
            rgba.extend([c[0], c[1], c[2], 255]);
        }
        let art = Atlas { name: "laser_crystal".to_owned(), width: 15, height: 15, rgba };
        let green = crystal_pixels(&art, 0.4);
        let mut seen: Vec<[u8; 3]> = green.chunks_exact(4).map(|p| [p[0], p[1], p[2]]).collect();
        seen.sort_unstable();
        seen.dedup();
        // The seven colours of the game's capture of a killer of hue 0.4 (a bright green crystal): the same value and saturation as the art, the hue turned.
        let mut expected: Vec<[u8; 3]> = vec![[18, 170, 52], [51, 234, 127], [105, 255, 167], [24, 191, 93], [174, 255, 203], [24, 120, 21], [213, 255, 228]];
        expected.sort_unstable();
        for (got, want) in seen.iter().zip(&expected) {
            for k in 0..3 {
                assert!((i32::from(got[k]) - i32::from(want[k])).abs() <= 2, "{seen:?} against {expected:?}");
            }
        }
        assert_eq!(seen.len(), expected.len());
        assert!(green.chunks_exact(4).all(|p| p[3] == 255));
        // `Color.HSVToRGB`, the shader's `_color`.
        assert_eq!(hsv_full(0.0), [1.0, 0.0, 0.0]);
        assert!((hsv_full(0.4)[2] - 0.4).abs() < 1e-5 && hsv_full(0.4)[0] == 0.0);
        // The baked atlas picks a degree: hue 0.4 is cell 144.
        let sprites = ToolSprites {
            crystal_hues: Some(SpriteRef { atlas: 3, rect: [0, 0, 15, 15], ppu: 1.0, pivot: Vec2::splat(0.5) }),
            ..ToolSprites::default()
        };
        assert_eq!(sprites.crystal_hue_cell(0.4).map(|s| s.rect), Some([0, 144 * 15, 15, 15]));
        assert_eq!(sprites.crystal_hue_cell(1.0).map(|s| s.rect), Some([0, 0, 15, 15]));
    }
}
