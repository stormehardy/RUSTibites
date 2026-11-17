//! `Utility.TimeFormat` (`Here/Utility/TimeFormat.cs`): a duration in the largest unit that fits, and the "3 hours 20
//! minutes" / "3h 20m" texts the lineage axis and the species tooltips use.

use bibites_core::datalog::LogLikeFormat;
use bibites_core::species_log::minutes_of_point;

/// `Timescale`; the numbers are the enum's (`FormattedTimeValue` counts units down by them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Timescale {
    /// `SubSeconds`.
    SubSeconds = 0,
    /// `Seconds`.
    Seconds = 1,
    /// `Minutes`.
    Minutes = 2,
    /// `Hours`.
    Hours = 3,
    /// `Days`.
    Days = 4,
    /// `Months` (30 days).
    Months = 5,
    /// `Years` (12 months).
    Years = 6,
}

impl Timescale {
    fn from_index(index: i32) -> Option<Self> {
        Some(match index {
            0 => Self::SubSeconds,
            1 => Self::Seconds,
            2 => Self::Minutes,
            3 => Self::Hours,
            4 => Self::Days,
            5 => Self::Months,
            6 => Self::Years,
            _ => return None,
        })
    }

    /// `FactorAtTimeScale`: seconds in one.
    #[must_use]
    pub fn factor(self) -> f32 {
        match self {
            Self::SubSeconds | Self::Seconds => 1.0,
            Self::Minutes => 60.0,
            Self::Hours => 3600.0,
            Self::Days => 86_400.0,
            Self::Months => 2_592_000.0,
            Self::Years => 31_104_000.0,
        }
    }

    /// `SmallUnitsAtTimeScale`.
    #[must_use]
    pub fn small_units(self) -> &'static str {
        match self {
            Self::SubSeconds | Self::Seconds => "s",
            Self::Minutes => "m",
            Self::Hours => "h",
            Self::Days => "d",
            Self::Months => "M",
            Self::Years => "y",
        }
    }

    /// `BigUnitsAtTimeScale`.
    #[must_use]
    pub fn big_units(self) -> &'static str {
        match self {
            Self::SubSeconds | Self::Seconds => "seconds",
            Self::Minutes => "minutes",
            Self::Hours => "hours",
            Self::Days => "days",
            Self::Months => "months",
            Self::Years => "years",
        }
    }
}

/// A duration split in a unit and a value (`TimeFormat`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeFormat {
    /// The unit it was put in.
    pub scale: Timescale,
    /// The value in that unit.
    pub val: f32,
}

impl TimeFormat {
    /// `new TimeFormat(value, valueScale)`.
    #[must_use]
    pub fn new(value: f32, value_scale: Timescale) -> Self {
        let seconds = value * value_scale.factor();
        if seconds < Timescale::Seconds.factor() {
            return Self {
                scale: Timescale::SubSeconds,
                val: seconds,
            };
        }
        let scale = if seconds < Timescale::Minutes.factor() {
            Timescale::Seconds
        } else if seconds < Timescale::Hours.factor() {
            Timescale::Minutes
        } else if seconds < Timescale::Days.factor() {
            Timescale::Hours
        } else if seconds < Timescale::Months.factor() {
            Timescale::Days
        } else if seconds < Timescale::Years.factor() {
            Timescale::Months
        } else {
            Timescale::Years
        };
        Self {
            scale,
            val: seconds / scale.factor(),
        }
    }

    /// `TimeOfPointInSerialConfig`: how long ago the point of the species log was taken (`-1` is now).
    #[must_use]
    pub fn of_species_point(point: i32) -> Self {
        Self::new(minutes_of_point(point) as f32, Timescale::Minutes)
    }

    /// `TimeOfPointInSerialConfig(config, point)`: how long ago the `point`th newest point (0 is the newest) of a serial log of these
    /// scales was logged, in units of `acquisition` times `factor`. A last scale of size 0 holds any number of points.
    #[must_use]
    pub fn of_serial_point(
        formats: &[LogLikeFormat],
        factor: i32,
        acquisition: Timescale,
        point: i32,
    ) -> Self {
        let mut step = i64::from(factor);
        let mut total = 0_i64;
        let mut left = i64::from(point) + 1;
        for (index, format) in formats.iter().enumerate() {
            let size = if index + 1 == formats.len() && format.size == 0 {
                i64::from(i32::MAX)
            } else {
                format.size as i64
            };
            if left > size {
                total += step * size;
                left -= size;
                step *= format.feed_ratio as i64;
                continue;
            }
            total += step * left;
            break;
        }
        Self::new(total as f32, acquisition)
    }

    /// `FormattedTimeValue(unitsPrecision, spacer, smallUnits, spaceBeforeUnits, minTimescale, neverShowDecimals)`
    /// with `neverShowDecimals` true: the largest unit and as many smaller ones as `units_precision` says.
    #[must_use]
    pub fn formatted(
        &self,
        units_precision: i32,
        spacer: &str,
        small_units: bool,
        space_before_units: bool,
        min_timescale: Timescale,
    ) -> String {
        let before = if space_before_units { " " } else { "" };
        let mut text = String::new();
        let start = self.scale as i32;
        let mut value = self.val;
        let mut scale = self.scale;
        for i in 0..=units_precision {
            let index = start - i;
            if index < min_timescale as i32 {
                break;
            }
            let Some(unit) = Timescale::from_index(index) else {
                break;
            };
            let units = if small_units {
                unit.small_units()
            } else {
                unit.big_units()
            };
            value *= scale.factor() / unit.factor();
            let whole = floor_approximately(value);
            if index > 1 {
                text.push_str(&format!(
                    "{}{:.0}{}{}",
                    if index < start { spacer } else { "" },
                    whole,
                    if space_before_units { " " } else { "" },
                    units
                ));
                scale = unit;
                value -= whole;
                continue;
            }
            text.push_str(&format!(
                "{}{}{}{}",
                if index < start { spacer } else { "" },
                round_half_up(value),
                before,
                units
            ));
            break;
        }
        text
    }
}

/// `MathfE.FloorApproximately`: the floor, or the rounded value when it is just under the next whole.
fn floor_approximately(value: f32) -> f32 {
    let rounded = value.round();
    let floor = value.floor();
    if value - floor > 0.999 {
        rounded
    } else {
        floor
    }
}

fn round_half_up(value: f32) -> String {
    format!("{:.0}", (value + 0.5).floor())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_is_put_in_its_largest_unit() {
        let t = TimeFormat::new(200.0, Timescale::Minutes);
        assert_eq!(t.scale, Timescale::Hours);
        assert!((t.val - 3.3333).abs() < 1e-3);
        assert_eq!(
            TimeFormat::new(30.0, Timescale::Seconds).scale,
            Timescale::Seconds
        );
        assert_eq!(
            TimeFormat::new(0.0, Timescale::Minutes).scale,
            Timescale::SubSeconds
        );
        assert_eq!(
            TimeFormat::new(60.0 * 24.0 * 3.0, Timescale::Minutes).scale,
            Timescale::Days
        );
    }

    #[test]
    fn the_texts_of_the_tooltips_and_the_axis() {
        // The tooltips: "approximately 3 hours 20 minutes ago".
        let t = TimeFormat::new(200.0, Timescale::Minutes);
        assert_eq!(
            t.formatted(1, " ", false, true, Timescale::Minutes),
            "3 hours 20 minutes"
        );
        // The axis: "3h 20m ago".
        assert_eq!(
            t.formatted(1, " ", true, false, Timescale::Minutes),
            "3h 20m"
        );
        // Whole hours still say the minutes.
        let hour = TimeFormat::new(60.0, Timescale::Minutes);
        assert_eq!(
            hour.formatted(1, " ", true, false, Timescale::Minutes),
            "1h 0m"
        );
        // Minutes alone.
        let five = TimeFormat::new(5.0, Timescale::Minutes);
        assert_eq!(
            five.formatted(1, " ", true, false, Timescale::Minutes),
            "5m"
        );
        assert_eq!(
            five.formatted(1, " ", false, true, Timescale::Minutes),
            "5 minutes"
        );
        // Days and hours.
        let day = TimeFormat::new(26.0 * 60.0, Timescale::Minutes);
        assert_eq!(
            day.formatted(1, " ", false, true, Timescale::Minutes),
            "1 days 2 hours"
        );
    }

    #[test]
    fn points_of_the_log_are_minutes_ago() {
        // Point 11 is the oldest of the first scale: 12 logs of 5 minutes.
        assert_eq!(
            TimeFormat::of_species_point(11).formatted(1, " ", true, false, Timescale::Minutes),
            "1h 0m"
        );
        assert!(TimeFormat::of_species_point(-1).val < 1e-4, "now");
    }
}
