//! The lineage tree's window: zoom, pan and the scroll bars (`LineageWindow` + `AncestralLineageScroll`, a
//! `ScrollRect`), over the layout of `lineage_tree.rs`.
//!
//! The graph is a content rectangle (`GraphElementHolder`: anchored along the viewport's bottom, pivot at its
//! bottom centre) scaled by the zoom. The mouse wheel scrolls it 35 units a notch (Shift: sideways), Ctrl + wheel
//! zooms by a sixth of the current zoom about the middle of the viewport's height, the right mouse button drags it,
//! and the scroll bars show where the view is. The `ScrollRect` is clamped (the content never leaves the view),
//! and its bars hide when the content fits, the viewport then growing into the room (`AutoHideAndExpandViewport`).
//! All numbers are the prefab's (read from level2 with `tools/unity`): sensitivity 35, bars 20 wide and 10 apart.

use bibites_core::ids::SpeciesId;
use bibites_core::species::SpeciesRegistry;

use crate::lineage_mesh::MeshLayout;
use crate::lineage_tree::{Layout, MAX_ZOOM, SIDE_PADDING, SpeciesInfo, TreeSettings};

/// `LineageView`: `x, y, w, h` in reference pixels, for the 1920 x 1080 canvas the prefab was captured on (see [`view_in`]).
pub const VIEW: [f32; 4] = [15.0, 50.0, 1895.0, 1020.0];

/// `LineageView` in a canvas of `canvas` reference px (`window / UI scale`): the prefab stretches it over the panel (anchors (0, 0) to (1, 1), offsets 15 from
/// the left, 50 from the top, 10 from the right and 10 from the bottom), so it grows and shrinks with the window's shape.
#[must_use]
pub fn view_in(canvas: [f32; 2]) -> [f32; 4] {
    // (A canvas too small for the two scroll bars keeps one of their room's worth of view.)
    [
        VIEW[0],
        VIEW[1],
        (canvas[0] - (VIEW[0] + 10.0)).max(BAR_ROOM + 1.0),
        (canvas[1] - (VIEW[1] + 10.0)).max(BAR_ROOM + 1.0),
    ]
}
/// A scroll bar's width (20) and its spacing (10): what the viewport gives up for one.
pub const BAR_ROOM: f32 = 30.0;
/// A scroll bar's thickness.
pub const BAR: f32 = 20.0;
/// `ScrollRect.scrollSensitivity`.
pub const SCROLL_SENSITIVITY: f32 = 35.0;
/// `LineageWindow.Update`: a wheel notch with Ctrl zooms by this share of the zoom.
pub const WHEEL_ZOOM: f32 = 1.0 / 6.0;
/// The content's height before the tree sets it (`GraphElementHolder.sizeDelta`).
const DEFAULT_CONTENT_HEIGHT: f32 = 975.8;

/// What a drag in progress moves.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Drag {
    /// Nothing.
    #[default]
    None,
    /// The right button drags the content: the cursor it was last seen at.
    Pan([f32; 2]),
    /// The horizontal bar's handle: where in the handle it was grabbed (0..1).
    HorizontalBar(f32),
    /// The vertical bar's handle.
    VerticalBar(f32),
}

/// The viewport and the bars that show, for the current zoom and position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    /// The whole view (`LineageView`, `x, y, w, h`, reference px): the viewport and the room of the scroll bars.
    pub view: [f32; 4],
    /// The viewport (`x, y, w, h`, reference px): the part of the view that shows the graph.
    pub viewport: [f32; 4],
    /// The vertical bar shows.
    pub vertical: bool,
    /// The horizontal bar shows.
    pub horizontal: bool,
}

/// One scroll bar's state: where the handle is and how much of the track it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarState {
    /// `Scrollbar.value`, 0..1 (the left, or the bottom, is 0).
    pub value: f32,
    /// `Scrollbar.size`, 0..1.
    pub size: f32,
}

/// The vertical bar's track (`Scrollbar Vertical`: 20 wide at the view's right edge, 30 shorter than the view).
#[must_use]
pub fn vertical_track(geometry: Geometry) -> [f32; 4] {
    let view = geometry.view;
    [view[0] + view[2] - BAR, view[1], BAR, view[3] - BAR_ROOM]
}

/// The horizontal bar's track (`Scrollbar Horizontal`: 20 high at the view's bottom, 30 narrower than the view).
#[must_use]
pub fn horizontal_track(geometry: Geometry) -> [f32; 4] {
    let view = geometry.view;
    [view[0], view[1] + view[3] - BAR, view[2] - BAR_ROOM, BAR]
}

/// A bar's sliding area: the track inset by 10 on every side (`Sliding Area`, size -20 x -20).
#[must_use]
pub fn sliding_area(track: [f32; 4]) -> [f32; 4] {
    [
        track[0] + 10.0,
        track[1] + 10.0,
        track[2] - 20.0,
        track[3] - 20.0,
    ]
}

/// A bar's handle: `size` of the sliding area, `value` of the way along what is left, and 10 more at every side
/// (the prefab's `Handle` is 20 x 20 over stretched anchors), so it fills the track across and reaches its ends.
#[must_use]
pub fn handle_rect(track: [f32; 4], bar: BarState, vertical: bool) -> [f32; 4] {
    let sliding = sliding_area(track);
    let along = bar.value * (1.0 - bar.size);
    if vertical {
        let length = sliding[3] * bar.size;
        let from_bottom = sliding[3] * along;
        let top = sliding[1] + sliding[3] - from_bottom - length;
        [
            sliding[0] - 10.0,
            top - 10.0,
            sliding[2] + 20.0,
            length + 20.0,
        ]
    } else {
        [
            sliding[0] + sliding[2] * along - 10.0,
            sliding[1] - 10.0,
            sliding[2] * bar.size + 20.0,
            sliding[3] + 20.0,
        ]
    }
}

/// The lineage window: the tree or the mesh in it (`SpeciesPanel.treeOrMesh`).
#[derive(Clone, Debug)]
pub struct View {
    /// The laid out tree (when the tree is the view that shows).
    pub layout: Layout,
    /// The options it was laid out with.
    pub settings: TreeSettings,
    /// The mesh view shows instead of the tree (`treeOrMesh` is false). Like the game's flag it stays as it was left.
    pub mesh_mode: bool,
    /// The laid out mesh (when the mesh is the view that shows).
    pub mesh: MeshLayout,
    /// The zoom (`GraphElementHolder.localScale`).
    pub zoom: f32,
    /// The content's `anchoredPosition` (x right, y up), relative to the viewport's bottom centre.
    pub pos: [f32; 2],
    /// A drag in progress.
    pub drag: Drag,
    /// `LineageView` for the canvas the panel is drawn on (`x, y, w, h`, reference px; see [`view_in`]).
    pub view: [f32; 4],
}

impl Default for View {
    fn default() -> Self {
        Self {
            layout: Layout::default(),
            settings: TreeSettings::default(),
            mesh_mode: false,
            mesh: MeshLayout::default(),
            zoom: 1.0,
            pos: [0.0, 0.0],
            drag: Drag::None,
            view: VIEW,
        }
    }
}

impl View {
    /// The view that shows has something to draw (`treePanel.treeBuilt`, `!meshPanel.empty`).
    #[must_use]
    pub fn built(&self) -> bool {
        if self.mesh_mode {
            self.mesh.built
        } else {
            self.layout.built
        }
    }

    /// The species of the view that shows (`SpeciesPanel.displayedSpecies`).
    #[must_use]
    pub fn displayed(&self) -> Vec<SpeciesId> {
        if self.mesh_mode {
            self.mesh.displayed.clone()
        } else {
            self.layout.displayed()
        }
    }

    /// Whether the view that shows has a node or a shape for the species.
    #[must_use]
    pub fn shows(&self, species: SpeciesId) -> bool {
        if self.mesh_mode {
            self.mesh.mesh_of(species).is_some()
        } else {
            self.layout.node_of(species).is_some()
        }
    }

    /// `LineageWindow.defaultStep`: the height of the axis' first point (the tree 75, the mesh 50).
    #[must_use]
    pub fn step(&self) -> f32 {
        if self.mesh_mode {
            crate::lineage_mesh::DEFAULT_STEP
        } else {
            crate::lineage_tree::DEFAULT_STEP
        }
    }

    /// `maxN` of the view that shows: how many points its axis runs to.
    #[must_use]
    pub fn max_n(&self) -> i32 {
        if self.mesh_mode {
            self.mesh.max_n
        } else {
            self.layout.max_n
        }
    }

    /// `SetMinMaxZoom`'s minimum for the view that shows.
    #[must_use]
    pub fn min_zoom(&self) -> f32 {
        if self.mesh_mode {
            self.mesh.min_zoom
        } else {
            self.layout.min_zoom
        }
    }

    /// The content's size in graph units: the tree's or the mesh's size once it was placed, else the viewport's.
    #[must_use]
    pub fn content_size(&self) -> [f32; 2] {
        if self.mesh_mode && self.mesh.built {
            self.mesh.size
        } else if !self.mesh_mode && self.layout.built {
            self.layout.graph_size
        } else {
            [self.view[2] - BAR_ROOM, DEFAULT_CONTENT_HEIGHT]
        }
    }

    /// The panel is drawn on a canvas of `canvas` reference px (`window / UI scale`): the view stretches with it
    /// (`RectTransformChangeHandler`: the tree is placed again for the new viewport, and the ScrollRect keeps the content in view).
    /// Returns whether the view changed; the mesh view is made for its viewport by the caller (it needs the species).
    pub fn set_canvas(&mut self, canvas: [f32; 2]) -> bool {
        let view = view_in(canvas);
        if view.iter().zip(self.view).all(|(a, b)| (a - b).abs() < 0.01) {
            return false;
        }
        self.view = view;
        if !self.mesh_mode && self.layout.built {
            let mut viewport = self.viewport_size();
            for _ in 0..6 {
                self.layout.place(viewport);
                let now = self.viewport_size();
                if (now[0] - viewport[0]).abs() < 0.01 && (now[1] - viewport[1]).abs() < 0.01 {
                    break;
                }
                viewport = now;
            }
            self.zoom_to(self.zoom);
        } else {
            self.clamp();
        }
        true
    }

    /// `LineageWindow.ResetView` (`SpeciesPanel.SelectPanel`, only when the view is switched): `ZoomToScale(1)` from the zoom the
    /// last view had, about the middle of the viewport's height, so the content is NOT taken back to the bottom: coming from the
    /// tree at its fit zoom (0.13 for a long history) the new view opens scrolled to wherever that middle point lands. The
    /// ScrollRect then keeps the content in view.
    pub fn reset_view(&mut self) {
        self.drag = Drag::None;
        self.zoom_to(1.0);
    }

    /// The viewport and the bars, following `ScrollRect.SetLayoutHorizontal` for `AutoHideAndExpandViewport`.
    #[must_use]
    pub fn geometry(&self) -> Geometry {
        let content = self.content_size();
        let (cw, ch) = (content[0] * self.zoom, content[1] * self.zoom);
        let mut size = [self.view[2], self.view[3]];
        let (mut vertical, mut horizontal) = (false, false);
        if ch > size[1] + 0.01 {
            vertical = true;
            size[0] -= BAR_ROOM;
        }
        if cw > size[0] + 0.01 {
            horizontal = true;
            size[1] -= BAR_ROOM;
        }
        // The vertical bar did not show at first and the horizontal one took room from the height.
        if !vertical && horizontal && ch > size[1] + 0.01 {
            vertical = true;
            size[0] -= BAR_ROOM;
        }
        Geometry {
            view: self.view,
            viewport: [self.view[0], self.view[1], size[0], size[1]],
            vertical,
            horizontal,
        }
    }

    /// The viewport's size in graph units at zoom 1 (what the tree is placed for).
    fn viewport_size(&self) -> [f32; 2] {
        let g = self.geometry();
        [g.viewport[2], g.viewport[3]]
    }

    /// `LineageTreePanel.OpenPanel` / `BuildTree`: lays the species out for the options and fits the view.
    pub fn rebuild(&mut self, infos: &[SpeciesInfo], settings: TreeSettings) {
        self.settings = settings;
        // The first layout is made for the prefab's viewport (both bars); the viewport then follows the content.
        let mut viewport = [self.view[2] - BAR_ROOM, self.view[3] - BAR_ROOM];
        self.layout = Layout::build(infos, settings, viewport);
        self.drag = Drag::None;
        if !self.layout.built {
            return;
        }
        self.zoom_to(self.layout.fit_zoom);
        // `RectTransformChangeHandler`: when the viewport changed (a bar showed or hid), the tree is placed again.
        for _ in 0..6 {
            let now = self.viewport_size();
            if (now[0] - viewport[0]).abs() < 0.01 && (now[1] - viewport[1]).abs() < 0.01 {
                break;
            }
            viewport = now;
            self.layout.place(viewport);
            self.zoom_to(self.zoom);
        }
    }

    /// `LineageMeshPanel.OpenPanel` / `GenerateMesh`: lays the species out as shapes for the options. The mesh is made
    /// for the viewport as it is (the vertical bar takes room from its width when the graph is taller than the view).
    pub fn rebuild_mesh(
        &mut self,
        registry: &SpeciesRegistry,
        now: crate::lineage_mesh::Now,
        settings: TreeSettings,
    ) {
        self.settings = settings;
        // (`GenerateMesh` does not reset the view: a rebuild for a changed option keeps the zoom and the place.)
        self.drag = Drag::None;
        let mut viewport = [self.view[2], self.view[3]];
        for _ in 0..4 {
            self.mesh = MeshLayout::build(
                registry,
                now,
                settings.hide_extinct,
                settings.show_favorites,
                viewport,
            );
            if !self.mesh.built {
                return;
            }
            let now = self.viewport_size();
            if (now[0] - viewport[0]).abs() < 0.01 && (now[1] - viewport[1]).abs() < 0.01 {
                break;
            }
            viewport = now;
        }
        // `SetMinMaxZoom` keeps the zoom within the new limits (`Zoom(0)`), and the ScrollRect the content in view.
        self.zoom_to(self.zoom);
    }

    /// The point of the content (`x` right from its left edge, `y` up from its bottom) under a point of the view in
    /// reference px: the inverse of [`View::to_screen`].
    #[must_use]
    pub fn to_content(&self, x: f32, y: f32) -> [f32; 2] {
        let g = self.geometry();
        let content = self.content_size();
        let [vx, vy, vw, vh] = g.viewport;
        [
            content[0] / 2.0 + (x - vx - vw / 2.0 - self.pos[0]) / self.zoom,
            (vy + vh - self.pos[1] - y) / self.zoom,
        ]
    }

    /// `LineageWindow.ZoomToScale`: zooms about the middle of the viewport's height.
    pub fn zoom_to(&mut self, scale: f32) {
        let (min, max) = (self.min_zoom(), MAX_ZOOM);
        let scale = if min <= max {
            scale.clamp(min, max)
        } else {
            scale.min(max)
        };
        let ratio = scale / self.zoom;
        let height = self.geometry().viewport[3];
        self.pos[0] -= self.pos[0] * (1.0 - ratio);
        self.pos[1] += (height / 2.0 - self.pos[1]) * (1.0 - ratio);
        self.zoom = scale;
        self.clamp();
    }

    /// `LineageWindow.Zoom`.
    pub fn zoom_by(&mut self, increment: f32) {
        self.zoom_to(self.zoom * (1.0 + increment));
    }

    /// The ScrollRect's clamping: the content never leaves the view (smaller content is centred across, and
    /// rests on the bottom).
    pub fn clamp(&mut self) {
        let g = self.geometry();
        let content = self.content_size();
        let (cw, ch) = (content[0] * self.zoom, content[1] * self.zoom);
        let (vw, vh) = (g.viewport[2], g.viewport[3]);
        let offset = |lo: f32, hi: f32, view: f32| -> f32 {
            let min_offset = -lo;
            let max_offset = view - hi;
            if min_offset < -0.001 {
                min_offset
            } else if max_offset > 0.001 {
                max_offset
            } else {
                0.0
            }
        };
        // x: relative to the viewport's left edge.
        let left = vw / 2.0 + self.pos[0] - cw / 2.0;
        let (lo, hi) = if cw < vw {
            let centre = left + cw / 2.0;
            (centre - vw / 2.0, centre + vw / 2.0)
        } else {
            (left, left + cw)
        };
        self.pos[0] += offset(lo, hi, vw);
        // y: up from the viewport's bottom edge; the content rests on it.
        let bottom = self.pos[1];
        self.pos[1] += offset(bottom, bottom + ch.max(vh), vh);
    }

    /// The mouse wheel: scrolls the content (`ScrollRect.OnScroll`: up reveals what is above, Shift moves sideways).
    pub fn scroll(&mut self, wheel: f32, shift: bool) {
        let (dx, dy) = if shift { (wheel, 0.0) } else { (0.0, -wheel) };
        self.pos[0] += dx * SCROLL_SENSITIVITY;
        self.pos[1] += dy * SCROLL_SENSITIVITY;
        self.clamp();
    }

    /// A right-button drag moved the cursor by this much (reference px, y down).
    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.pos[0] += dx;
        self.pos[1] -= dy;
        self.clamp();
    }

    /// A content point (x right from the content's left edge, y up from its bottom) in reference px.
    #[must_use]
    pub fn to_screen(&self, x: f32, y: f32) -> [f32; 2] {
        let g = self.geometry();
        let content = self.content_size();
        let [vx, vy, vw, vh] = g.viewport;
        [
            vx + vw / 2.0 + self.pos[0] + (x - content[0] / 2.0) * self.zoom,
            vy + vh - self.pos[1] - y * self.zoom,
        ]
    }

    /// The horizontal and the vertical bar.
    #[must_use]
    pub fn bars(&self) -> (BarState, BarState) {
        let g = self.geometry();
        let content = self.content_size();
        let (cw, ch) = (content[0] * self.zoom, content[1] * self.zoom);
        let (vw, vh) = (g.viewport[2], g.viewport[3]);
        let h_value = if cw <= vw {
            0.0
        } else {
            let left = vw / 2.0 + self.pos[0] - cw / 2.0;
            (-left / (cw - vw)).clamp(0.0, 1.0)
        };
        let v_value = if ch <= vh {
            0.0
        } else {
            (-self.pos[1] / (ch - vh)).clamp(0.0, 1.0)
        };
        (
            BarState {
                value: h_value,
                size: if cw > 0.0 {
                    (vw / cw).clamp(0.0, 1.0)
                } else {
                    1.0
                },
            },
            BarState {
                value: v_value,
                size: if ch > 0.0 {
                    (vh / ch).clamp(0.0, 1.0)
                } else {
                    1.0
                },
            },
        )
    }

    /// Sets a bar's value (`ScrollRect.SetNormalizedPosition`).
    pub fn set_bar(&mut self, vertical: bool, value: f32) {
        let g = self.geometry();
        let content = self.content_size();
        let (cw, ch) = (content[0] * self.zoom, content[1] * self.zoom);
        let (vw, vh) = (g.viewport[2], g.viewport[3]);
        let value = value.clamp(0.0, 1.0);
        if vertical {
            if ch > vh {
                self.pos[1] = -value * (ch - vh);
            }
        } else if cw > vw {
            self.pos[0] = -value * (cw - vw) - vw / 2.0 + cw / 2.0;
        }
        self.clamp();
    }

    /// The content's x of the tree's holder (`LineageTreePanel` is inset by the side padding).
    #[must_use]
    pub fn holder_x(x: f32) -> f32 {
        SIDE_PADDING + x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bibites_core::ids::SpeciesId;

    fn info(n: u32, parent: Option<u32>, apparition: i32, disappearance: i32) -> SpeciesInfo {
        SpeciesInfo {
            id: SpeciesId::new(n),
            parent: parent.map(SpeciesId::new),
            root: SpeciesId::new(1),
            name: format!("Genus species{n}"),
            favorite: false,
            apparition,
            disappearance,
            points_present: (apparition - disappearance).max(0) as u32,
        }
    }

    fn infos() -> Vec<SpeciesInfo> {
        vec![
            info(1, None, 150, -1),
            info(2, Some(1), 110, -1),
            info(3, Some(1), 90, -1),
            info(4, Some(2), 60, -1),
            info(5, Some(2), 40, -1),
        ]
    }

    #[test]
    fn opening_fits_the_width_and_rests_on_now() {
        let mut view = View::default();
        view.rebuild(&infos(), TreeSettings::default());
        assert!(view.layout.built);
        assert!(view.zoom > 0.0 && view.zoom <= MAX_ZOOM);
        // The view shows the bottom of the graph ("now") first.
        assert!(view.pos[1].abs() < 0.01, "{:?}", view.pos);
        let g = view.geometry();
        // The content is at least as tall as the viewport, so the vertical bar shows; the zoom fits the width,
        // so no horizontal bar does and the viewport takes its room.
        let content = view.content_size();
        assert!(content[0] * view.zoom <= g.viewport[2] + 0.5);
        assert!(!g.horizontal || content[0] * view.zoom > g.viewport[2]);
        assert_eq!(g.vertical, content[1] * view.zoom > VIEW[3] + 0.01);
    }

    #[test]
    fn the_view_cannot_leave_the_content() {
        let mut view = View::default();
        view.rebuild(&infos(), TreeSettings::default());
        // Scrolling down past "now" does nothing; scrolling up moves 35 units a notch until the top.
        view.scroll(-5.0, false);
        assert!(view.pos[1].abs() < 0.01);
        view.scroll(1.0, false);
        let content = view.content_size();
        let ch = content[1] * view.zoom;
        let vh = view.geometry().viewport[3];
        if ch > vh + 36.0 {
            assert!((view.pos[1] + 35.0).abs() < 0.01 || (view.pos[1] + (ch - vh)).abs() < 0.01);
        }
        for _ in 0..500 {
            view.scroll(1.0, false);
        }
        assert!(
            (view.pos[1] + (ch - vh).max(0.0)).abs() < 0.5,
            "{} of {}",
            view.pos[1],
            ch - vh
        );
        // The bars follow.
        let (_, vertical) = view.bars();
        assert!(vertical.value > 0.99 || ch <= vh);
        // Setting a bar moves the content back.
        view.set_bar(true, 0.0);
        assert!(view.pos[1].abs() < 0.01);
        view.set_bar(true, 1.0);
        assert!((view.pos[1] + (ch - vh).max(0.0)).abs() < 0.5);
    }

    #[test]
    fn zooming_keeps_the_middle_of_the_viewport_and_stays_within_the_limits() {
        let mut view = View::default();
        view.rebuild(&infos(), TreeSettings::default());
        let before = view.zoom;
        view.zoom_by(WHEEL_ZOOM);
        assert!(
            view.zoom >= before - 1e-6,
            "zoom in {before} -> {}",
            view.zoom
        );
        view.zoom_to(100.0);
        assert!(
            (view.zoom - MAX_ZOOM).abs() < 1e-6,
            "the most it can be zoomed in"
        );
        view.zoom_to(0.0001);
        assert!((view.zoom - view.layout.min_zoom).abs() < 1e-6, "the least");
        // However it is zoomed, the content stays within the view.
        for z in [MAX_ZOOM, 1.0, 0.7, view.layout.min_zoom] {
            view.zoom_to(z);
            let g = view.geometry();
            let content = view.content_size();
            let (cw, ch) = (content[0] * view.zoom, content[1] * view.zoom);
            let bottom = view.pos[1];
            assert!(
                bottom <= 0.01 && bottom + ch >= g.viewport[3] - 0.01,
                "zoom {z}: {bottom} {ch}"
            );
            let left = g.viewport[2] / 2.0 + view.pos[0] - cw / 2.0;
            assert!(
                left <= 0.01 && left + cw >= g.viewport[2] - 0.01 || cw < g.viewport[2],
                "zoom {z}"
            );
        }
    }

    #[test]
    fn a_zoomed_in_view_gets_both_bars_and_a_narrower_viewport() {
        let mut view = View::default();
        view.rebuild(&infos(), TreeSettings::default());
        view.zoom_to(MAX_ZOOM);
        let g = view.geometry();
        let content = view.content_size();
        if content[0] * MAX_ZOOM > VIEW[2] - BAR_ROOM {
            assert!(g.horizontal && g.vertical);
            assert!((g.viewport[2] - (VIEW[2] - BAR_ROOM)).abs() < 0.01);
            assert!((g.viewport[3] - (VIEW[3] - BAR_ROOM)).abs() < 0.01);
            let (h, v) = view.bars();
            assert!(h.size < 1.0 && v.size < 1.0);
            // Sideways scrolling with Shift moves the content until the bar is at its end.
            view.set_bar(false, 0.5);
            let mid = view.pos[0];
            view.scroll(1.0, true);
            assert!(
                view.pos[0] > mid,
                "wheel up with Shift moves the content right"
            );
        }
    }

    #[test]
    fn a_right_drag_pans_and_to_screen_follows_the_position() {
        let mut view = View::default();
        view.rebuild(&infos(), TreeSettings::default());
        view.zoom_to(1.0);
        let at = view.to_screen(500.0, 300.0);
        view.pan(0.0, -40.0);
        let moved = view.to_screen(500.0, 300.0);
        assert!((moved[0] - at[0]).abs() < 1e-3);
        // Dragging the mouse up by 40 px moved the content up by as much (until the clamp).
        assert!(moved[1] <= at[1] + 1e-3);
        // The bottom of the content is on the bottom of the viewport at first.
        view.set_bar(true, 0.0);
        let bottom = view.to_screen(0.0, 0.0);
        let g = view.geometry();
        assert!((bottom[1] - (g.viewport[1] + g.viewport[3])).abs() < 0.5);
    }

    /// A report on the layout of a real game save (skipped where the local test data is missing).
    #[test]
    fn a_real_save_lays_out_and_fits() {
        let path = std::path::Path::new("testdata/real/saves/lol.zip");
        let Ok(loaded) = crate::app::LoadedWorld::load(Some(path), 1, 1, 1) else {
            return;
        };
        let entries = loaded.world.species.species_list();
        let infos: Vec<SpeciesInfo> = entries
            .iter()
            .map(|entry| SpeciesInfo {
                id: entry.id,
                parent: entry.parent,
                root: entry.root,
                name: entry.name.clone(),
                favorite: entry.favorite,
                apparition: entry.total_index_of_apparition,
                disappearance: entry.total_index_of_disappearance,
                points_present: entry.n_points_present,
            })
            .collect();
        let mut view = View::default();
        view.rebuild(&infos, TreeSettings::default());
        eprintln!(
            "built {} nodes {} graph {:?} zoom {} (min {}) pos {:?} geometry {:?}",
            view.layout.built,
            view.layout.nodes.len(),
            view.layout.graph_size,
            view.zoom,
            view.layout.min_zoom,
            view.pos,
            view.geometry()
        );
        for (index, node) in view.layout.drawn() {
            eprintln!(
                "  node {index} species {} x {:.1} width {:.1} top {:.1} bottom {:.1} alive {} preview {:.2} fits {} parent {:?}",
                node.species.value(),
                node.x,
                node.assigned_width,
                node.top_height,
                node.bottom_height,
                node.alive,
                node.preview_scale,
                node.can_fit_preview,
                node.parent
            );
        }
        for info in &infos {
            eprintln!(
                "  species {} app {} dis {} pts {} parent {:?}",
                info.id.value(),
                info.apparition,
                info.disappearance,
                info.points_present,
                info.parent.map(|p| p.value())
            );
        }
    }

    #[test]
    fn nothing_to_show_leaves_the_view_unbuilt() {
        let mut view = View::default();
        view.rebuild(&[], TreeSettings::default());
        assert!(!view.layout.built);
        assert!(view.content_size()[1] > 0.0);
    }

    fn shown() -> TreeSettings {
        TreeSettings {
            hide_extinct: false,
            ..TreeSettings::default()
        }
    }

    #[test]
    fn a_short_mesh_fits_the_viewport_without_bars() {
        use crate::lineage_mesh::fixtures::{now_of, three};
        let (registry, ids) = three();
        let mut view = View {
            mesh_mode: true,
            zoom: 0.7,
            pos: [30.0, -40.0],
            ..View::default()
        };
        view.rebuild_mesh(&registry, &now_of(&registry), shown());
        assert!(view.built() && view.mesh.built && !view.layout.built);
        // `SetMinMaxZoom` keeps the zoom within the new limits (a rebuild does not reset the view) and the ScrollRect the short content on
        // the bottom, centred: six logs are shorter than the view, so there are no bars.
        assert_eq!(view.zoom, 1.0);
        assert!(view.pos[0].abs() < 1e-3 && view.pos[1] == 0.0, "{:?}", view.pos);
        let g = view.geometry();
        assert!(!g.vertical && !g.horizontal);
        assert_eq!(g.viewport, VIEW);
        assert_eq!(view.content_size(), [VIEW[2], VIEW[3]]);
        assert_eq!(view.step(), 50.0);
        assert_eq!(view.max_n(), 6);
        assert!(view.shows(ids[0]) && view.shows(ids[2]));
        assert_eq!(view.displayed().len(), 3);
        // The tree's own layout is not used while the mesh shows.
        let tree = View::default();
        assert_eq!(tree.step(), 75.0);
        assert!(!tree.built());
    }

    #[test]
    fn a_tall_mesh_is_made_for_the_viewport_the_vertical_bar_leaves() {
        use crate::lineage_mesh::fixtures::{Life, now_of, registry};
        let (registry, ids) = registry(&[Life {
            parent: None,
            born: 0,
            logs: &[(5, 300.0); 80],
        }]);
        let mut view = View {
            mesh_mode: true,
            ..View::default()
        };
        view.rebuild_mesh(&registry, &now_of(&registry), shown());
        assert!(view.built());
        let content = view.content_size();
        assert!(content[1] > VIEW[3], "taller than the view: {content:?}");
        let g = view.geometry();
        // The vertical bar takes 30 off the width, and the strip is made for what is left: it fills it exactly, so no
        // horizontal bar shows.
        assert!(g.vertical && !g.horizontal);
        assert!((g.viewport[2] - (VIEW[2] - BAR_ROOM)).abs() < 0.01);
        assert!(
            (content[0] - g.viewport[2]).abs() < 0.01,
            "{content:?} {g:?}"
        );
        // The view can zoom out until the whole graph is in it, and no further.
        view.zoom_to(0.0001);
        assert!((view.zoom - view.mesh.min_zoom).abs() < 1e-6);
        let g = view.geometry();
        assert!((content[1] * view.zoom - g.viewport[3]).abs() < 0.5);
        view.zoom_to(100.0);
        assert!((view.zoom - MAX_ZOOM).abs() < 1e-6);
        assert!(view.shows(ids[0]));
    }

    #[test]
    fn a_point_of_the_view_is_a_point_of_the_content_and_back() {
        use crate::lineage_mesh::fixtures::{Life, now_of, registry};
        let (registry, _) = registry(&[Life {
            parent: None,
            born: 0,
            logs: &[(5, 300.0); 80],
        }]);
        let mut view = View {
            mesh_mode: true,
            ..View::default()
        };
        view.rebuild_mesh(&registry, &now_of(&registry), shown());
        for (zoom, bar) in [(1.0, 0.0), (1.3, 0.4), (0.8, 1.0)] {
            view.zoom_to(zoom);
            view.set_bar(true, bar);
            for (x, y) in [(300.0, 200.0), (900.0, 1500.0), (1700.0, 40.0)] {
                let on_screen = view.to_screen(x, y);
                let back = view.to_content(on_screen[0], on_screen[1]);
                assert!(
                    (back[0] - x).abs() < 1e-2 && (back[1] - y).abs() < 1e-2,
                    "zoom {zoom}: {x},{y} -> {on_screen:?} -> {back:?}"
                );
            }
        }
    }

    /// A report on the mesh of a real game save (skipped where the local test data is missing).
    #[test]
    fn a_real_save_makes_a_mesh() {
        let path = std::path::Path::new("testdata/real/saves/lol.zip");
        let Ok(loaded) = crate::app::LoadedWorld::load(Some(path), 1, 1, 1) else {
            return;
        };
        let mut world = loaded.world;
        world.reconcile_species_counts();
        let registry = &world.species;
        let now = world.species_points_now();
        for settings in [shown(), TreeSettings::default()] {
            let mut view = View {
                mesh_mode: true,
                ..View::default()
            };
            view.rebuild_mesh(registry, &now, settings);
            eprintln!(
                "hide_extinct {} -> built {} meshes {} displayed {} size {:?} min zoom {} max n {} max energy {}",
                settings.hide_extinct,
                view.mesh.built,
                view.mesh.meshes.len(),
                view.mesh.displayed.len(),
                view.mesh.size,
                view.mesh.min_zoom,
                view.mesh.max_n,
                registry.max_energy()
            );
            for mesh in view.mesh.meshes.iter().take(30) {
                let genes = registry
                    .get(mesh.species)
                    .map(|record| {
                        let values = record.template.genome.values();
                        [values[5], values[6], values[7]]
                    })
                    .unwrap_or_default();
                eprintln!(
                    "  {} alive {} rungs {} bottom {:.0} top {:.0} width {:.0} preview {:?} blend {:.2} colour {:?} genes {:?}",
                    mesh.name,
                    mesh.alive,
                    mesh.rungs.len(),
                    mesh.bottom_height(),
                    mesh.top_height(),
                    mesh.size[0],
                    mesh.preview.map(|p| p.size),
                    mesh.blend(),
                    mesh.colour,
                    genes
                );
            }
        }
        for record in registry.records().iter().take(30) {
            eprintln!(
                "  record {} count {} energy {:.0} app {} dis {}",
                record.name(),
                record.stats.count,
                record.stats.energy,
                record.stats.log.total_index_of_apparition(),
                record.stats.log.total_index_of_disappearance()
            );
        }
    }

    #[test]
    fn a_mesh_without_history_is_not_built() {
        use crate::lineage_mesh::fixtures::now_of;
        let registry = bibites_core::species::SpeciesRegistry::new();
        let mut view = View {
            mesh_mode: true,
            ..View::default()
        };
        view.rebuild_mesh(&registry, &now_of(&registry), shown());
        assert!(!view.built());
        assert!(view.content_size()[1] > 0.0);
        assert!(view.displayed().is_empty());
    }

    #[test]
    fn the_view_stretches_with_the_canvas_like_the_prefabs_anchors() {
        // `LineageView`: anchors (0, 0) to (1, 1), 15 from the left, 50 from the top, 10 from the right and from the bottom.
        assert_eq!(view_in([1920.0, 1080.0]), VIEW);
        assert_eq!(view_in([2560.0, 1080.0]), [15.0, 50.0, 2535.0, 1020.0]);
        assert_eq!(view_in([1920.0, 1200.0]), [15.0, 50.0, 1895.0, 1140.0]);
        let mut view = View::default();
        assert!(!view.set_canvas([1920.0, 1080.0]), "the capture's canvas is the default");
        assert!(view.set_canvas([2560.0, 1200.0]));
        assert_eq!(view.geometry().view, [15.0, 50.0, 2535.0, 1140.0]);
        assert!(!view.set_canvas([2560.0, 1200.0]), "nothing changed");
        // The bars' tracks keep to the view's right and bottom edges.
        let g = view.geometry();
        let vertical = vertical_track(g);
        assert_eq!(vertical[0] + vertical[2], 15.0 + 2535.0);
        assert_eq!(vertical[1] + vertical[3], 50.0 + 1140.0 - BAR_ROOM);
        let horizontal = horizontal_track(g);
        assert_eq!(horizontal[1] + horizontal[3], 50.0 + 1140.0);
        // A canvas smaller than the bars' room still leaves a view.
        assert!(view_in([10.0, 10.0])[2] > 0.0 && view_in([10.0, 10.0])[3] > 0.0);
    }
}
