//! A solver for the layout of a prefab hierarchy, the way `UnityEngine.UI` does it: `HorizontalLayoutGroup` / `VerticalLayoutGroup`
//! (padding, spacing, child alignment, control / force-expand flags), `LayoutElement` (minimum, preferred and flexible sizes,
//! `ignoreLayout`), `ContentSizeFitter` and the preferred size of a `TextMeshProUGUI`.
//!
//! The data comes from `tools/unity/gen_layout_data.py` (`stats_layout_data.rs`). A panel whose captured geometry is one state of many
//! (the Stats panel with or without a parents line, the egg section, a dead bibite's buttons) is solved here for the state at hand; the
//! runtime UI then moves the captured rectangles by the difference between that state and the captured one.
//!
//! The algorithm is `LayoutGroup.CalculateLayoutInputHorizontal / Vertical` (bottom up: every node's minimum, preferred and flexible size
//! along an axis) and `SetLayoutHorizontal / Vertical` (top down: `HorizontalOrVerticalLayoutGroup.SetChildrenAlongAxis`), with
//! `LayoutUtility.GetLayoutProperty`'s rule for combining components (a `LayoutElement` value of 0 or more beats the group's or the text's;
//! the preferred size is never below the minimum). Not modelled: text wrapping (a text is one line), `Image` preferred sizes, child scale,
//! reversed arrangement.

use std::collections::HashMap;

/// `HorizontalLayoutGroup` / `VerticalLayoutGroup`.
#[derive(Clone, Copy, Debug)]
pub struct Group {
    /// `VerticalLayoutGroup`.
    pub vertical: bool,
    /// Padding left, right, top, bottom.
    pub pad: [f32; 4],
    /// Space between two children.
    pub spacing: f32,
    /// `TextAnchor` (0 upper left .. 8 lower right).
    pub align: i32,
    /// `childControlWidth`, `childControlHeight`.
    pub control: [bool; 2],
    /// `childForceExpandWidth`, `childForceExpandHeight`.
    pub expand: [bool; 2],
}

/// `LayoutElement` (-1 = unset).
#[derive(Clone, Copy, Debug)]
pub struct Elem {
    /// `ignoreLayout`.
    pub ignore: bool,
    /// Minimum width, height.
    pub min: [f32; 2],
    /// Preferred width, height.
    pub pref: [f32; 2],
    /// Flexible width, height.
    pub flex: [f32; 2],
}

/// `TextMeshProUGUI`.
#[derive(Clone, Copy, Debug)]
pub struct Text {
    /// The text of the prefab.
    pub text: &'static str,
    /// Font asset name.
    pub font: &'static str,
    /// Font size.
    pub size: f32,
    /// `FontStyles.Bold`.
    pub bold: bool,
    /// Horizontal alignment (1 left, 2 centre, 4 right ...). Part of the generated data; the solver only sizes.
    #[allow(dead_code)]
    pub halign: i32,
    /// Vertical alignment (256 top, 512 middle, 1024 bottom). Part of the generated data; the solver only sizes.
    #[allow(dead_code)]
    pub valign: i32,
}

/// One GameObject of the prefab.
#[derive(Clone, Copy, Debug)]
pub struct LNode {
    /// Name.
    pub name: &'static str,
    /// `activeSelf` as saved (part of the generated data; the states decide what is shown).
    #[allow(dead_code)]
    pub active: bool,
    /// Anchor min x, y, anchor max x, y, anchored position x, y, size delta x, y, pivot x, y (Unity's y is up).
    pub rect: [f32; 10],
    /// The layout group on it.
    pub group: Option<Group>,
    /// `ContentSizeFitter` horizontal and vertical fit (0 none, 1 minimum, 2 preferred).
    pub fit: [u8; 2],
    /// The `LayoutElement` on it.
    pub elem: Option<Elem>,
    /// The text on it.
    pub text: Option<Text>,
    /// The preferred size of the `Image` on it (the sprite's size, or a sliced sprite's border minimum).
    pub image: Option<[f32; 2]>,
    /// Children in sibling order.
    pub children: &'static [LNode],
}

/// Text sizes.
pub trait Measure {
    /// Advance of `text` set in `font` at `size`, in the bold style or not.
    fn text_width(&self, font: &str, text: &str, size: f32, bold: bool) -> f32;
    /// Height TMP gives a line of `font` at `size` (`ascentLine - descentLine`).
    fn line_height(&self, font: &str, size: f32) -> f32;
}

/// A rectangle `[x, y, w, h]`, top-left origin, y down.
pub type Rect = [f32; 4];

/// A `LayoutElement` the code sets at run time: for a node (its path and prefab data) the element to use in place of the prefab's.
pub type ElemOverride<'a> = dyn Fn(&str, &LNode) -> Option<Elem> + 'a;

/// What a solve is asked for.
pub struct Request<'a> {
    /// Whether a node is active (`path`: names from the root joined by `/`, the root's own name first).
    pub active: &'a dyn Fn(&str, &LNode) -> bool,
    /// The text a node shows, when not the prefab's.
    pub text: &'a dyn Fn(&str, &LNode) -> Option<String>,
    /// A `LayoutElement` the code sets at run time in place of the prefab's (`holder.preferredWidth = width`).
    pub elem: Option<&'a ElemOverride<'a>>,
}

/// The solved rectangles of the active nodes, by path, in the root's own coordinates (its top-left corner at the origin).
#[derive(Clone, Debug, Default)]
pub struct Solved {
    /// Rectangle of each active node.
    pub rects: HashMap<String, Rect>,
    /// The nodes a layout group placed (their rectangle follows the layout, not their anchors).
    pub placed: std::collections::HashSet<String>,
}

/// `Mathf.Clamp`: with `max` below `min` the result is `min` (Rust's `clamp` would panic).
fn unity_clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// The solver's working copy of one active node.
struct Item {
    node: &'static LNode,
    path: String,
    text: Option<String>,
    /// The node's `LayoutElement`: the request's, or the prefab's.
    elem: Option<Elem>,
    children: Vec<Item>,
    /// Final size and position relative to the parent's top-left corner.
    size: [f32; 2],
    pos: [f32; 2],
    /// Whether a layout group of the parent placed it (rather than its anchors).
    placed: bool,
}

/// `(minimum, preferred, flexible)` of an element along an axis.
type Sizes = (f32, f32, f32);

impl Item {
    fn build(node: &'static LNode, parent_path: &str, request: &Request<'_>) -> Option<Self> {
        let path = if parent_path.is_empty() {
            node.name.to_owned()
        } else {
            format!("{parent_path}/{}", node.name)
        };
        if !(request.active)(&path, node) {
            return None;
        }
        let text = (request.text)(&path, node);
        let elem = request
            .elem
            .and_then(|replace| replace(&path, node))
            .or(node.elem);
        let children = node
            .children
            .iter()
            .filter_map(|child| Self::build(child, &path, request))
            .collect();
        Some(Self {
            node,
            path,
            text,
            elem,
            children,
            size: [node.rect[6], node.rect[7]],
            pos: [0.0; 2],
            placed: false,
        })
    }

    /// The text this node shows.
    fn shown_text(&self) -> Option<&str> {
        self.text
            .as_deref()
            .or_else(|| self.node.text.as_ref().map(|text| text.text))
    }

    /// Children a group lays out (`LayoutGroup.rectChildren`: active, not ignoring the layout).
    fn laid_out(&self) -> impl Iterator<Item = &Item> {
        self.children
            .iter()
            .filter(|child| !child.elem.is_some_and(|elem| elem.ignore))
    }

    /// The group's totals along `axis` (`LayoutGroup.CalcAlongAxis`).
    fn group_totals(&self, group: &Group, axis: usize, measure: &dyn Measure) -> Sizes {
        let padding = if axis == 0 {
            group.pad[0] + group.pad[1]
        } else {
            group.pad[2] + group.pad[3]
        };
        let along_other_axis = group.vertical ^ (axis == 1);
        let (mut min, mut pref, mut flex) = (padding, padding, 0.0_f32);
        let mut count = 0;
        for child in self.laid_out() {
            let (c_min, c_pref, c_flex) = child.child_sizes(group, axis, measure);
            count += 1;
            if along_other_axis {
                min = min.max(c_min + padding);
                pref = pref.max(c_pref + padding);
                flex = flex.max(c_flex);
            } else {
                min += c_min + group.spacing;
                pref += c_pref + group.spacing;
                flex += c_flex;
            }
        }
        if !along_other_axis && count > 0 {
            min -= group.spacing;
            pref -= group.spacing;
        }
        (min, pref.max(min), flex)
    }

    /// `LayoutUtility.GetMinSize / GetPreferredSize / GetFlexibleSize` of this node as an element.
    fn element_sizes(&self, axis: usize, measure: &dyn Measure) -> Sizes {
        // Priority 0: the group's totals, the text's and the image's preferred sizes; where several set a value the largest wins
        // (`LayoutUtility.GetLayoutProperty`: equal priorities take the maximum).
        let (mut min, mut pref, mut flex): (Option<f32>, Option<f32>, Option<f32>) =
            (None, None, None);
        let take = |slot: &mut Option<f32>, value: f32| {
            *slot = Some(slot.map_or(value, |old| old.max(value)));
        };
        if let Some(group) = &self.node.group {
            let (a, b, c) = self.group_totals(group, axis, measure);
            take(&mut min, a);
            take(&mut pref, b);
            take(&mut flex, c);
        }
        if let (Some(text), Some(shown)) = (&self.node.text, self.shown_text()) {
            let size = if axis == 0 {
                measure.text_width(text.font, shown, text.size, text.bold)
            } else {
                measure.line_height(text.font, text.size)
            };
            take(&mut min, 0.0);
            take(&mut pref, size);
        }
        if let Some(image) = self.node.image {
            take(&mut min, 0.0);
            take(&mut pref, image[axis]);
        }
        // Priority 1: the `LayoutElement`'s values that are set.
        if let Some(elem) = &self.elem {
            if elem.min[axis] >= 0.0 {
                min = Some(elem.min[axis]);
            }
            if elem.pref[axis] >= 0.0 {
                pref = Some(elem.pref[axis]);
            }
            if elem.flex[axis] >= 0.0 {
                flex = Some(elem.flex[axis]);
            }
        }
        let min = min.unwrap_or(0.0);
        (min, pref.unwrap_or(0.0).max(min), flex.unwrap_or(0.0))
    }

    /// `HorizontalOrVerticalLayoutGroup.GetChildSizes`: what the parent's `group` gets from this child along `axis`.
    fn child_sizes(&self, group: &Group, axis: usize, measure: &dyn Measure) -> Sizes {
        if !group.control[axis] {
            let size = self.node.rect[6 + axis];
            return (size, size, 0.0);
        }
        let (min, pref, mut flex) = self.element_sizes(axis, measure);
        if group.expand[axis] {
            flex = flex.max(1.0);
        }
        (min, pref, flex)
    }

    /// `HorizontalOrVerticalLayoutGroup.SetChildrenAlongAxis` for the group on this node, whose size along `axis` is known.
    fn place_children(&mut self, group: &Group, axis: usize, measure: &dyn Measure) {
        let size = self.size[axis];
        let along_other_axis = group.vertical ^ (axis == 1);
        let alignment = if axis == 0 {
            (group.align % 3) as f32 * 0.5
        } else {
            (group.align / 3) as f32 * 0.5
        };
        let (pad_start, pad_end) = if axis == 0 {
            (group.pad[0], group.pad[1])
        } else {
            (group.pad[2], group.pad[3])
        };
        let padding = pad_start + pad_end;
        let totals = self.group_totals(group, axis, measure);
        let control = group.control[axis];
        // The children's sizes first (the immutable pass), then written back.
        let sizes: Vec<(usize, Sizes)> = self
            .children
            .iter()
            .enumerate()
            .filter(|(_, child)| !child.elem.is_some_and(|elem| elem.ignore))
            .map(|(index, child)| (index, child.child_sizes(group, axis, measure)))
            .collect();
        let start_offset = |required: f32| pad_start + (size - (required + padding)) * alignment;
        if along_other_axis {
            let inner = size - padding;
            for (index, (min, pref, flex)) in sizes {
                let required = unity_clamp(inner, min, if flex > 0.0 { size } else { pref });
                let child = &mut self.children[index];
                let own = child.node.rect[6 + axis];
                if control {
                    child.size[axis] = required;
                    child.pos[axis] = start_offset(required);
                } else {
                    child.pos[axis] = start_offset(required) + (required - own) * alignment;
                }
                child.placed = true;
            }
        } else {
            let (total_min, total_pref, total_flex) = totals;
            let mut pos = pad_start;
            let surplus = size - total_pref;
            let mut flex_multiplier = 0.0;
            if surplus > 0.0 {
                if total_flex == 0.0 {
                    pos = start_offset(total_pref - padding);
                } else if total_flex > 0.0 {
                    flex_multiplier = surplus / total_flex;
                }
            }
            let lerp = if (total_min - total_pref).abs() > f32::EPSILON {
                ((size - total_min) / (total_pref - total_min)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            for (index, (min, pref, flex)) in sizes {
                let child_size = min + (pref - min) * lerp + flex * flex_multiplier;
                let child = &mut self.children[index];
                if control {
                    child.size[axis] = child_size;
                    child.pos[axis] = pos;
                } else {
                    let own = child.node.rect[6 + axis];
                    child.pos[axis] = pos + (child_size - own) * alignment;
                }
                child.placed = true;
                pos += child_size + group.spacing;
            }
        }
    }

    /// Sets every descendant's size and position, parents first, one axis at a time.
    fn solve_axis(&mut self, axis: usize, measure: &dyn Measure) {
        if let Some(group) = self.node.group {
            self.place_children(&group, axis, measure);
        }
        // Children that no group placed keep their anchors: they follow this node's size.
        let parent_size = self.size;
        for child in &mut self.children {
            if !child.placed {
                child.anchor_axis(parent_size, axis);
            }
        }
        for child in &mut self.children {
            // A child with a `ContentSizeFitter` takes its own preferred size (after its children's inputs).
            child.fit_axis(axis, measure);
            child.solve_axis(axis, measure);
        }
    }

    /// `ContentSizeFitter`: the node's own size from its preferred / minimum size.
    fn fit_axis(&mut self, axis: usize, measure: &dyn Measure) {
        let (min, pref, _) = self.element_sizes(axis, measure);
        match self.node.fit[axis] {
            1 => self.size[axis] = min,
            2 => self.size[axis] = pref,
            _ => {}
        }
    }

    /// `RectTransform` math for a node nothing lays out: the size from the anchors and the position of its corner in the parent.
    fn anchor_axis(&mut self, parent_size: [f32; 2], axis: usize) {
        let rect = &self.node.rect;
        let (amin, amax, ap, sd, pivot) = if axis == 0 {
            (rect[0], rect[2], rect[4], rect[6], rect[8])
        } else {
            // y is up in Unity: measure from the bottom, convert to a top-left origin below.
            (rect[1], rect[3], rect[5], rect[7], rect[9])
        };
        let parent = parent_size[axis];
        let size = (amax - amin) * parent + sd;
        let reference = (amin + (amax - amin) * pivot) * parent;
        let low = reference + ap - pivot * size;
        self.size[axis] = size;
        self.pos[axis] = if axis == 0 {
            low
        } else {
            parent - (low + size)
        };
    }

    /// Absolute rectangles of this node and its descendants.
    fn collect(&self, origin: [f32; 2], solved: &mut Solved) {
        let at = [origin[0] + self.pos[0], origin[1] + self.pos[1]];
        solved.rects.insert(
            self.path.clone(),
            [at[0], at[1], self.size[0], self.size[1]],
        );
        if self.placed {
            solved.placed.insert(self.path.clone());
        }
        for child in &self.children {
            child.collect(at, solved);
        }
    }
}

/// Solves the layout of `root` for `request`: the root keeps its authored anchored position (it is the panel itself), its size comes
/// from its `ContentSizeFitter` or its size delta.
#[must_use]
pub fn solve(root: &'static LNode, request: &Request<'_>, measure: &dyn Measure) -> Solved {
    let Some(mut item) = Item::build(root, "", request) else {
        return Solved::default();
    };
    for axis in 0..2 {
        item.fit_axis(axis, measure);
        item.solve_axis(axis, measure);
    }
    let mut solved = Solved::default();
    // The root's own corner is the origin of its coordinates.
    item.pos = [0.0, 0.0];
    item.collect([0.0, 0.0], &mut solved);
    solved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is `size / 2` wide, a line is `size` high.
    struct Fixed;

    impl Measure for Fixed {
        fn text_width(&self, _font: &str, text: &str, size: f32, _bold: bool) -> f32 {
            text.chars().count() as f32 * size / 2.0
        }

        fn line_height(&self, _font: &str, size: f32) -> f32 {
            size
        }
    }

    const fn leaf(name: &'static str, elem: Option<Elem>, text: Option<Text>) -> LNode {
        LNode {
            name,
            active: true,
            rect: [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 10.0, 10.0, 0.5, 0.5],
            group: None,
            fit: [0, 0],
            elem,
            text,
            image: None,
            children: &[],
        }
    }

    const fn group(
        name: &'static str,
        group: Group,
        fit: [u8; 2],
        children: &'static [LNode],
    ) -> LNode {
        LNode {
            name,
            active: true,
            rect: [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 100.0, 100.0, 0.0, 1.0],
            group: Some(group),
            fit,
            elem: None,
            text: None,
            image: None,
            children,
        }
    }

    const NONE: Elem = Elem {
        ignore: false,
        min: [-1.0; 2],
        pref: [-1.0; 2],
        flex: [-1.0; 2],
    };
    const fn elem(min: [f32; 2], pref: [f32; 2], flex: [f32; 2]) -> Elem {
        Elem {
            ignore: false,
            min,
            pref,
            flex,
        }
    }
    const HGROUP: Group = Group {
        vertical: false,
        pad: [0.0; 4],
        spacing: 5.0,
        align: 3,
        control: [true, true],
        expand: [false, false],
    };

    fn all_active(_: &str, _: &LNode) -> bool {
        true
    }

    fn no_text(_: &str, _: &LNode) -> Option<String> {
        None
    }

    fn solve_with(root: &'static LNode) -> Solved {
        solve(
            root,
            &Request {
                active: &all_active,
                text: &no_text,
                elem: None,
            },
            &Fixed,
        )
    }

    const TITLE: Text = Text {
        text: "Title",
        font: "f",
        size: 20.0,
        bold: false,
        halign: 1,
        valign: 256,
    };

    #[test]
    fn a_horizontal_group_hands_the_surplus_to_its_flexible_child() {
        // Title (flexible, text 5 x 10 = 50 wide), a 60-wide button and a 40-wide one: 50 + 60 + 40 + 2 x 5 spacing = 160, in 300.
        static ROOT: LNode = group(
            "row",
            Group { ..HGROUP },
            [0, 0],
            &[
                leaf(
                    "title",
                    Some(elem([-1.0; 2], [-1.0; 2], [1.0, -1.0])),
                    Some(TITLE),
                ),
                leaf("a", Some(elem([60.0, -1.0], [-1.0; 2], [-1.0; 2])), None),
                leaf("b", Some(elem([40.0, -1.0], [-1.0; 2], [-1.0; 2])), None),
            ],
        );
        let mut wide = ROOT;
        wide.rect[6] = 300.0;
        wide.rect[7] = 30.0;
        let wide: &'static LNode = Box::leak(Box::new(wide));
        let solved = solve_with(wide);
        let title = solved.rects["row/title"];
        let a = solved.rects["row/a"];
        let b = solved.rects["row/b"];
        // 300 - 160 = 140 surplus to the title: 50 + 140 = 190.
        assert!((title[2] - 190.0).abs() < 1e-3, "{title:?}");
        assert!(
            (a[0] - 195.0).abs() < 1e-3 && (a[2] - 60.0).abs() < 1e-3,
            "{a:?}"
        );
        assert!(
            (b[0] - 260.0).abs() < 1e-3 && (b[2] - 40.0).abs() < 1e-3,
            "{b:?}"
        );
        // Without a fit the row keeps its size; the children are stretched to its height (controlled, no expand: preferred 0 -> the
        // text's line height for the title, the minimum 0 for the others).
        assert_eq!(solved.rects["row"][3], 30.0);
        assert!((title[3] - 20.0).abs() < 1e-3);
    }

    #[test]
    fn a_content_size_fitter_takes_the_preferred_size_of_the_group() {
        static ROOT: LNode = group(
            "panel",
            Group {
                vertical: true,
                pad: [10.0, 10.0, 5.0, 15.0],
                spacing: 4.0,
                align: 0,
                control: [true, true],
                expand: [false, false],
            },
            [2, 2],
            &[
                leaf("one", Some(elem([-1.0; 2], [80.0, 20.0], [-1.0; 2])), None),
                leaf("two", Some(elem([-1.0; 2], [120.0, 30.0], [-1.0; 2])), None),
            ],
        );
        let solved = solve_with(&ROOT);
        // Width: 10 + 120 + 10; height: 5 + 20 + 4 + 30 + 15.
        assert_eq!(solved.rects["panel"], [0.0, 0.0, 140.0, 74.0]);
        // A child as wide as it prefers (no flexible size), left aligned inside the padding.
        assert_eq!(solved.rects["panel/one"], [10.0, 5.0, 80.0, 20.0]);
        assert_eq!(solved.rects["panel/two"], [10.0, 29.0, 120.0, 30.0]);
    }

    #[test]
    fn a_vertical_group_that_expands_stretches_its_children() {
        static ROOT: LNode = group(
            "col",
            Group {
                vertical: true,
                pad: [0.0; 4],
                spacing: 0.0,
                align: 0,
                control: [true, true],
                expand: [true, false],
            },
            [0, 2],
            &[
                leaf("a", Some(elem([-1.0; 2], [10.0, 10.0], [-1.0; 2])), None),
                leaf("b", Some(elem([-1.0; 2], [50.0, 10.0], [-1.0; 2])), None),
            ],
        );
        let mut root = ROOT;
        root.rect[6] = 200.0;
        let root: &'static LNode = Box::leak(Box::new(root));
        let solved = solve_with(root);
        // Force-expand gives every child a flexible width: they take the whole 200.
        assert_eq!(solved.rects["col/a"][2], 200.0);
        assert_eq!(solved.rects["col/b"][2], 200.0);
        assert_eq!(solved.rects["col"][3], 20.0);
    }

    #[test]
    fn inactive_nodes_take_no_space_and_ignored_ones_none_either() {
        static ROOT: LNode = group(
            "row",
            HGROUP,
            [2, 2],
            &[
                leaf("a", Some(elem([30.0, 10.0], [-1.0; 2], [-1.0; 2])), None),
                leaf(
                    "hidden",
                    Some(elem([100.0, 10.0], [-1.0; 2], [-1.0; 2])),
                    None,
                ),
                leaf(
                    "loose",
                    Some(Elem {
                        ignore: true,
                        ..NONE
                    }),
                    None,
                ),
                leaf("b", Some(elem([30.0, 10.0], [-1.0; 2], [-1.0; 2])), None),
            ],
        );
        let solved = solve(
            &ROOT,
            &Request {
                active: &|path: &str, _: &LNode| path != "row/hidden",
                text: &no_text,
                elem: None,
            },
            &Fixed,
        );
        assert!(!solved.rects.contains_key("row/hidden"));
        // 30 + 5 + 30.
        assert_eq!(solved.rects["row"][2], 65.0);
        assert_eq!(solved.rects["row/b"][0], 35.0);
    }

    #[test]
    fn a_text_is_as_wide_as_its_text_and_a_layout_element_overrides_it() {
        static ROOT: LNode = group(
            "row",
            HGROUP,
            [2, 2],
            &[
                leaf("plain", None, Some(TITLE)),
                leaf(
                    "fixed",
                    Some(elem([-1.0; 2], [200.0, -1.0], [-1.0; 2])),
                    Some(TITLE),
                ),
            ],
        );
        let solved = solve_with(&ROOT);
        // "Title" is 5 characters of 10: 50; the fixed one 200; the row 50 + 5 + 200 and 20 high.
        assert_eq!(solved.rects["row/plain"][2], 50.0);
        assert_eq!(solved.rects["row/fixed"][2], 200.0);
        assert_eq!(solved.rects["row"][2], 255.0);
        assert_eq!(solved.rects["row"][3], 20.0);
    }

    #[test]
    fn the_text_of_a_request_replaces_the_prefabs() {
        static ROOT: LNode = group("row", HGROUP, [2, 2], &[leaf("t", None, Some(TITLE))]);
        let solved = solve(
            &ROOT,
            &Request {
                active: &all_active,
                text: &|path: &str, _: &LNode| {
                    (path == "row/t").then(|| "A much longer title".to_owned())
                },
                elem: None,
            },
            &Fixed,
        );
        assert_eq!(solved.rects["row/t"][2], 190.0);
    }

    #[test]
    fn children_without_a_group_follow_their_anchors() {
        // A stretched child (anchors 0..1, no size delta) fills its parent; a centred one sits in the middle.
        static CHILDREN: [LNode; 2] = [
            LNode {
                name: "fill",
                active: true,
                rect: [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5],
                group: None,
                fit: [0, 0],
                elem: None,
                text: None,
                image: None,
                children: &[],
            },
            LNode {
                name: "mid",
                active: true,
                rect: [0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 20.0, 10.0, 0.5, 0.5],
                group: None,
                fit: [0, 0],
                elem: None,
                text: None,
                image: None,
                children: &[],
            },
        ];
        static ROOT: LNode = LNode {
            name: "box",
            active: true,
            rect: [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 100.0, 40.0, 0.0, 1.0],
            group: None,
            fit: [0, 0],
            elem: None,
            text: None,
            image: None,
            children: &CHILDREN,
        };
        let solved = solve_with(&ROOT);
        assert_eq!(solved.rects["box/fill"], [0.0, 0.0, 100.0, 40.0]);
        assert_eq!(solved.rects["box/mid"], [40.0, 15.0, 20.0, 10.0]);
    }
}
