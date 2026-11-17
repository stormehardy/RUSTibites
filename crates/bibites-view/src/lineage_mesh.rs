//! The lineage mesh view's layout: a port of `LineageMeshPanel.GenerateMesh`, `LineageMesh.AddRung` / `AddTopCap` /
//! `AddBottomCap` / `AddMaxFadeEnd` / `CalculateDisplayBibite` / `OnMeshHover` and `UIMeshEventHandler`'s bounds check, and
//! `SpeciesRungData` (`Here/UIScripts/LineageMeshPanel.cs`, `Here/UIScripts.UIReferences.LineagePanel/`).
//!
//! The mesh view is a stream graph: every species is one shape, as wide at each point of the history as its share of the
//! energy all the species had then, the shapes side by side (in the order the species were recorded) in a strip the width of
//! which follows how much energy there was compared with the most there ever was (`(viewport - 2 side) 3^log10(energy / max)`).
//! The time axis is the tree's with its default step 50 instead of 75. A shape starts (at its top) at the point its species
//! first showed up; it ends at "now", or, when the species went extinct, in a point at the first log without it.
//!
//! Coordinates are graph units: `x` from the panel's left edge (the panel is inset by the side padding of 150 in the graph's
//! holder, as the tree's is), `y` up from the bottom (the height of the point, "now" is 0).

use std::sync::atomic::{AtomicU64, Ordering};

use bibites_core::ids::SpeciesId;
use bibites_core::species::{SpeciesRecord, SpeciesRegistry};
use bibites_core::species_log::SpeciesDataPoint;

use crate::lineage_tree::{SIDE_PADDING, time_axis};
use crate::time_format::{TimeFormat, Timescale};

/// `LineageMeshPanel.defaultStep`: the height of the first point of the mesh view's time axis.
pub const DEFAULT_STEP: f32 = 50.0;
/// The smallest square the species' picture is shown in (`minPreviewSize`).
const MIN_PREVIEW: f32 = 50.0;
/// The biggest (`maxPreviewSize`): the previewer is a 400 wide disc scaled to what fits.
const MAX_PREVIEW: f32 = 400.0;
/// The previewer's own size (`BodyPreview`).
pub const PREVIEWER: f32 = 400.0;

/// `SpeciesRungData`: what a shape has at one point of the history, which its tooltip tells.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rung {
    /// Bibites and eggs of the species.
    pub count: u32,
    /// Its share of all the species' count then.
    pub count_share: f32,
    /// Its energy.
    pub energy: f32,
    /// Its share of all the species' energy then.
    pub energy_share: f32,
    /// The point of the log (`-1` is now).
    pub index: i32,
}

impl Rung {
    /// `new SpeciesRungData(info, rungTotalEnergy, rungTotalCount, rungIndex)`.
    #[must_use]
    pub fn new(point: &SpeciesDataPoint, total_energy: f32, total_count: i32, index: i32) -> Self {
        let count = u32::from(point.count);
        Self {
            count,
            count_share: count as f32 / (total_count as f32).max(1.0),
            energy: point.energy,
            energy_share: point.energy / total_energy.max(1.0),
            index,
        }
    }

    /// `SpeciesRungData.EmptyRung`.
    #[must_use]
    pub fn empty(index: i32) -> Self {
        Self {
            index,
            ..Self::default()
        }
    }

    /// `SpeciesRungData.Text`: when, how many and how much energy, with the shares. The game's format string has `\n\r`
    /// between the last two lines; the carriage return draws nothing.
    #[must_use]
    pub fn text(&self) -> String {
        let when = if self.index < 0 {
            "now".to_owned()
        } else {
            format!(
                "{} ago",
                TimeFormat::of_species_point(self.index).formatted(
                    1,
                    " ",
                    true,
                    false,
                    Timescale::Minutes
                )
            )
        };
        format!(
            "{when}\nCount: {}  ({}%)\n\rEnergy: {} E  ({}%)",
            self.count,
            net_fixed(self.count_share * 100.0, 1),
            net_fixed(self.energy, 0),
            net_fixed(self.energy_share * 100.0, 1),
        )
    }
}

/// .NET's `F<decimals>`: the value rounded with halves away from zero.
fn net_fixed(value: f32, decimals: usize) -> String {
    let factor = 10_f64.powi(decimals as i32);
    let rounded = (f64::from(value) * factor).round() / factor;
    format!("{rounded:.decimals$}")
}

/// Where a shape shows its species' picture: the middle of the square and its side, in the shape's own space (the
/// previewer is scaled to `size / 400`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preview {
    /// The square's middle, from the shape's origin.
    pub centre: [f32; 2],
    /// The square's side.
    pub size: f32,
}

/// The outline of a shape at one height: where its two edges are and how they lean.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    /// The left edge (panel x).
    pub left: f32,
    /// The right edge.
    pub right: f32,
    /// How far the left edge moves sideways per unit of height.
    pub left_slope: f32,
    /// The same for the right edge.
    pub right_slope: f32,
}

/// One species' shape (`LineageMesh`).
#[derive(Clone, Debug)]
pub struct Mesh {
    /// The species.
    pub species: SpeciesId,
    /// Its name.
    pub name: String,
    /// The left and the right end of every rung, from the newest (the bottom) to the oldest, relative to `origin`.
    pub corners: Vec<[f32; 2]>,
    /// The shape's bottom left corner (`localPosition`).
    pub origin: [f32; 2],
    /// The shape's size (`rectSize`).
    pub size: [f32; 2],
    /// What the species had at each rung.
    pub rungs: Vec<Rung>,
    /// The species is alive: the shape is open at the bottom, at "now".
    pub alive: bool,
    /// The top cap was added: the shape is complete.
    pub capped: bool,
    /// The species' colour (`GenesToColor` of its colour genes).
    pub colour: [f32; 3],
    /// Its parent's colour (its own for a species without one): the shape fades into it towards its top.
    pub parent_colour: [f32; 3],
    /// The species has a parent (`species.parentSpecies != null`: its shape fades into the parent's colour whether or not the
    /// parent has a shape in the view).
    pub has_parent: bool,
    /// Where the fade into the parent's colour ends (`fadeEndHeight`), a height.
    pub fade_end: f32,
    /// How high it can end at most (`absoluteMaxFadeEndHeight`).
    pub absolute_max_fade_end: f32,
    /// Where the species' picture goes, when a square of 50 fits.
    pub preview: Option<Preview>,
    /// The bounds of the rungs added so far, before the shape is moved to its origin: `min x, min y, max x, max y`.
    bounds: [f32; 4],
}

impl Mesh {
    fn new(record: &SpeciesRecord, parent: Option<&SpeciesRecord>) -> Self {
        let colour_of =
            |record: &SpeciesRecord| crate::assets::body_color(record.template.genome.values());
        let colour = colour_of(record);
        Self {
            species: record.id,
            name: record.name(),
            corners: Vec::new(),
            origin: [0.0, 0.0],
            size: [0.0, 0.0],
            rungs: Vec::new(),
            alive: true,
            capped: false,
            colour,
            parent_colour: parent.map_or(colour, colour_of),
            has_parent: record.parent.is_some(),
            fade_end: 0.0,
            absolute_max_fade_end: 0.0,
            preview: None,
            bounds: [0.0; 4],
        }
    }

    /// `topHeight`: the height of the shape's top.
    #[must_use]
    pub fn top_height(&self) -> f32 {
        self.origin[1] + self.size[1]
    }

    /// `bottomHeight`.
    #[must_use]
    pub fn bottom_height(&self) -> f32 {
        self.origin[1]
    }

    /// `_BlendHeight`: how much of the shape's height, from the top, fades into the parent's colour.
    #[must_use]
    pub fn blend(&self) -> f32 {
        if self.has_parent && self.size[1] > 0.0 {
            ((self.top_height() - self.fade_end) / self.size[1]).max(0.0)
        } else {
            0.0
        }
    }

    /// `AddRung`.
    fn add_rung(&mut self, y: f32, x1: f32, x2: f32, rung: Rung) {
        if self.corners.is_empty() {
            self.bounds = [x1, y, x2, y];
        } else {
            self.bounds[3] = y;
            self.bounds[0] = self.bounds[0].min(x1);
            self.bounds[2] = self.bounds[2].max(x2);
        }
        self.corners.push([x1, y]);
        self.corners.push([x2, y]);
        self.rungs.push(rung);
    }

    /// `AddTopCap`: the shape's oldest end, a point; the shape is moved to its own origin and the species'
    /// picture is placed.
    fn add_top_cap(&mut self, x: f32, y: f32, index: i32) {
        self.add_rung(y, x, x, Rung::empty(index));
        let count = self.corners.len();
        let fade = (count / 5).clamp(1, 5);
        // `corners[Count - 3]` and `corners[Count - (2 * fade + 1)]` (a shape cannot start with its cap in a game save).
        let from_end = |back: usize| {
            count
                .checked_sub(back)
                .and_then(|at| self.corners.get(at))
                .map_or(y, |corner| corner[1])
        };
        self.absolute_max_fade_end = from_end(3);
        self.fade_end = self.absolute_max_fade_end.min(from_end(2 * fade + 1));
        let origin = [self.bounds[0], self.bounds[1]];
        for corner in &mut self.corners {
            corner[0] -= origin[0];
            corner[1] -= origin[1];
        }
        self.size = [self.bounds[2] - origin[0], self.bounds[3] - origin[1]];
        self.origin = origin;
        self.capped = true;
        self.calculate_display_bibite();
    }

    /// `AddBottomCap`: the shape of an extinct species ends in a point at the bottom.
    fn add_bottom_cap(&mut self, x: f32, y: f32, index: i32) {
        self.alive = false;
        self.add_rung(y, x, x, Rung::empty(index));
    }

    /// `AddMaxFadeEnd` (`Mathf.Clamp(fadeEndHeight, lastHeight, absoluteMaxFadeEndHeight)`).
    fn add_max_fade_end(&mut self, last_height: f32) {
        self.fade_end = if self.fade_end < last_height {
            last_height
        } else if self.fade_end > self.absolute_max_fade_end {
            self.absolute_max_fade_end
        } else {
            self.fade_end
        };
    }

    /// `CalculateDisplayBibite`: the biggest square (at least 50 across) the shape holds, found by growing a rectangle
    /// from every wide enough rung up the shape while it stays at least as high as it is wide.
    fn calculate_display_bibite(&mut self) {
        self.preview = None;
        if self.size[0] < MIN_PREVIEW || self.size[1] < MIN_PREVIEW {
            return;
        }
        // `Rect`s as `x, y, width, height`.
        let mut best = [0.0_f32; 4];
        let rungs = self.corners.len() / 2;
        for i in 0..rungs {
            let width = self.corners[2 * i + 1][0] - self.corners[2 * i][0];
            if width < MIN_PREVIEW {
                continue;
            }
            let mut rect = [self.corners[2 * i][0], self.corners[2 * i][1], width, 0.0];
            for j in (i + 1)..rungs {
                if rect[3] >= rect[2] || rect[2] <= MIN_PREVIEW {
                    break;
                }
                let left = rect[0].max(self.corners[2 * j][0]);
                let right = (rect[0] + rect[2]).min(self.corners[2 * j + 1][0]);
                let narrowed = rect[2].min(right - left);
                let height = self.corners[2 * j][1] - rect[1];
                if rect[3] > MIN_PREVIEW && rect[2].min(MAX_PREVIEW) * rect[3] > narrowed * height {
                    break;
                }
                rect[0] = left;
                rect[2] = narrowed;
                rect[3] = height;
            }
            let centre = [rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0];
            rect[3] = rect[3].min(rect[2]);
            rect[2] = rect[3].min(rect[2]);
            rect[0] = centre[0] - rect[2] / 2.0;
            rect[1] = centre[1] - rect[3] / 2.0;
            if rect[2] > MIN_PREVIEW && rect[2] > best[2] {
                best = rect;
            }
        }
        if best[2] > MIN_PREVIEW {
            self.preview = Some(Preview {
                centre: [best[0] + best[2] / 2.0, best[1] + best[3] / 2.0],
                size: best[2].min(MAX_PREVIEW),
            });
        }
    }

    /// The index of the first rung above local height `y` (the bounds check's and the tooltip's loop: it stops at the first
    /// rung whose height is more than `y`).
    fn first_rung_above(&self, y: f32) -> usize {
        let rungs = self.corners.len() / 2;
        let (mut low, mut high) = (0, rungs);
        while low < high {
            let middle = (low + high) / 2;
            if y < self.corners[2 * middle][1] {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        low
    }

    /// The outline at `height` (graph units), interpolated between the two rungs around it; `None` out of the shape's
    /// height (`UIMeshEventHandler.CheckPointIsInsideBounds`' first half).
    #[must_use]
    pub fn profile_at(&self, height: f32) -> Option<Profile> {
        if !self.capped {
            return None;
        }
        let y = height - self.origin[1];
        let above = self.first_rung_above(y);
        // Below the first rung, or at / past the last one: outside.
        if above < 1 || 2 * above >= self.corners.len() {
            return None;
        }
        let (l0, l1) = (self.corners[2 * above - 2], self.corners[2 * above]);
        let (r0, r1) = (self.corners[2 * above - 1], self.corners[2 * above + 1]);
        let run = l1[1] - l0[1];
        if run <= 0.0 {
            return None;
        }
        let t = (y - l0[1]) / run;
        Some(Profile {
            left: self.origin[0] + l0[0] + t * (l1[0] - l0[0]),
            right: self.origin[0] + r0[0] + t * (r1[0] - r0[0]),
            left_slope: (l1[0] - l0[0]) / run,
            right_slope: (r1[0] - r0[0]) / run,
        })
    }

    /// `UIMeshEventHandler.CheckPointIsInsideBounds` for a point of the graph (`x` from the panel's left edge).
    #[must_use]
    pub fn contains(&self, x: f32, height: f32) -> bool {
        self.profile_at(height)
            .is_some_and(|profile| x >= profile.left && x <= profile.right)
    }

    /// `OnMeshHover`: the rung nearest to `height` (a height in graph units).
    #[must_use]
    pub fn nearest_rung(&self, height: f32) -> Option<&Rung> {
        let y = height - self.origin[1];
        if y < 0.0 || y > self.size[1] || self.rungs.is_empty() {
            return None;
        }
        let rungs = self.corners.len() / 2;
        let mut i = 0;
        while i < self.rungs.len() && y >= self.corners[2 * i][1] {
            i += 1;
        }
        let below = i.saturating_sub(1);
        let above = i.min(rungs - 1);
        if self.corners[2 * above][1] - y < (y - self.corners[2 * below][1]).abs() {
            self.rungs.get(above)
        } else {
            self.rungs.get(below)
        }
    }
}

/// Makes the layouts tell each other apart (the drawn picture is kept for as long as the layout is the same one).
static SERIAL: AtomicU64 = AtomicU64::new(1);

/// The lineage mesh view (`LineageMeshPanel`): the shapes and the size of the graph they live in.
#[derive(Clone, Debug, Default)]
pub struct MeshLayout {
    /// The shapes, in the order their species were first needed.
    pub meshes: Vec<Mesh>,
    /// The species of the view, the ones that showed up first first (`SpeciesPanel.displayedSpecies`: by descending index
    /// of apparition, and the index counts back from now).
    pub displayed: Vec<SpeciesId>,
    /// The graph's size: the viewport's width and the height of the oldest point (at least the viewport's).
    pub size: [f32; 2],
    /// `SetMinMaxZoom`'s minimum: the viewport's height over the graph's.
    pub min_zoom: f32,
    /// `maxN`: the points the time axis shows.
    pub max_n: i32,
    /// There is something to show (`!empty`).
    pub built: bool,
    /// Which layout this is.
    pub serial: u64,
}

/// What the species have right now (`new SpeciesDataPoint(species)`): the members' count and energy. A species that is not
/// in the list has no members.
pub type Now<'a> = &'a [(SpeciesId, SpeciesDataPoint)];

/// The point of a species as it is now.
fn live(now: Now, species: SpeciesId) -> SpeciesDataPoint {
    now.iter()
        .find(|(id, _)| *id == species)
        .map(|(_, point)| *point)
        .unwrap_or_default()
}

/// The species of the mesh view: the recorded ones that have a history, and with "Hide Extinct Lineages" only the ones that
/// are around (or favourite, with "Show Favorited Lineages") and their ancestors.
fn listed<'a>(
    registry: &'a SpeciesRegistry,
    now: Now,
    hide_extinct: bool,
    show_favorites: bool,
) -> Vec<&'a SpeciesRecord> {
    let recorded: Vec<&SpeciesRecord> = registry
        .records()
        .iter()
        .filter(|record| {
            record.stats.log.total_index_of_apparition()
                != record.stats.log.total_index_of_disappearance()
        })
        .collect();
    if !hide_extinct {
        return recorded;
    }
    let mut list: Vec<&SpeciesRecord> = Vec::new();
    let mut queue: Vec<&SpeciesRecord> = recorded
        .iter()
        .copied()
        .filter(|record| live(now, record.id).count > 0 || (show_favorites && record.favorite))
        .collect();
    queue.reverse();
    while let Some(record) = queue.pop() {
        let mut at = Some(record);
        while let Some(species) = at {
            if list.iter().any(|listed| listed.id == species.id) {
                break;
            }
            list.push(species);
            at = species.parent.and_then(|parent| registry.get(parent));
        }
    }
    // `OrderBy(recordedSpecies.IndexOf)`: the recorded order (what is not recorded comes first, as `IndexOf` says -1).
    list.sort_by_key(|record| {
        recorded
            .iter()
            .position(|other| other.id == record.id)
            .map_or(-1, |at| at as i64)
    });
    list
}

/// `Mathf.Pow(3, Mathf.Log10(energy / maxEnergy))`: how much of the strip the energy fills. The most energy there ever was
/// fills all of it, so a quotient past one (a save without its maximum) or without a number does not make it wider.
fn width_factor(energy: f32, max_energy: f32) -> f32 {
    let factor = 3.0_f32.powf((energy / max_energy).log10());
    if factor.is_nan() {
        0.0
    } else {
        factor.min(1.0)
    }
}

impl MeshLayout {
    /// `GenerateMesh` for a viewport `viewport` (`width, height` in graph units at zoom 1); `now` is what the species have
    /// at this moment.
    #[must_use]
    pub fn build(
        registry: &SpeciesRegistry,
        now: Now,
        hide_extinct: bool,
        show_favorites: bool,
        viewport: [f32; 2],
    ) -> Self {
        let mut list = listed(registry, now, hide_extinct, show_favorites);
        if list.is_empty() {
            return Self::default();
        }
        let axis = time_axis();
        let height_of = |i: i32| axis.height_of_point(i, DEFAULT_STEP);
        let max_n = list
            .iter()
            .map(|record| record.stats.log.total_index_of_apparition())
            .max()
            .unwrap_or(-1)
            + 1;
        let max_energy = registry.max_energy();
        let room = (viewport[0] - 2.0 * SIDE_PADDING).max(0.0);
        let mut meshes: Vec<Mesh> = Vec::new();
        let mesh_of = |meshes: &mut Vec<Mesh>, record: &SpeciesRecord| -> usize {
            if let Some(at) = meshes.iter().position(|mesh| mesh.species == record.id) {
                return at;
            }
            meshes.push(Mesh::new(
                record,
                record.parent.and_then(|parent| registry.get(parent)),
            ));
            meshes.len() - 1
        };
        // The species that are alive start from "now".
        let alive: Vec<&SpeciesRecord> = list
            .iter()
            .copied()
            .filter(|record| record.stats.log.total_index_of_disappearance() < 0)
            .collect();
        let mut energy: f32 = alive.iter().map(|record| live(now, record.id).energy).sum();
        let mut count: i32 = alive
            .iter()
            .map(|record| i32::from(live(now, record.id).count))
            .sum();
        let mut width = room * width_factor(energy, max_energy);
        let mut x = (room - width) / 2.0;
        for record in &alive {
            let at = mesh_of(&mut meshes, record);
            let point = live(now, record.id);
            if point.count < 1 {
                meshes[at].add_bottom_cap(x, 0.0, 0);
                continue;
            }
            let share = width * point.energy / energy;
            let rung = Rung::new(&point, energy, count, -1);
            meshes[at].add_rung(0.0, x, x + share, rung);
            x += share;
        }
        for i in 0..=max_n {
            let y = height_of(i);
            let present: Vec<&SpeciesRecord> = list
                .iter()
                .copied()
                .filter(|record| {
                    let log = &record.stats.log;
                    log.total_index_of_disappearance() <= i
                        && log.total_index_of_apparition() + 1 >= i
                })
                .collect();
            let point_of = |record: &SpeciesRecord| record.stats.log.at(i as usize);
            energy = present.iter().map(|record| point_of(record).energy).sum();
            count = present
                .iter()
                .map(|record| i32::from(point_of(record).count))
                .sum();
            width = room * width_factor(energy, max_energy);
            x = (room - width) / 2.0;
            let mut capped: Vec<SpeciesId> = Vec::new();
            count = count.max(1);
            energy = energy.max(1.0);
            for record in &present {
                let at = mesh_of(&mut meshes, record);
                let log = &record.stats.log;
                if log.total_index_of_disappearance() == i {
                    meshes[at].add_bottom_cap(x, y, i);
                    continue;
                }
                if i == log.total_index_of_apparition() + 1 {
                    meshes[at].add_top_cap(x, y, i);
                    capped.push(record.id);
                    continue;
                }
                let point = point_of(record);
                let share = width * (point.energy / energy);
                let rung = Rung::new(&point, energy, count, i);
                meshes[at].add_rung(y, x, x + share, rung);
                x += share;
            }
            list.retain(|record| !capped.contains(&record.id));
        }
        // A shape fades into its parent's colour up to where its parent's shape ends, and its children's start.
        type Ends = (Option<SpeciesId>, Vec<SpeciesId>, f32, f32);
        let ends: Vec<Ends> = meshes
            .iter()
            .map(|mesh| {
                let record = registry.get(mesh.species);
                (
                    record.and_then(|record| record.parent),
                    record
                        .map(|record| record.children.clone())
                        .unwrap_or_default(),
                    mesh.top_height(),
                    mesh.bottom_height(),
                )
            })
            .collect();
        for (parent, children, top, bottom) in &ends {
            if let Some(parent) = parent
                && let Some(at) = meshes.iter().position(|mesh| mesh.species == *parent)
            {
                meshes[at].add_max_fade_end(*top);
            }
            for child in children {
                if let Some(at) = meshes.iter().position(|mesh| mesh.species == *child) {
                    meshes[at].add_max_fade_end(*bottom);
                }
            }
        }
        let mut displayed: Vec<SpeciesId> = meshes.iter().map(|mesh| mesh.species).collect();
        // `OrderByDescending(totalIndexOfApparition)` is stable.
        displayed.sort_by_key(|id| {
            std::cmp::Reverse(
                registry
                    .get(*id)
                    .map_or(-1, |record| record.stats.log.total_index_of_apparition()),
            )
        });
        let height = height_of(max_n).max(viewport[1]);
        Self {
            meshes,
            displayed,
            size: [viewport[0], height],
            min_zoom: viewport[1] / height,
            max_n,
            built: true,
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// The shape at a point of the graph (`x` from the panel's left edge, `y` up from the bottom): the topmost one.
    #[must_use]
    pub fn mesh_at(&self, x: f32, height: f32) -> Option<&Mesh> {
        self.meshes
            .iter()
            .rev()
            .find(|mesh| mesh.contains(x, height))
    }

    /// The shape of a species.
    #[must_use]
    pub fn mesh_of(&self, species: SpeciesId) -> Option<&Mesh> {
        self.meshes.iter().find(|mesh| mesh.species == species)
    }
}

/// Registries with a history, for the tests of this view and of its drawing.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use bibites_core::brain::Brain;
    use bibites_core::genome::Genome;

    /// One species of a test history: its parent (an index into the list), the log it was found at and what it had at each
    /// log from then on, `(count, energy)`; it is extinct after those.
    pub(crate) struct Life {
        pub(crate) parent: Option<usize>,
        pub(crate) born: usize,
        pub(crate) logs: &'static [(u32, f32)],
    }

    /// A registry that lived through the lives, one log at a time (every species is logged, so the ones before their
    /// birth have no points and the ones after their end have points without members).
    pub(crate) fn registry(lives: &[Life]) -> (SpeciesRegistry, Vec<SpeciesId>) {
        let mut registry = SpeciesRegistry::new();
        registry.set_prune_point_limit(1);
        let (genome, brain) = (Genome::starter(), Brain::starter());
        let steps = lives
            .iter()
            .map(|life| life.born + life.logs.len())
            .max()
            .unwrap_or(0);
        let mut ids: Vec<Option<SpeciesId>> = vec![None; lives.len()];
        for step in 0..steps {
            for (index, life) in lives.iter().enumerate() {
                if life.born != step {
                    continue;
                }
                let id = registry.add_template_species(
                    &format!("Zzspecies{index} zzname"),
                    "",
                    &genome,
                    &brain,
                );
                if let Some(parent) = life.parent.and_then(|parent| ids[parent]) {
                    let root = registry.get(parent).expect("parent").root;
                    let record = registry.get_mut(id).expect("species");
                    record.parent = Some(parent);
                    record.root = root;
                    registry.get_mut(parent).expect("parent").children.push(id);
                }
                ids[index] = Some(id);
            }
            for (index, life) in lives.iter().enumerate() {
                let Some(id) = ids[index] else { continue };
                let (count, energy) = step
                    .checked_sub(life.born)
                    .and_then(|k| life.logs.get(k))
                    .copied()
                    .unwrap_or((0, 0.0));
                // A species the registry pruned is gone (the tests keep their logs short enough for that not to happen).
                let Some(record) = registry.get_mut(id) else {
                    continue;
                };
                record.stats.count = count;
                record.stats.energy = energy;
            }
            registry.log_stats_with(&[]);
        }
        (registry, ids.into_iter().flatten().collect())
    }

    pub(crate) const VIEWPORT: [f32; 2] = [1865.0, 1020.0];

    /// What the species of a test registry have now: their records' counts and energies.
    pub(crate) fn now_of(registry: &SpeciesRegistry) -> Vec<(SpeciesId, SpeciesDataPoint)> {
        registry
            .records()
            .iter()
            .filter(|record| record.stats.count > 0)
            .map(|record| {
                (
                    record.id,
                    SpeciesDataPoint::from_counts(record.stats.count, record.stats.energy),
                )
            })
            .collect()
    }

    /// The layout of a test registry for the standard viewport.
    pub(crate) fn layout_of(
        registry: &SpeciesRegistry,
        hide_extinct: bool,
        show_favorites: bool,
    ) -> MeshLayout {
        MeshLayout::build(
            registry,
            &now_of(registry),
            hide_extinct,
            show_favorites,
            VIEWPORT,
        )
    }

    /// A root that went extinct after three logs, a root that lived through all of it, and a child of the second one that
    /// showed up at the fourth log.
    pub(crate) fn three() -> (SpeciesRegistry, Vec<SpeciesId>) {
        registry(&[
            Life {
                parent: None,
                born: 0,
                logs: &[(5, 100.0), (5, 100.0), (5, 100.0)],
            },
            Life {
                parent: None,
                born: 0,
                logs: &[(5, 300.0); 6],
            },
            Life {
                parent: Some(1),
                born: 3,
                logs: &[(3, 200.0); 3],
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{Life, VIEWPORT, layout_of, registry, three};
    use super::*;
    use bibites_core::brain::Brain;
    use bibites_core::genome::Genome;

    #[test]
    fn a_rung_tells_its_time_count_and_energy() {
        let point = SpeciesDataPoint::from_counts(3, 250.0);
        let rung = Rung::new(&point, 1000.0, 12, 11);
        assert_eq!(rung.count, 3);
        assert!((rung.count_share - 0.25).abs() < 1e-6);
        assert!((rung.energy_share - 0.25).abs() < 1e-6);
        let text = rung.text();
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines.len(), 3, "{text:?}");
        assert!(lines[0].ends_with(" ago"), "{text:?}");
        assert_eq!(lines[1], "Count: 3  (25.0%)");
        // The game's format string has "\n\r" before the last line.
        assert_eq!(lines[2], "\rEnergy: 250 E  (25.0%)");
        let now = Rung::new(&point, 1000.0, 12, -1);
        assert!(now.text().starts_with("now\nCount: 3"));
        // .NET rounds halves away from zero: 12.25 is F1 12.3, 0.5 is F0 1; the shares divide by at least one.
        assert_eq!(net_fixed(12.25, 1), "12.3");
        assert_eq!(net_fixed(0.5, 0), "1");
        let empty = Rung::empty(4).text();
        assert_eq!(empty.split('\n').nth(1), Some("Count: 0  (0.0%)"));
    }

    #[test]
    fn the_shapes_run_from_now_to_the_oldest_point_and_split_the_strip_by_energy() {
        let (registry, ids) = three();
        let layout = layout_of(&registry, false, false);
        assert!(layout.built);
        assert!(layout.min_zoom > 0.0 && layout.min_zoom <= 1.0);
        assert_eq!(layout.size[0], VIEWPORT[0]);
        assert!(layout.size[1] >= VIEWPORT[1]);
        // 6 logs: the oldest point is index 5, so the axis runs to 6.
        assert_eq!(layout.max_n, 6);
        let living = layout.mesh_of(ids[1]).expect("the living root");
        assert!(living.alive, "a living species is open at 'now'");
        assert_eq!(living.bottom_height(), 0.0);
        assert!(
            (living.top_height() - time_axis().height_of_point(6, DEFAULT_STEP)).abs() < 1e-3,
            "the top cap is on the oldest point's line"
        );
        assert_eq!(living.rungs[0].index, -1, "the first rung is now");
        assert_eq!(living.rungs.last().map(|rung| rung.index), Some(6));
        let gone = layout.mesh_of(ids[0]).expect("the extinct root");
        assert!(!gone.alive, "an extinct species ends in a point");
        // It lasted until the third log, so it ends in the point of the first log without it: index 2 of the log.
        assert!(
            (gone.bottom_height() - time_axis().height_of_point(2, DEFAULT_STEP)).abs() < 1e-3,
            "{} {}",
            gone.bottom_height(),
            time_axis().height_of_point(2, DEFAULT_STEP)
        );
        // The shapes side by side fill what the energy says of the strip, centred in it.
        let room = VIEWPORT[0] - 2.0 * SIDE_PADDING;
        let height = time_axis().height_of_point(4, DEFAULT_STEP) + 1.0;
        let spans: Vec<Profile> = layout
            .meshes
            .iter()
            .filter_map(|mesh| mesh.profile_at(height))
            .collect();
        assert_eq!(
            spans.len(),
            2,
            "the root and the extinct one at that height"
        );
        let left = spans.iter().map(|s| s.left).fold(f32::INFINITY, f32::min);
        let right = spans
            .iter()
            .map(|s| s.right)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            ((left + right) / 2.0 - room / 2.0).abs() < 0.5,
            "{left} {right}"
        );
        assert!(right - left <= room + 0.01);
        // `OrderByDescending(totalIndexOfApparition)`: the species that showed up first (the largest index) come first, in the
        // order the shapes were made when they showed up together.
        assert_eq!(layout.displayed, vec![ids[1], ids[0], ids[2]]);
    }

    #[test]
    fn the_energy_of_the_strip_scales_its_width() {
        // The most energy there ever was is at the newest log: the older ones have a third and two thirds of it.
        let (registry, _) = registry(&[Life {
            parent: None,
            born: 0,
            logs: &[(5, 300.0), (5, 600.0), (5, 900.0)],
        }]);
        let layout = layout_of(&registry, false, false);
        let mesh = &layout.meshes[0];
        let room = VIEWPORT[0] - 2.0 * SIDE_PADDING;
        assert!((registry.max_energy() - 900.0).abs() < 1e-3);
        // The log's points: index 0 is 900, 1 is 600 and 2 is 300; the profile is read just under each one's line.
        let width_at = |index: i32| {
            let y = time_axis().height_of_point(index, DEFAULT_STEP) - 0.001;
            let profile = mesh.profile_at(y).expect("inside");
            profile.right - profile.left
        };
        assert!((width_at(0) - room).abs() < 0.5, "{}", width_at(0));
        let expected = |energy: f32| room * 3.0_f32.powf((energy / 900.0).log10());
        assert!(
            (width_at(1) - expected(600.0)).abs() < 0.5,
            "{} {}",
            width_at(1),
            expected(600.0)
        );
        assert!(
            (width_at(2) - expected(300.0)).abs() < 0.5,
            "{} {}",
            width_at(2),
            expected(300.0)
        );
        assert!(width_at(2) < width_at(1) && width_at(1) < width_at(0));
    }

    #[test]
    fn a_child_fades_into_its_parents_colour_up_to_where_the_parent_ends() {
        let (registry, ids) = three();
        let layout = layout_of(&registry, false, false);
        let child = layout.mesh_of(ids[2]).expect("child");
        let root = layout.mesh_of(ids[1]).expect("root");
        assert!(child.has_parent && !root.has_parent);
        assert_eq!(root.blend(), 0.0, "a root does not fade");
        assert!(
            child.blend() >= 0.0 && child.blend() <= 1.0,
            "{}",
            child.blend()
        );
        assert!(child.fade_end <= child.absolute_max_fade_end + 1e-3);
        // The child fades at least from its parent's bottom (now, 0) and the fade ends under its own top.
        assert!(child.fade_end >= 0.0 && child.fade_end <= child.top_height());
        assert_eq!(root.parent_colour, root.colour);
        // The child's and its parent's template are the same here, so are the colours.
        assert_eq!(child.parent_colour, root.colour);
    }

    #[test]
    fn hiding_extinct_lineages_keeps_what_is_alive_and_its_ancestors() {
        let (registry, ids) = registry(&[
            Life {
                parent: None,
                born: 0,
                logs: &[(5, 100.0), (5, 100.0)],
            },
            Life {
                parent: Some(0),
                born: 1,
                logs: &[(4, 100.0); 3],
            },
            Life {
                parent: None,
                born: 0,
                logs: &[(2, 50.0), (2, 50.0)],
            },
        ]);
        let all = layout_of(&registry, false, false);
        let alive = layout_of(&registry, true, false);
        assert_eq!(all.meshes.len(), 3);
        // The child lives, so its extinct parent stays; the other root died out.
        assert!(alive.mesh_of(ids[1]).is_some() && alive.mesh_of(ids[0]).is_some());
        assert!(alive.mesh_of(ids[2]).is_none());
        assert_eq!(alive.displayed.len(), 2);
        // A favourite is kept with "Show Favorited Lineages".
        let mut registry = registry;
        registry.get_mut(ids[2]).expect("species").favorite = true;
        assert!(layout_of(&registry, true, false).mesh_of(ids[2]).is_none());
        assert!(layout_of(&registry, true, true).mesh_of(ids[2]).is_some());
    }

    #[test]
    fn a_big_shape_shows_its_species_picture_and_a_thin_one_does_not() {
        let (registry, ids) = registry(&[
            // Twelve logs fit the newest scale, so nothing is compressed (and the registry prunes nothing).
            Life {
                parent: None,
                born: 0,
                logs: &[(8, 4000.0); 12],
            },
            Life {
                parent: None,
                born: 0,
                logs: &[(1, 1.0), (1, 1.0)],
            },
        ]);
        let layout = layout_of(&registry, false, false);
        let big = layout.mesh_of(ids[0]).expect("long lived");
        let preview = big.preview.expect("a big shape holds the picture");
        assert!(preview.size > 50.0 && preview.size <= 400.0, "{preview:?}");
        // The square is inside the shape.
        let half = preview.size / 2.0;
        for (dx, dy) in [(-half, -half), (half, -half), (-half, half), (half, half)] {
            let at = [
                big.origin[0] + preview.centre[0] + dx,
                big.origin[1] + preview.centre[1] + dy,
            ];
            let profile = big
                .profile_at(at[1].clamp(big.origin[1] + 0.01, big.top_height() - 0.01))
                .expect("inside");
            assert!(
                at[0] >= profile.left - 1.0 && at[0] <= profile.right + 1.0,
                "{at:?} {profile:?}"
            );
        }
        let thin = layout.mesh_of(ids[1]).expect("short lived");
        assert!(thin.preview.is_none());
    }

    #[test]
    fn the_point_under_the_pointer_finds_its_shape_and_its_nearest_rung() {
        let (registry, ids) = registry(&[Life {
            parent: None,
            born: 0,
            logs: &[(5, 300.0); 12],
        }]);
        let layout = layout_of(&registry, false, false);
        let mesh = layout.mesh_of(ids[0]).expect("species");
        let height = mesh.bottom_height() + mesh.size[1] / 2.0;
        let profile = mesh.profile_at(height).expect("inside its height");
        let middle = (profile.left + profile.right) / 2.0;
        assert_eq!(
            layout.mesh_at(middle, height).map(|m| m.species),
            Some(ids[0])
        );
        // Outside the shape: nothing; above and below it too.
        assert!(layout.mesh_at(middle + 5000.0, height).is_none());
        assert!(layout.mesh_at(middle, mesh.top_height() + 100.0).is_none());
        assert!(layout.mesh_at(middle, -5.0).is_none());
        // The rung nearest to a height is the one whose line is nearest.
        let axis = time_axis();
        let line = axis.height_of_point(5, DEFAULT_STEP);
        let near = mesh.nearest_rung(line + 1.0).expect("a rung");
        assert_eq!(near.index, 5, "{near:?}");
        assert!(near.text().contains("Count: 5"));
        let up = mesh
            .nearest_rung(axis.height_of_point(6, DEFAULT_STEP) - 1.0)
            .expect("a rung");
        assert_eq!(up.index, 6);
    }

    #[test]
    fn a_registry_without_history_has_nothing_to_show() {
        let mut registry = SpeciesRegistry::new();
        registry.add_template_species("Zz zz", "", &Genome::starter(), &Brain::starter());
        let layout = layout_of(&registry, false, false);
        assert!(!layout.built);
        assert!(layout.meshes.is_empty() && layout.displayed.is_empty());
    }

    #[test]
    fn every_layout_has_its_own_serial() {
        let (registry, _) = three();
        let a = layout_of(&registry, false, false);
        let b = layout_of(&registry, false, false);
        assert_ne!(a.serial, b.serial);
    }
}
