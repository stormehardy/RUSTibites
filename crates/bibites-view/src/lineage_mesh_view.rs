//! Draws the lineage mesh view (`LineageMeshPanel`, `LineageMesh`, `UIMeshEventHandler`): the species as coloured shapes in
//! the graph's viewport, a navy disc with the species' picture and name in the big ones, the selected one outlined in
//! yellow, and the tooltip of the shape under the pointer.
//!
//! No renderer has polygons, so the shapes are painted into pictures: the graph is cut into tiles of 512 px (at the
//! current zoom and UI scale) that are painted when they first show and kept, so scrolling only draws the pictures it has,
//! and zooming or selecting another species paints the visible ones again. A shape's rows are exactly the game's mesh
//! (the left and right ends of each rung, interpolated between them). The colours are the pixel program of `Lineage Mesh Shader`
//! (sharedassets2 path 86, read from its DXBC): the species' colour (the `GenesToColor` of its colour genes) fades into its
//! parent's between `1 - _BlendHeight` and `1 - _BlendHeight / 2` of the shape's height and is the parent's above that, and the
//! border is a gradient from the border colour at the shape's LEFT edge to nothing `5` graph units inside (`_BorderWidth` 0.5 of
//! the 10 the mesh insets its border by); the right edge has none. The selected species (`FocusMesh`) is outlined in yellow on both
//! edges, `10` units wide, and its border does not fade with the colour.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bibites_core::ids::SpeciesId;
use bibites_core::math::Vec2;

use crate::app::App;
use crate::lineage_graph::{HIT_PART, intersect};
use crate::lineage_mesh::{Mesh, MeshLayout, PREVIEWER, Preview};
use crate::lineage_tree::SIDE_PADDING;
use crate::lineage_view::View;
use crate::species_panel::Drawer;
use crate::tooltip::{self, Tip, TipRect};
use crate::ui::{Rgba, UiAction, UiDraw, UiFrame, UiTexture};

const WHITE: Rgba = [255, 255, 255, 255];
/// `BodyPreview`'s `Image` colour (0, 0, 0.106): the disc behind the species' picture.
const DISC: [f32; 3] = [0.0, 0.0, 0.106];
/// The outline of a shape (`Color.black`).
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];
/// The outline of the selected shape (`Color.yellow`).
const YELLOW: [f32; 3] = [1.0, 1.0, 0.0];
/// How wide the outline is in graph units: `_BorderWidth` (5 / 10 and 10 / 10) of the 10 the mesh insets its border by.
const BORDER: f32 = 5.0;
const BORDER_FOCUSED: f32 = 10.0;
/// The species' name under its picture: Roboto-Bold up to 30.7 (auto-sized) in a 263.68 x 36 label 129.8 under the disc's middle.
const LABEL_PX: f32 = 30.7;
const LABEL_WIDTH: f32 = 263.68;
const LABEL_DROP: f32 = 129.8;

/// The side of a tile, px.
pub const TILE: usize = 512;
/// How many tiles are kept.
const CAPACITY: usize = 64;

/// Which picture a tile is: the layout it was painted for, the scale (graph units to px), the selected species and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileKey {
    /// The layout (`MeshLayout::serial`).
    pub serial: u64,
    /// The scale's bits.
    pub k: u32,
    /// The selected species' id (0: none).
    pub selected: u32,
    /// The tile's column.
    pub i: i32,
    /// The tile's row (from the content's top).
    pub j: i32,
}

/// Linear interpolation of colours.
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// The overlap of two intervals.
fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// Lays a colour with this coverage over a pixel (straight alpha).
fn blend(pixel: &mut [u8], colour: [f32; 3], alpha: f32) {
    let below = f32::from(pixel[3]) / 255.0;
    let out = alpha + below * (1.0 - alpha);
    if out <= 0.0 {
        return;
    }
    for channel in 0..3 {
        let value = (colour[channel] * 255.0 * alpha
            + f32::from(pixel[channel]) * below * (1.0 - alpha))
            / out;
        pixel[channel] = value.round().clamp(0.0, 255.0) as u8;
    }
    pixel[3] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Paints one row of a disc: `left` to `right` in `fill`. Pixels are covered by the share of them the span holds.
fn paint_span(row: &mut [u8], (left, right): (f32, f32), fill: [f32; 3]) {
    let width = row.len() / 4;
    let first = left.max(0.0).floor() as usize;
    let last = (right.min(width as f32).ceil().max(0.0) as usize).min(width);
    for p in first..last {
        let cover = overlap(left, right, p as f32, p as f32 + 1.0);
        if cover > 0.0 {
            blend(&mut row[4 * p..4 * p + 4], fill, cover.min(1.0));
        }
    }
}

/// What one row of a shape is made of (`Lineage Mesh Shader`'s pixel program, one row of the mesh's triangles).
struct ShapeRow {
    /// The row's left and right ends, px.
    ends: (f32, f32),
    /// How many px across the row make a graph unit along the edge's normal (`k * sqrt(1 + slope^2)`), at the left and at the right edge.
    across: (f32, f32),
    /// The colour from the shape's own (1) to its parent's (0): `g` of the program.
    own: f32,
    /// The shape's colour and its parent's.
    colours: ([f32; 3], [f32; 3]),
    /// The border colour (black, or yellow for the selected species).
    edge: [f32; 3],
    /// The selected species: both edges have the border, 10 units wide, whatever the fade.
    focused: bool,
}

/// `Lineage Mesh Shader` per pixel: `out = lerp(border, lerp(parent, own, g), 1 - x * t)`, with `g` the fade over the shape's
/// height, `t` the border gradient (`1 - d / 5` of the way in from the left edge, `1 - d / 10` from either edge when focused) and
/// `x` = `g` (1 when focused): the border fades away with the colour.
fn paint_shape_row(row: &mut [u8], shape: &ShapeRow) {
    let (left, right) = shape.ends;
    let width = row.len() / 4;
    let first = left.max(0.0).floor() as usize;
    let last = (right.min(width as f32).ceil().max(0.0) as usize).min(width);
    let band = if shape.focused { BORDER_FOCUSED } else { BORDER };
    let colour = mix(shape.colours.1, shape.colours.0, shape.own);
    let x = if shape.focused { 1.0 } else { shape.own };
    // The part of the row that is all the shape's own colour (pixels the shape covers completely, clear of the border's gradient) is written as it
    // is: it is most of every tile (a tile is 262 144 pixels, and the blend below does a handful of divisions each).
    let clear_left = left + band * shape.across.0 + 1.0;
    let clear_right = if shape.focused { right - band * shape.across.1 - 1.0 } else { right };
    let inner_start = (clear_left.max(0.0).ceil() as usize).clamp(first, last);
    let inner_end = (clear_right.max(0.0).floor() as usize).clamp(inner_start, last);
    if inner_end > inner_start {
        let filled = [
            (colour[0] * 255.0).round().clamp(0.0, 255.0) as u8,
            (colour[1] * 255.0).round().clamp(0.0, 255.0) as u8,
            (colour[2] * 255.0).round().clamp(0.0, 255.0) as u8,
            255,
        ];
        for pixel in row[4 * inner_start..4 * inner_end].chunks_exact_mut(4) {
            pixel.copy_from_slice(&filled);
        }
    }
    for p in (first..inner_start).chain(inner_end..last) {
        let (p0, p1) = (p as f32, p as f32 + 1.0);
        let cover = overlap(left, right, p0, p1);
        if cover <= 0.0 {
            continue;
        }
        // The middle of what the shape holds of the pixel.
        let at = (p0.max(left) + p1.min(right)) / 2.0;
        let from_left = (at - left) / shape.across.0.max(1e-3);
        let mut t = (1.0 - from_left / band).clamp(0.0, 1.0);
        if shape.focused {
            let from_right = (right - at) / shape.across.1.max(1e-3);
            t = t.max((1.0 - from_right / band).clamp(0.0, 1.0));
        }
        let painted = mix(shape.edge, colour, (1.0 - x * t).clamp(0.0, 1.0));
        blend(&mut row[4 * p..4 * p + 4], painted, cover.min(1.0));
    }
}

/// Paints the shape of `mesh` and its disc into a tile's rows. `k` is px per graph unit, `height` the graph's height, and
/// the tile starts at (`x0`, `y0`) px of the content (y down from its top).
fn paint_mesh(
    pixels: &mut [u8],
    mesh: &Mesh,
    focused: bool,
    k: f32,
    height: f32,
    (x0, y0): (f32, f32),
) {
    // A shape with no height has nothing to paint (and its gradient would divide by it).
    if mesh.size[1].partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return;
    }
    let top_row = (height - mesh.top_height()) * k - y0;
    let bottom_row = (height - mesh.bottom_height()) * k - y0;
    let rows = (pixels.len() / (4 * TILE)) as f32;
    let first = top_row.floor().max(0.0) as usize;
    let last = (bottom_row.ceil().min(rows).max(0.0)) as usize;
    let edge = if focused { YELLOW } else { BLACK };
    let blend_part = mesh.blend();
    for row in first..last {
        // The graph height at the middle of the pixel row.
        let h = height - (y0 + row as f32 + 0.5) / k;
        let Some(profile) = mesh.profile_at(h) else {
            continue;
        };
        let left = (SIDE_PADDING + profile.left) * k - x0;
        let right = (SIDE_PADDING + profile.right) * k - x0;
        if right <= 0.0 || left >= TILE as f32 || right <= left {
            continue;
        }
        let up = ((h - mesh.bottom_height()) / mesh.size[1]).clamp(0.0, 1.0);
        // `g`: 1 up to `1 - blend`, down to 0 at `1 - blend / 2`, 0 above (a shape without a parent has `blend` 0 and never fades).
        let own = if blend_part > 0.0 {
            (1.0 - (up - (1.0 - blend_part)) / (0.5 * blend_part)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let line = &mut pixels[row * TILE * 4..(row + 1) * TILE * 4];
        paint_shape_row(
            line,
            &ShapeRow {
                ends: (left, right),
                // A slanted edge is wider across the row for the same distance along its normal.
                across: (
                    (1.0 + profile.left_slope * profile.left_slope).sqrt() * k,
                    (1.0 + profile.right_slope * profile.right_slope).sqrt() * k,
                ),
                own,
                colours: (mesh.colour, mesh.parent_colour),
                edge,
                focused,
            },
        );
    }
    // The disc behind the picture, over the shape and its outline.
    let Some(preview) = mesh.preview else {
        return;
    };
    let centre = (
        (SIDE_PADDING + mesh.origin[0] + preview.centre[0]) * k - x0,
        (height - (mesh.origin[1] + preview.centre[1])) * k - y0,
    );
    let radius = preview.size / 2.0 * k;
    let first = (centre.1 - radius).floor().max(0.0) as usize;
    let last = ((centre.1 + radius).ceil().min(rows).max(0.0)) as usize;
    for row in first..last {
        let dy = row as f32 + 0.5 - centre.1;
        let chord = (radius * radius - dy * dy).max(0.0).sqrt();
        let line = &mut pixels[row * TILE * 4..(row + 1) * TILE * 4];
        let span = (centre.0 - chord, centre.0 + chord);
        paint_span(line, span, DISC);
    }
}

/// The picture of one tile of the graph: RGBA rows, transparent where no shape is. `k` is px per graph unit, the tile is the
/// `i`-th from the left and the `j`-th from the top.
#[must_use]
pub fn rasterise_tile(
    layout: &MeshLayout,
    k: f32,
    selected: Option<SpeciesId>,
    i: i32,
    j: i32,
) -> Vec<u8> {
    let mut pixels = vec![0_u8; TILE * TILE * 4];
    let (x0, y0) = (i as f32 * TILE as f32, j as f32 * TILE as f32);
    let height = layout.size[1];
    for mesh in &layout.meshes {
        let top = (height - mesh.top_height()) * k;
        let bottom = (height - mesh.bottom_height()) * k;
        if !mesh.capped || bottom < y0 || top > y0 + TILE as f32 {
            continue;
        }
        let focused = selected == Some(mesh.species);
        paint_mesh(&mut pixels, mesh, focused, k, height, (x0, y0));
    }
    pixels
}

/// A tile, kept: the picture (`None` when nothing is painted in it) and when it was last used.
struct Kept {
    key: TileKey,
    image: Option<Arc<UiTexture>>,
    used: u64,
}

static TILES: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
static CLOCK: AtomicU64 = AtomicU64::new(0);

/// Paints the tiles of `wanted` (column, row) that are not kept yet, in parallel, and keeps them.
fn warm_tiles(layout: &MeshLayout, k: f32, selected: Option<SpeciesId>, wanted: &[(i32, i32)]) {
    let key_of = |i: i32, j: i32| TileKey {
        serial: layout.serial,
        k: k.to_bits(),
        selected: selected.map_or(0, SpeciesId::value),
        i,
        j,
    };
    let missing: Vec<(i32, i32)> = {
        let Ok(kept) = TILES.lock() else {
            return;
        };
        wanted
            .iter()
            .copied()
            .filter(|&(i, j)| !kept.iter().any(|found| found.key == key_of(i, j)))
            .collect()
    };
    if missing.len() < 2 {
        return; // one is painted by its own `tile`
    }
    let workers = std::thread::available_parallelism().map_or(2, usize::from).min(8).min(missing.len());
    let painted: Vec<((i32, i32), Vec<u8>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let missing = &missing;
                scope.spawn(move || {
                    missing
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|&(i, j)| ((i, j), rasterise_tile(layout, k, selected, i, j)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|handle| handle.join().unwrap_or_default()).collect()
    });
    let stamp = CLOCK.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut kept) = TILES.lock() {
        for ((i, j), rgba) in painted {
            let image = rgba.chunks_exact(4).any(|pixel| pixel[3] != 0).then(|| {
                Arc::new(UiTexture { width: TILE as u32, height: TILE as u32, rgba })
            });
            if kept.len() >= CAPACITY
                && let Some(oldest) = kept.iter().enumerate().min_by_key(|(_, found)| found.used).map(|(at, _)| at)
            {
                kept.swap_remove(oldest);
            }
            kept.push(Kept { key: key_of(i, j), image, used: stamp });
        }
    }
}

/// The picture of a tile: the kept one, or a new one (the least recently used goes when too many are kept); `None` for a
/// tile with nothing in it, which is not drawn.
fn tile(
    layout: &MeshLayout,
    k: f32,
    selected: Option<SpeciesId>,
    i: i32,
    j: i32,
) -> Option<Arc<UiTexture>> {
    let key = TileKey {
        serial: layout.serial,
        k: k.to_bits(),
        selected: selected.map_or(0, SpeciesId::value),
        i,
        j,
    };
    let stamp = CLOCK.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut kept) = TILES.lock() {
        if let Some(found) = kept.iter_mut().find(|found| found.key == key) {
            found.used = stamp;
            return found.image.clone();
        }
    }
    let rgba = rasterise_tile(layout, k, selected, i, j);
    let image = rgba.chunks_exact(4).any(|pixel| pixel[3] != 0).then(|| {
        Arc::new(UiTexture {
            width: TILE as u32,
            height: TILE as u32,
            rgba,
        })
    });
    if let Ok(mut kept) = TILES.lock() {
        if kept.len() >= CAPACITY
            && let Some(oldest) = kept
                .iter()
                .enumerate()
                .min_by_key(|(_, found)| found.used)
                .map(|(at, _)| at)
        {
            kept.swap_remove(oldest);
        }
        kept.push(Kept {
            key,
            image: image.clone(),
            used: stamp,
        });
    }
    image
}

/// The tiles kept (for the tests).
#[cfg(test)]
pub(crate) fn tiles_kept() -> usize {
    TILES.lock().map_or(0, |kept| kept.len())
}

impl Drawer<'_> {
    /// The mesh view in the viewport `clip` (screen px): the shapes, the big ones' pictures and names, and the tooltip and
    /// the click of the shape under the pointer.
    pub(crate) fn mesh_view(&self, frame: &mut UiFrame, app: &App, clip: [f32; 4]) {
        let view = &app.lineage_view;
        let unit_of = |preview: Preview| preview.size / PREVIEWER * view.zoom * self.s;
        let wanted: Vec<_> = view
            .mesh
            .meshes
            .iter()
            .filter_map(|mesh| {
                mesh.preview.map(|preview| (mesh.species, crate::preview::MESH, [PREVIEWER, PREVIEWER], unit_of(preview)))
            })
            .collect();
        app.warm_species_pictures(&wanted);
        self.mesh_tiles(frame, view, app.species_info, clip);
        for mesh in &view.mesh.meshes {
            if let Some(preview) = mesh.preview {
                self.mesh_preview(frame, app, view, mesh, preview, clip);
            }
        }
        self.mesh_hover(frame, app, view, clip);
    }

    /// The content's top-left corner in screen px, snapped to the pixel grid so that the tiles meet exactly.
    fn mesh_origin(&self, view: &View) -> (f32, f32) {
        let g = view.geometry();
        let content = view.content_size();
        let k = view.zoom * self.s;
        let x = (g.viewport[0] + g.viewport[2] / 2.0 + view.pos[0] - content[0] / 2.0 * view.zoom)
            * self.s;
        let y = (g.viewport[1] + g.viewport[3] - view.pos[1]) * self.s - content[1] * k;
        (x.round(), y.round())
    }

    /// The tiles that show.
    fn mesh_tiles(
        &self,
        frame: &mut UiFrame,
        view: &View,
        selected: Option<SpeciesId>,
        clip: [f32; 4],
    ) {
        let k = view.zoom * self.s;
        let content = view.content_size();
        let (x0, y0) = self.mesh_origin(view);
        let size = TILE as f32;
        let columns = ((content[0] * k) / size).ceil() as i32;
        let rows = ((content[1] * k) / size).ceil() as i32;
        let first_i = (((clip[0] - x0) / size).floor() as i32).max(0);
        let last_i = (((clip[0] + clip[2] - x0) / size).ceil() as i32).min(columns);
        let first_j = (((clip[1] - y0) / size).floor() as i32).max(0);
        let last_j = (((clip[1] + clip[3] - y0) / size).ceil() as i32).min(rows);
        // The tiles that show and are not kept are painted together, on as many threads as there are cores (a zoom step paints all of them).
        let wanted: Vec<(i32, i32)> = (first_j..last_j)
            .flat_map(|j| (first_i..last_i).map(move |i| (i, j)))
            .filter(|&(i, j)| intersect([x0 + i as f32 * size, y0 + j as f32 * size, size, size], clip).is_some())
            .collect();
        warm_tiles(&view.mesh, k, selected, &wanted);
        for j in first_j..last_j {
            for i in first_i..last_i {
                let rect = [x0 + i as f32 * size, y0 + j as f32 * size, size, size];
                let Some(visible) = intersect(rect, clip) else {
                    continue;
                };
                let Some(image) = tile(&view.mesh, k, selected, i, j) else {
                    continue;
                };
                frame.draws.push(UiDraw::PixelsRegion {
                    texture: image,
                    region: [
                        visible[0] - rect[0],
                        visible[1] - rect[1],
                        visible[2],
                        visible[3],
                    ],
                    pos: Vec2::new(visible[0], visible[1]),
                    size: Vec2::new(visible[2], visible[3]),
                });
            }
        }
    }

    /// A graph point (panel x, height) in screen px.
    fn mesh_point(&self, view: &View, x: f32, height: f32) -> [f32; 2] {
        let at = view.to_screen(SIDE_PADDING + x, height);
        [at[0] * self.s, at[1] * self.s]
    }

    /// The species' picture and name on its disc (`BodyPreview`, scaled to `size / 400`).
    fn mesh_preview(
        &self,
        frame: &mut UiFrame,
        app: &App,
        view: &View,
        mesh: &Mesh,
        preview: Preview,
        clip: [f32; 4],
    ) {
        let k = view.zoom * self.s;
        let centre = self.mesh_point(
            view,
            mesh.origin[0] + preview.centre[0],
            mesh.origin[1] + preview.centre[1],
        );
        let side = preview.size * k;
        let rect = [centre[0] - side / 2.0, centre[1] - side / 2.0, side, side];
        let scale = preview.size / PREVIEWER;
        // The name hangs below the middle; the picture and the name both have to be near the view to matter.
        let reach = [
            rect[0],
            rect[1],
            side,
            side + (LABEL_DROP + 40.0) * scale * k,
        ];
        if intersect(reach, clip).is_none() {
            return;
        }
        // The picture covers the disc: the previewer's 400 canvas units across.
        let unit = scale * k;
        if let Some(texture) = app.species_picture(
            mesh.species,
            crate::preview::MESH,
            [PREVIEWER, PREVIEWER],
            unit,
        ) && let Some(visible) = intersect(rect, clip)
        {
            let (width, height) = (texture.width as f32, texture.height as f32);
            let region = [
                (visible[0] - rect[0]) / rect[2] * width,
                (visible[1] - rect[1]) / rect[3] * height,
                visible[2] / rect[2] * width,
                visible[3] / rect[3] * height,
            ];
            frame.draws.push(UiDraw::PixelsRegion {
                texture,
                region,
                pos: Vec2::new(visible[0], visible[1]),
                size: Vec2::new(visible[2], visible[3]),
            });
        }
        // The name: Roboto-Bold 30.7 in a label 263.68 wide, centred, shrunk to the room it has.
        let name = app
            .lineage_species
            .iter()
            .find(|entry| entry.id == mesh.species)
            .map_or_else(|| mesh.name.clone(), |entry| entry.name.clone());
        if LABEL_PX * unit < 3.0 {
            return;
        }
        // TMP's auto-sizing in the label's own units (`enableAutoSizing`: up to 30.7, in steps of 0.05, until the text - its trailing
        // space does not count - fills the 263.68 wide box: a live capture has 21.85 to 30.7), then scaled with the disc.
        let name = name.trim_end().to_owned();
        let size = crate::text_sdf::fit_size(1.0, LABEL_PX, LABEL_WIDTH, |size| {
            self.measure(&name, size, true)
        }) * unit;
        let width = self.measure(&name, size, true);
        let middle = centre[1] + LABEL_DROP * unit;
        let text_box = [
            centre[0] - width / 2.0,
            middle - size * 1.17 / 2.0,
            width,
            size * 1.17,
        ];
        // The viewport's mask cuts a name at its edge (the game draws what shows of it).
        if intersect(text_box, clip).is_some() {
            frame.draws.push(UiDraw::Clip(Some(clip)));
            frame.bold(name, Vec2::new(text_box[0], text_box[1]), size, WHITE);
            frame.draws.push(UiDraw::Clip(None));
        }
    }

    /// The shape under the pointer takes the click and shows its tooltip: the species' name over the rung nearest to the
    /// pointer (`OnMeshHover` / `OnMeshClick`). Only that shape registers its rectangle, so a click lands on the shape
    /// itself and not on a neighbour whose rectangle holds the same point.
    fn mesh_hover(&self, frame: &mut UiFrame, app: &App, view: &View, clip: [f32; 4]) {
        let cursor = self.cursor;
        if cursor.x < clip[0]
            || cursor.y < clip[1]
            || cursor.x >= clip[0] + clip[2]
            || cursor.y >= clip[1] + clip[3]
        {
            return;
        }
        let [cx, cy] = view.to_content(cursor.x / self.s, cursor.y / self.s);
        let Some(mesh) = view.mesh.mesh_at(cx - SIDE_PADDING, cy) else {
            return;
        };
        let Some(rung) = mesh.nearest_rung(cy) else {
            return;
        };
        // The shape's rectangle, cut to the view.
        let a = self.mesh_point(view, mesh.origin[0], mesh.top_height());
        let b = self.mesh_point(view, mesh.origin[0] + mesh.size[0], mesh.bottom_height());
        let Some(visible) = intersect([a[0], a[1], b[0] - a[0], b[1] - a[1]], clip) else {
            return;
        };
        let (pos, size) = (
            Vec2::new(visible[0], visible[1]),
            Vec2::new(visible[2], visible[3]),
        );
        // One id per shape, so that a press on one and a release on another is not a click (`isClick` is cleared when the
        // pointer leaves the shape).
        frame.hit(
            HIT_PART + mesh.species.value() % 9_999,
            pos,
            size,
            UiAction::SelectLineageNode(mesh.species.value()),
        );
        let name = app
            .lineage_species
            .iter()
            .find(|entry| entry.id == mesh.species)
            .map_or_else(|| mesh.name.clone(), |entry| entry.name.clone());
        frame.tips.push(TipRect {
            pos,
            size,
            tip: Tip {
                // A path starting with '@' names a tooltip whose text changes while it shows (the target stays).
                path: tooltip::intern(format!("@lineage-mesh/{}", mesh.species.value())),
                title: tooltip::intern(name),
                content: tooltip::intern(rung.text()),
                ui: true,
                instant: false,
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lineage_mesh::DEFAULT_STEP;
    use crate::lineage_mesh::fixtures::{layout_of, three};
    use crate::lineage_tree::time_axis;

    /// The pixel of a tile.
    fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
        let at = (y * TILE + x) * 4;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    /// The tile and the pixel in it of a graph point at scale `k` (panel x, height).
    fn locate(layout: &MeshLayout, k: f32, x: f32, height: f32) -> (i32, i32, usize, usize) {
        let px = (SIDE_PADDING + x) * k;
        let py = (layout.size[1] - height) * k;
        (
            (px / TILE as f32).floor() as i32,
            (py / TILE as f32).floor() as i32,
            (px % TILE as f32) as usize,
            (py % TILE as f32) as usize,
        )
    }

    /// The colour painted at a graph point (panel x, height) at scale 1.
    fn painted(layout: &MeshLayout, selected: Option<SpeciesId>, x: f32, height: f32) -> [u8; 4] {
        let (i, j, px, py) = locate(layout, 1.0, x, height);
        pixel(&rasterise_tile(layout, 1.0, selected, i, j), px, py)
    }

    #[test]
    fn a_shape_is_painted_with_its_colour_and_an_outline() {
        let (registry, ids) = three();
        let layout = layout_of(&registry, false, false);
        let mesh = layout.mesh_of(ids[1]).expect("the living root");
        // Between the lines of the second and the third point the edges of the living root are straight.
        let height = time_axis().height_of_point(1, DEFAULT_STEP) + 20.0;
        let profile = mesh.profile_at(height).expect("inside");
        assert!(profile.left_slope.abs() < 1e-3 && profile.right_slope.abs() < 1e-3);
        let expected: Vec<u8> = mesh
            .colour
            .iter()
            .map(|channel| (channel * 255.0).round() as u8)
            .collect();
        // Well inside, clear of the disc around the middle: the species' colour (a root does not fade).
        let inside = painted(&layout, None, profile.left + 40.0, height);
        assert_eq!(inside[3], 255, "opaque");
        for channel in 0..3 {
            assert!(
                (i32::from(inside[channel]) - i32::from(expected[channel])).abs() <= 1,
                "{inside:?} {expected:?}"
            );
        }
        // The border is `Lineage Mesh Shader`'s gradient on the LEFT edge only: black at the edge, the fill 5 units in (`t = 1 - d / 5`).
        let near = painted(&layout, None, profile.left + 1.0, height);
        for channel in 0..3 {
            assert!(
                i32::from(near[channel]) <= (f32::from(expected[channel]) * 0.4) as i32 + 1,
                "mostly black one unit in: {near:?} {expected:?}"
            );
        }
        let halfway = painted(&layout, None, profile.left + 2.5, height);
        assert!(
            halfway[2] > near[2] && halfway[2] < expected[2],
            "{near:?} {halfway:?} {expected:?}"
        );
        assert_eq!(painted(&layout, None, profile.left + 6.0, height)[..3], expected[..]);
        // The right edge has no border at all.
        assert_eq!(painted(&layout, None, profile.right - 1.0, height)[..3], expected[..]);
        // Beyond the shape nothing is painted.
        assert_eq!(painted(&layout, None, profile.left - 40.0, height)[3], 0);
        // The selected species' outline is yellow, 10 units wide and on both edges (`FocusMesh`: `_BorderWidth` 1, `_Focus`).
        for x in [profile.left + 1.0, profile.right - 1.0] {
            let yellow = painted(&layout, Some(ids[1]), x, height);
            assert!(
                yellow[0] > 225 && yellow[1] > 200 && yellow[2] < 70,
                "yellow at the edge: {yellow:?}"
            );
        }
        assert_eq!(
            painted(&layout, Some(ids[1]), profile.left + 11.0, height)[..3],
            expected[..],
            "10 units in it is the fill"
        );
        assert_eq!(
            painted(&layout, None, profile.left + 8.0, height)[..3],
            expected[..]
        );
        assert_ne!(
            painted(&layout, Some(ids[2]), profile.left + 1.0, height)[..3],
            painted(&layout, Some(ids[1]), profile.left + 1.0, height)[..3],
            "another species selected leaves this one's outline alone"
        );
    }

    #[test]
    fn a_child_fades_into_its_parents_colour_at_its_top() {
        let (registry, ids) = three();
        let mut layout = layout_of(&registry, false, false);
        // Both are made of the same template, so the fade is invisible here: give the child another colour.
        let at = layout
            .meshes
            .iter()
            .position(|mesh| mesh.species == ids[2])
            .expect("child");
        layout.meshes[at].colour = [1.0, 0.0, 0.0];
        layout.meshes[at].parent_colour = [0.0, 0.0, 1.0];
        let child = &layout.meshes[at];
        assert!(child.has_parent && child.blend() > 0.0);
        // A point 30% of the width right of the middle is clear of the disc, and inside the outline.
        let paint = |height: f32| {
            let profile = child.profile_at(height).expect("inside");
            let x = (profile.left + profile.right) / 2.0 + 0.3 * (profile.right - profile.left);
            painted(&layout, None, x, height)
        };
        let low = paint(child.bottom_height() + 5.0);
        assert_eq!(low, [255, 0, 0, 255], "its own colour at the bottom");
        // `g` falls from 1 at `1 - blend` of the height to 0 at `1 - blend / 2`: halfway down the ramp the colours are mixed evenly,
        // and above it the shape is its parent's colour.
        let up = 1.0 - 0.75 * child.blend();
        let middle = paint(child.bottom_height() + up * child.size[1]);
        assert!(
            (i32::from(middle[0]) - 127).abs() <= 12 && (i32::from(middle[2]) - 127).abs() <= 12,
            "half and half on the ramp {middle:?}"
        );
        let high = paint(child.top_height() - 1.0);
        assert_eq!(high, [0, 0, 255, 255], "its parent's colour at the very top {high:?}");
    }

    #[test]
    fn a_big_shape_has_a_dark_disc_for_its_picture() {
        let (registry, ids) =
            crate::lineage_mesh::fixtures::registry(&[crate::lineage_mesh::fixtures::Life {
                parent: None,
                born: 0,
                logs: &[(8, 4000.0); 12],
            }]);
        let layout = layout_of(&registry, false, false);
        let mesh = layout.mesh_of(ids[0]).expect("species");
        let preview = mesh.preview.expect("big enough");
        let at = (
            mesh.origin[0] + preview.centre[0],
            mesh.origin[1] + preview.centre[1],
        );
        let (i, j, x, y) = locate(&layout, 1.0, at.0, at.1);
        let tile = rasterise_tile(&layout, 1.0, None, i, j);
        // The middle of the disc is the previewer's navy; the picture goes over it.
        assert_eq!(pixel(&tile, x, y), [0, 0, 27, 255]);
    }

    #[test]
    fn tiles_painted_together_are_the_tiles_painted_one_by_one() {
        let (registry, ids) = three();
        let layout = layout_of(&registry, false, false);
        let bottom = (layout.size[1] / TILE as f32).ceil() as i32 - 1;
        // A scale no other test uses, so that nothing of these is kept yet.
        let k = 0.8125;
        let wanted: Vec<(i32, i32)> = (0..4).flat_map(|i| (0..=bottom).map(move |j| (i, j))).collect();
        warm_tiles(&layout, k, Some(ids[1]), &wanted);
        for &(i, j) in &wanted {
            let kept = tile(&layout, k, Some(ids[1]), i, j);
            let alone = rasterise_tile(&layout, k, Some(ids[1]), i, j);
            match kept {
                Some(image) => assert_eq!(image.rgba, alone, "tile {i},{j}"),
                None => assert!(alone.chunks_exact(4).all(|pixel| pixel[3] == 0), "tile {i},{j} is empty"),
            }
        }
    }

    #[test]
    fn a_rows_clear_middle_is_the_same_as_blending_it_pixel_by_pixel() {
        let row_of = |width: usize, ends: (f32, f32), focused: bool| {
            let mut row = vec![0_u8; width * 4];
            paint_shape_row(
                &mut row,
                &ShapeRow {
                    ends,
                    across: (1.5, 1.5),
                    own: 0.6,
                    colours: ([0.9, 0.1, 0.2], [0.1, 0.8, 0.4]),
                    edge: if focused { YELLOW } else { BLACK },
                    focused,
                },
            );
            row
        };
        for focused in [false, true] {
            let row = row_of(200, (10.3, 170.6), focused);
            // The clear middle is one colour, opaque; the edges are partly covered or in the border's gradient.
            assert_eq!(row[4 * 100..4 * 100 + 4], row[4 * 101..4 * 101 + 4]);
            assert_eq!(row[4 * 100 + 3], 255);
            assert_eq!(row[4 * 5 + 3], 0, "outside");
            assert!(row[4 * 10 + 3] < 255, "the left edge pixel is partly covered");
            assert!(row[4 * 170 + 3] < 255, "the right edge pixel is partly covered");
            assert_ne!(row[4 * 12..4 * 12 + 3], row[4 * 100..4 * 100 + 3], "the border gradient is not the fill");
        }
    }

    #[test]
    fn tiles_are_kept_and_the_least_used_go() {
        let (registry, ids) = three();
        let layout = layout_of(&registry, false, false);
        // The strip starts 150 in and "now" is at the bottom of the graph: the bottom row of tiles has the shapes.
        let bottom = (layout.size[1] / TILE as f32).ceil() as i32 - 1;
        let a = tile(&layout, 1.0, None, 0, bottom).expect("shapes in the bottom left tile");
        let again = tile(&layout, 1.0, None, 0, bottom).expect("kept");
        assert!(Arc::ptr_eq(&a, &again), "the same picture, not a new one");
        let selected = tile(&layout, 1.0, Some(ids[1]), 0, bottom).expect("selected");
        assert!(
            !Arc::ptr_eq(&a, &selected),
            "another selection, another picture"
        );
        // The top of the graph holds nothing: that tile is not drawn at all.
        assert!(tile(&layout, 1.0, None, 3, 0).is_none());
        for j in 0..(CAPACITY as i32 + 8) {
            let _ = tile(&layout, 0.5, None, 0, j);
        }
        assert!(tiles_kept() <= CAPACITY);
    }
}
