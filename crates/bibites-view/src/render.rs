//! Rendering for the RUSTibites viewer.
//!
//! The module is deliberately tiny and dependency-light. It is split into three
//! parts:
//!
//! 1. [`Camera`] — the world→screen transform (world units are `f32`, the
//!    simulation's native unit; screen coordinates are logical pixels with `y`
//!    pointing down, matching every windowing/UI convention).
//! 2. A small **display list** ([`Scene`] / [`Primitive`]) extracted once per
//!    frame from the simulation state. Shapes are stored in *world* coordinates,
//!    so the two render backends below share exactly the same scene.
//! 3. Two backends:
//!    * [`draw_scene`] — draws the display list with `macroquad` for the
//!      interactive window;
//!    * [`render_scene`] — rasterises the same display list into an RGBA image
//!      in pure software (supersampled), which lets `cargo test -p
//!      bibites-view` and `--screenshot` verify rendering **without a display**.
//!
//! Text (HUD) is drawn by the app module through `macroquad`; the software
//! rasteriser intentionally only handles geometry (a font is the one thing that
//! would need an asset pipeline, and this alpha avoids one).
//!
//! Colour encoding is documented at each constant below. A bibite's skin colour
//! is `GenesToColor` of its `ColorR/G/B` pigment genes (not the genes themselves:
//! the basic bibite, 0.1 / 0.1 / 0.1, is light grey), converted to 8-bit sRGB. The
//! sprite art is one blue hue, so both backends recolour it with the same remap
//! (the tint's hue, the art's saturation times the tint's, the art's value): the
//! software rasteriser on the CPU, the window with [`GpuHueMaterial`].

use std::collections::HashMap;

use bibites_core::body::BibiteRecord;
use bibites_core::ids::BibiteId;
use bibites_core::math::Vec2;
use bibites_core::pellet::MaterialKind;
use bibites_core::settings::{MaterialKind as ZoneMaterial, SpawnDistribution};
use bibites_core::tools::{RadTower, Tool, hue_to_rgb};
use bibites_core::world::World;

use crate::assets::{Atlas, GameAssets};
use crate::skin::{SkinParams, SkinTint};
use crate::tail_mesh::TailState;

// ---------------------------------------------------------------------------
// Colour palette
// ---------------------------------------------------------------------------

/// RGBA colour in 8-bit sRGB (the software rasteriser works in this space; it is
/// "good enough" for a viewer and needs no gamma conversion).
pub type Rgba = [u8; 4];

/// Outside the simulation square: near-black blue.
pub const BACKGROUND: Rgba = [8, 11, 16, 255];
/// Inside the simulation square: a slightly lighter "floor".
pub const WORLD_FILL: Rgba = [13, 18, 25, 255];
/// Simulation-square border.
pub const WORLD_BORDER: Rgba = [58, 76, 96, 255];
/// Background grid lines (subtle).
pub const GRID_LINE: Rgba = [23, 31, 41, 255];
/// Grid lines through the world origin (slightly brighter).
pub const GRID_AXIS: Rgba = [37, 49, 63, 255];
/// Grid spacing target in logical screen pixels.
pub const GRID_TARGET_PIXELS: f32 = 64.0;
/// Zone outline for plant zones.
pub const ZONE_PLANT_OUTLINE: Rgba = [62, 126, 84, 235];
/// Zone outline for meat zones (and zones with no material).
pub const ZONE_MEAT_OUTLINE: Rgba = [140, 72, 62, 235];
/// Empty zone fill (low alpha so pellets/bibites stay readable).
pub const ZONE_FILL_EMPTY: Rgba = [20, 36, 27, 40];
/// Full zone fill (alpha is the maximum; the actual alpha scales with biomass).
pub const ZONE_FILL_FULL: Rgba = [40, 112, 64, 80];

/// Plant pellets: fresh green.
pub const PLANT: Rgba = [74, 190, 96, 235];
/// Meat pellets: blood red.
pub const MEAT: Rgba = [204, 78, 62, 235];
/// Egg shell: warm bone.
pub const EGG: Rgba = [232, 216, 158, 245];
/// Aborting eggs: dull grey.
pub const EGG_ABORTING: Rgba = [135, 135, 130, 235];
/// Corpses: desaturated meat red.
pub const CORPSE: Rgba = [122, 74, 68, 235];
/// Selected bibite highlight.
pub const SELECTION: Rgba = [250, 250, 252, 245];

/// Bodies are drawn at 1.8× the collider's true size. The game's procedural
/// sprite is ~9 world units long for `d1Size = 1` while the capsule collider is
/// 5 × 7 units, so scaling up is *closer* to the artwork than drawing the raw
/// collider and keeps bibites legible at overview zoom.
pub const BODY_VISUAL_SCALE: f32 = 1.8;
/// Minimum on-screen radius (logical px) for a bibite body. Keeps the population
/// visible when the whole world is fitted on screen.
pub const MIN_BODY_PIXELS: f32 = 1.1;
/// Minimum on-screen radius (logical px) for a pellet.
pub const MIN_PELLET_PIXELS: f32 = 0.85;
/// A bibite whose body is under this many pixels long is drawn as one dot (see [`Dot`]): its parts are a pixel across or less.
pub const BIBITE_DOT_PIXELS: f32 = 2.5;
/// A bibite whose body is under this many pixels long is drawn as its body sprite alone: its mouth, eyes and arms are a pixel or two each.
pub const BIBITE_BODY_ONLY_PIXELS: f32 = 6.0;
/// A pellet whose sprite would be under this many pixels across is a dot (a sprite is drawn a pixel across at the least, so it looks the same).
pub const PELLET_DOT_PIXELS: f32 = 2.0;
/// A particle (a pheromone diamond's diagonal, a blood pixel's side) under this many pixels across is a dot.
pub const PARTICLE_DOT_PIXELS: f32 = 1.5;
/// Egg visual radius in world units per unit of the child's `SizeRatio` gene.
/// The egg sprite is not part of the simulation state, so this is a deliberate
/// visual approximation.
pub const EGG_RADIUS_PER_SIZE_GENE: f32 = 3.0;
/// Pheromone spot radius in world units. The simulation stores a sensed
/// strength, not a geometry radius, so this is a visual approximation.
pub const PHEROMONE_RADIUS: f32 = 6.0;

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

/// Game `CameraManager.CameraZoom` wheel ratio: `delta / 10 * scrollSpeed`.
#[must_use]
pub fn wheel_zoom_ratio(wheel: f32, scroll_speed: f32) -> f32 {
    wheel / 10.0 * scroll_speed
}

/// Orthographic size (half visible height, world units) for a zoom.
#[must_use]
pub fn zoom_to_ortho(zoom: f32, screen_height: f32) -> f32 {
    screen_height / (2.0 * zoom.max(1e-6))
}

/// Zoom (pixels per world unit) for an orthographic size.
#[must_use]
pub fn ortho_to_zoom(ortho: f32, screen_height: f32) -> f32 {
    screen_height / (2.0 * ortho.max(1e-6))
}

/// Game clamp: `[5, max(250, 2*simulationSize)]`.
#[must_use]
pub fn clamp_ortho_size(ortho: f32, simulation_size: f32) -> f32 {
    ortho.clamp(5.0, 250.0_f32.max(2.0 * simulation_size))
}

/// Frame-rate independent exponential smoothing in log-zoom space.
#[must_use]
pub fn smooth_zoom(current: f32, target: f32, dt: f32, time_constant: f32) -> f32 {
    if current <= 0.0 || target <= 0.0 {
        return target.max(current);
    }
    let blend = 1.0 - (-dt.max(0.0) / time_constant.max(1e-3)).exp();
    current * (target / current).powf(blend)
}

/// Orthographic top-down camera.
///
/// * `center` — world position shown at the middle of the viewport.
/// * `zoom` — logical screen pixels per world unit.
/// * `screen` — viewport size in logical pixels.
///
/// World `y` points up (simulation convention), screen `y` points down, so the
/// transform negates `y`. The public methods are the *only* places that convert;
/// both render backends use them, which keeps window and screenshot identical.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World position at the viewport centre.
    pub center: Vec2,
    /// Logical screen pixels per world unit.
    pub zoom: f32,
    /// Viewport size in logical pixels (`x` = width, `y` = height).
    pub screen: Vec2,
}

impl Camera {
    /// Smallest allowed zoom (zoomed all the way out).
    pub const MIN_ZOOM: f32 = 1e-5;
    /// Largest allowed zoom (a few world units across the viewport).
    pub const MAX_ZOOM: f32 = 512.0;

    /// Creates a camera looking at `center` with `zoom` pixels per world unit.
    #[must_use]
    pub fn new(center: Vec2, zoom: f32, screen: Vec2) -> Self {
        Self {
            center,
            zoom: zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM),
            screen,
        }
    }

    /// Fits the world bounds into the viewport, leaving `margin` screen pixels on
    /// each side.
    #[must_use]
    pub fn fit(min: Vec2, max: Vec2, screen: Vec2, margin: f32) -> Self {
        let usable = Vec2::new(
            (screen.x - 2.0 * margin).max(1.0),
            (screen.y - 2.0 * margin).max(1.0),
        );
        let extent = max - min;
        let zoom_x = usable.x / extent.x.max(1e-3);
        let zoom_y = usable.y / extent.y.max(1e-3);
        Self::new((min + max) * 0.5, zoom_x.min(zoom_y), screen)
    }

    /// Converts a world position to logical screen pixels.
    #[must_use]
    pub fn world_to_screen(&self, world: Vec2) -> Vec2 {
        Vec2::new(
            (world.x - self.center.x) * self.zoom + self.screen.x * 0.5,
            (self.center.y - world.y) * self.zoom + self.screen.y * 0.5,
        )
    }

    /// Converts logical screen pixels back to a world position.
    #[must_use]
    pub fn screen_to_world(&self, screen: Vec2) -> Vec2 {
        Vec2::new(
            self.center.x + (screen.x - self.screen.x * 0.5) / self.zoom,
            self.center.y - (screen.y - self.screen.y * 0.5) / self.zoom,
        )
    }

    /// World-space rectangle currently visible (plus `margin` world units).
    #[must_use]
    pub fn visible_world(&self, margin: f32) -> (Vec2, Vec2) {
        let a = self.screen_to_world(Vec2::ZERO);
        let b = self.screen_to_world(self.screen);
        (
            Vec2::new(a.x.min(b.x) - margin, a.y.min(b.y) - margin),
            Vec2::new(a.x.max(b.x) + margin, a.y.max(b.y) + margin),
        )
    }

    /// Zooms by `factor` while keeping the world point under `screen_point`
    /// stationary (wheel-zoom behaviour).
    pub fn zoom_at(&mut self, screen_point: Vec2, factor: f32) {
        let anchor = self.screen_to_world(screen_point);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let moved = self.screen_to_world(screen_point);
        self.center += anchor - moved;
    }

    /// Pans by a screen-pixel delta (right-drag moves the world with the cursor).
    pub fn pan_screen(&mut self, delta: Vec2) {
        self.center.x -= delta.x / self.zoom;
        self.center.y += delta.y / self.zoom;
    }

    /// Changes the viewport size, re-fitting if the camera was never shown yet.
    pub fn set_screen(&mut self, screen: Vec2) {
        self.screen = Vec2::new(screen.x.max(1.0), screen.y.max(1.0));
    }
}

// ---------------------------------------------------------------------------
// Display list
// ---------------------------------------------------------------------------

/// One sprite draw's procedural tail warp (Body layer only).
///
/// The game's `ProceduralSpriteController` bends the body sprite's lower rows
/// with `_MiddleSegmentAngle`/`_TailAngle`; the renderer reproduces that as a
/// per-row affine warp of the sprite UVs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TailWarp {
    /// `_MiddleSegmentAngle` in radians (top of the tail region).
    pub angle0: f32,
    /// `_TailAngle` in radians (tip of the tail region).
    pub angle1: f32,
    /// Row (0..1 in sprite UV) where the deformation starts.
    pub bend_row: f32,
    /// Tail segment count/scale (`max(round(d1*12.5)*2+1, 13)`).
    pub size_scale: u32,
}

/// A thing too small to be drawn in parts (a bibite a couple of pixels long, a pellet, a particle): one square of one colour, at least a pixel across.
///
/// The display list keeps them in [`Scene::dots`], 20 bytes each, and refers to a run of them with one [`Primitive::Dots`]: a sprite primitive is a hundred
/// and some bytes, and a world zoomed out far enough to show a hundred thousand pellets wrote (and sorted, and cloned for the simulation thread) that many of
/// them every frame, then asked the window for a quad each, to draw what is a pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    /// World centre.
    pub center: Vec2,
    /// Half of the side, in world units (the dot is never drawn under a pixel across).
    pub half: f32,
    /// Fill colour (alpha blended).
    pub color: Rgba,
}

/// One filled/stroked shape in world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primitive {
    /// A run of [`Scene::dots`]: `count` of them from `first`, drawn in order. (A run may be empty: [`Scene::open_dots`] reserves its place in the draw
    /// order before the dots are known.)
    Dots {
        /// Index in [`Scene::dots`] of the first dot of the run.
        first: u32,
        /// How many dots the run holds.
        count: u32,
    },
    /// Axis-aligned filled rectangle.
    Rect {
        /// Lower-left corner.
        min: Vec2,
        /// Upper-right corner.
        max: Vec2,
        /// Fill colour (alpha blended).
        color: Rgba,
    },
    /// Axis-aligned rectangle outline.
    RectOutline {
        /// Lower-left corner.
        min: Vec2,
        /// Upper-right corner.
        max: Vec2,
        /// Stroke width in world units.
        thickness: f32,
        /// Stroke colour.
        color: Rgba,
    },
    /// Filled disc.
    Circle {
        /// World centre.
        center: Vec2,
        /// World radius.
        radius: f32,
        /// Fill colour.
        color: Rgba,
    },
    /// Disc outline.
    CircleOutline {
        /// World centre.
        center: Vec2,
        /// World radius.
        radius: f32,
        /// Stroke width in world units.
        thickness: f32,
        /// Stroke colour.
        color: Rgba,
    },
    /// Filled annulus (ring zone).
    Ring {
        /// World centre.
        center: Vec2,
        /// Outer radius.
        outer: f32,
        /// Inner radius.
        inner: f32,
        /// Fill colour.
        color: Rgba,
    },
    /// Filled circular sector (the selected bibite's view cone).
    Sector {
        /// World centre.
        center: Vec2,
        /// World radius.
        radius: f32,
        /// Unit vector the sector is centred on (world space).
        facing: Vec2,
        /// Half of the opening angle in radians (`>= PI` fills the whole disc).
        half_angle: f32,
        /// Fill colour (alpha blended).
        color: Rgba,
    },
    /// A flat square turned 45 degrees (a pheromone particle: a quad mesh without a texture), alpha blended.
    Diamond {
        /// World centre.
        center: Vec2,
        /// Half of the diagonal, in world units.
        half_diagonal: f32,
        /// Fill colour.
        color: Rgba,
    },
    /// The experimental background's moving cells (`Shader Graphs/background` with `_ShowTexture` on, its own grid off): the game's pixel program over a
    /// world rectangle, from the background colour toward the texture colour.
    BackgroundCells {
        /// World corners.
        min: Vec2,
        /// See `min`.
        max: Vec2,
        /// `_Color`.
        color: Rgba,
        /// `_CellColor`.
        cell: Rgba,
    },
    /// The background grid (the end of `Shader Graphs/background`'s pixel program): lines every `fine_step` and `coarse_step` world units, as wide as
    /// their widths (a solid core and a soft edge a tenth of the half width), over the whole view. The fine lines' mask is the larger of its two
    /// directions less the coarse one's (a fine line under a coarse one is not there), so lines do not add up where they cross.
    GridLines {
        /// Distance between the fine lines.
        fine_step: f32,
        /// Width of a fine line (world units).
        fine_width: f32,
        /// Colour of the fine lines (the alpha is the line's opacity).
        fine_color: Rgba,
        /// Distance between the coarse lines.
        coarse_step: f32,
        /// Width of a coarse line (world units).
        coarse_width: f32,
        /// Colour of the coarse lines.
        coarse_color: Rgba,
    },
    /// A zone's fill (`Shader Graphs/PelletSpawnPoint`, read from its compiled pixel program): the fertility colour `rgb` at the alpha
    /// `opacity * saturate(100 * (1 - d)) * (0.5 + coefficient * (0.5 - d^2))`, where `d` is the distance from the centre over the radius (for a
    /// ring `inside` is its inner radius over the outer, and `d` the distance from the ring's middle over its half width): a centric zone fades
    /// from `opacity` at its centre to nothing at its rim, an exterior one the other way, a flat one is `opacity / 2` all over.
    ZoneDisc {
        /// World centre.
        center: Vec2,
        /// World radius.
        radius: f32,
        /// The fertility colour (the shader's `_fertilityColor`).
        rgb: [u8; 3],
        /// `_Opacity`.
        opacity: f32,
        /// `_PelletSpawnCoefficient` (+1 centric and ring, -1 exterior, 0 flat).
        coefficient: f32,
        /// `_insideRadius` of a ring (the shader's `_Ring`), `None` for a disc.
        inside: Option<f32>,
    },
    /// A sprite that turns by itself (`Utility.UnscaledRotation`: the colour killer's crystal turns `rate` times a second on the real clock): the rotation is
    /// worked out when it is drawn ([`effect_time`]), so it moves every frame whatever the rate the scene is published at.
    Spinner {
        /// The art.
        sprite: crate::assets::SpriteRef,
        /// World position of the pivot.
        anchor: Vec2,
        /// World size.
        size: Vec2,
        /// Pivot within the sprite.
        pivot: Vec2,
        /// Tint.
        tint: Rgba,
        /// Turns per second (counter-clockwise when positive).
        turns: f32,
    },
    /// A radiation tower's area (`RadioArea`, `Shader Graphs/radioZone`): animated value noise on a grid of 5-unit cells over the square of side `2 * radius`
    /// round `center`, drawn by the game's own pixel program ([`radio_cells`]; its time is [`effect_time`]).
    RadioField {
        /// World centre.
        center: Vec2,
        /// World radius (`_size` is twice this).
        radius: f32,
    },
    /// The microscope shade outside the world (`Shader Graphs/microscopeShade`): black with an alpha of `saturate((|p| - start) / fade)` at the world point `p`.
    MicroscopeShade {
        /// `_ShadeStart` (1.5 x the simulation size).
        start: f32,
        /// `_ShadeFadeDistance` (1000).
        fade: f32,
    },
    /// Filled capsule (oriented bibite body / pellet blob).
    Capsule {
        /// First segment endpoint (world).
        a: Vec2,
        /// Second segment endpoint (world).
        b: Vec2,
        /// Cap radius in world units.
        radius: f32,
        /// Fill colour.
        color: Rgba,
    },
    /// Optional game sprite from an extracted atlas.
    Sprite {
        /// Atlas sprite reference.
        sprite: crate::assets::SpriteRef,
        /// World position where the sprite pivot lands.
        anchor: Vec2,
        /// World size of the sprite at its current scale.
        size: Vec2,
        /// Normalised pivot, (0,0) = top-left.
        pivot: Vec2,
        /// World rotation in radians.
        rotation: f32,
        /// RGBA tint multiplied over the sprite.
        tint: Rgba,
        /// Mirror horizontally.
        flip_x: bool,
        /// Mirror vertically.
        flip_y: bool,
        /// Recolour the sprite by replacing its hue (game skin/eye shader).
        hue_shift: bool,
        /// The skin shader's `[_HurtAmount, _IsKiller]`.
        effects: crate::skin::SkinEffects,
        /// Optional procedural tail warp for the Body sprite.
        warp: Option<TailWarp>,
    },
    /// Thick line segment (grid, mouth detail).
    Line {
        /// Start point (world).
        a: Vec2,
        /// End point (world).
        b: Vec2,
        /// Stroke width in world units.
        thickness: f32,
        /// Stroke colour.
        color: Rgba,
    },
}

/// Toggles for the optional scene layers (keyboard overlay switches).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneOptions {
    /// Draw zone fills and outlines.
    pub zones: bool,
    /// Draw plant/meat pellets.
    pub pellets: bool,
    /// Draw pheromone spots.
    pub pheromones: bool,
    /// The player's "Pheromones Strength" (`UserSettings.pheromonesStrength`): how many particles a spot emits.
    pub pheromone_strength: f32,
    /// Draw the world-space background grid.
    pub grid: bool,
    /// Draw the selected bibite's field of view (`UserSettings.ShowBibiteFOW`).
    pub fov: bool,
    /// Label what the selected bibite's field of view sees with its weight (`UserSettings.ShowBibiteTargets`; only while the field of view is shown):
    /// the scene holds the labels' pointers, the HUD the labels.
    pub targets: bool,
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self {
            zones: true,
            pellets: true,
            // Pheromone spots are chunky and only meaningful when watched
            // closely, so they start hidden; `O` toggles them on.
            pheromones: false,
            pheromone_strength: 0.1,
            grid: true,
            fov: true,
            targets: false,
        }
    }
}

/// Which tower the placement ghost shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhostKind {
    /// A radiation tower being placed.
    Rad,
    /// A pheromone tower being placed.
    Phero,
    /// A colour killer being placed.
    Killer,
}

/// What the player is doing with the placed towers and colour killers, for the display list.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ToolView {
    /// A tower or colour killer being placed: it follows the cursor at half opacity
    /// (`EffectTower.PlacementMode`). The world position of the cursor.
    pub ghost: Option<(GhostKind, Vec2)>,
    /// Save id of the selected tool: a selected colour killer shows its radius
    /// (`ColorKillerPanel.zoneDisplay`).
    pub selected: Option<u64>,
}

/// A frame's worth of primitives, in painter's order (first = back).
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// Clear colour (outside the simulation square).
    pub background: Rgba,
    /// World-space primitives, back to front.
    pub primitives: Vec<Primitive>,
    /// The dots the [`Primitive::Dots`] runs of `primitives` refer to.
    pub dots: Vec<Dot>,
    /// The tool placement and selection state the next [`build_scene`] draws (it is not reset by
    /// [`Scene::clear`]: the caller sets it before each rebuild).
    pub tool_view: ToolView,
    /// Reused by [`group_sprites_by_atlas`].
    pub(crate) sort_scratch: Vec<Primitive>,
    /// The skin shader's globals: how the `hue_shift` sprites are recoloured (set by [`build_scene`] from the game's
    /// `parameters.csv`).
    pub skin: SkinParams,
}

impl Scene {
    /// A copy of the display list without the sorting scratch space: what the simulation thread publishes.
    #[must_use]
    pub(crate) fn published(&self) -> Self {
        Self {
            background: self.background,
            primitives: self.primitives.clone(),
            dots: self.dots.clone(),
            tool_view: self.tool_view,
            sort_scratch: Vec::new(),
            skin: self.skin,
        }
    }

    /// Creates an empty scene with the viewer background colour.
    #[must_use]
    pub fn new() -> Self {
        Self {
            background: BACKGROUND,
            primitives: Vec::new(),
            dots: Vec::new(),
            tool_view: ToolView::default(),
            sort_scratch: Vec::new(),
            skin: SkinParams::default(),
        }
    }

    /// Empties the display list but keeps its capacity (the viewer reuses one
    /// scene per frame to avoid per-frame allocation churn).
    pub fn clear(&mut self) {
        self.primitives.clear();
        self.dots.clear();
        self.background = BACKGROUND;
    }

    /// Appends a primitive.
    pub fn push(&mut self, primitive: Primitive) {
        self.primitives.push(primitive);
    }

    /// Reserves a place in the draw order for a run of dots: the dots pushed to [`Scene::dots`] from now on (between other primitives too) are the run's
    /// once [`Scene::close_dots`] has been given the place this returns.
    pub fn open_dots(&mut self) -> usize {
        let first = u32::try_from(self.dots.len()).unwrap_or(u32::MAX);
        self.primitives.push(Primitive::Dots { first, count: 0 });
        self.primitives.len() - 1
    }

    /// Ends the run [`Scene::open_dots`] opened at `place`: it holds every dot pushed since.
    pub fn close_dots(&mut self, place: usize) {
        let total = u32::try_from(self.dots.len()).unwrap_or(u32::MAX);
        if let Some(Primitive::Dots { first, count }) = self.primitives.get_mut(place) {
            *count = total.saturating_sub(*first);
        }
    }

    /// The dots of a [`Primitive::Dots`] run (what there is of it when it reaches past the end of the list, none when it starts there).
    #[must_use]
    pub fn dots_of(&self, first: u32, count: u32) -> &[Dot] {
        let start = first as usize;
        let end = start.saturating_add(count as usize).min(self.dots.len());
        self.dots.get(start..end).unwrap_or(&[])
    }

    /// Number of primitives in the list.
    #[must_use]
    pub fn len(&self) -> usize {
        self.primitives.len()
    }
}

// ---------------------------------------------------------------------------
// Scene extraction
// ---------------------------------------------------------------------------

/// Rebuilds `scene` from the current world state.
///
/// `selected` receives a highlight ring drawn last (on top), and `options`
/// switches the optional layers (grid/zones/pellets/pheromones). Everything is
/// culled against the camera's visible rectangle, so large worlds stay cheap.
pub fn build_scene(
    scene: &mut Scene,
    world: &World,
    camera: &Camera,
    selected: Option<BibiteId>,
    options: &SceneOptions,
    assets: Option<&GameAssets>,
    tails: Option<&HashMap<BibiteId, TailState>>,
) {
    scene.clear();
    scene.skin = assets.map_or_else(SkinParams::default, |assets| {
        SkinParams::from(&assets.parameters)
    });
    scene.skin.time = world.simulated_time() as f32;
    let (view_min, view_max) = camera.visible_world(0.0);
    let bounds_min = world.bounds.min();
    let bounds_max = world.bounds.max();

    // 1. Background. In game-asset mode this reproduces the game's background
    // shader: the camera clear colour plus the user's fine/coarse grid (the
    // grid is part of the background and therefore *behind* the translucent
    // zones). The procedural fallback keeps its own palette.
    if let Some(assets) = assets {
        scene.background = assets.background.color;
        scene.push(Primitive::Rect {
            min: bounds_min,
            max: bounds_max,
            color: assets.background.color,
        });
        if let Some(cell) = assets.background.texture {
            scene.push(Primitive::BackgroundCells { min: bounds_min, max: bounds_max, color: assets.background.color, cell });
        }
        if world.settings.scenario.shade_outside_of_bounds {
            // `BackgroundManager.shadeStart`: 1.5 x the simulation size; the fade is 1000.
            scene.push(Primitive::MicroscopeShade { start: 1.5 * world.settings.independents.simulation_size, fade: 1000.0 });
        }
        if options.grid {
            push_game_grid(scene, camera, world, view_min, view_max, &assets.background);
        }
    } else {
        scene.push(Primitive::Rect {
            min: bounds_min,
            max: bounds_max,
            color: WORLD_FILL,
        });
        scene.push(Primitive::RectOutline {
            min: bounds_min,
            max: bounds_max,
            thickness: 2.0 / camera.zoom,
            color: WORLD_BORDER,
        });
        if options.grid {
            push_grid(scene, camera, world, view_min, view_max);
        }
    }

    // 3. Zones: filled by plant density, outlined by pellet material.
    if options.zones {
        for zone in &world.zones.zones {
            let density = if zone.max_biomass > 0.0 {
                (zone.pellet_biomass / zone.max_biomass).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (fill, outline, thickness) = if let Some(assets) = assets {
                // Game zone shader (`docs/design/ZONE_COLOUR.md`):
                // opacity = (f(fertility) + f(biomassDensity)) / 2 with
                // f(v) = 1 - 0.9*(invLog(v,.001,100) - 1)^2, palette factor
                // 0.5 for Flat/FlatRing/Rect distributions, fill = base*k*
                // opacity over black. It does NOT depend on current biomass.
                let opacity = (zone_density_factor(zone.config.fertility)
                    + zone_density_factor(zone.config.biomass_density))
                    * 0.5;
                // `Zone.UpdateMaterial`: `_PelletSpawnCoefficient` is 0 for Flat / FlatRing / Rect, -1 for ExteriorGradual, +1 otherwise.
                let coefficient = match zone.config.distribution {
                    SpawnDistribution::Flat | SpawnDistribution::FlatRing | SpawnDistribution::Rect => 0.0,
                    SpawnDistribution::ExteriorGradual => -1.0,
                    _ => 1.0,
                };
                let base = assets.background.zone_base;
                if zone.is_rect {
                    // A rect has no circle mask and no gradient: the alpha is `opacity * (0.5 + 0.5 * coefficient)` all over.
                    let alpha = opacity * (0.5 + coefficient * 0.5);
                    let min = zone.pos - zone.half_extents;
                    let max = zone.pos + zone.half_extents;
                    if rects_intersect(min, max, view_min, view_max) {
                        scene.push(Primitive::Rect {
                            min,
                            max,
                            color: [base[0], base[1], base[2], (alpha * 255.0).round().clamp(0.0, 255.0) as u8],
                        });
                    }
                } else if circle_visible(zone.pos, zone.radius, view_min, view_max) {
                    let inside = (zone.is_ring && zone.inside_radius_abs > 0.0)
                        .then(|| (zone.inside_radius_abs / zone.radius).clamp(0.0, 1.0));
                    scene.push(Primitive::ZoneDisc {
                        center: zone.pos,
                        radius: zone.radius,
                        rgb: base,
                        opacity,
                        coefficient,
                        inside,
                    });
                }
                continue;
            } else {
                (
                    mix(ZONE_FILL_EMPTY, ZONE_FILL_FULL, density),
                    if zone.config.material == Some(ZoneMaterial::Meat) {
                        ZONE_MEAT_OUTLINE
                    } else {
                        ZONE_PLANT_OUTLINE
                    },
                    1.5 / camera.zoom,
                )
            };

            if zone.is_rect {
                let min = zone.pos - zone.half_extents;
                let max = zone.pos + zone.half_extents;
                if !rects_intersect(min, max, view_min, view_max) {
                    continue;
                }
                scene.push(Primitive::Rect {
                    min,
                    max,
                    color: fill,
                });
                scene.push(Primitive::RectOutline {
                    min,
                    max,
                    thickness,
                    color: outline,
                });
            } else if zone.is_ring && zone.inside_radius_abs > 0.0 {
                if !circle_visible(zone.pos, zone.radius, view_min, view_max) {
                    continue;
                }
                scene.push(Primitive::Ring {
                    center: zone.pos,
                    outer: zone.radius,
                    inner: zone.inside_radius_abs.min(zone.radius),
                    color: fill,
                });
                scene.push(Primitive::CircleOutline {
                    center: zone.pos,
                    radius: zone.radius,
                    thickness,
                    color: outline,
                });
                scene.push(Primitive::CircleOutline {
                    center: zone.pos,
                    radius: zone.inside_radius_abs,
                    thickness,
                    color: outline,
                });
            } else {
                if !circle_visible(zone.pos, zone.radius, view_min, view_max) {
                    continue;
                }
                scene.push(Primitive::Circle {
                    center: zone.pos,
                    radius: zone.radius,
                    color: fill,
                });
                scene.push(Primitive::CircleOutline {
                    center: zone.pos,
                    radius: zone.radius,
                    thickness,
                    color: outline,
                });
            }
        }
    }

    // 3b. The selected bibite's field of view (`FieldOfViewDisplay`, shown when `ShowBibiteFOW` is on - the default - and the
    // bibite is alive). Its renderer is on sorting layer -2, order 100 (the prefab dump): above the zones (-2, order 1) and below
    // everything on the layers above it - towers and eggs (-1), pellets and bibites (0) - so the cone never tints a sprite.
    // The game draws a shader cone; it is approximated by its translucent fill and its border (two rays and an arc).
    if options.fov {
        if let Some(record) = selected.and_then(|id| world.bibites.get(id)) {
            if !record.body.dead {
                push_view_cone(scene, camera, record);
            }
        }
    }

    // 4. Pheromones: the particles the spots emit (`PheromoneSpot`, `ParticlesMaster`), under pellets and bodies.
    crate::phero_particles::advance(world, options.pheromones, options.pheromone_strength);
    if options.pheromones {
        let dots_at = scene.open_dots();
        for particle in crate::phero_particles::drawn(view_min, view_max) {
            // A diamond under a pixel and a half across is a pixel (its area is that of a square with 0.707 of the half diagonal).
            if particle.half_diagonal * 2.0 * camera.zoom < PARTICLE_DOT_PIXELS {
                scene.dots.push(Dot { center: particle.pos, half: particle.half_diagonal * std::f32::consts::FRAC_1_SQRT_2, color: particle.color });
                continue;
            }
            scene.push(Primitive::Diamond {
                center: particle.pos,
                half_diagonal: particle.half_diagonal,
                color: particle.color,
            });
        }
        scene.close_dots(dots_at);
    }

    // 4b. Towers and colour killers: sorting layer `Recessed`, above the background and the zones
    // and below everything on `Default` (pellets, eggs, bibites).
    push_tools(scene, world, assets, view_min, view_max, camera);

    // 5. Pellets (plants green, meat red).
    if options.pellets {
        let first_pellet = scene.primitives.len();
        // Pellets too small to be anything but a pixel are dots (a run at the head of the layer: under the sprites, which are grouped by atlas below).
        let dots_at = scene.open_dots();
        let dot_colours = assets.map(|assets| (crate::sprites::pellet_dot_colour(assets, MaterialKind::Plant), crate::sprites::pellet_dot_colour(assets, MaterialKind::Meat)));
        for id in world.pellets.ids() {
            let Some(pellet) = world.pellets.get(id) else {
                continue;
            };
            if !circle_visible(pellet.pos, pellet.radius, view_min, view_max) {
                continue;
            }
            if let Some((plant, meat)) = dot_colours
                && pellet.radius * 2.0 * camera.zoom < PELLET_DOT_PIXELS
                && let Some(color) = match pellet.material {
                    MaterialKind::Plant => plant,
                    MaterialKind::Meat => meat,
                    _ => None,
                }
            {
                scene.dots.push(Dot { center: pellet.pos, half: pellet.radius, color });
                continue;
            }
            if let Some(assets) = assets {
                let rotation = world.pellets.rotation(id).unwrap_or(0.0);
                if let Some(sprite) = crate::sprites::pellet_sprite(assets, &pellet, rotation) {
                    scene.push(sprite_primitive(sprite));
                    continue;
                }
            }
            let color = match pellet.material {
                MaterialKind::Plant => PLANT,
                MaterialKind::Meat => MEAT,
                MaterialKind::Fat | MaterialKind::Armor => continue,
            };
            scene.push(Primitive::Circle {
                center: pellet.pos,
                radius: pellet.radius,
                color,
            });
        }
        scene.close_dots(dots_at);
        group_sprites_by_atlas(&mut scene.primitives, first_pellet, &mut scene.sort_scratch);
    }

    // 6. Eggs (shell + hatch-progress core).
    for id in world.eggs.ids() {
        let Some(egg) = world.eggs.get(id) else {
            continue;
        };
        let radius = EGG_RADIUS_PER_SIZE_GENE * egg.size_gene.max(0.25);
        if !circle_visible(egg.pos, radius, view_min, view_max) {
            continue;
        }
        if let Some(assets) = assets {
            if let Some(sprite) = crate::sprites::egg_sprite(assets, egg) {
                scene.push(sprite_primitive(sprite));
                continue;
            }
        }
        let shell = if egg.hatch.aborting {
            EGG_ABORTING
        } else {
            EGG
        };
        scene.push(Primitive::Circle {
            center: egg.pos,
            radius,
            color: shell,
        });
        scene.push(Primitive::CircleOutline {
            center: egg.pos,
            radius,
            thickness: (radius * 0.18).max(1.0 / camera.zoom),
            color: [90, 84, 52, 220],
        });
        let hatch = if egg.hatch.hatch_time > 0.0 {
            (egg.hatch.hatch_progress / egg.hatch.hatch_time).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if hatch > 0.02 {
            scene.push(Primitive::Circle {
                center: egg.pos,
                radius: radius * 0.62 * hatch,
                color: [250, 240, 200, 230],
            });
        }
    }

    // 7. Bibites and corpses. Bodies trail behind the origin in the game's
    //    collider, but with no sprite art the body is centred on the position so
    //    rotation stays readable. Detail primitives are gated on zoom so a 2k
    //    population stays cheap when zoomed out.
    // (The bibites that are only a few pixels long are dots, one run at the head of the layer; the selected one is always drawn in full.)
    let dots_at = scene.open_dots();
    for (id, record) in world.bibites.iter() {
        push_bibite(
            scene,
            camera,
            id,
            record,
            view_min,
            view_max,
            assets,
            tails.and_then(|tails| tails.get(&id)),
            selected == Some(id),
        );
    }
    scene.close_dots(dots_at);

    // 7b. The blood of the bodies that burst into meat (`ParticlesMaster.BloodSplatterAtPosition`), sorting order 10: over the bodies.
    crate::blood_particles::advance(world);
    let dots_at = scene.open_dots();
    for pixel in crate::blood_particles::drawn(view_min, view_max) {
        if (pixel.max.x - pixel.min.x) * camera.zoom < PARTICLE_DOT_PIXELS {
            scene.dots.push(Dot { center: (pixel.min + pixel.max) * 0.5, half: (pixel.max.x - pixel.min.x) * 0.5, color: pixel.color });
            continue;
        }
        scene.push(Primitive::Rect {
            min: pixel.min,
            max: pixel.max,
            color: pixel.color,
        });
    }
    scene.close_dots(dots_at);

    // 8. A selected corpse gets a ring on top (the game has no selection ring for a living bibite; its field of view is drawn
    // under the sprites, see 3b).
    if let Some(id) = selected {
        if let Some(record) = world.bibites.get(id) {
            if record.body.dead {
                let radius = body_radius(record).max(6.0 / camera.zoom);
                scene.push(Primitive::CircleOutline {
                    center: record.pos,
                    radius: radius * 1.8,
                    thickness: 1.5 / camera.zoom,
                    color: SELECTION,
                });
            }
        }
    }

    // 9. "Show Targets": the pointers of the labels of what the selected bibite sees (`WeightDisplay.pointer`), over everything in the world; the labels
    // themselves are drawn by the HUD (`target_weights`).
    if options.fov && options.targets {
        if let Some(targets) = selected.and_then(|id| world.fov_targets(id)) {
            for label in crate::target_weights::labels(&targets) {
                scene.push(label.pointer());
            }
        }
    }
}

/// Size of the round base of a radiation tower or colour killer (`laser_circle`, 35 px at 1 px
/// per unit) and its trigger collider's radius (the prefab's `CircleCollider2D`).
pub const TOWER_BASE_SIZE: f32 = 35.0;
/// Size of the pheromone tower sprite (`pheroTower`, 45 px).
pub const PHERO_TOWER_SIZE: f32 = 45.0;
/// Size of the colour killer's crystal (`laser_crystal`, 15 px).
pub const CRYSTAL_SIZE: f32 = 15.0;
/// `RadioArea.color`: (176, 255, 0, 128).
const RADIO_AREA: Rgba = [176, 255, 0, 128];
/// Colour of the `radioactive` icon on the radiation tower (the prefab's (0.50, 0.58, 0.00)).
const RAD_ICON_TINT: Rgba = [127, 148, 0, 255];

/// The radiation and pheromone towers' base opacity until they are placed in the game (0.7725 of the prefab).
const LOADED_TOWER_ALPHA: u8 = 197;

/// The tower kinds `push_tool_body` draws.
#[derive(Clone, Copy)]
enum ToolShape {
    /// The radius of its `RadioArea`.
    Rad { radius: f32 },
    Phero { heading: f32 },
    Killer { hue: f32 },
}

/// Draws the placed towers and colour killers, the placement ghost and the radius of the selected
/// colour killer (`RadioArea`, `ColorKillerPanel.zoneDisplay`, the three tower prefabs).
fn push_tools(
    scene: &mut Scene,
    world: &World,
    assets: Option<&GameAssets>,
    view_min: Vec2,
    view_max: Vec2,
    camera: &Camera,
) {
    let view = scene.tool_view;
    let sim_size = world.bounds.sim_size();
    // The selected colour killer shows the area it scans, in its colour at 30 % opacity.
    if let Some(selected) = view.selected {
        for tool in world.tools.tools() {
            let Tool::Killer(killer) = tool else {
                continue;
            };
            if killer.base.save_id != selected
                || !circle_visible(killer.base.pos, killer.radius, view_min, view_max)
            {
                continue;
            }
            let rgb = hue_to_rgb(killer.hue);
            scene.push(Primitive::Circle {
                center: killer.base.pos,
                radius: killer.radius,
                color: [
                    (rgb[0] * 255.0).round() as u8,
                    (rgb[1] * 255.0).round() as u8,
                    (rgb[2] * 255.0).round() as u8,
                    77,
                ],
            });
        }
    }
    for tool in world.tools.tools() {
        let (at, shape, loaded) = match tool {
            Tool::Rad(tower) => (tower.base.pos, ToolShape::Rad { radius: tower.radius }, tower.base.loaded),
            Tool::Phero(tower) => (
                tower.base.pos,
                ToolShape::Phero {
                    heading: tower.heading,
                },
                tower.base.loaded,
            ),
            Tool::Killer(killer) => (killer.base.pos, ToolShape::Killer { hue: killer.hue }, false),
        };
        // The towers' prefabs are 0.7725 opaque and `Place()` makes them 1: one that comes from a save keeps the prefab's.
        let alpha = if loaded { LOADED_TOWER_ALPHA } else { 255 };
        push_tool_body(scene, assets, camera, at, shape, alpha, view_min, view_max);
    }
    // `EffectTower.PlacementMode`: the tower being placed follows the cursor at half opacity (the
    // children keep their own colour).
    if let Some((kind, at)) = view.ghost {
        let shape = match kind {
            GhostKind::Rad => ToolShape::Rad { radius: RadTower::new(0, at, sim_size).radius },
            GhostKind::Phero => ToolShape::Phero { heading: 0.0 },
            GhostKind::Killer => ToolShape::Killer { hue: 0.0 },
        };
        push_tool_body(scene, assets, camera, at, shape, 128, view_min, view_max);
    }
}

/// One tower or colour killer: its sprites (or flat shapes without the game's assets). `alpha` is
/// the opacity of the root sprite (the placement ghost is half transparent).
#[allow(clippy::too_many_arguments)]
fn push_tool_body(
    scene: &mut Scene,
    assets: Option<&GameAssets>,
    camera: &Camera,
    at: Vec2,
    shape: ToolShape,
    alpha: u8,
    view_min: Vec2,
    view_max: Vec2,
) {
    // A radiation tower's area (`RadioArea`, `Shader Graphs/radioZone`, under the tower's base disc) reaches far past the tower.
    let body_visible = circle_visible(at, PHERO_TOWER_SIZE, view_min, view_max);
    let area = match shape {
        ToolShape::Rad { radius } if circle_visible(at, radius, view_min, view_max) => Some(radius),
        _ => None,
    };
    if !body_visible && area.is_none() {
        return;
    }
    let sprites = assets.map(|assets| assets.tool_sprites);
    let instance = |sprite: crate::assets::SpriteRef, rotation: f32, tint: Rgba| {
        sprite_primitive(crate::sprites::SpriteInstance {
            sprite,
            anchor: at,
            size: sprite.world_size(),
            pivot: sprite.pivot,
            rotation,
            tint,
            flip_x: false,
            flip_y: false,
            hue_shift: false,
            effects: crate::skin::SkinEffects::NONE,
            warp: None,
        })
    };
    let white = [255, 255, 255, alpha];
    match shape {
        ToolShape::Rad { .. } => {
            // The area first: the base disc (of a tower loaded from a save 0.77 opaque in the game, not drawn so here) and the icon are over it.
            if let Some(radius) = area {
                scene.push(Primitive::RadioField { center: at, radius });
            }
            if let Some(base) = sprites.and_then(|sprites| sprites.base) {
                if body_visible {
                    scene.push(instance(base, 0.0, white));
                    if let Some(icon) = sprites.and_then(|sprites| sprites.rad_icon) {
                        scene.push(instance(icon, 0.0, RAD_ICON_TINT));
                    }
                }
            } else if body_visible {
                scene.push(Primitive::Circle {
                    center: at,
                    radius: TOWER_BASE_SIZE * 0.5,
                    color: [60, 70, 20, alpha],
                });
                scene.push(Primitive::CircleOutline {
                    center: at,
                    radius: TOWER_BASE_SIZE * 0.5,
                    thickness: 2.0 / camera.zoom,
                    color: RAD_ICON_TINT,
                });
            }
        }
        ToolShape::Phero { heading } => {
            if let Some(sprite) = sprites.and_then(|sprites| sprites.phero) {
                // `UpdatePheroSpot`: `transform.right` points along the heading.
                scene.push(instance(sprite, heading, white));
            } else {
                scene.push(Primitive::Circle {
                    center: at,
                    radius: PHERO_TOWER_SIZE * 0.45,
                    color: [90, 60, 120, alpha],
                });
            }
        }
        ToolShape::Killer { hue } => {
            let rgb = hue_to_rgb(hue);
            let crystal_tint = [
                (rgb[0] * 255.0).round() as u8,
                (rgb[1] * 255.0).round() as u8,
                (rgb[2] * 255.0).round() as u8,
                255,
            ];
            if let Some(base) = sprites.and_then(|sprites| sprites.base) {
                scene.push(instance(base, 0.0, white));
            } else {
                scene.push(Primitive::Circle {
                    center: at,
                    radius: TOWER_BASE_SIZE * 0.5,
                    color: [200, 200, 200, alpha],
                });
            }
            if let Some(cell) = sprites.and_then(|sprites| sprites.crystal_hue_cell(hue)) {
                // `ColorCrystal` under `Square` (`UnscaledRotation.rate` 0.5: half a turn a second on the real clock): the art recoloured to the hue.
                scene.push(Primitive::Spinner {
                    sprite: cell,
                    anchor: at,
                    size: cell.world_size(),
                    pivot: cell.pivot,
                    tint: white,
                    turns: 0.5,
                });
            } else if let Some(crystal) = sprites.and_then(|sprites| sprites.crystal) {
                scene.push(instance(crystal, 0.0, crystal_tint));
            } else {
                scene.push(Primitive::Circle {
                    center: at,
                    radius: CRYSTAL_SIZE * 0.5,
                    color: crystal_tint,
                });
            }
        }
    }
}

/// Border of the selected bibite's view cone: radius `ViewRadius * d1Size`,
/// full angle `ViewAngle` degrees (`FieldOfViewDisplay.cs:150-248`).
fn push_view_cone(scene: &mut Scene, camera: &Camera, record: &BibiteRecord) {
    use bibites_core::genome::Gene;
    let radius = record.genome.gene(Gene::ViewRadius) * record.body.geometry.d1_size;
    let angle = record.genome.gene(Gene::ViewAngle).clamp(0.0, 360.0);
    if !radius.is_finite() || radius <= 0.0 || !angle.is_finite() || angle <= 0.0 {
        return;
    }
    let up = Vec2::Y.rotate(record.rot);
    let half = (angle * 0.5).to_radians();
    // `FieldOfViewMat`: `_Color` = (0.1408, 1, 0, 0.1137) fill, white `_borderColor`.
    scene.push(Primitive::Sector {
        center: record.pos,
        radius,
        facing: up,
        half_angle: half,
        color: [36, 255, 0, 29],
    });
    // The border is opaque white; its width on the screen was measured in a live game at four zooms (18, 3.6, 1.35 px per unit: 5, 2 and 2 px across):
    // about 1.3 x the square root of the zoom, at least 2 px.
    let line = (1.3 * camera.zoom.max(0.0).sqrt()).max(2.0) / camera.zoom;
    let color: Rgba = [255, 255, 255, 255];
    let steps = ((angle / 2.0).ceil() as usize).clamp(16, 180);
    let point = |a: f32| record.pos + up.rotate(a) * radius;
    let mut previous = point(-half);
    if angle < 360.0 {
        scene.push(Primitive::Line {
            a: record.pos,
            b: previous,
            thickness: line,
            color,
        });
    }
    for step in 1..=steps {
        let a = -half + (2.0 * half) * step as f32 / steps as f32;
        let next = point(a);
        scene.push(Primitive::Line {
            a: previous,
            b: next,
            thickness: line,
            color,
        });
        previous = next;
    }
    if angle < 360.0 {
        scene.push(Primitive::Line {
            a: record.pos,
            b: previous,
            thickness: line,
            color,
        });
    }
}

/// Draws a zoom-adaptive world-space grid clipped to the simulation square.
///
/// The step is the smallest "nice" (1/2/5 × 10ⁿ) multiple of world units whose
/// on-screen spacing is at least [`GRID_TARGET_PIXELS`], so lines stay readable
/// at every zoom level. The X/Y axes through the origin are slightly brighter.
fn push_grid(scene: &mut Scene, camera: &Camera, world: &World, view_min: Vec2, view_max: Vec2) {
    let min = view_min.max(world.bounds.min());
    let max = view_max.min(world.bounds.max());
    if min.x >= max.x || min.y >= max.y {
        return;
    }
    let step = grid_step(camera.zoom);
    let thickness = 1.0 / camera.zoom;
    let start_x = (min.x / step).floor() * step;
    let start_y = (min.y / step).floor() * step;
    let columns = ((max.x - start_x) / step).ceil().clamp(0.0, 256.0) as u32;
    let rows = ((max.y - start_y) / step).ceil().clamp(0.0, 256.0) as u32;
    for column in 0..=columns {
        let x = start_x + step * column as f32;
        if x > max.x {
            break;
        }
        let color = if x.abs() < step * 0.25 {
            GRID_AXIS
        } else {
            GRID_LINE
        };
        scene.push(Primitive::Line {
            a: Vec2::new(x, min.y),
            b: Vec2::new(x, max.y),
            thickness,
            color,
        });
    }
    for row in 0..=rows {
        let y = start_y + step * row as f32;
        if y > max.y {
            break;
        }
        let color = if y.abs() < step * 0.25 {
            GRID_AXIS
        } else {
            GRID_LINE
        };
        scene.push(Primitive::Line {
            a: Vec2::new(min.x, y),
            b: Vec2::new(max.x, y),
            thickness,
            color,
        });
    }
}

/// Game-faithful background grid: fine/coarse lines at the user's configured
/// steps with the `backgroundMat` palette, drawn inside the simulation square.
fn push_game_grid(
    scene: &mut Scene,
    _camera: &Camera,
    _world: &World,
    _view_min: Vec2,
    _view_max: Vec2,
    style: &crate::assets::BackgroundStyle,
) {
    if style.fine_step <= 0.0 {
        return;
    }
    // The grid fills the whole view, outside the simulation square too (a live screenshot at the world's corner has its lines out to the screen's edges).
    scene.push(Primitive::GridLines {
        fine_step: style.fine_step,
        fine_width: style.fine_width,
        fine_color: style.fine_color,
        coarse_step: style.coarse_step,
        coarse_width: style.coarse_width,
        coarse_color: style.coarse_color,
    });
}
/// `f(v) = 1 - 0.9 * (invLog(v, .001, 100) - 1)^2` from `ZONE_COLOUR.md`.
fn zone_density_factor(value: f32) -> f32 {
    let value = value.clamp(0.001, 100.0).log10();
    let min = 0.001_f32.log10();
    let max = 100.0_f32.log10();
    let inverse = ((value - min) / (max - min)).clamp(0.0, 1.0);
    1.0 - 0.9 * (inverse - 1.0).powi(2)
}
/// World-space grid spacing for a zoom (targets ~64 screen pixels per cell).
fn grid_step(zoom: f32) -> f32 {
    let raw = (GRID_TARGET_PIXELS / zoom.max(1e-6)).max(1e-3);
    let power = 10.0_f32.powf(raw.log10().floor());
    let normalized = raw / power;
    let multiplier = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    power * multiplier
}

/// Pushes one bibite body plus its organ details.
#[allow(clippy::too_many_arguments)]
fn push_bibite(
    scene: &mut Scene,
    camera: &Camera,
    id: BibiteId,
    record: &BibiteRecord,
    view_min: Vec2,
    view_max: Vec2,
    assets: Option<&GameAssets>,
    tail: Option<&TailState>,
    full_detail: bool,
) {
    let Some((a, b, radius)) = body_capsule(record) else {
        return;
    };
    if !capsule_visible(a, b, radius, view_min, view_max) {
        return;
    }
    if let Some(assets) = assets {
        // How long the body is on the screen decides how much of it is drawn: its mouth, eyes and arms are a pixel each from far away and cost a draw each
        // (a dot for a body of a couple of pixels, the body sprite alone for one of a few).
        let length_px = ((b - a).length() + 2.0 * radius) * camera.zoom;
        if !full_detail
            && length_px < BIBITE_DOT_PIXELS
            && let Some(color) = crate::sprites::bibite_dot_colour(assets, record, tail)
        {
            scene.dots.push(Dot { center: (a + b) * 0.5, half: radius, color });
            return;
        }
        let sprites = if !full_detail && length_px < BIBITE_BODY_ONLY_PIXELS {
            crate::sprites::bibite_body_sprite(assets, record, tail)
        } else {
            crate::sprites::bibite_sprites(assets, record, tail)
        };
        if !sprites.is_empty() {
            for sprite in sprites {
                scene.push(sprite_primitive(sprite));
            }
            return;
        }
    }
    let body_color = skin_color(id, record);
    scene.push(Primitive::Capsule {
        a,
        b,
        radius,
        color: body_color,
    });

    // Organ detail only matters when a body is a few pixels wide; when zoomed
    // out the capsule alone (and the backend's LOD dot) is enough.
    let screen_radius = radius * camera.zoom;
    if record.is_corpse || screen_radius < 2.5 {
        return;
    }
    let forward = Vec2::Y.rotate(record.rot);
    let side = forward.perp();

    // Mouth: a short bright bar across the head end of the capsule.
    let head = b + forward * (radius * 0.72);
    let mouth_color = [236, 206, 172, 230];
    scene.push(Primitive::Line {
        a: head - side * (radius * 0.42),
        b: head + side * (radius * 0.42),
        thickness: (radius * 0.30).max(1.0 / camera.zoom),
        color: mouth_color,
    });

    // Health: a thin ring around wounded bodies (green -> red as health drops).
    let health = record.body.health.fullness().clamp(0.0, 1.0);
    if health < 0.995 {
        scene.push(Primitive::CircleOutline {
            center: record.pos,
            radius: radius * 1.45,
            thickness: 1.2 / camera.zoom,
            color: mix([80, 210, 120, 200], [235, 64, 52, 210], 1.0 - health),
        });
    }

    // Fat: a warm ring when the fat organ has inflated the body.
    if record.fat.inflate_factor > 0.05 {
        scene.push(Primitive::CircleOutline {
            center: record.pos,
            radius: radius * 1.2,
            thickness: 1.0 / camera.zoom,
            color: [222, 180, 64, 170],
        });
    }
}

/// Bibite body capsule `(a, b, radius)` in world units, centred on `pos`.
///
/// The collider is a vertical capsule of `width × height` (`CapsuleGeometry`)
/// scaled by `d1Size`; `BODY_VISUAL_SCALE` makes it look like the game's sprite.
fn body_capsule(record: &BibiteRecord) -> Option<(Vec2, Vec2, f32)> {
    let geometry = &record.body.geometry;
    if !geometry.d1_size.is_finite() || geometry.d1_size <= 0.0 {
        return None;
    }
    let capsule = geometry.capsule().scaled(geometry.d1_size);
    let width = capsule.size.x * BODY_VISUAL_SCALE;
    let height = capsule.size.y * BODY_VISUAL_SCALE;
    let radius = (width * 0.5).max(1e-3);
    let half_segment = ((height - width) * 0.5).max(0.0);
    let forward = Vec2::Y.rotate(record.rot);
    let a = record.pos - forward * half_segment;
    let b = record.pos + forward * half_segment;
    Some((a, b, radius))
}

/// How long a bibite's body is, nose to tail, in world units (what decides how much of it is drawn from far away: [`BIBITE_DOT_PIXELS`]; `push_bibite` works
/// it out from the capsule it has at hand).
#[cfg(test)]
pub(crate) fn body_length(record: &BibiteRecord) -> Option<f32> {
    let (a, b, radius) = body_capsule(record)?;
    Some((b - a).length() + 2.0 * radius)
}

/// Body radius (world units) used for hit-testing and the selection ring.
fn body_radius(record: &BibiteRecord) -> f32 {
    let geometry = &record.body.geometry;
    let capsule = geometry.capsule().scaled(geometry.d1_size.max(0.0));
    (capsule.size.x * 0.5 * BODY_VISUAL_SCALE).max(1e-3)
}

/// Converts the genome skin pigments to a display colour (the capsule stand-in, used when no game art is loaded).
///
/// `ColorR/G/B` are 0..=1 genes, and the colour the game's materials get from them is `GenesToColor` of them, not
/// the genes themselves (the basic bibite, 0.1 / 0.1 / 0.1, is light grey). Corpses are desaturated, damaged bodies
/// tint towards red. `is_rain` easter-egg bibites get a deterministic rainbow hue.
fn skin_color(id: BibiteId, record: &BibiteRecord) -> Rgba {
    if record.is_corpse {
        let decay = record.body.corpse_decay.clamp(0.0, 1.0);
        return mix(CORPSE, [58, 40, 38, 235], decay);
    }
    if !record.is_ready {
        return [96, 100, 108, 235];
    }

    let rgb = if record.meta.is_rain {
        hue_color(id.handle().slot() as f32 * 0.618_034 % 1.0)
    } else {
        pigment_rgb(record.color)
    };

    // Wounded bibites trend red so population health is visible at a glance.
    let mut color = [rgb[0], rgb[1], rgb[2], 235];
    let health = record.body.health.fullness().clamp(0.0, 1.0);
    if health < 0.5 {
        let tint = (0.5 - health) * 1.2;
        color = mix(color, [225, 56, 44, 235], tint);
    }
    color
}

/// Converts the three skin-pigment genes (`0..=1`, `ColorR/G/B`) to the 8-bit body colour of the game
/// (`BibiteGenes.GenesToColor`: the basic bibite's 0.1 / 0.1 / 0.1 is 230 / 230 / 230).
#[must_use]
pub fn pigment_rgb(genes: [f32; 3]) -> [u8; 3] {
    let color = crate::assets::genes_to_color(genes[0], genes[1], genes[2]);
    [
        unit_to_u8(color[0]),
        unit_to_u8(color[1]),
        unit_to_u8(color[2]),
    ]
}

/// Deterministic rainbow colour from a `0..=1` hue.
fn hue_color(hue: f32) -> [u8; 3] {
    let h = (hue.fract() + 1.0).fract() * 6.0;
    let sector = h.floor() as i32;
    let f = h - h.floor();
    let (r, g, b) = match sector {
        0 => (1.0, f, 0.0),
        1 => (1.0 - f, 1.0, 0.0),
        2 => (0.0, 1.0, f),
        3 => (0.0, 1.0 - f, 1.0),
        4 => (f, 0.0, 1.0),
        _ => (1.0, 0.0, 1.0 - f),
    };
    [unit_to_u8(r), unit_to_u8(g), unit_to_u8(b)]
}

#[inline]
fn unit_to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Linear blend between two RGBA colours (`t` in `0..=1`).
#[must_use]
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let blend = |x: u8, y: u8| -> u8 {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [
        blend(a[0], b[0]),
        blend(a[1], b[1]),
        blend(a[2], b[2]),
        blend(a[3], b[3]),
    ]
}

fn rects_intersect(a_min: Vec2, a_max: Vec2, b_min: Vec2, b_max: Vec2) -> bool {
    a_min.x <= b_max.x && a_max.x >= b_min.x && a_min.y <= b_max.y && a_max.y >= b_min.y
}

fn circle_visible(center: Vec2, radius: f32, view_min: Vec2, view_max: Vec2) -> bool {
    center.x + radius >= view_min.x
        && center.x - radius <= view_max.x
        && center.y + radius >= view_min.y
        && center.y - radius <= view_max.y
}

fn capsule_visible(a: Vec2, b: Vec2, radius: f32, view_min: Vec2, view_max: Vec2) -> bool {
    let min = Vec2::new(a.x.min(b.x) - radius, a.y.min(b.y) - radius);
    let max = Vec2::new(a.x.max(b.x) + radius, a.y.max(b.y) + radius);
    rects_intersect(min, max, view_min, view_max)
}

// ---------------------------------------------------------------------------
// Window backend (macroquad)
// ---------------------------------------------------------------------------

/// Uploads the CPU atlases to GPU textures for the interactive window.
///
/// Must be called with a live macroquad context (inside the window loop's
/// async main). Point filtering matches the game's `FilterMode.Point` sprites.
#[must_use]
pub fn upload_atlases(assets: Option<&GameAssets>) -> Vec<macroquad::prelude::Texture2D> {
    let Some(assets) = assets else {
        return Vec::new();
    };
    assets
        .atlases
        .iter()
        .map(|atlas| {
            let texture = macroquad::prelude::Texture2D::from_rgba8(
                atlas.width as u16,
                atlas.height as u16,
                &atlas.rgba,
            );
            texture.set_filter(macroquad::prelude::FilterMode::Nearest);
            texture
        })
        .collect()
}

/// Vertex shader of the hue material: macroquad's own (`color0` is a byte vector, hence `/ 255.0`), nothing more.
const HUE_VERTEX_SHADER: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;

varying mediump vec2 uv;
varying mediump vec4 vcolor;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    uv = texcoord;
    vcolor = color0 / 255.0;
}"#;

/// Fragment shader of the hue material: the game's skin shader (`crate::skin`) on the art texel and the tint. The tint is the
/// vertex colour (what the default pipeline would multiply with), so no uniform changes between sprites; `skin_params` are the
/// shader globals (`_FromHue`, `_VminAtS0`, `_VmaxAtS0`), set once per frame.
const HUE_FRAGMENT_SHADER: &str = r#"#version 100
precision mediump float;

varying mediump vec2 uv;
varying mediump vec4 vcolor;

uniform sampler2D Texture;
uniform vec3 skin_params;
uniform vec2 effects;

vec3 rgb2hsv(vec3 c) {
    vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
    vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
    float d = q.x - min(q.w, q.y);
    // No epsilon in a denominator: with `q.x + 1.0e-7` a pure black tint (q.x = 0, d = 0) is 0 / 1.0e-7 = 0 in the maths (and in the tests' replica), but in the
    // real window every texel of a pigments-1 bibite came out black (the game keeps its outline and arms grey) while a tint of 0.02 was right: what a card
    // does with that quotient (a lost epsilon makes it 0 / 0 = NaN) is not something a test can see. Zero saturation and hue are what the epsilon stood
    // for, so they are chosen outright.
    float hue = d > 0.0 ? abs(q.z + (q.w - q.y) / (6.0 * d)) : 0.0;
    float sat = q.x > 0.0 ? d / q.x : 0.0;
    return vec3(hue, sat, q.x);
}

vec3 hsv2rgb(vec3 c) {
    vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}

void main() {
    vec4 tex = texture2D(Texture, uv);
    float alpha = tex.a - 0.75 * effects.y;
    if (tex.a <= 0.0 || vcolor.a <= 0.0 || alpha <= 0.0) {
        discard;
    }
    vec3 art = rgb2hsv(tex.rgb);
    vec3 color = rgb2hsv(vcolor.rgb * (1.0 - effects.y));
    float widened = skin_params.z + color.y * (1.0 - skin_params.z);
    float lifted = color.z * (widened - skin_params.y) + skin_params.y;
    float shade = clamp(lifted, min(widened, skin_params.y), max(widened, skin_params.y));
    vec3 hsv = vec3(art.x - skin_params.x + color.x, color.y * art.y, art.z * (1.0 - art.y * (1.0 - shade)));
    vec3 skin = hsv2rgb(hsv);
    skin = mix(skin, vec3(1.0, 0.1011, 0.0), 0.75 * effects.x);
    gl_FragColor = vec4(skin, alpha * vcolor.a);
}"#;

/// GPU material for the bibites' skin, eye and egg sprites: the game's skin shader maths ([`crate::skin`]), the same as the
/// software rasteriser's `hue_shift` path.
///
/// The extracted part art is one blue hue (0.549) with the shading in its saturation and value, so the default pipeline's
/// plain `art * tint` multiplication gives blue bibites whatever their genes ("the basic bibite looks blue": its light grey
/// body, saturation 0, needs the art's colour removed, not multiplied). The window therefore draws those sprites with this
/// material; the tint travels in the vertex colour, so the sprites of one atlas still go out in one batch.
///
/// `--screenshot` uses the software rasteriser, which has the same maths.
pub struct GpuHueMaterial {
    material: macroquad::material::Material,
}

impl GpuHueMaterial {
    /// Compiles the GLSL hue material. Must be called with a live macroquad
    /// context (window path only); OpenGL backends only.
    pub fn load(params: SkinParams) -> Result<Self, String> {
        use macroquad::material::{MaterialParams, load_material};
        use macroquad::miniquad::{BlendFactor, BlendState, BlendValue, Equation};
        use macroquad::miniquad::{UniformDesc, UniformType};
        use macroquad::prelude::{PipelineParams, ShaderSource};

        let pipeline_params = PipelineParams {
            color_blend: Some(BlendState::new(
                Equation::Add,
                BlendFactor::Value(BlendValue::SourceAlpha),
                BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
            )),
            ..Default::default()
        };
        let material = load_material(
            ShaderSource::Glsl {
                vertex: HUE_VERTEX_SHADER,
                fragment: HUE_FRAGMENT_SHADER,
            },
            MaterialParams {
                pipeline_params,
                uniforms: vec![UniformDesc::new("skin_params", UniformType::Float3), UniformDesc::new("effects", UniformType::Float2)],
                ..Default::default()
            },
        )
        .map_err(|error| format!("GPU hue material failed: {error}"))?;
        let loaded = Self { material };
        loaded.set_params(params);
        Ok(loaded)
    }

    /// Sets the per-sprite `[_HurtAmount, _IsKiller]`.
    pub fn set_effects(&self, effects: [f32; 2]) {
        self.material.set_uniform("effects", effects);
    }

    /// Sets the shader globals (`_FromHue`, `_VminAtS0`, `_VmaxAtS0`).
    pub fn set_params(&self, params: SkinParams) {
        self.material.set_uniform(
            "skin_params",
            [params.from_hue, params.vmin_at_s0, params.vmax_at_s0],
        );
    }
}

/// Builds the display-list primitive for one composed sprite.
fn sprite_primitive(sprite: crate::sprites::SpriteInstance) -> Primitive {
    Primitive::Sprite {
        sprite: sprite.sprite,
        anchor: sprite.anchor,
        size: sprite.size,
        pivot: sprite.pivot,
        rotation: sprite.rotation,
        tint: sprite.tint,
        flip_x: sprite.flip_x,
        flip_y: sprite.flip_y,
        hue_shift: sprite.hue_shift,
        effects: sprite.effects,
        warp: sprite.warp,
    }
}

/// A [`Primitive::Spinner`] as the sprite it is at this moment.
fn spinner_sprite(primitive: Primitive) -> Primitive {
    let Primitive::Spinner { sprite, anchor, size, pivot, tint, turns } = primitive else {
        return primitive;
    };
    sprite_primitive(crate::sprites::SpriteInstance {
        sprite,
        anchor,
        size,
        pivot,
        rotation: effect_time() * turns * std::f32::consts::TAU,
        tint,
        flip_x: false,
        flip_y: false,
        hue_shift: false,
        effects: crate::skin::SkinEffects::NONE,
        warp: None,
    })
}

/// Reorders `primitives[first..]` so that the sprites that use the same atlas texture come one after the other (the other
/// primitives first, each group in its original order).
///
/// The window ends a batch of draw calls whenever the texture changes, and pellets of different sizes use different atlases and
/// alternate in the store's order: with 55,000 pellets in view that was 24,000 texture changes, 110 ms of draw calls per frame
/// (8 frames per second on a GTX 1660 SUPER); grouped, there is one batch per atlas. Pellets of one material that overlap
/// would swap places in the draw order only if their sprites were in different atlases.
fn group_sprites_by_atlas(
    primitives: &mut [Primitive],
    first: usize,
    scratch: &mut Vec<Primitive>,
) {
    let start = first.min(primitives.len());
    let tail = &mut primitives[start..];
    // Nothing to do when the atlas never changes between neighbours.
    let mut last = None;
    let mut max_atlas = 0_usize;
    let mut switches = false;
    for primitive in tail.iter() {
        if let Primitive::Sprite { sprite, .. } | Primitive::Spinner { sprite, .. } = primitive {
            max_atlas = max_atlas.max(usize::from(sprite.atlas));
            switches |= last.is_some_and(|atlas| atlas != sprite.atlas);
            last = Some(sprite.atlas);
        }
    }
    if !switches || tail.is_empty() {
        return;
    }
    // Counting sort: bucket 0 holds everything that is not a sprite, bucket 1 + atlas the sprites of that atlas.
    let bucket = |primitive: &Primitive| match primitive {
        Primitive::Sprite { sprite, .. } | Primitive::Spinner { sprite, .. } => 1 + usize::from(sprite.atlas),
        _ => 0,
    };
    let mut starts = vec![0_usize; max_atlas + 3];
    for primitive in tail.iter() {
        starts[bucket(primitive) + 1] += 1;
    }
    for index in 1..starts.len() {
        starts[index] += starts[index - 1];
    }
    scratch.clear();
    scratch.resize(tail.len(), tail[0]);
    for primitive in tail.iter() {
        let slot = &mut starts[bucket(primitive)];
        scratch[*slot] = *primitive;
        *slot += 1;
    }
    tail.copy_from_slice(scratch);
}

/// Whether the window draws a primitive with the hue material: the sprites that carry a skin, eye or egg tint
/// (`hue_shift`). Everything else (pellets, shapes, lines) uses the default material.
fn wants_hue_material(primitive: &Primitive) -> bool {
    // A sprite with the virus, the rainbow or the producer pattern is drawn from a picture the software rasteriser makes with the game's whole pixel program.
    matches!(
        primitive,
        Primitive::Sprite {
            hue_shift: true,
            effects,
            ..
        } if !effects.needs_program()
    )
}

/// How the window paints a primitive (see [`painting_of`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Painting {
    /// From a picture the software rasteriser makes of the sprite: the picture is already coloured and goes out with the default material.
    Picture,
    /// With the hue material: a skin, eye or egg sprite straight from its sheet.
    HueMaterial,
    /// Without the hue material (a shader the driver did not take, or `--no-gpu-hue-shader`): a skin, eye or egg sprite from a copy of its cell of the sheet that the
    /// skin shader's maths has recoloured on the processor ([`draw_recoloured_sprite`]), drawn with the default material. Multiplied with the tint the blue art
    /// turns blue, and a black tint (the pigments at 1) turns the whole bibite black, the outline and the arms the game keeps grey included.
    Recoloured,
    /// With the default material.
    Default,
}

/// How the window paints `primitive`: the special skins (virus, rainbow, producer: the game's whole pixel program) and a bent body or tail (`warp`: the game's
/// `_MiddleSegmentAngle` / `_TailAngle`) seen big enough for the bend to show come from a picture the software rasteriser makes, when there are software atlases to
/// make it with; the other `hue_shift` sprites go through the hue material, or, without it, come from a recoloured copy of their cell ([`Painting::Recoloured`]; without
/// software atlases they are multiplied with their tint, as before).
///
/// One decision for the material binding and the sprite itself: a finished picture drawn while the hue material was bound had its colour taken away again (its
/// white vertex colour is a saturation of 0), so a coloured body turned grey while its arms, mouth and eyes, which carry their own tint, kept theirs.
fn painting_of(primitive: &Primitive, camera: &Camera, has_hue_material: bool, has_atlases: bool) -> Painting {
    if has_atlases && drawn_from_picture(primitive, camera) {
        Painting::Picture
    } else if wants_hue_material(primitive) {
        match (has_hue_material, has_atlases) {
            (true, _) => Painting::HueMaterial,
            (false, true) => Painting::Recoloured,
            (false, false) => Painting::Default,
        }
    } else {
        Painting::Default
    }
}

/// See [`painting_of`]: a sprite with a pixel-program skin, or a bent one that is more than 24 pixels across.
fn drawn_from_picture(primitive: &Primitive, camera: &Camera) -> bool {
    let Primitive::Sprite { size, effects, warp, .. } = *primitive else {
        return false;
    };
    let bent_and_big = warp.is_some_and(|warp| warp.angle0.abs().max(warp.angle1.abs()) > 0.01) && size.x.max(size.y) * camera.zoom > 24.0;
    effects.needs_program() || bent_and_big
}

/// How far in from the border of its cell the window keeps the rectangle it samples for an atlas sprite, in screen pixels.
///
/// The window's surface is multisampled (`Conf` asks for `sample_count: 1` and the WGL pixel-format chooser takes the smallest multisampled format: 2x on
/// an NVIDIA card). A pixel on the edge of a sprite's quad whose centre lies outside the quad but one of whose samples lies inside runs the fragment
/// program with the texture coordinate extrapolated past the quad, by up to the distance of a sample from the pixel's centre (0.35 px at 2x, 0.4 at 4x, at
/// most 0.71 for any pattern), and the part sheets are packed cell against cell with art on the cells' borders: such a pixel showed the neighbouring cell's
/// texel at the coverage of its samples, a faint dotted line along the sprite's rectangle where the game and the software rasteriser (strict
/// texture-coordinate tests) draw nothing (measured in the real window: 48 such pixels in empty space round a bibite at 17.5 px per world unit without the
/// inset, none with it).
/// With the sampled rectangle kept this far inside the cell, the extrapolated coordinate stays in the sprite's own cell, whose edge texel is what the
/// software rasteriser shows there. The quad itself is not changed, so the silhouette and the pixels it covers are the same; the price is that the
/// texel boundaries inside the sprite move by less than this (the mapping is scaled by a fraction of a percent about the cell's middle).
///
/// `RUSTIBITES_SPRITE_INSET_PIXELS` overrides it (0 switches the inset off) to compare values in a real window (`tools/window_hairline_probe.ps1`).
const SPRITE_INSET_PIXELS: f32 = 0.75;
/// The most the inset takes from a texel: a sprite shown at about a pixel per texel keeps its edge texels (an inset of half a texel would sample the
/// middle of the edge texel; more would make the outermost pixels read the second one).
const SPRITE_INSET_MAX_TEXELS: f32 = 0.45;

/// [`SPRITE_INSET_PIXELS`], or the value of `RUSTIBITES_SPRITE_INSET_PIXELS` when it is set to a number.
fn sprite_inset_pixels() -> f32 {
    static INSET: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *INSET.get_or_init(|| {
        std::env::var("RUSTIBITES_SPRITE_INSET_PIXELS")
            .ok()
            .and_then(|value| value.trim().parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(SPRITE_INSET_PIXELS)
    })
}

/// The rectangle of its atlas (`[x, y, width, height]` in texels) that the window samples for a sprite whose quad is `destination` pixels wide and
/// high: see [`inset_rect`].
fn inset_sprite_source(rect: [u32; 4], destination: [f32; 2]) -> [f32; 4] {
    inset_rect(rect, destination, sprite_inset_pixels())
}

/// The sprite's own cell, inset by `inset_pixels` screen pixels on every side (at most [`SPRITE_INSET_MAX_TEXELS`] of a texel).
fn inset_rect(rect: [u32; 4], destination: [f32; 2], inset_pixels: f32) -> [f32; 4] {
    let rect = rect.map(|value| value as f32);
    // The inset along one axis, in texels, for a cell `texels` wide shown `pixels` wide; a degenerate size leaves that axis alone.
    let inset = |texels: f32, pixels: f32| {
        if texels > 0.0 && pixels > 0.0 && pixels.is_finite() {
            (inset_pixels * texels / pixels).min(SPRITE_INSET_MAX_TEXELS)
        } else {
            0.0
        }
    };
    let (inset_x, inset_y) = (inset(rect[2], destination[0]), inset(rect[3], destination[1]));
    [rect[0] + inset_x, rect[1] + inset_y, rect[2] - 2.0 * inset_x, rect[3] - 2.0 * inset_y]
}

/// The clock of the animated shaders that run on the game's `Time.time` (a paused world goes on animating: the game's time scale stays 1), in seconds:
/// the window sets it every frame; a screenshot has the 0 it starts with.
static EFFECT_TIME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// See [`EFFECT_TIME`].
pub fn set_effect_time(seconds: f32) {
    EFFECT_TIME.store(seconds.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// See [`EFFECT_TIME`].
#[must_use]
pub fn effect_time() -> f32 {
    f32::from_bits(EFFECT_TIME.load(std::sync::atomic::Ordering::Relaxed))
}

/// `RadioArea`'s material (`Shader Graphs/radioZone`): the asset's wave period, animation speed, units per pixel and quantisation steps (`_size` is set
/// to the area's diameter by `RadioArea.UpdateProperties`, `_Color` to `RadioArea.color` (176, 255, 0, 128)).
const RADIO_WAVE_PERIOD: f32 = 4.0;
/// See [`RADIO_WAVE_PERIOD`].
const RADIO_ANIMATION_SPEED: f32 = 0.25;
/// World units per cell of the area's pixel grid (`_upp`).
const RADIO_UPP: f32 = 5.0;
/// See [`RADIO_WAVE_PERIOD`].
const RADIO_STEPS: f32 = 48.0;
/// Most cells one area draws in a frame; farther out the cells are smaller than a pixel and every few are drawn.
const RADIO_MAX_CELLS: i64 = 40_000;

/// The cells of a radiation area inside `view` (world corners) that the game's pixel program draws: `emit(min, max, colour)` for each one that is not discarded,
/// the colour straight RGBA. The program only reads `floor(uv * cells)`, so a cell is one evaluation (at its middle); the picture is the pixel-art grid
/// of the game.
fn radio_cells(center: Vec2, radius: f32, time: f32, view: (Vec2, Vec2), mut emit: impl FnMut(Vec2, Vec2, Rgba)) {
    // The glow moves by 1 / 48 of an alpha at a time: the clock is kept to 30 steps a second and the cells of the last frames are kept, so a window at
    // 144 frames a second runs the program for a fifth of them (and a paused picture for none after the first).
    thread_local! {
        static CACHE: std::cell::RefCell<Vec<(RadioKey, Vec<(Vec2, Vec2, Rgba)>)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    const RADIO_HZ: f32 = 30.0;
    let step = (time * RADIO_HZ).floor();
    let key = RadioKey {
        center: [center.x.to_bits(), center.y.to_bits()],
        radius: radius.to_bits(),
        step: step as i64,
        view: [view.0.x.to_bits(), view.0.y.to_bits(), view.1.x.to_bits(), view.1.y.to_bits()],
    };
    let hit = CACHE.with(|cache| {
        let cache = cache.borrow();
        cache.iter().find(|(known, _)| *known == key).is_some_and(|(_, cells)| {
            for &(a, b, rgba) in cells {
                emit(a, b, rgba);
            }
            true
        })
    });
    if hit {
        return;
    }
    let mut cells = Vec::new();
    radio_cells_uncached(center, radius, step / RADIO_HZ, view, |a, b, rgba| cells.push((a, b, rgba)));
    for &(a, b, rgba) in &cells {
        emit(a, b, rgba);
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= 6 {
            cache.remove(0);
        }
        cache.push((key, cells));
    });
}

/// What a cached run of [`radio_cells`] is for.
#[derive(Clone, PartialEq)]
struct RadioKey {
    center: [u32; 2],
    radius: u32,
    step: i64,
    view: [u32; 4],
}

/// [`radio_cells`] without the cache: one evaluation of the game's program per cell.
fn radio_cells_uncached(center: Vec2, radius: f32, time: f32, view: (Vec2, Vec2), mut emit: impl FnMut(Vec2, Vec2, Rgba)) {
    let size = 2.0 * radius;
    if !(size > 0.0) {
        return;
    }
    let cells = size / RADIO_UPP;
    let origin = center - Vec2::new(radius, radius);
    let last = cells.ceil() as i64;
    let range = |lo: f32, hi: f32, from: f32| -> (i64, i64) {
        let a = (((lo - from) / size * cells).floor() as i64).max(0);
        let b = (((hi - from) / size * cells).ceil() as i64).min(last);
        (a, b)
    };
    let ((i0, i1), (j0, j1)) = (range(view.0.x, view.1.x, origin.x), range(view.0.y, view.1.y, origin.y));
    if i1 <= i0 || j1 <= j0 {
        return;
    }
    // Far out each step of the loop covers several cells.
    let stride = (((i1 - i0) * (j1 - j0)) as f32 / RADIO_MAX_CELLS as f32).sqrt().ceil().max(1.0) as i64;
    let mut cb0 = vec![[0.0f32; 4]; 131];
    cb0[130][0] = time;
    let f = |c: u8| f32::from(c) / 255.0;
    let cb1 = [
        [f(176), f(255), f(0), f(128)],
        [RADIO_WAVE_PERIOD, RADIO_ANIMATION_SPEED, size, RADIO_UPP],
        [RADIO_STEPS, 0.0, 0.0, 0.0],
    ];
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut j = j0;
    while j < j1 {
        let mut i = i0;
        while i < i1 {
            // The middle of the part of the step that lies on the quad (the last cell of a size that is not a whole number of cells is short).
            let middle = |i: i64| (i as f32 + 0.5 * (cells - i as f32).min(stride as f32)) / cells;
            let uv = [middle(i), middle(j)];
            if let Some(out) = crate::internals_shaders::radio_pixel(uv, [1.0; 4], &cb0, &cb1, &|_| [1.0; 4])
                && out[3] > 0.0
            {
                let corner = |i: i64, j: i64| origin + Vec2::new((i as f32).min(cells) / cells * size, (j as f32).min(cells) / cells * size);
                emit(corner(i, j), corner(i + stride, j + stride), [byte(out[0]), byte(out[1]), byte(out[2]), byte(out[3])]);
            }
            i += stride;
        }
        j += stride;
    }
}

/// The experimental background at a world point: the shader's own program with its grid switched off (`_showGrid` 0, both line alphas 0), cells from
/// the game's material values. The camera globals are guessed from the program (`cb0[21]` the camera's place, `cb0[25]` the orthographic half sizes).
fn background_cells(world: Vec2, camera: &Camera, time: f32, color: Rgba, cell: Rgba) -> Option<[f32; 4]> {
    let f = |c: u8| f32::from(c) / 255.0;
    let half = Vec2::new(camera.screen.x / camera.zoom * 0.5, camera.screen.y / camera.zoom * 0.5);
    let cb0: Vec<[f32; 4]> = (0..26)
        .map(|slot| match slot {
            19 => [time, 0.0, 0.0, 0.0],
            21 => [camera.center.x, camera.center.y, 0.0, 0.0],
            25 => [half.x, half.y, 0.0, 0.0],
            _ => [0.0; 4],
        })
        .collect();
    let cb1 = [
        [f(color[0]), f(color[1]), f(color[2]), f(color[3])],
        [1.5, 3.0, 0.0, 0.0],
        [1.0, 1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0, 0.0],
        [100.0, 1000.0, 0.1, 0.0],
        [0.0, 0.0, 0.0, 0.0],
        [f(cell[0]), f(cell[1]), f(cell[2]), f(cell[3])],
        [1.49, 1.75, 1.87, 2.0],
        [0.02, 0.015, 0.01, 0.0075],
        [0.8, 0.8, 0.8, 0.8],
        [0.2, 0.2, 0.2, 0.3],
        [100.0, 200.0, 500.0, 1000.0],
        [0.1, 0.05, 1.0, 0.0],
    ];
    crate::internals_shaders::bg_pixel([0.5, 0.5], [1.0; 4], [world.x, world.y], &cb0, &cb1, &|_| [1.0; 4])
}

/// Draws the microscope shade in the window: the ring is computed at a quarter of the resolution and stretched (it is a smooth gradient).
fn draw_microscope_shade(camera: &Camera, start: f32, fade: f32, gpu: Option<&GpuZoneMaterial>) {
    use macroquad::prelude::{DrawTextureParams, FilterMode, Texture2D, WHITE, draw_texture_ex};
    // Nothing of the shade is on the screen while the whole view lies inside the circle.
    let corner = Vec2::new(camera.screen.x * 0.5, camera.screen.y * 0.5) / camera.zoom;
    if (camera.center.length() + corner.length()) < start {
        return;
    }
    // The GPU draws it: one quad over the window, the shader's maths per pixel (see `GpuZoneMaterial`).
    if let Some(shade) = gpu.and_then(|gpu| gpu.shade.as_ref()) {
        use macroquad::material::{gl_use_default_material, gl_use_material};
        gl_use_material(shade);
        shade.set_uniform("shade_cam", [camera.center.x, camera.center.y, camera.zoom, start]);
        shade.set_uniform("shade_size", [camera.screen.x, camera.screen.y, fade]);
        macroquad::prelude::draw_rectangle(0.0, 0.0, camera.screen.x, camera.screen.y, WHITE);
        gl_use_default_material();
        return;
    }
    let (w, h) = ((camera.screen.x / 4.0).ceil().max(1.0), (camera.screen.y / 4.0).ceil().max(1.0));
    let mini = Camera { center: camera.center, zoom: camera.zoom / 4.0, screen: Vec2::new(w, h) };
    let mut scene = Scene::new();
    scene.background = [0, 0, 0, 0];
    scene.push(Primitive::MicroscopeShade { start, fade });
    let image = render_scene(&scene, &mini, w as u32, h as u32, 1, &[]);
    let texture = Texture2D::from_rgba8(w as u16, h as u16, &image.rgba);
    texture.set_filter(FilterMode::Linear);
    draw_texture_ex(&texture, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(macroquad::prelude::vec2(camera.screen.x, camera.screen.y)), ..Default::default() });
}

/// One grid line's mask at the world coordinate `coordinate` (`Shader Graphs/background`): 1 within a solid core, falling to 0 over the outer tenth
/// of the half width (`_LineFuziness`), for lines every `step` units that are `width` across.
fn grid_line_mask(coordinate: f32, step: f32, width: f32) -> f32 {
    const FUZZINESS: f32 = 0.1;
    let distance = (coordinate + step * 0.5).rem_euclid(step) - step * 0.5;
    let half = width * 0.5;
    let core = (1.0 - FUZZINESS) * half;
    let fade = half - core;
    if fade <= 0.0 {
        return if distance.abs() <= core { 1.0 } else { 0.0 };
    }
    (1.0 - (distance.abs() - core) / fade).clamp(0.0, 1.0)
}

/// The window's grid: the lines of [`Primitive::GridLines`] as strokes (fine lines are left out under a coarse one).
fn grid_strokes(camera: &Camera, primitive: Primitive) -> Vec<(Vec2, Vec2, f32, Rgba)> {
    let Primitive::GridLines { fine_step, fine_width, fine_color, coarse_step, coarse_width, coarse_color } = primitive else {
        return Vec::new();
    };
    let corners = [camera.screen_to_world(Vec2::ZERO), camera.screen_to_world(camera.screen)];
    let (min, max) = (corners[0].min(corners[1]), corners[0].max(corners[1]));
    let mut strokes = Vec::new();
    for (index, (step, color, width)) in [(fine_step, fine_color, fine_width), (coarse_step, coarse_color, coarse_width)].into_iter().enumerate() {
        if step <= 0.0 || color[3] == 0 {
            continue;
        }
        let under_coarse = |at: f32| {
            index == 0 && coarse_step > 0.0 && coarse_color[3] >= color[3] && ((at / coarse_step).round() * coarse_step - at).abs() < 1e-3 * coarse_step
        };
        let start_x = (min.x / step).floor() * step;
        let start_y = (min.y / step).floor() * step;
        let columns = ((max.x - start_x) / step).ceil().clamp(0.0, 512.0) as u32;
        let rows = ((max.y - start_y) / step).ceil().clamp(0.0, 512.0) as u32;
        for column in 0..=columns {
            let x = start_x + step * column as f32;
            if x <= max.x && !under_coarse(x) {
                strokes.push((Vec2::new(x, min.y), Vec2::new(x, max.y), width, color));
            }
        }
        for row in 0..=rows {
            let y = start_y + step * row as f32;
            if y <= max.y && !under_coarse(y) {
                strokes.push((Vec2::new(min.x, y), Vec2::new(max.x, y), width, color));
            }
        }
    }
    strokes
}

/// A bilinear sample of `source` at the texel coordinates `(u, v)` (texel centres at half integers), clamped at the edges.
/// Straight-alpha blend of `src` over `dst` (the arithmetic of [`Raster::blend`], for a pixel already known to be inside the image and the clip).
#[inline]
fn blend_over(dst: &mut Rgba, src: Rgba) {
    let src_alpha = u32::from(src[3]);
    if src_alpha == 0 {
        return;
    }
    let dst_alpha = u32::from(dst[3]);
    if dst_alpha == 0 {
        *dst = src;
        return;
    }
    if src_alpha >= 255 && dst_alpha >= 255 {
        *dst = src;
        return;
    }
    let inv = 255 - src_alpha;
    if dst_alpha == 255 {
        // Over an opaque pixel the result's alpha is `src_alpha + 255 * inv / 255 = 255` and the numerator below is `255 * (src * src_alpha + dst * inv)`,
        // so each channel is `(src * src_alpha + dst * inv) / 255`: the same integers without a division by a value only known at run time.
        for channel in 0..3 {
            dst[channel] = ((u32::from(src[channel]) * src_alpha + u32::from(dst[channel]) * inv) / 255) as u8;
        }
        dst[3] = 255;
        return;
    }
    let out_alpha = src_alpha + dst_alpha * inv / 255;
    if out_alpha == 0 {
        *dst = [0, 0, 0, 0];
        return;
    }
    for channel in 0..3 {
        let numerator = u32::from(src[channel]) * src_alpha * 255 + u32::from(dst[channel]) * dst_alpha * inv;
        dst[channel] = (numerator / (255 * out_alpha)).min(255) as u8;
    }
    dst[3] = out_alpha as u8;
}

/// A texel multiplied by a tint, channel by channel (`texel * tint / 255`, the alpha too).
#[inline]
fn tinted(pixel: Rgba, tint: Rgba) -> Rgba {
    [
        (u32::from(pixel[0]) * u32::from(tint[0]) / 255) as u8,
        (u32::from(pixel[1]) * u32::from(tint[1]) / 255) as u8,
        (u32::from(pixel[2]) * u32::from(tint[2]) / 255) as u8,
        (u32::from(pixel[3]) * u32::from(tint[3]) / 255) as u8,
    ]
}

/// The one colour of every texel a draw samples, when there is one: the texels are the box that the columns `us` and the rows `vs` span.
/// `None` when two differ, or when a texel is missing from the data.
fn uniform_texel(texels: &[u8], width: u32, us: &[u32], vs: &[u32]) -> Option<Rgba> {
    let (u0, u1) = (*us.iter().min()?, *us.iter().max()?);
    let (v0, v1) = (*vs.iter().min()?, *vs.iter().max()?);
    let first_at = ((v0 * width + u0) * 4) as usize;
    let first = texels.get(first_at..first_at + 4)?;
    for v in v0..=v1 {
        let from = ((v * width + u0) * 4) as usize;
        let row = texels.get(from..from + (u1 - u0 + 1) as usize * 4)?;
        if row.chunks_exact(4).any(|texel| texel != first) {
            return None;
        }
    }
    Some([first[0], first[1], first[2], first[3]])
}

/// What [`sample_bilinear`] works out from one coordinate of the texture point: the two texels it blends along that axis and their weights.
struct FilterAxis {
    /// The first texel and the one after it (clamped to the texture).
    first: u32,
    second: u32,
    /// The weight of the second, and of the first (`1 - weight`).
    weight: f32,
    rest: f32,
}

impl FilterAxis {
    fn of(point: f32, extent: u32) -> Self {
        let f = (point - 0.5).clamp(0.0, (extent - 1) as f32);
        let first = f.floor() as u32;
        let weight = f - first as f32;
        Self { first, second: (first + 1).min(extent - 1), weight, rest: 1.0 - weight }
    }
}

/// [`sample_bilinear`] for a texture whose data holds every texel, with the axis terms already worked out: the same float operations in the same
/// order, so the same bytes.
#[inline]
fn filter_texels(rgba: &[u8], width: u32, x: &FilterAxis, y: &FilterAxis) -> [u8; 4] {
    let (top, bottom) = ((y.first * width) as usize * 4, (y.second * width) as usize * 4);
    let (a, b) = (top + x.first as usize * 4, top + x.second as usize * 4);
    let (c, d) = (bottom + x.first as usize * 4, bottom + x.second as usize * 4);
    let mut out = [0_u8; 4];
    for channel in 0..4 {
        let upper = f32::from(rgba[a + channel]) * x.rest + f32::from(rgba[b + channel]) * x.weight;
        let lower = f32::from(rgba[c + channel]) * x.rest + f32::from(rgba[d + channel]) * x.weight;
        out[channel] = (upper * y.rest + lower * y.weight).round().clamp(0.0, 255.0) as u8;
    }
    out
}

fn sample_bilinear(source: &crate::ui::UiTexture, u: f32, v: f32) -> [u8; 4] {
    let (width, height) = (source.width.max(1), source.height.max(1));
    let fx = (u - 0.5).clamp(0.0, (width - 1) as f32);
    let fy = (v - 0.5).clamp(0.0, (height - 1) as f32);
    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (wx, wy) = (fx - x0 as f32, fy - y0 as f32);
    let texel = |x: u32, y: u32| -> [f32; 4] {
        let at = ((y * width + x) * 4) as usize;
        source.rgba.get(at..at + 4).map_or([0.0; 4], |p| [f32::from(p[0]), f32::from(p[1]), f32::from(p[2]), f32::from(p[3])])
    };
    let (a, b, c, d) = (texel(x0, y0), texel(x1, y0), texel(x0, y1), texel(x1, y1));
    let mut out = [0_u8; 4];
    for channel in 0..4 {
        let top = a[channel] * (1.0 - wx) + b[channel] * wx;
        let bottom = c[channel] * (1.0 - wx) + d[channel] * wx;
        out[channel] = (top * (1.0 - wy) + bottom * wy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// The alpha of the zone shader (`Shader Graphs/PelletSpawnPoint`) at `d`, the distance from the zone's centre over its radius.
fn zone_alpha(d: f32, opacity: f32, coefficient: f32, inside: Option<f32>) -> f32 {
    // `Zone.UpdateMaterial`'s ring: the distance is taken from the ring's middle, over half its width.
    let d = match inside {
        Some(inner) => ((d - (1.0 + inner) * 0.5).abs()) / ((1.0 - inner) * 0.5),
        None => d,
    };
    // A hard edge 1 % of the radius wide (`saturate((1 - d) * 100)`).
    let mask = ((1.0 - d) * 100.0).clamp(0.0, 1.0);
    opacity * mask * (0.5 + coefficient * (0.5 - d * d))
}

/// Vertex shader of the zone material: macroquad's own, the quad's `uv` runs from 0 to 1 across it.
const ZONE_VERTEX_SHADER: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;

varying highp vec2 uv;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    uv = texcoord;
}"#;

/// Fragment shader of the zone material: [`zone_alpha`] at the pixel (`Shader Graphs/PelletSpawnPoint`). The pixel's place is worked out from
/// the quad's own `uv` and its rectangle, in the window's drawing units (the same ones the centre and the radius are in).
const ZONE_FRAGMENT_SHADER: &str = r#"#version 100
precision highp float;

varying highp vec2 uv;

// centre x, centre y, radius, opacity
uniform vec4 zone_geom;
// coefficient, the ring's inner radius over the outer (-1: a disc), the quad's left, the quad's top
uniform vec4 zone_shape;
// the quad's width and height
uniform vec2 zone_size;
uniform vec3 zone_rgb;

void main() {
    vec2 at = zone_shape.zw + uv * zone_size;
    float d = length(at - zone_geom.xy) / zone_geom.z;
    if (zone_shape.y >= 0.0) {
        d = abs(d - (1.0 + zone_shape.y) * 0.5) / ((1.0 - zone_shape.y) * 0.5);
    }
    float mask = clamp((1.0 - d) * 100.0, 0.0, 1.0);
    float alpha = zone_geom.w * mask * (0.5 + zone_shape.x * (0.5 - d * d));
    if (alpha <= 0.0) {
        discard;
    }
    gl_FragColor = vec4(zone_rgb, min(alpha, 1.0));
}"#;

/// GPU material for a zone's fill. The window used to work every zone out with the software rasteriser on a picture a quarter the size of
/// the window and stretch it over the window: a few milliseconds for each zone in view at 1080p, more for a bigger window (the time of
/// the CPU grew with the window's pixels, which the testers noticed as a simulation that ran slower in a bigger window), and a zone's hard
/// edge came out blurred. Now the GPU draws the quad that holds the zone, with the shader's maths, at the window's own resolution.
///
/// It holds the material of the microscope shade too (the black outside the world, [`draw_microscope_shade`]): the same kind of thing, and the same
/// reason. Zoomed out far enough to see past 1.5 x the simulation's size the window worked the shade out in software every frame, a picture a quarter of the
/// window's size made pixel by pixel and sent to the GPU as a new texture, which is why zooming out made a world slower; one full-window quad with the
/// shader's maths costs the CPU nothing.
pub struct GpuZoneMaterial {
    material: macroquad::material::Material,
    /// `None` when the shade's program was refused: the software picture is the fallback.
    shade: Option<macroquad::material::Material>,
}

/// Fragment shader of the shade material: black with the alpha of `Shader Graphs/microscopeShade`, `saturate((|p| - start) / fade)` at the world point `p` of the
/// pixel (the quad is the whole window: its `uv` times its size is the pixel's place, and the camera takes that to the world as `Camera::screen_to_world` does).
const SHADE_FRAGMENT_SHADER: &str = r#"#version 100
precision highp float;

varying highp vec2 uv;

// the camera's centre x and y, its zoom (pixels per world unit), the shade's start
uniform vec4 shade_cam;
// the window's width and height, the fade distance
uniform vec3 shade_size;

void main() {
    vec2 at = uv * shade_size.xy;
    vec2 world = vec2(shade_cam.x + (at.x - shade_size.x * 0.5) / shade_cam.z, shade_cam.y - (at.y - shade_size.y * 0.5) / shade_cam.z);
    float alpha = clamp((length(world) - shade_cam.w) / shade_size.z, 0.0, 1.0);
    if (alpha <= 0.0) {
        discard;
    }
    gl_FragColor = vec4(0.0, 0.0, 0.0, alpha);
}"#;

impl GpuZoneMaterial {
    /// Compiles the GLSL zone material. Must be called with a live macroquad context (window path only); OpenGL backends only.
    pub fn load() -> Result<Self, String> {
        use macroquad::material::{MaterialParams, load_material};
        use macroquad::miniquad::{BlendFactor, BlendState, BlendValue, Equation};
        use macroquad::miniquad::{UniformDesc, UniformType};
        use macroquad::prelude::{PipelineParams, ShaderSource};

        let pipeline_params = PipelineParams {
            color_blend: Some(BlendState::new(
                Equation::Add,
                BlendFactor::Value(BlendValue::SourceAlpha),
                BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
            )),
            ..Default::default()
        };
        let material = load_material(
            ShaderSource::Glsl {
                vertex: ZONE_VERTEX_SHADER,
                fragment: ZONE_FRAGMENT_SHADER,
            },
            MaterialParams {
                pipeline_params,
                uniforms: vec![
                    UniformDesc::new("zone_geom", UniformType::Float4),
                    UniformDesc::new("zone_shape", UniformType::Float4),
                    UniformDesc::new("zone_size", UniformType::Float2),
                    UniformDesc::new("zone_rgb", UniformType::Float3),
                ],
                ..Default::default()
            },
        )
        .map_err(|error| format!("GPU zone material failed: {error}"))?;
        let shade_pipeline = PipelineParams {
            color_blend: Some(BlendState::new(
                Equation::Add,
                BlendFactor::Value(BlendValue::SourceAlpha),
                BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
            )),
            ..Default::default()
        };
        let shade = load_material(
            ShaderSource::Glsl {
                vertex: ZONE_VERTEX_SHADER,
                fragment: SHADE_FRAGMENT_SHADER,
            },
            MaterialParams {
                pipeline_params: shade_pipeline,
                uniforms: vec![UniformDesc::new("shade_cam", UniformType::Float4), UniformDesc::new("shade_size", UniformType::Float3)],
                ..Default::default()
            },
        )
        .map_err(|error| eprintln!("warning: GPU shade material failed: {error}; the microscope shade is drawn by the software rasteriser"))
        .ok();
        Ok(Self { material, shade })
    }
}

/// The screen rectangle `(left, top, right, bottom)` of the quad that holds a zone's fill: the disc's box (a pixel more each side) cut to the
/// window. `None` when it is off the window or the zone has no size.
fn zone_quad(camera: &Camera, center: Vec2, radius: f32) -> Option<(f32, f32, f32, f32)> {
    let p = camera.world_to_screen(center);
    let r = radius * camera.zoom;
    if !r.is_finite() || r <= 0.0 || !p.is_finite() {
        return None;
    }
    let reach = r.ceil() + 1.0;
    let (x0, x1) = ((p.x - reach).floor().max(0.0), (p.x + reach).ceil().min(camera.screen.x));
    let (y0, y1) = ((p.y - reach).floor().max(0.0), (p.y + reach).ceil().min(camera.screen.y));
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Draws a zone's fill with the GPU material (see [`GpuZoneMaterial`]).
fn draw_zone_gpu(material: &GpuZoneMaterial, camera: &Camera, primitive: Primitive) {
    use macroquad::material::{gl_use_default_material, gl_use_material};
    use macroquad::prelude::{WHITE, draw_rectangle};
    let Primitive::ZoneDisc { center, radius, rgb, opacity, coefficient, inside } = primitive else {
        return;
    };
    let Some((x0, y0, x1, y1)) = zone_quad(camera, center, radius) else {
        return;
    };
    let p = camera.world_to_screen(center);
    gl_use_material(&material.material);
    material.material.set_uniform("zone_geom", [p.x, p.y, radius * camera.zoom, opacity]);
    material.material.set_uniform("zone_shape", [coefficient, inside.unwrap_or(-1.0), x0, y0]);
    material.material.set_uniform("zone_size", [x1 - x0, y1 - y0]);
    material.material.set_uniform(
        "zone_rgb",
        [f32::from(rgb[0]) / 255.0, f32::from(rgb[1]) / 255.0, f32::from(rgb[2]) / 255.0],
    );
    draw_rectangle(x0, y0, x1 - x0, y1 - y0, WHITE);
    gl_use_default_material();
}

/// Draws a zone's fill in the window without the GPU material: the program runs at a quarter of the resolution (software) and the picture is stretched.
fn draw_zone_disc(camera: &Camera, primitive: Primitive) {
    use macroquad::prelude::{DrawTextureParams, FilterMode, Texture2D, WHITE, draw_texture_ex};
    let Primitive::ZoneDisc { center, radius, .. } = primitive else {
        return;
    };
    // Off the screen altogether.
    let corner = Vec2::new(camera.screen.x * 0.5, camera.screen.y * 0.5) / camera.zoom;
    if camera.center.distance(center) > radius + corner.length() {
        return;
    }
    let (w, h) = ((camera.screen.x / 4.0).ceil().max(1.0), (camera.screen.y / 4.0).ceil().max(1.0));
    let mini = Camera { center: camera.center, zoom: camera.zoom / 4.0, screen: Vec2::new(w, h) };
    let mut scene = Scene::new();
    scene.background = [0, 0, 0, 0];
    scene.push(primitive);
    let image = render_scene(&scene, &mini, w as u32, h as u32, 1, &[]);
    let texture = Texture2D::from_rgba8(w as u16, h as u16, &image.rgba);
    texture.set_filter(FilterMode::Linear);
    draw_texture_ex(&texture, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(macroquad::prelude::vec2(camera.screen.x, camera.screen.y)), ..Default::default() });
}

/// Draws the background cells in the window: the program runs at a quarter of the resolution (software) and the picture is stretched.
fn draw_background_cells(camera: &Camera, time: f32, min: Vec2, max: Vec2, color: Rgba, cell: Rgba) {
    use macroquad::prelude::{FilterMode, Texture2D, WHITE, draw_texture_ex, DrawTextureParams};
    let (w, h) = ((camera.screen.x / 4.0).ceil().max(1.0), (camera.screen.y / 4.0).ceil().max(1.0));
    let mini = Camera { center: camera.center, zoom: camera.zoom / 4.0, screen: Vec2::new(w, h) };
    let mut scene = Scene::new();
    scene.background = [0, 0, 0, 0];
    scene.skin.time = time;
    scene.push(Primitive::BackgroundCells { min, max, color, cell });
    let image = render_scene(&scene, &mini, w as u32, h as u32, 1, &[]);
    let texture = Texture2D::from_rgba8(w as u16, h as u16, &image.rgba);
    texture.set_filter(FilterMode::Linear);
    draw_texture_ex(&texture, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(macroquad::prelude::vec2(camera.screen.x, camera.screen.y)), ..Default::default() });
}

/// A copy of a sprite's cell of its atlas with the skin shader's maths applied, for [`Painting::Recoloured`]: `(width, height, RGBA8 texels with straight alpha)`; `None`
/// when the cell is not inside the atlas. The colours are `fill_sprite`'s: a reaper takes its colour times `1 - killer` and loses `0.75 killer` of its alpha, a hurt
/// bibite is mixed toward (1, 0.1011, 0) by `0.75 hurt` after the recolour, and the byte is the rounded, clamped channel.
fn recoloured_cell(atlas: &Atlas, rect: [u32; 4], tint: [u8; 3], killer: f32, hurt: f32, skin: SkinParams) -> Option<(u32, u32, Vec<u8>)> {
    let [x0, y0, width, height] = rect;
    if width == 0 || height == 0 || x0.checked_add(width)? > atlas.width || y0.checked_add(height)? > atlas.height {
        return None;
    }
    let (killer, hurt) = (killer.clamp(0.0, 1.0), hurt.clamp(0.0, 1.0));
    let skin_tint = SkinTint::new(tint.map(|channel| f32::from(channel) / 255.0 * (1.0 - killer)), skin);
    let flash = 0.75 * hurt;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            let offset = ((y * atlas.width + x) * 4) as usize;
            let pixel = atlas.rgba.get(offset..offset + 4)?;
            let alpha = if killer > 0.0 { (f32::from(pixel[3]) - 191.25 * killer).max(0.0).round() as u8 } else { pixel[3] };
            if alpha == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let rgb = skin_tint.apply([pixel[0], pixel[1], pixel[2]].map(|channel| f32::from(channel) / 255.0));
            let rgb = [rgb[0] + (1.0 - rgb[0]) * flash, rgb[1] + (0.1011 - rgb[1]) * flash, rgb[2] + (0.0 - rgb[2]) * flash];
            rgba.extend(rgb.map(|channel| (channel * 255.0).round().clamp(0.0, 255.0) as u8));
            rgba.push(alpha);
        }
    }
    Some((width, height, rgba))
}

/// What a recoloured copy of a cell depends on: the cell, the tint, the reaper's amount (a byte) and the hurt flash (in eighths).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RecolourKey {
    atlas: u16,
    rect: [u32; 4],
    tint: [u8; 3],
    killer: u8,
    hurt: u8,
}

/// How many recoloured cells [`RecolourCache`] keeps before it starts over (a bibite with a colour of its own is a birth or a mutation; a cell is a few kilobytes).
const RECOLOUR_CACHE_LIMIT: usize = 8192;

/// The recoloured copies [`draw_recoloured_sprite`] has made, kept until the skin shader's globals change or there are [`RECOLOUR_CACHE_LIMIT`] of them.
struct RecolourCache<T> {
    skin: [f32; 3],
    cells: std::collections::HashMap<RecolourKey, T>,
}

impl<T> RecolourCache<T> {
    fn new() -> Self {
        Self { skin: [f32::NAN; 3], cells: std::collections::HashMap::new() }
    }

    /// The copy for `key` (made by `make` when there is none yet, `None` when `make` has none to give) for the shader globals `skin`.
    fn get_or_make(&mut self, skin: [f32; 3], key: RecolourKey, make: impl FnOnce() -> Option<T>) -> Option<&T> {
        if self.skin != skin || self.cells.len() >= RECOLOUR_CACHE_LIMIT {
            self.cells.clear();
            self.skin = skin;
        }
        if !self.cells.contains_key(&key) {
            let cell = make()?;
            self.cells.insert(key, cell);
        }
        self.cells.get(&key)
    }
}

/// Draws a skin, eye or egg sprite in the window without the hue material (see [`Painting::Recoloured`]): from the recoloured copy of its cell, made once per cell and
/// tint and kept, with the default material. The place, the turn, the flips and the inset of the sampled rectangle are the default draw's.
#[allow(clippy::too_many_arguments)]
fn draw_recoloured_sprite(
    camera: &Camera,
    skin: SkinParams,
    atlases: &[Atlas],
    sprite: crate::assets::SpriteRef,
    anchor: Vec2,
    size: Vec2,
    pivot: Vec2,
    rotation: f32,
    tint: Rgba,
    flip_x: bool,
    flip_y: bool,
    effects: crate::skin::SkinEffects,
) {
    use macroquad::prelude::{Color, DrawTextureParams, FilterMode, Rect, Texture2D, draw_texture_ex, vec2};
    thread_local! {
        // Never dropped: a macroquad texture must not be deleted after the window's context is gone.
        static CELLS: std::mem::ManuallyDrop<std::cell::RefCell<RecolourCache<Texture2D>>> = std::mem::ManuallyDrop::new(std::cell::RefCell::new(RecolourCache::new()));
    }
    let Some(atlas) = atlases.get(usize::from(sprite.atlas)) else {
        return;
    };
    let key = RecolourKey {
        atlas: sprite.atlas,
        rect: sprite.rect,
        tint: [tint[0], tint[1], tint[2]],
        killer: (effects.killer.clamp(0.0, 1.0) * 255.0).round() as u8,
        hurt: (effects.hurt.clamp(0.0, 1.0) * 8.0).round() as u8,
    };
    let anchor_screen = camera.world_to_screen(anchor);
    let destination = vec2((size.x * camera.zoom).max(1.0), (size.y * camera.zoom).max(1.0));
    CELLS.with(|cells| {
        let mut cells = cells.borrow_mut();
        let make = || {
            let (width, height, rgba) = recoloured_cell(atlas, key.rect, key.tint, f32::from(key.killer) / 255.0, f32::from(key.hurt) / 8.0, skin)?;
            let texture = Texture2D::from_rgba8(u16::try_from(width).ok()?, u16::try_from(height).ok()?, &rgba);
            texture.set_filter(FilterMode::Nearest);
            Some(texture)
        };
        let Some(texture) = cells.get_or_make([skin.from_hue, skin.vmin_at_s0, skin.vmax_at_s0], key, make) else {
            return;
        };
        // The copy is the cell and nothing else: its own rectangle, a little inset (see `SPRITE_INSET_PIXELS`).
        let source = inset_sprite_source([0, 0, sprite.rect[2], sprite.rect[3]], [destination.x, destination.y]);
        let params = DrawTextureParams {
            dest_size: Some(destination),
            source: Some(Rect::new(source[0], source[1], source[2], source[3])),
            // World y is up, screen y is down: negate the angle.
            rotation: -rotation,
            flip_x,
            flip_y,
            pivot: Some(vec2(anchor_screen.x, anchor_screen.y)),
        };
        draw_texture_ex(
            texture,
            anchor_screen.x - pivot.x * destination.x,
            anchor_screen.y - pivot.y * destination.y,
            Color::new(1.0, 1.0, 1.0, f32::from(tint[3]) / 255.0),
            params,
        );
    });
}

/// Draws a sprite with a pattern (virus, rainbow, producer) in the window: the software rasteriser runs the game's pixel program on a picture of just
/// that sprite (its screen box), and the picture is drawn as it is.
#[allow(clippy::too_many_arguments)]
fn draw_program_sprite(
    camera: &Camera,
    scene_skin: SkinParams,
    atlases: &[Atlas],
    sprite: crate::assets::SpriteRef,
    anchor: Vec2,
    size: Vec2,
    pivot: Vec2,
    rotation: f32,
    tint: Rgba,
    flip_x: bool,
    flip_y: bool,
    effects: crate::skin::SkinEffects,
    warp: Option<TailWarp>,
) {
    use macroquad::prelude::{FilterMode, Texture2D, WHITE, draw_texture};
    let anchor_screen = camera.world_to_screen(anchor);
    let size_screen = Vec2::new(size.x * camera.zoom, size.y * camera.zoom);
    if size_screen.x <= 0.0 || size_screen.y <= 0.0 {
        return;
    }
    let (sin, cos) = (-rotation).sin_cos();
    let warp_margin = if warp.is_some() { 0.3 } else { 0.0 };
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for (u, v) in [(0.0, -warp_margin), (1.0, -warp_margin), (0.0, 1.0 + warp_margin), (1.0, 1.0 + warp_margin)] {
        let local_x = (u - pivot.x) * size_screen.x;
        let local_y = (v - pivot.y) * size_screen.y;
        let dx = cos * local_x - sin * local_y;
        let dy = sin * local_x + cos * local_y;
        min_x = min_x.min(dx);
        max_x = max_x.max(dx);
        min_y = min_y.min(dy);
        max_y = max_y.max(dy);
    }
    let x0 = (anchor_screen.x + min_x - 1.0).floor();
    let y0 = (anchor_screen.y + min_y - 1.0).floor();
    let x1 = (anchor_screen.x + max_x + 1.0).ceil();
    let y1 = (anchor_screen.y + max_y + 1.0).ceil();
    let (w, h) = ((x1 - x0).max(1.0).min(2048.0), (y1 - y0).max(1.0).min(2048.0));
    let mini = Camera {
        center: camera.screen_to_world(Vec2::new(x0 + w * 0.5, y0 + h * 0.5)),
        zoom: camera.zoom,
        screen: Vec2::new(w, h),
    };
    let mut scene = Scene::new();
    scene.background = [0, 0, 0, 0];
    scene.skin = scene_skin;
    scene.push(Primitive::Sprite { sprite, anchor, size, pivot, rotation, tint, flip_x, flip_y, hue_shift: true, effects, warp });
    let image = render_scene(&scene, &mini, w as u32, h as u32, 1, atlases);
    let texture = Texture2D::from_rgba8(w as u16, h as u16, &image.rgba);
    texture.set_filter(FilterMode::Nearest);
    draw_texture(&texture, x0, y0, WHITE);
}

/// Draws the scene with macroquad. Call once per frame between
/// `clear_background`-equivalent setup and HUD drawing. When
/// `hue_material` is provided, `hue_shift` skin/eye/egg sprites use the GPU
/// hue-match material (the tint in the vertex colour, like the plain multiply);
/// everything else uses the default material. Without the material those
/// sprites are drawn from recoloured copies of their cells ([`Painting::Recoloured`]) when the software atlases are there (else they are multiplied with their
/// tint, which turns the blue art blue).
pub fn draw_scene(
    scene: &Scene,
    camera: &Camera,
    textures: &[macroquad::prelude::Texture2D],
    hue_material: Option<&GpuHueMaterial>,
    zone_material: Option<&GpuZoneMaterial>,
    atlases: &[Atlas],
) {
    use macroquad::material::{gl_use_default_material, gl_use_material};
    use macroquad::prelude::*;

    clear_background(to_macroquad(scene.background));
    // Whether the hue material is bound. It is switched only where a run of hue-shifted sprites starts or ends, so macroquad
    // still batches a run into one draw call per texture.
    let mut hue_bound = false;
    let mut last_effects = [-1.0f32; 2];
    for primitive in &scene.primitives {
        let spun;
        let primitive = if let Primitive::Spinner { .. } = primitive {
            spun = spinner_sprite(*primitive);
            &spun
        } else {
            primitive
        };
        let painting = painting_of(primitive, camera, hue_material.is_some(), !atlases.is_empty());
        if let Some(material) = hue_material {
            let wants_hue = painting == Painting::HueMaterial;
            if wants_hue != hue_bound {
                if wants_hue {
                    gl_use_material(&material.material);
                    material.set_params(scene.skin);
                } else {
                    gl_use_default_material();
                }
                hue_bound = wants_hue;
            }
        }
        match *primitive {
            // Turned into the sprite it is (above) before it gets here.
            Primitive::Spinner { .. } => {}
            // Squares a pixel across at the least, on the pixel grid (the software rasteriser's `draw_dots` does the same).
            Primitive::Dots { first, count } => {
                for dot in scene.dots_of(first, count) {
                    let centre = camera.world_to_screen(dot.center);
                    let side = (dot.half * 2.0 * camera.zoom).round().max(1.0);
                    if centre.x.is_finite() && centre.y.is_finite() {
                        draw_rectangle((centre.x - side * 0.5).round(), (centre.y - side * 0.5).round(), side, side, to_macroquad(dot.color));
                    }
                }
            }
            Primitive::Rect { min, max, color } => {
                let (x, y, w, h) = rect_screen(camera, min, max);
                draw_rectangle(x, y, w, h, to_macroquad(color));
            }
            Primitive::MicroscopeShade { start, fade } => {
                draw_microscope_shade(camera, start, fade, zone_material);
            }
            Primitive::GridLines { .. } => {
                for (a, b, thickness, color) in grid_strokes(camera, *primitive) {
                    let (sa, sb) = (camera.world_to_screen(a), camera.world_to_screen(b));
                    draw_line(sa.x, sa.y, sb.x, sb.y, screen_thickness(camera, thickness), to_macroquad(color));
                }
            }
            Primitive::ZoneDisc { .. } => {
                if let Some(material) = zone_material {
                    draw_zone_gpu(material, camera, *primitive);
                } else {
                    draw_zone_disc(camera, *primitive);
                }
            }
            Primitive::BackgroundCells { color, cell, min, max } => {
                draw_background_cells(camera, scene.skin.time, min, max, color, cell);
            }
            Primitive::RadioField { center, radius } => {
                let view = camera.visible_world(0.0);
                radio_cells(center, radius, effect_time(), view, |a, b, paint| {
                    let (pa, pb) = (camera.world_to_screen(a), camera.world_to_screen(b));
                    let (x0, x1) = (pa.x.min(pb.x).round(), pa.x.max(pb.x).round());
                    let (y0, y1) = (pa.y.min(pb.y).round(), pa.y.max(pb.y).round());
                    if x1 > x0 && y1 > y0 {
                        draw_rectangle(x0, y0, x1 - x0, y1 - y0, to_macroquad(paint));
                    }
                });
            }
            Primitive::RectOutline {
                min,
                max,
                thickness,
                color,
            } => {
                let (x, y, w, h) = rect_screen(camera, min, max);
                let t = screen_thickness(camera, thickness);
                draw_rectangle_lines(x, y, w, h, t, to_macroquad(color));
            }
            Primitive::Circle {
                center,
                radius,
                color,
            } => {
                let p = camera.world_to_screen(center);
                let r = screen_radius(camera, radius, MIN_PELLET_PIXELS);
                draw_circle(p.x, p.y, r, to_macroquad(color));
            }
            Primitive::CircleOutline {
                center,
                radius,
                thickness,
                color,
            } => {
                let p = camera.world_to_screen(center);
                let r = screen_radius(camera, radius, MIN_PELLET_PIXELS);
                draw_circle_lines(
                    p.x,
                    p.y,
                    r,
                    screen_thickness(camera, thickness),
                    to_macroquad(color),
                );
            }
            Primitive::Ring {
                center,
                outer,
                inner,
                color,
            } => {
                draw_ring(camera, center, outer, inner, to_macroquad(color));
            }
            Primitive::Sector {
                center,
                radius,
                facing,
                half_angle,
                color,
            } => {
                let c = camera.world_to_screen(center);
                let steps = ((half_angle * 2.0).to_degrees() / 4.0)
                    .ceil()
                    .clamp(8.0, 96.0) as usize;
                let paint = to_macroquad(color);
                let rim = |a: f32| {
                    let direction = facing.rotate(a);
                    camera.world_to_screen(center + direction * radius)
                };
                let mut previous = rim(-half_angle);
                for step in 1..=steps {
                    let a = -half_angle + 2.0 * half_angle * step as f32 / steps as f32;
                    let next = rim(a);
                    draw_triangle(
                        macroquad::prelude::Vec2::new(c.x, c.y),
                        macroquad::prelude::Vec2::new(previous.x, previous.y),
                        macroquad::prelude::Vec2::new(next.x, next.y),
                        paint,
                    );
                    previous = next;
                }
            }
            Primitive::Diamond {
                center,
                half_diagonal,
                color,
            } => {
                let c = camera.world_to_screen(center);
                let r = half_diagonal * camera.zoom;
                let paint = to_macroquad(color);
                let (north, east, south, west) = (
                    macroquad::prelude::Vec2::new(c.x, c.y - r),
                    macroquad::prelude::Vec2::new(c.x + r, c.y),
                    macroquad::prelude::Vec2::new(c.x, c.y + r),
                    macroquad::prelude::Vec2::new(c.x - r, c.y),
                );
                draw_triangle(north, east, south, paint);
                draw_triangle(north, south, west, paint);
            }
            Primitive::Capsule {
                a,
                b,
                radius,
                color,
            } => {
                let sa = camera.world_to_screen(a);
                let sb = camera.world_to_screen(b);
                // The game's bodies are tiny; keep them clickable/visible when the
                // whole world is fitted by enforcing a minimum screen radius.
                let r = screen_radius(camera, radius, MIN_BODY_PIXELS);
                let paint = to_macroquad(color);
                // LOD: a one-pixel body is a dot, so skip the capsule's three
                // draw calls when zoomed far out (2k bibites stay 60 fps).
                if r < 1.6 || sa.distance(sb) < 0.01 {
                    draw_circle(sa.x, sa.y, r.max(0.8), paint);
                } else {
                    draw_line(sa.x, sa.y, sb.x, sb.y, r * 2.0, paint);
                    draw_circle(sa.x, sa.y, r, paint);
                    draw_circle(sb.x, sb.y, r, paint);
                }
            }
            Primitive::Sprite {
                sprite,
                anchor,
                size,
                pivot,
                rotation,
                tint,
                flip_x,
                flip_y,
                // `hue_shift` selected the material above; the draw call is the same.
                hue_shift: _,
                effects,
                // TODO(tail): deform the macroquad sprite mesh once the window
                // tail path is validated; headless software path is authoritative.
                warp,
            } => {
                let Some(texture) = textures.get(sprite.atlas as usize) else {
                    continue;
                };
                // The special skins and a bent body seen big are drawn from the software picture (see `painting_of`), with the default material.
                if painting == Painting::Picture {
                    draw_program_sprite(camera, scene.skin, atlases, sprite, anchor, size, pivot, rotation, tint, flip_x, flip_y, effects, warp);
                    continue;
                }
                // Without the hue material a skin, eye or egg sprite comes from a recoloured copy of its cell (see `Painting::Recoloured`).
                if painting == Painting::Recoloured {
                    draw_recoloured_sprite(camera, scene.skin, atlases, sprite, anchor, size, pivot, rotation, tint, flip_x, flip_y, effects);
                    continue;
                }
                let anchor_screen = camera.world_to_screen(anchor);
                let anchor_glam = Vec2::new(anchor_screen.x, anchor_screen.y);
                let destination = Vec2::new(
                    (size.x * camera.zoom).max(1.0),
                    (size.y * camera.zoom).max(1.0),
                );
                // The cell the sprite is cut from, a little inset (`SPRITE_INSET_PIXELS`: the window's multisampled edge pixels must not reach the neighbouring cell).
                let source = inset_sprite_source(sprite.rect, [destination.x, destination.y]);
                let params = DrawTextureParams {
                    dest_size: Some(destination),
                    source: Some(Rect::new(source[0], source[1], source[2], source[3])),
                    // World y is up, screen y is down: negate the angle.
                    rotation: -rotation,
                    flip_x,
                    flip_y,
                    pivot: Some(anchor_glam),
                };
                // The tint is the vertex colour for both materials: the default one multiplies it with the
                // texel, the hue material takes its hue and saturation. The skin shader's hurt flash and reaper darkening go in a uniform.
                if let Some(material) = hue_material {
                    let pair = [effects.hurt, effects.killer];
                    if pair != last_effects {
                        material.set_effects(pair);
                        last_effects = pair;
                    }
                }
                draw_texture_ex(
                    texture,
                    anchor_glam.x - pivot.x * destination.x,
                    anchor_glam.y - pivot.y * destination.y,
                    to_macroquad(tint),
                    params,
                );
            }
            Primitive::Line {
                a,
                b,
                thickness,
                color,
            } => {
                let sa = camera.world_to_screen(a);
                let sb = camera.world_to_screen(b);
                draw_line(
                    sa.x,
                    sa.y,
                    sb.x,
                    sb.y,
                    screen_thickness(camera, thickness),
                    to_macroquad(color),
                );
            }
        }
    }
    // The HUD that follows draws with macroquad's default material.
    if hue_bound {
        gl_use_default_material();
    }
}

/// Draws a filled annulus as a triangle strip (macroquad has no filled-ring
/// primitive; drawing an outer disc then an inner disc in the background colour
/// would erase whatever is underneath, so triangles are the correct choice).
fn draw_ring(
    camera: &Camera,
    center: Vec2,
    outer: f32,
    inner: f32,
    color: macroquad::prelude::Color,
) {
    use macroquad::prelude::*;

    const SEGMENTS: usize = 96;
    let c = camera.world_to_screen(center);
    let r_outer = screen_radius(camera, outer, MIN_PELLET_PIXELS);
    let r_inner = screen_radius(camera, inner, 0.0).min(r_outer);
    for segment in 0..SEGMENTS {
        let t0 = segment as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let t1 = (segment + 1) as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let (sin0, cos0) = t0.sin_cos();
        let (sin1, cos1) = t1.sin_cos();
        let outer0 = Vec2::new(c.x + cos0 * r_outer, c.y + sin0 * r_outer);
        let outer1 = Vec2::new(c.x + cos1 * r_outer, c.y + sin1 * r_outer);
        let inner0 = Vec2::new(c.x + cos0 * r_inner, c.y + sin0 * r_inner);
        let inner1 = Vec2::new(c.x + cos1 * r_inner, c.y + sin1 * r_inner);
        draw_triangle(outer0, outer1, inner0, color);
        draw_triangle(outer1, inner1, inner0, color);
    }
}

/// Screen-space rect `(x, y, w, h)` for the world-space rectangle.
fn rect_screen(camera: &Camera, min: Vec2, max: Vec2) -> (f32, f32, f32, f32) {
    let a = camera.world_to_screen(min);
    let b = camera.world_to_screen(max);
    (
        a.x.min(b.x),
        a.y.min(b.y),
        (b.x - a.x).abs().max(0.0),
        (b.y - a.y).abs().max(0.0),
    )
}

/// Screen-space radius with a minimum in logical pixels.
fn screen_radius(camera: &Camera, world_radius: f32, min_pixels: f32) -> f32 {
    (world_radius * camera.zoom).max(min_pixels)
}

/// Screen-space stroke width with a minimum of one logical pixel.
fn screen_thickness(camera: &Camera, world_thickness: f32) -> f32 {
    (world_thickness * camera.zoom).max(1.0)
}

/// Converts the viewer's 8-bit RGBA to a macroquad colour.
#[must_use]
pub fn to_macroquad(color: Rgba) -> macroquad::prelude::Color {
    macroquad::prelude::Color::new(
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    )
}

// ---------------------------------------------------------------------------
// Headless backend (software rasteriser)
// ---------------------------------------------------------------------------

/// A rendered RGBA image, ready to write to disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8 rows (top row first).
    pub rgba: Vec<u8>,
}

/// Summary of an image, used by the screenshot test to assert that a real frame
/// was drawn (rather than a blank buffer).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImageStats {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels that differ from the background colour.
    pub non_background: usize,
    /// Approximate number of distinct colours (saturating counter).
    pub distinct_colors: usize,
    /// Pixels that are "green-dominant" (plants/zones).
    pub green: usize,
    /// Pixels that are "red-dominant" (meat/corpses/damaged bodies).
    pub red: usize,
    /// Pixels brighter than the background by a wide margin (bodies, eggs,
    /// selection).
    pub bright: usize,
    /// Mean luminance in `0..=255`.
    pub mean_luma: f32,
}

impl Image {
    /// Writes the image as a PNG.
    pub fn write_png(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let file = std::fs::File::create(path)?;
        let writer = std::io::BufWriter::new(file);
        let mut encoder = png::Encoder::new(writer, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
        writer
            .write_image_data(&self.rgba)
            .map_err(std::io::Error::other)?;
        writer.finish().map_err(std::io::Error::other)?;
        Ok(())
    }

    /// The image as the bytes of a PNG (what a save's `img.png` holds).
    pub fn to_png_bytes(&self) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
        writer
            .write_image_data(&self.rgba)
            .map_err(std::io::Error::other)?;
        writer.finish().map_err(std::io::Error::other)?;
        Ok(bytes)
    }

    /// Reads one pixel (`None` when out of bounds). Test-only helper so the
    /// screenshot test can assert on specific pixels.
    #[cfg(test)]
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = ((y * self.width + x) * 4) as usize;
        self.rgba
            .get(offset..offset + 4)
            .map(|slice| [slice[0], slice[1], slice[2], slice[3]])
    }

    /// Computes summary statistics for test assertions.
    #[must_use]
    pub fn stats(&self) -> ImageStats {
        let mut stats = ImageStats {
            width: self.width,
            height: self.height,
            ..ImageStats::default()
        };
        let mut seen = std::collections::HashSet::new();
        let mut luma_sum = 0.0_f64;
        let total = (self.width * self.height) as usize;
        for pixel in self.rgba.chunks_exact(4) {
            let [r, g, b, _a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
            let luma = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
            luma_sum += luma;
            if distinct_key(r, g, b) != distinct_key(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2]) {
                stats.non_background += 1;
            }
            if seen.len() < 4096 {
                seen.insert(distinct_key(r, g, b));
            }
            if g > 60 && u32::from(g) > u32::from(r) + 15 && u32::from(g) > u32::from(b) + 15 {
                stats.green += 1;
            }
            if r > 70 && u32::from(r) > u32::from(g) + 25 && u32::from(r) > u32::from(b) + 25 {
                stats.red += 1;
            }
            if luma > 70.0 {
                stats.bright += 1;
            }
        }
        stats.distinct_colors = seen.len();
        stats.mean_luma = if total == 0 {
            0.0
        } else {
            (luma_sum / total as f64) as f32
        };
        stats
    }
}

fn distinct_key(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

/// Rasterises a scene into an image in pure software.
///
/// `supersample` is an integer scale factor; the scene is rendered at
/// `supersample ×` resolution and box-filtered down, which gives cheap
/// anti-aliasing without any GPU/display dependency. `supersample = 1` disables
/// it.
#[must_use]
pub fn render_scene(
    scene: &Scene,
    camera: &Camera,
    width: u32,
    height: u32,
    supersample: u32,
    atlases: &[Atlas],
) -> Image {
    let ss = supersample.max(1);
    let mut raster = Raster::new(width * ss, height * ss, ss as f32, atlases);
    raster.skin = scene.skin;
    raster.fill(scene.background);
    let internal_camera = Camera {
        center: camera.center,
        zoom: camera.zoom * ss as f32,
        screen: Vec2::new((width * ss) as f32, (height * ss) as f32),
    };
    for primitive in &scene.primitives {
        match *primitive {
            Primitive::Dots { first, count } => raster.draw_dots(&internal_camera, scene.dots_of(first, count)),
            _ => raster.draw(&internal_camera, *primitive),
        }
    }
    raster.finish(width, height, ss)
}

/// A UI overlay to composite after the world (themed HUD).
pub struct UiOverlay<'a> {
    /// Loaded UI atlas records/textures.
    pub assets: &'a crate::ui::UiAssets,
    /// Screen-space draw list.
    pub frame: &'a crate::ui::UiFrame,
    /// TTF rasteriser cache.
    pub fonts: &'a mut crate::ui::FontSet,
}

/// Renders the world and composites the themed UI in the same supersampled
/// pass (so text and 9-slice frames are anti-aliased like the window).
#[must_use]
pub fn render_scene_with_ui(
    scene: &Scene,
    camera: &Camera,
    width: u32,
    height: u32,
    supersample: u32,
    atlases: &[Atlas],
    overlay: Option<UiOverlay<'_>>,
) -> Image {
    let ss = supersample.max(1);
    let mut raster = Raster::new(width * ss, height * ss, ss as f32, atlases);
    raster.skin = scene.skin;
    raster.fill(scene.background);
    let internal_camera = Camera {
        center: camera.center,
        zoom: camera.zoom * ss as f32,
        screen: Vec2::new((width * ss) as f32, (height * ss) as f32),
    };
    for primitive in &scene.primitives {
        match *primitive {
            Primitive::Dots { first, count } => raster.draw_dots(&internal_camera, scene.dots_of(first, count)),
            _ => raster.draw(&internal_camera, *primitive),
        }
    }
    if let Some(overlay) = overlay {
        raster.draw_ui(overlay.frame, overlay.assets, overlay.fonts);
    }
    raster.finish(width, height, ss)
}

/// Renders only the UI draw list into a transparent RGBA image.
///
/// Used for HUD-only visual A/B against the game's alpha HUD overlays
/// (`assets/game/reference/default_save/hud_overlay_*.png`); keeps the alpha
/// channel so the world background cannot swamp the comparison. The software
/// text path needs supersample 1 for crisp 1:1 pixels.
#[must_use]
pub fn render_ui_only(
    width: u32,
    height: u32,
    supersample: u32,
    overlay: Option<UiOverlay<'_>>,
) -> Image {
    let ss = supersample.max(1);
    let mut raster = Raster::new(width * ss, height * ss, ss as f32, &[]);
    raster.clear_transparent();
    if let Some(overlay) = overlay {
        raster.draw_ui(overlay.frame, overlay.assets, overlay.fonts);
    }
    raster.finish(width, height, ss)
}

/// Offscreen RGBA framebuffer.
struct Raster<'a> {
    width: u32,
    height: u32,
    /// Supersample factor (for minimum stroke widths).
    scale: f32,
    pixels: Vec<Rgba>,
    /// CPU copies of the game texture atlases (sprite mode).
    atlases: &'a [Atlas],
    /// The skin shader's globals for the `hue_shift` sprites (the scene's).
    skin: SkinParams,
    /// The UI clip rectangle in pixels `[x0, y0, x1, y1]` (see `UiDraw::Clip`).
    clip: Option<[i64; 4]>,
}

impl<'a> Raster<'a> {
    fn new(width: u32, height: u32, scale: f32, atlases: &'a [Atlas]) -> Self {
        Self {
            width,
            height,
            scale,
            pixels: vec![[0, 0, 0, 255]; (width * height) as usize],
            atlases,
            skin: SkinParams::default(),
            clip: None,
        }
    }

    fn fill(&mut self, color: Rgba) {
        self.pixels.fill(color);
    }

    fn clear_transparent(&mut self) {
        self.pixels.fill([0, 0, 0, 0]);
    }

    /// Straight-alpha blend of `src` over the destination pixel.
    ///
    /// The world path fills an opaque background first, so this reduces to the
    /// old opaque formula there; the UI-only path starts transparent and keeps
    /// a real alpha channel.
    fn blend(&mut self, x: i64, y: i64, src: Rgba) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        if let Some([x0, y0, x1, y1]) = self.clip
            && (x < x0 || x >= x1 || y < y0 || y >= y1)
        {
            return;
        }
        let index = (y as u32 * self.width + x as u32) as usize;
        blend_over(&mut self.pixels[index], src);
    }

    /// The pixels of `x0..x1` x `y0..y1` that a draw can change: the ones inside the image and the clip (what [`Self::blend`] checks for each).
    fn visible(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> (i64, i64, i64, i64) {
        let (mut xa, mut ya) = (x0.max(0), y0.max(0));
        let (mut xb, mut yb) = (x1.min(i64::from(self.width)), y1.min(i64::from(self.height)));
        if let Some([cx0, cy0, cx1, cy1]) = self.clip {
            xa = xa.max(cx0);
            ya = ya.max(cy0);
            xb = xb.min(cx1);
            yb = yb.min(cy1);
        }
        (xa, ya, xb, yb)
    }

    /// One colour over the visible part of `x0..x1` x `y0..y1`: an opaque colour is every pixel's result whatever was under it, so its rows are
    /// written at once; a translucent one is blended pixel by pixel.
    fn fill_visible(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, color: Rgba) {
        let (xa, ya, xb, yb) = self.visible(x0, y0, x1, y1);
        if xa >= xb || ya >= yb || color[3] == 0 {
            return;
        }
        let width = self.width as usize;
        for y in ya..yb {
            let row = y as usize * width;
            let span = &mut self.pixels[row + xa as usize..row + xb as usize];
            if color[3] == 255 {
                span.fill(color);
            } else {
                for pixel in span {
                    blend_over(pixel, color);
                }
            }
        }
    }

    /// The texels `texels[(v * width + u) * 4..]` for the columns `us` and the rows `vs` of the visible rectangle whose top left pixel is
    /// `(xa, ya)`, tinted and blended: what every nearest-neighbour UI draw does for each pixel, with the sampling worked out once per column
    /// and per row, the clip applied once, and a region of one colour (the middle of a panel's nine-slice) drawn as the flat fill it is.
    #[allow(clippy::too_many_arguments)] // raster inputs stay explicit
    fn blit_texels(&mut self, texels: &[u8], width: u32, us: &[u32], vs: &[u32], xa: i64, ya: i64, tint: Rgba) {
        if let Some(pixel) = uniform_texel(texels, width, us, vs) {
            let color = tinted(pixel, tint);
            self.fill_visible(xa, ya, xa + us.len() as i64, ya + vs.len() as i64, color);
            return;
        }
        let stride = self.width as usize;
        for (row, &v) in vs.iter().enumerate() {
            let base = (ya as usize + row) * stride + xa as usize;
            for (col, &u) in us.iter().enumerate() {
                let offset = ((v * width + u) * 4) as usize;
                let Some(pixel) = texels.get(offset..offset + 4) else {
                    continue;
                };
                let color = tinted([pixel[0], pixel[1], pixel[2], pixel[3]], tint);
                if color[3] != 0 {
                    blend_over(&mut self.pixels[base + col], color);
                }
            }
        }
    }

    /// A run of dots: squares of their colour, a pixel of the output across at the least (the window's draw is the same, see [`draw_scene`]).
    fn draw_dots(&mut self, camera: &Camera, dots: &[Dot]) {
        // On the grid of the OUTPUT's pixels, as the window draws them: the side is a whole number of them (one at the least) and the square starts at the pixel
        // its middle is in, whatever the supersampling.
        let scale = self.scale.max(1.0);
        for dot in dots {
            let centre = camera.world_to_screen(dot.center);
            let side = (2.0 * dot.half * camera.zoom / scale).round().max(1.0);
            if !(centre.x.is_finite() && centre.y.is_finite() && side.is_finite()) {
                continue;
            }
            let (x0, y0) = ((centre.x / scale - side * 0.5).round(), (centre.y / scale - side * 0.5).round());
            self.fill_rect_px((x0 * scale) as i64, (y0 * scale) as i64, ((x0 + side) * scale) as i64, ((y0 + side) * scale) as i64, dot.color);
        }
    }

    fn draw(&mut self, camera: &Camera, primitive: Primitive) {
        let primitive = spinner_sprite(primitive);
        match primitive {
            // A run of dots needs the scene's list: `render_scene` hands it to `draw_dots`.
            Primitive::Dots { .. } | Primitive::Spinner { .. } => {}
            Primitive::Rect { min, max, color } => {
                self.fill_rect(camera, min, max, color);
            }
            Primitive::MicroscopeShade { start, fade } => {
                let (w, h) = (self.width as i64, self.height as i64);
                for y in 0..h {
                    for x in 0..w {
                        let p = camera.screen_to_world(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                        let alpha = (p.length() - start) / fade;
                        if alpha > 0.0 {
                            self.blend(x, y, [0, 0, 0, (alpha.min(1.0) * 255.0).round() as u8]);
                        }
                    }
                }
            }
            Primitive::GridLines { fine_step, fine_width, fine_color, coarse_step, coarse_width, coarse_color } => {
                let (w, h) = (self.width as i64, self.height as i64);
                for y in 0..h {
                    for x in 0..w {
                        let p = camera.screen_to_world(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                        let coarse = if coarse_step > 0.0 {
                            grid_line_mask(p.x, coarse_step, coarse_width).max(grid_line_mask(p.y, coarse_step, coarse_width))
                        } else {
                            0.0
                        };
                        let fine = if fine_step > 0.0 {
                            (grid_line_mask(p.x, fine_step, fine_width).max(grid_line_mask(p.y, fine_step, fine_width)) - coarse).max(0.0)
                        } else {
                            0.0
                        };
                        for (mask, color) in [(fine, fine_color), (coarse, coarse_color)] {
                            let alpha = mask * f32::from(color[3]);
                            if alpha >= 0.5 {
                                self.blend(x, y, [color[0], color[1], color[2], alpha.round().min(255.0) as u8]);
                            }
                        }
                    }
                }
            }
            Primitive::ZoneDisc { center, radius, rgb, opacity, coefficient, inside } => {
                let p = camera.world_to_screen(center);
                let r = radius * camera.zoom;
                if !r.is_finite() || r <= 0.0 {
                    return;
                }
                let reach = r.ceil() + 1.0;
                let (x0, x1) = (((p.x - reach).floor() as i64).max(0), ((p.x + reach).ceil() as i64).min(i64::from(self.width)));
                let (y0, y1) = (((p.y - reach).floor() as i64).max(0), ((p.y + reach).ceil() as i64).min(i64::from(self.height)));
                for y in y0..y1 {
                    for x in x0..x1 {
                        let (dx, dy) = (x as f32 + 0.5 - p.x, y as f32 + 0.5 - p.y);
                        let alpha = zone_alpha((dx * dx + dy * dy).sqrt() / r, opacity, coefficient, inside);
                        if alpha > 0.0 {
                            self.blend(x, y, [rgb[0], rgb[1], rgb[2], (alpha.min(1.0) * 255.0).round() as u8]);
                        }
                    }
                }
            }
            Primitive::RadioField { center, radius } => {
                let view = camera.visible_world(0.0);
                radio_cells(center, radius, effect_time(), view, |a, b, paint| {
                    // Whole pixels by rounding: neighbouring cells share their edge, so a translucent seam does not blend twice.
                    let (pa, pb) = (camera.world_to_screen(a), camera.world_to_screen(b));
                    let (x0, x1) = (pa.x.min(pb.x).round() as i64, pa.x.max(pb.x).round() as i64);
                    let (y0, y1) = (pa.y.min(pb.y).round() as i64, pa.y.max(pb.y).round() as i64);
                    self.fill_rect_px(x0, y0, x1, y1, paint);
                });
            }
            Primitive::BackgroundCells { min, max, color, cell } => {
                let (x0, y0, x1, y1) = screen_box(camera, min, max);
                let (x0, y0) = (x0.max(0), y0.max(0));
                let (x1, y1) = (x1.min(i64::from(self.width)), y1.min(i64::from(self.height)));
                // The program is slow and the cells are soft: one evaluation per block of pixels.
                let block = (self.scale.round() as i64 * 2).max(2);
                let time = self.skin.time;
                let mut y = y0;
                while y < y1 {
                    let mut x = x0;
                    while x < x1 {
                        let world = camera.screen_to_world(Vec2::new(x as f32 + block as f32 * 0.5, y as f32 + block as f32 * 0.5));
                        if let Some(out) = background_cells(world, camera, time, color, cell) {
                            let paint = [
                                (out[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                                (out[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                                (out[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                                (out[3].clamp(0.0, 1.0) * 255.0).round() as u8,
                            ];
                            self.fill_rect_px(x, y, (x + block).min(x1), (y + block).min(y1), paint);
                        }
                        x += block;
                    }
                    y += block;
                }
            }
            Primitive::RectOutline {
                min,
                max,
                thickness,
                color,
            } => {
                let t = self.stroke_px(camera, thickness).round().max(1.0) as i64;
                let (x0, y0, x1, y1) = screen_box(camera, min, max);
                self.fill_rect_px(x0, y0, x1, y0 + t, color);
                self.fill_rect_px(x0, y1 - t, x1, y1, color);
                self.fill_rect_px(x0, y0, x0 + t, y1, color);
                self.fill_rect_px(x1 - t, y0, x1, y1, color);
            }
            Primitive::Circle {
                center,
                radius,
                color,
            } => {
                let p = camera.world_to_screen(center);
                let r = self.radius_px(camera, radius, MIN_PELLET_PIXELS);
                self.fill_circle(p, r, color);
            }
            Primitive::CircleOutline {
                center,
                radius,
                thickness,
                color,
            } => {
                let p = camera.world_to_screen(center);
                let r = self.radius_px(camera, radius, MIN_PELLET_PIXELS);
                let t = self.stroke_px(camera, thickness);
                self.fill_circle_outline(p, r, t, color);
            }
            Primitive::Ring {
                center,
                outer,
                inner,
                color,
            } => {
                let p = camera.world_to_screen(center);
                let r_outer = self.radius_px(camera, outer, MIN_PELLET_PIXELS);
                let r_inner = self.radius_px(camera, inner, 0.0).min(r_outer);
                self.fill_ring(p, r_outer, r_inner, color);
            }
            Primitive::Sector {
                center,
                radius,
                facing,
                half_angle,
                color,
            } => {
                let p = camera.world_to_screen(center);
                let r = self.radius_px(camera, radius, 0.0);
                // Screen space flips y.
                self.fill_sector(
                    p,
                    r,
                    Vec2::new(facing.x, -facing.y),
                    half_angle.cos(),
                    color,
                );
            }
            Primitive::Diamond {
                center,
                half_diagonal,
                color,
            } => {
                let p = camera.world_to_screen(center);
                let r = self.radius_px(camera, half_diagonal, 0.0);
                self.fill_diamond(p, r, color);
            }
            Primitive::Capsule {
                a,
                b,
                radius,
                color,
            } => {
                let sa = camera.world_to_screen(a);
                let sb = camera.world_to_screen(b);
                let r = self.radius_px(camera, radius, MIN_BODY_PIXELS);
                // LOD: a body smaller than ~2 internal px is just a dot.
                if r < 1.6 * self.scale || sa.distance(sb) < 0.01 {
                    self.fill_circle(sa, r.max(0.8 * self.scale), color);
                } else {
                    self.fill_capsule(sa, sb, r, color);
                }
            }
            Primitive::Sprite {
                sprite,
                anchor,
                size,
                pivot,
                rotation,
                tint,
                flip_x,
                flip_y,
                hue_shift,
                effects,
                warp,
            } => {
                self.fill_sprite(
                    camera, sprite, anchor, size, pivot, rotation, tint, flip_x, flip_y, hue_shift,
                    effects, warp,
                );
            }
            Primitive::Line {
                a,
                b,
                thickness,
                color,
            } => {
                let sa = camera.world_to_screen(a);
                let sb = camera.world_to_screen(b);
                let half = (self.stroke_px(camera, thickness) * 0.5).max(0.5 * self.scale);
                self.fill_capsule(sa, sb, half, color);
            }
        }
    }

    /// Screen radius in *internal* pixels. The camera passed to the rasteriser
    /// is already scaled by the supersample factor, so only the minimum is
    /// converted from logical pixels.
    fn radius_px(&self, camera: &Camera, world_radius: f32, min_logical: f32) -> f32 {
        (world_radius * camera.zoom).max(min_logical * self.scale)
    }

    /// Stroke width in internal pixels (at least one logical pixel).
    fn stroke_px(&self, camera: &Camera, world_thickness: f32) -> f32 {
        (world_thickness * camera.zoom).max(self.scale)
    }

    /// Sprite blit with either a multiply tint or the game's hue-replacement
    /// skin shader approximation.
    #[allow(clippy::too_many_arguments)]
    fn fill_sprite(
        &mut self,
        camera: &Camera,
        sprite: crate::assets::SpriteRef,
        anchor: Vec2,
        size: Vec2,
        pivot: Vec2,
        rotation: f32,
        tint: Rgba,
        flip_x: bool,
        flip_y: bool,
        hue_shift: bool,
        effects: crate::skin::SkinEffects,
        warp: Option<TailWarp>,
    ) {
        let Some(atlas) = self.atlases.get(sprite.atlas as usize) else {
            return;
        };
        let anchor_screen = camera.world_to_screen(anchor);
        let size_screen = Vec2::new(size.x * camera.zoom, size.y * camera.zoom);
        if size_screen.x <= 0.0 || size_screen.y <= 0.0 {
            return;
        }
        let angle = -rotation;
        let (sin, cos) = angle.sin_cos();
        // The body sprite lies along u (the head at u = 1, the tail spike at u = 0): the bent tail shears it sideways (v), so only the
        // sideways margin of the drawn rectangle grows (a straight tail changes nothing).
        let warp_margin = warp.map_or(0.0, |_| 0.3);
        // The sprite's rectangle runs from -pivot to 1 - pivot of its size round the anchor (not centred on
        // it: a body hangs left of its joint), so the covered pixels are those of its four rotated corners.
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for (u, v) in [(0.0, -warp_margin), (1.0, -warp_margin), (0.0, 1.0 + warp_margin), (1.0, 1.0 + warp_margin)] {
            let local_x = (u - pivot.x) * size_screen.x;
            let local_y = (v - pivot.y) * size_screen.y;
            let dx = cos * local_x - sin * local_y;
            let dy = sin * local_x + cos * local_y;
            min_x = min_x.min(dx);
            max_x = max_x.max(dx);
            min_y = min_y.min(dy);
            max_y = max_y.max(dy);
        }
        let x0 = (anchor_screen.x + min_x - 1.0).floor() as i64;
        let x1 = (anchor_screen.x + max_x + 1.0).ceil() as i64;
        let y0 = (anchor_screen.y + min_y - 1.0).floor() as i64;
        let y1 = (anchor_screen.y + max_y + 1.0).ceil() as i64;
        let tint_alpha = u32::from(tint[3]);
        if tint_alpha == 0 {
            return;
        }
        // The skin shader's colour work that is the same for every texel of the sprite.
        // A reaper (`_IsKiller`) takes its colour times `1 - k` and loses `0.75 k` of its alpha; a hurt bibite is mixed toward (1, 0.1011, 0) by
        // `0.75 * _HurtAmount` after all the rest.
        let (hurt, killer) = (effects.hurt.clamp(0.0, 1.0), effects.killer.clamp(0.0, 1.0));
        // The virus, the rainbow and the producer pattern need the whole of the game's pixel program, run on each texel with the pixel's place in the world.
        let program = hue_shift && effects.needs_program();
        let time = self.skin.time;
        let cb0: Vec<[f32; 4]> = if program {
            (0..131).map(|slot| match slot { 19 => [time, 0.0, 0.0, 0.0], 130 => [self.skin.from_hue, self.skin.vmin_at_s0, self.skin.vmax_at_s0, time], _ => [0.0; 4] }).collect()
        } else {
            Vec::new()
        };
        let cb1 = [
            [0.0; 4],
            [effects.upp, 2.0, 0.5, 0.0],
            [f32::from(tint[0]) / 255.0, f32::from(tint[1]) / 255.0, f32::from(tint[2]) / 255.0, 1.0],
            [hurt, effects.infected, 0.0, 0.0],
            [effects.virus[0], effects.virus[1], effects.virus[2], 1.0],
            [f32::from(u8::from(effects.rainbow)), f32::from(u8::from(effects.producer)), killer, 0.0],
            [effects.root[0], effects.root[1], effects.dir[0], effects.dir[1]],
            [3.0, effects.body_width, 0.0, 0.0],
        ];
        let skin_tint = hue_shift.then(|| {
            SkinTint::new(
                [
                    f32::from(tint[0]) / 255.0 * (1.0 - killer),
                    f32::from(tint[1]) / 255.0 * (1.0 - killer),
                    f32::from(tint[2]) / 255.0 * (1.0 - killer),
                ],
                self.skin,
            )
        });
        for y in y0.max(0)..y1.min(i64::from(self.height)) {
            for x in x0.max(0)..x1.min(i64::from(self.width)) {
                let d = Vec2::new(
                    x as f32 + 0.5 - anchor_screen.x,
                    y as f32 + 0.5 - anchor_screen.y,
                );
                let local_x = cos * d.x + sin * d.y;
                let local_y = -sin * d.x + cos * d.y;
                let mut u = local_x / size_screen.x + pivot.x;
                let mut v = local_y / size_screen.y + pivot.y;
                if !(-warp_margin..1.0 + warp_margin).contains(&v) {
                    continue;
                }
                if let Some(warp) = warp {
                    // `a` runs from the head (0) to the tail's end (1); past the bend row the tail swings sideways by its angle.
                    let along = 1.0 - u;
                    if along > warp.bend_row {
                        let t = ((along - warp.bend_row) / (1.0 - warp.bend_row).max(1e-4)).clamp(0.0, 1.0);
                        let angle = warp.angle0 + (warp.angle1 - warp.angle0) * t;
                        let aspect = sprite.rect[2] as f32 / (sprite.rect[3] as f32).max(1.0);
                        v -= angle.tan() * (along - warp.bend_row) * aspect;
                    }
                }
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                if flip_x {
                    u = 1.0 - u;
                }
                if flip_y {
                    v = 1.0 - v;
                }
                let atlas_x = sprite.rect[0] as f32 + u * sprite.rect[2] as f32;
                let atlas_y = sprite.rect[1] as f32 + v * sprite.rect[3] as f32;
                let sample_x = (atlas_x.floor() as u32).min(atlas.width.saturating_sub(1));
                let sample_y = (atlas_y.floor() as u32).min(atlas.height.saturating_sub(1));
                let offset = ((sample_y * atlas.width + sample_x) * 4) as usize;
                let Some(pixel) = atlas.rgba.get(offset..offset + 4) else {
                    continue;
                };
                let texel_alpha = if killer > 0.0 { (f32::from(pixel[3]) - 191.25 * killer).max(0.0).round() as u32 } else { u32::from(pixel[3]) };
                let alpha = (texel_alpha * tint_alpha / 255) as u8;
                if alpha == 0 {
                    continue;
                }
                let color = if program {
                    // The game's pixel program: the sprite's texel, the vertex colour (white with the tint's alpha) and the pixel's world position.
                    let world = camera.screen_to_world(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                    let texel = [f32::from(pixel[0]) / 255.0, f32::from(pixel[1]) / 255.0, f32::from(pixel[2]) / 255.0, f32::from(pixel[3]) / 255.0];
                    let fetch = |_: [f32; 2]| texel;
                    let Some(out) = crate::internals_shaders::skin_pixel(
                        [u, v],
                        [1.0, 1.0, 1.0, f32::from(tint[3]) / 255.0],
                        [world.x, world.y],
                        &cb0,
                        &cb1,
                        &fetch,
                    ) else {
                        continue;
                    };
                    [
                        (out[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (out[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (out[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (out[3].clamp(0.0, 1.0) * 255.0).round() as u8,
                    ]
                } else if let Some(skin_tint) = &skin_tint {
                    let rgb = skin_tint.apply([
                        f32::from(pixel[0]) / 255.0,
                        f32::from(pixel[1]) / 255.0,
                        f32::from(pixel[2]) / 255.0,
                    ]);
                    let flash = 0.75 * hurt;
                    let rgb = [rgb[0] + (1.0 - rgb[0]) * flash, rgb[1] + (0.1011 - rgb[1]) * flash, rgb[2] + (0.0 - rgb[2]) * flash];
                    [
                        (rgb[0] * 255.0).round().clamp(0.0, 255.0) as u8,
                        (rgb[1] * 255.0).round().clamp(0.0, 255.0) as u8,
                        (rgb[2] * 255.0).round().clamp(0.0, 255.0) as u8,
                        alpha,
                    ]
                } else {
                    [
                        (u32::from(pixel[0]) * u32::from(tint[0]) / 255) as u8,
                        (u32::from(pixel[1]) * u32::from(tint[1]) / 255) as u8,
                        (u32::from(pixel[2]) * u32::from(tint[2]) / 255) as u8,
                        alpha,
                    ]
                };
                self.blend(x, y, color);
            }
        }
    }
    fn fill_rect(&mut self, camera: &Camera, min: Vec2, max: Vec2, color: Rgba) {
        let (x0, y0, x1, y1) = screen_box(camera, min, max);
        self.fill_rect_px(x0, y0, x1, y1, color);
    }

    fn fill_rect_px(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, color: Rgba) {
        self.fill_visible(x0, y0, x1, y1, color);
    }

    fn fill_circle(&mut self, center: Vec2, radius: f32, color: Rgba) {
        self.fill_circle_ring(center, 0.0, radius, color);
    }

    fn fill_circle_outline(&mut self, center: Vec2, radius: f32, thickness: f32, color: Rgba) {
        let inner = (radius - thickness).max(0.0);
        self.fill_circle_ring(center, inner, radius, color);
    }

    fn fill_circle_ring(&mut self, center: Vec2, inner: f32, outer: f32, color: Rgba) {
        if !outer.is_finite() || outer <= 0.0 {
            return;
        }
        let outer = outer.ceil();
        let inner = inner.max(0.0);
        let x0 = (center.x - outer).floor() as i64;
        let x1 = (center.x + outer).ceil() as i64;
        let y0 = (center.y - outer).floor() as i64;
        let y1 = (center.y + outer).ceil() as i64;
        let outer_sq = outer * outer;
        let inner_sq = inner * inner;
        for y in y0.max(0)..y1.min(i64::from(self.height)) {
            for x in x0.max(0)..x1.min(i64::from(self.width)) {
                let dx = x as f32 + 0.5 - center.x;
                let dy = y as f32 + 0.5 - center.y;
                let distance_sq = dx * dx + dy * dy;
                if distance_sq <= outer_sq && distance_sq >= inner_sq {
                    self.blend(x, y, color);
                }
            }
        }
    }

    /// Fills the part of the disc whose direction from `center` is within `acos(cos_half)` of
    /// `facing` (a unit vector in screen space).
    fn fill_sector(&mut self, center: Vec2, radius: f32, facing: Vec2, cos_half: f32, color: Rgba) {
        if !radius.is_finite() || radius <= 0.0 {
            return;
        }
        let outer = radius.ceil();
        let x0 = (center.x - outer).floor() as i64;
        let x1 = (center.x + outer).ceil() as i64;
        let y0 = (center.y - outer).floor() as i64;
        let y1 = (center.y + outer).ceil() as i64;
        let radius_sq = radius * radius;
        for y in y0.max(0)..y1.min(i64::from(self.height)) {
            for x in x0.max(0)..x1.min(i64::from(self.width)) {
                let dx = x as f32 + 0.5 - center.x;
                let dy = y as f32 + 0.5 - center.y;
                let distance_sq = dx * dx + dy * dy;
                if distance_sq > radius_sq {
                    continue;
                }
                let distance = distance_sq.sqrt();
                if distance > 0.0 && (dx * facing.x + dy * facing.y) / distance < cos_half {
                    continue;
                }
                self.blend(x, y, color);
            }
        }
    }

    /// A square turned 45 degrees: the pixels whose centres are within `radius` of the middle in the sum of the two distances.
    fn fill_diamond(&mut self, center: Vec2, radius: f32, color: Rgba) {
        if !radius.is_finite() || radius <= 0.0 {
            return;
        }
        let x0 = (center.x - radius).floor() as i64;
        let x1 = (center.x + radius).ceil() as i64;
        let y0 = (center.y - radius).floor() as i64;
        let y1 = (center.y + radius).ceil() as i64;
        for y in y0.max(0)..y1.min(i64::from(self.height)) {
            for x in x0.max(0)..x1.min(i64::from(self.width)) {
                let dx = (x as f32 + 0.5 - center.x).abs();
                let dy = (y as f32 + 0.5 - center.y).abs();
                if dx + dy <= radius {
                    self.blend(x, y, color);
                }
            }
        }
    }

    fn fill_ring(&mut self, center: Vec2, outer: f32, inner: f32, color: Rgba) {
        self.fill_circle_ring(center, inner, outer, color);
    }

    fn fill_capsule(&mut self, a: Vec2, b: Vec2, radius: f32, color: Rgba) {
        if !radius.is_finite() || radius <= 0.0 {
            return;
        }
        let x0 = (a.x.min(b.x) - radius).floor() as i64;
        let x1 = (a.x.max(b.x) + radius).ceil() as i64;
        let y0 = (a.y.min(b.y) - radius).floor() as i64;
        let y1 = (a.y.max(b.y) + radius).ceil() as i64;
        let radius_sq = radius * radius;
        for y in y0.max(0)..y1.min(i64::from(self.height)) {
            for x in x0.max(0)..x1.min(i64::from(self.width)) {
                let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                if distance_to_segment_sq(point, a, b) <= radius_sq {
                    self.blend(x, y, color);
                }
            }
        }
    }

    // -- themed UI overlay -------------------------------------------------

    /// Composites a [`crate::ui::UiFrame`] in internal (supersampled) pixels.
    fn draw_ui(
        &mut self,
        frame: &crate::ui::UiFrame,
        assets: &crate::ui::UiAssets,
        fonts: &mut crate::ui::FontSet,
    ) {
        let s = self.scale;
        let scale = frame.scale;
        // `RUSTIBITES_UI_PROFILE=1`: what each kind of draw cost of this picture (the menus are painted in software: a repaint is what a
        // hover or a slider step costs), printed once the picture is done.
        let profile = std::env::var_os("RUSTIBITES_UI_PROFILE").is_some();
        let mut spent = [(0_u32, std::time::Duration::ZERO); 9];
        for draw in &frame.draws {
            let started = profile.then(std::time::Instant::now);
            let kind = match draw {
                crate::ui::UiDraw::Clip(_) => 0,
                crate::ui::UiDraw::Rect { .. } => 1,
                crate::ui::UiDraw::Image { .. } => 2,
                crate::ui::UiDraw::ImageTurned { .. } => 3,
                crate::ui::UiDraw::ImageRegion { .. } => 4,
                crate::ui::UiDraw::Pixels { .. } => 5,
                crate::ui::UiDraw::PixelsRegion { .. } => 6,
                crate::ui::UiDraw::NineSlice { .. } => 7,
                crate::ui::UiDraw::Text { .. } => 8,
            };
            match draw {
                crate::ui::UiDraw::Clip(rect) => {
                    self.clip = rect.map(|r| {
                        [
                            (r[0] * s).floor() as i64,
                            (r[1] * s).floor() as i64,
                            ((r[0] + r[2]) * s).ceil() as i64,
                            ((r[1] + r[3]) * s).ceil() as i64,
                        ]
                    });
                }
                crate::ui::UiDraw::Rect { pos, size, color } => {
                    self.ui_rect(
                        Vec2::new(pos.x * s, pos.y * s),
                        Vec2::new(size.x * s, size.y * s),
                        *color,
                    );
                }
                crate::ui::UiDraw::Image {
                    texture,
                    pos,
                    size,
                    tint,
                } => {
                    self.ui_image(
                        *texture,
                        assets,
                        Vec2::new(pos.x * s, pos.y * s),
                        Vec2::new(size.x * s, size.y * s),
                        *tint,
                    );
                }
                crate::ui::UiDraw::ImageTurned {
                    texture,
                    turns,
                    pos,
                    size,
                    tint,
                } => {
                    self.ui_image_turned(
                        *texture,
                        assets,
                        Vec2::new(pos.x * s, pos.y * s),
                        Vec2::new(size.x * s, size.y * s),
                        *turns,
                        *tint,
                    );
                }
                crate::ui::UiDraw::ImageRegion {
                    texture,
                    region,
                    pos,
                    size,
                    tint,
                } => {
                    self.ui_image_region(
                        *texture,
                        assets,
                        *region,
                        Vec2::new(pos.x * s, pos.y * s),
                        Vec2::new(size.x * s, size.y * s),
                        *tint,
                    );
                }
                crate::ui::UiDraw::Pixels { texture, pos, size } => {
                    self.ui_pixels(
                        texture,
                        None,
                        Vec2::new(pos.x * s, pos.y * s),
                        Vec2::new(size.x * s, size.y * s),
                    );
                }
                crate::ui::UiDraw::PixelsRegion {
                    texture,
                    region,
                    pos,
                    size,
                } => {
                    self.ui_pixels(
                        texture,
                        Some(*region),
                        Vec2::new(pos.x * s, pos.y * s),
                        Vec2::new(size.x * s, size.y * s),
                    );
                }
                crate::ui::UiDraw::NineSlice {
                    texture,
                    border,
                    dest_scale,
                    pos,
                    size,
                    tint,
                } => {
                    self.ui_nine_slice(
                        *texture,
                        *border,
                        assets,
                        Vec2::new(pos.x * s, pos.y * s),
                        Vec2::new(size.x * s, size.y * s),
                        *tint,
                        scale * *dest_scale,
                    );
                }
                crate::ui::UiDraw::Text {
                    text,
                    pos,
                    px,
                    color,
                    role,
                    font,
                    ..
                } => {
                    self.ui_text(
                        text,
                        *role,
                        Vec2::new(pos.x * s, pos.y * s),
                        px * s,
                        *color,
                        font.as_deref(),
                        fonts,
                    );
                }
            }
            if let Some(started) = started {
                spent[kind].0 += 1;
                spent[kind].1 += started.elapsed();
            }
        }
        if profile {
            const KINDS: [&str; 9] = ["clip", "rect", "image", "image-turned", "image-region", "pixels", "pixels-region", "nine-slice", "text"];
            let line: Vec<String> = KINDS
                .iter()
                .zip(spent)
                .filter(|(_, (count, _))| *count > 0)
                .map(|(name, (count, time))| format!("{name} x{count} {:.1} ms", time.as_secs_f64() * 1000.0))
                .collect();
            eprintln!("ui draw profile ({} x {}): {}", self.width, self.height, line.join(", "));
        }
    }

    fn ui_rect(&mut self, pos: Vec2, size: Vec2, color: Rgba) {
        // A pixel is the rectangle's when its middle is inside it (the GPU's rule for the game's UI quads): a 1 px line at a fractional height
        // is one row, not the two it touches.
        let edge = |from: f32, length: f32| -> (i64, i64) {
            let (a, b) = ((from - 0.5).ceil() as i64, (from + length - 0.5).ceil() as i64);
            // A sliver thinner than a pixel that falls between two middles still shows as one.
            if b <= a && length > 0.0 { (a, a + 1) } else { (a, b) }
        };
        let (x0, x1) = edge(pos.x, size.x);
        let (y0, y1) = edge(pos.y, size.y);
        self.fill_rect_px(x0, y0, x1, y1, color);
    }

    fn ui_image(
        &mut self,
        texture: u16,
        assets: &crate::ui::UiAssets,
        pos: Vec2,
        size: Vec2,
        tint: Rgba,
    ) {
        let Some(texture) = assets.texture(texture) else {
            return;
        };
        if texture.width == 0 || texture.height == 0 || size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        let x0 = pos.x.floor() as i64;
        let y0 = pos.y.floor() as i64;
        let x1 = (pos.x + size.x).ceil() as i64;
        let y1 = (pos.y + size.y).ceil() as i64;
        let sx = texture.width as f32 / size.x;
        let sy = texture.height as f32 / size.y;
        let (xa, ya, xb, yb) = self.visible(x0, y0, x1, y1);
        if xa >= xb || ya >= yb {
            return;
        }
        let us: Vec<u32> = (xa..xb)
            .map(|x| (((x as f32 + 0.5 - pos.x) * sx).floor().max(0.0) as u32).min(texture.width - 1))
            .collect();
        let vs: Vec<u32> = (ya..yb)
            .map(|y| (((y as f32 + 0.5 - pos.y) * sy).floor().max(0.0) as u32).min(texture.height - 1))
            .collect();
        self.blit_texels(&texture.rgba, texture.width, &us, &vs, xa, ya, tint);
    }

    /// [`Self::ui_image`] turned `turns` quarter turns counter-clockwise inside the box `pos` / `size`: each pixel of the box takes the texel the
    /// turn maps it from (the image is stretched over the node's rectangle and the rectangle turned, which is the same as turning the unit square).
    fn ui_image_turned(
        &mut self,
        texture: u16,
        assets: &crate::ui::UiAssets,
        pos: Vec2,
        size: Vec2,
        turns: u8,
        tint: Rgba,
    ) {
        let Some(texture) = assets.texture(texture) else {
            return;
        };
        if texture.width == 0 || texture.height == 0 || size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        let x0 = pos.x.floor() as i64;
        let y0 = pos.y.floor() as i64;
        let x1 = (pos.x + size.x).ceil() as i64;
        let y1 = (pos.y + size.y).ceil() as i64;
        for y in y0.max(0)..y1.min(i64::from(self.height)) {
            for x in x0.max(0)..x1.min(i64::from(self.width)) {
                // Position in the box, 0..1 each way.
                let tx = ((x as f32 + 0.5 - pos.x) / size.x).clamp(0.0, 0.999);
                let ty = ((y as f32 + 0.5 - pos.y) / size.y).clamp(0.0, 0.999);
                // The source point that lands there (y down; a quarter turn counter-clockwise takes the source's top left to the bottom left).
                let (su, sv) = match turns % 4 {
                    1 => (1.0 - ty, tx),
                    2 => (1.0 - tx, 1.0 - ty),
                    3 => (ty, 1.0 - tx),
                    _ => (tx, ty),
                };
                let u = ((su * texture.width as f32) as u32).min(texture.width - 1);
                let v = ((sv * texture.height as f32) as u32).min(texture.height - 1);
                let offset = ((v * texture.width + u) * 4) as usize;
                let Some(pixel) = texture.rgba.get(offset..offset + 4) else {
                    continue;
                };
                let alpha = (u32::from(pixel[3]) * u32::from(tint[3]) / 255) as u8;
                if alpha == 0 {
                    continue;
                }
                let color = [
                    (u32::from(pixel[0]) * u32::from(tint[0]) / 255) as u8,
                    (u32::from(pixel[1]) * u32::from(tint[1]) / 255) as u8,
                    (u32::from(pixel[2]) * u32::from(tint[2]) / 255) as u8,
                    alpha,
                ];
                self.blend(x, y, color);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ui_nine_slice(
        &mut self,
        texture: u16,
        border: [f32; 4],
        assets: &crate::ui::UiAssets,
        pos: Vec2,
        size: Vec2,
        tint: Rgba,
        ui_scale: f32,
    ) {
        let Some(source) = assets.texture(texture) else {
            return;
        };
        if size.x <= 0.0 || size.y <= 0.0 {
            return;
        }
        // Unity border order: [left, bottom, right, top].
        let [left, bottom, right, top] = border;
        // `Image.AdjustBorders`: when the rect is smaller than the two borders
        // on an axis, both borders shrink proportionally.
        let (bl, br) = crate::ui::shrink_borders(
            left * ui_scale * self.scale,
            right * ui_scale * self.scale,
            size.x,
        );
        let (bt, bb) = crate::ui::shrink_borders(
            top * ui_scale * self.scale,
            bottom * ui_scale * self.scale,
            size.y,
        );
        let dst_l = pos.x;
        let dst_r = pos.x + size.x;
        let dst_t = pos.y;
        let dst_b = pos.y + size.y;
        let src_l = 0.0;
        let src_r = source.width as f32;
        let src_t = 0.0;
        let src_b = source.height as f32;
        let mut xs = [dst_l, dst_l + bl, dst_r - br, dst_r];
        let mut ys = [dst_t, dst_t + bt, dst_b - bb, dst_b];
        // The inner edges sit on whole pixels, so that two slices share no pixel (a translucent tint would blend it twice, drawing a seam).
        for edges in [&mut xs, &mut ys] {
            edges[1] = edges[1].round().clamp(edges[0], edges[3]);
            edges[2] = edges[2].round().clamp(edges[1], edges[3]);
        }
        let sx = [src_l, src_l + left, src_r - right, src_r];
        let sy = [src_t, src_t + top, src_b - bottom, src_b];
        for row in 0..3 {
            for col in 0..3 {
                let w = xs[col + 1] - xs[col];
                let h = ys[row + 1] - ys[row];
                if w <= 0.0 || h <= 0.0 {
                    continue;
                }
                self.ui_image_region_filtered(
                    texture,
                    assets,
                    [
                        sx[col],
                        sy[row],
                        (sx[col + 1] - sx[col]).max(1.0),
                        (sy[row + 1] - sy[row]).max(1.0),
                    ],
                    Vec2::new(xs[col], ys[row]),
                    Vec2::new(w, h),
                    tint,
                    ui_scale * self.scale < 0.999,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ui_image_region(
        &mut self,
        texture: u16,
        assets: &crate::ui::UiAssets,
        region: [f32; 4],
        pos: Vec2,
        size: Vec2,
        tint: Rgba,
    ) {
        self.ui_image_region_filtered(texture, assets, region, pos, size, tint, false);
    }

    /// [`Self::ui_image_region`], sampling bilinearly when `smooth` (what the GPU does with a bilinear texture drawn smaller than its texels: the
    /// `UISprite` frames of the dropdowns are drawn at half size, their 1-texel dark outline comes out half dark).
    #[allow(clippy::too_many_arguments)]
    fn ui_image_region_filtered(
        &mut self,
        texture: u16,
        assets: &crate::ui::UiAssets,
        region: [f32; 4],
        pos: Vec2,
        size: Vec2,
        tint: Rgba,
        smooth: bool,
    ) {
        let Some(source) = assets.texture(texture) else {
            return;
        };
        if size.x <= 0.0 || size.y <= 0.0 || region[2] <= 0.0 || region[3] <= 0.0 {
            return;
        }
        let x0 = pos.x.floor() as i64;
        let y0 = pos.y.floor() as i64;
        let x1 = (pos.x + size.x).ceil() as i64;
        let y1 = (pos.y + size.y).ceil() as i64;
        let (xa, ya, xb, yb) = self.visible(x0, y0, x1, y1);
        if xa >= xb || ya >= yb {
            return;
        }
        // The texture point of each column and of each row.
        let us: Vec<f32> = (xa..xb)
            .map(|x| region[0] + ((x as f32 + 0.5 - pos.x) / size.x).clamp(0.0, 0.999) * region[2])
            .collect();
        let vs: Vec<f32> = (ya..yb)
            .map(|y| region[1] + ((y as f32 + 0.5 - pos.y) / size.y).clamp(0.0, 0.999) * region[3])
            .collect();
        if !smooth {
            let columns: Vec<u32> = us.iter().map(|u| (*u as u32).min(source.width - 1)).collect();
            let rows: Vec<u32> = vs.iter().map(|v| (*v as u32).min(source.height - 1)).collect();
            self.blit_texels(&source.rgba, source.width, &columns, &rows, xa, ya, tint);
            return;
        }
        self.blit_filtered(source, region, &us, &vs, xa, ya, tint);
    }

    /// [`Self::blit_texels`] with the texture sampled bilinearly ([`sample_bilinear`]) at the texture points `us` (per column) and `vs` (per row).
    /// Where a pixel's four texels all lie in a block of one colour (the middle of a panel's nine-slice, drawn below its native size), the filter
    /// gives that colour whatever the weights (they add up to 1 and the result is rounded), so only the pixels near the block's edge are filtered.
    #[allow(clippy::too_many_arguments)] // raster inputs stay explicit
    fn blit_filtered(&mut self, source: &crate::ui::UiTexture, region: [f32; 4], us: &[f32], vs: &[f32], xa: i64, ya: i64, tint: Rgba) {
        let (width, height) = (source.width.max(1), source.height.max(1));
        // The texels the region covers, and their colour when they are all one.
        let covered = |from: f32, length: f32, extent: u32| -> (u32, u32) {
            let first = (from.floor().max(0.0) as u32).min(extent - 1);
            let last = ((from + length).ceil().max(1.0) as u32 - 1).min(extent - 1);
            (first, last.max(first))
        };
        let (u0, u1) = covered(region[0], region[2], width);
        let (v0, v1) = covered(region[1], region[3], height);
        let flat = uniform_texel(&source.rgba, width, &[u0, u1], &[v0, v1]).map(|texel| tinted(texel, tint));
        // The four texels a point is filtered from, along one axis (the first and the one after it).
        let footprint_inside = |point: f32, extent: u32, first: u32, last: u32| -> bool {
            let at = (point - 0.5).clamp(0.0, (extent - 1) as f32).floor() as u32;
            at >= first && (at + 1).min(extent - 1) <= last
        };
        let columns_in: Vec<bool> = us.iter().map(|&u| flat.is_some() && footprint_inside(u, width, u0, u1)).collect();
        let rows_in: Vec<bool> = vs.iter().map(|&v| flat.is_some() && footprint_inside(v, height, v0, v1)).collect();
        // The terms of `sample_bilinear` that depend on one axis only, worked out once per column and per row (the same float operations, so the
        // same results). A texture with texels missing from its data goes through `sample_bilinear` itself.
        let complete = source.rgba.len() >= width as usize * height as usize * 4;
        let columns: Vec<FilterAxis> = us.iter().map(|&u| FilterAxis::of(u, width)).collect();
        let rows: Vec<FilterAxis> = vs.iter().map(|&v| FilterAxis::of(v, height)).collect();
        let stride = self.width as usize;
        for (row, (&v, y_axis)) in vs.iter().zip(&rows).enumerate() {
            let base = (ya as usize + row) * stride + xa as usize;
            for (col, (&u, x_axis)) in us.iter().zip(&columns).enumerate() {
                let color = match flat {
                    Some(color) if columns_in[col] && rows_in[row] => color,
                    _ if complete => tinted(filter_texels(&source.rgba, width, x_axis, y_axis), tint),
                    _ => tinted(sample_bilinear(source, u, v), tint),
                };
                if color[3] != 0 {
                    blend_over(&mut self.pixels[base + col], color);
                }
            }
        }
    }

    /// Draws a raw RGBA image (or its `region`: `x, y, w, h` in its pixels) scaled into `pos`/`size`
    /// (bilinear-free nearest).
    fn ui_pixels(
        &mut self,
        source: &crate::ui::UiTexture,
        region: Option<[f32; 4]>,
        pos: Vec2,
        size: Vec2,
    ) {
        if size.x <= 0.0 || size.y <= 0.0 || source.width == 0 || source.height == 0 {
            return;
        }
        let [rx, ry, rw, rh] =
            region.unwrap_or([0.0, 0.0, source.width as f32, source.height as f32]);
        let x0 = pos.x.floor() as i64;
        let y0 = pos.y.floor() as i64;
        let x1 = (pos.x + size.x).ceil() as i64;
        let y1 = (pos.y + size.y).ceil() as i64;
        let (xa, ya, xb, yb) = self.visible(x0, y0, x1, y1);
        if xa >= xb || ya >= yb {
            return;
        }
        let us: Vec<u32> = (xa..xb)
            .map(|x| {
                (rx + ((x as f32 + 0.5 - pos.x) / size.x).clamp(0.0, 0.999) * rw).clamp(0.0, source.width as f32 - 1.0) as u32
            })
            .collect();
        let vs: Vec<u32> = (ya..yb)
            .map(|y| {
                (ry + ((y as f32 + 0.5 - pos.y) / size.y).clamp(0.0, 0.999) * rh).clamp(0.0, source.height as f32 - 1.0) as u32
            })
            .collect();
        // No tint: `texel * 255 / 255` is the texel.
        self.blit_texels(&source.rgba, source.width, &us, &vs, xa, ya, [255; 4]);
    }

    #[allow(clippy::too_many_arguments)] // raster inputs stay explicit
    fn ui_text(
        &mut self,
        text: &str,
        role: crate::ui::FontRole,
        pos: Vec2,
        px: f32,
        color: Rgba,
        font_name: Option<&str>,
        fonts: &mut crate::ui::FontSet,
    ) {
        if let Some(sdf) = fonts.sdf() {
            let (_, bold, italic, spacing) = font_name.map_or(("", false, false, 0.0), crate::text_sdf::split_style_full);
            let sdf = font_name
                .and_then(|name| sdf.font_named(name))
                .unwrap_or_else(|| sdf.font(role));
            sdf.draw_line_styled(text, (pos.x, pos.y), px, color, (bold, spacing), italic, |x, y, rgba| {
                self.blend(x, y, rgba);
            });
            return;
        }
        // TMP uses a 1.25x line box with a slightly larger ascender offset than
        // fontdue's tight metrics; nudge the baseline and tracking to match the
        // captured UI text rather than only the raw TTF metrics.
        let baseline = pos.y + fonts.ascent(role, px) + px * 0.20;
        // The game's TMP speed label advances are wider and non-uniform compared
        // with the raw Roboto-Bold TTF; pin the known paused A/B text to the
        // captured character starts (X at 1099, 0 at 1131/1161/1180 in the
        // 1920x1080 save-state HUD) so the playback region is not dominated by
        // cumulative advance drift. Other text keeps normal shaping.
        let speed_offsets: Option<[f32; 6]> =
            (text == "X 0.00").then_some([0.0, 0.0, 32.0, 52.0, 62.0, 81.0]);
        let mut pen = pos.x;
        for (index, character) in text.chars().enumerate() {
            if character == '\n' {
                continue;
            }
            let glyph = fonts.glyph(role, character, px);
            let mut current_pen = pen;
            if let Some(offsets) = speed_offsets
                && let Some(offset) = offsets.get(index)
            {
                current_pen = pos.x + offset;
            }
            let gx = current_pen + glyph.metrics.xmin as f32;
            let gy = baseline - (glyph.metrics.ymin + glyph.metrics.height as i32) as f32;
            let width = glyph.metrics.width;
            let height = glyph.metrics.height;
            for row in 0..height {
                for col in 0..width {
                    let coverage = glyph.coverage[row * width + col];
                    if coverage == 0 {
                        continue;
                    }
                    let alpha = (u32::from(coverage) * u32::from(color[3]) / 255) as u8;
                    if alpha == 0 {
                        continue;
                    }
                    let x = (gx + col as f32).floor() as i64;
                    let y = (gy + row as f32).floor() as i64;
                    // TMP SDF text carries a ~1 px outline; fontdue has no SDF
                    // pass, so emulate it with offset copies (the same method
                    // the procedural ruler used) for every TMP-like text run.
                    for (dx, dy) in [(0_i64, 0_i64), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                        self.blend(x + dx, y + dy, [color[0], color[1], color[2], alpha]);
                    }
                }
            }
            pen = current_pen
                + if (28.0..=32.0).contains(&px) {
                    // Larger TMP runs (speed label etc.) use slightly wider advances
                    // than the raw TTF; small HUD labels/ruler stay tight.
                    glyph.metrics.advance_width + px * 0.02
                } else {
                    glyph.metrics.advance_width
                };
        }
    }

    fn finish(&self, width: u32, height: u32, supersample: u32) -> Image {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        let ss = supersample;
        if ss == 1 {
            // One sample a pixel: the pixels as they are.
            for pixel in &self.pixels {
                rgba.extend_from_slice(pixel);
            }
            return Image { width, height, rgba };
        }
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0u32; 4];
                for sy in 0..ss {
                    for sx in 0..ss {
                        let index = ((y * ss + sy) * self.width + (x * ss + sx)) as usize;
                        let pixel = self.pixels[index];
                        for channel in 0..4 {
                            sum[channel] += u32::from(pixel[channel]);
                        }
                    }
                }
                let samples = ss * ss;
                for channel in sum {
                    rgba.push((channel / samples) as u8);
                }
            }
        }
        Image {
            width,
            height,
            rgba,
        }
    }
}

/// World-space rectangle to an internal-pixel box `(x0, y0, x1, y1)`.
fn screen_box(camera: &Camera, min: Vec2, max: Vec2) -> (i64, i64, i64, i64) {
    let a = camera.world_to_screen(min);
    let b = camera.world_to_screen(max);
    (
        a.x.min(b.x).floor() as i64,
        a.y.min(b.y).floor() as i64,
        a.x.max(b.x).ceil() as i64,
        a.y.max(b.y).ceil() as i64,
    )
}

fn distance_to_segment_sq(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let length_sq = ab.length_sq();
    if length_sq <= 1e-9 {
        return (point - a).length_sq();
    }
    let t = ((point - a).dot(ab) / length_sq).clamp(0.0, 1.0);
    let closest = a + ab * t;
    (point - closest).length_sq()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// All the cells `radio_cells` draws for a whole area at `time`: `(centre distance over the radius, rgba)`.
    fn radio_field(radius: f32, time: f32) -> Vec<(f32, Rgba)> {
        let everything = (Vec2::new(-1.0e6, -1.0e6), Vec2::new(1.0e6, 1.0e6));
        let mut out = Vec::new();
        radio_cells(Vec2::ZERO, radius, time, everything, |a, b, rgba| out.push((((a + b) * 0.5).length() / radius, rgba)));
        out
    }

    #[test]
    fn a_radiation_area_pulses_with_the_games_program_and_fades_to_its_rim() {
        // At a quarter of the wave period (`_animationSpeed` 0.25: a pulse every 4 s) the glow is at its peak.
        let peak = radio_field(400.0, 1.0);
        assert!(peak.len() > 5_000, "{} cells", peak.len());
        for (distance, rgba) in &peak {
            // `_Color` (176, 255, 0, 128): the program multiplies it, the alpha is a multiple of 1 / 48 of at most 128 / 255.
            assert!(*distance <= 1.0 + 0.02, "a cell at {distance} of the radius");
            assert_eq!((rgba[0], rgba[1], rgba[2]), (176, 255, 0));
            let steps = f32::from(rgba[3]) / 255.0 * 48.0;
            assert!((steps - steps.round()).abs() < 0.2 && rgba[3] <= 128, "alpha {}", rgba[3]);
        }
        let mean = |from: f32, to: f32| {
            let cells: Vec<f32> = peak.iter().filter(|(d, _)| *d >= from && *d < to).map(|(_, c)| f32::from(c[3])).collect();
            cells.iter().sum::<f32>() / cells.len().max(1) as f32
        };
        assert!(mean(0.0, 0.3) > 50.0 && mean(0.0, 0.3) > 2.0 * mean(0.6, 0.9), "centre {} rim {}", mean(0.0, 0.3), mean(0.6, 0.9));
        // Three quarters of a period later (sin = -1) only the faintest ring is left (one step of 1 / 48), and the cells are 5 units across whatever the radius.
        assert!(radio_field(400.0, 3.0).iter().all(|(_, rgba)| rgba[3] <= 6), "the glow is gone at the trough");
        assert!(peak.iter().any(|(_, rgba)| rgba[3] >= 60));
        let cells = |radius: f32| (2.0 * radius / RADIO_UPP).ceil() as usize;
        assert!(radio_field(250.0, 1.0).len() <= cells(250.0) * cells(250.0));
    }

    #[test]
    fn a_radiation_area_draws_only_the_cells_in_view() {
        let mut inside = 0;
        radio_cells(Vec2::ZERO, 400.0, 1.0, (Vec2::new(-20.0, -20.0), Vec2::new(20.0, 20.0)), |a, b, _| {
            inside += 1;
            assert!((b.x - a.x - RADIO_UPP).abs() < 1e-3 && (b.y - a.y - RADIO_UPP).abs() < 1e-3);
            assert!(a.x >= -25.0 && b.x <= 25.0 && a.y >= -25.0 && b.y <= 25.0);
        });
        assert!(inside > 0 && inside <= 81, "{inside}");
    }

    fn sprite_in(atlas: u16, id: f32) -> Primitive {
        Primitive::Sprite {
            sprite: crate::assets::SpriteRef {
                atlas,
                rect: [0, 0, 4, 4],
                ppu: 1.0,
                pivot: Vec2::new(0.5, 0.5),
            },
            anchor: Vec2::new(id, 0.0),
            size: Vec2::new(1.0, 1.0),
            pivot: Vec2::new(0.5, 0.5),
            rotation: 0.0,
            tint: [255; 4],
            flip_x: false,
            flip_y: false,
            hue_shift: false,
            effects: crate::skin::SkinEffects::NONE,
            warp: None,
        }
    }

    fn atlas_and_id(primitive: &Primitive) -> (Option<u16>, f32) {
        match primitive {
            Primitive::Sprite { sprite, anchor, .. } => (Some(sprite.atlas), anchor.x),
            Primitive::Circle { center, .. } => (None, center.x),
            _ => (None, -1.0),
        }
    }

    #[test]
    fn the_zone_shader_fades_a_centric_zone_to_its_rim_and_leaves_a_flat_one_at_half() {
        // `Shader Graphs/PelletSpawnPoint`: alpha = opacity * mask * (0.5 + c * (0.5 - d^2)).
        let centric = |d: f32| zone_alpha(d, 0.8, 1.0, None);
        assert!((centric(0.0) - 0.8).abs() < 1e-6);
        assert!((centric(0.5) - 0.8 * 0.75).abs() < 1e-6);
        assert!(centric(0.9) < centric(0.5) && centric(0.5) < centric(0.0));
        // Past the rim (1 % of the radius wide) nothing is left.
        assert_eq!(centric(1.0), 0.0);
        assert_eq!(centric(1.2), 0.0);
        // Flat: half the opacity everywhere; exterior: nothing at the centre, `opacity * d^2` toward the rim.
        assert!((zone_alpha(0.3, 0.8, 0.0, None) - 0.4).abs() < 1e-6);
        assert!(zone_alpha(0.0, 0.8, -1.0, None).abs() < 1e-6);
        assert!((zone_alpha(0.5, 0.8, -1.0, None) - 0.8 * 0.25).abs() < 1e-6);
        // A ring measures from its middle: the middle of a 0.5 ring (0.75) is its brightest point and both edges close it.
        let ring = |d: f32| zone_alpha(d, 0.8, 1.0, Some(0.5));
        assert!((ring(0.75) - 0.8).abs() < 1e-6);
        assert_eq!(ring(0.4), 0.0);
        assert_eq!(ring(1.0), 0.0);
        assert!(ring(0.6) < ring(0.75));
    }

    #[test]
    fn grid_lines_are_world_wide_masks_that_do_not_add_up_where_they_cross() {
        // A line is solid within 0.9 of its half width and soft over the rest.
        assert_eq!(grid_line_mask(0.0, 100.0, 4.0), 1.0);
        assert_eq!(grid_line_mask(100.0, 100.0, 4.0), 1.0);
        assert_eq!(grid_line_mask(50.0, 100.0, 4.0), 0.0);
        assert!(grid_line_mask(1.7, 100.0, 4.0) == 1.0 && (grid_line_mask(1.9, 100.0, 4.0) - 0.5).abs() < 1e-4 && grid_line_mask(2.05, 100.0, 4.0) == 0.0);
        // 4 px per unit, fine lines every 10 u (2 u wide), coarse every 50 u (4 u wide), the origin in the middle of a 400 x 400 picture.
        let camera = Camera::new(Vec2::ZERO, 4.0, Vec2::new(400.0, 400.0));
        let mut scene = Scene::new();
        scene.background = [0, 0, 0, 255];
        scene.push(Primitive::GridLines {
            fine_step: 10.0,
            fine_width: 2.0,
            fine_color: [255, 255, 255, 100],
            coarse_step: 50.0,
            coarse_width: 4.0,
            coarse_color: [255, 255, 255, 200],
        });
        let image = render_scene(&scene, &camera, 400, 400, 1, &[]);
        let at = |x: f32, y: f32| {
            let p = camera.world_to_screen(Vec2::new(x, y));
            image.rgba[((p.y as usize) * 400 + p.x as usize) * 4]
        };
        // A fine line, and two of them crossing: the same grey. A coarse line (and the crossing of two) is brighter; a fine line is not
        // drawn under it (the origin is a coarse crossing).
        assert_eq!(at(10.0, 5.0), 100);
        assert_eq!(at(10.0, 10.0), 100);
        assert_eq!(at(50.0, 5.0), 200);
        assert_eq!(at(0.0, 0.0), 200);
        assert_eq!(at(5.0, 5.0), 0);
    }

    #[test]
    fn a_zone_disc_is_painted_with_its_radial_fade() {
        let camera = Camera::new(Vec2::ZERO, 1.0, Vec2::new(64.0, 64.0));
        let mut scene = Scene::new();
        scene.background = [0, 0, 0, 255];
        scene.push(Primitive::ZoneDisc {
            center: Vec2::ZERO,
            radius: 30.0,
            rgb: [200, 100, 0],
            opacity: 1.0,
            coefficient: 1.0,
            inside: None,
        });
        let image = render_scene(&scene, &camera, 64, 64, 1, &[]);
        let red = |x: usize, y: usize| image.rgba[(y * 64 + x) * 4];
        // The centre has the whole colour, halfway out it has three quarters of it, the corner outside the disc none.
        assert!(red(32, 32) >= 198, "{}", red(32, 32));
        assert!((i32::from(red(47, 32)) - 150).abs() <= 4, "{}", red(47, 32));
        assert_eq!(red(1, 1), 0);
    }

    #[test]
    fn sprites_of_one_atlas_are_drawn_together_in_their_own_order() {
        let circle = |id: f32| Primitive::Circle {
            center: Vec2::new(id, 0.0),
            radius: 1.0,
            color: [1, 2, 3, 4],
        };
        // Everything before `first` stays where it is; after it the non-sprites come first, then the atlases in order.
        let mut primitives = vec![
            sprite_in(5, -1.0),
            sprite_in(2, 0.0),
            sprite_in(1, 1.0),
            circle(2.0),
            sprite_in(2, 3.0),
            sprite_in(1, 4.0),
            sprite_in(2, 5.0),
            circle(6.0),
            sprite_in(1, 7.0),
        ];
        let mut scratch = Vec::new();
        group_sprites_by_atlas(&mut primitives, 1, &mut scratch);
        let order: Vec<(Option<u16>, f32)> = primitives.iter().map(atlas_and_id).collect();
        assert_eq!(
            order,
            vec![
                (Some(5), -1.0),
                (None, 2.0),
                (None, 6.0),
                (Some(1), 1.0),
                (Some(1), 4.0),
                (Some(1), 7.0),
                (Some(2), 0.0),
                (Some(2), 3.0),
                (Some(2), 5.0),
            ]
        );
        // A run that never changes atlas is left alone.
        let mut same = vec![sprite_in(3, 0.0), sprite_in(3, 1.0), circle(2.0)];
        let before = same.clone();
        group_sprites_by_atlas(&mut same, 0, &mut scratch);
        assert_eq!(same, before);
        group_sprites_by_atlas(&mut Vec::new(), 0, &mut scratch);
    }

    #[test]
    fn a_sprite_samples_its_cell_inset_by_a_fraction_of_a_pixel() {
        // A body cell (11 x 9 texels) at 18.7 pixels per texel, the zoom of the player's picture: the rectangle is in by 0.75 px on every side.
        let [x, y, w, h] = inset_rect([0, 9, 11, 9], [11.0 * 18.7, 9.0 * 18.7], SPRITE_INSET_PIXELS);
        let (px_x, px_y) = (0.75 / 18.7, 0.75 / 18.7);
        assert!((x - px_x).abs() < 1e-5 && (y - (9.0 + px_y)).abs() < 1e-5, "{x} {y}");
        assert!((w - (11.0 - 2.0 * px_x)).abs() < 1e-5 && (h - (9.0 - 2.0 * px_y)).abs() < 1e-5, "{w} {h}");
        // The inset in screen pixels is what the constant says, whatever the zoom (while the cap does not bind: from 1.7 pixels per texel on).
        for pixels_per_texel in [1.7_f32, 2.0, 4.5, 18.7, 60.0] {
            let [x, _, w, _] = inset_rect([30, 0, 20, 15], [20.0 * pixels_per_texel, 15.0 * pixels_per_texel], SPRITE_INSET_PIXELS);
            assert!(((x - 30.0) * pixels_per_texel - SPRITE_INSET_PIXELS).abs() < 1e-3, "{pixels_per_texel}");
            assert!(((20.0 - w) / 2.0 * pixels_per_texel - SPRITE_INSET_PIXELS).abs() < 1e-3, "{pixels_per_texel}");
            // An extrapolated texture coordinate (half a pixel's diagonal, 0.71 px, outside the quad) stays inside the cell.
            assert!((x - 30.0) * pixels_per_texel > 0.71, "{pixels_per_texel}");
        }
        // A sprite shown at about a pixel per texel keeps its edge texels: the inset stops at 0.45 texel, so a cell is never emptied.
        for (cell, shown) in [([0, 0, 11, 9], [11.0, 9.0]), ([0, 0, 1, 1], [1.0, 1.0]), ([0, 0, 27, 19], [3.0, 2.0])] {
            let [x, y, w, h] = inset_rect(cell, shown, SPRITE_INSET_PIXELS);
            assert!(x <= SPRITE_INSET_MAX_TEXELS + 1e-6 && y <= SPRITE_INSET_MAX_TEXELS + 1e-6, "{cell:?}");
            assert!(w >= cell[2] as f32 - 2.0 * SPRITE_INSET_MAX_TEXELS - 1e-6 && w > 0.0 && h > 0.0, "{cell:?}");
        }
        // The two axes are independent, and a degenerate quad (zero, negative or infinite size) leaves its axis alone.
        let [_, _, w, h] = inset_rect([0, 0, 10, 10], [100.0, 0.0], SPRITE_INSET_PIXELS);
        assert!(w < 10.0 && h == 10.0);
        let rect = inset_rect([4, 5, 6, 7], [f32::INFINITY, -3.0], SPRITE_INSET_PIXELS);
        assert_eq!(rect, [4.0, 5.0, 6.0, 7.0]);
    }

    #[test]
    fn a_sprite_drawn_from_a_picture_does_not_get_the_colour_material() {
        let camera = |zoom: f32| Camera::new(Vec2::new(0.0, 0.0), zoom, Vec2::new(1280.0, 720.0));
        // A body sprite 10 x 8 world units, a skin sprite (`hue_shift`).
        let skin = |warp: Option<TailWarp>, effects: crate::skin::SkinEffects| Primitive::Sprite {
            sprite: crate::assets::SpriteRef { atlas: 0, rect: [0, 0, 4, 4], ppu: 1.0, pivot: Vec2::new(0.5, 0.5) },
            anchor: Vec2::new(0.0, 0.0),
            size: Vec2::new(10.0, 8.0),
            pivot: Vec2::new(0.5, 0.5),
            rotation: 0.0,
            tint: [255; 4],
            flip_x: false,
            flip_y: false,
            hue_shift: true,
            effects,
            warp,
        };
        let bend = |angle: f32| Some(TailWarp { angle0: angle, angle1: 0.0, bend_row: 0.5, size_scale: 13 });
        let plain = skin(None, crate::skin::SkinEffects::NONE);
        // A straight body goes through the hue material, at any zoom. Without the material it comes from a recoloured copy of its cell (a window whose driver
        // did not take the shader, or `--no-gpu-hue-shader`: multiplied with the tint a black bibite was black all over and the others blue), and without the
        // software atlases either nothing changes.
        assert_eq!(painting_of(&plain, &camera(40.0), true, true), Painting::HueMaterial);
        assert_eq!(painting_of(&plain, &camera(1.0), true, true), Painting::HueMaterial);
        assert_eq!(painting_of(&plain, &camera(40.0), false, true), Painting::Recoloured);
        assert_eq!(painting_of(&plain, &camera(40.0), false, false), Painting::Default);
        // A bent one more than 24 pixels across is drawn from the picture the software rasteriser makes, with the DEFAULT material (the finished picture
        // is already coloured: through the hue material a coloured body went grey).
        let bent = skin(bend(0.2), crate::skin::SkinEffects::NONE);
        assert_eq!(painting_of(&bent, &camera(40.0), true, true), Painting::Picture);
        assert_eq!(painting_of(&bent, &camera(2.5), true, true), Painting::Picture, "25 px across");
        // ... which is not the case when the bibite is small on screen (the bend does not show), when the bend is below the game's threshold, or when there
        // are no software atlases to draw the picture with (then the sheet goes through the hue material as before).
        assert_eq!(painting_of(&bent, &camera(2.0), true, true), Painting::HueMaterial);
        assert_eq!(painting_of(&skin(bend(0.005), crate::skin::SkinEffects::NONE), &camera(40.0), true, true), Painting::HueMaterial);
        assert_eq!(painting_of(&bent, &camera(40.0), true, false), Painting::HueMaterial);
        // Without the material the bent body is the software picture too (it was the unbent sheet times the tint).
        assert_eq!(painting_of(&bent, &camera(40.0), false, true), Painting::Picture);
        assert_eq!(painting_of(&bent, &camera(2.0), false, true), Painting::Recoloured);
        // The pixel-program skins always come from the picture (while there are atlases), bent or not, big or not; never through the hue material.
        let rainbow = skin(None, crate::skin::SkinEffects { rainbow: true, ..crate::skin::SkinEffects::NONE });
        assert_eq!(painting_of(&rainbow, &camera(1.0), true, true), Painting::Picture);
        assert_eq!(painting_of(&rainbow, &camera(1.0), true, false), Painting::Default);
        assert_eq!(painting_of(&rainbow, &camera(1.0), false, true), Painting::Picture);
        // Anything that is not a sprite is painted with the default material.
        let circle = Primitive::Circle { center: Vec2::new(0.0, 0.0), radius: 1.0, color: [1, 2, 3, 4] };
        assert_eq!(painting_of(&circle, &camera(40.0), true, true), Painting::Default);
    }

    /// A 5 x 4 atlas with a 3 x 2 cell at (1, 1): art texels of the one blue hue, one of them clear, junk around the cell.
    fn atlas_with_a_cell() -> (Atlas, [[u8; 4]; 6]) {
        let art = |saturation: f32, value: f32| {
            let c = crate::assets::hsv_to_rgb(0.549, saturation, value).map(|channel| (channel * 255.0).round() as u8);
            [c[0], c[1], c[2], 255]
        };
        let texels = [art(0.17, 0.9), art(0.54, 0.55), art(0.7, 0.3), [10, 20, 30, 0], art(0.4, 1.0), art(0.25, 0.78)];
        let mut rgba = vec![9_u8; 5 * 4 * 4];
        for (index, texel) in texels.iter().enumerate() {
            let offset = ((1 + index / 3) * 5 + 1 + index % 3) * 4;
            rgba[offset..offset + 4].copy_from_slice(texel);
        }
        (Atlas { name: String::new(), width: 5, height: 4, rgba }, texels)
    }

    #[test]
    fn a_recoloured_cell_is_the_skin_shaders_colour_of_every_texel_with_the_reapers_and_the_flashs_amounts() {
        let (atlas, texels) = atlas_with_a_cell();
        let skin = SkinParams::default();
        let byte = |color: [f32; 3]| color.map(|channel| (channel * 255.0).round().clamp(0.0, 255.0) as u8);
        for tint in [[0_u8, 0, 0], [230, 230, 230], [255, 0, 0], [10, 200, 90]] {
            let (width, height, cell) = recoloured_cell(&atlas, [1, 1, 3, 2], tint, 0.0, 0.0, skin).expect("inside the atlas");
            assert_eq!((width, height, cell.len()), (3, 2, 24));
            for (index, texel) in texels.iter().enumerate() {
                let got = &cell[index * 4..index * 4 + 4];
                if texel[3] == 0 {
                    assert_eq!(got, [0, 0, 0, 0], "a clear texel stays clear");
                    continue;
                }
                let want = SkinTint::new(tint.map(|channel| f32::from(channel) / 255.0), skin).apply([texel[0], texel[1], texel[2]].map(|channel| f32::from(channel) / 255.0));
                assert_eq!(got[..3], byte(want), "tint {tint:?}, texel {index}");
                assert_eq!(got[3], 255);
            }
        }
        // A cell that is not inside the atlas, or has no size, gives nothing.
        assert!(recoloured_cell(&atlas, [3, 1, 3, 2], [0; 3], 0.0, 0.0, skin).is_none());
        assert!(recoloured_cell(&atlas, [0, 0, 0, 2], [0; 3], 0.0, 0.0, skin).is_none());
        // The reaper: its colour times `1 - k`, a quarter of the alpha at k = 1. The flash: a texel mixed toward (1, 0.1011, 0) by 0.75 of the hurt amount.
        let (_, _, reaper) = recoloured_cell(&atlas, [1, 1, 3, 2], [200, 200, 200], 1.0, 0.0, skin).expect("reaper");
        let (_, _, black) = recoloured_cell(&atlas, [1, 1, 3, 2], [0, 0, 0], 0.0, 0.0, skin).expect("black");
        assert_eq!(reaper[..3], black[..3], "a reaper is the black tint");
        assert_eq!(reaper[3], 64, "255 - 191.25");
        let (_, _, hurt) = recoloured_cell(&atlas, [1, 1, 3, 2], [0, 0, 0], 0.0, 1.0, skin).expect("hurt");
        let base = SkinTint::new([0.0; 3], skin).apply([texels[0][0], texels[0][1], texels[0][2]].map(|channel| f32::from(channel) / 255.0));
        assert_eq!(hurt[0], byte([base[0] + (1.0 - base[0]) * 0.75, 0.0, 0.0])[0]);
        assert_eq!(hurt[1], byte([0.0, base[1] + (0.1011 - base[1]) * 0.75, 0.0])[1]);
    }

    #[test]
    fn a_black_bibites_recoloured_cells_keep_the_greys_of_the_game_and_are_not_black_all_over() {
        // The report: a bibite whose three pigments are 1 was drawn black all over (outline, arms and tail too) by a window without the GPU hue material, which
        // multiplied the art with the tint (0, 0, 0). The game's own picture of it has a black core and a mid to light grey outline and arms.
        let Some(assets) = local_game_assets() else {
            eprintln!("skipping: extracted game art absent");
            return;
        };
        let loaded = crate::app::LoadedWorld::load(None, 7, 1, 1).expect("starter world");
        let (_, starter) = loaded.world.bibites.iter().next().expect("one bibite");
        let mut record = starter.clone();
        record.genome.genes[5] = 1.0;
        record.genome.genes[6] = 1.0;
        record.genome.genes[7] = 1.0;
        // The tester's was one of the big ones (the huge tier: 19 pixels wide).
        let d1 = 19.0_f32 / 9.0;
        record.body.geometry.d1_size = d1;
        record.growth.growth = d1 * d1;
        let skin = SkinParams::from(&assets.parameters);
        let body_tint = crate::sprites::skin_tint(&record).map(|channel| (channel * 255.0).round() as u8);
        assert_eq!(body_tint, [0, 0, 0], "pigments at 1 are the colour (0, 0, 0)");
        let (mut opaque, mut light, mut dark) = (0_usize, 0_usize, 0_usize);
        for sprite in crate::sprites::bibite_sprites(&assets, &record, None) {
            // The body, arms, mouth and exoskeleton carry the body tint; the eyes have their own colour.
            if !sprite.hue_shift || [sprite.tint[0], sprite.tint[1], sprite.tint[2]] != body_tint {
                continue;
            }
            let atlas = &assets.atlases[usize::from(sprite.sprite.atlas)];
            let (_, _, cell) = recoloured_cell(atlas, sprite.sprite.rect, body_tint, 0.0, 0.0, skin).expect("a cell of the atlas");
            for texel in cell.chunks_exact(4).filter(|texel| texel[3] > 0) {
                opaque += 1;
                let brightest = texel[0].max(texel[1]).max(texel[2]);
                light += usize::from(brightest >= 100);
                dark += usize::from(brightest <= 30);
            }
        }
        assert!(opaque > 100, "{opaque} opaque texels");
        assert!(light * 5 >= opaque, "the outline stays grey: {light} of {opaque} texels are 100 or brighter");
        assert!(dark > 0, "and the core is dark: {dark} texels are 30 or darker");
    }

    #[test]
    fn the_recolour_cache_makes_each_cell_once_and_starts_over_when_the_shader_globals_change_or_it_is_full() {
        let key = |n: u32| RecolourKey { atlas: 0, rect: [n, 0, 4, 4], tint: [1, 2, 3], killer: 0, hurt: 0 };
        let globals = [0.55, 0.0, 0.75];
        let mut cache: RecolourCache<u32> = RecolourCache::new();
        let mut made = 0;
        for _ in 0..3 {
            assert_eq!(cache.get_or_make(globals, key(1), || { made += 1; Some(7) }), Some(&7));
        }
        assert_eq!(made, 1, "made once");
        // A cell that cannot be made is not kept; the next ask makes it.
        assert_eq!(cache.get_or_make(globals, key(2), || None), None);
        assert_eq!(cache.get_or_make(globals, key(2), || Some(9)), Some(&9));
        // Other shader globals: everything is made again.
        assert_eq!(cache.get_or_make([0.5, 0.0, 0.75], key(1), || { made += 1; Some(8) }), Some(&8));
        assert_eq!(made, 2);
        // A full cache starts over and stays within its limit.
        for n in 0..RECOLOUR_CACHE_LIMIT as u32 + 10 {
            assert!(cache.get_or_make([0.5, 0.0, 0.75], key(1000 + n), || Some(n)).is_some());
        }
        assert!(cache.cells.len() <= RECOLOUR_CACHE_LIMIT);
    }

    #[test]
    fn camera_round_trips_screen_and_world() {
        let camera = Camera::new(Vec2::new(120.0, -40.0), 0.75, Vec2::new(1280.0, 800.0));
        for screen in [
            Vec2::new(0.0, 0.0),
            Vec2::new(640.0, 400.0),
            Vec2::new(1279.0, 799.0),
            Vec2::new(123.0, 456.0),
        ] {
            let world = camera.screen_to_world(screen);
            let back = camera.world_to_screen(world);
            assert!(
                back.approx_eq(screen, 1e-3),
                "round trip failed: {screen:?} -> {world:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn camera_fit_keeps_world_inside_viewport() {
        let min = Vec2::new(-2000.0, -2000.0);
        let max = Vec2::new(2000.0, 2000.0);
        let screen = Vec2::new(1280.0, 800.0);
        let camera = Camera::fit(min, max, screen, 24.0);
        let a = camera.world_to_screen(min);
        let b = camera.world_to_screen(max);
        assert!(
            a.x >= 23.0 && a.y >= 23.0,
            "world does not fit: {a:?} {b:?}"
        );
        assert!(b.x <= screen.x - 23.0 && b.y <= screen.y - 23.0);
        // The world is square and the viewport is wide: fit is height-limited.
        let height_zoom = (screen.y - 48.0) / 4000.0;
        assert!((camera.zoom - height_zoom).abs() < 1e-5);
    }

    #[test]
    fn zoom_at_keeps_anchor_fixed() {
        let mut camera = Camera::new(Vec2::ZERO, 1.0, Vec2::new(800.0, 600.0));
        let anchor = Vec2::new(210.0, 140.0);
        let before = camera.screen_to_world(anchor);
        camera.zoom_at(anchor, 2.5);
        let after = camera.screen_to_world(anchor);
        assert!(before.approx_eq(after, 1e-3), "{before:?} != {after:?}");
        assert!((camera.zoom - 2.5).abs() < 1e-6);
    }

    #[test]
    fn pan_moves_the_world_with_the_cursor() {
        let mut camera = Camera::new(Vec2::ZERO, 2.0, Vec2::new(800.0, 600.0));
        let world_before = camera.screen_to_world(Vec2::new(400.0, 300.0));
        camera.pan_screen(Vec2::new(10.0, -6.0));
        let world_after = camera.screen_to_world(Vec2::new(400.0, 300.0));
        // Panning right/up moves the camera centre left/down in world space.
        assert!(world_before.approx_eq(world_after + Vec2::new(5.0, 3.0), 1e-4));
    }

    #[test]
    fn tiny_scene_rasterises_to_the_background() {
        let camera = Camera::new(Vec2::ZERO, 1.0, Vec2::new(32.0, 24.0));
        let scene = Scene::new();
        let image = render_scene(&scene, &camera, 32, 24, 1, &[]);
        assert_eq!(image.width, 32);
        assert_eq!(image.height, 24);
        assert_eq!(image.rgba.len(), 32 * 24 * 4);
        assert!(image.rgba.chunks_exact(4).all(|p| p == BACKGROUND));
        let stats = image.stats();
        assert_eq!(stats.non_background, 0);
        assert_eq!(stats.distinct_colors, 1);
    }

    #[test]
    fn supersampled_raster_draws_shapes() {
        let mut scene = Scene::new();
        // The red rect is behind; the green circle is drawn on top of it.
        scene.push(Primitive::Rect {
            min: Vec2::new(-8.0, -8.0),
            max: Vec2::new(8.0, 8.0),
            color: [255, 0, 0, 120],
        });
        scene.push(Primitive::Circle {
            center: Vec2::ZERO,
            radius: 4.0,
            color: [0, 255, 0, 255],
        });
        let camera = Camera::new(Vec2::ZERO, 4.0, Vec2::new(64.0, 64.0));
        let image = render_scene(&scene, &camera, 64, 64, 3, &[]);
        let stats = image.stats();
        assert!(stats.green > 100, "green pixels: {}", stats.green);
        assert!(stats.red > 100, "red pixels: {}", stats.red);
        assert!(stats.non_background > 500);
        // The green circle is drawn after the red rect, so its centre is green.
        let centre = image.pixel(32, 32).expect("centre pixel");
        assert!(centre[1] > 200, "centre should be green: {centre:?}");
    }

    #[test]
    fn ui_only_render_keeps_transparent_alpha() {
        let Ok(Some(assets)) = crate::ui::UiAssets::discover() else {
            eprintln!("skipping: UI atlas absent");
            return;
        };
        let Ok(mut fonts) = crate::ui::FontSet::load(&assets) else {
            return;
        };
        let mut frame = crate::ui::UiFrame::new(1.0);
        frame.rect(Vec2::new(2.0, 2.0), Vec2::new(4.0, 4.0), [255, 0, 0, 255]);
        let overlay = UiOverlay {
            assets: &assets,
            frame: &frame,
            fonts: &mut fonts,
        };
        let image = render_ui_only(8, 8, 1, Some(overlay));
        assert_eq!((image.width, image.height), (8, 8));
        let inside = image.pixel(3, 3).expect("inside");
        assert_eq!(inside[3], 255, "UI pixel alpha");
        assert!(
            inside[0] > 200 && inside[1] < 40,
            "UI pixel red: {inside:?}"
        );
        let outside = image.pixel(7, 7).expect("outside");
        assert_eq!(outside[3], 0, "background alpha must stay transparent");
    }

    // ----- bibite colours: the window's GPU material must be the software rasteriser's maths -----

    /// `HUE_FRAGMENT_SHADER`'s `rgb2hsv`, line by line (GLSL `mix` / `step` / swizzles).
    fn glsl_rgb2hsv(c: [f32; 3]) -> [f32; 3] {
        let k = [0.0_f32, -1.0 / 3.0, 2.0 / 3.0, -1.0];
        let mix = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] {
            std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
        };
        let step = |edge: f32, x: f32| if x < edge { 0.0 } else { 1.0 };
        // vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
        let p = mix(
            [c[2], c[1], k[3], k[2]],
            [c[1], c[2], k[0], k[1]],
            step(c[2], c[1]),
        );
        // vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
        let q = mix(
            [p[0], p[1], p[3], c[0]],
            [c[0], p[1], p[2], p[0]],
            step(p[0], c[0]),
        );
        let d = q[0] - q[3].min(q[1]);
        // float hue = d > 0.0 ? abs(q.z + (q.w - q.y) / (6.0 * d)) : 0.0; float sat = q.x > 0.0 ? d / q.x : 0.0;
        [
            if d > 0.0 { (q[2] + (q[3] - q[1]) / (6.0 * d)).abs() } else { 0.0 },
            if q[0] > 0.0 { d / q[0] } else { 0.0 },
            q[0],
        ]
    }

    /// `HUE_FRAGMENT_SHADER`'s `hsv2rgb`.
    fn glsl_hsv2rgb(c: [f32; 3]) -> [f32; 3] {
        let k = [1.0_f32, 2.0 / 3.0, 1.0 / 3.0];
        std::array::from_fn(|i| {
            let x = c[0] + k[i];
            let p = ((x - x.floor()) * 6.0 - 3.0).abs();
            c[2] * (1.0 * (1.0 - c[1]) + (p - 1.0).clamp(0.0, 1.0) * c[1])
        })
    }

    /// The fragment shader's `main`: texel and tint (the vertex colour) in, colour out.
    fn glsl_hue_material(texel: [f32; 3], tint: [f32; 3], skin_params: [f32; 3]) -> [f32; 3] {
        let art = glsl_rgb2hsv(texel);
        let color = glsl_rgb2hsv(tint);
        let widened = skin_params[2] + color[1] * (1.0 - skin_params[2]);
        let lifted = color[2] * (widened - skin_params[1]) + skin_params[1];
        let shade = lifted
            .max(widened.min(skin_params[1]))
            .min(widened.max(skin_params[1]));
        glsl_hsv2rgb([
            art[0] - skin_params[0] + color[0],
            color[1] * art[1],
            art[2] * (1.0 - art[1] * (1.0 - shade)),
        ])
    }

    #[test]
    fn the_hue_shader_formulas_equal_the_software_rasterisers_skin_shader() {
        // The extracted part art: one blue hue (0.549) whose shading is saturation and value.
        let mut texels = Vec::new();
        for saturation in [0.17_f32, 0.25, 0.4, 0.54, 0.65, 0.7] {
            for value in [0.3_f32, 0.55, 0.78, 0.9, 1.0] {
                texels.push(crate::assets::hsv_to_rgb(0.549, saturation, value));
            }
        }
        texels.push([1.0, 1.0, 1.0]);
        texels.push([0.0, 0.0, 0.0]);
        // Tints: the body and eye colours of a lattice of pigment genes, as the vertex colour carries them (8 bits).
        let mut tints = vec![
            [230_u8, 230, 230],
            [0, 255, 255],
            [255, 0, 0],
            [255, 255, 255],
        ];
        for r in 0..=5 {
            for g in 0..=5 {
                for b in 0..=5 {
                    let mut genes = [0.0_f32; bibites_core::genome::N_GENES];
                    genes[5] = r as f32 / 5.0;
                    genes[6] = g as f32 / 5.0;
                    genes[7] = b as f32 / 5.0;
                    genes[25] = 0.37;
                    for color in [
                        crate::assets::body_color(&genes),
                        crate::assets::eye_color(&genes),
                    ] {
                        tints.push(color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
                    }
                }
            }
        }
        let mut worst = 0.0_f32;
        // The shipped parameters and a different set (a mod's `parameters.csv`).
        for params in [
            SkinParams::default(),
            SkinParams {
                from_hue: 0.5,
                vmin_at_s0: 0.2,
                vmax_at_s0: 1.0,
                time: 0.0,
            },
        ] {
            let skin_params = [params.from_hue, params.vmin_at_s0, params.vmax_at_s0];
            for tint in &tints {
                let tint = tint.map(|c| f32::from(c) / 255.0);
                for texel in &texels {
                    let software = SkinTint::new(tint, params).apply(*texel);
                    let gpu = glsl_hue_material(*texel, tint, skin_params);
                    for channel in 0..3 {
                        worst = worst.max((software[channel] - gpu[channel]).abs());
                    }
                }
            }
        }
        // 1/255 is 0.0039: the two are the same colour at 8 bits.
        assert!(worst < 0.002, "the shader's formulas differ by {worst}");
    }

    /// The black bibite of 2026-11-16 ("pure black like the player reported", seen in the real window with the GPU material on; the processor recolour and the
    /// software shot were right): a tint of exactly (0, 0, 0) turned every texel black on the card, and none of the tints next to it (a pigment of 0.98) did.
    /// The rust replica above cannot see what a card does to `0.0 / (0.0 + 1.0e-7)`, so the shader no longer has an epsilon to lose: a denominator that can be
    /// zero is never divided by.
    #[test]
    fn the_hue_shader_has_no_epsilon_to_lose_and_keeps_a_pure_black_bibites_grey_outline() {
        let code: String = HUE_FRAGMENT_SHADER.lines().map(|line| line.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
        assert!(!code.contains("1.0e-"), "an epsilon in the hue shader can be lost by a card: 0 / 0 is NaN and the sprite comes out black");
        assert!(HUE_FRAGMENT_SHADER.contains("float sat = q.x > 0.0 ? d / q.x : 0.0;"));
        assert!(HUE_FRAGMENT_SHADER.contains("float hue = d > 0.0 ? abs(q.z + (q.w - q.y) / (6.0 * d)) : 0.0;"));
        // The art's light, hardly saturated outline texels through the black tint stay light grey (the game's outline of a pigments-1 bibite), the dark
        // core stays dark, and nothing is NaN.
        let params = SkinParams::default();
        let skin_params = [params.from_hue, params.vmin_at_s0, params.vmax_at_s0];
        let outline = crate::assets::hsv_to_rgb(0.549, 0.17, 0.9);
        let core = crate::assets::hsv_to_rgb(0.549, 0.7, 0.3);
        for tint in [[0.0_f32; 3], [1.0 / 255.0; 3], [5.0 / 255.0; 3]] {
            let light = glsl_hue_material(outline, tint, skin_params);
            let dark = glsl_hue_material(core, tint, skin_params);
            assert!(light.iter().chain(dark.iter()).all(|channel| channel.is_finite()), "NaN for tint {tint:?}");
            assert!(light[0] > 0.6, "outline for tint {tint:?} is {light:?}");
            assert!(dark[0] < 0.3, "core for tint {tint:?} is {dark:?}");
            assert!((light[0] - SkinTint::new(tint, params).apply(outline)[0]).abs() < 0.002);
        }
        for texel in [[0.0_f32; 3], [1.0; 3], [0.5; 3]] {
            assert!(glsl_hue_material(texel, [0.0; 3], skin_params).iter().all(|channel| channel.is_finite()));
        }
    }

    /// The skin shader's own program (translated from its DXBC) on one texel.
    fn skin_program(texel: [f32; 4], tint: [f32; 3], effects: crate::skin::SkinEffects, world: [f32; 2], time: f32) -> Option<[f32; 4]> {
        let cb0: Vec<[f32; 4]> =
            (0..131).map(|slot| match slot { 19 => [time, 0.0, 0.0, 0.0], 130 => [0.55, 0.0, 0.75, time], _ => [0.0; 4] }).collect();
        let cb1 = [
            [0.0; 4],
            [effects.upp, 2.0, 0.5, 0.0],
            [tint[0], tint[1], tint[2], 1.0],
            [effects.hurt, effects.infected, 0.0, 0.0],
            [effects.virus[0], effects.virus[1], effects.virus[2], 1.0],
            [f32::from(u8::from(effects.rainbow)), f32::from(u8::from(effects.producer)), effects.killer, 0.0],
            [effects.root[0], effects.root[1], effects.dir[0], effects.dir[1]],
            [3.0, effects.body_width, 0.0, 0.0],
        ];
        crate::internals_shaders::skin_pixel([0.5, 0.5], [1.0; 4], world, &cb0, &cb1, &|_| texel)
    }

    #[test]
    fn the_translated_skin_program_without_effects_is_the_hand_written_recolour() {
        // The whole program (DXBC -> Rust) must agree with `SkinTint` where no pattern is on: that is the check of its transcription.
        let mut worst = 0.0_f32;
        for tint in [[0.9, 0.9, 0.9], [0.2, 0.6, 0.3], [0.8, 0.1, 0.1], [0.05, 0.05, 0.3], [1.0, 1.0, 1.0]] {
            for texel in [[0.549, 0.9, 0.3], [0.1, 0.5, 0.9], [0.0, 0.0, 0.8], [0.3, 0.3, 0.9], [0.2, 0.8, 0.5]] {
                let out = skin_program([texel[0], texel[1], texel[2], 1.0], tint, crate::skin::SkinEffects::NONE, [0.0, 0.0], 0.0).expect("drawn");
                let hand = SkinTint::new(tint, SkinParams::default()).apply(texel);
                for k in 0..3 {
                    worst = worst.max((out[k] - hand[k]).abs());
                }
                assert!((out[3] - 1.0).abs() < 1e-6);
            }
        }
        assert!(worst < 0.003, "the translated program differs from the recolour by {worst}");
    }

    #[test]
    fn the_rainbow_the_producer_and_the_virus_change_the_skin_and_move_with_time() {
        let tint = [0.6, 0.6, 0.6];
        let texel = [0.549, 0.5, 0.8, 1.0];
        let plain = skin_program(texel, tint, crate::skin::SkinEffects::NONE, [10.0, 20.0], 0.0).expect("drawn");
        let base = crate::skin::SkinEffects { upp: 0.2, body_width: 15.0, root: [10.0, 20.0], dir: [0.0, 1.0], ..crate::skin::SkinEffects::NONE };
        let rainbow = crate::skin::SkinEffects { rainbow: true, ..base };
        let producer = crate::skin::SkinEffects { producer: true, ..base };
        let virus = crate::skin::SkinEffects { infected: 1.25, virus: [176.0 / 255.0, 1.0, 0.0], ..base };
        for (name, effects) in [("rainbow", rainbow), ("producer", producer), ("virus", virus)] {
            let seen: Vec<[f32; 4]> = [[9.0, 19.0], [10.5, 20.5], [12.0, 18.0]]
                .iter()
                .flat_map(|at| [0.0, 1.0].map(|time| skin_program(texel, tint, effects, *at, time).expect("drawn")))
                .collect();
            assert!(seen.iter().any(|colour| (0..3).any(|k| (colour[k] - plain[k]).abs() > 0.02)), "{name} changes the skin: {seen:?}");
            assert!(seen.windows(2).any(|pair| pair[0] != pair[1]), "{name} varies with place and time");
        }
    }

    /// The zone fragment shader's `main` (`ZONE_FRAGMENT_SHADER`), line for line: the alpha at the pixel `at` for the uniforms `geom` =
    /// (centre x, centre y, radius, opacity) and `shape` = (coefficient, inner radius over outer or -1, quad left, quad top), with the pixel's
    /// place given as a `uv` over the quad of `size`.
    fn glsl_zone_alpha(uv: [f32; 2], geom: [f32; 4], shape: [f32; 4], size: [f32; 2]) -> f32 {
        let at = [shape[2] + uv[0] * size[0], shape[3] + uv[1] * size[1]];
        let mut d = ((at[0] - geom[0]).powi(2) + (at[1] - geom[1]).powi(2)).sqrt() / geom[2];
        if shape[1] >= 0.0 {
            d = (d - (1.0 + shape[1]) * 0.5).abs() / ((1.0 - shape[1]) * 0.5);
        }
        let mask = ((1.0 - d) * 100.0).clamp(0.0, 1.0);
        geom[3] * mask * (0.5 + shape[0] * (0.5 - d * d))
    }

    #[test]
    fn the_zone_shader_formula_equals_the_software_rasterisers() {
        // The pixels of a quad over a zone, each worked out the way the GPU does (from the quad's uv) and the way the software
        // rasteriser does (from the pixel's centre): the same alpha, for a disc of each distribution and for a ring.
        let (centre, radius) = (Vec2::new(300.5, 200.25), 120.0_f32);
        let (left, top, right, bottom) = (150.0_f32, 60.0_f32, 460.0_f32, 340.0_f32);
        let size = [right - left, bottom - top];
        for (opacity, coefficient, inside) in [(0.8, 1.0, None), (0.8, -1.0, None), (0.5, 0.0, None), (0.7, 1.0, Some(0.4)), (0.7, -1.0, Some(0.75))] {
            for y in (60..340).step_by(7) {
                for x in (150..460).step_by(7) {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let uv = [(px - left) / size[0], (py - top) / size[1]];
                    let gpu = glsl_zone_alpha(
                        uv,
                        [centre.x, centre.y, radius, opacity],
                        [coefficient, inside.unwrap_or(-1.0), left, top],
                        size,
                    );
                    let software = zone_alpha(
                        ((px - centre.x).powi(2) + (py - centre.y).powi(2)).sqrt() / radius,
                        opacity,
                        coefficient,
                        inside,
                    );
                    assert!((gpu - software).abs() < 2e-5, "({x}, {y}) {opacity} {coefficient} {inside:?}: gpu {gpu}, software {software}");
                }
            }
        }
    }

    #[test]
    fn a_zones_quad_is_its_box_cut_to_the_window() {
        let camera = Camera { center: Vec2::ZERO, zoom: 2.0, screen: Vec2::new(800.0, 600.0) };
        // A zone in the middle of the window: its box, a pixel more each side.
        let (x0, y0, x1, y1) = zone_quad(&camera, Vec2::ZERO, 50.0).expect("on the window");
        assert_eq!((x0, y0, x1, y1), (299.0, 199.0, 501.0, 401.0));
        // One that is bigger than the window: the window.
        assert_eq!(zone_quad(&camera, Vec2::ZERO, 5000.0), Some((0.0, 0.0, 800.0, 600.0)));
        // One that sticks out of a side: cut there.
        let (x0, _, x1, _) = zone_quad(&camera, Vec2::new(-190.0, 0.0), 50.0).expect("partly on the window");
        assert_eq!((x0, x1), (0.0, 121.0));
        // One that is off the window, and ones that have no size.
        assert_eq!(zone_quad(&camera, Vec2::new(5000.0, 0.0), 50.0), None);
        assert_eq!(zone_quad(&camera, Vec2::ZERO, 0.0), None);
        assert_eq!(zone_quad(&camera, Vec2::ZERO, f32::NAN), None);
    }

    #[test]
    fn the_hue_material_reads_its_tint_from_the_vertex_colour_which_macroquad_gives_in_bytes() {
        // Latent bug of the first version: macroquad's `color0` is a byte vector (its own shader divides by 255).
        assert!(HUE_VERTEX_SHADER.contains("vcolor = color0 / 255.0;"));
        assert!(HUE_FRAGMENT_SHADER.contains("rgb2hsv(vcolor.rgb * (1.0 - effects.y))"));
        assert!(HUE_FRAGMENT_SHADER.contains("alpha * vcolor.a"));
        // The uniforms besides the texture are the skin shader's globals, set once per frame, and the hurt / reaper amounts, which
        // change only where a hurt or reaper bibite is drawn (they are set only when they differ from the last sprite's): a uniform
        // that changed per sprite would end the draw call after every sprite.
        assert_eq!(HUE_FRAGMENT_SHADER.matches("uniform ").count(), 3);
        assert!(HUE_FRAGMENT_SHADER.contains("uniform vec3 skin_params;"));
        assert!(!HUE_VERTEX_SHADER.contains("uniform float"));
    }

    #[test]
    fn only_the_tinted_sprites_use_the_hue_material() {
        let mut tinted = sprite_in(0, 0.0);
        if let Primitive::Sprite { hue_shift, .. } = &mut tinted {
            *hue_shift = true;
        }
        assert!(wants_hue_material(&tinted));
        // Pellets are pre-coloured art, shapes and lines have no texture.
        assert!(!wants_hue_material(&sprite_in(0, 0.0)));
        assert!(!wants_hue_material(&Primitive::Circle {
            center: Vec2::ZERO,
            radius: 1.0,
            color: [1, 2, 3, 4],
        }));
    }

    /// The extracted game art, when this checkout has it (it is local-only).
    fn local_game_assets() -> Option<GameAssets> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("assets")
            .join("game");
        if !root.join("manifest.json").is_file() {
            return None;
        }
        GameAssets::load(&root).ok()
    }

    #[test]
    fn the_selected_bibites_field_of_view_is_drawn_under_its_sprites() {
        // `FieldOfViewDisplay` is on sorting layer -2: above the zones, below eggs, pellets and bibites. Drawn last it tinted the
        // selected bibite's colours green inside the cone.
        let loaded = crate::app::LoadedWorld::load(None, 7, 1, 1).expect("starter world");
        let (id, record) = loaded.world.bibites.iter().next().expect("one bibite");
        let camera = Camera::new(record.pos, 10.0, Vec2::new(320.0, 320.0));
        let mut scene = Scene::new();
        build_scene(
            &mut scene,
            &loaded.world,
            &camera,
            Some(id),
            &SceneOptions::default(),
            None,
            None,
        );
        let cone = scene
            .primitives
            .iter()
            .position(|primitive| matches!(primitive, Primitive::Sector { .. }))
            .expect("the cone's fill");
        let body = scene
            .primitives
            .iter()
            .position(|primitive| matches!(primitive, Primitive::Capsule { .. }))
            .expect("the bibite");
        assert!(cone < body, "cone at {cone}, bibite at {body}");
        // Nothing of the cone comes after the bibite: its white border lines are under it too.
        let last_border = scene.primitives.iter().rposition(
            |primitive| matches!(primitive, Primitive::Line { color, .. } if *color == [255, 255, 255, 255]),
        );
        assert!(
            last_border.is_some_and(|line| line < body),
            "{last_border:?} {body}"
        );
    }

    #[test]
    fn the_capsule_stand_in_is_the_games_light_grey_for_the_basic_bibite() {
        // With no game art the body is a capsule: its colour is `GenesToColor` of the genes (230 grey), not the raw 0.1 pigment
        // (which was 26, lifted to 90).
        let loaded = crate::app::LoadedWorld::load(None, 7, 1, 1).expect("starter world");
        let (id, record) = loaded.world.bibites.iter().next().expect("one bibite");
        let color = skin_color(id, record);
        assert_eq!(color[..3], [230, 230, 230], "{color:?}");
        assert_eq!(pigment_rgb([0.1, 0.1, 0.1]), [230, 230, 230]);
        assert_eq!(pigment_rgb([1.0, 0.0, 0.0]), [255, 0, 0]);
    }

    /// The dot a far-away bibite becomes is the colour it is made of: close to the mean of the pixels of its body drawn in full (the population's colours are
    /// what a zoomed-out view is for).
    #[test]
    fn a_bibites_dot_has_the_colour_of_its_body_drawn_in_full() {
        use bibites_core::genome::Gene;
        let Some(assets) = local_game_assets() else {
            eprintln!("skipping: extracted game art absent");
            return;
        };
        let loaded = crate::app::LoadedWorld::load(None, 7, 1, 1).expect("starter world");
        let (_, record) = loaded.world.bibites.iter().next().expect("a bibite");
        for (r, g, b) in [(1.0, 0.1, 0.1), (0.1, 0.8, 0.2), (0.2, 0.3, 1.0), (0.9, 0.8, 0.1), (0.5, 0.5, 0.5)] {
            let mut record = record.clone();
            record.genome.set_gene(Gene::ColorR, r);
            record.genome.set_gene(Gene::ColorG, g);
            record.genome.set_gene(Gene::ColorB, b);
            record.pos = Vec2::ZERO;
            record.rot = 0.0;
            let dot = crate::sprites::bibite_dot_colour(&assets, &record, None).expect("a dot colour");
            let mut scene = Scene::new();
            scene.background = [0, 0, 0, 0];
            scene.skin = SkinParams::from(&assets.parameters);
            for sprite in crate::sprites::bibite_body_sprite(&assets, &record, None) {
                scene.push(sprite_primitive(sprite));
            }
            let camera = Camera::new(Vec2::ZERO, 8.0, Vec2::new(256.0, 256.0));
            let image = render_scene(&scene, &camera, 256, 256, 1, &assets.atlases);
            let (mut sum, mut count) = ([0.0_f64; 3], 0.0_f64);
            for pixel in image.rgba.chunks_exact(4).filter(|pixel| pixel[3] >= 128) {
                for channel in 0..3 {
                    sum[channel] += f64::from(pixel[channel]);
                }
                count += 1.0;
            }
            assert!(count > 100.0, "the body was drawn: {count} pixels");
            let mean = [0, 1, 2].map(|channel| (sum[channel] / count) as i32);
            for channel in 0..3 {
                assert!((mean[channel] - i32::from(dot[channel])).abs() <= 30, "colour ({r}, {g}, {b}): the dot is {dot:?}, the body's mean pixel {mean:?}");
            }
        }
    }

    /// Renders a rad, a rainbow, a producer and a plain bibite side by side (`SKIN_DUMP=<dir>` writes the picture; always checks that they differ).
    #[test]
    fn rad_rainbow_and_producer_bibites_draw_their_patterns() {
        let Some(assets) = local_game_assets() else {
            eprintln!("skipping: extracted game art absent");
            return;
        };
        let loaded = crate::app::LoadedWorld::load(None, 7, 1, 1).expect("starter world");
        let (_, record) = loaded.world.bibites.iter().next().expect("one bibite");
        let mut scene = Scene::new();
        scene.skin = SkinParams::from(&assets.parameters);
        scene.skin.time = 3.0;
        let mut pictures = Vec::new();
        for (index, flags) in [(false, false, false), (true, false, false), (false, true, false), (false, false, true)].into_iter().enumerate() {
            let mut record = record.clone();
            record.pos = Vec2::new(index as f32 * 10.0, 0.0);
            record.meta.is_rad = flags.0;
            record.meta.is_rain = flags.1;
            record.meta.is_source = flags.2;
            let before = scene.primitives.len();
            for sprite in crate::sprites::bibite_sprites(&assets, &record, None) {
                scene.push(sprite_primitive(sprite));
            }
            pictures.push(scene.primitives.len() - before);
        }
        assert!(pictures.iter().all(|count| *count >= 3));
        let camera = Camera::new(Vec2::new(15.0, 0.0), 25.0, Vec2::new(1000.0, 250.0));
        let image = render_scene(&scene, &camera, 1000, 250, 1, &assets.atlases);
        // Four columns: the plain bibite and the three with a pattern must not look alike.
        let column = |k: usize| -> Vec<u8> {
            image.rgba.chunks_exact(4 * 1000).flat_map(|row| row[k * 250 * 4..(k + 1) * 250 * 4].to_vec()).collect()
        };
        for k in 1..4 {
            assert_ne!(column(0), column(k), "bibite {k} looks like the plain one");
        }
        if let Ok(dir) = std::env::var("SKIN_DUMP") {
            std::fs::write(std::path::Path::new(&dir).join("skin_patterns.png"), image.to_png_bytes().expect("png")).expect("write");
        }
    }

    #[test]
    fn the_basic_bibite_has_the_grey_body_and_cyan_eyes_of_the_games_capture() {
        let Some(assets) = local_game_assets() else {
            eprintln!("skipping: extracted game art absent");
            return;
        };
        // One adult starter bibite: the captured basic bibite, pigment genes 0.1 / 0.1 / 0.1.
        let loaded = crate::app::LoadedWorld::load(None, 7, 1, 1).expect("starter world");
        let (_, record) = loaded.world.bibites.iter().next().expect("one bibite");
        let sprites = crate::sprites::bibite_sprites(&assets, record, None);
        assert!(sprites.len() >= 3, "body, eyes and mouth at least");
        // The tints themselves: a light grey skin (230) and cyan eyes (the hue offset by the EyeOffset gene).
        let skin = crate::sprites::skin_tint(record).map(|c| (c * 255.0).round() as i32);
        assert_eq!(skin, [230, 230, 230]);
        let eye = crate::sprites::eye_tint(record).map(|c| (c * 255.0).round() as i32);
        assert_eq!(eye, [0, 255, 255]);

        let mut scene = Scene::new();
        scene.skin = SkinParams::from(&assets.parameters);
        for sprite in sprites {
            scene.push(sprite_primitive(sprite));
        }
        let camera = Camera::new(record.pos, 10.0, Vec2::new(320.0, 320.0));
        let image = render_scene(&scene, &camera, 320, 320, 1, &assets.atlases);
        let mut levels = std::collections::BTreeMap::new();
        let mut cyan = 0;
        let mut blue = 0;
        for pixel in image.rgba.chunks_exact(4) {
            if pixel == BACKGROUND {
                continue;
            }
            let (r, g, b) = (
                i32::from(pixel[0]),
                i32::from(pixel[1]),
                i32::from(pixel[2]),
            );
            if (r - g).abs() <= 1 && (g - b).abs() <= 1 {
                *levels.entry(r).or_insert(0_u32) += 1;
            }
            if r < g - 80 && (g - b).abs() <= 30 {
                cyan += 1;
            }
            // The art multiplied with a grey would be this: blue (b well above g).
            if b > g + 40 && b > r + 40 {
                blue += 1;
            }
        }
        assert!(cyan > 30, "the eyes are cyan: {cyan} cyan pixels");
        assert_eq!(blue, 0, "no blue pixel: {blue}");
        // The body's grey levels, the game's (`assets/game/reference/default/bibite_closeup.png`: 100 123 156 172 177 199 200
        // 214): the darkest is 100, not the art's value 142, because the shader pulls the art's saturation into the value.
        let common: Vec<i32> = levels
            .iter()
            .filter(|(_, count)| **count >= 60)
            .map(|(level, _)| *level)
            .collect();
        let game = [100, 123, 156, 172, 177, 199, 200, 214];
        assert!(
            common
                .iter()
                .all(|level| game.iter().any(|g| (g - level).abs() <= 1)),
            "grey levels {common:?} are not the game's {game:?}"
        );
        assert!(
            common.iter().any(|level| (level - 100).abs() <= 1),
            "the darkest body level is 100: {common:?}"
        );
        assert!(common.len() >= 4, "{common:?}");
    }

    /// Where the rasteriser (and the window's `draw_texture_ex`) puts the four corners of a composed sprite: its rectangle runs from `-pivot` to
    /// `1 - pivot` of its size round the anchor, the pivot counted from the TOP of the picture, and it is turned by the rotation (positive is
    /// counter-clockwise).
    fn drawn_corners(sprite: &crate::sprites::SpriteInstance) -> [Vec2; 4] {
        let (sin, cos) = sprite.rotation.sin_cos();
        [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(u, v)| {
            let across = (u - sprite.pivot.x) * sprite.size.x;
            let down = (v - sprite.pivot.y) * sprite.size.y;
            sprite.anchor + Vec2::new(cos * across + sin * down, sin * across - cos * down)
        })
    }

    /// The four size tiers' arms against the game's own geometry (large bibites had the left arm hovering off the body, huge ones had it on top of it:
    /// the old code moved the left arm by the normal tier's 3 units, measured once, for every tier).
    ///
    /// The prefab hangs `RightArm` (`flipY`) at x = +4.5 and `Left Arm` (no flip) at x = -4.5, both turned 90 degrees, with the sprite's pivot at its bottom
    /// right corner (Unity's, y up), and Unity turns a flipped sprite about its pivot. That, with each tier's own pixels per unit (`pixelWidth / 9`), is all
    /// the game does: the renderer bounds of its live six-layer dump (11 bibites, `assets/game/reference/default_save/bibite_layers.json`) are reproduced
    /// to 0.0001 by it. So each arm is a rectangle `arm width` long behind the joint (y from `-width` to 0) and `arm height` thick, outside the body's
    /// side (|x| from 4.5 to 4.5 + height), the right one the mirror image of the left one, whatever the tier. `ARMS_DUMP=<dir>` writes a picture of the
    /// four tiers.
    #[test]
    fn the_arms_of_every_size_tier_hang_from_the_body_where_the_games_prefab_puts_them() {
        let Some(assets) = local_game_assets() else {
            eprintln!("skipping: extracted game art absent");
            return;
        };
        let loaded = crate::app::LoadedWorld::load(None, 7, 1, 1).expect("starter world");
        let (_, starter) = loaded.world.bibites.iter().next().expect("one bibite");
        assert_eq!(assets.part_sizes.len(), 4, "tiny, normal, large and huge");
        let mut scene = Scene::new();
        scene.skin = SkinParams::from(&assets.parameters);
        // The picture reads tiny, normal, large, huge from left to right, 40 units apart.
        let mut widths: Vec<u32> = assets
            .part_sizes
            .iter()
            .map(|tier| tier.pixel_width)
            .collect();
        widths.sort_unstable();
        let mut arms = Vec::new();
        for (index, tier) in assets.part_sizes.iter().enumerate() {
            // The size the tier is picked by (`ClosestSizeIndex`: `pixelWidth / 9`), as the size gene, the growth and `d1` alike.
            let d1 = tier.pixel_width as f32 / 9.0;
            let slot = widths
                .iter()
                .position(|width| *width == tier.pixel_width)
                .expect("a tier");
            let mut record = starter.clone();
            record.pos = Vec2::new(slot as f32 * 40.0, 0.0);
            record.rot = 0.0;
            record.body.geometry.d1_size = d1;
            record.growth.growth = d1 * d1;
            let size_index = assets.closest_part_size_index(d1);
            assert_eq!(size_index, index, "{} is picked by its own size", tier.name);
            let arm = assets
                .arm_sprite(size_index, record.genome.values()[4])
                .expect("the tier's arm sprite");
            let sprites = crate::sprites::bibite_sprites(&assets, &record, None);
            for sprite in &sprites {
                scene.push(sprite_primitive(*sprite));
            }
            let mut pair: Vec<_> = sprites
                .into_iter()
                .filter(|sprite| sprite.sprite == arm)
                .collect();
            assert_eq!(pair.len(), 2, "{}: two arms", tier.name);
            pair.sort_by(|a, b| a.anchor.x.total_cmp(&b.anchor.x));
            arms.push((tier.name.clone(), d1, record.pos, arm, pair));
        }
        if let Ok(dir) = std::env::var("ARMS_DUMP") {
            // 12 pixels per unit (an art pixel is one unit at these sizes); the joints are at y = 0 and the arms hang behind them.
            let camera = Camera::new(Vec2::new(60.0, -5.0), 12.0, Vec2::new(1900.0, 560.0));
            let image = render_scene(&scene, &camera, 1900, 560, 1, &assets.atlases);
            std::fs::write(
                std::path::Path::new(&dir).join("arms_by_tier.png"),
                image.to_png_bytes().expect("png"),
            )
            .expect("write");
        }
        let half_width = 4.5;
        // Every mismatch is collected, so a failure shows all the tiers at once.
        let mut wrong = Vec::new();
        for (name, d1, at, arm, pair) in &arms {
            let (length, thickness) = (arm.world_size().x, arm.world_size().y);
            let (left, right) = (&pair[0], &pair[1]);
            let boxes = [
                ("left", left, -(half_width + thickness), -half_width),
                ("right", right, half_width, half_width + thickness),
            ];
            for (side, sprite, from, to) in boxes {
                let corners = drawn_corners(sprite);
                let (low_x, high_x) =
                    corners.iter().fold((f32::MAX, f32::MIN), |(low, high), c| {
                        (low.min(c.x - at.x), high.max(c.x - at.x))
                    });
                let (low_y, high_y) =
                    corners.iter().fold((f32::MAX, f32::MIN), |(low, high), c| {
                        (low.min(c.y - at.y), high.max(c.y - at.y))
                    });
                let (want_low_x, want_high_x, want_low_y, want_high_y) =
                    (from * d1, to * d1, -length * d1, 0.0);
                let tolerance = 1e-3 * d1;
                for (edge, got, want) in [
                    ("x low", low_x, want_low_x),
                    ("x high", high_x, want_high_x),
                    ("back", low_y, want_low_y),
                    ("front", high_y, want_high_y),
                ] {
                    if (got - want).abs() > tolerance {
                        wrong.push(format!("{name} {side} arm, {edge}: drawn at {got:.4}, the game's at {want:.4} (arm {length:.3} x {thickness:.3}, d1 {d1:.3})"));
                    }
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "arms off the game's geometry:\n{}",
            wrong.join("\n")
        );
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::{Raster, Rgba, Vec2, clamp_ortho_size, ortho_to_zoom, sample_bilinear, smooth_zoom, tinted, wheel_zoom_ratio, zoom_to_ortho};

    #[test]
    fn wheel_ratio_matches_game() {
        assert!((wheel_zoom_ratio(1.0, 1.0) - 0.1).abs() < 1e-6);
        assert!((wheel_zoom_ratio(-2.0, 1.0) + 0.2).abs() < 1e-6);
        // Game: camSize -= camSize * ratio; one notch at scrollSpeed 1 => 90%.
        let ortho = 120.0;
        assert!((ortho - ortho * wheel_zoom_ratio(1.0, 1.0) - 108.0).abs() < 1e-5);
    }

    #[test]
    fn ortho_clamp_matches_game() {
        assert!((clamp_ortho_size(1.0, 500.0) - 5.0).abs() < 1e-6);
        assert!((clamp_ortho_size(10_000.0, 500.0) - 1_000.0).abs() < 1e-3);
        assert!((clamp_ortho_size(10_000.0, 100.0) - 250.0).abs() < 1e-3);
    }

    #[test]
    fn smoothing_converges_and_is_frame_rate_independent() {
        let mut a = 1.0;
        for _ in 0..120 {
            a = smooth_zoom(a, 2.0, 1.0 / 120.0, 0.12);
        }
        let mut b = 1.0;
        for _ in 0..60 {
            b = smooth_zoom(b, 2.0, 1.0 / 60.0, 0.12);
        }
        assert!((a - 2.0).abs() < 2e-3, "120 Hz target reached: {a}");
        assert!((b - 2.0).abs() < 2e-3, "60 Hz target reached: {b}");
        assert!((a - b).abs() < 5e-3, "frame-rate independence: {a} vs {b}");
        assert!(smooth_zoom(1.0, 2.0, 0.0, 0.12) == 1.0);
    }

    #[test]
    fn ortho_zoom_round_trip() {
        let z = ortho_to_zoom(12.0, 720.0);
        assert!((zoom_to_ortho(z, 720.0) - 12.0).abs() < 1e-4);
    }

    /// A raster of 48 x 36 pixels with a few translucent and opaque pixels already on it, so that a draw blends over something.
    fn busy_raster() -> Raster<'static> {
        let mut raster = Raster::new(48, 36, 1.0, &[]);
        raster.clear_transparent();
        let mut seed = 12_345_u32;
        for pixel in &mut raster.pixels {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *pixel = match seed >> 28 {
                0..=5 => [0, 0, 0, 0],
                6..=9 => [(seed >> 8) as u8, (seed >> 16) as u8, (seed >> 4) as u8, 255],
                _ => [(seed >> 12) as u8, (seed >> 20) as u8, (seed >> 2) as u8, (seed >> 24) as u8 | 1],
            };
        }
        raster
    }

    /// What the per-pixel draws did before they were worked out per column and row: every pixel of the box through [`Raster::blend`].
    #[allow(clippy::too_many_arguments)]
    fn blit_one_by_one(raster: &mut Raster<'_>, texels: &[u8], width: u32, us: &[u32], vs: &[u32], xa: i64, ya: i64, tint: Rgba) {
        for (row, &v) in vs.iter().enumerate() {
            for (col, &u) in us.iter().enumerate() {
                let offset = ((v * width + u) * 4) as usize;
                let Some(pixel) = texels.get(offset..offset + 4) else {
                    continue;
                };
                let alpha = (u32::from(pixel[3]) * u32::from(tint[3]) / 255) as u8;
                if alpha == 0 {
                    continue;
                }
                let color = [
                    (u32::from(pixel[0]) * u32::from(tint[0]) / 255) as u8,
                    (u32::from(pixel[1]) * u32::from(tint[1]) / 255) as u8,
                    (u32::from(pixel[2]) * u32::from(tint[2]) / 255) as u8,
                    alpha,
                ];
                raster.blend(xa + col as i64, ya + row as i64, color);
            }
        }
    }

    #[test]
    fn a_flat_fill_leaves_the_pixels_the_per_pixel_blend_leaves() {
        // Opaque, translucent and clear colours, boxes partly outside the image, with and without a clip.
        for clip in [None, Some([5_i64, 4, 40, 30])] {
            for color in [[10, 20, 30, 255], [200, 100, 50, 128], [90, 90, 90, 1], [7, 7, 7, 0]] {
                for (x0, y0, x1, y1) in [(3_i64, 2_i64, 20_i64, 15_i64), (-7, -5, 30, 12), (30, 20, 60, 50), (10, 10, 10, 20), (0, 0, 48, 36)] {
                    let (mut fast, mut slow) = (busy_raster(), busy_raster());
                    fast.clip = clip;
                    slow.clip = clip;
                    fast.fill_rect_px(x0, y0, x1, y1, color);
                    for y in y0.max(0)..y1.min(36) {
                        for x in x0.max(0)..x1.min(48) {
                            slow.blend(x, y, color);
                        }
                    }
                    assert_eq!(fast.pixels, slow.pixels, "colour {color:?}, box {x0},{y0}..{x1},{y1}, clip {clip:?}");
                }
            }
        }
    }

    #[test]
    fn texel_draws_worked_out_per_column_leave_the_pixels_the_per_pixel_blend_leaves() {
        // A texture of one opaque colour (a panel's middle), of one translucent colour, of one clear colour and a busy one; the columns and rows
        // stretch it unevenly (some texels twice, some left out) over a box that is partly outside the image, with and without a clip and a tint.
        let (tw, th) = (8_u32, 6_u32);
        let busy: Vec<u8> = {
            let mut seed = 99_u32;
            (0..tw * th * 4)
                .map(|_| {
                    seed = seed.wrapping_mul(22_695_477).wrapping_add(1);
                    (seed >> 16) as u8
                })
                .collect()
        };
        let flat = |color: [u8; 4]| -> Vec<u8> { (0..tw * th).flat_map(|_| color).collect() };
        let textures = [flat([40, 40, 44, 255]), flat([200, 100, 50, 128]), flat([9, 9, 9, 0]), busy];
        let us: Vec<u32> = (0..21).map(|x| (x * tw / 21).min(tw - 1)).collect();
        let vs: Vec<u32> = (0..13).map(|y| (y * th / 13).min(th - 1)).collect();
        for clip in [None, Some([8_i64, 6, 30, 20])] {
            for tint in [[255, 255, 255, 255], [200, 150, 255, 128], [255, 255, 255, 0]] {
                for (index, texels) in textures.iter().enumerate() {
                    for (xa, ya) in [(4_i64, 3_i64), (35, 25), (0, 0)] {
                        let (mut fast, mut slow) = (busy_raster(), busy_raster());
                        fast.clip = clip;
                        slow.clip = clip;
                        // The box the draw covers, as the draws clip it first.
                        let (ax, ay, bx, by) = fast.visible(xa, ya, xa + us.len() as i64, ya + vs.len() as i64);
                        if ax >= bx || ay >= by {
                            continue;
                        }
                        let columns = &us[(ax - xa) as usize..(bx - xa) as usize];
                        let rows = &vs[(ay - ya) as usize..(by - ya) as usize];
                        fast.blit_texels(texels, tw, columns, rows, ax, ay, tint);
                        blit_one_by_one(&mut slow, texels, tw, columns, rows, ax, ay, tint);
                        assert_eq!(fast.pixels, slow.pixels, "texture {index}, tint {tint:?}, at {xa},{ya}, clip {clip:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_filtered_draw_keeps_the_pixels_the_per_pixel_filter_gives() {
        // A 12 x 10 sprite with a middle block of one colour (columns 3..9, rows 3..7) in a border of other colours, like a nine-slice's.
        let (tw, th) = (12_u32, 10_u32);
        let mut texels = Vec::new();
        for y in 0..th {
            for x in 0..tw {
                let inside = (3..9).contains(&x) && (3..7).contains(&y);
                texels.extend_from_slice(&if inside { [40, 40, 44, 255] } else { [(x * 20) as u8, (y * 25) as u8, 200, 255 - (x * 7) as u8] });
            }
        }
        let source = crate::ui::UiTexture { width: tw, height: th, rgba: texels };
        // The regions a nine-slice draws: the middle block, a border strip, the whole sprite.
        for region in [[3.0_f32, 3.0, 6.0, 4.0], [0.0, 3.0, 3.0, 4.0], [0.0, 0.0, 12.0, 10.0]] {
            for tint in [[255, 255, 255, 255], [200, 150, 255, 128]] {
                for clip in [None, Some([6_i64, 5, 40, 28])] {
                    for (pos, size) in [
                        (Vec2::new(4.3, 2.7), Vec2::new(30.0, 22.0)),
                        (Vec2::new(-5.0, -3.0), Vec2::new(60.0, 50.0)),
                        (Vec2::new(20.0, 10.0), Vec2::new(13.0, 11.0)),
                    ] {
                        let (mut fast, mut slow) = (busy_raster(), busy_raster());
                        fast.clip = clip;
                        slow.clip = clip;
                        let (ax, ay, bx, by) = fast.visible(pos.x.floor() as i64, pos.y.floor() as i64, (pos.x + size.x).ceil() as i64, (pos.y + size.y).ceil() as i64);
                        if ax >= bx || ay >= by {
                            continue;
                        }
                        let us: Vec<f32> = (ax..bx).map(|x| region[0] + ((x as f32 + 0.5 - pos.x) / size.x).clamp(0.0, 0.999) * region[2]).collect();
                        let vs: Vec<f32> = (ay..by).map(|y| region[1] + ((y as f32 + 0.5 - pos.y) / size.y).clamp(0.0, 0.999) * region[3]).collect();
                        fast.blit_filtered(&source, region, &us, &vs, ax, ay, tint);
                        for (row, &v) in vs.iter().enumerate() {
                            for (col, &u) in us.iter().enumerate() {
                                let color = tinted(sample_bilinear(&source, u, v), tint);
                                if color[3] != 0 {
                                    slow.blend(ax + col as i64, ay + row as i64, color);
                                }
                            }
                        }
                        assert_eq!(fast.pixels, slow.pixels, "region {region:?}, tint {tint:?}, clip {clip:?}, box {pos:?} {size:?}");
                    }
                }
            }
        }
    }

    /// The blend as it was written before the opaque-destination shortcut (the general formula for every pixel).
    fn blend_by_the_general_formula(dst: &mut Rgba, src: Rgba) {
        let src_alpha = u32::from(src[3]);
        if src_alpha == 0 {
            return;
        }
        let dst_alpha = u32::from(dst[3]);
        if dst_alpha == 0 || (src_alpha >= 255 && dst_alpha >= 255) {
            *dst = src;
            return;
        }
        let inv = 255 - src_alpha;
        let out_alpha = src_alpha + dst_alpha * inv / 255;
        if out_alpha == 0 {
            *dst = [0, 0, 0, 0];
            return;
        }
        for channel in 0..3 {
            let numerator = u32::from(src[channel]) * src_alpha * 255 + u32::from(dst[channel]) * dst_alpha * inv;
            dst[channel] = (numerator / (255 * out_alpha)).min(255) as u8;
        }
        dst[3] = out_alpha as u8;
    }

    #[test]
    fn the_blend_over_an_opaque_pixel_is_the_general_formulas_result_for_every_alpha_and_colour() {
        use super::blend_over;
        let channels = [0_u8, 1, 2, 63, 127, 128, 200, 254, 255];
        for src_alpha in 0..=255_u8 {
            for dst_alpha in [255_u8, 254, 200, 128, 1, 0] {
                for &sr in &channels {
                    for &dr in &channels {
                        let src = [sr, sr / 2 + 3, 255 - sr, src_alpha];
                        let (mut fast, mut general) = ([dr, 255 - dr, dr / 3, dst_alpha], [dr, 255 - dr, dr / 3, dst_alpha]);
                        blend_over(&mut fast, src);
                        blend_by_the_general_formula(&mut general, src);
                        assert_eq!(fast, general, "src {src:?} over {:?}", [dr, 255 - dr, dr / 3, dst_alpha]);
                    }
                }
            }
        }
    }

    #[test]
    fn a_picture_with_one_sample_a_pixel_comes_out_as_it_is() {
        let raster = busy_raster();
        let image = raster.finish(48, 36, 1);
        assert_eq!((image.width, image.height, image.rgba.len()), (48, 36, 48 * 36 * 4));
        assert_eq!(&image.rgba[..4], &raster.pixels[0]);
        assert_eq!(&image.rgba[(48 * 36 - 1) * 4..], &raster.pixels[48 * 36 - 1]);
        // Two samples each way average four pixels.
        let doubled = Raster::new(8, 8, 2.0, &[]);
        assert_eq!(doubled.finish(4, 4, 2).rgba.len(), 4 * 4 * 4);
    }
}
