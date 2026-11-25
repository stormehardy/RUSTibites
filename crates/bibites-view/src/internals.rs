//! The Biology panel's "Internals" picture: what the game's `internals preview` image shows of a bibite's organs, and the map of which
//! organ each of its cells belongs to (`BiologyPanel.GenerateInternalsMap`, read back from the `internalssections` material).
//!
//! The pixel programs are the game's own (`internals_shaders.rs`, generated from the compiled shaders). The constant-buffer slots
//! were matched to the material properties by reading the two programs side by side: the shared geometry code reads the same
//! values from different slots.

use crate::internals_shaders::{acid_pixel, preview_pixel, sections_pixel};
use crate::ui::UiTexture;

/// `interns_width` / `interns_height`: the material's `_Width` and `_Height`.
pub const WIDTH: usize = 61;
/// See [`WIDTH`].
pub const HEIGHT: usize = 39;
/// The preview material's `_SizeFactor`: the picture is drawn this many times finer than the 61 x 39 map.
pub const SIZE_FACTOR: f32 = 1.0;

/// What `BiologyPanel.OnTargetChanged` writes into the two materials.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Params {
    /// `fat.inflateFactor`.
    pub inflate: f32,
    /// `genes.wombAreaPortion`.
    pub womb: f32,
    /// `genes.armorAreaPortion`.
    pub armor: f32,
    /// `genes.throatAreaPortion`.
    pub throat: f32,
    /// `genes.stomachAreaPortion`.
    pub stomach: f32,
    /// `genes.jawAreaPortion`.
    pub jaw: f32,
    /// `genes.moveMusclesAreaPortion`.
    pub muscles: f32,
    /// `_FatStart` (a fraction of the width; 1 while there is no fat section).
    pub fat_start: f32,
    /// `_FatEnd`.
    pub fat_end: f32,
    /// `_FatFullness`.
    pub fat_fullness: f32,
    /// `genes.GetBodyColor()`.
    pub color: [f32; 4],
}

/// What `BiologyPanel.OnTargetChanged` reads from the bibite. The fat section's start and end are found from the map
/// (`GenerateInternalsMap`), so they are filled in by [`Params::with_fat_range`] after the map is known.
pub fn params_of(record: &bibites_core::body::BibiteRecord) -> Params {
    use bibites_core::genome::Gene;
    let genome = &record.genome;
    let [r, g, b] = bibites_core::genome::genes_to_color(
        genome.gene(Gene::ColorR),
        genome.gene(Gene::ColorG),
        genome.gene(Gene::ColorB),
    );
    let mut params = Params {
        inflate: record.fat.inflate_factor,
        womb: genome.womb_area_portion(),
        armor: genome.organ_area_portion(Gene::ArmorWAG),
        throat: genome.organ_area_portion(Gene::ThroatWAG),
        stomach: genome.organ_area_portion(Gene::StomachWAG),
        jaw: genome.organ_area_portion(Gene::MouthMusclesWAG),
        muscles: genome.organ_area_portion(Gene::MoveMusclesWAG),
        fat_start: 1.0,
        fat_end: 0.0,
        fat_fullness: if record.fat.base_capacity > 0.0 {
            (record.fat.amount / record.fat.base_capacity).min(1.0)
        } else {
            0.0
        },
        color: [r, g, b, 1.0],
    };
    params.with_fat_range();
    params
}

impl Params {
    /// `GenerateInternalsMap`: the columns where the map's fat section (7) begins and ends, as fractions of the width.
    /// The game scans the lower half of the map from the left for the first column holding a 7 (`fatEnd`) and from the right
    /// for the last one (`fatStart`); with no fat section the material keeps its defaults (start 1, end 0).
    pub fn with_fat_range(&mut self) {
        let map = sections(self);
        let mut end: i32 = -1;
        let mut start: i32 = -1;
        let low = 2 + (HEIGHT - 1) / 2;
        for x in 2..WIDTH {
            if end >= 0 {
                break;
            }
            for y in low..HEIGHT {
                if map[x + y * WIDTH] == 7 {
                    end = x as i32;
                    break;
                }
            }
        }
        let mut x = WIDTH as i32 - 1;
        while x > end && start < 0 {
            for y in low..HEIGHT {
                if map[x as usize + y * WIDTH] == 7 {
                    start = x;
                    break;
                }
            }
            x -= 1;
        }
        if end > 0 {
            self.fat_start = start as f32 / WIDTH as f32;
            self.fat_end = end as f32 / WIDTH as f32;
        }
    }
}

/// The organ of each cell (`internsMap`, row 0 at the bottom as the render target is read): 0 outside, 1 body, 2 stomach, 3 womb, 4 mouth,
/// 5 body, 6 mouth, 7 fat.
pub fn sections(p: &Params) -> Vec<u8> {
    let cb = [
        [0.0; 4],
        [0.0; 4],
        [p.inflate, p.womb, WIDTH as f32, HEIGHT as f32],
        [p.armor, p.throat, p.stomach, p.jaw],
        [p.muscles, 0.0, 0.0, 0.0],
    ];
    let mut map = vec![0u8; WIDTH * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let uv = [(x as f32 + 0.5) / WIDTH as f32, (y as f32 + 0.5) / HEIGHT as f32];
            let colour = sections_pixel(uv, &cb);
            map[x + y * WIDTH] = (colour[0] * 7.0).round() as u8;
        }
    }
    map
}

/// The preview picture: `((W - 1) * 3 + 1) x ((H - 1) * 3 + 1)` pixels, top row first.
pub fn preview_size() -> (usize, usize) {
    (
        ((WIDTH - 1) as f32 * SIZE_FACTOR) as usize + 1,
        ((HEIGHT - 1) as f32 * SIZE_FACTOR) as usize + 1,
    )
}

/// Run the preview program over the picture.
pub fn preview(p: &Params) -> UiTexture {
    let cb = [
        [0.0; 4],
        [0.0; 4],
        [p.inflate, p.womb, WIDTH as f32, SIZE_FACTOR],
        [HEIGHT as f32, p.armor, p.throat, p.stomach],
        [p.jaw, p.muscles, p.fat_fullness, p.fat_start],
        [p.fat_end, 0.0, 0.0, 0.0],
        p.color,
    ];
    let (w, h) = preview_size();
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            // The texture's v runs up the picture: the top row of the image is v = 1.
            let uv = [(x as f32 + 0.5) / w as f32, 1.0 - (y as f32 + 0.5) / h as f32];
            let c = preview_pixel(uv, &cb);
            let at = (x + y * w) * 4;
            for k in 0..4 {
                rgba[at + k] = (c[k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    UiTexture {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

thread_local! {
    static PREVIEW: std::cell::RefCell<Option<(Params, std::sync::Arc<UiTexture>)>> = const { std::cell::RefCell::new(None) };
}

/// [`preview`] of the last parameters asked for, kept: the panel is redrawn every frame and a bibite's organs change rarely.
pub fn preview_cached(p: &Params) -> std::sync::Arc<UiTexture> {
    PREVIEW.with(|cell| {
        let mut cell = cell.borrow_mut();
        if let Some((key, texture)) = cell.as_ref()
            && key == p
        {
            return std::sync::Arc::clone(texture);
        }
        let texture = std::sync::Arc::new(preview(p));
        *cell = Some((*p, std::sync::Arc::clone(&texture)));
        texture
    })
}

thread_local! {
    static SECTIONS: std::cell::RefCell<Option<(Params, std::sync::Arc<Vec<u8>>)>> = const { std::cell::RefCell::new(None) };
}

/// [`sections`] of the last parameters asked for (the pointer asks every frame).
pub fn sections_cached(p: &Params) -> std::sync::Arc<Vec<u8>> {
    SECTIONS.with(|cell| {
        let mut cell = cell.borrow_mut();
        if let Some((key, map)) = cell.as_ref()
            && key == p
        {
            return std::sync::Arc::clone(map);
        }
        let map = std::sync::Arc::new(sections(p));
        *cell = Some((*p, std::sync::Arc::clone(&map)));
        map
    })
}

/// What `BiologyPanel` reads of a bibite's stomach.
pub fn stomach_info_of(record: &bibites_core::body::BibiteRecord) -> StomachInfo {
    use bibites_core::genome::Gene;
    let stomach = &record.stomach;
    StomachInfo {
        area: stomach.area,
        total: stomach.total_amount(),
        acid: record.brain.output_value(bibites_core::brain::Output::Digestion),
        bonus: stomach.digestion_potential_bonus,
        malus: stomach.efficiency_malus,
        effectiveness: stomach.digestion_effectiveness,
        portion: record.genome.organ_area_portion(Gene::StomachWAG),
        items: stomach
            .contents()
            .map(|content| StomachItem {
                material: content.material,
                amount: content.amount,
                average_chunk: content.average_chunk_amount,
                digestion_rate: content.digestion_rate,
                efficiency: content.last_conversion_efficiency,
            })
            .collect(),
        dead: record.body.dead,
    }
}

/// Where the stomach is in the 61 x 39 map (`InternalsCalculator.FromGenes`, in the map's cells).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StomachShape {
    /// `stomachOffset`: the centre of the stomach (`y` is the body's middle row).
    pub offset: [f32; 2],
    /// `stomach_r`: the radius of its rounded ends, plus a half.
    pub r: f32,
    /// `stomach_w`: its whole length.
    pub w: f32,
    /// The womb in pixels of the picture: its centre's `x` (`(wombOffset.x - womb_h / 3) * 5`) and the radius its eggs may fill (`5 * (womb_r - 0.5)`).
    pub womb: [f32; 2],
}

impl StomachShape {
    /// `InternalsCalculator.FromGenes`, for the organs the stomach's place depends on.
    pub fn of(p: &Params) -> Self {
        use std::f32::consts::{FRAC_PI_2, PI};
        // `Mathf.Round` rounds halves to even.
        let round = f32::round_ties_even;
        let width = WIDTH as f32;
        let height = HEIGHT as f32;
        let body_y = height / 2.0;
        let num = round(body_y);
        let num2 = width - num + 1.0;
        let num3 = FRAC_PI_2 - 2.0 * (num / num2).atan();
        let num4 = num2 / num3.sin();
        let num5 = (1.0 - p.armor).sqrt();
        let num6 = (1.0f32).max(round(num * (1.0 - num5)));
        let num7 = num - num6;
        let num8 = num4 - num6;
        let num9 = (1.0 - num7 / num8).acos();
        let num10 = num8 * num9.sin();
        let num11 = FRAC_PI_2 * num7 * num7 + num8 * num8 * num9 - num10 * (num8 - num7);
        let num12 = p.womb.max(0.02).sqrt();
        let womb_r = round(num * num12) - 2.5;
        let womb_h = round(num2 * num12) - 5.0;
        let num13 = 1.0 + 2.0 * ((num11 * p.throat).sqrt() / 2.0).floor();
        let womb_x = round(num6 / num9.cos() + womb_h + 1.0);
        let num14 = p.muscles.sqrt();
        let num15 = round((num7 - 2.0) * num14);
        let num16 = num11 * (p.stomach - 0.1);
        let num17 = 3.0;
        let k = 0.858_407_26_f32;
        let mut r = round(2.0 + (num16 / (-k + 2.0 * num17)).sqrt().max((num13 - 1.0) / 2.0));
        let mut num18 = round(r - (r * r - num13 * num13 / 4.0).sqrt());
        let mut w = round(((num16 / r + k * r) / 2.0).max(num13));
        if w + womb_h + womb_r > width - (num13 - 1.0) / 2.0 {
            w = width - (num13 - 1.0) / 2.0 - womb_h - womb_r;
            r = ((-w + (w * w + num16 * -k).sqrt()) / -k).floor();
            if r > width / 2.0 - num6 - num15 + 1.5 {
                r = width / 2.0 - num6 - num15 + 1.5;
                w = (((num16 / r + k * r) / 2.0).max(num13)).floor();
            }
        }
        r = r.floor() + 0.5;
        w = w.floor() + 3.0;
        if num18 < womb_x + womb_r + w + num13 + 1.0 - width {
            num18 = (womb_x + womb_r + w + num13 + 1.0 - width).floor();
            if num18 > num13 / 2.0 {
                num18 = (num18 - (num18 - num13 / 2.0) / 1.5).floor();
            }
        }
        let x = (width - num13 + num18 - w / 2.0).floor();
        let _ = PI;
        Self {
            offset: [x, body_y],
            r,
            w,
            womb: [(womb_x - womb_h / 3.0) * 5.0, 5.0 * (womb_r - 0.5)],
        }
    }

    /// The numbers `BiologyPanel` keeps for the pellets, in pixels of the picture (5 per cell): the centre's `x`, the radius a pellet's centre
    /// may reach, the length and height of the capsule.
    pub fn pixels(&self) -> (f32, f32, f32, f32) {
        let r = 5.0 * (self.r - 0.5);
        (5.0 * self.offset[0], r, 5.0 * self.w, 2.0 * r)
    }
}

/// `UI/StomachAcid` over the picture: the liquid at the bottom of the stomach, as the game's program draws it (straight RGBA of the
/// output, blended by its alpha). `time` is `Time.unscaledTime`.
pub fn acid_picture(shape: &StomachShape, level: f32, time: f32) -> UiTexture {
    let cb = [
        [0.0; 4],
        [0.0; 4],
        [0.0; 4],
        [WIDTH as f32, HEIGHT as f32, level, 0.0],
        // `_Color` of the `acidLiquid` material.
        [0.354_925_87, 0.494_117_65, 0.0, 0.572_549_04],
        [shape.offset[0], shape.offset[1], shape.w, 2.0 * shape.r],
        [time, 0.0, 0.0, 0.0],
    ];
    let mut rgba = vec![0u8; WIDTH * HEIGHT * 4];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let uv = [
                (x as f32 + 0.5) / WIDTH as f32,
                1.0 - (y as f32 + 0.5) / HEIGHT as f32,
            ];
            let c = acid_pixel(uv, &cb);
            let at = (x + y * WIDTH) * 4;
            for k in 0..4 {
                rgba[at + k] = (c[k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    UiTexture {
        width: WIDTH as u32,
        height: HEIGHT as u32,
        rgba,
    }
}

/// One material in the stomach (`StomachContent`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StomachItem {
    /// Plant or meat.
    pub material: bibites_core::organs::StomachMaterial,
    /// `amount` (u²).
    pub amount: f32,
    /// `averageChunkAmount`.
    pub average_chunk: f32,
    /// `digestionRate` (u²/s).
    pub digestion_rate: f32,
    /// `lastConversionEfficiency`.
    pub efficiency: f32,
}

impl StomachItem {
    /// `MatterMaterial.Name`.
    pub fn name(&self) -> &'static str {
        match self.material {
            bibites_core::organs::StomachMaterial::Plant => "Plant",
            bibites_core::organs::StomachMaterial::Meat => "Meat",
        }
    }

    /// `digestionRate * efficiency * Material.EnergyDensity` (the energy the slot gives per second).
    pub fn energy_gain(&self) -> f32 {
        self.digestion_rate * self.efficiency * self.material.params().energy_density
    }
}

/// What the Biology panel shows of the stomach (`BibiteStomach`, read by `UpdateStomachSection` and `AnimateStomach`).
#[derive(Clone, Debug, PartialEq)]
pub struct StomachInfo {
    /// `stomach.area`: the capacity (u²).
    pub area: f32,
    /// `stomach.totalAmount`.
    pub total: f32,
    /// `stomach.digestOutput`: the brain's Digestion output (the acid).
    pub acid: f32,
    /// `stomach.digestionPotentialBonus`.
    pub bonus: f32,
    /// `stomach.efficiencyMalus`.
    pub malus: f32,
    /// `stomach.digestionEffectiveness`.
    pub effectiveness: f32,
    /// `genes.stomachAreaPortion`.
    pub portion: f32,
    /// The contents.
    pub items: Vec<StomachItem>,
    /// `body.dead`: `AnimateStomach` returns before the pellets digest, and a dead bibite swallows and vomits nothing, so the picture keeps what it holds.
    pub dead: bool,
}

impl StomachInfo {
    /// `stomach.fullness`.
    pub fn fullness(&self) -> f32 {
        if self.area > 0.0 {
            self.total / self.area
        } else {
            0.0
        }
    }
}

/// The pellets that float in the picture of a stomach (`StomachContentPellet`): `FillStomachContentUI` cuts each content into chunks of its mean
/// chunk (at least a thirtieth of the stomach and 0.2, at most half of it), each between half and one and a half times that. The game places
/// them at random and lets a physics body move them about; here the places come from a hash of the bibite and the chunk, and do not move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pellet {
    /// Centre in pixels of the picture (`x` right, `y` down from the top).
    pub pos: [f32; 2],
    /// Side of its square in pixels.
    pub size: f32,
    /// Plant or meat.
    pub material: bibites_core::organs::StomachMaterial,
    /// `amount`.
    pub amount: f32,
    /// How far it is turned, in radians (the physics body's `rotation`).
    pub rotation: f32,
}

fn hash01(seed: u64) -> f32 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// The pellets of `info`, placed inside the stomach `shape`.
pub fn pellets(info: &StomachInfo, shape: &StomachShape, seed: u64) -> Vec<Pellet> {
    let (cx, r, w, h) = shape.pixels();
    let mut out = Vec::new();
    for (slot, item) in info.items.iter().enumerate() {
        let chunk = item
            .average_chunk
            .max(info.area / 30.0)
            .max(0.2)
            .min(info.area / 2.0);
        let mut left = item.amount;
        let mut k = 0u64;
        while left > 0.0 && out.len() < 400 {
            let key = seed ^ ((slot as u64) << 56) ^ (k << 20);
            let mut amount = chunk * (0.5 + hash01(key));
            if amount > left {
                amount = left;
                left = -0.01;
            } else {
                left -= amount;
            }
            // `AddMatterPellet`: the scale of the sprite and a random point of the capsule.
            let scale = 0.8 * ((amount / info.area).sqrt() * r / 10.0).max(r / 100.0);
            let radius = (amount / std::f32::consts::PI).sqrt();
            let (cw, ch) = ((w - radius).max(0.0), (h - radius).max(0.0));
            // A point of the capsule `cw x ch`: its rounded ends have the radius `ch / 2`.
            let rad = ch / 2.0;
            let straight = (cw - ch).max(0.0);
            let area_line = straight * ch;
            let area_ends = std::f32::consts::PI * rad * rad;
            let pick = hash01(key ^ 0x5151) * (area_line + area_ends).max(1e-6);
            let (px, py) = if pick < area_line {
                (
                    (hash01(key ^ 0x7777) - 0.5) * straight,
                    (hash01(key ^ 0xA2A2) - 0.5) * ch,
                )
            } else {
                let angle = hash01(key ^ 0xA2A2) * std::f32::consts::TAU;
                let dist = hash01(key ^ 0x3333).sqrt() * rad;
                let side = if hash01(key ^ 0x1111) < 0.5 { -1.0 } else { 1.0 };
                (
                    side * (straight / 2.0 + angle.cos().abs() * dist),
                    angle.sin() * dist,
                )
            };
            out.push(Pellet {
                pos: [cx + px, 97.5 + py],
                size: 20.0 * scale,
                material: item.material,
                amount,
                rotation: hash01(key ^ 0x5151) * std::f32::consts::TAU,
            });
            k += 1;
        }
    }
    out
}

/// The sprite of a pellet of `size_factor = sqrt(amount / 20)` (`ClosestSizeIndex` over `pellet_sizes`: medium 9, tiny 5, large 14 pixels wide,
/// measured against the first).
pub fn pellet_sprite(material: bibites_core::organs::StomachMaterial, amount: f32) -> &'static str {
    let factor = (amount / 20.0).sqrt();
    let mut best = "medium";
    let mut distance = f32::INFINITY;
    for (name, ratio) in [("medium", 1.0f32), ("tiny", 5.0 / 9.0), ("large", 14.0 / 9.0)] {
        let d = (factor - ratio).abs();
        if d < distance {
            distance = d;
            best = name;
        }
    }
    match (material, best) {
        (bibites_core::organs::StomachMaterial::Plant, "tiny") => "tiny_plant",
        (bibites_core::organs::StomachMaterial::Plant, "large") => "large_plant",
        (bibites_core::organs::StomachMaterial::Plant, _) => "medium_plant",
        (_, "tiny") => "tiny_meat",
        (_, "large") => "large_meat",
        _ => "medium_meat",
    }
}

/// One egg in the womb of the picture (`UIEgg`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EggInPicture {
    /// Centre in pixels of the picture.
    pub pos: [f32; 2],
    /// Scale of its sprite (the sprite's own pixels times this).
    pub scale: f32,
    /// Its amount (`SetSize(scale, amount)`: the sprite follows `sqrt(amount / 100)`).
    pub amount: f32,
    /// How far it is turned, in degrees (`rotation = Random.value * 360`; a hash of the bibite and the egg here).
    pub rotation: f32,
}

/// `BiologyPanel.UpdateWombAnimation`: a whole egg for every full unit of progress and a growing one for the part: `ceil(progress)` eggs of the scale
/// `sqrt(eggSize / area) * womb_r / 5`, the last one smaller by the square root of its progress, each at a random place of the womb (a hash of the bibite
/// and the egg here) and turned by a random angle (`Random.value * 360`, kept per 10 degrees).
pub fn eggs_in_womb(progress: f32, current: f32, egg_size: f32, womb_area: f32, shape: &StomachShape, seed: u64) -> Vec<EggInPicture> {
    let count = progress.ceil().max(0.0) as usize;
    if count == 0 || womb_area <= 0.0 {
        return Vec::new();
    }
    let [cx, womb_r] = shape.womb;
    let scale = (egg_size / womb_area).sqrt() * womb_r / 5.0;
    (0..count.min(200))
        .map(|i| {
            let last = i + 1 == count;
            let key = seed ^ 0xE66 ^ ((i as u64) << 24);
            // `Random.insideUnitCircle`
            let angle = hash01(key ^ 0x31) * std::f32::consts::TAU;
            let dist = hash01(key ^ 0x32).sqrt() * (womb_r - 5.0 * scale).max(0.0);
            let (egg_scale, amount) = if last {
                (scale * current.max(0.0).sqrt(), egg_size * current)
            } else {
                (scale, egg_size)
            };
            EggInPicture {
                pos: [cx + dist * angle.cos(), 97.5 + dist * angle.sin()],
                scale: egg_scale,
                amount,
                rotation: hash01(key ^ 0x33) * 360.0,
            }
        })
        .collect()
}

/// The egg sheet and the width of its art (`ClosestSizeIndex` over `egg_sizes`: medium 11 first, tiny 6, large 17 pixels wide).
pub fn egg_sheet(amount: f32) -> (&'static str, f32) {
    let factor = (amount / 100.0).max(0.0).sqrt();
    let mut best = ("medium", 11.0);
    let mut distance = f32::INFINITY;
    for (name, width) in [("medium", 11.0f32), ("tiny", 6.0), ("large", 17.0)] {
        let d = (factor - width / 11.0).abs();
        if d < distance {
            distance = d;
            best = (name, width);
        }
    }
    best
}

thread_local! {
    static EGG_ART: std::cell::RefCell<std::collections::HashMap<(&'static str, [u32; 3], u16), std::sync::Arc<UiTexture>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The egg art of a size in the body's colour (the skin shader's maths over the game's blue egg sheet), kept per size and colour.
pub fn egg_texture(sheet: &'static str, color: [f32; 3], rotation: f32) -> Option<std::sync::Arc<UiTexture>> {
    // The turned picture is kept for every 10 degrees.
    let step = ((rotation.rem_euclid(360.0) / 10.0).round() as u16) % 36;
    let key = (sheet, [color[0].to_bits(), color[1].to_bits(), color[2].to_bits()], step);
    EGG_ART.with(|cell| {
        if let Some(texture) = cell.borrow().get(&key) {
            return Some(std::sync::Arc::clone(texture));
        }
        let file = std::fs::File::open(format!("assets/game/sheets/{sheet}_egg.png")).ok()?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).ok()?;
        let channels = info.color_type.samples();
        let tint = crate::skin::SkinTint::new(color, crate::skin::SkinParams::default());
        let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
        for texel in buffer[..info.buffer_size()].chunks(channels) {
            let (rgb, alpha) = match channels {
                4 => ([texel[0], texel[1], texel[2]], texel[3]),
                3 => ([texel[0], texel[1], texel[2]], 255),
                2 => ([texel[0]; 3], texel[1]),
                _ => ([texel[0]; 3], 255),
            };
            let out = tint.apply([f32::from(rgb[0]) / 255.0, f32::from(rgb[1]) / 255.0, f32::from(rgb[2]) / 255.0]);
            rgba.extend_from_slice(&[
                (out[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                (out[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                (out[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                alpha,
            ]);
        }
        let upright = UiTexture {
            width: info.width,
            height: info.height,
            rgba,
        };
        let texture = std::sync::Arc::new(turned(&upright, f32::from(step) * 10.0));
        cell.borrow_mut().insert(key, std::sync::Arc::clone(&texture));
        Some(texture)
    })
}

thread_local! {
    static EGG_SHEETS: std::cell::RefCell<std::collections::HashMap<&'static str, std::rc::Rc<UiTexture>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The raw egg sheet (the game's blue art), decoded once.
fn egg_sheet_pixels(sheet: &'static str) -> Option<std::rc::Rc<UiTexture>> {
    EGG_SHEETS.with(|cell| {
        if let Some(texture) = cell.borrow().get(sheet) {
            return Some(std::rc::Rc::clone(texture));
        }
        let file = std::fs::File::open(format!("assets/game/sheets/{sheet}_egg.png")).ok()?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).ok()?;
        let channels = info.color_type.samples();
        let mut rgba = Vec::with_capacity((info.width * info.height * 4) as usize);
        for texel in buffer[..info.buffer_size()].chunks(channels) {
            let (rgb, alpha) = match channels {
                4 => ([texel[0], texel[1], texel[2]], texel[3]),
                3 => ([texel[0], texel[1], texel[2]], 255),
                2 => ([texel[0]; 3], texel[1]),
                _ => ([texel[0]; 3], 255),
            };
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
        }
        let texture = std::rc::Rc::new(UiTexture { width: info.width, height: info.height, rgba });
        cell.borrow_mut().insert(sheet, std::rc::Rc::clone(&texture));
        Some(texture)
    })
}

/// The growing egg of the womb (`UIEgg.SetGrowing`): the game's `UIEggShader` run over the egg sheet with the glow switched on (a scan line that
/// crosses the egg: green while it grows, orange while it is absorbed), at `time` seconds, then turned by `rotation` degrees as the image is.
/// `production` is the brain's egg production (its sign picks the colour).
pub fn growing_egg_texture(sheet: &'static str, color: [f32; 3], rotation: f32, production: f32, time: f32) -> Option<std::sync::Arc<UiTexture>> {
    let upright = shaded_egg(sheet, color, true, production, time)?;
    let step = ((rotation.rem_euclid(360.0) / 10.0).round() as u16) % 36;
    Some(std::sync::Arc::new(turned(&upright, f32::from(step) * 10.0)))
}

/// The egg sheet run through `UIEggShader` (upright): `growing` is the material's `_growing`.
fn shaded_egg(sheet: &'static str, color: [f32; 3], growing: bool, production: f32, time: f32) -> Option<UiTexture> {
    let art = egg_sheet_pixels(sheet)?;
    let (w, h) = (art.width as usize, art.height as usize);
    let params = crate::skin::SkinParams::default();
    let cb0: Vec<[f32; 4]> = (0..131)
        .map(|slot| if slot == 130 { [params.from_hue, params.vmin_at_s0, params.vmax_at_s0, time] } else { [0.0; 4] })
        .collect();
    let absorbing = if production < 0.0 { 1.0 } else { 0.0 };
    let cb1 = [
        [w as f32, h as f32, w as f32, h as f32],
        [color[0], color[1], color[2], 0.0],
        // `_scanlineWidth`, `_growing`, `_absorbing`, `_AnimationSpeed`.
        [0.7, if growing { 1.0 } else { 0.0 }, absorbing, 3.0],
    ];
    let fetch = |uv: [f32; 2]| -> [f32; 4] {
        let x = ((uv[0] * w as f32).floor() as i64).clamp(0, w as i64 - 1) as usize;
        let y = (((1.0 - uv[1]) * h as f32).floor() as i64).clamp(0, h as i64 - 1) as usize;
        let at = (y * w + x) * 4;
        [
            f32::from(art.rgba[at]) / 255.0,
            f32::from(art.rgba[at + 1]) / 255.0,
            f32::from(art.rgba[at + 2]) / 255.0,
            f32::from(art.rgba[at + 3]) / 255.0,
        ]
    };
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let uv = [(x as f32 + 0.5) / w as f32, 1.0 - (y as f32 + 0.5) / h as f32];
            let Some(out) = crate::internals_shaders::egg_pixel(uv, [1.0; 4], &cb0, &cb1, &fetch) else {
                continue;
            };
            let to = (y * w + x) * 4;
            for k in 0..4 {
                rgba[to + k] = (out[k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    Some(UiTexture { width: art.width, height: art.height, rgba })
}

/// A picture turned `degrees` about its middle (an `Image` with a rotated transform: the picture sampled point by point onto a square that holds the
/// turned corners).
pub(crate) fn turned(texture: &UiTexture, degrees: f32) -> UiTexture {
    // The game's GPU samples the turned picture at the screen's resolution, so a turned square keeps its straight edges: the result here is
    // drawn on a grid `TURN_SS` times finer than the picture's own pixels (callers divide the width by it for the size in picture pixels).
    let ss = TURN_SS as f32;
    let (w, h) = (texture.width as f32, texture.height as f32);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let side = (w * cos.abs() + h * sin.abs()).max(w * sin.abs() + h * cos.abs()).ceil().max(1.0);
    let side_px = (side * ss) as u32;
    let mut rgba = vec![0u8; (side_px * side_px * 4) as usize];
    for y in 0..side_px {
        for x in 0..side_px {
            // The output pixel's middle, from the middle of the output, turned back into the picture.
            let (dx, dy) = ((x as f32 + 0.5) / ss - side / 2.0, (y as f32 + 0.5) / ss - side / 2.0);
            let (sx, sy) = (dx * cos + dy * sin + w / 2.0, -dx * sin + dy * cos + h / 2.0);
            if sx < 0.0 || sy < 0.0 || sx >= w || sy >= h {
                continue;
            }
            let from = ((sy as u32 * texture.width + sx as u32) * 4) as usize;
            let to = ((y * side_px + x) * 4) as usize;
            rgba[to..to + 4].copy_from_slice(&texture.rgba[from..from + 4]);
        }
    }
    UiTexture { width: side_px, height: side_px, rgba }
}

/// How many times finer than the picture's own pixels a turned picture is drawn (see [`turned`]).
pub const TURN_SS: u32 = 6;

thread_local! {
    static TURNED_ART: std::cell::RefCell<std::collections::HashMap<(u16, u16), std::sync::Arc<UiTexture>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A texture of the UI atlas turned `radians` (kept for every 10 degrees), for the pellets that spin in the stomach's picture.
pub fn turned_texture(ui: &crate::ui::UiAssets, slot: u16, radians: f32) -> Option<std::sync::Arc<UiTexture>> {
    let step = ((radians.to_degrees().rem_euclid(360.0) / 10.0).round() as u16) % 36;
    TURNED_ART.with(|cell| {
        if let Some(texture) = cell.borrow().get(&(slot, step)) {
            return Some(std::sync::Arc::clone(texture));
        }
        let texture = std::sync::Arc::new(turned(ui.texture_pixels(slot)?, f32::from(step) * 10.0));
        cell.borrow_mut().insert((slot, step), std::sync::Arc::clone(&texture));
        Some(texture)
    })
}

/// The acid level of a stomach drawn by `UI/StomachAcid`.
#[allow(dead_code)]
pub fn acid(uv: [f32; 2], cb: &[[f32; 4]]) -> [f32; 4] {
    acid_pixel(uv, cb)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The organ map the translated sections program makes is the one the game's GPU makes: `internals_gpu_maps.txt` holds the GPU's own output
    /// (the parity plugin's `POST /d/blit_material` on the game's `sections_mat`, read back with `ReadPixels` exactly as `GenerateInternalsMap`
    /// does) for eleven sets of material values (the last one is the `internsMap` the game itself made for a live bibite), the top row of each picture first. (Two `movc` instructions select a NEGATED register: the
    /// translator used to drop the minus, which bent the whole body outline.)
    #[test]
    fn the_organ_map_is_what_the_games_gpu_makes() {
        let text = include_str!("internals_gpu_maps.txt");
        let mut cases = 0;
        let mut lines = text.lines().filter(|line| !line.starts_with('#')).peekable();
        while let Some(head) = lines.next() {
            let values: std::collections::HashMap<&str, f32> = head
                .strip_prefix("case ")
                .expect("a case line")
                .split(' ')
                .filter_map(|pair| pair.split_once('='))
                .map(|(key, value)| (key, value.parse().expect("number")))
                .collect();
            let params = Params {
                inflate: values["inflate"],
                womb: values["womb"],
                armor: values["armor"],
                throat: values["throat"],
                stomach: values["stomach"],
                jaw: values["jaw"],
                muscles: values["muscles"],
                ..captured()
            };
            let map = sections(&params);
            let mut differing = Vec::new();
            for row in 0..HEIGHT {
                let expected = lines.next().expect("a picture row");
                for (x, digit) in expected.bytes().enumerate() {
                    let ours = map[x + (HEIGHT - 1 - row) * WIDTH];
                    if ours != digit - b'0' {
                        differing.push((x, HEIGHT - 1 - row, ours, digit - b'0'));
                    }
                }
            }
            assert!(differing.is_empty(), "{head}: {} cells differ from the GPU (x, y, ours, GPU), first: {:?}", differing.len(), &differing[..differing.len().min(6)]);
            cases += 1;
        }
        assert_eq!(cases, 11);
    }

    /// The preview picture the translated program makes is the one the game's GPU makes: `internals_gpu_preview.bin` holds the GPU's RGBA
    /// (61 x 39, bottom row first) of the `interns` material for four sets of values (`tools/parity/capture_material_oracle.py`); each of our
    /// channel values may be one off (the GPU rounds a unorm8 its own way).
    #[test]
    fn the_preview_is_what_the_games_gpu_makes() {
        let live = Params {
            inflate: 0.0,
            womb: 0.190_816_33,
            armor: 0.005_102_041,
            throat: 0.099_999_994,
            stomach: 0.381_632_66,
            jaw: 0.081_632_66,
            muscles: 0.049_999_997,
            fat_start: 0.819_672_1,
            fat_end: 0.196_721_32,
            fat_fullness: 0.025_957_566,
            color: [0.9, 0.9, 0.9, 1.0],
        };
        let narrow = Params {
            inflate: 0.0,
            womb: 0.0,
            armor: 0.0,
            throat: 0.0,
            stomach: 0.0,
            jaw: 0.0,
            muscles: 0.0,
            fat_start: 0.6,
            fat_end: 0.4,
            fat_fullness: 0.5,
            color: [0.5, 0.5, 0.5, 1.0],
        };
        let cases = [
            ("live values", live),
            ("live values, full of fat", Params { fat_fullness: 1.0, ..live }),
            ("a narrow fat range on grey", narrow),
            ("inflated", Params { inflate: 0.5, fat_fullness: 0.5, ..live }),
        ];
        let gpu = include_bytes!("internals_gpu_preview.bin");
        let (w, h) = (WIDTH, HEIGHT);
        assert_eq!(gpu.len(), cases.len() * w * h * 4);
        for (index, (name, params)) in cases.iter().enumerate() {
            let ours = preview(params);
            assert_eq!((ours.width as usize, ours.height as usize), (w, h));
            let block = &gpu[index * w * h * 4..][..w * h * 4];
            let mut worst = 0i32;
            let mut wrong = 0;
            for y in 0..h {
                for x in 0..w {
                    for k in 0..4 {
                        let theirs = i32::from(block[(x + (h - 1 - y) * w) * 4 + k]);
                        let difference = (theirs - i32::from(ours.rgba[(x + y * w) * 4 + k])).abs();
                        worst = worst.max(difference);
                        wrong += usize::from(difference > 1);
                    }
                }
            }
            assert_eq!(wrong, 0, "{name}: {wrong} channel values differ from the GPU by more than 1 (worst {worst})");
        }
    }

    /// The materials as the captured scene had them (`internals_preview` and `internalssections` in sharedassets2).
    fn captured() -> Params {
        Params {
            inflate: 0.0,
            womb: 0.043_171_08,
            armor: 0.042_103_75,
            throat: 0.300_057_5,
            stomach: 0.201_963_3,
            jaw: 0.157_451_16,
            muscles: 0.109_625_94,
            fat_start: 1.0,
            fat_end: 0.0,
            fat_fullness: 0.0,
            color: [0.988_306_3, 0.453_058_1, 0.495_261_2, 1.0],
        }
    }

    /// Probe (ignored): the preview picture for a few inflate / fullness pairs, to C:/Temp/internals_probe_<n>.png.
    #[test]
    #[ignore]
    fn internals_probe() {
        let base = Params {
            inflate: 1.0,
            womb: 0.19081633,
            armor: 0.005102041,
            throat: 0.1,
            stomach: 0.38163266,
            jaw: 0.08163266,
            muscles: 0.05,
            fat_start: 0.9508197,
            fat_end: 0.04918033,
            fat_fullness: 1.0,
            color: [0.3, 0.8, 0.4, 1.0],
        };
        for (n, (inflate, fullness)) in [(1.0, 1.0), (1.0, 0.0), (0.545, 1.0), (0.0, 1.0), (0.0, 0.0)].into_iter().enumerate() {
            let params = Params { inflate, fat_fullness: fullness, ..base };
            let picture = preview(&params);
            let file = std::fs::File::create(format!("C:/Temp/internals_probe_{n}.png")).unwrap();
            let mut encoder = png::Encoder::new(file, picture.width, picture.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&picture.rgba).unwrap();
        }
    }

    #[test]
    fn the_egg_shader_without_its_glow_is_the_skin_recolour_and_with_it_the_scan_line_moves() {
        if !std::path::Path::new("assets/game/sheets/medium_egg.png").exists() {
            eprintln!("skipping: no game sheets");
            return;
        }
        let color = [0.8, 0.3, 0.2];
        // Switched off, the program is the skin shader on the art: the plain recolour the womb's eggs have always had.
        let plain = shaded_egg("medium", color, false, 0.0, 0.0).expect("shader");
        let expected = egg_texture("medium", color, 0.0).expect("recolour");
        // (`egg_texture` is drawn on the finer grid of the turned pictures.)
        let plain_fine = turned(&plain, 0.0);
        assert_eq!((plain_fine.width, plain_fine.height), (expected.width, expected.height));
        // (The shader discards the empty texels, so only the drawn ones are compared.)
        let worst = plain_fine
            .rgba
            .chunks_exact(4)
            .zip(expected.rgba.chunks_exact(4))
            .filter(|(a, b)| a[3] > 0 && b[3] > 0)
            .flat_map(|(a, b)| (0..4).map(move |k| (i32::from(a[k]) - i32::from(b[k])).abs()))
            .max()
            .unwrap_or(0);
        assert!(worst <= 2, "the translated program differs from the skin maths by {worst}/255");
        // Switched on, a scan line crosses the egg: at some moment of its sweep it paints part of it, in another colour when absorbing.
        let at = |production: f32, time: f32| shaded_egg("medium", color, true, production, time).expect("glow");
        let times = [0.0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3];
        assert!(times.iter().any(|t| at(1.0, *t).rgba != plain.rgba), "the glow changes the egg while the line crosses it");
        assert!(times.iter().any(|t| at(1.0, *t).rgba != at(1.0, 0.0).rgba), "the scan line moves");
        assert!(times.iter().any(|t| at(-1.0, *t).rgba != at(1.0, *t).rgba), "absorbing has its own colour");
    }

    #[test]
    fn the_map_has_every_organ_and_the_stomach_is_where_the_calculator_puts_it() {
        let params = captured();
        let map = sections(&params);
        for id in 1..=7u8 {
            assert!(map.contains(&id), "organ {id} is in the map");
        }
        // Outside the body is 0; the stomach's centre cell is the stomach (2).
        assert_eq!(map[0], 0);
        let shape = StomachShape::of(&params);
        let x = shape.offset[0] as usize;
        let y = (shape.offset[1] - 0.5) as usize;
        assert_eq!(map[x + y * WIDTH], 2, "{shape:?}");
        // The material's own numbers for the captured bibite: the stomach is 26 cells long and 27 tall (`_StomachDim` of the `acidLiquid` material).
        assert!((shape.w - 26.0).abs() < 1.0 && (2.0 * shape.r - 27.0).abs() < 2.0, "{shape:?}");
    }

    #[test]
    fn the_pellets_make_up_the_contents_and_stay_in_the_picture() {
        let params = captured();
        let shape = StomachShape::of(&params);
        let info = StomachInfo {
            area: 60.0,
            total: 30.0,
            acid: 0.5,
            bonus: 0.0,
            malus: 0.0,
            effectiveness: 1.0,
            portion: 0.2,
            items: vec![StomachItem {
                material: bibites_core::organs::StomachMaterial::Plant,
                amount: 30.0,
                average_chunk: 0.0,
                digestion_rate: 1.0,
                efficiency: 0.4,
            }],
            dead: false,
        };
        let pellets = pellets(&info, &shape, 7);
        assert!(pellets.len() >= 10, "{}", pellets.len());
        let total: f32 = pellets.iter().map(|p| p.amount).sum();
        assert!((total - 30.0).abs() < 1e-3, "{total}");
        for pellet in &pellets {
            assert!((0.0..305.0).contains(&pellet.pos[0]) && (0.0..195.0).contains(&pellet.pos[1]), "{pellet:?}");
        }
        // The same bibite gives the same places.
        assert_eq!(pellets, super::pellets(&info, &shape, 7));
    }

    #[test]
    fn the_womb_shows_a_whole_egg_per_unit_of_progress_and_a_growing_one() {
        let params = captured();
        let shape = StomachShape::of(&params);
        assert!(eggs_in_womb(0.0, 0.0, 20.0, 300.0, &shape, 1).is_empty());
        let eggs = eggs_in_womb(2.4, 0.4, 20.0, 300.0, &shape, 1);
        assert_eq!(eggs.len(), 3, "ceil(progress) eggs");
        assert!((eggs[0].amount - 20.0).abs() < 1e-5 && (eggs[1].amount - 20.0).abs() < 1e-5);
        assert!((eggs[2].amount - 8.0).abs() < 1e-4, "the last one has 0.4 of an egg");
        assert!(eggs[2].scale < eggs[0].scale);
        // The same bibite gives the same places; the sprite follows sqrt(amount / 100).
        assert_eq!(eggs, eggs_in_womb(2.4, 0.4, 20.0, 300.0, &shape, 1));
        assert_eq!(egg_sheet(100.0).0, "medium");
        assert_eq!(egg_sheet(10.0).0, "tiny");
        assert_eq!(egg_sheet(280.0).0, "large");
    }

    #[test]
    fn dump_pictures() {
        let Ok(dir) = std::env::var("INTERNALS_DUMP") else {
            return;
        };
        let texture = preview(&captured());
        let file = std::fs::File::create(format!("{dir}/preview.png")).unwrap();
        let mut encoder = png::Encoder::new(file, texture.width, texture.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&texture.rgba).unwrap();
        let map = sections(&captured());
        let mut text = String::new();
        for y in (0..HEIGHT).rev() {
            for x in 0..WIDTH {
                text.push(char::from(b'0' + map[x + y * WIDTH]));
            }
            text.push('\n');
        }
        std::fs::write(format!("{dir}/sections.txt"), text).unwrap();
    }
}
