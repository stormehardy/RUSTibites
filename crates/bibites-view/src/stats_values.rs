//! The numbers of the inspector's Stats panel, written the way the game writes them.
//!
//! Every value of the panel is a `FloatValueTextHandle`; the prefab serializes its format (prefix, suffix, precision, factor and the
//! `contract` / `alwaysShowSign` / `precisionIsSI` / `isInt` switches, read from level2 with `tools/unity/...`), and
//! `OnValueChange` prints `value.ScientificFormat(precision, suffix, alwaysShowSign, SIUnits: contract, prefixWithUnits: !isInt,
//! scientificPrecision: precisionIsSI && !isInt, isInt: isInt)`. With `contract` on the number takes the SI prefix of its own decade
//! (a bibite that moves at 0.5 u/s shows "500.00 mu/s", an area of 1234 u² "1.23 ku²"); off, it is plain digits and the unit ("725s",
//! "0.0HP"). The viewer used to print `{:.2} u` for all of them and a made-up "12m 5s" for the time alive.
//!
//! The egg section (`BibiteStatsPanel.UpdateEggStats`) has two bars, `ValueSliderHandle`s: [`slider_fill_width`] is their fill.

use bibites_core::format::FloatValueFormat;
use bibites_core::math::Vec2;

use crate::app::SelectionPanel;

/// A `FloatValueTextHandle` with no prefix, a factor of 1 and no forced sign (what the Stats panel's handles are).
const fn handle(
    units: &'static str,
    precision: i32,
    contract: bool,
    is_int: bool,
) -> FloatValueFormat {
    FloatValueFormat {
        prefix: "",
        units,
        precision,
        factor: 1.0,
        si: contract,
        always_show_sign: false,
        precision_is_si: false,
        is_int,
    }
}

/// `Generation` (and the egg's): plain digits.
pub const GENERATION: FloatValueFormat = handle("", 0, false, false);
/// `Lenght` (the prefab's spelling): `bodyLength`.
pub const LENGTH: FloatValueFormat = handle("u", 2, true, false);
/// `Size` of a bibite (`realBodyArea`) and of an egg (`eggArea`).
pub const AREA: FloatValueFormat = handle("u\u{b2}", 2, true, false);
/// `Mass`: bibite and egg.
pub const MASS: FloatValueFormat = handle("g", 2, true, false);
/// `Speed`: `SignedVelocity`.
pub const SPEED: FloatValueFormat = handle("u/s", 2, true, false);
/// `Time Alive`: whole seconds, no SI prefix, no minutes.
pub const TIME_ALIVE: FloatValueFormat = handle("s", 0, false, false);
/// `Distance Traveled`.
pub const DISTANCE: FloatValueFormat = handle("u", 2, true, false);
/// `Number of eggs` and `Alive Children`: integers.
pub const COUNT_INT: FloatValueFormat = handle("", 0, false, true);
/// `BibitesBitten` and `Times Attacked`: digits.
pub const COUNT: FloatValueFormat = handle("", 0, false, false);
/// `TotalDamagesDealt` and `Total Damages Suffered`.
pub const DAMAGE: FloatValueFormat = handle("HP", 1, false, false);
/// The egg's `Total energy` row.
pub const EGG_ENERGY: FloatValueFormat = handle("E", 2, true, false);
/// The hatch progress bar's two numbers (value and scale), seconds.
pub const HATCH_TIME: FloatValueFormat = handle("s", 1, false, false);
/// The stored energy bar's two numbers.
pub const STORED_ENERGY: FloatValueFormat = handle("E", 1, true, false);

/// The text of the Stats panel node `rest` (its path below `.../BibitePanels/`) for the target `panel`, when the node shows a value.
#[must_use]
pub fn text(panel: &SelectionPanel, rest: &str) -> Option<String> {
    let row = rest.strip_prefix("StatsPanel/MainSection/")?;
    if let Some(egg) = &panel.egg {
        // `BibiteStatsPanel.UpdateEggStats`; the bars' scales are what `InitSliderHandle` got when the panel was filled
        // (`hatchTime`, `hatching.energy`).
        return Some(match row {
            "EggStats/Generation/ValueIndicator" => GENERATION.format(panel.generation as f32),
            "EggStats/Size/ValueIndicator" => AREA.format(egg.egg_area),
            "EggStats/Mass/ValueIndicator" => MASS.format(egg.egg_mass),
            "EggStats/Energy/ValueIndicator" => EGG_ENERGY.format(egg.energy),
            "EggStats/Hatch progress/ValueSlider/progressValue" => {
                HATCH_TIME.format(egg.hatch_progress)
            }
            "EggStats/Hatch progress/ValueSlider/maxValue" => HATCH_TIME.format(egg.hatch_time),
            "EggStats/Energy Stored/ValueSlider/progressValue" => {
                STORED_ENERGY.format(egg.free_energy)
            }
            "EggStats/Energy Stored/ValueSlider/maxValue" => STORED_ENERGY.format(egg.energy),
            _ => return None,
        });
    }
    // `BibiteStatsPanel.UpdateBibiteStats`.
    Some(match row {
        "BibiteStats/Generation/ValueIndicator" => GENERATION.format(panel.generation as f32),
        "BibiteStats/Lenght/ValueIndicator" => LENGTH.format(panel.body_length),
        "BibiteStats/Size/ValueIndicator" => AREA.format(panel.real_body_area),
        "BibiteStats/Mass/ValueIndicator" => MASS.format(panel.mass),
        "BibiteStats/Speed/ValueIndicator" => SPEED.format(panel.signed_velocity),
        "BibiteStats/Time Alive/ValueIndicator" => TIME_ALIVE.format(panel.age_seconds),
        "BibiteStats/Distance Traveled/ValueIndicator" => DISTANCE.format(panel.travelled),
        "BibiteStats/Number of eggs/ValueIndicator" => COUNT_INT.format(panel.eggs_laid as f32),
        "BibiteStats/Alive Children/Button /ValueIndicator" => {
            COUNT_INT.format(panel.alive_children as f32)
        }
        "BibiteStats/BibitesBitten/ValueIndicator" => COUNT.format(panel.bibites_bitten as f32),
        "BibiteStats/TotalDamagesDealt/ValueIndicator" => DAMAGE.format(panel.damage_dealt),
        "BibiteStats/Times Attacked/ValueIndicator" => COUNT.format(panel.times_attacked as f32),
        "BibiteStats/Total Damages Suffered/ValueIndicator" => DAMAGE.format(panel.damage_received),
        _ => return None,
    })
}

/// `Mathf.Round(x)` interpolated into a string (`StatusBarsReference.UpdateHealth` / `UpdateEnergy`): half to even, no decimals.
fn rounded(value: f32) -> String {
    format!("{}", value.round_ties_even())
}

/// The texts of the two status bars over the target (`StatusBarsReference`): a bibite's health and energy over their maxima, an
/// egg's spent share of its required energy over that energy (`HatchRatio() * totalRequiredEnergy / totalRequiredEnergy`) and its
/// free energy over its energy (`GUIManager.UpdateRealTimeUI`).
#[must_use]
pub fn status_bar_texts(panel: &SelectionPanel) -> (String, String) {
    match &panel.egg {
        Some(egg) => {
            // `Mathf.Min(hatchProgress / hatchTime, 1f)`
            let ratio = (egg.hatch_progress / egg.hatch_time).min(1.0);
            (
                format!(
                    "{} / {}",
                    rounded(ratio * egg.total_required_energy),
                    rounded(egg.total_required_energy)
                ),
                format!("{} / {}", rounded(egg.free_energy), rounded(egg.energy)),
            )
        }
        None => (
            format!("{} / {}", rounded(panel.health), rounded(panel.max_health)),
            format!("{} / {}", rounded(panel.energy), rounded(panel.max_energy)),
        ),
    }
}

/// How full the two status bars are (`StatusBarsReference.UpdateHealth` / `UpdateEnergy`: `fillAmount = value / max`): a bibite's health
/// and energy over their maxima, an egg's hatch ratio and its free energy over its energy. A bar whose maximum is 0 is empty (the
/// game's NaN draws nothing).
#[must_use]
pub fn status_bar_ratios(panel: &SelectionPanel) -> (f32, f32) {
    match &panel.egg {
        Some(egg) => (
            share(
                (egg.hatch_progress / egg.hatch_time).min(1.0) * egg.total_required_energy,
                egg.total_required_energy,
            ),
            share(egg.free_energy, egg.energy),
        ),
        None => (
            share(panel.health, panel.max_health),
            share(panel.energy, panel.max_energy),
        ),
    }
}

/// `value / max` as a bar's `fillAmount`: clamped to the bar, and 0 where the game's division is NaN or infinite.
fn share(value: f32, max: f32) -> f32 {
    let ratio = value / max;
    if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// `InfoSliderHandle.ScaledX` for a slider whose rectangle is `size` canvas units and whose scale is `scale`: the width of its fill, a cap of
/// eight "pixels" (`1 / ppu` each, `ppu = max(targetppu, 8 / width, 12 / height)`, `targetppu` 0.5 for the egg's bars) plus the
/// share `min(|value / scale|, 1)` of the rest. The bars' `bufferFactor` and `bufferValue` are 0, so the scale only grows when the value
/// reaches it, to the value itself.
#[must_use]
pub fn slider_fill_width(value: f32, scale: f32, size: Vec2) -> f32 {
    const TARGET_PPU: f32 = 0.5;
    let ppu = TARGET_PPU.max((8.0 / size.x).max(12.0 / size.y));
    let cap = 8.0 / ppu;
    let ratio = (value / scale).abs().min(1.0);
    cap + ratio * (size.x - cap)
}

/// `(value, scale)` of the egg section's bar whose `Fill` node is `rest` (its path below `.../BibitePanels/`).
#[must_use]
pub fn egg_slider(panel: &SelectionPanel, rest: &str) -> Option<(f32, f32)> {
    let egg = panel.egg.as_ref()?;
    match rest.strip_prefix("StatsPanel/MainSection/EggStats/")? {
        "Hatch progress/ValueSlider/Fill" => Some((egg.hatch_progress, egg.hatch_time)),
        "Energy Stored/ValueSlider/Fill" => Some((egg.free_energy, egg.energy)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_bibite_shows_milli_units_like_the_game() {
        // 0.5 u/s: the decade is -1, so the prefix is "m" and the number 500.00.
        assert_eq!(SPEED.format(0.5), "500.00 mu/s");
        assert_eq!(SPEED.format(-0.5), "-500.00 mu/s");
        assert_eq!(SPEED.format(0.0), "0.00 u/s");
        assert_eq!(MASS.format(0.0321), "32.10 mg");
        assert_eq!(AREA.format(1234.5), "1.23 ku\u{b2}");
        assert_eq!(LENGTH.format(15.61), "15.61 u");
        assert_eq!(DISTANCE.format(1500.0), "1.50 ku");
    }

    #[test]
    fn the_plain_rows_have_no_prefix() {
        assert_eq!(TIME_ALIVE.format(725.4), "725s");
        assert_eq!(TIME_ALIVE.format(0.0), "0s");
        assert_eq!(DAMAGE.format(0.0), "0.0HP");
        assert_eq!(DAMAGE.format(12.34), "12.3HP");
        assert_eq!(GENERATION.format(100.0), "100");
        assert_eq!(COUNT_INT.format(7.0), "7");
        assert_eq!(COUNT.format(3.0), "3");
    }

    #[test]
    fn the_egg_bars_print_seconds_and_energy_with_one_decimal() {
        assert_eq!(HATCH_TIME.format(5.12), "5.1s");
        assert_eq!(HATCH_TIME.format(0.0), "0.0s");
        assert_eq!(STORED_ENERGY.format(5.1), "5.1 E");
        assert_eq!(STORED_ENERGY.format(0.0), "0.0 E");
        assert_eq!(EGG_ENERGY.format(0.11), "110.00 mE");
    }

    #[test]
    fn the_status_bars_are_the_share_of_the_maximum_and_never_nan() {
        assert_eq!(share(5.0, 10.0), 0.5);
        assert_eq!(share(15.0, 10.0), 1.0);
        assert_eq!(share(1.0, 0.0), 0.0);
        assert_eq!(share(0.0, 0.0), 0.0);
        assert_eq!(share(-3.0, 10.0), 0.0);
    }

    #[test]
    fn the_fill_of_a_slider_is_a_cap_plus_the_share_of_the_scale() {
        // The egg's bars are 200 x 26: ppu = max(0.5, 0.04, 0.46) = 0.5, so the cap is 16.
        let size = Vec2::new(200.0, 26.0);
        assert!((slider_fill_width(0.0, 10.0, size) - 16.0).abs() < 1e-4);
        assert!((slider_fill_width(5.0, 10.0, size) - (16.0 + 0.5 * 184.0)).abs() < 1e-4);
        assert!((slider_fill_width(10.0, 10.0, size) - 200.0).abs() < 1e-4);
        // A value beyond the scale raises the scale to itself: the bar stays full.
        assert!((slider_fill_width(12.0, 10.0, size) - 200.0).abs() < 1e-4);
        // `Mathf.Min(NaN, 1)` is 1: an empty scale shows a full bar.
        assert!((slider_fill_width(0.0, 0.0, size) - 200.0).abs() < 1e-4);
        // A narrow bar takes the pixel size from its width: 8 / 40 = 0.2 < 0.5 still, but 12 / 10 = 1.2 wins on height.
        let thin = Vec2::new(40.0, 10.0);
        assert!((slider_fill_width(0.0, 1.0, thin) - 8.0 / 1.2).abs() < 1e-4);
    }
}
