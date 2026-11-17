//! The lineage tree's layout: a port of the game's `LineageWindow` (the time axis), `LineageTreePanel.BuildTree` /
//! `PlaceTree`, `LineageSpeciesNode` and `SpeciesColumn` (`Here/UIScripts.UIReferences.LineagePanel/`).
//!
//! Heights are graph units measured up from the bottom of the graph: "now" is 0 and a species' bar spans from
//! the height of its newest present point (`totalIndexOfDisappearance`, -1 while alive) to the height of its oldest
//! (`totalIndexOfApparition`). The time axis is not linear: a point of the log's coarser scales is drawn smaller
//! (`HeightOfPoint`). Columns are the tree's horizontal slots: the heaviest child continues its parent's column,
//! lighter ones go to either side, and short leaves are packed into a neighbour's free column
//! (`SpeciesColumn.CheckToInsertNode`). Everything is computed in graph units; zoom, pan and the scroll bars
//! belong to the view (`lineage_view.rs`).

use std::sync::OnceLock;

use bibites_core::datalog::LogLikeFormat;
use bibites_core::ids::SpeciesId;
use bibites_core::species_log::SPECIES_FORMATS;

/// `LineageTreePanel.defaultStep`: the height of the first point.
pub const DEFAULT_STEP: f32 = 75.0;
/// `LineageTreePanel.sidePadding`.
pub const SIDE_PADDING: f32 = 150.0;
/// `paddingBetweenColumns`.
const PADDING_BETWEEN_COLUMNS: f32 = 20.0;
/// `paddingBetweenTemplateLineages`.
const PADDING_BETWEEN_TEMPLATE_LINEAGES: f32 = 100.0;
/// `defaultColumnSize`.
const DEFAULT_COLUMN_SIZE: f32 = 120.0;
/// `LineageSpeciesNode.MinWidth`.
pub const MIN_WIDTH: f32 = 35.0;
/// The preview box of a node (`SpeciesPreview.sizeDelta`).
pub const PREVIEW_SIZE: [f32; 2] = [180.0, 160.0];
/// `LineageSpeciesNode.MinPreviewScale`.
const MIN_PREVIEW_SCALE: f32 = 0.5;
/// The most the tree can be zoomed in (`SetMinMaxZoom(.., 1.5)`).
pub const MAX_ZOOM: f32 = 1.5;
/// The graph is this much taller than its oldest point (`HeightOfPoint(maxN) + 120`).
const TOP_PADDING: f32 = 120.0;

/// `LineageWindow`'s height of every point of the log: `linesProgress` and `linesSteps` of
/// `DataLogger.SerialSpeciesConfig`.
#[derive(Clone, Debug)]
pub struct TimeAxis {
    progress: Vec<f32>,
    /// `linesSteps`.
    #[allow(dead_code)]
    steps: Vec<f32>,
    /// `nonInfiniteMaxN`: the points of the finite scales.
    pub non_infinite_max_n: usize,
}

impl TimeAxis {
    /// `LineageWindow.Initialize`.
    #[must_use]
    pub fn new() -> Self {
        let formats: &[LogLikeFormat] = &SPECIES_FORMATS;
        let total: usize = formats.iter().map(|format| format.size).sum();
        let mut progress = vec![0.0_f32; total];
        let mut steps = vec![0.0_f32; total + 1];
        let mut at = 0;
        for (index, format) in formats.iter().enumerate() {
            let size = format.size;
            let feed = format.feed_ratio as f32;
            let base = (1.0 + index as f32 / 3.0) / feed.min(3.0);
            for k in 0..size {
                let value = (size - 1 - k) as f32 / (size - 1) as f32 * (1.0 - base) + base;
                progress[at] = value;
                if at > 0 {
                    steps[at - 1] = value;
                }
                if at > 0 {
                    progress[at] += progress[at - 1];
                }
                at += 1;
            }
        }
        steps[at - 1] = 1.0;
        Self {
            progress,
            steps,
            non_infinite_max_n: total,
        }
    }

    /// `HeightOfPoint(i)` for a step of `default_step`.
    #[must_use]
    pub fn height_of_point(&self, i: i32, default_step: f32) -> f32 {
        let units = if i >= 0 {
            let i = i as usize;
            if i >= self.non_infinite_max_n {
                self.progress[self.progress.len() - 1] + (i + 1 - self.non_infinite_max_n) as f32
            } else {
                self.progress[i]
            }
        } else {
            0.0
        };
        default_step * units
    }

    /// `StepOfPoint(i)`: the gap between point `i` and the one before it (the game only hands it to the axis
    /// lines as their largest label size, which nothing reads).
    #[must_use]
    #[allow(dead_code)]
    pub fn step_of_point(&self, i: i32, default_step: f32) -> f32 {
        default_step
            * if i < 0 {
                1.0
            } else if (i as usize) < self.non_infinite_max_n {
                self.steps[i as usize]
            } else {
                (i as usize + 1 - self.non_infinite_max_n) as f32
            }
    }
}

impl Default for TimeAxis {
    fn default() -> Self {
        Self::new()
    }
}

/// The shared time axis.
#[must_use]
pub fn time_axis() -> &'static TimeAxis {
    static AXIS: OnceLock<TimeAxis> = OnceLock::new();
    AXIS.get_or_init(TimeAxis::new)
}

/// What the tree needs to know of one recorded species.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesInfo {
    /// The species.
    pub id: SpeciesId,
    /// Its parent species.
    pub parent: Option<SpeciesId>,
    /// The root of its lineage (`Species.rootSpecies`).
    pub root: SpeciesId,
    /// `genericName + " " + specificName`.
    pub name: String,
    /// The favourite flag.
    pub favorite: bool,
    /// `speciesData.totalIndexOfApparition`.
    pub apparition: i32,
    /// `speciesData.totalIndexOfDisappearance` (`-1` while alive).
    pub disappearance: i32,
    /// `speciesData.nPointsPresent`.
    pub points_present: u32,
}

/// The three tree options of the Settings panel (`LineagePanelSettingsManager`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeSettings {
    /// "Hide Extinct Lineages".
    pub hide_extinct: bool,
    /// "Show Favorited Lineages".
    pub show_favorites: bool,
    /// "Scale Important Species".
    pub scale_species: bool,
}

impl Default for TreeSettings {
    /// The settings' default values.
    fn default() -> Self {
        Self {
            hide_extinct: true,
            show_favorites: false,
            scale_species: true,
        }
    }
}

/// One species of the tree (`LineageSpeciesNode`).
#[derive(Clone, Debug)]
pub struct Node {
    /// The species it draws.
    pub species: SpeciesId,
    /// The root of the species' lineage (`Species.rootSpecies`).
    pub root: SpeciesId,
    /// `speciesData.nPointsPresent`.
    pub points_present: u32,
    /// Its name.
    pub name: String,
    /// Favourite flag.
    pub favorite: bool,
    /// The parent node (the nearest drawn ancestor once nodes were left out).
    pub parent: Option<usize>,
    /// The child nodes.
    pub children: Vec<usize>,
    /// `hasParent`.
    pub has_parent: bool,
    /// Alive: present in the newest point.
    pub alive: bool,
    /// `totalIndexOfApparition`.
    pub apparition: i32,
    /// `totalIndexOfDisappearance`.
    pub disappearance: i32,
    /// The node is drawn.
    pub will_render: bool,
    /// The top of the node in graph units (`topHeight`; above the species' first point for a child, which has
    /// its arrow head there).
    pub top_height: f32,
    /// The bottom (`bottomHeight`).
    pub bottom_height: f32,
    height_span: f32,
    available_height: f32,
    /// `previewScale` (of the 180 x 160 preview box).
    pub preview_scale: f32,
    /// The preview box fits (`canFitPreview`), else a small point stands for the species.
    pub can_fit_preview: bool,
    /// `preferredWidth`.
    pub preferred_width: f32,
    /// The width of its column (`assignedWidth`).
    pub assigned_width: f32,
    /// `branchWeight`.
    pub branch_weight: f32,
    /// The x of the column centre in the tree's holder (`topAnchor.x`).
    pub x: f32,
    /// `parentSpeciesNode.topAnchor.x - topAnchor.x` (`LineageLinkToParentNode.SetDist`).
    pub link_dist: f32,
    /// Left out of the tree (a pooled node).
    pub retired: bool,
}

impl Node {
    /// `LineageSpeciesNode.weight`.
    fn weight(&self) -> f32 {
        if !self.will_render {
            return 0.0;
        }
        ((self.children.len() + 1) as f32).sqrt()
            * self.preferred_width
            * if self.alive { 3.0 } else { 1.0 }
    }
}

/// `SpeciesColumn`: the nodes that share a horizontal slot.
#[derive(Clone, Debug)]
struct Column {
    id: usize,
    nodes: Vec<usize>,
    weight: f32,
    is_parent: bool,
    is_template: bool,
    root: SpeciesId,
}

/// A laid out tree.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// Every node that was made; the ones with `retired` set were left out.
    pub nodes: Vec<Node>,
    columns: Vec<Column>,
    templates: Vec<usize>,
    settings: TreeSettings,
    total_weight: f32,
    /// The tree has something to draw (`treeBuilt`).
    pub built: bool,
    /// `maxN`: the oldest point any species has.
    pub max_n: i32,
    /// The size of the graph in graph units (`SetDimension`: the tree's width plus the side paddings, the height of
    /// the oldest point plus 120).
    pub graph_size: [f32; 2],
    /// The least the view can be zoomed out (`SetMinMaxZoom`).
    pub min_zoom: f32,
    /// The zoom `BuildTree` ends with (the tree fits the viewport's width, and the tallest living node its height).
    pub fit_zoom: f32,
}

impl Layout {
    /// `LineageTreePanel.BuildTree`: makes the nodes of the species, hides the ones the settings say, assigns
    /// the columns and places everything for `viewport` (the viewport's size in graph units at zoom 1).
    #[must_use]
    pub fn build(infos: &[SpeciesInfo], settings: TreeSettings, viewport: [f32; 2]) -> Self {
        let axis = time_axis();
        let mut layout = Self {
            settings,
            ..Self::default()
        };
        if infos.is_empty() {
            return layout;
        }
        layout.max_n = infos.iter().map(|info| info.apparition).max().unwrap_or(0);
        // `orderby totalIndexOfApparition descending, speciesID`.
        let mut order: Vec<usize> = (0..infos.len()).collect();
        order.sort_by(|&a, &b| {
            infos[b]
                .apparition
                .cmp(&infos[a].apparition)
                .then(infos[a].id.cmp(&infos[b].id))
        });
        let mut node_of: std::collections::HashMap<SpeciesId, usize> =
            std::collections::HashMap::new();
        for &index in &order {
            let info = &infos[index];
            if info.apparition < 2 {
                break;
            }
            let node_index = layout.nodes.len();
            layout.nodes.push(Node {
                species: info.id,
                root: info.root,
                points_present: info.points_present,
                name: info.name.clone(),
                favorite: info.favorite,
                parent: None,
                children: Vec::new(),
                has_parent: false,
                alive: info.disappearance < 0,
                apparition: info.apparition,
                disappearance: info.disappearance,
                will_render: false,
                top_height: 0.0,
                bottom_height: 0.0,
                height_span: 0.0,
                available_height: 0.0,
                preview_scale: 0.0,
                can_fit_preview: false,
                preferred_width: 0.0,
                assigned_width: 0.0,
                branch_weight: 0.0,
                x: 0.0,
                link_dist: 0.0,
                retired: false,
            });
            node_of.insert(info.id, node_index);
            // The parent was made already (it is older); a parent the tree does not have makes a root.
            let parent_node = info.parent.and_then(|parent| node_of.get(&parent).copied());
            if let Some(parent_node) = parent_node {
                layout.set_parent(node_index, Some(parent_node));
            } else {
                layout.set_parent(node_index, None);
                layout.templates.push(node_index);
            }
            let bottom = axis.height_of_point(info.disappearance, DEFAULT_STEP);
            let top = axis.height_of_point(info.apparition, DEFAULT_STEP);
            layout.set_anchors(node_index, top, bottom);
        }
        // Nodes that cannot be drawn (too short) or have no point and no child are left out; their children go to
        // their parent.
        let doomed: Vec<usize> = (0..layout.nodes.len())
            .filter(|&n| {
                let node = &layout.nodes[n];
                !node.will_render || (node.points_present < 1 && node.children.is_empty())
            })
            .collect();
        for n in doomed {
            layout.not_rendered_and_retire(n);
        }
        layout
            .templates
            .retain(|&n| !layout.nodes[n].retired && layout.nodes[n].will_render);
        let templates = layout.templates.clone();
        for template in templates {
            layout.check_descendants(template, settings.hide_extinct, settings.show_favorites);
        }
        layout
            .templates
            .retain(|&n| !layout.nodes[n].retired && layout.nodes[n].will_render);
        layout.columns.clear();
        let mut next_column_id = 0;
        for template in layout.templates.clone() {
            let mut columns = vec![layout.new_column(template, &mut next_column_id)];
            layout.build_columns(template, &mut columns, None, 0, &mut next_column_id);
            layout.columns.extend(columns);
        }
        layout.total_weight = layout
            .columns
            .iter()
            .map(|column| {
                if settings.scale_species {
                    column.weight
                } else {
                    DEFAULT_COLUMN_SIZE
                }
            })
            .sum();
        if layout.columns.is_empty() {
            layout.built = false;
            return layout;
        }
        layout.built = true;
        layout.place(viewport);
        // The tree opens fitting the viewport's width, and the tallest node that reaches "now" its height.
        let mut zoom = viewport[0] / layout.graph_size[0];
        let alive_tops: Vec<f32> = layout
            .nodes
            .iter()
            .filter(|node| !node.retired && node.bottom_height < 1.0)
            .map(|node| node.top_height)
            .collect();
        if let Some(tallest) = alive_tops.into_iter().reduce(f32::max) {
            zoom = zoom.min(viewport[1] / (50.0 + tallest));
        }
        layout.fit_zoom = zoom;
        layout
    }

    /// `LineageSpeciesNode.SetParentNode`.
    fn set_parent(&mut self, node: usize, new_parent: Option<usize>) {
        self.nodes[node].has_parent = new_parent.is_some();
        if self.nodes[node].parent == new_parent {
            return;
        }
        if new_parent.is_none()
            && let Some(old) = self.nodes[node].parent
        {
            self.nodes[old].children.retain(|&child| child != node);
        }
        self.nodes[node].parent = new_parent;
        if let Some(parent) = new_parent {
            self.nodes[parent].children.push(node);
        }
    }

    /// `LineageSpeciesNode.SetAnchors`.
    fn set_anchors(&mut self, node: usize, top: f32, bottom: f32) {
        let n = &mut self.nodes[node];
        n.top_height = if n.has_parent { top + 28.5 } else { top };
        n.bottom_height = bottom;
        n.height_span = top - bottom;
        let needed = (if n.alive { 20.0 } else { 17.5 }) + if n.has_parent { 50.5 } else { 17.5 };
        n.will_render = !n.has_parent || n.height_span >= needed;
        if n.will_render {
            n.available_height = n.height_span - 25.0 - if n.has_parent { 27.5 } else { 0.0 };
            n.preview_scale = (n.available_height / PREVIEW_SIZE[1]
                * if n.has_parent { 1.0 } else { 2.0 })
            .min(1.0);
            n.can_fit_preview = n.preview_scale > MIN_PREVIEW_SCALE;
            n.preferred_width = if n.can_fit_preview {
                PREVIEW_SIZE[0] * n.preview_scale
            } else {
                MIN_WIDTH
            };
        }
    }

    /// `LineageSpeciesNode.NotRenderedAndRetire`.
    fn not_rendered_and_retire(&mut self, node: usize) {
        self.nodes[node].will_render = false;
        let parent = self.nodes[node].parent;
        for child in self.nodes[node].children.clone() {
            self.set_parent(child, parent);
        }
        if let Some(parent) = parent {
            self.nodes[parent].children.retain(|&child| child != node);
            let bottom = self.nodes[node].bottom_height;
            self.nodes[parent].bottom_height = self.nodes[parent].bottom_height.min(bottom);
        }
        self.retire(node);
    }

    /// `ReturnToPool` / `Retire`.
    fn retire(&mut self, node: usize) {
        self.nodes[node].retired = true;
        self.set_parent(node, None);
        self.nodes[node].children.clear();
    }

    /// `LineageSpeciesNode.CheckDescendants`: hides the branches without a living species (and, unless
    /// the favourites are shown, without a favourite) and sums the branch weights.
    fn check_descendants(&mut self, node: usize, hide_extinct: bool, show_favorites: bool) -> bool {
        self.nodes[node].branch_weight = self.nodes[node].weight();
        let mut any = false;
        for child in self.nodes[node].children.clone() {
            any |= self.check_descendants(child, hide_extinct, show_favorites);
        }
        let rendered: f32 = self.nodes[node]
            .children
            .iter()
            .filter(|&&child| self.nodes[child].will_render)
            .map(|&child| self.nodes[child].branch_weight)
            .sum();
        self.nodes[node].branch_weight += rendered;
        let n = &self.nodes[node];
        if !hide_extinct || n.alive || any || (show_favorites && n.favorite) {
            return true;
        }
        self.nodes[node].will_render = false;
        self.retire(node);
        false
    }

    fn new_column(&self, node: usize, next_id: &mut usize) -> Column {
        let n = &self.nodes[node];
        let id = *next_id;
        *next_id += 1;
        Column {
            id,
            nodes: vec![node],
            weight: n.weight(),
            is_parent: n
                .children
                .iter()
                .filter(|&&c| self.nodes[c].will_render)
                .count()
                > 0,
            is_template: !n.has_parent,
            root: n.root,
        }
    }

    /// `SpeciesColumn.CheckToInsertNode`. The game's columns are structs in a list, so the call works on a copy:
    /// the node joins the column, but its weight and extent stay what they were.
    fn check_to_insert(&self, column: &Column, node: usize) -> bool {
        if column.is_parent || column.is_template || self.nodes[node].root != column.root {
            return false;
        }
        let n = &self.nodes[node];
        column.nodes.iter().all(|&other| {
            let o = &self.nodes[other];
            n.bottom_height - 25.0 > o.top_height || n.top_height < o.bottom_height - 25.0
        })
    }

    /// `LineageSpeciesNode.BuildColumns`: returns the positions the columns of this branch took.
    fn build_columns(
        &self,
        node: usize,
        columns: &mut Vec<Column>,
        to_avoid: Option<&mut Vec<usize>>,
        mut index: usize,
        next_id: &mut usize,
    ) -> Vec<usize> {
        let mut owned_avoid: Vec<usize>;
        let to_avoid = match to_avoid {
            Some(list) => list,
            None => {
                owned_avoid = columns.iter().map(|column| column.id).collect::<Vec<_>>();
                &mut owned_avoid
            }
        };
        let mut left = 0.0_f32;
        let mut right = 0.0_f32;
        let mut taken: Vec<usize> = Vec::new();
        let children = &self.nodes[node].children;
        // `children.OrderByDescending(branchWeight).FirstOrDefault()`: the first of the heaviest.
        let heaviest =
            children
                .iter()
                .copied()
                .fold(None, |best: Option<usize>, child| match best {
                    Some(b) if self.nodes[child].branch_weight <= self.nodes[b].branch_weight => {
                        Some(b)
                    }
                    _ => Some(child),
                });
        let mut by_height = children.clone();
        by_height.sort_by(|&a, &b| {
            self.nodes[b]
                .top_height
                .total_cmp(&self.nodes[a].top_height)
        });
        for child in by_height {
            let has_children = !self.nodes[child].children.is_empty();
            if !has_children {
                let mut inserted = false;
                if index > 0 && !to_avoid.contains(&columns[index - 1].id) {
                    if self.check_to_insert(&columns[index - 1], child) {
                        columns[index - 1].nodes.push(child);
                        inserted = true;
                    }
                } else if index + 1 < columns.len()
                    && !to_avoid.contains(&columns[index + 1].id)
                    && self.check_to_insert(&columns[index + 1], child)
                {
                    columns[index + 1].nodes.push(child);
                    inserted = true;
                }
                if inserted {
                    continue;
                }
            }
            let position;
            if Some(child) == heaviest || left <= right {
                position = index;
                index += 1;
                left += self.nodes[child].weight();
            } else {
                position = index + 1;
                right += self.nodes[child].weight();
            }
            let column = self.new_column(child, next_id);
            let column_id = column.id;
            columns.insert(position.min(columns.len()), column);
            taken.push(position);
            if has_children {
                to_avoid.push(column_id);
                let below =
                    self.build_columns(child, columns, Some(&mut *to_avoid), position, next_id);
                if position < index {
                    index += below.len();
                }
                taken.extend(below);
                to_avoid.retain(|&id| id != column_id);
            }
        }
        taken
    }

    /// `LineageTreePanel.PlaceTree`: widths and positions for a viewport of this size (graph units at zoom 1).
    pub fn place(&mut self, viewport: [f32; 2]) {
        if !self.built {
            return;
        }
        let axis = time_axis();
        let n_columns = self.columns.len() as f32;
        let gaps = PADDING_BETWEEN_COLUMNS * (n_columns - 1.0)
            + PADDING_BETWEEN_TEMPLATE_LINEAGES * (self.templates.len() as f32 - 1.0);
        let min_total = MIN_WIDTH * n_columns;
        let scale = self.settings.scale_species;
        let mut width = (if scale {
            (self.total_weight / min_total).powf(0.66) * min_total
        } else {
            self.total_weight
        }) + gaps;
        width = width.max(viewport[0] - 2.0 * SIDE_PADDING);
        let mut weight_left = self.total_weight;
        let mut width_left = width - gaps;
        // `treeColumns.OrderBy(weight)`: the lightest first, stable.
        let mut by_weight: Vec<usize> = (0..self.columns.len()).collect();
        by_weight.sort_by(|&a, &b| self.columns[a].weight.total_cmp(&self.columns[b].weight));
        for column in by_weight {
            let share = if scale {
                self.columns[column].weight
            } else {
                DEFAULT_COLUMN_SIZE
            };
            let assigned = (width_left * share / weight_left).max(MIN_WIDTH);
            weight_left -= share;
            width_left -= assigned;
            for &node in &self.columns[column].nodes.clone() {
                self.nodes[node].assigned_width = assigned;
            }
        }
        let mut root = self.columns[0].root;
        let mut cursor = 0.0_f32;
        for column in 0..self.columns.len() {
            if root != self.columns[column].root {
                cursor += PADDING_BETWEEN_TEMPLATE_LINEAGES;
                root = self.columns[column].root;
            }
            let column_width = self.columns[column]
                .nodes
                .iter()
                .map(|&node| self.nodes[node].assigned_width)
                .fold(f32::MIN, f32::max);
            let x = cursor + column_width / 2.0;
            for node in self.columns[column].nodes.clone() {
                self.update_placement(node, x, self.nodes[node].assigned_width);
            }
            cursor += column_width + PADDING_BETWEEN_COLUMNS;
        }
        for column in 0..self.columns.len() {
            for node in self.columns[column].nodes.clone() {
                self.update_link_to_parent(node);
            }
        }
        self.graph_size = [
            width + 2.0 * SIDE_PADDING,
            axis.height_of_point(self.max_n, DEFAULT_STEP) + TOP_PADDING,
        ];
        let fit = [
            1.0,
            viewport[1] / self.graph_size[1],
            viewport[0] / self.graph_size[0],
        ]
        .into_iter()
        .fold(f32::MAX, f32::min);
        self.min_zoom = fit
            .max(viewport[1] / axis.height_of_point(axis.non_infinite_max_n as i32, DEFAULT_STEP));
    }

    /// `LineageSpeciesNode.UpdatePlacement`.
    fn update_placement(&mut self, node: usize, x: f32, available_width: f32) {
        self.nodes[node].assigned_width = available_width;
        let child_top = self.nodes[node]
            .children
            .iter()
            .map(|&child| self.nodes[child].top_height)
            .reduce(f32::max);
        if let Some(top) = child_top {
            let n = &mut self.nodes[node];
            n.top_height = n.top_height.max(top + 27.5);
        }
        let n = &mut self.nodes[node];
        n.x = x;
        n.available_height = n.top_height
            - child_top.map_or(n.bottom_height, |top| top + 10.0)
            - if n.has_parent { 27.5 } else { 0.0 };
        n.preview_scale = (n.available_height / PREVIEW_SIZE[1]
            * if n.has_parent { 1.0 } else { 2.0 })
        .min(n.preview_scale);
        if available_width < n.preferred_width {
            n.preview_scale = n.preview_scale.min(available_width / PREVIEW_SIZE[0]);
        }
        n.can_fit_preview = n.preview_scale > MIN_PREVIEW_SCALE;
    }

    /// `LineageSpeciesNode.UpdateLinkToParent`.
    fn update_link_to_parent(&mut self, node: usize) {
        if self.nodes[node].has_parent
            && let Some(parent) = self.nodes[node].parent
        {
            self.nodes[node].link_dist = self.nodes[parent].x - self.nodes[node].x;
        }
    }

    /// The node of a species, when it is drawn.
    #[must_use]
    pub fn node_of(&self, species: SpeciesId) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| !node.retired && node.species == species)
    }

    /// The nodes that are drawn, in the order they were made.
    pub fn drawn(&self) -> impl Iterator<Item = (usize, &Node)> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| !node.retired && node.will_render)
    }

    /// `SpeciesPanel.UpdateDisplayedSpecies`: the species the graph shows, the oldest first.
    #[must_use]
    pub fn displayed(&self) -> Vec<SpeciesId> {
        let mut nodes: Vec<&Node> = self.nodes.iter().filter(|node| !node.retired).collect();
        nodes.sort_by_key(|node| std::cmp::Reverse(node.apparition));
        nodes.iter().map(|node| node.species).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> SpeciesId {
        SpeciesId::new(n)
    }

    fn info(n: u32, parent: Option<u32>, apparition: i32, disappearance: i32) -> SpeciesInfo {
        SpeciesInfo {
            id: id(n),
            parent: parent.map(id),
            root: id(1),
            name: format!("Genus species{n}"),
            favorite: false,
            apparition,
            disappearance,
            points_present: (apparition - disappearance).max(0) as u32,
        }
    }

    const VIEWPORT: [f32; 2] = [1865.0, 990.0];

    #[test]
    fn the_time_axis_grows_with_the_points_and_the_older_scales_take_more_height() {
        let axis = time_axis();
        assert_eq!(axis.non_infinite_max_n, 260);
        assert_eq!(axis.height_of_point(-1, 75.0), 0.0);
        // The first point is a full step, the second a little less.
        assert!((axis.height_of_point(0, 75.0) - 75.0).abs() < 1e-3);
        let second = axis.height_of_point(1, 75.0) - axis.height_of_point(0, 75.0);
        assert!(second < 75.0 && second > 60.0, "{second}");
        // Heights only grow.
        let mut last = -1.0;
        for i in -1..300 {
            let h = axis.height_of_point(i, 75.0);
            assert!(h > last || i == -1, "point {i}");
            last = h;
        }
        // Within the first scale the steps shrink; the older scales (more time per point) take more height per point.
        assert!(axis.step_of_point(10, 75.0) < axis.step_of_point(0, 75.0));
        assert!(axis.step_of_point(200, 75.0) > axis.step_of_point(0, 75.0));
        // Past the finite scales every point is one more step, the step growing by one (the game's quirk).
        let beyond = axis.height_of_point(300, 75.0) - axis.height_of_point(299, 75.0);
        assert!((beyond - 75.0).abs() < 1e-3);
    }

    #[test]
    fn a_lineage_gets_columns_widths_and_links_to_its_parent() {
        let infos = [
            info(1, None, 100, -1),
            info(2, Some(1), 60, -1),
            info(3, Some(1), 40, -1),
            info(4, Some(2), 20, -1),
        ];
        let layout = Layout::build(&infos, TreeSettings::default(), VIEWPORT);
        assert!(layout.built);
        let root = layout.node_of(id(1)).unwrap();
        assert!(!layout.nodes[root].has_parent);
        for n in [2, 3, 4] {
            let node = &layout.nodes[layout.node_of(id(n)).unwrap()];
            assert!(node.has_parent && node.will_render);
            assert!(node.link_dist != 0.0 || node.x == layout.nodes[node.parent.unwrap()].x);
        }
        // Every drawn species has a column x inside the tree, columns do not overlap, and the graph is at
        // least as wide as the viewport.
        let mut xs: Vec<f32> = layout.drawn().map(|(_, node)| node.x).collect();
        xs.sort_by(f32::total_cmp);
        xs.dedup();
        assert!(xs.len() >= 3, "{xs:?}");
        for pair in xs.windows(2) {
            assert!(pair[1] - pair[0] >= MIN_WIDTH, "{pair:?}");
        }
        assert!(layout.graph_size[0] >= VIEWPORT[0]);
        // A child's top is 28.5 above its first point, a root's is its first point.
        let axis = time_axis();
        let child = &layout.nodes[layout.node_of(id(3)).unwrap()];
        assert!(child.top_height >= axis.height_of_point(40, DEFAULT_STEP) + 28.5 - 1e-3);
        assert_eq!(
            layout.nodes[root].top_height.round() as i32,
            layout.nodes[root]
                .top_height
                .max(axis.height_of_point(100, DEFAULT_STEP))
                .round() as i32
        );
        // The graph reaches the oldest point plus the padding.
        assert!(
            (layout.graph_size[1] - (axis.height_of_point(100, DEFAULT_STEP) + 120.0)).abs() < 1e-2
        );
        // The tree opens fitting its width, and no more than the viewport's height.
        assert!(
            layout.fit_zoom > 0.0 && layout.fit_zoom <= VIEWPORT[0] / layout.graph_size[0] + 1e-6
        );
        assert!(layout.min_zoom <= layout.fit_zoom + 1e-6 || layout.fit_zoom < layout.min_zoom);
    }

    #[test]
    fn a_new_species_is_left_out_until_it_has_two_points_and_a_short_child_is_too_short_to_draw() {
        // Apparition 1 is too new (`totalIndexOfApparition < 2` ends the list); a child that has been around for
        // less than the height of its arrow head and label is not drawn, and its own child goes to the grandparent.
        let infos = [
            info(1, None, 100, -1),
            info(2, Some(1), 3, 2),
            info(3, Some(1), 1, -1),
        ];
        let layout = Layout::build(&infos, TreeSettings::default(), VIEWPORT);
        assert!(layout.node_of(id(3)).is_none(), "apparition 1 has no node");
        let short = layout.nodes.iter().find(|n| n.species == id(2)).unwrap();
        assert!(
            !short.will_render,
            "{} high is too short",
            short.height_span
        );
        assert!(layout.node_of(id(2)).is_none());
        assert!(layout.node_of(id(1)).is_some());
        assert_eq!(layout.displayed(), vec![id(1)]);
    }

    #[test]
    fn extinct_branches_are_hidden_unless_the_setting_or_a_favourite_keeps_them() {
        let infos = [
            info(1, None, 120, -1),
            info(2, Some(1), 80, 30),
            info(3, Some(2), 60, 40),
            info(4, Some(1), 70, -1),
        ];
        let hidden = Layout::build(&infos, TreeSettings::default(), VIEWPORT);
        assert!(hidden.node_of(id(2)).is_none() && hidden.node_of(id(3)).is_none());
        assert!(hidden.node_of(id(4)).is_some());
        let shown = Layout::build(
            &infos,
            TreeSettings {
                hide_extinct: false,
                ..TreeSettings::default()
            },
            VIEWPORT,
        );
        assert!(shown.node_of(id(2)).is_some() && shown.node_of(id(3)).is_some());
        // A favourite keeps its lineage when the setting is on.
        let mut favourite = infos.clone();
        favourite[2].favorite = true;
        let kept = Layout::build(
            &favourite,
            TreeSettings {
                show_favorites: true,
                ..TreeSettings::default()
            },
            VIEWPORT,
        );
        assert!(kept.node_of(id(3)).is_some() && kept.node_of(id(2)).is_some());
        let not_kept = Layout::build(&favourite, TreeSettings::default(), VIEWPORT);
        assert!(not_kept.node_of(id(3)).is_none());
    }

    #[test]
    fn every_species_gone_means_nothing_to_draw() {
        let infos = [info(1, None, 100, 50)];
        let layout = Layout::build(&infos, TreeSettings::default(), VIEWPORT);
        assert!(
            !layout.built,
            "a lineage with no living species is hidden whole"
        );
        assert!(
            Layout::build(&[], TreeSettings::default(), VIEWPORT)
                .node_of(id(1))
                .is_none()
        );
    }

    #[test]
    fn species_of_different_templates_are_set_apart_and_leaves_share_a_free_column() {
        let mut infos = [
            info(1, None, 100, -1),
            info(2, None, 90, -1),
            // Two leaves of the first template that do not overlap in time can share a column.
            info(3, Some(1), 90, 60),
            info(4, Some(1), 50, -1),
        ];
        infos[1].root = id(2);
        let settings = TreeSettings {
            hide_extinct: false,
            ..TreeSettings::default()
        };
        let layout = Layout::build(&infos, settings, VIEWPORT);
        let x = |n: u32| layout.nodes[layout.node_of(id(n)).unwrap()].x;
        // The second template starts 100 after the first lineage's last column.
        assert!(x(2) - x(4).max(x(3)).max(x(1)) >= PADDING_BETWEEN_TEMPLATE_LINEAGES);
        assert!(
            (x(3) - x(4)).abs() < 1e-3,
            "the two leaves do not overlap in time, so they share a column"
        );
        let widths: Vec<f32> = layout.drawn().map(|(_, n)| n.assigned_width).collect();
        assert!(widths.iter().all(|w| *w >= MIN_WIDTH));
    }

    #[test]
    fn without_scaling_every_column_is_as_wide() {
        let infos = [
            info(1, None, 100, -1),
            info(2, Some(1), 80, -1),
            info(3, Some(1), 50, -1),
        ];
        let layout = Layout::build(
            &infos,
            TreeSettings {
                scale_species: false,
                ..TreeSettings::default()
            },
            VIEWPORT,
        );
        let widths: Vec<f32> = layout.drawn().map(|(_, n)| n.assigned_width).collect();
        let first = widths[0];
        assert!(
            widths.iter().all(|w| (w - first).abs() < 1e-2),
            "{widths:?}"
        );
    }
}
