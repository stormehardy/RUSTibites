//! Draws the lineage tree (`LineageTreePanel`, `LineageSpeciesNode`, `LineageWindow`'s time axis) in the graph
//! view: the navy viewport, the axis lines and their labels, the species as the game's `SpeciesTreeNode`
//! prefab (the vertical bar, the link to the parent with its elbows and arrow head, the template / alive /
//! extinct markers, the preview box with the name), the selected species outlined in yellow, and the
//! scroll bars. Everything the viewport's mask would cut is cut here (`clip_*`): the renderers have no clip.
//!
//! Prefab values (sharedassets2, `SpeciesTreeNode`): the bar is 11 wide and spans from 17.5 above the bottom to
//! 50.5 (17.5 for a template) under the top; the link is 11 high at the top, its ends 11 x 11 elbows or
//! junctions and an arrow head 35 x 21 hanging at the child; a template shows a 35 x 35 symbol on its top,
//! a living species a 35 x 21 arrow on its bottom, an extinct one a 35 x 35 cross; the preview box is 180 x 160
//! (`Long_UI`, border 7, a 6 inset dark `bg`, the name in Roboto-Bold 15.35 at the bottom) scaled to what fits,
//! else a 25 x 23 point.

use bibites_core::math::Vec2;

use crate::app::App;
use crate::lineage_tree::Node;
use crate::lineage_view::{BAR, BarState, Geometry, View};
use crate::species_panel::Drawer;
use crate::time_format::{TimeFormat, Timescale};
use crate::tooltip::{self, Tip, TipRect};
use crate::ui::{FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame};

const WHITE: Rgba = [255, 255, 255, 255];
/// The viewport's background (0, 0, 0.106).
const NAVY: Rgba = [0, 0, 27, 255];
/// The axis labels (251, 254, 254).
const AXIS_INK: Rgba = [251, 254, 254, 255];
/// The axis labels' font: `LiberationSans SDF - Fallback`, bold style.
const AXIS_FONT: &str = "LiberationSans SDF - Fallback +bold";
/// The axis lines: white at 0.255.
const AXIS_LINE: Rgba = [255, 255, 255, 65];
/// The scroll bars' tracks (`Background` at 0.255).
const TRACK: Rgba = [255, 255, 255, 65];
/// The preview box's `bg` (0.118).
const PREVIEW_BG: Rgba = [30, 30, 30, 255];
/// `highlight` material's `_HighlightColor` (0.984, 1, 0).
const HIGHLIGHT: Rgba = [251, 255, 0, 255];
/// `Color.yellow`: the selected preview frame.
const YELLOW: Rgba = [255, 255, 0, 255];
/// The scroll bars' handle (`UISprite`).
const HANDLE: Rgba = [255, 255, 255, 255];

/// Hit ids of the tree's parts start here (one per part; a lineage has hundreds).
pub const HIT_PART: u32 = 200_000;

/// A rectangle `[x, y, w, h]`.
type Rect = [f32; 4];

/// The overlap of two rectangles.
pub(crate) fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

fn inside(a: Rect, clip: Rect) -> bool {
    a[0] >= clip[0] - 0.01
        && a[1] >= clip[1] - 0.01
        && a[0] + a[2] <= clip[0] + clip[2] + 0.01
        && a[1] + a[3] <= clip[1] + clip[3] + 0.01
}

/// A flat rectangle cut to `clip`.
pub(crate) fn clip_rect(frame: &mut UiFrame, rect: Rect, color: Rgba, clip: Rect) {
    if let Some(r) = intersect(rect, clip) {
        frame.rect(Vec2::new(r[0], r[1]), Vec2::new(r[2], r[3]), color);
    }
}

/// A part `region` (`[x, y, w, h]` in texture pixels, `None` for the whole sprite) of a texture stretched over
/// `rect`, cut to `clip`: the part of the sprite the mask leaves.
pub(crate) fn clip_image(
    frame: &mut UiFrame,
    ui: &UiAssets,
    texture: u16,
    region: Option<Rect>,
    rect: Rect,
    tint: Rgba,
    clip: Rect,
) {
    let Some(source) = ui.texture(texture) else {
        return;
    };
    let region = region.unwrap_or([0.0, 0.0, source.width as f32, source.height as f32]);
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        return;
    }
    let Some(r) = intersect(rect, clip) else {
        return;
    };
    if inside(rect, clip) && region == [0.0, 0.0, source.width as f32, source.height as f32] {
        frame.draws.push(UiDraw::Image {
            texture,
            pos: Vec2::new(rect[0], rect[1]),
            size: Vec2::new(rect[2], rect[3]),
            tint,
        });
        return;
    }
    // The cut rectangle shows this share of the sprite's region.
    let u0 = (r[0] - rect[0]) / rect[2];
    let v0 = (r[1] - rect[1]) / rect[3];
    let u1 = (r[0] + r[2] - rect[0]) / rect[2];
    let v1 = (r[1] + r[3] - rect[1]) / rect[3];
    frame.draws.push(UiDraw::ImageRegion {
        texture,
        region: [
            region[0] + u0 * region[2],
            region[1] + v0 * region[3],
            (u1 - u0) * region[2],
            (v1 - v0) * region[3],
        ],
        pos: Vec2::new(r[0], r[1]),
        size: Vec2::new(r[2], r[3]),
        tint,
    });
}

/// [`clip_image`] of a whole sprite with a white tint (for the tests).
#[cfg(test)]
pub(crate) fn clip_image_for_tests(
    frame: &mut UiFrame,
    ui: &UiAssets,
    texture: u16,
    rect: [f32; 4],
    clip: [f32; 4],
) {
    clip_image(frame, ui, texture, None, rect, WHITE, clip);
}

/// A nine-sliced frame cut to `clip`: the nine pieces drawn as sprite regions (`Image.AdjustBorders` included).
#[allow(clippy::too_many_arguments)]
pub(crate) fn clip_nine_slice(
    frame: &mut UiFrame,
    ui: &UiAssets,
    texture: u16,
    border: f32,
    border_dest: f32,
    rect: Rect,
    tint: Rgba,
    clip: Rect,
) {
    let Some(source) = ui.texture(texture) else {
        return;
    };
    if rect[2] <= 0.0 || rect[3] <= 0.0 {
        return;
    }
    let (bl, br) = crate::ui::shrink_borders(border_dest, border_dest, rect[2]);
    let (bt, bb) = crate::ui::shrink_borders(border_dest, border_dest, rect[3]);
    let xs = [
        rect[0],
        rect[0] + bl,
        rect[0] + rect[2] - br,
        rect[0] + rect[2],
    ];
    let ys = [
        rect[1],
        rect[1] + bt,
        rect[1] + rect[3] - bb,
        rect[1] + rect[3],
    ];
    let (w, h) = (source.width as f32, source.height as f32);
    let sx = [0.0, border, w - border, w];
    let sy = [0.0, border, h - border, h];
    for row in 0..3 {
        for col in 0..3 {
            let piece = [
                xs[col],
                ys[row],
                xs[col + 1] - xs[col],
                ys[row + 1] - ys[row],
            ];
            if piece[2] <= 0.0 || piece[3] <= 0.0 {
                continue;
            }
            let region = [
                sx[col],
                sy[row],
                (sx[col + 1] - sx[col]).max(1.0),
                (sy[row + 1] - sy[row]).max(1.0),
            ];
            clip_image(frame, ui, texture, Some(region), piece, tint, clip);
        }
    }
}

impl Drawer<'_> {
    /// The graph view: the viewport with the tree in it, and the scroll bars.
    pub(crate) fn lineage_graph(&self, frame: &mut UiFrame, app: &App) {
        let view = &app.lineage_view;
        let g = view.geometry();
        let clip = self.scaled(g.viewport);
        frame.rect(
            Vec2::new(clip[0], clip[1]),
            Vec2::new(clip[2], clip[3]),
            NAVY,
        );
        if view.mesh_mode && view.mesh.built {
            self.axis(frame, view, clip);
            self.mesh_view(frame, app, clip);
        } else if !view.mesh_mode && view.layout.built {
            self.axis(frame, view, clip);
            let mut next_hit = HIT_PART;
            let selected = app.species_info;
            // The nodes draw in the order they were made (the oldest first), so the newer ones are over them.
            let drawn: Vec<usize> = view.layout.drawn().map(|(index, _)| index).collect();
            // The pictures the visible boxes need, made together (in parallel) before the nodes draw.
            let wanted: Vec<_> = drawn
                .iter()
                .map(|&index| &view.layout.nodes[index])
                .filter(|node| node.can_fit_preview)
                .filter(|node| {
                    let x = View::holder_x(node.x);
                    let scale = node.preview_scale;
                    let top = if node.has_parent { node.top_height - 27.5 } else { node.top_height + 80.0 * scale };
                    let rect = self.content_rect(view, x - 90.0 * scale, top - 160.0 * scale, x + 90.0 * scale, top);
                    intersect(rect, clip).is_some()
                })
                .map(|node| {
                    (
                        node.species,
                        crate::preview::TREE_NODE,
                        [180.0, 160.0],
                        node.preview_scale * view.zoom * self.s,
                    )
                })
                .collect();
            app.warm_species_pictures(&wanted);
            for index in drawn {
                let node = &view.layout.nodes[index];
                let outlined = selected == Some(node.species);
                self.node(frame, app, view, node, outlined, clip, &mut next_hit);
            }
        }
        self.scroll_bars(frame, view, g);
    }

    /// A rectangle in reference px as screen px.
    fn scaled(&self, rect: Rect) -> Rect {
        [
            rect[0] * self.s,
            rect[1] * self.s,
            rect[2] * self.s,
            rect[3] * self.s,
        ]
    }

    /// The screen rectangle of a content rectangle given by its left, bottom, right and top.
    fn content_rect(&self, view: &View, left: f32, bottom: f32, right: f32, top: f32) -> Rect {
        let a = view.to_screen(left, top);
        let b = view.to_screen(right, bottom);
        [
            a[0] * self.s,
            a[1] * self.s,
            (b[0] - a[0]) * self.s,
            (b[1] - a[1]) * self.s,
        ]
    }

    /// `LineageWindow`'s time lines: one at the height of every point, across the viewport, with the time on
    /// both sides (the lines keep to the viewport's edges as the graph scrolls).
    fn axis(&self, frame: &mut UiFrame, view: &View, clip: Rect) {
        let axis = crate::lineage_tree::time_axis();
        let zoom = view.zoom;
        let first = view.max_n().max(axis.non_infinite_max_n as i32);
        let step = view.step();
        for i in -1..first {
            let height = axis.height_of_point(i, step);
            let y = view.to_screen(0.0, height)[1] * self.s;
            // The line, 1 px thick: `line.localScale.y = 1 / scale`.
            let line = [clip[0], y - self.s, clip[2], self.s];
            if intersect(line, clip).is_none() && y > clip[1] + clip[3] + 40.0 * self.s {
                continue;
            }
            clip_rect(frame, line, AXIS_LINE, clip);
            // The label sits on the line: the holder is 13 to 21 units high by the zoom
            // (`UpdateLineScaleToMatchParent`) and the text hangs from its top.
            let holder = if zoom > 1.0 {
                lerp(13.0, 17.0, 1.0 / zoom)
            } else {
                lerp(17.0 + (17.0 - 13.0), 17.0, zoom)
            };
            // `LiberationSans SDF - Fallback`, bold style (TMP widens the regular atlas), auto-sized to the holder's height (0.8941 of it:
            // 15.2 in the 17 high holder at zoom 1, 16.95 in the 18.96 one at 0.504 - two live captures), under the holder's zoom.
            let px = 0.8941 * holder * zoom * self.s;
            if px < 3.0 {
                continue;
            }
            let top = y - (1.0 + holder) * zoom * self.s;
            let text = axis_label(i);
            let width = self.measure_named(AXIS_FONT, &text, px);
            let height_px = px * 1.17;
            // The holder's layout group pads both sides by 5 (the text is not scaled with the graph: `InverseParentScale`).
            let left_box = [clip[0] + 5.0 * self.s, top, width, height_px];
            let right_box = [
                clip[0] + clip[2] - 5.0 * self.s - width,
                top,
                width,
                height_px,
            ];
            for text_box in [left_box, right_box] {
                if inside(text_box, clip) {
                    frame.draws.push(UiDraw::Text {
                        font: Some(AXIS_FONT.to_owned()),
                        text: text.clone(),
                        pos: Vec2::new(text_box[0], text_box[1]),
                        px,
                        color: AXIS_INK,
                        role: FontRole::Body,
                    });
                }
            }
        }
    }

    /// One species of the tree.
    #[allow(clippy::too_many_arguments)]
    fn node(
        &self,
        frame: &mut UiFrame,
        app: &App,
        view: &View,
        node: &Node,
        outlined: bool,
        clip: Rect,
        next_hit: &mut u32,
    ) {
        let x = View::holder_x(node.x);
        let (top, bottom) = (node.top_height, node.bottom_height);
        let zoom = view.zoom;
        let name = app
            .lineage_species
            .iter()
            .find(|entry| entry.id == node.species)
            .map_or_else(|| node.name.clone(), |entry| entry.name.clone());
        let parent_name = app
            .lineage_species
            .iter()
            .find(|entry| entry.id == node.species)
            .and_then(|entry| entry.parent)
            .and_then(|parent| app.lineage_species.iter().find(|entry| entry.id == parent))
            .map(|entry| entry.name.clone());
        let age = |point: i32| {
            TimeFormat::of_species_point(point).formatted(1, " ", false, true, Timescale::Minutes)
        };
        let mut part = |frame: &mut UiFrame,
                        sprite: &str,
                        rect: Rect,
                        tip: Option<(&'static str, String, bool)>,
                        clickable: bool| {
            let Some(texture) = self.ui.texture_of_alias(sprite) else {
                return;
            };
            if outlined && let Some(own) = intersect(rect, clip) {
                // The `highlight` material's `_focus` (`Unlit/highlightableGraph`): the alpha becomes the sum of the sprite's own and its four
                // neighbours 2 sprite pixels away (no diagonals), coloured with the highlight colour - and it stays inside the sprite's own quad.
                let (w, h) = self
                    .ui
                    .texture(texture)
                    .map_or((1.0, 1.0), |t| (t.width as f32, t.height as f32));
                let (ox, oy) = (2.0 * rect[2] / w, 2.0 * rect[3] / h);
                for (dx, dy) in [(-ox, 0.0), (ox, 0.0), (0.0, -oy), (0.0, oy)] {
                    clip_image(
                        frame,
                        self.ui,
                        texture,
                        None,
                        [rect[0] + dx, rect[1] + dy, rect[2], rect[3]],
                        HIGHLIGHT,
                        own,
                    );
                }
            }
            clip_image(frame, self.ui, texture, None, rect, WHITE, clip);
            if clickable || tip.is_some() {
                if let Some(visible) = intersect(rect, clip) {
                    if clickable {
                        frame.hit(
                            *next_hit,
                            Vec2::new(visible[0], visible[1]),
                            Vec2::new(visible[2], visible[3]),
                            UiAction::SelectLineageNode(node.species.value()),
                        );
                        *next_hit += 1;
                    }
                    if let Some((title, content, instant)) = tip {
                        let content = tooltip::intern(content);
                        frame.tips.push(TipRect {
                            pos: Vec2::new(visible[0], visible[1]),
                            size: Vec2::new(visible[2], visible[3]),
                            tip: Tip {
                                path: "",
                                title,
                                content,
                                ui: true,
                                instant,
                            },
                        });
                    }
                }
            }
        };
        // The vertical bar: from 17.5 over the bottom to 50.5 (a template: 17.5) under the top.
        let bar_top = top - if node.has_parent { 50.5 } else { 17.5 };
        if bar_top > bottom + 17.5 {
            let rect = self.content_rect(view, x - 5.5, bottom + 17.5, x + 5.5, bar_top);
            part(frame, "lineage-vertical", rect, None, true);
        }
        // The link to the parent: a horizontal line from the child to the parent, its ends the elbow at the
        // child's side and the junction where it meets the parent, and the arrow head hanging at the child.
        if node.has_parent {
            let dist = node.link_dist;
            let (left, right) = (x.min(x + dist), x.max(x + dist));
            let birth = format!(
                "{} diverged from {} approximately {} ago",
                name,
                parent_name.clone().unwrap_or_default(),
                age(node.apparition)
            );
            let link = self.content_rect(view, left, top - 5.5, right, top + 5.5);
            // The whole link takes the tooltip (`parentLink`'s trigger is instant).
            if let Some(visible) = intersect(link, clip) {
                frame.tips.push(TipRect {
                    pos: Vec2::new(visible[0], visible[1]),
                    size: Vec2::new(visible[2], visible[3]),
                    tip: Tip {
                        path: "",
                        title: "Species' Birth",
                        content: tooltip::intern(birth),
                        ui: true,
                        instant: true,
                    },
                });
            }
            if right - left > 11.0 {
                let line = self.content_rect(view, left + 5.5, top - 5.5, right - 5.5, top + 5.5);
                part(frame, "lineage-horizontal", line, None, true);
            }
            let (first, second) = if dist > 0.0 {
                ("lineage-elbow-right", "lineage-junction-left")
            } else {
                ("lineage-junction-right", "lineage-elbow-left")
            };
            let a = self.content_rect(view, left - 5.5, top - 5.5, left + 5.5, top + 5.5);
            let b = self.content_rect(view, right - 5.5, top - 5.5, right + 5.5, top + 5.5);
            part(frame, first, a, None, true);
            part(frame, second, b, None, true);
            let arrow = self.content_rect(view, x - 17.5, top - 28.5, x + 17.5, top - 7.5);
            part(frame, "lineage-arrowhead", arrow, None, true);
            let neck = self.content_rect(view, x - 5.5, top - 7.5, x + 5.5, top - 5.5);
            part(frame, "lineage-vertical", neck, None, true);
        } else {
            // A template: its symbol on the top.
            let rect = self.content_rect(view, x - 17.5, top - 17.5, x + 17.5, top + 17.5);
            let tip = (
                "Species' Introduction",
                format!(
                    "{} was introduced in the simulation as a template approximately {} ago",
                    name,
                    age(node.apparition)
                ),
                false,
            );
            part(frame, "lineage-template", rect, Some(tip), true);
        }
        // The end: an arrow for a species that is still around, a cross for one that went extinct.
        if node.alive {
            let rect = self.content_rect(view, x - 17.5, bottom - 1.0, x + 17.5, bottom + 20.0);
            let tip = ("Alive", format!("{name} is still around and well."), false);
            part(frame, "lineage-arrowhead", rect, Some(tip), true);
        } else {
            let rect = self.content_rect(view, x - 17.5, bottom - 17.5, x + 17.5, bottom + 17.5);
            let tip = (
                "Extinction",
                format!(
                    "{} went extinct approximately {} ago",
                    name,
                    age(node.disappearance)
                ),
                false,
            );
            part(frame, "lineage-termination", rect, Some(tip), true);
        }
        // The species: the preview box when it fits, else a point.
        if node.can_fit_preview {
            self.preview_box(
                frame,
                app,
                view,
                node,
                &name,
                x,
                outlined,
                parent_name.as_deref(),
                clip,
                next_hit,
            );
        } else if node.has_parent {
            let rect = self.content_rect(view, x - 12.5, top - 27.5 - 23.0, x + 12.5, top - 27.5);
            let tip = (
                tooltip::intern(name.clone()),
                format!("A child species of {}", parent_name.unwrap_or_default()),
                false,
            );
            part(frame, "lineage-node-small", rect, Some(tip), true);
        }
        let _ = zoom;
    }

    /// The preview box of a species: the frame, the dark field, the name.
    #[allow(clippy::too_many_arguments)]
    fn preview_box(
        &self,
        frame: &mut UiFrame,
        app: &App,
        view: &View,
        node: &Node,
        name: &str,
        x: f32,
        outlined: bool,
        parent_name: Option<&str>,
        clip: Rect,
        next_hit: &mut u32,
    ) {
        let scale = node.preview_scale;
        let (w, h) = (180.0 * scale, 160.0 * scale);
        let top = if node.has_parent {
            node.top_height - 27.5
        } else {
            node.top_height + h / 2.0
        };
        let rect = self.content_rect(view, x - w / 2.0, top - h, x + w / 2.0, top);
        let tint = if outlined { YELLOW } else { WHITE };
        // `Long_UI`: border 70 px, drawn at `ppum 2` (7 units) of a box scaled by `scale` and the zoom.
        let unit = scale * view.zoom * self.s;
        match self.ui.texture_of_alias("Long_UI") {
            Some(texture) => {
                clip_nine_slice(frame, self.ui, texture, 70.0, 7.0 * unit, rect, tint, clip)
            }
            None => clip_rect(frame, rect, tint, clip),
        }
        let inset = 6.0 * unit;
        let field = [
            rect[0] + inset,
            rect[1] + inset,
            rect[2] - 2.0 * inset,
            rect[3] - 2.0 * inset,
        ];
        clip_rect(frame, field, PREVIEW_BG, clip);
        // The species' procedural bibite (`SpeciesPreview/proceduralBibite`), drawn over the dark field: a picture
        // of the whole box at the size it shows, cut by the scroll view's mask.
        // Only a box that shows asks for its picture (the pictures of the nodes outside the view were made too, one after another: 10 to 60 ms
        // of a frame while zooming a long tree).
        if let Some(visible) = intersect(rect, clip)
            && let Some(texture) = app.species_picture(
                node.species,
                crate::preview::TREE_NODE,
                [180.0, 160.0],
                unit,
            )
        {
            let (width, height) = (texture.width as f32, texture.height as f32);
            let region = [
                (visible[0] - rect[0]) / rect[2] * width,
                (visible[1] - rect[1]) / rect[3] * height,
                visible[2] / rect[2] * width,
                visible[3] / rect[3] * height,
            ];
            frame.draws.push(crate::ui::UiDraw::PixelsRegion {
                texture,
                region,
                pos: Vec2::new(visible[0], visible[1]),
                size: Vec2::new(visible[2], visible[3]),
            });
        }
        // The name: Roboto-Bold 15.35, at the bottom, centred, shrunk to the room it has.
        let px = 15.35 * unit;
        if px >= 3.0 {
            let room = field[2];
            let size = self.fitted(name, px, true, room);
            let width = self.measure(name, size, true);
            let text_box = [
                rect[0] + (rect[2] - width) / 2.0,
                rect[1] + rect[3] - 6.0 * unit - size * 1.17,
                width,
                size * 1.17,
            ];
            // The viewport's mask cuts a name at its edge (the game draws what shows of it).
            if intersect(text_box, clip).is_some() {
                frame.draws.push(crate::ui::UiDraw::Clip(Some(clip)));
                frame.bold(
                    name.to_owned(),
                    Vec2::new(text_box[0], text_box[1]),
                    size,
                    WHITE,
                );
                frame.draws.push(crate::ui::UiDraw::Clip(None));
            }
        }
        if let Some(visible) = intersect(rect, clip) {
            frame.hit(
                *next_hit,
                Vec2::new(visible[0], visible[1]),
                Vec2::new(visible[2], visible[3]),
                UiAction::SelectLineageNode(node.species.value()),
            );
            *next_hit += 1;
            frame.tips.push(TipRect {
                pos: Vec2::new(visible[0], visible[1]),
                size: Vec2::new(visible[2], visible[3]),
                tip: Tip {
                    path: "",
                    title: tooltip::intern(name.to_owned()),
                    content: tooltip::intern(
                        parent_name.map_or_else(String::new, |parent| {
                            format!("Child species of {parent}")
                        }),
                    ),
                    ui: true,
                    instant: false,
                },
            });
        }
    }

    /// The scroll bars: the tracks (`Background` at 0.255) where a bar shows and the handles.
    fn scroll_bars(&self, frame: &mut UiFrame, view: &View, g: Geometry) {
        let (horizontal, vertical) = view.bars();
        let track_v = crate::lineage_view::vertical_track(g);
        let track_h = crate::lineage_view::horizontal_track(g);
        // The track is the `Background` sprite (sliced, border 10) at 0.255, the handle the `UISprite` one (as the Information panel's list).
        if g.vertical {
            self.slice(frame, "Background", 10.0, 1.0, track_v, TRACK);
            self.handle(frame, track_v, vertical, true);
        }
        if g.horizontal {
            self.slice(frame, "Background", 10.0, 1.0, track_h, TRACK);
            self.handle(frame, track_h, horizontal, false);
        }
    }

    /// A bar's handle in its sliding area (the track inset by 10): `size` of it, at `value` of the rest.
    fn handle(&self, frame: &mut UiFrame, track: Rect, bar: BarState, vertical: bool) {
        let rect = crate::lineage_view::handle_rect(track, bar, vertical);
        self.slice(frame, "UISprite", 10.0, 1.0, rect, HANDLE);
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// `GraphLineTextReference.InitializeLine(TimeFormat)`: "now", or the time ago in small units.
fn axis_label(point: i32) -> String {
    let time = TimeFormat::of_species_point(point);
    if time.val < 0.0001 {
        "now".to_owned()
    } else {
        format!(
            "{} ago",
            time.formatted(1, " ", true, false, Timescale::Minutes)
        )
    }
}

#[allow(dead_code)]
const _BAR: f32 = BAR;
