//! The Biology panel's pop-ups of the womb, the mouth, the fat and the body: the pop-up that shows while the pointer rests on that organ's cells of the
//! picture (`BiologyPanel.OnPointerMove`; the stomach's is in `biology_view.rs`).
//!
//! The womb's, mouth's and fat's layouts are the prefab's, measured in the native capture (`biology_popups_data.rs`, written by
//! `tools/ui/gen_biology_popups.py`, with the number format of every text read from the prefab's `FloatValueTextHandle`s) and bound to the selected bibite
//! here by node path. The body's pop-up was never laid out in the capture (every size is 0), so its rows are laid out here from the prefab's order, fonts
//! and the layout groups' 5 pixel spacing.
//!
//! Not modelled: the eggs drawn in the womb of the picture, the pop-ups of the health, energy, growth and age bars, and the slider scales that the game keeps
//! per bibite for the pop-up sliders that raise their own scale (here they are the value of the moment, at least the prefab's start value).

use std::collections::HashMap;

use bibites_core::format::FloatValueFormat;
use bibites_core::math::Vec2;

use crate::app::App;
use crate::biology_popups_data::{self as data, Def, Fmt, Kind, Node};
use crate::biology_view::Painter;
use crate::information_panel::{Face, Measure};
use crate::text_sdf::SdfFonts;
use crate::ui::{FontRole, Rgba, UiAssets, UiFrame};

const INK: Rgba = [33, 34, 33, 255];

/// Which organ's pop-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Organ {
    /// Map id 2.
    Stomach,
    /// Map id 3.
    Womb,
    /// Map ids 4 and 6.
    Mouth,
    /// Map id 7.
    Fat,
    /// Map ids 1 and 5.
    Body,
}

/// `BiologyPanel.OnPointerMove`'s switch on the map's value.
pub fn organ_of_id(id: u8) -> Option<Organ> {
    match id {
        1 | 5 => Some(Organ::Body),
        2 => Some(Organ::Stomach),
        3 => Some(Organ::Womb),
        4 | 6 => Some(Organ::Mouth),
        7 => Some(Organ::Fat),
        _ => None,
    }
}

/// What the pop-ups show of a bibite (`BibiteBody`, `BibiteEggLayingOrgan`, `BibiteMouth`, `BibiteFatOrgan` and the organs' masses).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrganInfo {
    // Womb.
    /// `growth.maturity > 1` (`onceMatureWombSection`).
    pub mature: bool,
    pub womb_area: f32,
    pub womb_portion: f32,
    pub egg_size: f32,
    pub egg_capacity: f32,
    pub egg_float_capacity: f32,
    pub egg_production: f32,
    /// `SetGrowing(body.dead ? 0 : eggProduction)`: the growing egg of the womb glows while it is not 0 (green), orange when it is negative (absorbing).
    pub egg_glow: f32,
    pub egg_energy_rate: f32,
    pub egg_progress_rate: f32,
    pub egg_progress: f32,
    pub n_egg: f32,
    pub eggs_mass: f32,
    pub want2lay: f32,
    pub egg_energy: f32,
    /// `eggProgress` (whole eggs and the part of the next).
    pub egg_total_progress: f32,
    // Mouth.
    pub bite_period: f32,
    pub bite_progress: f32,
    pub swallow_output: f32,
    pub can_swallow: bool,
    pub throat_area: f32,
    pub throat_portion: f32,
    pub swallow_width: f32,
    pub swallow_amount: f32,
    pub no_bite_amount: f32,
    pub attack_output: f32,
    pub can_attack: bool,
    pub jaw_area: f32,
    pub jaw_portion: f32,
    pub jaw_strength: f32,
    pub bite_force: f32,
    // Fat.
    pub energy_ratio: f32,
    pub fat_threshold: f32,
    pub fat_deadband: f32,
    pub fat_activation: f32,
    pub fat_rate: f32,
    pub fat_efficiency: f32,
    pub fat_energy_rate: f32,
    pub fat_amount: f32,
    pub fat_area: f32,
    pub fat_base_capacity: f32,
    pub fat_portion: f32,
    pub fat_inflate: f32,
    pub fat_energy: f32,
    pub fat_sustain: f32,
    pub fat_mass: f32,
    // Health, age, growth and energy pop-ups.
    pub body_energy_density: f32,
    pub blood_energy_density: f32,
    pub heal_rate: f32,
    pub heal_rate_hp: f32,
    pub missing_health_penalty: f32,
    pub age_hours: f32,
    pub aging_threshold: f32,
    pub ageing_factor: f32,
    pub strength_penalty: f32,
    pub metabolism_penalty: f32,
    pub size_factor: f32,
    pub size_gene: f32,
    pub growth: f32,
    pub maturity: f32,
    pub body_length: f32,
    pub metabolic_efficiency: f32,
    /// The ten rates of the Flow details pop-up, in the prefab's order (digestion, movement, growth, metabolism, pheromones, healing, brain, egg production,
    /// fat conversion, fat sustainment), as `BiologyPanel.UpdateBodySection` feeds its rate sliders.
    pub energy_flows: [f32; 10],
    // Body.
    pub body_area: f32,
    pub base_area: f32,
    pub armor_area: f32,
    pub armor_portion: f32,
    pub armor_thickness: f32,
    pub armor_strength: f32,
    pub total_energy: f32,
    pub energy_density: f32,
    pub meat_equivalence: f32,
    pub mass: f32,
    pub base_mass: f32,
    pub armor_mass: f32,
    pub stomach_mass: f32,
    pub move_area: f32,
    pub move_portion: f32,
    pub move_strength: f32,
    pub turn_strength: f32,
}

/// Reads the numbers the pop-ups show from a bibite, as `BiologyPanel` does.
pub fn organ_info_of(record: &bibites_core::body::BibiteRecord, settings: &bibites_core::settings::SimSettings) -> OrganInfo {
    use bibites_core::body::organ_material;
    use bibites_core::brain::Output;
    use bibites_core::genome::Gene;
    let genome = &record.genome;
    let layer = &record.egg_layer;
    let fat_material = organ_material(&settings.materials.fat);
    let armor_material = organ_material(&settings.materials.armor);
    let meat_material = organ_material(&settings.materials.meat);
    let base_area = record.derived.base_body_area;
    let want_eat = record.brain.output_value(Output::Want2Eat);
    let want_attack = record.brain.output_value(Output::Want2Attack);
    let ready = record.mouth.bite_progress >= record.mouth_derived.bite_period;
    let swallow_width = if want_eat > 0.15 {
        record.mouth.desire_eat * record.mouth_derived.throat_width
    } else {
        0.0
    };
    let nominal = layer.progress.floor();
    let fat = &record.fat;
    let armor_area = base_area * record.armor.portion;
    let eggs_mass = settings.scenario.bibite_mass_density * layer.egg_size * layer.progress;
    let extra = fat.amount * settings.materials.fat.energy_density + layer.progress * layer.egg_energy;
    let total_energy = record.body.total_energy(extra);
    let body_area = record.body.geometry.real_body_area;
    let bs = bibites_core::body::BodySettings::from_sim_settings(settings);
    let age_hours = record.clock.time_alive / 3600.0;
    let ageing = bibites_core::body::ageing_factor(age_hours, settings.scenario.ageing_threshold, record.body.params.metabolic_rate);
    let aging_threshold = settings.scenario.ageing_threshold / record.body.params.metabolic_rate;
    let heal_rate = record.body.heal_max_rate(&bs);
    let efficiency = record.body.params.metabolic_efficiency;
    let dead = record.body.dead;
    // `body.dead ? 0f : x`; the rates are written `(0f - x) / efficiency`, which is never negative zero.
    let live = |value: f32| if dead { 0.0 } else { value };
    let energy_flows = [
        live(record.stomach.energy_gain_rate()),
        live((0.0 - record.body.movement_cost) / efficiency),
        live((0.0 - record.growth.growth_cost_rate) / efficiency),
        (0.0 - record.body.metabolism) / efficiency,
        live((0.0 - record.pheromone.cost) / efficiency),
        live((0.0 - record.body.healing_rate_energy_cost) / efficiency),
        live((0.0 - record.derived.brain_upkeep) / efficiency),
        live((0.0 - record.egg_layer.energy_consumption) / efficiency),
        live(record.fat.last_energy_gain_rate),
        (0.0 - record.fat.last_energy_sustain_rate) / efficiency,
    ];
    OrganInfo {
        energy_flows,
        body_energy_density: record.body.params.base_energy_density,
        blood_energy_density: record.body.params.base_energy_density * settings.scenario.health_energy_factor / settings.scenario.health_per_area,
        heal_rate,
        heal_rate_hp: heal_rate * record.body.health.max_amount,
        missing_health_penalty: (1.0 - record.body.health.fullness().max(0.0).sqrt()).abs(),
        age_hours,
        aging_threshold,
        ageing_factor: ageing,
        strength_penalty: bibites_core::body::age_strength_penalty_factor(ageing, &bs) - 1.0,
        metabolism_penalty: bibites_core::body::age_metabolism_penalty_factor(ageing, &bs),
        size_factor: record.body.geometry.d1_size,
        size_gene: genome.gene(Gene::SizeRatio),
        growth: record.growth.growth,
        maturity: record.growth.maturity(&record.growth_genes),
        body_length: record.body.geometry.body_length,
        metabolic_efficiency: record.body.params.metabolic_efficiency,
        mature: record.growth.maturity(&record.growth_genes) >= 1.0,
        womb_area: layer.area,
        womb_portion: genome.womb_area_portion(),
        egg_size: layer.egg_size,
        egg_capacity: layer.capacity as f32,
        egg_float_capacity: if layer.egg_size > 0.0 { layer.area / layer.egg_size } else { 0.0 },
        egg_production: record.brain.output_value(Output::EggProduction),
        egg_glow: live(record.brain.output_value(Output::EggProduction)),
        egg_energy_rate: -layer.energy_consumption,
        egg_progress_rate: layer.progress_rate,
        egg_progress: if nominal as u32 != layer.capacity || layer.capacity == 0 {
            layer.progress % 1.0
        } else {
            1.0
        },
        n_egg: nominal,
        eggs_mass,
        want2lay: record.brain.output_value(Output::Want2Lay),
        egg_energy: layer.egg_energy,
        egg_total_progress: layer.progress,
        bite_period: record.mouth_derived.bite_period,
        bite_progress: record.mouth.bite_progress,
        swallow_output: want_eat,
        can_swallow: want_eat > 0.15 && ready,
        throat_area: record.mouth_derived.throat_width * record.mouth_derived.throat_width,
        throat_portion: genome.organ_area_portion(Gene::ThroatWAG),
        swallow_width,
        swallow_amount: std::f32::consts::PI * (swallow_width / 2.0).powi(2),
        no_bite_amount: std::f32::consts::PI * (swallow_width / 4.0).powi(2),
        attack_output: want_attack,
        can_attack: want_attack > 0.15 && ready,
        jaw_area: record.mouth_derived.jaw_area,
        jaw_portion: genome.organ_area_portion(Gene::MouthMusclesWAG),
        jaw_strength: record.mouth_derived.jaw_strength,
        bite_force: if record.mouth.desire_attack > 0.15 {
            record.mouth_derived.jaw_strength * record.mouth.desire_attack
        } else {
            0.0
        },
        energy_ratio: record.body.energy_ratio(),
        fat_threshold: fat.threshold,
        fat_deadband: fat.deadband,
        fat_activation: fat.last_activation,
        fat_rate: fat.last_amount_rate,
        fat_efficiency: fat_material.conversion_efficiency(1.0 - fat.last_activation.abs().sqrt()),
        fat_energy_rate: fat.last_energy_gain_rate,
        fat_amount: fat.amount,
        fat_area: fat.area,
        fat_base_capacity: fat.base_capacity,
        fat_portion: genome.organ_area_portion(Gene::FatWAG),
        fat_inflate: fat.inflate_factor,
        fat_energy: fat.amount * settings.materials.fat.energy_density,
        fat_sustain: -fat.last_energy_sustain_rate,
        fat_mass: fat.mass(&fat_material),
        body_area,
        base_area,
        armor_area,
        armor_portion: genome.organ_area_portion(Gene::ArmorWAG),
        armor_thickness: record.armor.thickness,
        armor_strength: record.armor.strength,
        total_energy,
        energy_density: if body_area > 0.0 { total_energy / body_area } else { 0.0 },
        meat_equivalence: meat_material.amount_of_energy(total_energy),
        mass: record.derived.mass,
        base_mass: record.body.geometry.base_mass,
        armor_mass: record.armor.mass(armor_area, &armor_material),
        stomach_mass: record.stomach.mass(),
        move_area: base_area * genome.organ_area_portion(Gene::MoveMusclesWAG),
        move_portion: genome.organ_area_portion(Gene::MoveMusclesWAG),
        move_strength: record.propulsion_derived.muscle_move_strength,
        turn_strength: record.propulsion_derived.muscle_turn_strength,
    }
}

/// What a node's value is bound to.
enum Bind {
    /// The text.
    Text(String),
    /// A yes or no in the answer colours (`BoolValueTextHandle`: true (0.088, 0.638, 0), false (0.783, 0.044, 0)).
    Bool(bool),
    /// The fill of a value slider, as `value / scale` (the sign does not matter).
    Ratio(f32),
    /// A marker's place on its slider, as a fraction of the scale.
    Marker(f32),
    /// A rate slider: the value and the scale it is shown at.
    Rate(f32, f32),
}

fn number(fmt: &Fmt, value: f32) -> String {
    FloatValueFormat {
        prefix: fmt.prefix,
        units: fmt.suffix,
        precision: fmt.precision,
        factor: fmt.factor,
        si: fmt.si,
        always_show_sign: fmt.sign,
        precision_is_si: fmt.precision_is_si,
        is_int: fmt.int,
    }
    .format(value)
}

fn yes_no(value: bool) -> Bind {
    Bind::Bool(value)
}

/// The slider scale the game keeps for a pop-up slider that raises its own scale: it starts at `start` for each bibite and becomes any value that
/// reaches it (`InfoSliderHandle.OnValueChange` with the prefab's buffer of 0).
fn scale(key: &'static str, seed: u64, value: f32, start: f32) -> f32 {
    crate::biology_view::remember_scale_pub(key, seed, value, start)
}

/// The womb's bindings, by path under `EggsDetailPopup`.
fn womb(info: &OrganInfo, seed: u64, path: &str, fmt: Option<&Fmt>) -> Option<Bind> {
    let text = |value: f32| fmt.map(|f| Bind::Text(number(f, value)));
    match path {
        "Holder/Mature/BoolInfoText" => Some(yes_no(info.mature)),
        "Holder/Basic Holder/Capacity/ValueIndicator" => text(info.womb_area),
        "Holder/Basic Holder/Capacity/wombPortion" => text(info.womb_portion),
        "Holder/Basic Holder/EggSize/ValueIndicator" => text(info.egg_size),
        "Holder/Basic Holder/EggCapacity/ValueIndicator" => text(info.egg_float_capacity),
        "Holder/Basic Holder/EggCapacity/FlooredCapacity/ValueIndicator" => text(info.egg_capacity),
        "Holder/Once Mature Section/Production Holder/production/ProgressSlider/Fill" => Some(Bind::Ratio(info.egg_production)),
        "Holder/Once Mature Section/Production Holder/production/ProgressSlider/Indicator" => Some(Bind::Marker(0.15)),
        "Holder/Once Mature Section/Production Holder/production/ProgressSlider/FloatInfoText" => text(info.egg_production),
        "Holder/Once Mature Section/Production Holder/Energy consumption/RateSlider/CenterPoint" => Some(Bind::Rate(
            info.egg_energy_rate,
            scale("womb-energy", seed, info.egg_energy_rate, 1.0),
        )),
        "Holder/Once Mature Section/Production Holder/Progress Rate/FloatInfoText" => text(info.egg_progress_rate),
        "Holder/Once Mature Section/Production Holder/Egg Progress/ValueSlider/Fill" => Some(Bind::Ratio(info.egg_progress)),
        "Holder/Once Mature Section/Production Holder/Egg Progress/ValueSlider/FloatInfoText" => text(info.egg_progress),
        "Holder/Once Mature Section/Eggs Stored/ValueIndicator" => text(info.n_egg),
        "Holder/Once Mature Section/Eggs Stored/ValueIndicator#2" => text(info.egg_capacity),
        "Holder/Once Mature Section/Eggs Holder/Egg Energy/ValueIndicator" => text(info.egg_energy),
        "Holder/Once Mature Section/Eggs Holder/Eggs Mass:/ValueIndicator" => text(info.eggs_mass),
        "Holder/Once Mature Section/Eggs Holder/Laying/ProgressSlider/Fill" => Some(Bind::Ratio(info.want2lay)),
        "Holder/Once Mature Section/Eggs Holder/Laying/ProgressSlider/Indicator" => Some(Bind::Marker(0.25)),
        "Holder/Once Mature Section/Eggs Holder/Laying/ProgressSlider/FloatInfoText" => text(info.want2lay),
        _ => None,
    }
}

/// The mouth's bindings, by path under `MouthDetailsPopUp`.
fn mouth(info: &OrganInfo, path: &str, fmt: Option<&Fmt>) -> Option<Bind> {
    let text = |value: f32| fmt.map(|f| Bind::Text(number(f, value)));
    match path {
        "BitePeriod/ValueIndicator" => text(info.bite_period),
        "BiteProgress/ValueSlider/Fill" => Some(Bind::Ratio(if info.bite_period > 0.0 { info.bite_progress / info.bite_period } else { 0.0 })),
        "Holder/Eat Holder/Swallow Output/ValueIndicator" => text(info.swallow_output),
        "Holder/Eat Holder/CanSwallow/BoolText" => Some(yes_no(info.can_swallow)),
        "Holder/Eat Holder/ThroatArea/ValueIndicator" => text(info.throat_area),
        "Holder/Eat Holder/ThroatArea/throat Portion" => text(info.throat_portion),
        "Holder/Eat Holder/MouthOpening/ValueIndicator" => text(info.swallow_width),
        "Holder/Eat Holder/MouthOpening2/ValueIndicator" => text(info.swallow_amount),
        "Holder/Eat Holder/FilterFeedingSize/ValueIndicator" => text(info.no_bite_amount),
        "Holder/AttackHolder/Attack Output/ValueIndicator" => text(info.attack_output),
        "Holder/AttackHolder/CanAttack/BoolText" => Some(yes_no(info.can_attack)),
        "Holder/AttackHolder/JawArea/ValueIndicator" => text(info.jaw_area),
        "Holder/AttackHolder/JawArea/jawPortion" => text(info.jaw_portion),
        "Holder/AttackHolder/MouthStrength/ValueIndicator" => text(info.jaw_strength),
        "Holder/AttackHolder/AttackStrength/ValueIndicator" => text(info.bite_force),
        _ => None,
    }
}

/// The fat's bindings, by path under `FatDetailsPopup`.
fn fat(info: &OrganInfo, seed: u64, path: &str, fmt: Option<&Fmt>) -> Option<Bind> {
    let text = |value: f32| fmt.map(|f| Bind::Text(number(f, value)));
    let area = info.fat_area.max(1e-6);
    match path {
        "Holder/Sliders Holder/energyRatio/DeadbandSlider/Fill" => Some(Bind::Ratio(info.energy_ratio)),
        "Holder/Sliders Holder/energyRatio/DeadbandSlider/Indicator" => Some(Bind::Marker(info.fat_threshold - info.fat_deadband)),
        "Holder/Sliders Holder/energyRatio/DeadbandSlider/Indicator (1)" => {
            Some(Bind::Marker((info.fat_threshold + info.fat_deadband).min(0.99)))
        }
        "Holder/Sliders Holder/energyRatio/DeadbandSlider/FloatInfoText" => text(info.energy_ratio),
        "Holder/Sliders Holder/Activation/fatActivation/CenterPoint" => {
            Some(Bind::Rate(info.fat_activation, scale("fat-activation", seed, info.fat_activation, 1.0)))
        }
        "Holder/Sliders Holder/Production/fatProduction/CenterPoint" => {
            Some(Bind::Rate(info.fat_rate, scale("fat-production", seed, info.fat_rate, 0.05)))
        }
        "Holder/Sliders Holder/Efficiency/fatEficiency/Fill" => Some(Bind::Ratio(info.fat_efficiency)),
        "Holder/Sliders Holder/Efficiency/fatEficiency/holder/stored" => text(info.fat_efficiency),
        "Holder/Sliders Holder/EnergyProduction/fatEnergy/CenterPoint" => {
            Some(Bind::Rate(info.fat_energy_rate, scale("fat-energy", seed, info.fat_energy_rate, 1.0)))
        }
        "Holder/Stored section/Stored/TitleHolder/fatCap" => text(info.fat_area),
        "Holder/Stored section/Stored/TitleHolder/fatPortion" => text(info.fat_portion),
        "Holder/Stored section/Stored/fatReserves/Fill" => Some(Bind::Ratio(info.fat_amount / area)),
        "Holder/Stored section/Stored/fatReserves/holder/stored" => text(info.fat_amount),
        "Holder/Stored section/Stored/fatReserves/holder/Capacity" => text(info.fat_area),
        "Holder/Stored section/InflateRatio/HeaderHolder/inflateFactorText" => text(info.fat_inflate * 0.35),
        "Holder/Stored section/InflateRatio/inflateSlider/Fill" => Some(Bind::Ratio(info.fat_amount / area)),
        "Holder/Stored section/InflateRatio/inflateSlider/Indicator" => Some(Bind::Marker(info.fat_base_capacity / area)),
        "Holder/Stored section/TotalEnergy/totalFatEnergyText" => text(info.fat_energy),
        "Holder/Stored section/Sustain/fatSustain/CenterPoint" => {
            Some(Bind::Rate(info.fat_sustain, scale("fat-sustain", seed, info.fat_sustain, 0.05)))
        }
        "Holder/Stored section/Mass/fatMassText" => text(info.fat_mass),
        _ => None,
    }
}

/// The health pop-up's bindings, by path under `HealthDetailsPopUp`.
fn health(info: &OrganInfo, path: &str, fmt: Option<&Fmt>) -> Option<Bind> {
    let text = |value: f32| fmt.map(|f| Bind::Text(number(f, value)));
    match path {
        "BaseEnergyDensityCost/ValueIndicator" => text(info.body_energy_density),
        "BloodEnergyDensity/ValueIndicator" => text(info.blood_energy_density),
        "heal holder/HealRate/HealRate" => text(info.heal_rate),
        "heal holder/HealRateHP/HealRateHP" => text(info.heal_rate_hp),
        "HealthImpactsHolder/MovementPenalty/ValueIndicator" => text(info.missing_health_penalty),
        _ => None,
    }
}

/// The energy bar's pop-up (`EnergyInfoPopUp`).
fn energy_info(info: &OrganInfo, path: &str, fmt: Option<&Fmt>) -> Option<Bind> {
    let text = |value: f32| fmt.map(|f| Bind::Text(number(f, value)));
    match path {
        "InfoHolder/Metabolic Efficiency/ValueIndicator" => text(info.metabolic_efficiency),
        "InfoHolder/EggCost/ValueIndicator" => text(info.egg_energy),
        _ => None,
    }
}

/// The growth bar's pop-up (`MaturityDetailsPopUp`).
fn growth(info: &OrganInfo, path: &str, fmt: Option<&Fmt>) -> Option<Bind> {
    let text = |value: f32| fmt.map(|f| Bind::Text(number(f, value)));
    match path {
        "SizeHolder/FloatInfoText" => text(info.size_factor),
        "Size Infos/Size Gene/ValueIndicator" => text(info.size_gene),
        "Size Infos/Growth/ValueIndicator" => text(info.growth),
        "Size Infos/Size(1D)/ValueIndicator" => text(info.body_length),
        "Size Infos/Size(2D)/ValueIndicator" => text(info.base_area),
        "MaturityTitle/BoolInfoText" | "CanLayEggsMaturity/BoolText" => Some(yes_no(info.mature)),
        "Maturity/ValueIndicator" => text(info.maturity),
        _ => None,
    }
}

/// The age bar's pop-up (`AgeDetailsPopUp`).
fn age(info: &OrganInfo, path: &str, fmt: Option<&Fmt>) -> Option<Bind> {
    let text = |value: f32| fmt.map(|f| Bind::Text(number(f, value)));
    match path {
        "TitleHolder/ValueIndicator" => text(info.age_hours),
        "Holder/AgeingThreshold/ValueIndicator" => text(info.aging_threshold),
        "Holder/AgeingFactorMultiplicator/ValueIndicator" => text(info.ageing_factor),
        "Holder/StrengthPenalty/ValueIndicator" => text(info.strength_penalty),
        "Holder/MetabolismPenalty/ValueIndicator" => text(info.metabolism_penalty),
        _ => None,
    }
}

/// The pop-ups of the body section's bars: the pointer on the row opens it (`OnHoverShow`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarPopup {
    /// The Health bar.
    Health,
    /// The Energy bar.
    Energy,
    /// The Growth bar.
    Growth,
    /// The Age bar.
    Age,
}

/// How far the body section moves down when the "This bibite is DEAD" line shows above it (its 11.16 and the layout's 15).
pub const DEAD_LINE_SHIFT: f32 = 26.16;
/// How far what is under the Energy row moves up when a dead bibite hides it (24 and the group's 10).
pub const ENERGY_ROW_PITCH: f32 = 34.0;

/// The bar row under a point of the reference screen (`x` 25..399, rows 24 high), with the middle of the row. A dead bibite has no Energy row, the section
/// starts lower and the rows under the Energy row move up.
pub fn bar_at(reference: Vec2, dead: bool) -> Option<(BarPopup, f32)> {
    if !(25.0..399.0).contains(&reference.x) {
        return None;
    }
    let rows: Vec<(BarPopup, f32)> = if dead {
        let d = DEAD_LINE_SHIFT;
        vec![
            (BarPopup::Health, 143.5 + d),
            (BarPopup::Growth, 245.5 + d - ENERGY_ROW_PITCH),
            (BarPopup::Age, 279.5 + d - ENERGY_ROW_PITCH),
        ]
    } else {
        vec![
            (BarPopup::Health, 143.5),
            (BarPopup::Energy, 177.5),
            (BarPopup::Growth, 245.5),
            (BarPopup::Age, 279.5),
        ]
    };
    rows.iter()
        .find(|(_, top)| reference.y >= *top && reference.y < *top + 24.0)
        .map(|(bar, top)| (*bar, *top + 12.0))
}

/// `ProgressSliderHandle` / `ValueSliderHandle`: the pixel length of the sprite (`1 / ppu`).
fn pixel_length(w: f32, h: f32) -> f32 {
    1.0 / 0.5_f32.max(8.0 / w.max(1.0)).max(12.0 / h.max(1.0))
}

/// Draws a data-driven pop-up with its top left at (`x`, `y`) and the height the visible nodes need; returns the height.
fn draw_def(
    p: &mut Painter<'_>,
    def: &Def,
    top_left: (f32, f32),
    center_y: f32,
    visibility: &dyn Fn(&str) -> Option<bool>,
    bind: &dyn Fn(&str, Option<&Fmt>) -> Option<Bind>,
) {
    let by_path: HashMap<&str, &Node> = def.nodes.iter().map(|n| (n.path, n)).collect();
    // Which nodes show: the prefab's flag, a pop-up's own say, and every ancestor.
    let mut off: Vec<&str> = Vec::new();
    let mut visible: Vec<&Node> = Vec::new();
    let mut bottom = 0.0f32;
    for node in def.nodes.iter().skip(1) {
        if off.iter().any(|prefix| node.path.starts_with(&format!("{prefix}/"))) {
            continue;
        }
        let name = node.path.rsplit('/').next().unwrap_or("");
        let slider_part = matches!(name, "PositiveFill" | "NegativeFill" | "PositiveRateText" | "NegativeRateText" | "divider");
        let own = if slider_part { true } else { visibility(node.path).unwrap_or(node.shown) };
        if !own {
            off.push(node.path);
            continue;
        }
        bottom = bottom.max(node.y + node.h);
        visible.push(node);
    }
    let height = bottom + def.pad_bottom;
    let (x0, _) = top_left;
    let y0 = center_y - height / 2.0;
    remember_rect([x0, y0, def.w, height]);
    p.slice("base_icon", [5.0, 5.0, 5.0, 5.0], 1.0, [x0, y0, def.w, height], [255, 255, 255, 255]);
    let mut skipped: Vec<&str> = Vec::new();
    for node in visible {
        // Parts of a rate slider are drawn with its `CenterPoint`.
        let parent = node.path.rsplit_once('/').map_or("", |(parent, _)| parent);
        if skipped.contains(&parent) {
            continue;
        }
        let rect = [x0 + node.x, y0 + node.y, node.w, node.h];
        let name = node.path.rsplit('/').next().unwrap_or("");
        match node.kind {
            Kind::Group => {
                if name == "CenterPoint"
                    && let Some(Bind::Rate(value, scale)) = bind(node.path, None)
                    && let Some(slider) = by_path.get(parent)
                {
                    skipped.push(node.path);
                    let slider_rect = [x0 + slider.x, y0 + slider.y, slider.w, slider.h];
                    let pl = pixel_length(slider.w, slider.h);
                    let positive_px = by_path
                        .get(format!("{}/PositiveRateText", node.path).as_str())
                        .and_then(|n| if let Kind::Text { px, .. } = n.kind { Some(px) } else { None })
                        .unwrap_or(14.3);
                    let negative_px = by_path
                        .get(format!("{}/NegativeRateText", node.path).as_str())
                        .and_then(|n| if let Kind::Text { px, .. } = n.kind { Some(px) } else { None })
                        .unwrap_or(11.9);
                    let text_fmt = by_path
                        .get(format!("{}/PositiveRateText", node.path).as_str())
                        .and_then(|n| if let Kind::Text { fmt, .. } = n.kind { fmt } else { None });
                    let shown = text_fmt.map(|f| number(&f, value));
                    p.rate_slider_pl(slider_rect, value, scale, pl, shown.as_deref(), positive_px, negative_px);
                }
            }
            Kind::Image { sprite, tint } => {
                if name == "CenterPoint" {
                    continue;
                }
                let slider = by_path.get(parent);
                let pl = slider.map_or(2.0, |s| pixel_length(s.w, s.h));
                match sprite {
                    "tiled_bars_1" => {
                        p.slice("tiled_bars_1", [4.0, 5.0, 4.0, 5.0], pl, rect, [255, 255, 255, 255]);
                    }
                    "tiled_bars_0" if name == "Fill" => {
                        let ratio = match bind(node.path, None) {
                            Some(Bind::Ratio(ratio)) => ratio.abs(),
                            _ => 0.0,
                        };
                        let track = slider.map_or(rect[2], |s| s.w);
                        let fill = 8.0 * pl + ratio.min(1.0) * (track - 8.0 * pl);
                        p.slice("tiled_bars_0", [4.0, 5.0, 4.0, 5.0], pl, [rect[0], rect[1], fill, rect[3]], [tint[0], tint[1], tint[2], tint[3]]);
                    }
                    "" if name.starts_with("Indicator") => {
                        if let (Some(Bind::Marker(fraction)), Some(slider)) = (bind(node.path, None), slider) {
                            let track = slider.w;
                            let at = 8.0 * pl + fraction.clamp(0.0, 1.0) * (track - 8.0 * pl) - 4.0 * pl;
                            let mid = y0 + slider.y + slider.h / 2.0;
                            p.frame_rect([x0 + slider.x + at, mid - 2.0 * pl, 2.0 * pl, 4.0 * pl], [tint[0], tint[1], tint[2], 255]);
                        }
                    }
                    _ => {}
                }
            }
            Kind::Text { text, font, px, align, color, bold, fmt } => {
                let mut answer: Option<Rgba> = None;
                let shown = match bind(node.path, fmt.as_ref()) {
                    Some(Bind::Text(value)) => value,
                    Some(Bind::Bool(value)) => {
                        answer = Some(if value { [22, 163, 0, 255] } else { [200, 11, 0, 255] });
                        if value { "Yes" } else { "No" }.to_owned()
                    }
                    // Labels keep their prefab text; a number the pop-up has no value for is left out rather than shown stale.
                    _ if fmt.is_none() => text.to_owned(),
                    _ => continue,
                };
                let role = if bold { FontRole::Bold } else { FontRole::Body };
                let face = Face { name: font, role, line: 1.171_875 };
                // The white `LiberationSans SDF` labels are drawn dark (the material's face colour is black).
                let ink: Rgba = if font == "LiberationSans SDF" && color != [0, 0, 0] {
                    INK
                } else {
                    [color[0], color[1], color[2], 255]
                };
                p.text(face, &shown, rect, align, px, answer.unwrap_or(ink));
            }
        }
    }
}

/// The body's rows: (label, size of the label, the value's text, an optional share, the indent, a gutter sign).
struct Row {
    label: &'static str,
    px: f32,
    value: String,
    share: Option<String>,
    indent: f32,
    sign: Option<&'static str>,
}

fn body_rows(info: &OrganInfo) -> Vec<Option<Row>> {
    let value = |units: &'static str, precision: i32, si: bool, v: f32| {
        FloatValueFormat { units, precision, si, ..FloatValueFormat::default() }.format(v)
    };
    let share = |v: f32| {
        FloatValueFormat { prefix: "(", units: "%)", precision: 1, factor: 100.0, si: false, ..FloatValueFormat::default() }.format(v)
    };
    let row = |label, px, value, share, indent, sign| Some(Row { label, px, value, share, indent, sign });
    vec![
        row("Body Size:", 18.0, value("u²", 2, true, info.body_area), None, 0.0, None),
        row("Base Size:", 16.0, value("u²", 2, true, info.base_area), None, 22.0, Some("=")),
        row("Inflation Factor:", 16.0, value("", 2, true, info.fat_inflate * 0.35 + 1.0), None, 22.0, Some("x")),
        row("Total Body Energy:", 16.0, value("E", 2, true, info.total_energy), None, 22.0, None),
        row("Energy Density:", 16.0, value("E/u²", 1, false, info.energy_density), None, 22.0, None),
        row("Meat Equivalence:", 16.0, value("u²", 2, true, info.meat_equivalence), None, 22.0, None),
        None,
        row("Armor Area:", 18.0, value("u²", 2, true, info.armor_area), Some(share(info.armor_portion)), 0.0, None),
        row("Armor Thickness:", 16.0, value("u", 1, true, info.armor_thickness), None, 22.0, None),
        row("Armor Strength:", 16.0, value("N", 1, true, info.armor_strength), None, 22.0, None),
        None,
        row("Bibite Mass:", 18.0, value("g", 2, true, info.mass), None, 0.0, None),
        row("Base Mass:", 16.0, value("g", 2, true, info.base_mass), None, 22.0, Some("=")),
        row("Armor Mass:", 16.0, value("g", 2, true, info.armor_mass), None, 22.0, Some("+")),
        row("Stomach Mass:", 16.0, value("g", 2, true, info.stomach_mass), None, 22.0, Some("+")),
        row("Eggs Mass:", 16.0, value("g", 2, true, info.eggs_mass), None, 22.0, Some("+")),
        row("Fat Mass:", 16.0, value("g", 2, true, info.fat_mass), None, 22.0, Some("+")),
        None,
        row("Muscles Area:", 18.0, value(" u²", 2, true, info.move_area), Some(share(info.move_portion)), 0.0, None),
        row("Propel Strength:", 16.0, value("N", 1, true, info.move_strength), None, 22.0, None),
        row("Turning Torque:", 16.0, value("Nu", 1, true, info.turn_strength), None, 22.0, None),
    ]
}

fn draw_body(p: &mut Painter<'_>, info: &OrganInfo, x0: f32, center_y: f32) {
    use crate::information_panel::ROBOTO_BOLD;
    let liberation_bold = Face { name: "LiberationSans SDF", role: FontRole::Bold, line: 1.117_5 };
    let rows = body_rows(info);
    // The pop-up is 284.1 wide with 10 around; the rows sit 5 apart, a separator is 5 high.
    let width = 284.1;
    let mut height = 10.0 + 22.35 + 5.0;
    for row in &rows {
        height += match row {
            Some(row) => (row.px * 1.117_5).ceil() + 5.0,
            None => 5.0 + 5.0,
        };
    }
    height += 5.0;
    let y0 = center_y - height / 2.0;
    p.slice("base_icon", [5.0, 5.0, 5.0, 5.0], 1.0, [x0, y0, width, height], [255, 255, 255, 255]);
    p.text(liberation_bold, "Body Details:", [x0 + 10.0, y0 + 10.0, 264.0, 22.35], -1, 20.0, INK);
    let mut y = y0 + 10.0 + 22.35 + 5.0;
    for row in rows {
        let Some(row) = row else {
            p.frame_rect([x0 + 10.0, y + 4.0, width - 20.0, 1.0], [38, 38, 38, 255]);
            y += 10.0;
            continue;
        };
        let line = (row.px * 1.117_5).ceil();
        let left = x0 + 10.0 + row.indent;
        p.text(liberation_bold, row.label, [left, y, 150.0, line], -1, row.px, INK);
        if let Some(sign) = row.sign {
            p.text(ROBOTO_BOLD, sign, [x0 + 10.0, y, 12.0, line], 0, 14.0, [41, 41, 41, 255]);
        }
        let right = x0 + width - 10.0;
        let share_w = if row.share.is_some() { 53.0 } else { 0.0 };
        if let Some(share) = &row.share {
            p.text(ROBOTO_BOLD, share, [right - 48.2, y, 48.2, line], 1, 14.0, [56, 56, 56, 255]);
        }
        p.text(ROBOTO_BOLD, &row.value, [right - share_w - 120.0, y, 120.0, line], 1, row.px, [0, 0, 0, 255]);
        y += line + 5.0;
    }
}

/// The Flow bar's row (`EnergyRateSliderHolder`, 24 high from x 25 to 399) and where its pop-up hangs (`EnergyDetailsPopUp`: 9 right of the row, centred on it).
const FLOW_ROW_TOP: f32 = 211.5;
const FLOW_POPUP_W: f32 = 424.0;
/// The height of one of its rows (the layout group's measure) and of the whole pop-up (10 + 20 + 5 + 10 rows 5 apart + 8).
const FLOW_ROW_H: f32 = 17.439;
const FLOW_H: f32 = 10.0 + 20.0 + 5.0 + 10.0 * FLOW_ROW_H + 9.0 * 5.0 + 8.0;
/// The rows of the pop-up in the prefab's order.
const FLOW_ROWS: [&str; 10] = [
    "Digestion:",
    "Movement:",
    "Growth:",
    "Metabolism:",
    "Pheromones:",
    "Healing:",
    "Brain:",
    "Egg Production:",
    "Fat Conversion:",
    "Fat Sustainment:",
];
// The Flow pop-up's click areas: the block 9000..=9001 (it hangs over the Information panel, which has 8800.., and shares no id with it: a click on
// the Sort toggle closed that panel when both were on 8801).
const HIT_FLOW_PANEL: u32 = 9000;
const HIT_FLOW_SORT: u32 = 9001;

/// The middle of the Flow row when the pointer is on it (`EnergyRateSliderHolder`). A dead bibite's rows are higher by what its missing Energy row and
/// extra line add up to.
pub fn flow_row_at(reference: Vec2, dead: bool) -> Option<f32> {
    let top = if dead { FLOW_ROW_TOP + DEAD_LINE_SHIFT - ENERGY_ROW_PITCH } else { FLOW_ROW_TOP };
    ((25.0..399.0).contains(&reference.x) && (top..top + 24.0).contains(&reference.y)).then_some(top + 12.0)
}

/// What a bar row's pop-up hangs on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HoverKey {
    /// The Health, Energy, Growth or Age bar.
    Bar(BarPopup),
    /// The Flow bar.
    Flow,
}

/// `OnHoverShow` of the bar rows (`openDelay` and `closeDelay` 0.3 s, real time) with `OnHoverKeep` on the pop-ups: the pop-up opens once the pointer has
/// rested 0.3 s on its row and closes 0.3 s after the pointer left both the row and the pop-up.
#[derive(Clone, Copy, Debug, Default)]
pub struct BarHover {
    target: Option<(HoverKey, f32)>,
    held: f32,
    /// The pop-up that shows, with the middle of its row.
    pub shown: Option<(HoverKey, f32)>,
    away: f32,
}

const HOVER_DELAY: f32 = 0.3;

thread_local! {
    static LAST_RECT: std::cell::Cell<Option<[f32; 4]>> = const { std::cell::Cell::new(None) };
}

fn remember_rect(rect: [f32; 4]) {
    LAST_RECT.with(|cell| cell.set(Some(rect)));
}

impl BarHover {
    /// One frame: `reference` is the pointer on the 1920 x 1080 canvas, `active` whether the Biology panel shows a bibite's bars.
    pub fn update(&mut self, reference: Vec2, dead: bool, active: bool, dt: f32) {
        if !active {
            *self = Self::default();
            return;
        }
        let row = bar_at(reference, dead)
            .map(|(bar, center)| (HoverKey::Bar(bar), center))
            .or_else(|| flow_row_at(reference, dead).map(|center| (HoverKey::Flow, center)));
        let in_popup = self.shown.is_some()
            && LAST_RECT.with(|cell| cell.get()).is_some_and(|r| {
                reference.x >= r[0] && reference.x < r[0] + r[2] && reference.y >= r[1] && reference.y < r[1] + r[3]
            });
        let target = row.or(if in_popup { self.shown } else { None });
        if target.map(|t| t.0) == self.target.map(|t| t.0) {
            self.held += dt;
        } else {
            self.held = dt;
        }
        self.target = target;
        match target {
            Some(t) => {
                if self.shown.map(|s| s.0) == Some(t.0) {
                    self.away = 0.0;
                    self.shown = Some(t);
                } else if self.held >= HOVER_DELAY {
                    self.shown = Some(t);
                    self.away = 0.0;
                }
            }
            None => {
                if self.shown.is_some() {
                    self.away += dt;
                    if self.away >= HOVER_DELAY {
                        self.shown = None;
                    }
                }
            }
        }
    }
}

/// `EnergyDetailsPopUp`: the ten rate sliders with the shared scale (it starts at 0.25 for each bibite and becomes any value that reaches it) in the prefab's
/// order, or sorted by value when the Sort toggle is on. Returns the checkbox's rectangle.
fn draw_flow(p: &mut Painter<'_>, info: &OrganInfo, sort: bool, x0: f32, center_y: f32) -> [f32; 4] {
    use crate::information_panel::ROBOTO_BOLD;
    let bold = Face { name: "LiberationSans SDF", role: FontRole::Bold, line: 1.117_5 };
    let y0 = center_y - FLOW_H / 2.0;
    remember_rect([x0, y0, FLOW_POPUP_W, FLOW_H]);
    p.slice("base_icon", [5.0, 5.0, 5.0, 5.0], 1.0, [x0, y0, FLOW_POPUP_W, FLOW_H], [255, 255, 255, 255]);
    // The title line: the title and the toggle (a 20 square, 10 away from its label, 153 wide with the label's 123) share what is left equally.
    let title_w = p.measure.width(bold, "Energy Flow Details:", 18.0);
    let toggle_w = 20.0 + 10.0 + 123.0;
    let toggle_x = x0 + 10.0 + title_w + (404.0 - title_w - toggle_w) / 2.0;
    p.text(bold, "Energy Flow Details:", [x0 + 10.0, y0 + 10.0, title_w, 20.0], -1, 18.0, INK);
    let check = [toggle_x, y0 + 10.0, 20.0, 20.0];
    p.slice("base_icon", [5.0, 5.0, 5.0, 5.0], 0.5, check, [255, 255, 255, 255]);
    if sort {
        p.icon("checkmark_icon", check);
    }
    p.text(bold, "Sort", [toggle_x + 30.0, y0 + 10.0, 123.0, 20.0], -1, 18.0, INK);
    // The shared scale, then the rows.
    let seed = p.seed;
    let mut scale = 0.25f32;
    for value in info.energy_flows {
        scale = crate::biology_view::remember_scale_pub("energy-details", seed, value, 0.25).max(scale);
    }
    let mut order: Vec<usize> = (0..10).collect();
    if sort {
        order.sort_by(|a, b| info.energy_flows[*b].partial_cmp(&info.energy_flows[*a]).unwrap_or(std::cmp::Ordering::Equal));
    }
    let value_format = FloatValueFormat { units: "E/s", precision: 2, si: false, ..FloatValueFormat::default() };
    let pl = pixel_length(219.0, FLOW_ROW_H);
    for (place, index) in order.into_iter().enumerate() {
        let top = y0 + 10.0 + 20.0 + 5.0 + place as f32 * (FLOW_ROW_H + 5.0);
        let value = info.energy_flows[index];
        p.text(bold, FLOW_ROWS[index], [x0 + 10.0, top, 110.0, FLOW_ROW_H], -1, 16.0, INK);
        p.rate_slider_pl([x0 + 125.0, top, 219.0, FLOW_ROW_H], value, scale, pl, None, 14.3, 11.9);
        p.text(ROBOTO_BOLD, &value_format.format(value), [x0 + 349.0, top, 65.0, FLOW_ROW_H], 1, 14.0, [0, 0, 0, 255]);
    }
    check
}

/// The Flow bar's pop-up, with the pop-up swallowing clicks and its Sort toggle.
pub fn draw_flow_popup(frame: &mut UiFrame, app: &App, ui: &UiAssets, metrics: Option<&SdfFonts>, width: f32, height: f32, center: f32) {
    let Some(info) = app.selection.as_ref().and_then(|panel| panel.organs.as_ref()) else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let seed = crate::biology_view::selection_seed(app);
    let x = 25.0 + 374.0 + 9.0;
    let check = {
        let mut p = Painter::new(frame, ui, s, Measure { fonts: metrics }, seed);
        draw_flow(&mut p, info, app.sort_energy_details, x, center)
    };
    let at = |x: f32, y: f32| Vec2::new(x * s, y * s);
    frame.hit(HIT_FLOW_PANEL, at(x, center - FLOW_H / 2.0), at(FLOW_POPUP_W, FLOW_H), crate::ui::UiAction::Consume);
    frame.hit(HIT_FLOW_SORT, at(check[0], check[1]), at(153.0, 20.0), crate::ui::UiAction::ToggleEnergySort);
}

/// The pop-up of `organ` for the selected bibite, hanging to the right of the panel's `Section`.
pub fn draw(frame: &mut UiFrame, app: &App, ui: &UiAssets, metrics: Option<&SdfFonts>, width: f32, height: f32, organ: Organ) {
    let Some(info) = app.selection.as_ref().and_then(|panel| panel.organs.as_ref()) else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let seed = crate::biology_view::selection_seed(app);
    let mut p = Painter::new(frame, ui, s, Measure { fonts: metrics }, seed);
    // Right of the `Section` (374 from the picture's left), 9 away, on its middle (3.7 below it in these pop-ups).
    let picture = crate::biology_view::picture(app.selection.as_ref().is_some_and(|panel| panel.dead));
    let x = picture[0] + 374.0 + 9.0;
    let center = picture[1] + picture[3] / 2.0 - 3.71;
    match organ {
        Organ::Womb => {
            // `UpdateIsMature`: the section of what a mature womb does shows once the bibite is mature.
            let visibility = |path: &str| (path == "Holder/Once Mature Section").then_some(info.mature);
            draw_def(&mut p, &data::WOMB, (x, 0.0), center, &visibility, &|path, fmt| womb(info, seed, path, fmt));
        }
        Organ::Mouth => draw_def(&mut p, &data::MOUTH, (x, 0.0), center, &|_| None, &|path, fmt| mouth(info, path, fmt)),
        Organ::Fat => draw_def(&mut p, &data::FAT, (x, 0.0), center, &|_| None, &|path, fmt| fat(info, seed, path, fmt)),
        Organ::Body => draw_body(&mut p, info, x, center),
        Organ::Stomach => {}
    }
    let _ = Vec2::ZERO;
}

/// The pop-up of a bar, hanging to the right of its row (9 from the panel's body section) on the row's middle.
pub fn draw_bar(frame: &mut UiFrame, app: &App, ui: &UiAssets, metrics: Option<&SdfFonts>, width: f32, height: f32, bar: BarPopup, center: f32) {
    let Some(info) = app.selection.as_ref().and_then(|panel| panel.organs.as_ref()) else {
        return;
    };
    let s = UiFrame::scaler(width, height);
    let seed = crate::biology_view::selection_seed(app);
    let mut p = Painter::new(frame, ui, s, Measure { fonts: metrics }, seed);
    let x = 25.0 + 374.0 + 9.0;
    let none = |_: &str| None;
    match bar {
        BarPopup::Health => draw_def(&mut p, &data::HEALTH, (x, 0.0), center, &none, &|path, fmt| health(info, path, fmt)),
        BarPopup::Energy => draw_def(&mut p, &data::ENERGY_INFO, (x, 0.0), center, &none, &|path, fmt| energy_info(info, path, fmt)),
        BarPopup::Growth => draw_def(&mut p, &data::GROWTH, (x, 0.0), center, &none, &|path, fmt| growth(info, path, fmt)),
        BarPopup::Age => draw_def(&mut p, &data::AGE, (x, 0.0), center, &none, &|path, fmt| age(info, path, fmt)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_of_the_map_values_is_the_games() {
        assert_eq!(organ_of_id(0), None);
        assert_eq!(organ_of_id(1), Some(Organ::Body));
        assert_eq!(organ_of_id(2), Some(Organ::Stomach));
        assert_eq!(organ_of_id(3), Some(Organ::Womb));
        assert_eq!(organ_of_id(4), Some(Organ::Mouth));
        assert_eq!(organ_of_id(5), Some(Organ::Body));
        assert_eq!(organ_of_id(6), Some(Organ::Mouth));
        assert_eq!(organ_of_id(7), Some(Organ::Fat));
    }

    #[test]
    fn a_text_is_written_by_the_prefabs_number_format() {
        let fmt = Fmt {
            prefix: "(",
            suffix: "%)",
            precision: 1,
            factor: 100.0,
            si: false,
            sign: false,
            precision_is_si: false,
            int: false,
        };
        assert_eq!(number(&fmt, 0.187), "(18.7%)");
    }

    #[test]
    fn the_sliders_pixel_length_follows_their_height() {
        // 24 high (the Biology bars): 2 pixels per sprite pixel; 16 high (the pop-up bars): 4/3.
        assert!((pixel_length(250.0, 24.0) - 2.0).abs() < 1e-5);
        assert!((pixel_length(233.0, 16.0) - 4.0 / 3.0).abs() < 1e-5);
    }
}
