//! RUSTibites Improved: the streams, the force towers and the whirlpool towers, stepped together.
//!
//! Each of the three things pushes the bodies of the world ([`crate::stream`], [`crate::force_tower`], [`crate::whirlpool`]). They were three steps that each walked every
//! bibite, every pellet and every egg of the world, one tower after the other, looking each body up by its id: with a few dozen fields in a world of a hundred thousand
//! bodies that was millions of lookups a tick, nearly all of them for a body the field had nothing to do with. This module does the same work in one pass over the world.
//!
//! **What it computes is what the three steps computed, bit for bit.** A body gets, one after the other, the push of every field that reaches it: the force towers first
//! (in the order of the tool list), then the whirlpool towers (the same), then the streams (in the order of the world's list), each push worked out from the velocity the
//! body has by then (a whirlpool's hold depends on it), and the pellets are marked for the physics step in the order the old steps marked them. Only the way the bodies that a
//! field reaches are found is new: a coarse grid over the fields' boxes tells, for the cell a body is in, which fields can reach it, in order, and the field's own test
//! (the one the old steps used, to the last bit) then says whether it does. The old steps are kept in the tests as the reference that this is checked against
//! (`world.rs`, `the_fields_step_..._the_old_steps`).
//!
//! Nothing here draws a random number, and with no field in the world nothing here runs.

use crate::force_tower::ForceTower;
use crate::ids::BibiteId;
use crate::improved::Feature;
use crate::math::Vec2;
use crate::stream::StreamKick;
use crate::tools::Tool;
use crate::whirlpool::WhirlpoolTower;
use crate::world::World;

/// Which of the three kinds of field a step covers (the tick covers all, the single steps one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Covered {
    /// The force towers.
    pub forces: bool,
    /// The whirlpool towers.
    pub whirls: bool,
    /// The streams.
    pub streams: bool,
}

impl Covered {
    /// Every kind: what the tick does.
    pub(crate) const ALL: Covered = Covered { forces: true, whirls: true, streams: true };
    /// The force towers alone.
    pub(crate) const FORCES: Covered = Covered { forces: true, whirls: false, streams: false };
    /// The whirlpool towers alone.
    pub(crate) const WHIRLS: Covered = Covered { forces: false, whirls: true, streams: false };
    /// The streams alone.
    pub(crate) const STREAMS: Covered = Covered { forces: false, whirls: false, streams: true };
}

/// What a field pushes with.
#[derive(Clone, Copy, Debug)]
enum Source {
    Force(ForceTower),
    Whirl(WhirlpoolTower),
    Stream(StreamKick),
}

/// Where a field acts: the test the old steps made of a body's position, to the last bit.
#[derive(Clone, Copy, Debug)]
enum Region {
    /// Nearer than the range to the tower: `(centre - at).length_sq() < range * range`.
    Circle { centre: Vec2, range: f32, reach_sq: f32 },
    /// Inside the box of the band: `low <= at <= high` on both axes.
    Box { low: Vec2, high: Vec2 },
}

impl Region {
    #[inline]
    fn contains(&self, at: Vec2) -> bool {
        match *self {
            Region::Circle { centre, reach_sq, .. } => (centre - at).length_sq() < reach_sq,
            Region::Box { low, high } => at.x >= low.x && at.x <= high.x && at.y >= low.y && at.y <= high.y,
        }
    }

    /// A box with every point the region contains in it, a little roomier than it needs to be (the cells of the grid are chosen from it, and a body must never be left
    /// out of a cell it could be found in); `None` when it is not a finite box, which no body is ever inside.
    fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let (low, high) = match *self {
            Region::Circle { centre, range, .. } => {
                let pad = range * 1.0001 + 0.01;
                (centre - Vec2::splat(pad), centre + Vec2::splat(pad))
            }
            Region::Box { low, high } => {
                let pad = 0.01 + 1.0e-4 * low.x.abs().max(low.y.abs()).max(high.x.abs()).max(high.y.abs());
                (low - Vec2::splat(pad), high + Vec2::splat(pad))
            }
        };
        (low.is_finite() && high.is_finite()).then_some((low, high))
    }
}

/// One field of this tick: what it pushes with, where it acts and on what.
#[derive(Clone, Copy, Debug)]
struct Field {
    source: Source,
    region: Region,
    bibites: bool,
    pellets: bool,
    eggs: bool,
    /// The place of the tower in the tool list (a stream has none).
    tool: Option<usize>,
    /// How many bodies it moved this tick (what the tower's panel says).
    moved: u32,
}

impl Field {
    /// The change of velocity the field gives a body at `at` moving at `vel`: what the old steps asked of the tower or the stream.
    #[inline]
    fn kick(&self, at: Vec2, vel: Vec2, dt: f32) -> Vec2 {
        match &self.source {
            Source::Force(tower) => tower.kick(at, dt),
            Source::Whirl(tower) => tower.kick(at, vel, dt),
            Source::Stream(stream) => stream.kick(at),
        }
    }
}

/// The most places in cells that the fields' boxes may take between them in a tick (the grid is as fine as that allows), and the most cells there may be.
const REGISTRATION_BUDGET: u64 = 20_000;
const MAX_CELLS: usize = 17_500;

/// The cells (first x, first y, last x, last y) the box `(low, high)` reaches in a grid of `nx` by `ny` cells that starts at `origin` and has `inv_cell` cells to the unit.
fn cell_span(origin: Vec2, inv_cell: f32, nx: i32, ny: i32, (low, high): (Vec2, Vec2)) -> (i32, i32, i32, i32) {
    let cx = |x: f32| (((x - origin.x) * inv_cell) as i32).clamp(0, nx - 1);
    let cy = |y: f32| (((y - origin.y) * inv_cell) as i32).clamp(0, ny - 1);
    (cx(low.x), cy(low.y), cx(high.x), cy(high.y))
}

/// A grid over the fields' boxes: for the cell a position is in, the fields (by place in the list, in order) whose box reaches the cell. The cells are as small as the
/// fields allow (a field takes about as many cells as its box is wide in cells, squared, and all of them together at most [`REGISTRATION_BUDGET`]), so that the few
/// bodies in a cell that a field reaches are not the many bodies of a big one.
#[derive(Debug, Default)]
struct Grid {
    origin: Vec2,
    inv_cell: f32,
    nx: i32,
    ny: i32,
    /// The first entry of each cell in `list`, and one more at the end.
    start: Vec<u32>,
    list: Vec<u32>,
    /// Scratch for building.
    fill: Vec<u32>,
    /// One bit for each cell that has a candidate: the test every body makes first, in a few thousand bytes that stay in the first-level cache.
    occupied: Vec<u64>,
}

impl Grid {
    /// Builds the grid for `fields`; `false` when no field has a box (nothing is inside any of them).
    fn build(&mut self, fields: &[Field]) -> bool {
        let mut low = Vec2::splat(f32::INFINITY);
        let mut high = Vec2::splat(f32::NEG_INFINITY);
        let mut any = false;
        for field in fields {
            if let Some((lo, hi)) = field.region.bounds() {
                low = low.min(lo);
                high = high.max(hi);
                any = true;
            }
        }
        if !any || !(low.is_finite() && high.is_finite()) {
            return false;
        }
        let extent = (high.x - low.x).max(high.y - low.y);
        // The finest grid whose fields' places stay inside the budget (the coarsest one if none does).
        let mut chosen = (low, 1.0, 1, 1);
        for side in [128.0_f32, 96.0, 64.0, 48.0, 32.0, 24.0, 16.0, 12.0, 8.0, 6.0, 4.0, 3.0, 2.0, 1.0] {
            let cell = (extent / side).max(1.0);
            let inv_cell = 1.0 / cell;
            let nx = (((high.x - low.x) * inv_cell) as i32).saturating_add(1).clamp(1, 4096);
            let ny = (((high.y - low.y) * inv_cell) as i32).saturating_add(1).clamp(1, 4096);
            chosen = (low, inv_cell, nx, ny);
            if (nx as usize) * (ny as usize) > MAX_CELLS {
                continue;
            }
            let places: u64 = fields
                .iter()
                .filter_map(|field| field.region.bounds())
                .map(|bounds| {
                    let (x0, y0, x1, y1) = cell_span(low, inv_cell, nx, ny, bounds);
                    ((x1 - x0 + 1) as u64) * ((y1 - y0 + 1) as u64)
                })
                .sum();
            if places <= REGISTRATION_BUDGET {
                break;
            }
        }
        (self.origin, self.inv_cell, self.nx, self.ny) = chosen;
        let cells = (self.nx as usize) * (self.ny as usize);
        self.start.clear();
        self.start.resize(cells + 1, 0);
        // Counts per cell (shifted one place so that the prefix sums give the starts), then the fields in order.
        for field in fields {
            if let Some(bounds) = field.region.bounds() {
                let (x0, y0, x1, y1) = cell_span(self.origin, self.inv_cell, self.nx, self.ny, bounds);
                for cy in y0..=y1 {
                    for cx in x0..=x1 {
                        self.start[(cy * self.nx + cx) as usize + 1] += 1;
                    }
                }
            }
        }
        self.occupied.clear();
        self.occupied.resize(cells.div_ceil(64), 0);
        for cell in 0..cells {
            if self.start[cell + 1] > 0 {
                self.occupied[cell >> 6] |= 1 << (cell & 63);
            }
            self.start[cell + 1] += self.start[cell];
        }
        self.list.clear();
        self.list.resize(self.start[cells] as usize, 0);
        self.fill.clear();
        self.fill.extend_from_slice(&self.start[..cells]);
        for (index, field) in fields.iter().enumerate() {
            if let Some(bounds) = field.region.bounds() {
                let (x0, y0, x1, y1) = cell_span(self.origin, self.inv_cell, self.nx, self.ny, bounds);
                for cy in y0..=y1 {
                    for cx in x0..=x1 {
                        let at = (cy * self.nx + cx) as usize;
                        self.list[self.fill[at] as usize] = index as u32;
                        self.fill[at] += 1;
                    }
                }
            }
        }
        true
    }

    /// The fields that can reach `at`, in order: those whose box reaches its cell (a position outside every box has none, up to a cell's width past its edge; the field's own
    /// test says whether it does reach it). The cell comes from a truncating, saturating conversion and one unsigned comparison for each axis, with no branch on the sign:
    /// a position left of the grid by less than a cell lands in cell 0 and one that is not a number in cell 0 too, where the field's test turns them down.
    #[inline]
    fn candidates(&self, at: Vec2) -> &[u32] {
        let cx = ((at.x - self.origin.x) * self.inv_cell) as i32;
        let cy = ((at.y - self.origin.y) * self.inv_cell) as i32;
        if (cx as u32) >= (self.nx as u32) || (cy as u32) >= (self.ny as u32) {
            return &[];
        }
        let cell = (cy * self.nx + cx) as usize;
        if (self.occupied[cell >> 6] >> (cell & 63)) & 1 == 0 {
            return &[];
        }
        &self.list[self.start[cell] as usize..self.start[cell + 1] as usize]
    }
}

/// What a step keeps from one tick to the next so that it allocates nothing once it has run.
#[derive(Debug, Default)]
pub(crate) struct FieldScratch {
    fields: Vec<Field>,
    /// The places in the tool list of the towers whose count of bodies moved is written (the inactive ones too: it says 0 then).
    towers: Vec<usize>,
    grid: Grid,
    bibite_hits: Vec<(BibiteId, Vec2)>,
    pellet_hits: Vec<(u32, Vec2)>,
    bibites: Vec<Vec<(BibiteId, Vec2)>>,
    pellets: Vec<Vec<(u32, Vec2)>>,
    eggs: Vec<Vec<(crate::ids::EggId, Vec2)>>,
}

/// Empties `lists` to `count` empty lists, keeping what room they had.
fn reset<T>(lists: &mut Vec<Vec<T>>, count: usize) {
    for list in lists.iter_mut() {
        list.clear();
    }
    if lists.len() < count {
        lists.resize_with(count, Vec::new);
    }
}

impl World {
    /// RUSTibites Improved, after the pellet producers and before the bodies move (phase 13): the fields `covered` names, in one pass (see the module's documentation).
    /// A kind of field acts only while its switch ([`Feature::ForceTower`], [`Feature::Whirlpool`], [`Feature::Streams`]) is on and the world has one.
    pub(crate) fn step_fields(&mut self, dt: f32, covered: Covered) {
        let forces = covered.forces && self.improved.is_on(Feature::ForceTower) && self.tools.tools().iter().any(|tool| matches!(tool, Tool::Force(_)));
        let whirls = covered.whirls && self.improved.is_on(Feature::Whirlpool) && self.tools.tools().iter().any(|tool| matches!(tool, Tool::Whirl(_)));
        let streams = covered.streams && self.improved.is_on(Feature::Streams) && self.streams.iter().any(crate::stream::Stream::is_active);
        if !(forces || whirls || streams) {
            return;
        }
        let mut scratch = std::mem::take(&mut self.field_scratch);
        scratch.fields.clear();
        scratch.towers.clear();
        // The fields in the order the old steps took them: the force towers, the whirlpool towers, the streams.
        if forces {
            for (index, tool) in self.tools.tools().iter().enumerate() {
                if let Tool::Force(tower) = tool {
                    scratch.towers.push(index);
                    if tower.is_active() {
                        let range = tower.range;
                        scratch.fields.push(Field {
                            source: Source::Force(*tower),
                            region: Region::Circle { centre: tower.base.pos, range, reach_sq: range * range },
                            bibites: tower.bibites,
                            pellets: tower.pellets,
                            eggs: tower.eggs,
                            tool: Some(index),
                            moved: 0,
                        });
                    }
                }
            }
        }
        if whirls {
            for (index, tool) in self.tools.tools().iter().enumerate() {
                if let Tool::Whirl(tower) = tool {
                    scratch.towers.push(index);
                    if tower.is_active() {
                        let range = tower.range;
                        scratch.fields.push(Field {
                            source: Source::Whirl(*tower),
                            region: Region::Circle { centre: tower.base.pos, range, reach_sq: range * range },
                            bibites: tower.bibites,
                            pellets: tower.pellets,
                            eggs: tower.eggs,
                            tool: Some(index),
                            moved: 0,
                        });
                    }
                }
            }
        }
        if streams {
            for stream in &self.streams {
                let Some(corners) = stream.corners().filter(|_| stream.is_active()) else {
                    continue;
                };
                let Some(kick) = stream.prepare(dt) else {
                    continue;
                };
                // The box of the band: a body outside it is not in the band.
                let (low, high) = corners.iter().fold((corners[0], corners[0]), |(low, high), corner| (low.min(*corner), high.max(*corner)));
                scratch.fields.push(Field { source: Source::Stream(kick), region: Region::Box { low, high }, bibites: true, pellets: true, eggs: true, tool: None, moved: 0 });
            }
        }
        if !scratch.fields.is_empty() && scratch.grid.build(&scratch.fields) {
            let FieldScratch { fields, grid, bibite_hits, pellet_hits, bibites, pellets, eggs, .. } = &mut scratch;
            let count = fields.len();
            // The bibites: the records the grid says some field can reach, found on the pool when there are many, then sorted into the fields' lists in dense order.
            if fields.iter().any(|field| field.bibites) {
                bibite_hits.clear();
                let executor = crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref());
                let grid_ref: &Grid = grid;
                self.bibites.filter_map_dense_into(executor, bibite_hits, |id, record| (!grid_ref.candidates(record.pos).is_empty()).then_some((id, record.pos)));
                reset(bibites, count);
                for &(id, pos) in bibite_hits.iter() {
                    for &at in grid.candidates(pos) {
                        let field = &fields[at as usize];
                        if field.bibites && field.region.contains(pos) {
                            bibites[at as usize].push((id, pos));
                        }
                    }
                }
                for (at, hits) in bibites.iter().enumerate().take(count) {
                    for &(id, pos) in hits {
                        let Some(vel) = self.bibites.velocity_of(id) else {
                            continue;
                        };
                        let kick = fields[at].kick(pos, vel, dt);
                        if kick != Vec2::ZERO && kick.is_finite() && self.bibites.push_velocity(id, kick) {
                            fields[at].moved += 1;
                        }
                    }
                }
            }
            // The pellets no mouth holds, in dense order; a pellet that is pushed wakes up, and the physics step hears of them in the order of the fields, as it did.
            if fields.iter().any(|field| field.pellets) {
                reset(pellets, count);
                pellet_hits.clear();
                let executor = crate::sim::ParallelExecutor::new(self.sensing_pool.as_ref());
                let grid_ref: &Grid = grid;
                // (The test of a pellet is a few instructions, so the pool pays from a larger store than for the scans that do real work on each.)
                self.pellets.free_where_into(executor, self.scan_gate().saturating_mul(8), pellet_hits, |pos| !grid_ref.candidates(pos).is_empty());
                for &(slot, pos) in pellet_hits.iter() {
                    for &at in grid.candidates(pos) {
                        let field = &fields[at as usize];
                        if field.pellets && field.region.contains(pos) {
                            pellets[at as usize].push((slot, pos));
                        }
                    }
                }
                for (at, hits) in pellets.iter().enumerate().take(count) {
                    for &(slot, pos) in hits {
                        let vel = self.pellets.velocity_of_slot(slot);
                        let kick = fields[at].kick(pos, vel, dt);
                        if kick != Vec2::ZERO && kick.is_finite() {
                            self.pellets.add_velocity_to_slot(slot, kick);
                            fields[at].moved += 1;
                        }
                    }
                }
            }
            if fields.iter().any(|field| field.eggs) {
                reset(eggs, count);
                for id in self.eggs.ids() {
                    let Some(pos) = self.eggs.get(id).map(|egg| egg.pos) else {
                        continue;
                    };
                    for &at in grid.candidates(pos) {
                        let field = &fields[at as usize];
                        if field.eggs && field.region.contains(pos) {
                            eggs[at as usize].push((id, pos));
                        }
                    }
                }
                for (at, hits) in eggs.iter().enumerate().take(count) {
                    for &(id, pos) in hits {
                        let Some(vel) = self.eggs.get(id).map(|egg| egg.vel) else {
                            continue;
                        };
                        let kick = fields[at].kick(pos, vel, dt);
                        if kick != Vec2::ZERO
                            && kick.is_finite()
                            && let Some(egg) = self.eggs.get_mut(id)
                            && egg.vel.is_finite()
                        {
                            egg.vel += kick;
                            fields[at].moved += 1;
                        }
                    }
                }
            }
        }
        // The towers' own counts of the bodies they moved in this tick (0 for a tower that is not acting).
        for &index in &scratch.towers {
            let moved = scratch.fields.iter().find(|field| field.tool == Some(index)).map_or(0, |field| field.moved);
            match self.tools.tools_mut().get_mut(index) {
                Some(Tool::Force(tower)) => tower.affected = moved,
                Some(Tool::Whirl(tower)) => tower.affected = moved,
                _ => {}
            }
        }
        self.field_scratch = scratch;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scatter(u64);

    impl Scatter {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 40) as f32) / ((1_u64 << 24) as f32)
        }

        fn between(&mut self, low: f32, high: f32) -> f32 {
            low + (high - low) * self.next()
        }
    }

    fn dummy(region: Region) -> Field {
        Field { source: Source::Force(ForceTower::new(0, Vec2::ZERO)), region, bibites: true, pellets: true, eggs: true, tool: None, moved: 0 }
    }

    fn circle(centre: Vec2, range: f32) -> Region {
        Region::Circle { centre, range, reach_sq: range * range }
    }

    #[test]
    fn the_grid_lists_every_field_that_contains_a_point_in_order_whatever_the_fields_and_the_point() {
        let mut rng = Scatter(0x6A1D);
        let mut grid = Grid::default();
        for case in 0..60 {
            let count = 1 + (case * 7) % 70;
            let mut fields = Vec::new();
            for _ in 0..count {
                let centre = Vec2::new(rng.between(-3000.0, 3000.0), rng.between(-3000.0, 3000.0));
                if rng.next() < 0.5 {
                    fields.push(dummy(circle(centre, rng.between(10.0, 5000.0))));
                } else {
                    let size = Vec2::new(rng.between(1.0, 4000.0), rng.between(1.0, 4000.0));
                    fields.push(dummy(Region::Box { low: centre, high: centre + size }));
                }
            }
            assert!(grid.build(&fields), "case {case}: there are boxes");
            // Random points, and the points on the edges of every region (the edges are where a cell could be missed).
            let mut points: Vec<Vec2> = (0..1500).map(|_| Vec2::new(rng.between(-9000.0, 9000.0), rng.between(-9000.0, 9000.0))).collect();
            for field in &fields {
                match field.region {
                    Region::Circle { centre, range, .. } => {
                        for (dx, dy) in [(range, 0.0), (-range, 0.0), (0.0, range), (0.0, -range), (range * 0.999, 0.0), (range * 1.001, 0.0), (0.0, 0.0)] {
                            points.push(centre + Vec2::new(dx, dy));
                        }
                    }
                    Region::Box { low, high } => {
                        for (x, y) in [(low.x, low.y), (high.x, high.y), (low.x, high.y), (high.x, low.y), ((low.x + high.x) * 0.5, low.y), (low.x - 1e-3, low.y), (high.x + 1e-3, high.y)] {
                            points.push(Vec2::new(x, y));
                        }
                    }
                }
            }
            for at in points {
                let found = grid.candidates(at);
                assert!(found.windows(2).all(|pair| pair[0] < pair[1]), "case {case}: the candidates are in the order of the fields: {found:?}");
                for (index, field) in fields.iter().enumerate() {
                    if field.region.contains(at) {
                        assert!(found.contains(&(index as u32)), "case {case}: field {index} contains {at:?} but the grid does not list it ({found:?})");
                    }
                }
            }
        }
    }

    #[test]
    fn a_position_that_is_not_a_number_or_far_outside_has_no_candidates_and_a_field_without_a_box_is_never_found() {
        let mut grid = Grid::default();
        let fields = vec![dummy(circle(Vec2::ZERO, 100.0)), dummy(circle(Vec2::new(f32::NAN, 0.0), 100.0)), dummy(Region::Box { low: Vec2::new(-5.0, -5.0), high: Vec2::new(5.0, 5.0) })];
        assert!(grid.build(&fields));
        for at in [Vec2::new(f32::INFINITY, 0.0), Vec2::new(1e9, 1e9), Vec2::new(-1e9, 0.0), Vec2::new(0.0, f32::NEG_INFINITY)] {
            assert!(grid.candidates(at).is_empty(), "{at:?}");
        }
        // A position that is not a number may come out with candidates (cell 0): no field contains it.
        for at in [Vec2::new(f32::NAN, 0.0), Vec2::new(0.0, f32::NAN), Vec2::new(f32::NAN, f32::NAN)] {
            assert!(!fields.iter().any(|field| field.region.contains(at)), "{at:?}");
        }
        assert_eq!(grid.candidates(Vec2::ZERO), &[0, 2]);
        // No field has a box: nothing to build.
        let nothing = vec![dummy(circle(Vec2::new(f32::NAN, 0.0), 100.0))];
        assert!(!grid.build(&nothing));
        assert!(!circle(Vec2::new(f32::NAN, 0.0), 100.0).contains(Vec2::ZERO));
    }

    #[test]
    fn a_region_tests_a_position_as_the_old_steps_did() {
        let centre = Vec2::new(3.0, -4.0);
        let region = circle(centre, 100.0);
        for at in [Vec2::new(103.0, -4.0), Vec2::new(102.99, -4.0), Vec2::new(3.0, 96.0), Vec2::new(63.0, 76.0), Vec2::new(3.0, -4.0)] {
            assert_eq!(region.contains(at), (centre - at).length_sq() < 100.0 * 100.0, "{at:?}");
        }
        let band = Region::Box { low: Vec2::new(-200.0, -50.0), high: Vec2::new(200.0, 50.0) };
        assert!(band.contains(Vec2::new(-200.0, 50.0)) && band.contains(Vec2::new(200.0, -50.0)), "the edges are in");
        assert!(!band.contains(Vec2::new(200.1, 0.0)) && !band.contains(Vec2::new(0.0, -50.1)));
    }
}
