//! The captured screens in a window that is not the capture's: the layout of the original game at another canvas size.
//!
//! The captures (`ui_runtime_native`) hold every node's rectangle for a canvas of 1920 x 1080 reference px. The canvas is the window over the UI scale
//! (`UIScaler`), so a window that is not 16:9, or a UI scale other than 1, is a canvas of another shape; the game's `RectTransform` anchors and its layout
//! groups then give every panel another place and another size (the settings page grows wider with a wide window, the title screen's buttons stay in their
//! column, the scenario list takes the width the window has more...). Moving a whole captured subtree by one anchor's share of the difference (what the
//! frame composer did) moves the panels but never resizes one.
//!
//! This solves the new rectangles from the captured ones, the way `UnityEngine.UI` does it, as a *change* from the captured solution: what the captured
//! solution has fixed stays fixed (a text's preferred size, a button's size), and what the canvas decides moves.
//! - A node's rectangle is `RectTransform`'s: its anchors in the parent's new rectangle, its margins kept (stretched anchors) or its offset from the anchor
//!   point kept (point anchors); a `ContentSizeFitter` keeps the size and the pivot's place; an `AspectRatioFitter` derives one size from the other.
//! - A `HorizontalLayoutGroup` / `VerticalLayoutGroup` shares what its size has gained (or lost) among its children by their flexible sizes (`LayoutElement`,
//!   or the group's own; `childForceExpand` makes every child at least 1), moves the children that are not flexible by the group's alignment, and gives the
//!   children it controls the whole cross axis when they are flexible there.
//! - A `GridLayoutGroup` fits as many cells as the width holds and aligns the rows.
//!
//! What it does not know: the text of a node that wraps differently in another width (its height), and nodes the game makes at run time (graph ticks, bars that
//! follow a bibite). Measured against the original game's own rectangles at 2560 x 1080 (`tools/ui`, `assets/game/reference/layout_gt`) 97.8 % of the nodes that
//! are not those are within 1.5 px.

use std::collections::HashMap;

use serde_json::Value;

use crate::ui_runtime::Node;

/// A rectangle in canvas units, y up: `[x0, y0, x1, y1]`.
pub type Rect = [f32; 4];

/// The layout components of a node, as captured.
#[derive(Clone, Debug, Default)]
pub struct LayoutInfo {
    /// `HorizontalLayoutGroup` / `VerticalLayoutGroup`.
    pub group: Option<Group>,
    /// `GridLayoutGroup`.
    pub grid: Option<Grid>,
    /// `LayoutElement`: flexible width and height (-1: not set), `ignoreLayout`.
    pub element: Option<Element>,
    /// `ContentSizeFitter` on the horizontal and the vertical axis (preferred or minimum size).
    pub fit: [bool; 2],
    /// `AspectRatioFitter`: the mode and the ratio.
    pub aspect: Option<(Aspect, f32)>,
    /// `ScrollRect` that scrolls up and down: the name of its content node.
    pub scroll: Option<String>,
    /// A `Scrollbar` component.
    pub scrollbar: bool,
}

/// `HorizontalLayoutGroup` / `VerticalLayoutGroup`.
#[derive(Clone, Copy, Debug)]
pub struct Group {
    /// `VerticalLayoutGroup`.
    pub vertical: bool,
    /// Padding: left, right, top, bottom.
    pub padding: [f32; 4],
    /// Space between two children.
    pub spacing: f32,
    /// Alignment of the children on the x and the y axis (0 at the left / top, 1 at the right / bottom).
    pub align: [f32; 2],
    /// `childControlWidth`, `childControlHeight`.
    pub control: [bool; 2],
    /// `childForceExpandWidth`, `childForceExpandHeight`.
    pub expand: [bool; 2],
    /// `reverseArrangement`.
    pub reverse: bool,
}

/// `GridLayoutGroup`.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    /// Padding: left, right, top, bottom.
    pub padding: [f32; 4],
    /// Cell size.
    pub cell: [f32; 2],
    /// Space between cells.
    pub spacing: [f32; 2],
    /// Alignment of the rows on the x axis.
    pub align_x: f32,
    /// A fixed number of columns (`constraint` 1), if the constraint is one.
    pub columns: Option<usize>,
}

/// `LayoutElement`.
#[derive(Clone, Copy, Debug)]
pub struct Element {
    /// Flexible width, height (-1: not set).
    pub flexible: [f32; 2],
    /// `ignoreLayout`.
    pub ignore: bool,
}

/// `AspectRatioFitter.AspectMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aspect {
    /// The height follows the width.
    WidthControlsHeight,
    /// The width follows the height.
    HeightControlsWidth,
    /// The biggest rectangle of the ratio that fits in the parent.
    FitInParent,
    /// The smallest rectangle of the ratio that covers the parent.
    EnvelopeParent,
}

fn alignment(name: &str) -> [f32; 2] {
    match name {
        "UpperCenter" => [0.5, 0.0],
        "UpperRight" => [1.0, 0.0],
        "MiddleLeft" => [0.0, 0.5],
        "MiddleCenter" => [0.5, 0.5],
        "MiddleRight" => [1.0, 0.5],
        "LowerLeft" => [0.0, 1.0],
        "LowerCenter" => [0.5, 1.0],
        "LowerRight" => [1.0, 1.0],
        _ => [0.0, 0.0],
    }
}

fn number(component: &Value, key: &str) -> Option<f32> {
    component.get(key).and_then(Value::as_f64).map(|value| value as f32)
}

fn flag(component: &Value, key: &str) -> bool {
    component.get(key).and_then(Value::as_bool).unwrap_or(false)
}

impl LayoutInfo {
    /// Takes a captured component in when it is one of the layout components (any other is left alone).
    pub fn absorb(&mut self, kind: &str, component: &Value) {
        if component.get("enabled").and_then(Value::as_bool) == Some(false) {
            return;
        }
        let padding = |component: &Value| {
            let pad = component.get("padding");
            let side = |key: &str| pad.and_then(|pad| number(pad, key)).unwrap_or(0.0);
            [side("left"), side("right"), side("top"), side("bottom")]
        };
        match kind {
            "UnityEngine.UI.HorizontalLayoutGroup" | "UnityEngine.UI.VerticalLayoutGroup" => {
                self.group = Some(Group {
                    vertical: kind.ends_with("VerticalLayoutGroup"),
                    padding: padding(component),
                    spacing: number(component, "spacing").unwrap_or(0.0),
                    align: alignment(component.get("child_alignment").and_then(Value::as_str).unwrap_or("")),
                    control: [flag(component, "child_control_width"), flag(component, "child_control_height")],
                    expand: [flag(component, "child_force_expand_width"), flag(component, "child_force_expand_height")],
                    reverse: flag(component, "reverse_arrangement"),
                });
            }
            "UnityEngine.UI.GridLayoutGroup" => {
                let pair = |key: &str| {
                    component
                        .get(key)
                        .map(|pair| [number(pair, "x").unwrap_or(0.0), number(pair, "y").unwrap_or(0.0)])
                        .unwrap_or([0.0; 2])
                };
                let fixed = component.get("constraint").and_then(Value::as_str) == Some("FixedColumnCount");
                self.grid = Some(Grid {
                    padding: padding(component),
                    cell: pair("cell_size"),
                    spacing: pair("spacing"),
                    align_x: alignment(component.get("child_alignment").and_then(Value::as_str).unwrap_or(""))[0],
                    columns: fixed.then(|| component.get("constraint_count").and_then(Value::as_u64).unwrap_or(1).max(1) as usize),
                });
            }
            "UnityEngine.UI.LayoutElement" => {
                self.element = Some(Element {
                    flexible: [number(component, "flexible_width").unwrap_or(-1.0), number(component, "flexible_height").unwrap_or(-1.0)],
                    ignore: flag(component, "ignore_layout"),
                });
            }
            "UnityEngine.UI.ContentSizeFitter" => {
                let fits = |key: &str| matches!(component.get(key).and_then(Value::as_str), Some("PreferredSize" | "MinSize"));
                self.fit = [fits("horizontal_fit"), fits("vertical_fit")];
            }
            "UnityEngine.UI.AspectRatioFitter" => {
                let mode = match component.get("aspect_mode").and_then(Value::as_str) {
                    Some("WidthControlsHeight") => Some(Aspect::WidthControlsHeight),
                    Some("HeightControlsWidth") => Some(Aspect::HeightControlsWidth),
                    Some("FitInParent") => Some(Aspect::FitInParent),
                    Some("EnvelopeParent") => Some(Aspect::EnvelopeParent),
                    _ => None,
                };
                self.aspect = mode.map(|mode| (mode, number(component, "aspect_ratio").unwrap_or(1.0)));
            }
            "UnityEngine.UI.ScrollRect" => {
                self.scroll = flag(component, "vertical").then(|| component.get("content").and_then(Value::as_str).unwrap_or("Content").to_owned());
            }
            "UnityEngine.UI.Scrollbar" => self.scrollbar = true,
            _ => {}
        }
    }
}

/// The new rectangles of the nodes of one or more canvas trees in a canvas of another size.
#[derive(Debug, Default)]
pub struct Relaid {
    rects: HashMap<usize, Rect>,
    /// The scrollbars the game hides in this canvas: a list that fits its (taller) view has none (`ScrollRect.verticalScrollbarVisibility` is
    /// `AutoHideAndExpandViewport`), and its viewport takes the room the bar had.
    hidden: std::collections::HashSet<usize>,
}

impl Relaid {
    /// The new rectangle of `node` (y up, canvas units), if it has one (a node the capture has no rectangle of has none: its parent's decides).
    #[must_use]
    pub fn rect_of(&self, node: &Node) -> Option<Rect> {
        self.rects.get(&(node as *const Node as usize)).copied()
    }

    /// Whether the layout hides `node`, a scrollbar the content of its view does not need in this canvas.
    #[must_use]
    pub fn hides(&self, node: &Node) -> bool {
        self.hidden.contains(&(node as *const Node as usize))
    }

    /// Whether no node has one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
}

/// The capture's canvas: what every old rectangle is laid out for.
pub const CAPTURE_CANVAS: [f32; 2] = [1920.0, 1080.0];

/// Lays the canvas trees `roots` out again for a canvas of `canvas` units. A canvas of the capture's own size (within half a unit) gives nothing: the captured
/// rectangles are right.
#[must_use]
pub fn relayout(roots: &[&Node], canvas: [f32; 2]) -> Relaid {
    let mut relaid = Relaid::default();
    if (canvas[0] - CAPTURE_CANVAS[0]).abs() < 0.5 && (canvas[1] - CAPTURE_CANVAS[1]).abs() < 0.5 {
        return relaid;
    }
    let screen = [0.0, 0.0, canvas[0], canvas[1]];
    for root in roots {
        let Some(old) = root.native_rect else {
            continue;
        };
        // A root that was the whole screen is the whole canvas now; any other keeps its rectangle.
        let full = (old[2] - old[0] - CAPTURE_CANVAS[0]).abs() < 2.0 && (old[3] - old[1] - CAPTURE_CANVAS[1]).abs() < 2.0;
        solve(&mut relaid, root, if full { screen } else { old });
    }
    relaid
}

fn width(rect: Rect, axis: usize) -> f32 {
    rect[axis + 2] - rect[axis]
}

fn solve(relaid: &mut Relaid, node: &Node, new: Rect) {
    relaid.rects.insert(node as *const Node as usize, new);
    let Some(old) = node.native_rect else {
        return;
    };
    let mut placed: Vec<Option<Rect>> = vec![None; node.children.len()];
    if node.active_in_hierarchy {
        if let Some(grid) = &node.layout.grid {
            place_grid(node, grid, new, &mut placed);
        } else if let Some(group) = &node.layout.group {
            place_group(node, group, old, new, &mut placed);
        }
        auto_hide_scrollbar(relaid, node, old, new, &mut placed);
    }
    for (index, child) in node.children.iter().enumerate() {
        if child.native_rect.is_none() {
            continue;
        }
        let rect = placed[index].unwrap_or_else(|| place(child, old, new));
        solve(relaid, child, rect);
    }
}

/// A scroll view whose content fits its view in this canvas has no scrollbar and its viewport is as wide as the view (`AutoHideAndExpandViewport`: the
/// viewport is `-(bar width + spacing)` narrower only while the bar is there). Applies where the capture shows the bar and the content as long as it
/// was captured (a list the viewer fills itself decides for itself).
fn auto_hide_scrollbar(relaid: &mut Relaid, node: &Node, old: Rect, new: Rect, placed: &mut [Option<Rect>]) {
    let Some(content_name) = &node.layout.scroll else {
        return;
    };
    let named = |child: &Node, name: &str| child.path.rsplit('/').next() == Some(name);
    let Some(bar) = node.children.iter().position(|child| child.layout.scrollbar && child.active && child.native_rect.is_some_and(|rect| width(rect, 1) > width(rect, 0))) else {
        return;
    };
    let Some(view) = node.children.iter().position(|child| child.native_rect.is_some() && child.children.iter().any(|inner| named(inner, content_name))) else {
        return;
    };
    let (view_node, bar_node) = (&node.children[view], &node.children[bar]);
    let (Some(view_old), Some(content_old)) = (view_node.native_rect, view_node.children.iter().find(|inner| named(inner, content_name)).and_then(|inner| inner.native_rect)) else {
        return;
    };
    // What the bar took from the view's width as captured.
    let narrowed = width(old, 0) - width(view_old, 0);
    if narrowed < 0.5 {
        return;
    }
    let mut view_new = placed[view].unwrap_or_else(|| place(view_node, old, new));
    if width(content_old, 1) > width(view_new, 1) + 0.01 {
        return;
    }
    view_new[2] += narrowed;
    placed[view] = Some(view_new);
    relaid.hidden.insert(bar_node as *const Node as usize);
}

/// The children a layout group lays out: active, not ignoring the layout, and with a captured rectangle.
fn laid_out(node: &Node) -> Vec<usize> {
    node.children
        .iter()
        .enumerate()
        .filter(|(_, child)| child.active && child.native_rect.is_some() && !child.layout.element.is_some_and(|element| element.ignore))
        .map(|(index, _)| index)
        .collect()
}

/// A node's flexible size along `axis` (0 across, 1 down): its `LayoutElement`'s, else its own group's.
fn flexible(node: &Node, axis: usize) -> f32 {
    if let Some(element) = &node.layout.element
        && element.flexible[axis] >= 0.0
    {
        return element.flexible[axis];
    }
    node.layout.group.as_ref().map_or(0.0, |group| group_flexible(node, group, axis))
}

fn child_flexible(group: &Group, child: &Node, axis: usize) -> f32 {
    let size = if group.control[axis] { flexible(child, axis) } else { 0.0 };
    if group.expand[axis] { size.max(1.0) } else { size }
}

fn group_flexible(node: &Node, group: &Group, axis: usize) -> f32 {
    let kids = laid_out(node);
    if kids.is_empty() {
        return 0.0;
    }
    let along = (axis == 1) == group.vertical;
    let values = kids.iter().map(|&index| child_flexible(group, &node.children[index], axis));
    if along { values.sum() } else { values.fold(0.0, f32::max) }
}

fn place_group(node: &Node, group: &Group, old: Rect, new: Rect, placed: &mut [Option<Rect>]) {
    let mut kids = laid_out(node);
    if kids.is_empty() {
        return;
    }
    if group.reverse {
        kids.reverse();
    }
    let main = usize::from(group.vertical);
    let cross = 1 - main;
    let [left, right, top, bottom] = group.padding;
    let (pad_lo, pad_hi) = if group.vertical { (top, bottom) } else { (left, right) };
    let (cross_lo, cross_hi) = if group.vertical { (left, right) } else { (top, bottom) };
    let (old_size, new_size) = (width(old, main), width(new, main));
    let rects: Vec<Rect> = kids.iter().map(|&index| node.children[index].native_rect.unwrap_or(old)).collect();
    let old_sizes: Vec<f32> = rects.iter().map(|rect| width(*rect, main)).collect();
    let flexes: Vec<f32> = kids.iter().map(|&index| child_flexible(group, &node.children[index], main)).collect();
    let total_flexible: f32 = flexes.iter().sum();
    let gaps = group.spacing * (kids.len() - 1) as f32;
    // What the children take with no surplus (their own sizes): the group's size over that is its surplus, which is shared among the flexible ones.
    let preferred_total = old_sizes.iter().sum::<f32>() + gaps + pad_lo + pad_hi;
    let align_main = if group.vertical { group.align[1] } else { group.align[0] };
    let controlled = group.control[main];
    let sizes_hold_the_surplus = controlled && total_flexible > 0.0;
    // The cell of every child (the size the group gives it) as it was captured and as it is now.
    let (cells_old, cells_new, start_shift): (Vec<f32>, Vec<f32>, f32) = if total_flexible > 0.0 {
        if sizes_hold_the_surplus {
            // The flexible children were given the surplus the group had: their captured sizes hold it, and what the size gained is shared by the flexible sizes.
            let gained = new_size - old_size;
            (old_sizes.clone(), old_sizes.iter().zip(&flexes).map(|(size, flex)| size + flex * gained / total_flexible).collect(), 0.0)
        } else {
            // Children the group does not size sit in cells that hold the surplus, and stay the size they are.
            let share_old = (old_size - preferred_total).max(0.0) / total_flexible;
            let share_new = (new_size - preferred_total).max(0.0) / total_flexible;
            (
                old_sizes.iter().zip(&flexes).map(|(size, flex)| size + flex * share_old).collect(),
                old_sizes.iter().zip(&flexes).map(|(size, flex)| size + flex * share_new).collect(),
                0.0,
            )
        }
    } else {
        // Nothing is flexible: the children move by the group's alignment of what its size gained.
        (old_sizes.clone(), old_sizes.clone(), align_main * ((new_size - preferred_total).max(0.0) - (old_size - preferred_total).max(0.0)))
    };
    let inner_new = (width(new, cross) - cross_lo - cross_hi).max(0.0);
    let inner_old = (width(old, cross) - cross_lo - cross_hi).max(0.0);
    let align_cross = if group.vertical { group.align[0] } else { group.align[1] };
    let mut gained_before = 0.0;
    for (slot, &index) in kids.iter().enumerate() {
        let child = &node.children[index];
        let child_old = rects[slot];
        // Where the child is from the group's start edge (the left, or the top), as captured, and now.
        let old_position = if main == 0 { child_old[0] - old[0] } else { old[3] - child_old[3] };
        let size = if controlled { cells_new[slot] } else { old_sizes[slot] };
        let inside_cell = if controlled { 0.0 } else { (cells_new[slot] - cells_old[slot]) * align_main };
        let position = old_position + start_shift + gained_before + inside_cell;
        gained_before += cells_new[slot] - cells_old[slot];
        let along = if main == 0 { [new[0] + position, new[0] + position + size] } else { [new[3] - position - size, new[3] - position] };
        // The cross axis: a child the group sizes and that is flexible there fills the group; the others keep their size and their place by the alignment.
        let fills = group.control[cross] && child_flexible(group, child, cross) > 0.0;
        let cross_old = width(child_old, cross);
        let cross_size = if fills { inner_new } else { cross_old };
        let old_offset = if cross == 0 { child_old[0] - old[0] } else { old[3] - child_old[3] };
        let offset = if fills { cross_lo } else { old_offset + align_cross * ((inner_new - cross_size) - (inner_old - cross_old)) };
        let across = if cross == 0 { [new[0] + offset, new[0] + offset + cross_size] } else { [new[3] - offset - cross_size, new[3] - offset] };
        placed[index] = Some(if main == 0 { [along[0], across[0], along[1], across[1]] } else { [across[0], along[0], across[1], along[1]] });
    }
}

fn place_grid(node: &Node, grid: &Grid, new: Rect, placed: &mut [Option<Rect>]) {
    let kids = laid_out(node);
    if kids.is_empty() {
        return;
    }
    let [left, right, top, _] = grid.padding;
    let inner = width(new, 0) - left - right;
    let fitting = (((inner + grid.spacing[0] + 0.001) / (grid.cell[0] + grid.spacing[0])).floor() as usize).max(1);
    let columns = grid.columns.unwrap_or(fitting);
    let used = columns.min(kids.len());
    let used_width = used as f32 * grid.cell[0] + used.saturating_sub(1) as f32 * grid.spacing[0];
    let x_start = new[0] + left + (inner - used_width) * grid.align_x;
    for (slot, &index) in kids.iter().enumerate() {
        let (column, row) = (slot % columns, slot / columns);
        let x0 = x_start + column as f32 * (grid.cell[0] + grid.spacing[0]);
        let y1 = new[3] - top - row as f32 * (grid.cell[1] + grid.spacing[1]);
        placed[index] = Some([x0, y1 - grid.cell[1], x0 + grid.cell[0], y1]);
    }
}

/// `child`'s rectangle in a parent that was `old` and is `new`, by `RectTransform`'s rules (anchors, pivot) and the fitters.
fn place(child: &Node, old: Rect, new: Rect) -> Rect {
    let Some(child_old) = child.native_rect else {
        return new;
    };
    let rect = &child.rect;
    let anchors = [(rect.anchor_min.x, rect.anchor_max.x), (rect.anchor_min.y, rect.anchor_max.y)];
    let pivot = [rect.pivot.x, rect.pivot.y];
    let layout = &child.layout;
    if let Some((mode @ (Aspect::FitInParent | Aspect::EnvelopeParent), ratio)) = layout.aspect {
        let ratio = if ratio > 0.0 { ratio } else { 1.0 };
        let (parent_w, parent_h) = (width(new, 0), width(new, 1));
        let sizes = if (parent_h * ratio < parent_w) ^ (mode == Aspect::FitInParent) { [parent_w, parent_w / ratio] } else { [parent_h * ratio, parent_h] };
        let mut out = [0.0; 4];
        for axis in 0..2 {
            let low = new[axis] + pivot[axis] * width(new, axis) - pivot[axis] * sizes[axis];
            out[axis] = low;
            out[axis + 2] = low + sizes[axis];
        }
        return out;
    }
    let axis_rect = |axis: usize, size: Option<f32>| -> (f32, f32) {
        let (min, max) = anchors[axis];
        let stretched = (max - min).abs() > 1e-6;
        let (old_lo, old_hi) = (child_old[axis], child_old[axis + 2]);
        let (parent_old_lo, parent_old_hi) = (old[axis], old[axis + 2]);
        let (parent_new_lo, parent_new_hi) = (new[axis], new[axis + 2]);
        let a0_old = parent_old_lo + min * (parent_old_hi - parent_old_lo);
        let a1_old = parent_old_lo + max * (parent_old_hi - parent_old_lo);
        let a0_new = parent_new_lo + min * (parent_new_hi - parent_new_lo);
        let a1_new = parent_new_lo + max * (parent_new_hi - parent_new_lo);
        if layout.fit[axis] || size.is_some() {
            let size = size.unwrap_or(old_hi - old_lo);
            let reference_old = old_lo + pivot[axis] * (old_hi - old_lo);
            let reference_new = if stretched {
                a0_new + pivot[axis] * (a1_new - a0_new) + (reference_old - (a0_old + pivot[axis] * (a1_old - a0_old)))
            } else {
                a0_new + (reference_old - a0_old)
            };
            let low = reference_new - pivot[axis] * size;
            return (low, low + size);
        }
        if stretched {
            (a0_new + (old_lo - a0_old), a1_new + (old_hi - a1_old))
        } else {
            (old_lo + (a0_new - a0_old), old_hi + (a0_new - a0_old))
        }
    };
    let old_ratio = width(child_old, 0) / width(child_old, 1).max(1e-6);
    let mut out = [0.0; 4];
    match layout.aspect {
        Some((Aspect::HeightControlsWidth, ratio)) => {
            (out[1], out[3]) = axis_rect(1, None);
            (out[0], out[2]) = axis_rect(0, Some((out[3] - out[1]) * if ratio > 0.0 { ratio } else { old_ratio }));
        }
        Some((Aspect::WidthControlsHeight, ratio)) => {
            (out[0], out[2]) = axis_rect(0, None);
            (out[1], out[3]) = axis_rect(1, Some((out[2] - out[0]) / if ratio > 0.0 { ratio } else { old_ratio }));
        }
        _ => {
            (out[0], out[2]) = axis_rect(0, None);
            (out[1], out[3]) = axis_rect(1, None);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_runtime::{Fraction, Point};

    fn node(path: &str, bbox: Rect, anchors: ([f32; 2], [f32; 2])) -> Node {
        let mut node = Node { path: path.to_owned(), native_rect: Some(bbox), active: true, active_in_hierarchy: true, ..Node::default() };
        node.rect.anchor_min = Fraction { x: anchors.0[0], y: anchors.0[1] };
        node.rect.anchor_max = Fraction { x: anchors.1[0], y: anchors.1[1] };
        node.rect.pivot = Fraction { x: 0.5, y: 0.5 };
        node.rect.anchored_position = Point::default();
        node
    }

    fn canvas_root(children: Vec<Node>) -> Node {
        let mut root = node("UI", [0.0, 0.0, 1920.0, 1080.0], ([0.0, 0.0], [1.0, 1.0]));
        root.children = children;
        root
    }

    fn rect_in(relaid: &Relaid, root: &Node, path: &[usize]) -> Rect {
        let mut at = root;
        for &index in path {
            at = &at.children[index];
        }
        relaid.rect_of(at).expect("the node has a new rectangle")
    }

    #[test]
    fn a_canvas_of_the_captures_size_changes_nothing() {
        let root = canvas_root(vec![node("UI/a", [10.0, 10.0, 100.0, 100.0], ([0.0, 0.0], [0.0, 0.0]))]);
        assert!(relayout(&[&root], [1920.0, 1080.0]).is_empty());
        assert!(relayout(&[&root], [1920.2, 1079.8]).is_empty(), "within half a unit");
        assert!(!relayout(&[&root], [2560.0, 1080.0]).is_empty());
    }

    #[test]
    fn the_anchors_decide_where_a_panel_goes_in_a_wider_and_taller_canvas() {
        // Left, right and top-centre point anchors, a corner at the bottom right, a panel stretched across.
        let root = canvas_root(vec![
            node("UI/left", [10.0, 900.0, 110.0, 1000.0], ([0.0, 1.0], [0.0, 1.0])),
            node("UI/right", [1810.0, 900.0, 1910.0, 1000.0], ([1.0, 1.0], [1.0, 1.0])),
            node("UI/centre", [910.0, 1000.0, 1010.0, 1060.0], ([0.5, 1.0], [0.5, 1.0])),
            node("UI/corner", [1800.0, 20.0, 1900.0, 70.0], ([1.0, 0.0], [1.0, 0.0])),
            node("UI/bar", [20.0, 500.0, 1900.0, 540.0], ([0.0, 0.5], [1.0, 0.5])),
        ]);
        let relaid = relayout(&[&root], [2560.0, 1200.0]);
        assert_eq!(rect_in(&relaid, &root, &[]), [0.0, 0.0, 2560.0, 1200.0], "the root is the canvas");
        // Wider by 640 and taller by 120: the left stays, the right moves 640, the centre 320; the top ones move up 120, the bottom corner only right.
        assert_eq!(rect_in(&relaid, &root, &[0]), [10.0, 1020.0, 110.0, 1120.0]);
        assert_eq!(rect_in(&relaid, &root, &[1]), [2450.0, 1020.0, 2550.0, 1120.0]);
        assert_eq!(rect_in(&relaid, &root, &[2]), [1230.0, 1120.0, 1330.0, 1180.0]);
        assert_eq!(rect_in(&relaid, &root, &[3]), [2440.0, 20.0, 2540.0, 70.0]);
        // A stretched panel keeps its margins (20 left, 20 right) and, centred at 0.5 on y, moves up half the new height.
        assert_eq!(rect_in(&relaid, &root, &[4]), [20.0, 560.0, 2540.0, 600.0]);
    }

    #[test]
    fn a_content_size_fitter_keeps_the_size_and_the_place_in_the_middle() {
        let mut holder = node("UI/holder", [300.0, 400.0, 1620.0, 700.0], ([0.0, 0.0], [1.0, 1.0]));
        holder.layout.fit = [true, true];
        let root = canvas_root(vec![holder]);
        let relaid = relayout(&[&root], [2560.0, 1080.0]);
        // The anchors stretch over the screen, but the holder is as big as its content: it stays 1320 wide and stays in the middle.
        let got = rect_in(&relaid, &root, &[0]);
        assert_eq!(got[2] - got[0], 1320.0);
        assert!(((got[0] + got[2]) / 2.0 - 1280.0).abs() < 1e-3, "{got:?}");
    }

    fn group_of(vertical: bool, align: [f32; 2], control: [bool; 2], expand: [bool; 2]) -> Group {
        Group { vertical, padding: [0.0; 4], spacing: 10.0, align, control, expand, reverse: false }
    }

    #[test]
    fn a_horizontal_group_gives_what_the_canvas_has_more_to_its_flexible_child_and_pushes_the_ones_after_it() {
        // A list (flexible) and a column of buttons (fixed), 1900 wide with spacing 10: the list takes the 640 the window has more.
        let mut group = node("UI/row", [10.0, 10.0, 1910.0, 1070.0], ([0.0, 0.0], [1.0, 1.0]));
        group.layout.group = Some(group_of(false, [0.0, 0.0], [true, true], [false, false]));
        let mut list = node("UI/row/list", [10.0, 10.0, 1410.0, 1070.0], ([0.0, 0.0], [0.0, 1.0]));
        list.layout.element = Some(Element { flexible: [1.0, -1.0], ignore: false });
        let buttons = node("UI/row/buttons", [1420.0, 10.0, 1910.0, 1070.0], ([0.0, 0.0], [0.0, 1.0]));
        group.children = vec![list, buttons];
        let root = canvas_root(vec![group]);
        let relaid = relayout(&[&root], [2560.0, 1080.0]);
        // The row stretches over the canvas (margins 10); the list grows from 1400 to 2040 wide; the buttons keep 490 and sit at its end.
        assert_eq!(rect_in(&relaid, &root, &[0]), [10.0, 10.0, 2550.0, 1070.0]);
        assert_eq!(rect_in(&relaid, &root, &[0, 0]), [10.0, 10.0, 2050.0, 1070.0]);
        assert_eq!(rect_in(&relaid, &root, &[0, 1]), [2060.0, 10.0, 2550.0, 1070.0]);
    }

    #[test]
    fn children_a_group_does_not_size_sit_in_their_cell_at_the_groups_alignment() {
        // `childForceExpand` with no control: each of the two buttons gets half of the row as its cell and stays the size it was, centred in it.
        let mut row = node("UI/row", [0.0, 100.0, 1000.0, 200.0], ([0.0, 0.0], [1.0, 0.0]));
        row.layout.group = Some(Group { padding: [0.0; 4], spacing: 0.0, ..group_of(false, [0.5, 0.5], [false, false], [true, true]) });
        let a = node("UI/row/a", [200.0, 120.0, 300.0, 180.0], ([0.0, 0.0], [0.0, 0.0]));
        let b = node("UI/row/b", [700.0, 120.0, 800.0, 180.0], ([0.0, 0.0], [0.0, 0.0]));
        row.children = vec![a, b];
        let mut root = node("UI", [0.0, 0.0, 1920.0, 1080.0], ([0.0, 0.0], [1.0, 1.0]));
        root.children = vec![row];
        let relaid = relayout(&[&root], [2560.0, 1080.0]);
        // The row (anchors 0..1 across) is 1640 wide now (margin 920 right): the cells are 820 wide: 100 wide buttons centred in them.
        let row_rect = rect_in(&relaid, &root, &[0]);
        assert_eq!(row_rect[2] - row_rect[0], 1640.0);
        let first = rect_in(&relaid, &root, &[0, 0]);
        let second = rect_in(&relaid, &root, &[0, 1]);
        assert_eq!((first[2] - first[0], second[2] - second[0]), (100.0, 100.0));
        assert!((first[0] - (row_rect[0] + 360.0)).abs() < 1e-3 && (second[0] - (row_rect[0] + 1180.0)).abs() < 1e-3, "{first:?} {second:?} {row_rect:?}");
    }

    #[test]
    fn a_grid_fits_as_many_cells_as_the_width_holds_and_centres_the_rows() {
        let mut grid = node("UI/grid", [0.0, 0.0, 1000.0, 600.0], ([0.0, 0.0], [1.0, 1.0]));
        grid.layout.grid = Some(Grid { padding: [25.0, 25.0, 84.0, 25.0], cell: [128.0, 128.0], spacing: [32.0, 32.0], align_x: 0.5, columns: None });
        grid.children = (0..5).map(|n| node(&format!("UI/grid/{n}"), [0.0, 0.0, 128.0, 128.0], ([0.0, 1.0], [0.0, 1.0]))).collect();
        let root = canvas_root(vec![grid]);
        let relaid = relayout(&[&root], [2560.0, 1080.0]);
        // The grid's own rectangle is stretched: 1640 wide now (its margin at the right is 920) and 600 high, so 1590 inside the padding: (1590 + 32) / 160 = 10
        // cells fit, 5 are used: 5 * 128 + 4 * 32 = 768 wide, centred; the first row is 84 under the top.
        let first = rect_in(&relaid, &root, &[0, 0]);
        let last = rect_in(&relaid, &root, &[0, 4]);
        assert!((first[0] - (25.0 + (1590.0 - 768.0) / 2.0)).abs() < 1e-3, "{first:?}");
        assert_eq!(first[3], 600.0 - 84.0);
        assert!((last[0] - (first[0] + 4.0 * 160.0)).abs() < 1e-3 && last[1] == first[1], "one row: {last:?}");
    }

    #[test]
    fn an_aspect_ratio_fitter_keeps_its_ratio_in_a_parent_of_another_shape() {
        let mut picture = node("UI/picture", [100.0, 100.0, 500.0, 500.0], ([0.0, 0.0], [1.0, 1.0]));
        picture.layout.aspect = Some((Aspect::FitInParent, 1.0));
        let root = canvas_root(vec![picture]);
        let relaid = relayout(&[&root], [2560.0, 1080.0]);
        let got = rect_in(&relaid, &root, &[0]);
        // The parent is 2560 x 1080: the square that fits is 1080 across, in the middle.
        assert_eq!((got[2] - got[0], got[3] - got[1]), (1080.0, 1080.0));
        assert!(((got[0] + got[2]) / 2.0 - 1280.0).abs() < 1e-3, "{got:?}");
    }

    #[test]
    fn a_scroll_view_whose_content_fits_a_taller_view_loses_its_scrollbar_and_widens_its_viewport() {
        // A view 100 wide and 900 high at the left, stretched from 180 above the bottom to the top; its viewport is 20 short of it (the bar's room), the content 1000 high is captured as taller than the
        // view; the bar is a tall thin child at the view's right edge.
        let scroll_view = |content_height: f32| {
            let mut view = node("UI/List/Scroll View", [0.0, 180.0, 100.0, 1080.0], ([0.0, 0.0], [0.0, 1.0]));
            view.layout.scroll = Some("Content".to_owned());
            let mut viewport = node("UI/List/Scroll View/Viewport", [0.0, 180.0, 80.0, 1080.0], ([0.0, 0.0], [1.0, 1.0]));
            let mut content = node("UI/List/Scroll View/Viewport/Content", [0.0, 1080.0 - content_height, 80.0, 1080.0], ([0.0, 1.0], [1.0, 1.0]));
            content.rect.pivot = Fraction { x: 0.5, y: 1.0 };
            viewport.children.push(content);
            let mut bar = node("UI/List/Scroll View/Scrollbar Vertical", [80.0, 180.0, 100.0, 1080.0], ([1.0, 0.0], [1.0, 1.0]));
            bar.layout.scrollbar = true;
            view.children = vec![viewport, bar];
            canvas_root(vec![view])
        };
        // The content is longer than the view in a canvas 1200 high as well as in one 1080 high: the bar stays.
        let long = scroll_view(1400.0);
        let relaid = relayout(&[&long], [1920.0, 1200.0]);
        assert!(!relaid.hides(&long.children[0].children[1]));
        // A content 1000 high fits the view of a canvas 1200 high (the view grows by 120 to 1020): the bar goes and the viewport takes its 20.
        let fits = scroll_view(1000.0);
        let relaid = relayout(&[&fits], [1920.0, 1200.0]);
        assert!(relaid.hides(&fits.children[0].children[1]));
        let viewport = rect_in(&relaid, &fits, &[0, 0]);
        assert_eq!((viewport[0], viewport[2]), (0.0, 100.0));
        // ... and the content, stretched across the viewport, is 20 wider.
        let content = rect_in(&relaid, &fits, &[0, 0, 0]);
        assert_eq!((content[0], content[2]), (0.0, 100.0));
        // The same content in the capture's own height has to keep its bar (the capture is right as it is).
        assert!(!relayout(&[&fits], [2560.0, 1080.0]).hides(&fits.children[0].children[1]));
    }

    #[test]
    fn a_node_without_a_captured_rectangle_has_none_and_its_children_are_left_to_their_own() {
        let mut ghost = node("UI/ghost", [0.0, 0.0, 10.0, 10.0], ([0.0, 0.0], [0.0, 0.0]));
        ghost.native_rect = None;
        let root = canvas_root(vec![ghost]);
        let relaid = relayout(&[&root], [2560.0, 1080.0]);
        assert!(relaid.rect_of(&root.children[0]).is_none());
    }

    #[test]
    fn the_layout_components_are_read_from_the_capture() {
        let mut info = LayoutInfo::default();
        let value = |text: &str| serde_json::from_str::<Value>(text).expect("json");
        info.absorb(
            "UnityEngine.UI.HorizontalLayoutGroup",
            &value(r#"{"enabled":true,"padding":{"left":5,"right":6,"top":7,"bottom":8},"spacing":3,"child_alignment":"MiddleRight","child_control_width":true,"child_control_height":false,"child_force_expand_width":false,"child_force_expand_height":true,"reverse_arrangement":false}"#),
        );
        let group = info.group.expect("a group");
        assert_eq!((group.vertical, group.padding, group.spacing, group.align), (false, [5.0, 6.0, 7.0, 8.0], 3.0, [1.0, 0.5]));
        assert_eq!((group.control, group.expand), ([true, false], [false, true]));
        info.absorb("UnityEngine.UI.LayoutElement", &value(r#"{"enabled":true,"ignore_layout":true,"flexible_width":2,"flexible_height":-1}"#));
        assert_eq!(info.element.map(|element| (element.flexible, element.ignore)), Some(([2.0, -1.0], true)));
        info.absorb("UnityEngine.UI.ContentSizeFitter", &value(r#"{"enabled":true,"horizontal_fit":"Unconstrained","vertical_fit":"PreferredSize"}"#));
        assert_eq!(info.fit, [false, true]);
        info.absorb("UnityEngine.UI.AspectRatioFitter", &value(r#"{"enabled":true,"aspect_mode":"HeightControlsWidth","aspect_ratio":1.5}"#));
        assert_eq!(info.aspect, Some((Aspect::HeightControlsWidth, 1.5)));
        // A disabled component is not part of the layout.
        let mut off = LayoutInfo::default();
        off.absorb("UnityEngine.UI.ContentSizeFitter", &value(r#"{"enabled":false,"horizontal_fit":"PreferredSize","vertical_fit":"PreferredSize"}"#));
        assert_eq!(off.fit, [false, false]);
    }
}
