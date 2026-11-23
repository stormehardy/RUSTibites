//! Click and rectangle selection (`UserControl.cs`) and the numbers of the multi-selection panel
//! (`SelectionPanel.cs`).
//!
//! * A press shorter than [`CLICK_MAX_SECONDS`] is a click. On empty space it selects the closest
//!   living bibite whose edge is within `orthographicSize * SelectionRadius` of the cursor
//!   (`SelectClosestBibiteInCircle`); a click that lands on a bibite or egg body selects that one
//!   (`TargetObjectOnClick`). Clicking empty space with nothing near does not deselect.
//! * A longer press is a drag: every bibite, egg and plant/meat pellet with one corner of its
//!   collider bounds inside the screen rectangle joins the multi-selection
//!   (`SelectObjectsInRectangle`). A bibite's bounds are the union of its Mouth and Tail collider
//!   bounds (`getSelectionCorners`), the other entities' their own collider.
//! * The panel's counts and energies are a snapshot taken when the selection is made or an entry
//!   is removed from it (`UpdatePanelInformationFromSelection`), not live values.

use bibites_core::body::{BibiteRecord, EggRecord};
use bibites_core::format::FloatValueFormat;
use bibites_core::genome::Gene;
use bibites_core::ids::{BibiteId, EggId, PelletId, SpeciesId};
use bibites_core::math::Vec2;
use bibites_core::pellet::MaterialKind;
use bibites_core::physics::prefab;
use bibites_core::world::World;

/// `UserControl.minButtonDownTimeToBeADragAction`: presses shorter than this are clicks.
pub const CLICK_MAX_SECONDS: f32 = 0.15;
/// `UserSettings.SelectionRadius` default (fraction of the half screen height).
pub const DEFAULT_SELECTION_RADIUS: f32 = 0.075;

/// Something a click can land on directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entity {
    /// A bibite (living or a corpse).
    Bibite(BibiteId),
    /// An egg.
    Egg(EggId),
}

/// One kind of entry in the selection panel (`SelectionPanel.MultiSelectableEntity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Bibites.
    Bibite,
    /// Eggs.
    Egg,
    /// Plant pellets.
    Plant,
    /// Meat pellets.
    Meat,
}

impl Kind {
    /// Panel order and row label (`bibitesSelectedHandle`, `eggsSelectedHandle`, ...).
    pub const ALL: [(Self, &'static str); 4] = [
        (Self::Bibite, "Bibites"),
        (Self::Egg, "Eggs"),
        (Self::Plant, "Plants"),
        (Self::Meat, "Meats"),
    ];

    /// Index in [`Self::ALL`].
    #[must_use]
    pub fn index(self) -> u8 {
        match self {
            Self::Bibite => 0,
            Self::Egg => 1,
            Self::Plant => 2,
            Self::Meat => 3,
        }
    }

    /// Kind of an index in [`Self::ALL`].
    #[must_use]
    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL
            .get(usize::from(index))
            .map(|(kind, _label)| *kind)
    }
}

/// Count and energy of one panel row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tally {
    /// Entities of this kind.
    pub count: usize,
    /// Their summed energy (`totalEnergy`, `energy`).
    pub energy: f32,
}

/// One row of the species list (`bibitesPerSpecies`).
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesRow {
    /// The species.
    pub species: SpeciesId,
    /// Its name.
    pub name: String,
    /// Selected bibites of it and their energy.
    pub tally: Tally,
}

/// What the panel shows: filled in by [`MultiSelection::refresh`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelInfo {
    /// Every kind's row, in [`Kind::ALL`] order.
    pub kinds: [Tally; 4],
    /// Species rows, the most numerous first.
    pub species: Vec<SpeciesRow>,
}

impl PanelInfo {
    /// Entities in the selection.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.kinds.iter().map(|tally| tally.count).sum()
    }

    /// Summed energy of the selection.
    #[must_use]
    pub fn total_energy(&self) -> f32 {
        self.kinds.iter().map(|tally| tally.energy).sum()
    }
}

/// The entities picked by a drag rectangle (`UserControl.multiSelection` and the panel's lists).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiSelection {
    /// Selected bibites (living and corpses).
    pub bibites: Vec<BibiteId>,
    /// Selected eggs.
    pub eggs: Vec<EggId>,
    /// Selected plant pellets.
    pub plants: Vec<PelletId>,
    /// Selected meat pellets.
    pub meats: Vec<PelletId>,
    /// The panel's numbers (a snapshot, see the module docs).
    pub info: PanelInfo,
}

impl MultiSelection {
    /// `true` when nothing is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bibites.is_empty()
            && self.eggs.is_empty()
            && self.plants.is_empty()
            && self.meats.is_empty()
    }

    /// Entities picked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bibites.len() + self.eggs.len() + self.plants.len() + self.meats.len()
    }

    /// Recomputes the panel's numbers from the world (`UpdatePanelInformationFromSelection`).
    pub fn refresh(&mut self, world: &World) {
        let mut info = PanelInfo::default();
        let mut species: Vec<SpeciesRow> = Vec::new();
        for &id in &self.bibites {
            let Some(record) = world.bibites.get(id) else {
                continue;
            };
            let energy = bibite_energy(world, record);
            info.kinds[0].count += 1;
            info.kinds[0].energy += energy;
            let Some(id) = record.meta.species else {
                continue;
            };
            match species.iter_mut().find(|row| row.species == id) {
                Some(row) => {
                    row.tally.count += 1;
                    row.tally.energy += energy;
                }
                None => species.push(SpeciesRow {
                    species: id,
                    name: world
                        .species
                        .get(id)
                        .map_or_else(|| format!("Species {id}"), |record| record.name()),
                    tally: Tally { count: 1, energy },
                }),
            }
        }
        for &id in &self.eggs {
            if let Some(egg) = world.eggs.get(id) {
                info.kinds[1].count += 1;
                info.kinds[1].energy += egg.hatch.energy;
            }
        }
        for (slot, list) in [(2, &self.plants), (3, &self.meats)] {
            for &id in list {
                if let Some(energy) = world.pellets.energy(id) {
                    info.kinds[slot].count += 1;
                    info.kinds[slot].energy += energy;
                }
            }
        }
        // `OrderByDescending(count)` over the dictionary's insertion order (a stable sort).
        species.sort_by_key(|row| std::cmp::Reverse(row.tally.count));
        info.species = species;
        self.info = info;
    }

    /// Takes a whole kind out of the selection (the row's X button).
    pub fn remove_kind(&mut self, kind: Kind, world: &World) {
        match kind {
            Kind::Bibite => self.bibites.clear(),
            Kind::Egg => self.eggs.clear(),
            Kind::Plant => self.plants.clear(),
            Kind::Meat => self.meats.clear(),
        }
        self.refresh(world);
    }

    /// Takes one species' bibites out of the selection (a species row's X button).
    pub fn remove_species(&mut self, species: SpeciesId, world: &World) {
        self.bibites.retain(|&id| {
            world
                .bibites
                .get(id)
                .is_none_or(|record| record.meta.species != Some(species))
        });
        self.refresh(world);
    }

    /// `SelectionPanel.KillSelection`: living bibites die, corpses explode into meat, eggs are
    /// aborted and pellets destroyed. Entities that no longer exist are skipped.
    pub fn kill(&self, world: &mut World) -> usize {
        let mut affected = 0;
        for &id in &self.bibites {
            affected += usize::from(world.kill_or_explode_bibite(id));
        }
        for &id in &self.eggs {
            affected += usize::from(world.abort_egg(id));
        }
        for &id in self.plants.iter().chain(&self.meats) {
            if world.pellets.contains(id) {
                world.despawn_pellet(id);
                affected += 1;
            }
        }
        affected
    }

    /// `SelectionPanel.LayEggs`: every living selected bibite lays one egg (no energy cost).
    pub fn lay_eggs(&self, world: &mut World) -> u32 {
        let mut laid = 0;
        for &id in &self.bibites {
            if world
                .bibites
                .get(id)
                .is_some_and(|record| !record.body.dead)
            {
                laid += world.lay_egg_once(id);
            }
        }
        laid
    }
}

/// `BibiteBody.totalEnergy` as the panels show it: stored energy plus the fat's and the egg being
/// made's.
#[must_use]
pub fn bibite_energy(world: &World, record: &BibiteRecord) -> f32 {
    let fat_density = world.settings.materials.fat.energy_density;
    let extra =
        record.egg_layer.progress * record.egg_layer.egg_energy + record.fat.amount * fat_density;
    record.body.total_energy(extra)
}

/// Half extents of the world-axis bounds of a box with half size `half` rotated by `angle`.
fn box_half_extents(half: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(
        half.x * cos.abs() + half.y * sin.abs(),
        half.x * sin.abs() + half.y * cos.abs(),
    )
}

/// Half extents of the world-axis bounds of an upright capsule of `size` rotated by `angle`.
fn capsule_half_extents(size: Vec2, angle: f32) -> Vec2 {
    let radius = size.x * 0.5;
    let core = ((size.y - size.x) * 0.5).max(0.0);
    let (sin, cos) = angle.sin_cos();
    Vec2::new(core * sin.abs() + radius, core * cos.abs() + radius)
}

/// World-axis bounds `(min, max)` of a bibite's Mouth and Tail colliders together
/// (`UserControl.getSelectionCorners`).
#[must_use]
pub fn bibite_bounds(record: &BibiteRecord) -> (Vec2, Vec2) {
    let d1 = record.body.geometry.d1_size;
    let rotation = record.rot;
    let mouth_size = prefab::mouth_box_size(record.genome.gene(Gene::Diet)) * d1;
    let mouth_centre = record.pos + prefab::mouth_offset(d1).rotate(rotation);
    let mouth_half = box_half_extents(mouth_size * 0.5, rotation + prefab::PART_ROTATION);
    let inflate = record.body.geometry.inflate_factor;
    let tail_size = prefab::tail_capsule_size(inflate) * d1;
    let tail_centre = record.pos + (prefab::tail_capsule_offset(inflate) * d1).rotate(rotation);
    let tail_half = capsule_half_extents(tail_size, rotation);
    (
        Vec2::new(
            (mouth_centre.x - mouth_half.x).min(tail_centre.x - tail_half.x),
            (mouth_centre.y - mouth_half.y).min(tail_centre.y - tail_half.y),
        ),
        Vec2::new(
            (mouth_centre.x + mouth_half.x).max(tail_centre.x + tail_half.x),
            (mouth_centre.y + mouth_half.y).max(tail_centre.y + tail_half.y),
        ),
    )
}

/// Collider circle of an egg: centre and radius.
#[must_use]
pub fn egg_circle(egg: &EggRecord) -> (Vec2, f32) {
    let scale = egg.size_gene.max(0.0);
    (
        egg.pos + (prefab::EGG_CIRCLE_OFFSET * scale).rotate(egg.rot),
        prefab::EGG_CIRCLE_RADIUS * scale,
    )
}

fn circle_bounds(centre: Vec2, radius: f32) -> (Vec2, Vec2) {
    (
        Vec2::new(centre.x - radius, centre.y - radius),
        Vec2::new(centre.x + radius, centre.y + radius),
    )
}

fn corners((min, max): (Vec2, Vec2)) -> [Vec2; 4] {
    [
        Vec2::new(min.x, min.y),
        Vec2::new(min.x, max.y),
        Vec2::new(max.x, min.y),
        Vec2::new(max.x, max.y),
    ]
}

/// Screen-space test of `UserControl.SelectObjectsInRectangle`: the entities with a corner of their
/// bounds inside the rectangle spanned by `a` and `b` (screen pixels, y down). `to_screen` maps
/// world points to screen pixels, `screen` is the window size; entities whose centre is further
/// than 20 % outside the window are skipped without testing their corners.
#[must_use]
pub fn select_in_rectangle(
    world: &World,
    to_screen: impl Fn(Vec2) -> Vec2,
    screen: Vec2,
    a: Vec2,
    b: Vec2,
) -> MultiSelection {
    let (min, max) = (
        Vec2::new(a.x.min(b.x), a.y.min(b.y)),
        Vec2::new(a.x.max(b.x), a.y.max(b.y)),
    );
    // `Rect.Contains`: the max edges are exclusive.
    let contains =
        |point: Vec2| point.x >= min.x && point.x < max.x && point.y >= min.y && point.y < max.y;
    let near_view = |centre: Vec2| {
        let screen_point = to_screen(centre);
        let (x, y) = (
            screen_point.x / screen.x.max(1.0),
            screen_point.y / screen.y.max(1.0),
        );
        -0.2 < x && x < 1.2 && -0.2 < y && y < 1.2
    };
    let hit = |centre: Vec2, bounds: (Vec2, Vec2)| {
        near_view(centre)
            && corners(bounds)
                .into_iter()
                .any(|corner| contains(to_screen(corner)))
    };
    let mut selection = MultiSelection::default();
    for (id, record) in world.bibites.iter() {
        if hit(record.pos, bibite_bounds(record)) {
            selection.bibites.push(id);
        }
    }
    for id in world.pellets.ids() {
        let Some(view) = world.pellets.get(id) else {
            continue;
        };
        let Some(radius) = world.pellets.collider_radius(id) else {
            continue;
        };
        if !hit(view.pos, circle_bounds(view.pos, radius)) {
            continue;
        }
        match view.material {
            MaterialKind::Plant => selection.plants.push(id),
            MaterialKind::Meat => selection.meats.push(id),
            // Only plants and meat are listed by the panel.
            MaterialKind::Fat | MaterialKind::Armor => {}
        }
    }
    for id in world.eggs.ids() {
        let Some(egg) = world.eggs.get(id) else {
            continue;
        };
        let (centre, radius) = egg_circle(egg);
        if hit(egg.pos, circle_bounds(centre, radius)) {
            selection.eggs.push(id);
        }
    }
    selection.refresh(world);
    selection
}

/// `SelectClosestBibiteInCircle`: the closest living bibite on screen whose edge is within `radius`
/// (world units) of `center`.
#[must_use]
pub fn closest_bibite_in_circle(
    world: &World,
    center: Vec2,
    radius: f32,
    on_screen: impl Fn(Vec2) -> bool,
) -> Option<BibiteId> {
    let mut best: Option<(f32, BibiteId)> = None;
    for (id, record) in world.bibites.iter() {
        if record.body.dead || !on_screen(record.pos) {
            continue;
        }
        let distance = record.pos.distance(center);
        if !(distance - record.body.geometry.effective_radius > radius)
            && best.is_none_or(|(nearest, _)| distance < nearest)
        {
            best = Some((distance, id));
        }
    }
    best.map(|(_, id)| id)
}

/// The bibite or egg whose body is under `point` (a press on it is handled by its own
/// `TargetObjectOnClick`, not by the background's click and drag logic). Corpses count.
#[must_use]
pub fn entity_at(world: &World, point: Vec2) -> Option<Entity> {
    let mut best: Option<(f32, Entity)> = None;
    for (id, record) in world.bibites.iter() {
        let distance = record.pos.distance(point);
        if distance <= record.body.geometry.effective_radius
            && best.is_none_or(|(nearest, _)| distance < nearest)
        {
            best = Some((distance, Entity::Bibite(id)));
        }
    }
    for id in world.eggs.ids() {
        let Some(egg) = world.eggs.get(id) else {
            continue;
        };
        let (centre, radius) = egg_circle(egg);
        let distance = centre.distance(point);
        if distance <= radius && best.is_none_or(|(nearest, _)| distance < nearest) {
            best = Some((distance, Entity::Egg(id)));
        }
    }
    best.map(|(_, entity)| entity)
}

/// The free pellet whose body is under `point` (`DraggableObject` lets the player drag it at
/// once; pellets are not selectable).
#[must_use]
pub fn pellet_at(world: &World, point: Vec2) -> Option<PelletId> {
    let mut best: Option<(f32, PelletId)> = None;
    for id in world.pellets.ids() {
        let Some(view) = world.pellets.get(id) else {
            continue;
        };
        if view.held {
            continue;
        }
        let distance = view.pos.distance(point);
        let radius = world.pellets.radius(id).unwrap_or(0.0);
        if distance <= radius && best.is_none_or(|(nearest, _)| distance < nearest) {
            best = Some((distance, id));
        }
    }
    best.map(|(_, id)| id)
}

/// The pellet whose body is under `point`, found through the world's spatial hash (cheap enough to
/// ask every frame, unlike [`pellet_at`]'s scan): held pellets count too, a pellet in a mouth
/// still has its mouse-over trigger (`PelletTooltipTrigger`).
#[must_use]
pub fn pellet_under(world: &World, point: Vec2) -> Option<PelletId> {
    use bibites_core::world::{ClassMask, QueryScratch};
    let mut scratch = QueryScratch::new();
    let hits = world
        .hash
        .query_circle(point, 0.0, ClassMask::PELLET, &mut scratch);
    hits.hits
        .iter()
        .filter_map(|hit| {
            let id = PelletId::from_handle(hit.handle);
            let distance = world.pellets.pos(id)?.distance(point);
            Some((distance, id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// Radius of a tower's or colour killer's trigger `CircleCollider2D` (the prefabs: 17.5 for the
/// radiation tower and the colour killer, 19 for the pheromone tower).
#[must_use]
pub fn tool_collider_radius(tool: &bibites_core::tools::Tool) -> f32 {
    match tool {
        bibites_core::tools::Tool::Phero(_) => 19.0,
        bibites_core::tools::Tool::Rad(_) | bibites_core::tools::Tool::Killer(_) | bibites_core::tools::Tool::Gene(_) | bibites_core::tools::Tool::Vent(_) | bibites_core::tools::Tool::Force(_) | bibites_core::tools::Tool::Whirl(_) => 17.5,
    }
}

/// The save id of the placed tower or colour killer whose collider is under `point` (the
/// `TargetObjectOnClick` on its prefab; the last placed one wins an overlap). Bibites, eggs and
/// pellets are drawn above the towers, so the caller asks them first.
#[must_use]
pub fn tool_at(world: &World, point: Vec2) -> Option<u64> {
    world
        .tools
        .tools()
        .iter()
        .rev()
        .find(|tool| tool.base().pos.distance(point) <= tool_collider_radius(tool))
        .map(bibites_core::tools::Tool::save_id)
}

/// The number formats of the panel's value fields (`FloatValueTextHandle` settings in the prefab).
pub mod format {
    use super::FloatValueFormat;

    /// "N Entities": integer count followed by a space.
    #[must_use]
    pub fn entity_count() -> FloatValueFormat {
        FloatValueFormat {
            units: " ",
            precision: 0,
            si: false,
            ..FloatValueFormat::default()
        }
    }

    /// The whole selection's energy: `1.23kE`.
    #[must_use]
    pub fn total_energy() -> FloatValueFormat {
        FloatValueFormat {
            units: "E",
            precision: 3,
            si: true,
            ..FloatValueFormat::default()
        }
    }

    /// A row's count.
    #[must_use]
    pub fn row_count() -> FloatValueFormat {
        FloatValueFormat {
            precision: 0,
            si: false,
            ..FloatValueFormat::default()
        }
    }

    /// A row's energy: `(120E)`.
    #[must_use]
    pub fn row_energy() -> FloatValueFormat {
        FloatValueFormat {
            prefix: "(",
            units: "E)",
            precision: 2,
            si: true,
            ..FloatValueFormat::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_bounds_follow_the_rotation() {
        let half = Vec2::new(1.0, 3.0);
        let upright = box_half_extents(half, 0.0);
        assert!((upright.x - 1.0).abs() < 1e-6 && (upright.y - 3.0).abs() < 1e-6);
        let turned = box_half_extents(half, std::f32::consts::FRAC_PI_2);
        assert!((turned.x - 3.0).abs() < 1e-5 && (turned.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn capsule_bounds_are_tighter_than_the_rotated_box() {
        let size = Vec2::new(5.0, 7.0);
        let upright = capsule_half_extents(size, 0.0);
        assert!((upright.x - 2.5).abs() < 1e-6 && (upright.y - 3.5).abs() < 1e-6);
        let angle = std::f32::consts::FRAC_PI_4;
        let capsule = capsule_half_extents(size, angle);
        let rect = box_half_extents(size * 0.5, angle);
        assert!(capsule.x < rect.x && capsule.y < rect.y);
    }

    #[test]
    fn panel_formats_match_the_prefab_texts() {
        assert_eq!(format::entity_count().format(0.0), "0 ");
        assert_eq!(format::entity_count().format(12.0), "12 ");
        assert_eq!(format::row_count().format(7.0), "7");
        // The prefab's "(120E)" is only the editor's sample: precision 2 means two decimals, and
        // `ScientificFormat` puts a space before the SI prefix and units.
        assert_eq!(format::row_energy().format(120.0), "(120.00 E)");
        assert_eq!(format::total_energy().format(1000.0), "1.000 kE");
    }

    #[test]
    fn kind_indices_round_trip() {
        for (kind, _label) in Kind::ALL {
            assert_eq!(Kind::from_index(kind.index()), Some(kind));
        }
        assert_eq!(Kind::from_index(9), None);
    }

    #[test]
    fn counts_and_energy_sum_over_the_kinds() {
        let info = PanelInfo {
            kinds: [
                Tally {
                    count: 2,
                    energy: 10.0,
                },
                Tally {
                    count: 1,
                    energy: 5.0,
                },
                Tally::default(),
                Tally {
                    count: 3,
                    energy: 1.5,
                },
            ],
            species: Vec::new(),
        };
        assert_eq!(info.total_count(), 6);
        assert!((info.total_energy() - 16.5).abs() < 1e-6);
    }
}
