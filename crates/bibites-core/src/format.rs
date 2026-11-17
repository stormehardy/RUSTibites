//! Number formatting used by the game's UI (`FloatFormatter.ScientificFormat`,
//! `FloatValueFormat`).
//!
//! Ported from `Here/ScriptHelpers/FloatFormatter.cs` and
//! `Here/UIScripts.InfoHandles/FloatValueFormat.cs`. Every inspector, settings
//! and graph label in the game goes through these, so the viewer should too.

/// SI prefix of a value (`FloatFormatter.ScientificPrefix`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScientificPrefix {
    /// Prefix text (`"k"`, `"m"`, ...); empty outside f..T.
    pub prefix: &'static str,
    /// `floor(log10(|v|))`.
    pub value_exponent: i32,
    /// `3 * floor(value_exponent / 3)`.
    pub unit_exponent: i32,
}

impl ScientificPrefix {
    /// `new ScientificPrefix(exponentOfValue)`.
    #[must_use]
    pub fn new(exponent_of_value: i32) -> Self {
        let unit_exponent = 3 * (exponent_of_value as f32 / 3.0).floor() as i32;
        let prefix = match unit_exponent {
            -12 => "f",
            -9 => "n",
            -6 => "\u{3bc}",
            -3 => "m",
            0 => "",
            3 => "k",
            6 => "M",
            9 => "G",
            12 => "T",
            _ => "",
        };
        Self {
            prefix,
            value_exponent: exponent_of_value,
            unit_exponent,
        }
    }

    /// `10^valueExponent`.
    #[must_use]
    pub fn value_factor(self) -> f32 {
        10.0_f32.powi(self.value_exponent)
    }

    /// `10^unitExponent`.
    #[must_use]
    pub fn unit_factor(self) -> f32 {
        10.0_f32.powi(self.unit_exponent)
    }
}

/// `val.SIPrefix()`.
#[must_use]
pub fn si_prefix(val: f32) -> ScientificPrefix {
    let abs = val.abs();
    // `Mathf.Approximately(x, 0)`: |x| < 8 * Mathf.Epsilon (1.1755e-38);
    // non-finite values have no meaningful decade.
    if abs < 8.0 * 1.175_494_4e-38 || !abs.is_finite() {
        return ScientificPrefix::new(0);
    }
    ScientificPrefix::new(abs.log10().floor() as i32)
}

/// `Mathf.Round` (round half to even).
fn mathf_round(x: f32) -> f32 {
    x.round_ties_even()
}

/// .NET `ToString("F<n>")`.
fn fixed(x: f32, decimals: i32) -> String {
    format!("{:.*}", decimals.max(0) as usize, x)
}

/// Options for [`scientific_format`] (the C# optional parameters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScientificOptions<'a> {
    /// Units suffix.
    pub units: &'a str,
    /// Prefix positive values with `+`.
    pub always_show_sign: bool,
    /// Put the SI prefix right after the number (with a space) instead of
    /// before the units.
    pub prefix_with_units: bool,
    /// Treat `precision` as significant digits.
    pub scientific_precision: bool,
    /// Use SI prefixes.
    pub si_units: bool,
    /// Integer value (precision 0 when no prefix applies).
    pub is_int: bool,
}

impl Default for ScientificOptions<'_> {
    fn default() -> Self {
        Self {
            units: "",
            always_show_sign: false,
            prefix_with_units: true,
            scientific_precision: true,
            si_units: true,
            is_int: false,
        }
    }
}

/// `FloatFormatter.ScientificFormat(float, ...)`.
#[must_use]
pub fn scientific_format(val: f32, precision: i32, opts: ScientificOptions<'_>) -> String {
    let sign = if val.is_nan() || val < 0.0 {
        "-"
    } else if opts.always_show_sign {
        "+"
    } else {
        ""
    };
    let mut num = val.abs();
    let prefix = si_prefix(val);
    let mut precision = precision;
    if opts.is_int && prefix.unit_exponent == 0 {
        precision = 0;
    }
    let text = if opts.scientific_precision {
        precision -= prefix.value_exponent;
        let scale = 10.0_f32.powi(precision);
        num = mathf_round(num * scale) / scale;
        format!(
            "{sign}{}",
            fixed(
                num / prefix.unit_factor(),
                (precision + prefix.unit_exponent).max(0)
            )
        )
    } else {
        format!("{sign}{}", fixed(num / prefix.unit_factor(), precision))
    };
    if opts.si_units {
        let units = opts.units.replace(' ', "");
        let (before, after) = if opts.prefix_with_units {
            (" ", "")
        } else {
            ("", " ")
        };
        format!("{text}{before}{}{after}{units}", prefix.prefix)
    } else {
        format!("{sign}{}{}", fixed(num, precision.max(0)), opts.units)
    }
}

/// `FloatValueFormat`: a reusable format description for an info field.
#[derive(Clone, Debug, PartialEq)]
pub struct FloatValueFormat {
    /// Text put before the number.
    pub prefix: &'static str,
    /// Units suffix.
    pub units: &'static str,
    /// Decimal / significant digits.
    pub precision: i32,
    /// Multiplier applied when `si` is false.
    pub factor: f32,
    /// Use SI prefixes.
    pub si: bool,
    /// Always print the sign.
    pub always_show_sign: bool,
    /// `precision` counts significant digits.
    pub precision_is_si: bool,
    /// Integer field.
    pub is_int: bool,
}

impl Default for FloatValueFormat {
    fn default() -> Self {
        Self {
            prefix: "",
            units: "",
            precision: 1,
            factor: 1.0,
            si: true,
            always_show_sign: false,
            precision_is_si: false,
            is_int: false,
        }
    }
}

impl FloatValueFormat {
    /// `FloatValueFormat.percentage`.
    #[must_use]
    pub fn percentage() -> Self {
        Self {
            units: "%",
            factor: 100.0,
            si: false,
            precision: 1,
            ..Self::default()
        }
    }

    /// `FloatValueFormat.id` (`"(#123)"`).
    #[must_use]
    pub fn id() -> Self {
        Self {
            si: false,
            precision: 0,
            is_int: true,
            prefix: "(#",
            units: ")",
            ..Self::default()
        }
    }

    /// `FloatValueFormat.Format`.
    #[must_use]
    pub fn format(&self, val: f32) -> String {
        let value = if self.si { val } else { val * self.factor };
        let text = scientific_format(
            value,
            self.precision,
            ScientificOptions {
                units: self.units,
                always_show_sign: self.always_show_sign,
                prefix_with_units: !self.is_int,
                scientific_precision: self.precision_is_si && !self.is_int,
                si_units: self.si,
                is_int: self.is_int,
            },
        );
        format!("{}{text}", self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_follow_decades() {
        assert_eq!(si_prefix(1234.5).prefix, "k");
        assert_eq!(si_prefix(0.0).prefix, "");
        assert_eq!(si_prefix(0.0042).prefix, "m");
        assert_eq!(si_prefix(5.0e6).prefix, "M");
        assert_eq!(si_prefix(-250.0).prefix, "");
    }

    #[test]
    fn scientific_precision_rounds_to_significant_digits() {
        // 1234.5: exponent 3 -> precision 0 -> round-half-even 1234 -> "1.234".
        let opts = ScientificOptions {
            units: "E",
            ..ScientificOptions::default()
        };
        assert_eq!(scientific_format(1234.5, 3, opts), "1.234 kE");
        assert_eq!(scientific_format(-0.5, 2, opts), "-500 mE");
    }

    #[test]
    fn percentage_and_id_formats() {
        assert_eq!(FloatValueFormat::percentage().format(0.256), "25.6%");
        assert_eq!(FloatValueFormat::id().format(123.0), "(#123)");
    }

    #[test]
    fn sign_and_plain_modes() {
        let opts = ScientificOptions {
            always_show_sign: true,
            si_units: false,
            scientific_precision: false,
            units: " u",
            ..ScientificOptions::default()
        };
        assert_eq!(scientific_format(2.0, 2, opts), "+2.00 u");
        assert_eq!(scientific_format(-2.0, 1, opts), "-2.0 u");
    }
}
