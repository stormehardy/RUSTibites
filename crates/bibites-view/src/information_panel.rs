//! The Information panel (`InformationPanel`, "Information and Statistics"): the counts and the energy of
//! what the world holds as a list and a pie chart, the five most numerous species and the tags.
//!
//! Geometry, fonts and layout rules are the prefab's (level2, `UICanvas/MainUI/Layer1/Panels/Right Panels/
//! QuickStatsHolder/InformationPanel`, read with `tools/unity/dump_ui_tree.py`; the row prefabs are
//! `sharedassets2` 172 `SpeciesInfoHandle`, 183 `wedgePrefab`, 195 `TagElementHandle`). The panel hangs from
//! the top-right corner of the `Right Panels` rectangle (10 units in from the right, 80 down), a
//! `VerticalLayoutGroup` (padding 20 / 20 / 10 / 20, spacing 20) with a `ContentSizeFitter`, so its width
//! follows its widest section and its height the number of rows. The prefab's stored sizes are stale
//! (the panel is saved inactive), so the layout is computed the way Unity does it: preferred sizes from
//! the text, the `LayoutElement`s and the groups' spacing.
//!
//! What it does (`InformationPanel.cs`, `ZoneManager.UpdateBiomass`):
//! * the numbers are a snapshot taken once a second of real time (`CountBiomass(1f)`), the bibites
//!   without the corpses, the eggs apart, "Free" being the zones' free biomass;
//! * the `count` / `energy` button swaps the two sections and the key the species and the tags are sorted
//!   by (at the next refresh, as in the game);
//! * a species row selects a random living member of it, an egg when it has none; a tag row a random
//!   living bibite or egg with that tag ("Untagged" meaning none); Ctrl held repeats the pick whenever
//!   the target is gone (`UserControl.toRepeatOnDeath`);
//! * the tags list scrolls with the wheel (`ScrollRect`, clamped, sensitivity 35, 7 rows high at most),
//!   and shows its scroll bar only when the rows do not fit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bibites_core::format::FloatValueFormat;
use bibites_core::ids::SpeciesId;
use bibites_core::math::Vec2;
use bibites_core::world::World;

use crate::app::App;
use crate::lineage_graph::{clip_nine_slice, intersect};
use crate::text_sdf::SdfFonts;
use crate::ui::{ControlCommand, FontRole, Rgba, UiAction, UiAssets, UiDraw, UiFrame, UiTexture};

/// `CountBiomass(1f)`: seconds of real time between two refreshes.
pub const REFRESH_SECONDS: f32 = 1.0;

/// The top species shown (`speciesItems`: five items made in `InitPanel`).
pub const MAX_SPECIES: usize = 5;
/// The tag rows the list is tall enough for (`Mathf.Min(7, tagItems.Count)`).
const MAX_TAG_ROWS: usize = 7;
/// `ScrollRect.scrollSensitivity` of the tags list.
const SCROLL_SENSITIVITY: f32 = 35.0;

// Hit ids of the panel. An id is a click area's identity: `UiFrame::interact` answers with every action registered under the id the pointer is on, so no
// other panel may have one of these (the Save Bibite dialog and the Biology Flow pop-up were on 8800.. too, and a click on either fired this panel's
// `close` / `mode` as well). The block 8800..=8861 is this panel's; the tags have a block of their own, as there are as many rows as the world has
// tags and a row on 8830 + its index met the scroll bar's 8860 / 8861 from the 31st tag on.
const HIT_BODY: u32 = 8800;
const HIT_CLOSE: u32 = 8801;
const HIT_MODE: u32 = 8802;
const HIT_SPECIES: u32 = 8810;
const HIT_BAR_TRACK: u32 = 8860;
const HIT_BAR_HANDLE: u32 = 8861;
const HIT_TAGS: u32 = 930_000;

const WHITE: Rgba = [255, 255, 255, 255];
const BLACK: Rgba = [0, 0, 0, 255];
/// The row prefabs' text colour (0.149).
const ROW_INK: Rgba = [38, 38, 38, 255];
/// `PieChart`'s `Circle` image: (0.149).
const PIE_BACK: Rgba = [38, 38, 38, 255];

/// The corner the panel hangs from: `Right Panels` ends 10 units in from the canvas' right edge and
/// starts 80 below its top.
const RIGHT_MARGIN: f32 = 10.0;
const TOP: f32 = 80.0;
/// `VerticalLayoutGroup` of the panel.
const PAD_LEFT: f32 = 20.0;
const PAD_RIGHT: f32 = 20.0;
const PAD_TOP: f32 = 10.0;
const PAD_BOTTOM: f32 = 20.0;
const SPACING: f32 = 20.0;
/// Spacing inside the Entities, Species and Tags sections.
const SECTION_SPACING: f32 = 10.0;
/// Spacing of the wedge rows and of a wedge row's parts.
const WEDGE_SPACING: f32 = 5.0;
/// The colour square of a wedge row.
const INDICATOR: f32 = 20.0;
/// `LayoutElement.minWidth` of a wedge row's value.
const VALUE_MIN_W: f32 = 150.0;
/// `LayoutElement.preferredWidth` of the pie chart.
const PIE: f32 = 200.0;
/// The wedges are 3 units inside the dark disc (`wedgePrefab` size (-6, -6)).
const PIE_INSET: f32 = 3.0;
/// Spacing between the rows list and the pie chart.
const SECTION_GAP: f32 = 20.0;
const COUNT_BUTTON_W: f32 = 60.0;
const ENERGY_BUTTON_W: f32 = 75.0;
/// A species / tag row: padding 5, 20 high content, 1 unit between rows.
const ROW_H: f32 = 30.0;
const ROW_PITCH: f32 = 31.0;
const ROW_PAD: f32 = 5.0;
const ROW_GAP: f32 = 5.0;
const SPECIES_NAME_W: f32 = 150.0;
const TAG_NAME_W: f32 = 120.0;
const ROW_COUNT_W: f32 = 40.0;
/// `Content` (padding 5) around the rows holder (padding 3, 3, 3, 0).
const CONTENT_PAD: f32 = 5.0;
const HOLDER_PAD: f32 = 3.0;
/// The tags list's scroll bar (20 wide, the viewport gives up 20 + 25 of spacing for it).
const SCROLLBAR_W: f32 = 20.0;
const SCROLLBAR_SPACE: f32 = 45.0;
/// `Long_UI` sliced frame of the tags list: border 70 at 500 pixels per unit.
const LIST_FRAME_SCALE: f32 = 0.2;

const WEDGE_PLANT: Rgba = [39, 172, 34, 255];
const WEDGE_MEAT: Rgba = [215, 38, 61, 255];
/// (71/85, 0.8431373, 0.8431373).
const WEDGE_EGG: Rgba = [213, 215, 215, 255];
const WEDGE_FREE: Rgba = [170, 188, 65, 255];

/// A font of the prefab and what is needed to place text set in it.
#[derive(Clone, Copy)]
pub(crate) struct Face {
    /// TMP font asset.
    pub(crate) name: &'static str,
    pub(crate) role: FontRole,
    /// Line height over the font size when no font data is loaded.
    pub(crate) line: f32,
}

pub(crate) const ROBOTO_BOLD: Face = Face {
    name: "Roboto-Bold SDF NoOutline",
    role: FontRole::Bold,
    line: 1.171_875,
};
pub(crate) const ROBOTO: Face = Face {
    name: "Roboto-Regular SDF",
    role: FontRole::Body,
    line: 1.171_875,
};
pub(crate) const LIBERATION: Face = Face {
    name: "LiberationSans SDF",
    role: FontRole::Body,
    line: 1.117_5,
};
/// `LiberationSans SDF` with the Bold style: the atlas has no bold face, so TMP widens its glyphs and advances every character more (the section titles
/// and the mode button; the material's face is black whatever the label's colour).
pub(crate) const LIBERATION_BOLD: Face = Face {
    name: "LiberationSans SDF +bold",
    role: FontRole::Bold,
    line: 1.117_5,
};
/// `Roboto-Bold SDF NoOutline` with the Bold style on top ("Entities:").
pub(crate) const ROBOTO_BOLD_STYLE: Face = Face {
    name: "Roboto-Bold SDF NoOutline +bold",
    role: FontRole::Bold,
    line: 1.171_875,
};

/// Text measuring with the loaded fonts, or an estimate without them.
#[derive(Clone, Copy)]
pub(crate) struct Measure<'a> {
    pub(crate) fonts: Option<&'a SdfFonts>,
}

impl Measure<'_> {
    /// Advance of `text` set at `px` canvas units.
    pub(crate) fn width(&self, face: Face, text: &str, px: f32) -> f32 {
        match self.fonts {
            Some(fonts) => fonts.measure_named(Some(face.name), face.role, text, px),
            None => text.chars().count() as f32 * px * 0.56,
        }
    }

    /// Height of one line of `face` at `px`.
    pub(crate) fn height(&self, face: Face, px: f32) -> f32 {
        match self.fonts {
            Some(fonts) => match fonts.font_named(face.name) {
                Some(font) => font.line_height * px / font.point_size.max(1.0),
                None => fonts.line_height(face.role, px),
            },
            None => face.line * px,
        }
    }
}

/// One of the top species.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesRow {
    /// The species.
    pub id: SpeciesId,
    /// Its name.
    pub name: String,
    /// `Species.count`: living bibites and eggs.
    pub count: u32,
    /// `Species.energy`.
    pub energy: f32,
}

/// One tag (`BibiteTag`), or the "Untagged" pseudo-tag.
#[derive(Clone, Debug, PartialEq)]
pub struct TagRow {
    /// The tag's text.
    pub name: String,
    /// `BibiteTag.count`: living bibites and eggs carrying it.
    pub count: u32,
    /// `BibiteTag.energy`: the bibites' total energy and the energy of the eggs that are not aborting.
    pub energy: f32,
}

/// What `InformationPanel.UpdateInformation` was given, and the lists it builds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Living bibites (`BibiteTracker.bibites`, corpses excluded).
    pub bibites: u32,
    /// Eggs (`BibiteTracker.eggs`).
    pub eggs: u32,
    /// `PlantCounter.Count`.
    pub plants: u32,
    /// `MeatCounter.Count`.
    pub meats: u32,
    /// `ZoneManager.freeBiomass`.
    pub free_energy: f32,
    /// Sum of the bibites' `totalEnergy`.
    pub bibite_energy: f32,
    /// Sum of the eggs' energy.
    pub egg_energy: f32,
    /// `PlantCounter.Biomass`.
    pub plant_energy: f32,
    /// `MeatCounter.Biomass`.
    pub meat_energy: f32,
    /// The top species, in the order shown.
    pub species: Vec<SpeciesRow>,
    /// Active species with at least one member (`NSpecies`).
    pub n_species: u32,
    /// The tags in the order shown ("Untagged" included).
    pub tags: Vec<TagRow>,
}

/// The text of the pseudo-tag of the bibites and eggs without a tag.
pub const UNTAGGED: &str = "Untagged";

impl Snapshot {
    /// `ZoneManager.UpdateBiomass` and `InformationPanel.UpdateInformation` on the live world. `by_energy`
    /// is `!countSection.activeSelf`; `tag_order` is the order the tag items were made in
    /// (`tagItems`), kept from one refresh to the next because the sort by count or energy is stable
    /// over it.
    #[must_use]
    pub fn collect(world: &World, by_energy: bool, tag_order: &mut Vec<String>) -> Self {
        let fat_density = world.settings.materials.fat.energy_density;
        let mut snapshot = Self::default();
        let mut untagged = (0_u32, 0.0_f32);
        // `Species.count` / `Species.energy`: the members' number and total energy (`bibites.Count + eggs.Count`).
        let mut species_sums: HashMap<u32, (u32, f32)> = HashMap::new();
        // First-seen order of the real tags, then their sums.
        let mut seen: Vec<&str> = Vec::new();
        let mut tags: HashMap<&str, (u32, f32)> = HashMap::new();
        for (_, record) in world.bibites.iter() {
            if record.body.dead {
                continue;
            }
            snapshot.bibites += 1;
            let extra = record.egg_layer.progress * record.egg_layer.egg_energy
                + record.fat.amount * fat_density;
            let energy = record.body.total_energy(extra);
            snapshot.bibite_energy += energy;
            if let Some(species) = record.meta.species {
                let sums = species_sums.entry(species.value()).or_default();
                sums.0 += 1;
                sums.1 += energy;
            }
            match record
                .meta
                .species_tag
                .as_deref()
                .filter(|tag| !tag.is_empty())
            {
                Some(tag) => {
                    let entry = tags.entry(tag).or_insert_with(|| {
                        seen.push(tag);
                        (0, 0.0)
                    });
                    entry.0 += 1;
                    entry.1 += energy;
                }
                None => {
                    untagged.0 += 1;
                    untagged.1 += energy;
                }
            }
        }
        for id in world.eggs.ids() {
            let Some(egg) = world.eggs.get(id) else {
                continue;
            };
            snapshot.eggs += 1;
            snapshot.egg_energy += egg.hatch.energy;
            // An egg that aborts leaves its species (`EggHatching.Abort`).
            if !egg.hatch.aborting
                && let Some(species) = egg.meta.species
            {
                let sums = species_sums.entry(species.value()).or_default();
                sums.0 += 1;
                sums.1 += egg.hatch.energy;
            }
            // `BibiteTag.energy` leaves out an egg that is aborting.
            let energy = if egg.hatch.aborting {
                0.0
            } else {
                egg.hatch.energy
            };
            match egg
                .meta
                .species_tag
                .as_deref()
                .filter(|tag| !tag.is_empty())
            {
                Some(tag) => {
                    let entry = tags.entry(tag).or_insert_with(|| {
                        seen.push(tag);
                        (0, 0.0)
                    });
                    entry.0 += 1;
                    entry.1 += energy;
                }
                None => {
                    untagged.0 += 1;
                    untagged.1 += energy;
                }
            }
        }
        let pellets = world.pellets.counts();
        snapshot.plants = pellets.plant_count;
        snapshot.meats = pellets.meat_count;
        snapshot.plant_energy = pellets.plant_biomass as f32;
        snapshot.meat_energy = pellets.meat_biomass as f32;
        snapshot.free_energy = world.free_biomass();

        // `tagItems`: the pseudo-tag first, then the tags; an item is found by its text, so a tag
        // called "Untagged" takes over the pseudo-tag's item. Items of tags that are gone are dropped.
        let mut rows: HashMap<String, (u32, f32)> = HashMap::new();
        rows.insert(UNTAGGED.to_owned(), untagged);
        for tag in &seen {
            if let Some(sums) = tags.get(tag) {
                rows.insert((*tag).to_owned(), *sums);
            }
        }
        tag_order.retain(|name| rows.contains_key(name));
        if !tag_order.iter().any(|name| name == UNTAGGED) {
            tag_order.insert(0, UNTAGGED.to_owned());
        }
        for tag in &seen {
            if !tag_order.iter().any(|name| name == tag) {
                tag_order.push((*tag).to_owned());
            }
        }
        let mut ordered: Vec<TagRow> = tag_order
            .iter()
            .filter_map(|name| {
                rows.get(name).map(|(count, energy)| TagRow {
                    name: name.clone(),
                    count: *count,
                    energy: *energy,
                })
            })
            .collect();
        // `OrderByDescending` is stable.
        ordered.sort_by(|a, b| {
            let (x, y) = if by_energy {
                (a.energy, b.energy)
            } else {
                (a.count as f32, b.count as f32)
            };
            y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal)
        });
        snapshot.tags = ordered;

        // The species: `activeSpecies` by count or energy, the first five, stopping at an empty one.
        let mut active: Vec<SpeciesRow> = world
            .species
            .active_species()
            .iter()
            .filter_map(|id| {
                world.species.get(*id).map(|record| {
                    let (count, energy) =
                        species_sums.get(&id.value()).copied().unwrap_or_default();
                    SpeciesRow {
                        id: *id,
                        name: record.name(),
                        count,
                        energy,
                    }
                })
            })
            .collect();
        snapshot.n_species = active.iter().filter(|row| row.count > 0).count() as u32;
        active.sort_by(|a, b| {
            let (x, y) = if by_energy {
                (a.energy, b.energy)
            } else {
                (a.count as f32, b.count as f32)
            };
            y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal)
        });
        snapshot.species = active
            .into_iter()
            .take(MAX_SPECIES)
            .take_while(|row| row.count >= 1)
            .collect();
        snapshot
    }

    /// `countPieChart.UpdateValues` / `energyPieChart.UpdateValues`' arguments.
    fn pie_values(&self, energy: bool) -> Vec<f32> {
        if energy {
            vec![
                self.free_energy,
                self.bibite_energy,
                self.egg_energy,
                self.plant_energy,
                self.meat_energy,
            ]
        } else {
            vec![
                self.bibites as f32,
                self.eggs as f32,
                self.plants as f32,
                self.meats as f32,
            ]
        }
    }
}

/// The panel's state in the app.
#[derive(Debug, Default)]
pub struct InfoState {
    /// `!countSection.activeSelf`: the energy section is the one shown.
    pub energy_mode: bool,
    /// The numbers of the last refresh (`None` until the panel was open once).
    snapshot: Option<Snapshot>,
    /// Real seconds since that refresh.
    since_refresh: f32,
    /// `tagItems`' order, see [`Snapshot::collect`].
    tag_order: Vec<String>,
    /// How far the tags list is scrolled, in canvas units (`Content.anchoredPosition.y`).
    scroll: f32,
    /// The tags list's rectangle on screen (`x, y, w, h` in pixels) of the last frame, for the wheel.
    list_rect: std::cell::Cell<[f32; 4]>,
    /// The panel was open at the last frame (an opening makes the next refresh at once).
    was_open: bool,
    /// The scroll bar of the last frame, for the drag.
    bar: std::cell::Cell<Bar>,
    /// A drag of the scroll bar's handle: how far the cursor was below the handle's top when it was pressed (canvas units).
    drag: Option<f32>,
}

impl InfoState {
    /// The last snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    /// One frame of the app: the panel opening refreshes at once, and while it is open the numbers
    /// refresh every [`REFRESH_SECONDS`] of real time.
    pub fn frame(&mut self, open: bool, world: &World, real_dt: f32) {
        if open && !self.was_open {
            self.opened();
        }
        self.was_open = open;
        if open {
            self.update(world, real_dt);
        }
    }

    /// The panel was opened: the next refresh is at once.
    pub fn opened(&mut self) {
        self.snapshot = None;
        self.since_refresh = 0.0;
        self.scroll = 0.0;
    }

    /// One frame of real time with the panel open: a refresh when a second is up (or none was made).
    pub fn update(&mut self, world: &World, real_dt: f32) {
        self.since_refresh += real_dt.max(0.0);
        if self.snapshot.is_none() || self.since_refresh >= REFRESH_SECONDS {
            self.since_refresh = 0.0;
            self.snapshot = Some(Snapshot::collect(
                world,
                self.energy_mode,
                &mut self.tag_order,
            ));
            self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        }
    }

    /// The `count` / `energy` button: the other section shows.
    pub fn toggle_mode(&mut self) {
        self.energy_mode = !self.energy_mode;
    }

    /// How far the tags list can scroll (0 when its rows fit).
    #[must_use]
    pub fn max_scroll(&self) -> f32 {
        self.snapshot
            .as_ref()
            .map_or(0.0, |snapshot| scroll_range(snapshot.tags.len()))
    }

    /// The wheel over the tags list: `ScrollRect.OnScroll`, clamped. Returns whether it was over the list
    /// (the camera does not zoom then).
    pub fn wheel(&mut self, cursor: Vec2, wheel: f32) -> bool {
        let [x, y, w, h] = self.list_rect.get();
        if w <= 0.0 || cursor.x < x || cursor.y < y || cursor.x >= x + w || cursor.y >= y + h {
            return false;
        }
        self.scroll = (self.scroll - wheel * SCROLL_SENSITIVITY).clamp(0.0, self.max_scroll());
        true
    }

    /// The scroll offset (canvas units).
    #[must_use]
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// A press on the scroll bar: on the handle it starts a drag, on the track it pages towards the click by the
    /// handle's length (`Scrollbar.OnPointerDown` / `ClickRepeat`, once).
    pub fn press_bar(&mut self, handle: bool, cursor: Vec2) {
        let bar = self.bar.get();
        if bar.s <= 0.0 {
            return;
        }
        let y = cursor.y / bar.s;
        if handle {
            self.drag = Some(y - bar.handle[1]);
        } else if bar.range > 0.0 {
            let page = bar.size * bar.range;
            let step = if y < bar.handle[1] { -page } else { page };
            self.scroll = (self.scroll + step).clamp(0.0, self.max_scroll());
        }
    }

    /// Whether the handle is being dragged.
    #[must_use]
    pub fn bar_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The held handle follows the cursor (`Scrollbar.OnDrag`), keeping where it was grabbed.
    pub fn drag_bar(&mut self, cursor: Vec2) {
        let Some(grab) = self.drag else {
            return;
        };
        let bar = self.bar.get();
        if bar.s <= 0.0 {
            return;
        }
        let travel = (1.0 - bar.size) * bar.area_h;
        if travel > 0.0 {
            let top = cursor.y / bar.s - grab;
            let fraction = ((top - (bar.area_top - HANDLE_OVERHANG)) / travel).clamp(0.0, 1.0);
            self.scroll = fraction * bar.range;
        }
    }

    /// The button was let go.
    pub fn release_bar(&mut self) {
        self.drag = None;
    }
}

/// Height of the tags list's viewport for `n` tag items: `tagSection.preferredHeight`.
fn list_height(n: usize) -> f32 {
    15.0 + 30.0 * n.min(MAX_TAG_ROWS) as f32
}

/// Height of the tags list's content: `Content` (padding 5) around the holder (padding 3 on top,
/// rows 30 high with 1 between).
fn content_height(n: usize) -> f32 {
    if n == 0 {
        return 2.0 * CONTENT_PAD + HOLDER_PAD;
    }
    2.0 * CONTENT_PAD + HOLDER_PAD + n as f32 * ROW_H + (n - 1) as f32
}

/// How far the tags list scrolls: the content that does not fit in the viewport.
#[must_use]
pub fn scroll_range(n: usize) -> f32 {
    let range = content_height(n) - list_height(n);
    // `vScrollingNeeded`: the content is taller than the viewport by more than 0.01.
    if range > 0.01 { range } else { 0.0 }
}

/// A rectangle `[x, y, w, h]`.
type Rect = [f32; 4];

/// How far the handle sticks out of the sliding area at each end (`Handle` is 20 units larger than its area).
const HANDLE_OVERHANG: f32 = 10.0;

/// The tags list's scroll bar in canvas units: its track, its handle and what a drag needs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Bar {
    track: Rect,
    handle: Rect,
    /// Top and height of the sliding area (the track less 10 at each end).
    area_top: f32,
    area_h: f32,
    /// The handle's share of the track (`Scrollbar.size`).
    size: f32,
    /// How far the list scrolls.
    range: f32,
    /// Canvas units to screen pixels (0 while no bar was drawn).
    s: f32,
}

/// The scroll bar of a list `list` scrolled by `scroll` with `content` units of rows: the `Background` track, 40 from
/// the list's right edge, and the handle, which spans the sliding area (the track less 10 at each end) and sticks out
/// 10 at both ends.
fn bar_of(list: Rect, scroll: f32, content: f32, s: f32) -> Bar {
    let track = [list[0] + list[2] - 40.0, list[1], SCROLLBAR_W, list[3]];
    let area_top = track[1] + 10.0;
    let area_h = track[3] - 20.0;
    let size = (list[3] / content).clamp(0.0, 1.0);
    let range = (content - list[3]).max(0.0);
    let value = if range > 0.0 {
        1.0 - scroll / range
    } else {
        1.0
    };
    // `value` is 1 at the top; the handle's fractions are measured from the bottom.
    let low = value * (1.0 - size);
    let high = low + size;
    let top = area_top + (1.0 - high) * area_h - HANDLE_OVERHANG;
    let bottom = area_top + (1.0 - low) * area_h + HANDLE_OVERHANG;
    Bar {
        track,
        handle: [track[0], top, SCROLLBAR_W, bottom - top],
        area_top,
        area_h,
        size,
        range,
        s,
    }
}

/// Where everything of the panel goes, in canvas units.
#[derive(Clone, Debug, PartialEq)]
struct Layout {
    /// The panel.
    panel: Rect,
    left: f32,
    right: f32,
    title_y: f32,
    header_y: f32,
    header_h: f32,
    button: Rect,
    section_y: f32,
    /// Width of the wedge rows' column.
    column_w: f32,
    column_y: f32,
    pie: Rect,
    species_title_y: f32,
    title_h: f32,
    species_rows_y: f32,
    tags_title_y: f32,
    list: Rect,
    scrollbar: bool,
    /// Width the rows of the species list get.
    species_row_w: f32,
}

/// The wedge rows of a section: label, colour (none for the total) and value text.
fn wedge_rows(snapshot: &Snapshot, energy: bool) -> Vec<(&'static str, Option<Rgba>, String)> {
    let count = FloatValueFormat {
        si: false,
        precision: 0,
        ..FloatValueFormat::default()
    };
    let energy_format = FloatValueFormat {
        units: "E",
        ..count.clone()
    };
    if energy {
        let total = snapshot.free_energy
            + snapshot.bibite_energy
            + snapshot.egg_energy
            + snapshot.meat_energy
            + snapshot.plant_energy;
        vec![
            ("Total", None, energy_format.format(total)),
            (
                "Free",
                Some(WEDGE_FREE),
                energy_format.format(snapshot.free_energy),
            ),
            (
                "Bibites",
                Some(WHITE),
                energy_format.format(snapshot.bibite_energy),
            ),
            (
                "Eggs",
                Some(WEDGE_EGG),
                energy_format.format(snapshot.egg_energy),
            ),
            (
                "Plants",
                Some(WEDGE_PLANT),
                energy_format.format(snapshot.plant_energy),
            ),
            (
                "Meats",
                Some(WEDGE_MEAT),
                energy_format.format(snapshot.meat_energy),
            ),
        ]
    } else {
        let total = snapshot.bibites + snapshot.eggs + snapshot.plants + snapshot.meats;
        vec![
            ("Total", None, count.format(total as f32)),
            (
                "Bibites",
                Some(WHITE),
                count.format(snapshot.bibites as f32),
            ),
            ("Eggs", Some(WEDGE_EGG), count.format(snapshot.eggs as f32)),
            (
                "Plants",
                Some(WEDGE_PLANT),
                count.format(snapshot.plants as f32),
            ),
            (
                "Meats",
                Some(WEDGE_MEAT),
                count.format(snapshot.meats as f32),
            ),
        ]
    }
}

/// The colours of a pie chart's wedges, in `WedgeInfo` order.
fn pie_colors(energy: bool) -> Vec<Rgba> {
    if energy {
        vec![WEDGE_FREE, WHITE, WEDGE_EGG, WEDGE_PLANT, WEDGE_MEAT]
    } else {
        vec![WHITE, WEDGE_EGG, WEDGE_PLANT, WEDGE_MEAT]
    }
}

/// The text a row shows for a count and for an energy.
fn row_values(count: u32, energy: f32) -> (String, String) {
    let count_format = FloatValueFormat {
        si: false,
        precision: 0,
        ..FloatValueFormat::default()
    };
    let energy_format = FloatValueFormat {
        units: "E",
        ..count_format.clone()
    };
    (
        count_format.format(count as f32),
        energy_format.format(energy),
    )
}

/// Preferred width of a species or tag row (`HorizontalLayoutGroup`: padding 5, spacing 5, the index, the
/// name box, the empty spacer, "count:", the count's box, "totalEnergy:" and the energy).
fn row_preferred_width(measure: &Measure<'_>, index: usize, name_w: f32, energy: &str) -> f32 {
    2.0 * ROW_PAD
        + measure.width(ROBOTO, &format!("{}.", index + 1), 14.0)
        + name_w
        + measure.width(ROBOTO, "count:", 14.0)
        + ROW_COUNT_W
        + measure.width(ROBOTO, "totalEnergy:", 14.0)
        + measure.width(ROBOTO_BOLD, energy, 14.0)
        + 6.0 * ROW_GAP
}

impl Layout {
    /// The layout of the panel for `snapshot` in the section shown, `canvas_w` canvas units wide.
    fn new(snapshot: &Snapshot, energy_mode: bool, measure: &Measure<'_>, canvas_w: f32) -> Self {
        // Heights of one line of each text.
        let title_line = measure.height(ROBOTO_BOLD, 26.0);
        let header_h = measure.height(ROBOTO_BOLD, 20.0);
        let bar_h = measure
            .height(LIBERATION, 16.0)
            .max(measure.height(ROBOTO_BOLD, 14.0));

        // Preferred widths of what can set the panel's width.
        let title_w = measure.width(ROBOTO_BOLD, "Information and Statistics", 26.0);
        let button_w = if energy_mode {
            ENERGY_BUTTON_W
        } else {
            COUNT_BUTTON_W
        };
        let header_w = measure.width(ROBOTO_BOLD, "Entities:", 20.0) + SECTION_SPACING + button_w;
        let rows = wedge_rows(snapshot, energy_mode);
        let column_pref = rows
            .iter()
            .map(|(label, color, value)| {
                let indicator = if color.is_some() {
                    INDICATOR + WEDGE_SPACING
                } else {
                    0.0
                };
                indicator
                    + measure.width(ROBOTO_BOLD, label, 20.0)
                    + WEDGE_SPACING
                    + measure.width(ROBOTO_BOLD, value, 20.0).max(VALUE_MIN_W)
            })
            .fold(0.0_f32, f32::max);
        let section_w = column_pref + SECTION_GAP + PIE;
        let species_title_w = {
            let top = snapshot.species.len().to_string();
            let all = snapshot.n_species.to_string();
            [
                measure.width(LIBERATION, "Top Species ", 16.0),
                measure.width(LIBERATION, "(", 16.0),
                measure.width(ROBOTO_BOLD, &top, 14.0),
                measure.width(LIBERATION, "of", 16.0),
                measure.width(ROBOTO_BOLD, &all, 14.0),
                measure.width(LIBERATION, "active species) :", 16.0),
            ]
            .iter()
            .sum::<f32>()
                + 5.0 * WEDGE_SPACING
        };
        let species_w = snapshot
            .species
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let (_, energy) = row_values(row.count, row.energy);
                row_preferred_width(measure, index, SPECIES_NAME_W, &energy)
            })
            .fold(0.0_f32, f32::max)
            + 2.0 * (CONTENT_PAD + HOLDER_PAD);
        let tags_title_w = measure.width(LIBERATION, "Tags:", 16.0);
        let inner_w = title_w
            .max(header_w)
            .max(section_w)
            .max(species_title_w)
            .max(species_w)
            .max(tags_title_w);
        let width = inner_w + PAD_LEFT + PAD_RIGHT;

        // Heights.
        let section_h =
            PIE.max(rows.len() as f32 * header_h + (rows.len() - 1) as f32 * WEDGE_SPACING);
        let entities_h = header_h + SECTION_SPACING + section_h;
        let n_species = snapshot.species.len();
        let species_content = 2.0 * CONTENT_PAD
            + HOLDER_PAD
            + if n_species == 0 {
                0.0
            } else {
                n_species as f32 * ROW_H + (n_species - 1) as f32
            };
        let species_h = bar_h + SECTION_SPACING + species_content;
        let n_tags = snapshot.tags.len();
        let list_h = list_height(n_tags);
        let tags_h = bar_h + SECTION_SPACING + list_h;
        let height = PAD_TOP
            + title_line
            + SPACING
            + entities_h
            + SPACING
            + species_h
            + SPACING
            + tags_h
            + PAD_BOTTOM;

        let x = canvas_w - RIGHT_MARGIN - width;
        let left = x + PAD_LEFT;
        let right = x + width - PAD_RIGHT;
        let title_y = TOP + PAD_TOP;
        let header_y = title_y + title_line + SPACING;
        let section_y = header_y + header_h + SECTION_SPACING;
        let species_title_y = header_y + entities_h + SPACING;
        let species_rows_y = species_title_y + bar_h + SECTION_SPACING + CONTENT_PAD + HOLDER_PAD;
        let tags_title_y = species_title_y + species_h + SPACING;
        let list_y = tags_title_y + bar_h + SECTION_SPACING;
        let column_h = rows.len() as f32 * header_h + (rows.len() - 1) as f32 * WEDGE_SPACING;
        let inner = right - left;
        Self {
            panel: [x, TOP, width, height],
            left,
            right,
            title_y,
            header_y,
            header_h,
            button: [right - button_w, header_y, button_w, header_h],
            section_y,
            column_w: inner - SECTION_GAP - PIE,
            column_y: section_y + (section_h - column_h) / 2.0,
            pie: [right - PIE, section_y, PIE, PIE],
            species_title_y,
            title_h: bar_h,
            species_rows_y,
            tags_title_y,
            list: [left, list_y, inner, list_h],
            scrollbar: scroll_range(n_tags) > 0.0,
            species_row_w: inner - 2.0 * (CONTENT_PAD + HOLDER_PAD),
        }
    }
}

/// Draws the panel, the pie chart and the lists, and registers their hit areas.
pub fn draw(
    frame: &mut UiFrame,
    app: &App,
    ui: &UiAssets,
    metrics: Option<&SdfFonts>,
    width: f32,
    height: f32,
) {
    let Some(snapshot) = app.information.snapshot() else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let measure = Measure { fonts: metrics };
    let energy = app.information.energy_mode;
    let layout = Layout::new(snapshot, energy, &measure, width / s);
    let mut painter = Painter {
        frame,
        ui,
        s,
        measure,
    };
    painter.panel(&layout);
    painter.header(&layout, energy);
    painter.entities(&layout, snapshot, energy);
    painter.species(&layout, snapshot);
    let bar = painter.tags(&layout, snapshot, app.information.scroll());
    app.information.bar.set(bar.unwrap_or_default());
    app.information.list_rect.set([
        layout.list[0] * s,
        layout.list[1] * s,
        layout.list[2] * s,
        layout.list[3] * s,
    ]);
}

/// Draws into a frame in canvas units scaled to the screen.
struct Painter<'a> {
    frame: &'a mut UiFrame,
    ui: &'a UiAssets,
    s: f32,
    measure: Measure<'a>,
}

impl Painter<'_> {
    fn at(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(x * self.s, y * self.s)
    }

    fn dim(&self, w: f32, h: f32) -> Vec2 {
        Vec2::new(w * self.s, h * self.s)
    }

    /// A text with its top left at (`x`, `y`).
    fn text(&mut self, face: Face, text: &str, x: f32, y: f32, px: f32, color: Rgba) {
        if text.is_empty() {
            return;
        }
        self.frame.draws.push(UiDraw::Text {
            font: Some(face.name.to_owned()),
            text: text.to_owned(),
            pos: self.at(x, y),
            px: px * self.s,
            color,
            role: face.role,
        });
    }

    /// A text right-aligned to `right`.
    fn text_right(&mut self, face: Face, text: &str, right: f32, y: f32, px: f32, color: Rgba) {
        let w = self.measure.width(face, text, px);
        self.text(face, text, right - w, y, px, color);
    }

    /// A text centred on `centre`.
    fn text_centre(&mut self, face: Face, text: &str, centre: f32, y: f32, px: f32, color: Rgba) {
        let w = self.measure.width(face, text, px);
        self.text(face, text, centre - w / 2.0, y, px, color);
    }

    /// A 9-sliced sprite; `dest_scale` is the border's size over the sprite's (`1 / pixelsPerUnitMultiplier`).
    fn slice(&mut self, sprite: &str, border: f32, dest_scale: f32, rect: Rect, tint: Rgba) {
        let (pos, size) = (self.at(rect[0], rect[1]), self.dim(rect[2], rect[3]));
        match self.ui.texture_of_alias(sprite) {
            Some(texture) => self.frame.draws.push(UiDraw::NineSlice {
                texture,
                border: [border; 4],
                dest_scale,
                pos,
                size,
                tint,
            }),
            None => self.frame.rect(pos, size, tint),
        }
    }

    /// A sprite stretched over `rect`.
    fn icon(&mut self, sprite: &str, rect: Rect) {
        if let Some(texture) = self.ui.texture_of_alias(sprite) {
            let (pos, size) = (self.at(rect[0], rect[1]), self.dim(rect[2], rect[3]));
            self.frame.draws.push(UiDraw::Image {
                texture,
                pos,
                size,
                tint: WHITE,
            });
        }
    }

    fn hit(&mut self, id: u32, rect: Rect, action: UiAction) {
        let (pos, size) = (self.at(rect[0], rect[1]), self.dim(rect[2], rect[3]));
        self.frame.hit(id, pos, size, action);
    }

    /// The frame (`base_icon`, 0.949), the title and the close button.
    fn panel(&mut self, layout: &Layout) {
        // Border 5 at multiplier 1.
        self.slice("base_icon", 5.0, 1.0, layout.panel, [255, 255, 255, 242]);
        // The panel swallows the clicks that land on it (its `Image` is a raycast target).
        self.hit(HIT_BODY, layout.panel, UiAction::Consume);
        let title = "Information and Statistics";
        let centre = (layout.left + layout.right) / 2.0;
        self.text_centre(ROBOTO_BOLD, title, centre, layout.title_y, 26.0, BLACK);
        // `CloseSettings`: 30 x 30, 10 units from the corner.
        let close = [
            layout.panel[0] + layout.panel[2] - 10.0 - 30.0,
            layout.panel[1] + 10.0,
            30.0,
            30.0,
        ];
        self.icon("1727490", close);
        self.hit(
            HIT_CLOSE,
            close,
            UiAction::Invoke(ControlCommand::InformationClose),
        );
    }

    /// "Entities:" and the button of the section shown (`count` while the counts show).
    fn header(&mut self, layout: &Layout, energy: bool) {
        self.text(
            ROBOTO_BOLD_STYLE,
            "Entities:",
            layout.left,
            layout.header_y,
            20.0,
            BLACK,
        );
        self.slice("base_icon", 5.0, 1.0, layout.button, WHITE);
        let label = if energy { "energy" } else { "count" };
        let line = self.measure.height(LIBERATION_BOLD, 18.0);
        self.text_centre(
            LIBERATION_BOLD,
            label,
            layout.button[0] + layout.button[2] / 2.0,
            layout.button[1] + (layout.button[3] - line) / 2.0,
            18.0,
            BLACK,
        );
        self.hit(
            HIT_MODE,
            layout.button,
            UiAction::Invoke(ControlCommand::InformationToggleMode),
        );
    }

    /// The section shown: the wedge rows and the pie chart.
    fn entities(&mut self, layout: &Layout, snapshot: &Snapshot, energy: bool) {
        let rows = wedge_rows(snapshot, energy);
        let line = layout.header_h;
        let column_right = layout.left + layout.column_w;
        for (index, (label, color, value)) in rows.iter().enumerate() {
            let top = layout.column_y + index as f32 * (line + WEDGE_SPACING);
            let mut x = layout.left;
            if let Some(color) = color {
                // `color indicator`: 20 x 20, `base_icon` at multiplier 1.25, centred in the row.
                self.slice(
                    "base_icon",
                    5.0,
                    0.8,
                    [x, top + (line - INDICATOR) / 2.0, INDICATOR, INDICATOR],
                    *color,
                );
                x += INDICATOR + WEDGE_SPACING;
            }
            self.text(ROBOTO_BOLD, label, x, top, 20.0, BLACK);
            self.text_right(ROBOTO_BOLD, value, column_right, top, 20.0, BLACK);
        }
        self.pie(layout, snapshot, energy);
    }

    /// The pie chart: the dark `Circle` and, over it, one radial fill per wedge.
    fn pie(&mut self, layout: &Layout, snapshot: &Snapshot, energy: bool) {
        let values = snapshot.pie_values(energy);
        let colors = pie_colors(energy);
        let fills = pie_fills(&values);
        let diameter = (PIE * self.s).round().max(1.0) as u32;
        let texture = pie_texture(&fills, &colors, diameter, self.s);
        let pos = self.at(layout.pie[0], layout.pie[1]);
        self.frame.draws.push(UiDraw::Pixels {
            texture,
            pos: Vec2::new(pos.x.round(), pos.y.round()),
            size: Vec2::new(diameter as f32, diameter as f32),
        });
    }

    /// A section's title bar: the translucent `Background` and the white texts on it.
    fn title_bar(&mut self, layout: &Layout, y: f32) {
        self.slice(
            "Background",
            10.0,
            1.0,
            [layout.left, y, layout.right - layout.left, layout.title_h],
            [255, 255, 255, 100],
        );
    }

    /// "Top Species (N of M active species) :" and the rows of the species.
    fn species(&mut self, layout: &Layout, snapshot: &Snapshot) {
        let y = layout.species_title_y;
        self.title_bar(layout, y);
        let bar = layout.title_h;
        let line14 = self.measure.height(ROBOTO_BOLD, 14.0);
        let top = snapshot.species.len().to_string();
        let all = snapshot.n_species.to_string();
        let mut x = layout.left;
        for (face, text, px, color) in [
            (LIBERATION_BOLD, "Top Species ", 16.0, BLACK),
            (LIBERATION_BOLD, "(", 16.0, BLACK),
            (ROBOTO_BOLD, top.as_str(), 14.0, BLACK),
            (LIBERATION_BOLD, "of", 16.0, BLACK),
            (ROBOTO_BOLD, all.as_str(), 14.0, BLACK),
            (LIBERATION_BOLD, "active species) :", 16.0, BLACK),
        ] {
            let w = self.measure.width(face, text, px);
            // The numbers are stretched to the bar's height and centred in it.
            let offset = if px < 16.0 { (bar - line14) / 2.0 } else { 0.0 };
            self.text(face, text, x, y + offset, px, color);
            x += w + WEDGE_SPACING;
        }
        for (index, row) in snapshot.species.iter().enumerate() {
            let rect = [
                layout.left + CONTENT_PAD + HOLDER_PAD,
                layout.species_rows_y + index as f32 * ROW_PITCH,
                layout.species_row_w,
                ROW_H,
            ];
            let (count, energy) = row_values(row.count, row.energy);
            self.row(
                rect,
                (index, &row.name, &count, &energy),
                SPECIES_NAME_W,
                None,
            );
            self.hit(
                HIT_SPECIES + index as u32,
                rect,
                UiAction::Invoke(ControlCommand::InformationSelectSpecies(index as u8)),
            );
        }
    }

    /// One species or tag row (`SpeciesInfoHandle` / `TagElementHandle`): a `base_icon` card at multiplier 0.9
    /// with the index, the name, "count:" and its number, "totalEnergy:" and its number. A `clip` cuts the
    /// card to the list's viewport and leaves the texts of a card it cuts out.
    fn row(
        &mut self,
        rect: Rect,
        (index, name, count, energy): (usize, &str, &str, &str),
        name_w: f32,
        clip: Option<Rect>,
    ) {
        let border = 5.0 / 0.9;
        match clip {
            None => self.slice("base_icon", 5.0, 1.0 / 0.9, rect, WHITE),
            Some(clip) => {
                if let Some(texture) = self.ui.texture_of_alias("base_icon") {
                    let s = self.s;
                    clip_nine_slice(
                        self.frame,
                        self.ui,
                        texture,
                        5.0,
                        border * s,
                        [rect[0] * s, rect[1] * s, rect[2] * s, rect[3] * s],
                        WHITE,
                        [clip[0] * s, clip[1] * s, clip[2] * s, clip[3] * s],
                    );
                } else if let Some(cut) = intersect(rect, clip) {
                    let (pos, size) = (self.at(cut[0], cut[1]), self.dim(cut[2], cut[3]));
                    self.frame.rect(pos, size, WHITE);
                }
                // The texts only show whole.
                let whole =
                    rect[1] >= clip[1] - 0.01 && rect[1] + rect[3] <= clip[1] + clip[3] + 0.01;
                if !whole {
                    return;
                }
            }
        }
        let line14 = self.measure.height(ROBOTO, 14.0);
        let y14 = rect[1] + ROW_PAD + (ROW_H - 2.0 * ROW_PAD - line14) / 2.0;
        let right = rect[0] + rect[2] - ROW_PAD;
        let mut x = rect[0] + ROW_PAD;
        let index_text = format!("{}.", index + 1);
        self.text(ROBOTO, &index_text, x, y14, 14.0, ROW_INK);
        x += self.measure.width(ROBOTO, &index_text, 14.0) + ROW_GAP;
        // The name's box is `name_w` wide; a longer name is cut to it.
        let shown = self.fit(ROBOTO, name, name_w, 16.0);
        self.text(ROBOTO, &shown, x, rect[1] + ROW_PAD, 16.0, ROW_INK);
        // From the right: the energy, its label, the count's box, then the flexible "count:" label.
        let energy_w = self.measure.width(ROBOTO_BOLD, energy, 14.0);
        self.text(ROBOTO_BOLD, energy, right - energy_w, y14, 14.0, ROW_INK);
        let label_w = self.measure.width(ROBOTO, "totalEnergy:", 14.0);
        let label_x = right - energy_w - ROW_GAP - label_w;
        self.text(ROBOTO, "totalEnergy:", label_x, y14, 14.0, ROW_INK);
        let count_x = label_x - ROW_GAP - ROW_COUNT_W;
        self.text(ROBOTO_BOLD, count, count_x, y14, 14.0, ROW_INK);
        self.text_right(ROBOTO, "count:", count_x - ROW_GAP, y14, 14.0, ROW_INK);
    }

    /// `text` cut to what fits in `width` set at `px`.
    fn fit(&self, face: Face, text: &str, width: f32, px: f32) -> String {
        let mut shown = String::new();
        for character in text.chars() {
            shown.push(character);
            if self.measure.width(face, &shown, px) > width {
                shown.pop();
                break;
            }
        }
        shown
    }

    /// "Tags:" and the scrolling list of the tags; the scroll bar when there is one.
    fn tags(&mut self, layout: &Layout, snapshot: &Snapshot, scroll: f32) -> Option<Bar> {
        self.title_bar(layout, layout.tags_title_y);
        self.text(
            LIBERATION_BOLD,
            "Tags:",
            layout.left,
            layout.tags_title_y,
            16.0,
            BLACK,
        );
        let list = layout.list;
        // `tagSection`: the `Long_UI` frame (border 70 at 500 pixels per unit).
        self.slice("Long_UI", 70.0, LIST_FRAME_SCALE, list, WHITE);
        let viewport_w = if layout.scrollbar {
            list[2] - SCROLLBAR_SPACE
        } else {
            list[2]
        };
        let viewport = [list[0], list[1], viewport_w, list[3]];
        let first_y = list[1] + CONTENT_PAD + HOLDER_PAD - scroll;
        let row_w = viewport_w - 2.0 * (CONTENT_PAD + HOLDER_PAD);
        for (index, tag) in snapshot.tags.iter().enumerate() {
            let rect = [
                list[0] + CONTENT_PAD + HOLDER_PAD,
                first_y + index as f32 * ROW_PITCH,
                row_w,
                ROW_H,
            ];
            let Some(visible) = intersect(rect, viewport) else {
                continue;
            };
            let (count, energy) = row_values(tag.count, tag.energy);
            self.row(
                rect,
                (index, &tag.name, &count, &energy),
                TAG_NAME_W,
                Some(viewport),
            );
            self.hit(
                HIT_TAGS + index as u32,
                visible,
                UiAction::Invoke(ControlCommand::InformationSelectTag(index as u32)),
            );
        }
        if layout.scrollbar {
            let bar = bar_of(list, scroll, content_height(snapshot.tags.len()), self.s);
            self.scrollbar(&bar);
            return Some(bar);
        }
        None
    }

    /// The list's scroll bar: the `Background` track at 0.255 and the `UISprite` handle (see [`bar_of`]), with their
    /// hit areas (the handle's on top).
    fn scrollbar(&mut self, bar: &Bar) {
        self.slice("Background", 10.0, 1.0, bar.track, [255, 255, 255, 65]);
        self.hit(HIT_BAR_TRACK, bar.track, UiAction::InformationScroll(false));
        self.slice("UISprite", 10.0, 1.0, bar.handle, WHITE);
        self.hit(
            HIT_BAR_HANDLE,
            bar.handle,
            UiAction::InformationScroll(true),
        );
    }
}

/// `PieChartHandle.UpdateValues`: wedge `j` fills `1 - (the share of the wedges before it)`; the first
/// one is full. With nothing to share only the first shows (the others divide by zero).
fn pie_fills(values: &[f32]) -> Vec<f32> {
    let total: f32 = values.iter().sum();
    let mut fill = 1.0_f32;
    let mut out = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        out.push(fill);
        if total > 0.0 {
            fill -= value / total;
        } else if index == 0 {
            fill = 0.0;
        }
    }
    out
}

/// Cache of the last pictures of the pie charts (the numbers change once a second).
type PieEntry = ((Vec<(u32, Rgba)>, u32), Arc<UiTexture>);
static PIES: Mutex<Vec<PieEntry>> = Mutex::new(Vec::new());

/// The pie chart's picture, `diameter` pixels wide: the dark disc and the wedges, each a clockwise fill
/// from the top (`Image.Type.Filled`, `Radial360`, origin Top) of a disc 3 units smaller, painted in order.
fn pie_texture(fills: &[f32], colors: &[Rgba], diameter: u32, s: f32) -> Arc<UiTexture> {
    let key: (Vec<(u32, Rgba)>, u32) = (
        fills
            .iter()
            .zip(colors)
            .map(|(fill, color)| (fill.to_bits(), *color))
            .collect(),
        diameter,
    );
    if let Ok(cache) = PIES.lock()
        && let Some((_, texture)) = cache.iter().find(|(cached, _)| *cached == key)
    {
        return texture.clone();
    }
    let texture = Arc::new(render_pie(fills, colors, diameter, s));
    if let Ok(mut cache) = PIES.lock() {
        cache.retain(|(cached, _)| cached.1 == diameter);
        cache.push((key, texture.clone()));
        let excess = cache.len().saturating_sub(4);
        cache.drain(..excess);
    }
    texture
}

/// The pixels of a pie chart.
fn render_pie(fills: &[f32], colors: &[Rgba], diameter: u32, s: f32) -> UiTexture {
    let n = diameter as usize;
    let centre = diameter as f32 / 2.0;
    let back_radius = PIE * s / 2.0;
    let wedge_radius = (PIE - 2.0 * PIE_INSET) * s / 2.0;
    let mut rgba = vec![0_u8; n * n * 4];
    for py in 0..n {
        for px in 0..n {
            let dx = px as f32 + 0.5 - centre;
            let dy = py as f32 + 0.5 - centre;
            let radius = (dx * dx + dy * dy).sqrt();
            let back = (back_radius - radius + 0.5).clamp(0.0, 1.0);
            if back <= 0.0 {
                continue;
            }
            // Clockwise from the top, as a share of the turn.
            let mut turn = dx.atan2(-dy) / std::f32::consts::TAU;
            if turn < 0.0 {
                turn += 1.0;
            }
            let mut pixel = [
                f32::from(PIE_BACK[0]),
                f32::from(PIE_BACK[1]),
                f32::from(PIE_BACK[2]),
            ];
            let wedge = (wedge_radius - radius + 0.5).clamp(0.0, 1.0);
            if wedge > 0.0 {
                for (fill, color) in fills.iter().zip(colors) {
                    // A fill outside [0, 1] or not a number draws nothing (`Clamp01` of NaN is NaN).
                    if turn < *fill {
                        for channel in 0..3 {
                            pixel[channel] += (f32::from(color[channel]) - pixel[channel]) * wedge;
                        }
                    }
                }
            }
            let at = (py * n + px) * 4;
            rgba[at] = pixel[0].round() as u8;
            rgba[at + 1] = pixel[1].round() as u8;
            rgba[at + 2] = pixel[2].round() as u8;
            rgba[at + 3] = (back * 255.0).round() as u8;
        }
    }
    UiTexture {
        width: diameter,
        height: diameter,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LoadedWorld;

    fn app() -> App {
        let loaded = LoadedWorld::load(None, 41, 10, 1).expect("starter world");
        let mut app = App::new(loaded, Vec2::new(1920.0, 1080.0), None);
        // The starter world has no species yet: record them.
        recorded_species(&mut app);
        app.panels.open(crate::ui::PanelKind::Information);
        app.information.opened();
        app.information.update(&app.world, 0.0);
        app
    }

    /// Three species with 6, 3 and 1 of the ten bibites as members, counted.
    fn recorded_species(app: &mut App) {
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        let species: Vec<_> = ["Zzalpha zzone", "Zzbeta zztwo", "Zzgamma zzthree"]
            .iter()
            .map(|name| {
                app.world
                    .species
                    .add_template_species(name, "", &genome, &brain)
            })
            .collect();
        for (index, id) in ids.iter().enumerate() {
            let group = match index {
                0..=5 => 0,
                6..=8 => 1,
                _ => 2,
            };
            app.world.bibites.get_mut(*id).expect("record").meta.species = Some(species[group]);
        }
        app.world.reconcile_species_counts();
    }

    fn frame_of(app: &App) -> UiFrame {
        let mut frame = UiFrame::new(1.0);
        draw(&mut frame, app, &UiAssets::empty(), None, 1920.0, 1080.0);
        frame
    }

    fn texts(frame: &UiFrame) -> Vec<String> {
        frame
            .draws
            .iter()
            .filter_map(|draw| match draw {
                UiDraw::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_snapshot_counts_what_the_world_holds() {
        let app = app();
        let snapshot = app.information.snapshot().expect("a snapshot");
        let live = app
            .world
            .bibites
            .iter()
            .filter(|(_, record)| !record.body.dead)
            .count() as u32;
        assert_eq!(snapshot.bibites, live);
        assert_eq!(snapshot.eggs, app.world.eggs.len() as u32);
        let pellets = app.world.pellets.counts();
        assert_eq!(snapshot.plants, pellets.plant_count);
        assert_eq!(snapshot.meats, pellets.meat_count);
        assert!((snapshot.free_energy - app.world.free_biomass()).abs() < 1e-3);
        assert!(snapshot.bibite_energy > 0.0);
    }

    #[test]
    fn corpses_are_not_counted_and_their_energy_is_not_either() {
        let mut app = app();
        let before = app.information.snapshot().expect("snapshot").clone();
        let id = app
            .world
            .bibites
            .iter()
            .next()
            .map(|(id, _)| id)
            .expect("a bibite");
        app.world.bibites.get_mut(id).expect("record").body.dead = true;
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let after = app.information.snapshot().expect("snapshot");
        assert_eq!(after.bibites, before.bibites - 1);
        assert!(after.bibite_energy < before.bibite_energy);
    }

    #[test]
    fn the_numbers_refresh_once_a_second_of_real_time() {
        let mut app = app();
        let first = app.information.snapshot().expect("snapshot").clone();
        let id = app
            .world
            .bibites
            .iter()
            .next()
            .map(|(id, _)| id)
            .expect("a bibite");
        app.world.bibites.get_mut(id).expect("record").body.dead = true;
        app.information.update(&app.world, 0.4);
        assert_eq!(app.information.snapshot(), Some(&first), "not yet");
        app.information.update(&app.world, 0.4);
        assert_eq!(app.information.snapshot(), Some(&first), "0.8 s");
        app.information.update(&app.world, 0.3);
        assert_eq!(
            app.information.snapshot().map(|s| s.bibites),
            Some(first.bibites - 1),
            "after a second"
        );
    }

    #[test]
    fn tags_group_bibites_and_eggs_and_untagged_ones_come_first() {
        let mut app = app();
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        for id in ids.iter().take(3) {
            app.world
                .bibites
                .get_mut(*id)
                .expect("record")
                .meta
                .species_tag = Some("alpha".to_owned());
        }
        app.world
            .bibites
            .get_mut(ids[3])
            .expect("record")
            .meta
            .species_tag = Some("beta".to_owned());
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let snapshot = app.information.snapshot().expect("snapshot");
        let by_name = |name: &str| snapshot.tags.iter().find(|tag| tag.name == name).cloned();
        assert_eq!(by_name("alpha").expect("alpha").count, 3);
        assert_eq!(by_name("beta").expect("beta").count, 1);
        let untagged = by_name(UNTAGGED).expect("untagged");
        assert_eq!(untagged.count, snapshot.bibites + snapshot.eggs - 4);
        // Sorted by count, largest first: the untagged ones are the most numerous here.
        let counts: Vec<u32> = snapshot.tags.iter().map(|tag| tag.count).collect();
        assert!(
            counts.windows(2).all(|pair| pair[0] >= pair[1]),
            "{counts:?}"
        );
        // Empty tags count as untagged.
        app.world
            .bibites
            .get_mut(ids[3])
            .expect("record")
            .meta
            .species_tag = Some(String::new());
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let snapshot = app.information.snapshot().expect("snapshot");
        assert!(snapshot.tags.iter().all(|tag| tag.name != "beta"));
    }

    #[test]
    fn tags_sort_by_energy_in_the_energy_section() {
        let mut app = app();
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        // One bibite with a tag of its own; make it the richest.
        {
            let record = app.world.bibites.get_mut(ids[0]).expect("record");
            record.meta.species_tag = Some("rich".to_owned());
            record.body.energy.amount += 1.0e6;
        }
        app.information.energy_mode = true;
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let snapshot = app.information.snapshot().expect("snapshot");
        assert_eq!(snapshot.tags[0].name, "rich", "{:?}", snapshot.tags);
        // By count the same tag is last.
        app.information.energy_mode = false;
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let snapshot = app.information.snapshot().expect("snapshot");
        assert_eq!(snapshot.tags[0].name, UNTAGGED);
    }

    #[test]
    fn the_top_species_are_the_five_largest_and_stop_at_an_empty_one() {
        let mut app = app();
        let snapshot = app.information.snapshot().expect("snapshot");
        let counts: Vec<u32> = snapshot.species.iter().map(|row| row.count).collect();
        assert_eq!(counts, vec![6, 3, 1]);
        assert_eq!(snapshot.n_species, 3);
        // A species made from a template has no members: it is counted nowhere and never shown.
        let (genome, brain) = {
            let (_, record) = app.world.bibites.iter().next().expect("a bibite");
            (record.genome.clone(), record.brain.clone())
        };
        app.world
            .species
            .add_template_species("Zzempty zzspecies", "", &genome, &brain);
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let snapshot = app.information.snapshot().expect("snapshot");
        assert_eq!(snapshot.species.len(), 3);
        assert_eq!(
            snapshot.n_species, 3,
            "`NSpecies` counts the species with a member"
        );
    }

    #[test]
    fn a_species_count_and_energy_are_its_members_own() {
        let mut app = app();
        // An egg of the first species counts as a member while it is not aborting.
        let row = app.information.snapshot().expect("snapshot").species[0].clone();
        let parent = app
            .world
            .bibites
            .iter()
            .find(|(_, record)| record.meta.species == Some(row.id))
            .map(|(id, _)| id)
            .expect("a member");
        assert!(app.world.lay_egg_once(parent) >= 1);
        let egg = app.world.eggs.ids().next().expect("egg");
        app.world.eggs.get_mut(egg).expect("egg").meta.species = Some(row.id);
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let with_egg = app.information.snapshot().expect("snapshot").species[0].clone();
        assert_eq!(with_egg.id, row.id);
        assert_eq!(with_egg.count, row.count + 1);
        let egg_energy = app.world.eggs.get(egg).expect("egg").hatch.energy;
        assert!((with_egg.energy - row.energy - egg_energy).abs() < 1e-2 * (1.0 + egg_energy));
        // The members' energies are what the species' energy is made of.
        let fat = app.world.settings.materials.fat.energy_density;
        let members: f32 = app
            .world
            .bibites
            .iter()
            .filter(|(_, record)| record.meta.species == Some(row.id) && !record.body.dead)
            .map(|(_, record)| {
                record.body.total_energy(
                    record.egg_layer.progress * record.egg_layer.egg_energy
                        + record.fat.amount * fat,
                )
            })
            .sum();
        assert!(
            (row.energy - members).abs() < 1e-2 * (1.0 + members),
            "{} vs {members}",
            row.energy
        );
        // An aborting egg leaves its species.
        app.world.eggs.get_mut(egg).expect("egg").hatch.aborting = true;
        app.information.opened();
        app.information.update(&app.world, 0.0);
        assert_eq!(
            app.information.snapshot().expect("snapshot").species[0].count,
            row.count
        );
    }

    #[test]
    fn the_species_sort_by_energy_in_the_energy_section() {
        let mut app = app();
        // Make the smallest species the richest: its one member holds a lot of energy.
        let lone = app
            .world
            .bibites
            .iter()
            .find(|(_, record)| {
                record.meta.species
                    == app
                        .information
                        .snapshot()
                        .expect("snapshot")
                        .species
                        .last()
                        .map(|row| row.id)
            })
            .map(|(id, _)| id)
            .expect("the lone bibite");
        let species = app
            .world
            .bibites
            .get(lone)
            .expect("record")
            .meta
            .species
            .expect("species");
        app.world
            .bibites
            .get_mut(lone)
            .expect("record")
            .body
            .energy
            .amount += 1.0e6;
        app.information.energy_mode = true;
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let rows = &app.information.snapshot().expect("snapshot").species;
        assert_eq!(rows[0].id, species, "{rows:?}");
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn the_pie_wedges_fill_from_one_downwards() {
        assert_eq!(pie_fills(&[1.0, 1.0, 2.0]), vec![1.0, 0.75, 0.5]);
        assert_eq!(pie_fills(&[0.0, 5.0]), vec![1.0, 1.0]);
        // Nothing to share: only the first wedge is full.
        assert_eq!(pie_fills(&[0.0, 0.0, 0.0]), vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn the_pie_is_painted_clockwise_from_the_top() {
        // Two wedges: the first (red) is full, the second (blue) fills the first quarter. Clockwise
        // from the top, that is the upper-right quadrant; the rest shows the first wedge.
        let texture = render_pie(
            &[1.0, 0.25],
            &[[255, 0, 0, 255], [0, 0, 255, 255]],
            200,
            1.0,
        );
        let at = |x: usize, y: usize| {
            let i = (y * 200 + x) * 4;
            [
                texture.rgba[i],
                texture.rgba[i + 1],
                texture.rgba[i + 2],
                texture.rgba[i + 3],
            ]
        };
        assert_eq!(at(150, 50), [0, 0, 255, 255], "upper right");
        assert_eq!(at(150, 150), [255, 0, 0, 255], "lower right");
        assert_eq!(at(50, 150), [255, 0, 0, 255], "lower left");
        assert_eq!(at(50, 50), [255, 0, 0, 255], "upper left");
        // Outside the disc: transparent; the rim between the wedges and the disc is the dark back.
        assert_eq!(at(0, 0)[3], 0);
        assert_eq!(
            at(100, 1)[..3],
            [38, 38, 38],
            "between the wedge and the rim"
        );
    }

    #[test]
    fn the_panel_hangs_from_the_top_right_corner_and_follows_its_rows() {
        let app = app();
        let snapshot = app.information.snapshot().expect("snapshot");
        let measure = Measure { fonts: None };
        let layout = Layout::new(snapshot, false, &measure, 1920.0);
        // Its right edge is 10 in from the canvas', its top 80 down.
        assert!((layout.panel[0] + layout.panel[2] - 1910.0).abs() < 1e-3);
        assert!((layout.panel[1] - 80.0).abs() < 1e-3);
        // The height is the sum of the sections (prefab: 10 + 30.47 + 20 + 233.44 + 20 + the species
        // section + 20 + the tags section + 20).
        let n_species = snapshot.species.len();
        let species = 17.88
            + 10.0
            + 13.0
            + if n_species == 0 {
                0.0
            } else {
                n_species as f32 * 31.0 - 1.0
            };
        let tags = 17.88 + 10.0 + list_height(snapshot.tags.len());
        let expected = 10.0 + 30.47 + 20.0 + 233.44 + 20.0 + species + 20.0 + tags + 20.0;
        assert!(
            (layout.panel[3] - expected).abs() < 0.2,
            "{} vs {expected}",
            layout.panel[3]
        );
        // The pie chart sits at the section's right end.
        assert!((layout.pie[0] + layout.pie[2] - layout.right).abs() < 1e-3);
    }

    #[test]
    fn the_prefab_without_rows_is_437_units_high() {
        // The stored size of the panel (no species, no tag items) is 437.67.
        let measure = Measure { fonts: None };
        let snapshot = Snapshot::default();
        let mut empty = snapshot;
        empty.tags.clear();
        let layout = Layout::new(&empty, false, &measure, 1920.0);
        assert!(
            (layout.panel[3] - 437.67).abs() < 0.2,
            "{}",
            layout.panel[3]
        );
    }

    #[test]
    fn the_list_scrolls_only_when_its_rows_do_not_fit() {
        // Viewport 15 + 30 n (n <= 7), content 31 n + 12.
        assert_eq!(scroll_range(0), 0.0);
        assert_eq!(scroll_range(1), 0.0);
        assert_eq!(scroll_range(2), 0.0);
        assert_eq!(scroll_range(3), 0.0, "equal sizes need no scroll bar");
        assert!((scroll_range(4) - 1.0).abs() < 1e-4);
        assert!((scroll_range(7) - 4.0).abs() < 1e-4);
        assert!((scroll_range(8) - (31.0 * 8.0 + 12.0 - 225.0)).abs() < 1e-3);
    }

    #[test]
    fn the_wheel_scrolls_the_list_by_35_a_notch_and_clamps() {
        let mut app = app();
        // Many tags so that the list scrolls.
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        for (index, id) in ids.iter().enumerate() {
            app.world
                .bibites
                .get_mut(*id)
                .expect("record")
                .meta
                .species_tag = Some(format!("t{index}"));
        }
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let n = app.information.snapshot().expect("snapshot").tags.len();
        let _ = frame_of(&app);
        let list = app.information.list_rect.get();
        let inside = Vec2::new(list[0] + 10.0, list[1] + 10.0);
        let outside = Vec2::new(list[0] - 10.0, list[1] + 10.0);
        assert!(!app.information.wheel(outside, 1.0));
        let range = scroll_range(n);
        if range > 0.0 {
            assert!(app.information.wheel(inside, -1.0));
            assert!((app.information.scroll() - range.min(35.0)).abs() < 1e-3);
            assert!(app.information.wheel(inside, -100.0));
            assert!((app.information.scroll() - range).abs() < 1e-3);
            assert!(app.information.wheel(inside, 100.0));
            assert_eq!(app.information.scroll(), 0.0);
        }
    }

    #[test]
    fn the_count_section_lists_the_four_kinds_and_the_energy_section_adds_free() {
        let mut app = app();
        let counts = texts(&frame_of(&app));
        for label in ["Total", "Bibites", "Eggs", "Plants", "Meats"] {
            assert!(
                counts.iter().any(|text| text == label),
                "{label} in {counts:?}"
            );
        }
        assert!(!counts.iter().any(|text| text == "Free"));
        assert!(
            counts.iter().any(|text| text == "count"),
            "the button names the section shown"
        );
        app.information.toggle_mode();
        app.information.update(&app.world, 1.0);
        let energy = texts(&frame_of(&app));
        assert!(energy.iter().any(|text| text == "Free"), "{energy:?}");
        assert!(energy.iter().any(|text| text == "energy"));
        assert!(
            energy.iter().any(|text| text.ends_with('E')),
            "energies carry the E suffix"
        );
    }

    #[test]
    fn the_numbers_are_plain_digits_without_si_prefixes() {
        let snapshot = Snapshot {
            bibites: 1234,
            free_energy: 123_456.0,
            ..Snapshot::default()
        };
        let counts = wedge_rows(&snapshot, false);
        assert_eq!(counts[0].2, "1234");
        assert_eq!(counts[1].2, "1234");
        let energies = wedge_rows(&snapshot, true);
        assert_eq!(energies[1].2, "123456E");
        assert_eq!(
            row_values(5, 1_000_000.0),
            ("5".to_owned(), "1000000E".to_owned())
        );
    }

    #[test]
    fn the_panel_has_its_buttons_rows_and_swallows_clicks() {
        let app = app();
        let frame = frame_of(&app);
        let actions: Vec<UiAction> = frame.actions.iter().map(|(_, action)| *action).collect();
        assert!(actions.contains(&UiAction::Consume));
        assert!(actions.contains(&UiAction::Invoke(ControlCommand::InformationClose)));
        assert!(actions.contains(&UiAction::Invoke(ControlCommand::InformationToggleMode)));
        assert!(
            actions.contains(&UiAction::Invoke(ControlCommand::InformationSelectSpecies(
                0
            )))
        );
        assert!(actions.contains(&UiAction::Invoke(ControlCommand::InformationSelectTag(0))));
        // The panel does not cover the screen: a click on the world still reaches it.
        assert_eq!(frame.action_at(Vec2::new(100.0, 600.0)), None);
        // ...while one on the panel is swallowed (the panel's body or a row of it, never the world).
        assert!(frame.action_at(Vec2::new(1700.0, 150.0)).is_some());
    }

    use crate::app::Reselect;

    fn click(app: &mut App, command: ControlCommand) {
        app.apply_ui_actions(&[UiAction::Invoke(command)]);
    }

    #[test]
    fn a_species_row_selects_a_living_member_of_that_species() {
        let mut app = app();
        let row = app.information.snapshot().expect("snapshot").species[0].clone();
        click(&mut app, ControlCommand::InformationSelectSpecies(0));
        let id = app.selected.expect("a bibite is selected");
        let record = app.world.bibites.get(id).expect("record");
        assert_eq!(record.meta.species, Some(row.id));
        assert!(!record.body.dead);
        assert_eq!(app.reselect, None, "without Ctrl nothing repeats");
        // A row that is not there does nothing.
        let mut fresh = self::app();
        click(&mut fresh, ControlCommand::InformationSelectSpecies(9));
        assert!(fresh.selected.is_none());
    }

    #[test]
    fn a_species_without_living_members_gives_one_of_its_eggs() {
        let mut app = app();
        let row = app.information.snapshot().expect("snapshot").species[0].clone();
        let members: Vec<_> = app
            .world
            .bibites
            .iter()
            .filter(|(_, record)| record.meta.species == Some(row.id))
            .map(|(id, _)| id)
            .collect();
        let parent = members[0];
        assert!(app.world.lay_egg_once(parent) >= 1, "an egg was laid");
        let egg = app.world.eggs.ids().next().expect("the egg");
        app.world.eggs.get_mut(egg).expect("egg").meta.species = Some(row.id);
        for id in members {
            app.world.bibites.get_mut(id).expect("record").body.dead = true;
        }
        click(&mut app, ControlCommand::InformationSelectSpecies(0));
        assert_eq!(app.selected_egg, Some(egg));
        assert!(app.selected.is_none());
    }

    #[test]
    fn ctrl_on_a_species_row_repeats_the_pick_after_the_target_is_gone() {
        let mut app = app();
        let row = app.information.snapshot().expect("snapshot").species[0].clone();
        app.ctrl_held = true;
        app.reselection_delay = 0.5;
        click(&mut app, ControlCommand::InformationSelectSpecies(0));
        assert_eq!(app.reselect, Some(Reselect::Species(row.id)));
        let first = app.selected.expect("selected");
        // The target dies; after the reselection delay another member of the species is picked.
        app.world.bibites.get_mut(first).expect("record").body.dead = true;
        for _ in 0..8 {
            app.advance(0.25);
        }
        let next = app.selected.expect("a new target");
        assert_ne!(next, first);
        let record = app.world.bibites.get(next).expect("record");
        assert_eq!(record.meta.species, Some(row.id));
        assert!(!record.body.dead);
    }

    #[test]
    fn a_tag_row_selects_a_bibite_with_that_tag_and_untagged_selects_one_without() {
        let mut app = app();
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        for id in ids.iter().take(2) {
            app.world
                .bibites
                .get_mut(*id)
                .expect("record")
                .meta
                .species_tag = Some("alpha".to_owned());
        }
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let tags = app.information.snapshot().expect("snapshot").tags.clone();
        let alpha = tags
            .iter()
            .position(|tag| tag.name == "alpha")
            .expect("alpha");
        click(&mut app, ControlCommand::InformationSelectTag(alpha as u32));
        let picked = app.selected.expect("selected");
        assert_eq!(
            app.world
                .bibites
                .get(picked)
                .expect("record")
                .meta
                .species_tag
                .as_deref(),
            Some("alpha")
        );
        let untagged = tags
            .iter()
            .position(|tag| tag.name == UNTAGGED)
            .expect("untagged");
        click(
            &mut app,
            ControlCommand::InformationSelectTag(untagged as u32),
        );
        let picked = app.selected.expect("selected");
        assert_eq!(
            app.world
                .bibites
                .get(picked)
                .expect("record")
                .meta
                .species_tag
                .as_deref()
                .unwrap_or(""),
            ""
        );
        assert_eq!(app.reselect, None);
        app.ctrl_held = true;
        click(&mut app, ControlCommand::InformationSelectTag(alpha as u32));
        assert_eq!(app.reselect, Some(Reselect::Tag));
    }

    #[test]
    fn a_tag_row_can_pick_an_egg() {
        let mut app = app();
        let parent = app
            .world
            .bibites
            .iter()
            .next()
            .map(|(id, _)| id)
            .expect("bibite");
        assert!(app.world.lay_egg_once(parent) >= 1);
        let egg = app.world.eggs.ids().next().expect("egg");
        app.world.eggs.get_mut(egg).expect("egg").meta.species_tag = Some("eggy".to_owned());
        app.information.opened();
        app.information.update(&app.world, 0.0);
        let tags = app.information.snapshot().expect("snapshot").tags.clone();
        let eggy = tags
            .iter()
            .position(|tag| tag.name == "eggy")
            .expect("eggy");
        assert_eq!(tags[eggy].count, 1);
        click(&mut app, ControlCommand::InformationSelectTag(eggy as u32));
        assert_eq!(app.selected_egg, Some(egg));
    }

    #[test]
    fn the_buttons_swap_the_section_and_close_the_panel() {
        let mut app = app();
        assert!(!app.information.energy_mode);
        click(&mut app, ControlCommand::InformationToggleMode);
        assert!(app.information.energy_mode);
        click(&mut app, ControlCommand::InformationToggleMode);
        assert!(!app.information.energy_mode);
        click(&mut app, ControlCommand::InformationClose);
        assert!(!app.panels.is_open(crate::ui::PanelKind::Information));
        // Opening it again refreshes the numbers at once, on the next frame.
        let id = app
            .world
            .bibites
            .iter()
            .next()
            .map(|(id, _)| id)
            .expect("bibite");
        app.world.bibites.get_mut(id).expect("record").body.dead = true;
        let before = app.information.snapshot().expect("snapshot").bibites;
        click(&mut app, ControlCommand::OpenInformation);
        assert!(app.panels.is_open(crate::ui::PanelKind::Information));
        app.advance(0.0);
        assert_eq!(
            app.information.snapshot().map(|s| s.bibites),
            Some(before - 1)
        );
    }

    #[test]
    fn the_numbers_are_not_collected_while_the_panel_is_closed() {
        let mut app = app();
        click(&mut app, ControlCommand::InformationClose);
        let snapshot = app.information.snapshot().cloned();
        let id = app
            .world
            .bibites
            .iter()
            .next()
            .map(|(id, _)| id)
            .expect("bibite");
        app.world.bibites.get_mut(id).expect("record").body.dead = true;
        for _ in 0..20 {
            app.advance(0.25);
        }
        assert_eq!(app.information.snapshot().cloned(), snapshot);
    }

    /// `app()` with a tag on every bibite (nine tags, one of them twice): the list has more rows than fit.
    fn busy_app() -> App {
        let mut app = app();
        let ids: Vec<_> = app.world.bibites.iter().map(|(id, _)| id).collect();
        for (index, id) in ids.iter().enumerate() {
            let record = app.world.bibites.get_mut(*id).expect("record");
            record.meta.species_tag = Some(format!("tag number {}", index % 9));
        }
        app.information.opened();
        app.information.update(&app.world, 0.0);
        app
    }

    #[test]
    fn the_tags_list_shows_its_rows_by_the_scroll_and_a_scroll_bar_when_they_do_not_fit() {
        let mut app = busy_app();
        let n = app.information.snapshot().expect("snapshot").tags.len();
        assert_eq!(n, 10, "nine tags and the untagged ones");
        let rows_hit = |app: &App| -> Vec<u32> {
            let frame = frame_of(app);
            (0..n as u32)
                .filter(|index| {
                    frame.actions.iter().any(|(_, action)| {
                        *action == UiAction::Invoke(ControlCommand::InformationSelectTag(*index))
                    })
                })
                .collect()
        };
        // Seven rows fit (the eighth starts at the viewport's lower edge): the last three are below it.
        assert_eq!(rows_hit(&app), (0..7).collect::<Vec<u32>>());
        let _ = frame_of(&app);
        let list = app.information.list_rect.get();
        let inside = Vec2::new(list[0] + 20.0, list[1] + 20.0);
        // Scrolled to the end: the first rows are above the viewport.
        assert!(app.information.wheel(inside, -100.0));
        assert!((app.information.scroll() - scroll_range(n)).abs() < 1e-3);
        let visible = rows_hit(&app);
        assert!(visible.contains(&9) && visible.contains(&3), "{visible:?}");
        assert!(
            !visible.contains(&0) && !visible.contains(&1),
            "{visible:?}"
        );
        // The scroll bar takes 45 units from the rows' width.
        let measure = Measure { fonts: None };
        let snapshot = app.information.snapshot().expect("snapshot");
        assert!(Layout::new(snapshot, false, &measure, 1920.0).scrollbar);
    }

    #[test]
    fn the_scroll_bar_handle_follows_the_cursor_and_the_track_pages() {
        let mut app = busy_app();
        let frame = frame_of(&app);
        let has = |frame: &UiFrame, handle: bool| {
            frame
                .actions
                .iter()
                .any(|(_, action)| *action == UiAction::InformationScroll(handle))
        };
        assert!(has(&frame, true) && has(&frame, false));
        // Not with rows that fit.
        assert!(!has(&frame_of(&self::app()), true));
        let bar = app.information.bar.get();
        assert!(bar.s > 0.0 && bar.range > 0.0);
        // Press the handle 20 units under its top.
        let at = |bar: &Bar, y: f32| Vec2::new((bar.handle[0] + 5.0) * bar.s, y * bar.s);
        app.information
            .press_bar(true, at(&bar, bar.handle[1] + 20.0));
        assert!(app.information.bar_dragging());
        // Dragged far below / above: the ends of the scroll.
        app.information.drag_bar(at(&bar, bar.handle[1] + 1000.0));
        assert!((app.information.scroll() - bar.range).abs() < 1e-3);
        app.information.drag_bar(at(&bar, bar.handle[1] - 1000.0));
        assert_eq!(app.information.scroll(), 0.0);
        // The handle's top half-way along its travel: half-way scrolled.
        let travel = (1.0 - bar.size) * bar.area_h;
        let top = bar.area_top - HANDLE_OVERHANG + travel / 2.0;
        app.information.drag_bar(at(&bar, top + 20.0));
        assert!((app.information.scroll() - bar.range / 2.0).abs() < 1e-2);
        app.information.release_bar();
        assert!(!app.information.bar_dragging());
        app.information.drag_bar(at(&bar, 0.0));
        assert!(
            (app.information.scroll() - bar.range / 2.0).abs() < 1e-2,
            "no drag, no move"
        );
        // A press on the track under the handle pages down by the handle's share of the range; above it, up.
        let _ = frame_of(&app);
        let bar = app.information.bar.get();
        let before = app.information.scroll();
        app.information
            .press_bar(false, at(&bar, bar.handle[1] + bar.handle[3] + 3.0));
        let page = bar.size * bar.range;
        assert!((app.information.scroll() - (before + page).min(bar.range)).abs() < 1e-3);
        let _ = frame_of(&app);
        let bar = app.information.bar.get();
        let before = app.information.scroll();
        app.information
            .press_bar(false, at(&bar, bar.handle[1] - 3.0));
        assert!((app.information.scroll() - (before - page).max(0.0)).abs() < 1e-3);
    }
}
