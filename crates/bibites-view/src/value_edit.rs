//! Typed values: the edit field a slider row opens when its value text is clicked.
//!
//! `SettingSlider.OpenEditPanel` hides the slider and its reset button (`NormalSecton`) and shows an
//! `EditSection` with a `TMP_InputField` (integer or decimal content) in their place, filled with
//! `setting.val * format.factor` (`float.ToString()`) and selected whole (`onFocusSelectAll`). While
//! typing, `UpdateValueDuringEdit` shows the value the text means in the row's value text; Enter
//! submits (`SubmitEditValue`), Escape and a press anywhere else close the field without applying it
//! (`CloseEditPanel(false)`, `ExitEditField`). The slider row is a `HorizontalLayoutGroup`, so the
//! name, the value text and the field share the row by Unity's flexible sizes ([`row_parts`]).

use crate::dyn_panel::FieldRef;

/// What the field accepts (`TMP_InputField.contentType`: integers for `IntSettingSlider`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// `ContentType.IntegerNumber`: digits and a leading minus.
    Integer,
    /// `ContentType.DecimalNumber`: digits, a leading minus and one decimal separator.
    Decimal,
}

/// What a slider row's setting says about its typed values (`NumericSetting`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    /// `format.factor`: the field shows `value * factor`.
    pub factor: f32,
    /// `minValue` (a setting is `canBeNegative` when this is below 0).
    pub min: f32,
    /// `maxValue`.
    pub max: f32,
    /// `canGoOutOfBounds` (true unless a setting says otherwise): typed values past the slider's
    /// ends are kept.
    pub out_of_bounds: bool,
    /// The setting is an `IntSetting`: the typed number is cut to a whole one.
    pub integer: bool,
    /// A `LogSettingSlider`, whose `SubmitEditValue` is its own.
    pub log: bool,
}

/// A key that moves the caret of an edit field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaretKey {
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Ctrl + A.
    SelectAll,
}

/// The state of an open edit field.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueEdit {
    /// The row being edited.
    pub field: FieldRef,
    /// What the field accepts.
    pub content: Content,
    /// The text in the field.
    pub text: String,
    /// The caret and the selection (`onFocusSelectAll` selects everything on opening: the next valid key replaces it).
    pub caret: crate::text_field::Caret,
    /// Seconds since the last key (the caret blinks from there, `ResetBlinkTime`).
    pub idle: f32,
}

/// `TMP_InputField.caretBlinkRate` in the prefab.
const BLINK_RATE: f32 = 0.85;

impl ValueEdit {
    /// `OpenEditPanel`: the field holds the setting's value times the factor, selected.
    #[must_use]
    pub fn open(field: FieldRef, shown: f32, content: Content) -> Self {
        let text = dotnet_float_string(shown);
        let mut caret = crate::text_field::Caret::at_end(&text);
        caret.select_all(&text);
        Self { field, content, text, caret, idle: 0.0 }
    }

    /// Whether some text is selected.
    #[must_use]
    pub fn selected(&self) -> bool {
        self.caret.selection().is_some()
    }

    /// Left / Right / Home / End (Shift extends the selection) and Ctrl + A.
    pub fn move_caret(&mut self, key: CaretKey, extend: bool) {
        match key {
            CaretKey::Left => self.caret.left(extend),
            CaretKey::Right => self.caret.right(&self.text, extend),
            CaretKey::Home => self.caret.home(extend),
            CaretKey::End => self.caret.end(&self.text, extend),
            CaretKey::SelectAll => self.caret.select_all(&self.text),
        }
        self.idle = 0.0;
    }

    /// Delete: the selection, or the character after the caret.
    pub fn delete_forward(&mut self) {
        self.caret.delete(&mut self.text);
        self.idle = 0.0;
    }

    /// `TMP_InputField.Validate` (`CharacterValidation.Integer` / `Decimal`) against the whole text,
    /// the selection still in it: the character to insert, if the field takes it.
    fn validate(&self, ch: char) -> Option<char> {
        // The insertion point is the start of the selection, else the caret; a minus fits there only at the very start (and not twice).
        let point = self.caret.selection().map_or(self.caret.pos, |(from, _)| from);
        let at_start = point == 0 && (self.selected() || !self.text.contains('-'));
        match ch {
            '0'..='9' => Some(ch),
            '-' if at_start => Some('-'),
            // Whichever separator was typed becomes the culture's, and only one is allowed.
            '.' | ',' if self.content == Content::Decimal && !self.text.contains(['.', ',']) => {
                Some('.')
            }
            _ => None,
        }
    }

    /// `TMP_InputField.Append(char)`: a character the field takes replaces the selection or joins
    /// the end of the text. Returns whether the text changed.
    pub fn type_char(&mut self, ch: char) -> bool {
        let Some(ch) = self.validate(ch) else {
            return false;
        };
        self.caret.insert(&mut self.text, &ch.to_string());
        self.idle = 0.0;
        true
    }

    /// Ctrl + V: every character goes through [`ValueEdit::type_char`] in turn.
    pub fn paste(&mut self, text: &str) {
        for ch in text.chars() {
            self.type_char(ch);
        }
    }

    /// Backspace (and Delete, the caret sitting at the end): the selection goes, else the last
    /// character.
    pub fn backspace(&mut self) {
        self.caret.backspace(&mut self.text);
        self.idle = 0.0;
    }

    /// Whether the caret is drawn: no selection, and the first half of the blink period
    /// (`caretBlinkRate`).
    #[must_use]
    pub fn caret_visible(&self) -> bool {
        let period = 1.0 / BLINK_RATE;
        !self.selected() && self.idle % period <= period / 2.0
    }
}

impl Rules {
    /// `setting.canBeNegative`.
    fn can_be_negative(&self) -> bool {
        self.min < 0.0
    }

    /// `UpdateValueDuringEdit` / `SettingSlider.SubmitEditValue`: the value the text means after
    /// the factor and the clamps (`None` when the text does not parse: the C# logs and returns).
    #[must_use]
    pub fn value_of(&self, text: &str) -> Option<f32> {
        let mut number = parse_number(&fixed_up(text))? / self.factor;
        if !self.can_be_negative() {
            number = number.max(0.0);
        }
        if !self.out_of_bounds {
            number = number.clamp(self.min, self.max);
        }
        Some(number)
    }

    /// What `UpdateValueDuringEdit` shows in the value text (a zero is never negative).
    #[must_use]
    pub fn preview(&self, text: &str) -> Option<f32> {
        let number = self.value_of(text)?;
        Some(if number.abs() < 8.0 * f32::MIN_POSITIVE {
            0.0
        } else {
            number
        })
    }

    /// The value Enter applies: `SubmitEditValue` of the row's slider class (a `LogSettingSlider`
    /// clamps before dividing by the factor and has no fix-ups for a lone sign or point), cut to a
    /// whole number for an `IntSetting`.
    #[must_use]
    pub fn submitted(&self, text: &str) -> Option<f32> {
        let number = if self.log {
            let text = if text.is_empty() { "0" } else { text };
            let mut number = parse_number(text)?;
            if !self.can_be_negative() {
                number = number.max(0.0);
            }
            if !self.out_of_bounds {
                number = number.clamp(self.min, self.max);
            }
            number / self.factor
        } else {
            self.value_of(text)?
        };
        Some(if self.integer { number.trunc() } else { number })
    }
}

/// The fix-ups `SubmitEditValue` makes before parsing: an empty text is "0", a sign gets a zero after
/// it, a leading point gets one before it.
fn fixed_up(text: &str) -> String {
    let mut text = if text.is_empty() {
        "0".to_owned()
    } else {
        text.to_owned()
    };
    if let Some(sign) = text.chars().next().filter(|c| matches!(c, '-' | '+')) {
        text = format!("{sign}0{}", &text[1..]);
    }
    if text.starts_with(['.', ',']) {
        text.insert(0, '0');
    }
    text
}

/// `float.Parse(text.Replace(",", "."), NumberStyles.Any, CultureInfo.InvariantCulture)`.
fn parse_number(text: &str) -> Option<f32> {
    text.replace(',', ".").parse::<f32>().ok()
}

/// .NET's `float.ToString()`: the shortest digits that read back as the same float, fixed notation
/// unless the point lies more than `max(digits, 7)` places to the right of the first digit or more
/// than 3 to the left of it, then `d.dddE+XX`.
#[must_use]
pub fn dotnet_float_string(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Infinity" } else { "Infinity" }.to_owned();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if value == 0.0 {
        return format!("{sign}0");
    }
    let sci = format!("{:e}", value.abs());
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let count = digits.len() as i32;
    // Where the decimal point sits relative to the first digit (`number.Scale`).
    let scale = exponent + 1;
    if scale > count.max(7) || scale < -3 {
        let exponent = scale - 1;
        let tail = &digits[1..];
        let point = if tail.is_empty() { "" } else { "." };
        return format!(
            "{sign}{}{point}{tail}E{}{:02}",
            &digits[..1],
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        );
    }
    if scale <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-scale) as usize))
    } else if scale >= count {
        format!("{sign}{digits}{}", "0".repeat((scale - count) as usize))
    } else {
        let (whole, fraction) = digits.split_at(scale as usize);
        format!("{sign}{whole}.{fraction}")
    }
}

/// The advance of a character of the field's font (LiberationSans SDF, 2048 units to the em).
fn advance(ch: char) -> f32 {
    match ch {
        '.' | ',' => 569.0,
        '-' => 682.0,
        '+' => 1196.0,
        'E' => 1366.0,
        _ => 1139.0,
    }
}

/// The width of `text` at font size `size` (the field only holds digits, a point and a minus).
#[must_use]
pub fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(advance).sum::<f32>() / 2048.0 * size
}

/// A child of a `HorizontalLayoutGroup` with `childControlWidth`: its minimum, preferred and
/// flexible width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Share {
    /// `minWidth`.
    pub min: f32,
    /// `preferredWidth`.
    pub preferred: f32,
    /// `flexibleWidth`.
    pub flexible: f32,
}

/// `HorizontalLayoutGroup.SetChildrenAlongAxis` (no force-expand): the width of each child of a row
/// `width` wide with `spacing` between them. Below the preferred total the children shrink from
/// their preferred to their minimum widths together; above it the flexible ones share the surplus.
#[must_use]
pub fn distribute(width: f32, spacing: f32, shares: &[Share]) -> Vec<f32> {
    let gaps = spacing * shares.len().saturating_sub(1) as f32;
    let min: f32 = shares.iter().map(|s| s.min).sum::<f32>() + gaps;
    let preferred: f32 = shares.iter().map(|s| s.preferred).sum::<f32>() + gaps;
    let flexible: f32 = shares.iter().map(|s| s.flexible).sum();
    let lerp = if (preferred - min).abs() > f32::EPSILON {
        ((width - min) / (preferred - min)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let surplus = width - preferred;
    let per_flex = if surplus > 0.0 && flexible > 0.0 {
        surplus / flexible
    } else {
        0.0
    };
    shares
        .iter()
        .map(|s| s.min + (s.preferred - s.min) * lerp + s.flexible * per_flex)
        .collect()
}

/// The horizontal parts of a slider row (`SettingSlider` prefab): x ranges relative to the row's left
/// edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowParts {
    /// The setting's name (`Name`).
    pub name: [f32; 2],
    /// The value text (`FloatInfoText`).
    pub value: [f32; 2],
    /// The slider and its reset button (`NormalSecton`), or the edit field (`EditSection`).
    pub body: [f32; 2],
}

/// The shares of the row's three children (`SettingSlider` prefab, level `sharedassets0`): the name
/// 100 / 125 / 2, the value text 60 / 80 / 1, then the normal section (the slider panel 100 / 200 / 1
/// and the 40-wide reset button 15 apart) or the edit section (its field is at least 300 wide).
fn row_shares(editing: bool) -> [Share; 3] {
    let body = if editing {
        Share {
            min: 300.0,
            preferred: 300.0,
            flexible: 1.0,
        }
    } else {
        Share {
            min: 100.0 + 15.0 + 40.0,
            preferred: 200.0 + 15.0 + 40.0,
            flexible: 1.0,
        }
    };
    [
        Share {
            min: 100.0,
            preferred: 125.0,
            flexible: 2.0,
        },
        Share {
            min: 60.0,
            preferred: 80.0,
            flexible: 1.0,
        },
        body,
    ]
}

/// Spacing of the row's `HorizontalLayoutGroup`.
pub const ROW_SPACING: f32 = 15.0;

/// Where the name, the value text and the slider (or the edit field) sit in a row `width` wide.
#[must_use]
pub fn row_parts(width: f32, editing: bool) -> RowParts {
    let widths = distribute(width, ROW_SPACING, &row_shares(editing));
    let name = [0.0, widths[0]];
    let value_start = name[1] + ROW_SPACING;
    let value = [value_start, value_start + widths[1]];
    let body_start = value[1] + ROW_SPACING;
    RowParts {
        name,
        value,
        body: [body_start, body_start + widths[2]],
    }
}

/// Height of the edit field (`EditSection` / `InputField (TMP)` preferred height).
pub const FIELD_HEIGHT: f32 = 34.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules {
            factor: 1.0,
            min: 0.0,
            max: 10.0,
            out_of_bounds: true,
            integer: false,
            log: false,
        }
    }

    fn edit(shown: f32, content: Content) -> ValueEdit {
        ValueEdit::open(FieldRef::Setting(0), shown, content)
    }

    #[test]
    fn a_float_reads_like_dotnets_to_string() {
        for (value, text) in [
            (0.0, "0"),
            (1.0, "1"),
            (7.5, "7.5"),
            (100.0, "100"),
            (-2.5, "-2.5"),
            (0.001, "0.001"),
            (0.0001, "0.0001"),
            (0.00001, "1E-05"),
            (1.5e-7, "1.5E-07"),
            (12345.678, "12345.678"),
            (16_777_216.0, "16777216"),
            (1.0e7, "1E+07"),
            (3.4028235e38, "3.4028235E+38"),
            (0.15, "0.15"),
            (2000.0, "2000"),
        ] {
            assert_eq!(dotnet_float_string(value), text, "{value:e}");
        }
        // Negative zero keeps its sign, as in .NET Core 3.0 and later.
        assert_eq!(dotnet_float_string(-0.0), "-0");
    }

    #[test]
    fn opening_the_field_fills_it_with_the_selected_value() {
        let edit = edit(7.5, Content::Decimal);
        assert_eq!(edit.text, "7.5");
        assert!(edit.selected(), "`onFocusSelectAll`");
        assert!(!edit.caret_visible(), "a selection has no caret");
    }

    #[test]
    fn the_first_valid_key_replaces_the_selection_and_the_rest_append() {
        let mut edit = edit(7.5, Content::Decimal);
        // Letters are not taken; the selection stays.
        assert!(!edit.type_char('x'));
        assert!(edit.selected() && edit.text == "7.5");
        // A point is refused while the (selected) text has one, as `Validate` looks at the whole text.
        assert!(!edit.type_char('.'));
        assert!(edit.type_char('4'));
        assert_eq!(edit.text, "4");
        assert!(!edit.selected());
        assert!(edit.type_char('2'));
        assert!(edit.type_char(','));
        assert_eq!(edit.text, "42.", "a comma becomes the separator");
        assert!(!edit.type_char('.'), "one separator only");
        assert!(edit.type_char('5'));
        assert_eq!(edit.text, "42.5");
        // A minus only goes at the start.
        assert!(!edit.type_char('-'));
        assert_eq!(edit.text, "42.5");
    }

    #[test]
    fn a_minus_goes_first_and_an_integer_field_has_no_point() {
        let mut edit = edit(5.0, Content::Integer);
        assert!(edit.type_char('-'), "the selection starts at 0");
        assert!(edit.type_char('3'));
        assert!(!edit.type_char('.'));
        assert!(!edit.type_char('-'));
        assert_eq!(edit.text, "-3");
        // Emptied, the field takes a minus again.
        edit.backspace();
        edit.backspace();
        assert_eq!(edit.text, "");
        assert!(edit.type_char('-'));
        assert_eq!(edit.text, "-");
        // The cursor is not before the dash, so digits follow it.
        assert!(edit.type_char('7'));
        assert_eq!(edit.text, "-7");
    }

    #[test]
    fn the_caret_moves_and_typing_goes_in_at_it() {
        let mut edit = edit(12.5, Content::Decimal);
        edit.move_caret(CaretKey::Home, false);
        assert!(!edit.selected(), "Home drops the selection");
        assert!(edit.type_char('-'), "a minus fits at the start");
        assert_eq!(edit.text, "-12.5");
        assert!(!edit.type_char('-'), "not twice");
        edit.move_caret(CaretKey::End, false);
        edit.move_caret(CaretKey::Left, false);
        assert!(edit.type_char('9'));
        assert_eq!(edit.text, "-12.95");
        edit.delete_forward();
        assert_eq!(edit.text, "-12.9");
        edit.move_caret(CaretKey::Left, true);
        edit.move_caret(CaretKey::Left, true);
        assert_eq!(edit.caret.selection(), Some((3, 5)));
        edit.backspace();
        assert_eq!(edit.text, "-12");
    }

    #[test]
    fn backspace_deletes_the_selection_then_one_character() {
        let mut edit = edit(12.5, Content::Decimal);
        edit.backspace();
        assert_eq!(edit.text, "");
        assert!(!edit.selected());
        edit.paste("1,2x3-4");
        assert_eq!(
            edit.text, "1.234",
            "each pasted character is validated in turn"
        );
        edit.backspace();
        assert_eq!(edit.text, "1.23");
    }

    #[test]
    fn the_caret_blinks_at_the_prefabs_rate_and_restarts_on_a_key() {
        let mut edit = edit(1.0, Content::Decimal);
        edit.type_char('2');
        assert!(edit.caret_visible());
        edit.idle = 0.5;
        assert!(
            edit.caret_visible(),
            "visible for the first half of 1 / 0.85 s"
        );
        edit.idle = 0.6;
        assert!(!edit.caret_visible());
        edit.idle = 1.2;
        assert!(edit.caret_visible(), "and again in the next period");
        edit.backspace();
        assert_eq!(edit.idle, 0.0);
    }

    #[test]
    fn typed_text_means_its_number_divided_by_the_factor() {
        let percent = Rules {
            factor: 100.0,
            min: 0.0,
            max: 1.0,
            ..rules()
        };
        assert_eq!(percent.value_of("50"), Some(0.5));
        assert_eq!(percent.preview("50"), Some(0.5));
        // An empty text is 0; a lone sign or point gets its zero.
        assert_eq!(percent.value_of(""), Some(0.0));
        assert_eq!(percent.value_of("."), Some(0.0));
        assert_eq!(percent.value_of(".5"), Some(0.005));
        assert_eq!(
            percent.value_of("-"),
            Some(0.0),
            "not negative: clamped to 0"
        );
        // Out of the slider's range is kept (`canGoOutOfBounds`).
        assert_eq!(percent.value_of("250"), Some(2.5));
        // ... unless the setting says no.
        let bounded = Rules {
            out_of_bounds: false,
            ..percent
        };
        assert_eq!(bounded.value_of("250"), Some(1.0));
        assert_eq!(bounded.value_of("-5"), Some(0.0));
        // A setting that can be negative keeps the sign.
        let signed = Rules {
            min: -10.0,
            max: 10.0,
            ..rules()
        };
        assert_eq!(signed.value_of("-4.5"), Some(-4.5));
        assert_eq!(signed.value_of("-.5"), Some(-0.5));
        // The preview never shows a negative zero.
        assert!(signed.preview("-0").is_some_and(|v| v.is_sign_positive()));
    }

    #[test]
    fn an_integer_setting_cuts_the_typed_number() {
        let whole = Rules {
            integer: true,
            max: 500.0,
            ..rules()
        };
        assert_eq!(whole.submitted("12.9"), Some(12.0));
        assert_eq!(
            whole.preview("12.9"),
            Some(12.9),
            "the preview keeps the fraction"
        );
        let signed = Rules {
            min: -50.0,
            ..whole
        };
        assert_eq!(
            signed.submitted("-3.9"),
            Some(-3.0),
            "toward zero, as `(int)`"
        );
    }

    #[test]
    fn a_log_slider_submits_its_own_way() {
        // `LogSettingSlider.SubmitEditValue`: clamped before the factor, a lone minus does not parse.
        let log = Rules {
            factor: 100.0,
            min: 0.0,
            max: 1.0,
            out_of_bounds: false,
            log: true,
            ..rules()
        };
        // 250 is clamped to 1 first, then divided: 0.01 (the linear way gives 1.0).
        assert_eq!(log.submitted("250"), Some(0.01));
        assert_eq!(Rules { log: false, ..log }.submitted("250"), Some(1.0));
        assert_eq!(log.submitted(""), Some(0.0));
        assert_eq!(log.submitted("-"), None);
        assert_eq!(log.submitted(".5"), Some(0.005));
    }

    #[test]
    fn the_layout_shares_match_the_captured_rows() {
        // Captured rows (the scenario editor's tab and the world's panel): the name, the value text
        // and the normal section, 15 apart.
        let editor = row_parts(1087.8 - 41.0, false);
        assert!((editor.name[1] - 403.4).abs() < 0.1, "{editor:?}");
        assert!((editor.value[0] - 418.4).abs() < 0.1);
        assert!((editor.value[1] - (678.6 - 41.0)).abs() < 0.1);
        assert!((editor.body[0] - (693.6 - 41.0)).abs() < 0.1);
        assert!((editor.body[1] - (1087.8 - 41.0)).abs() < 0.1);
        let world = row_parts(1871.0 - 944.0, false);
        assert!((world.name[1] - 343.5).abs() < 0.1, "{world:?}");
        assert!((world.value[1] - (1491.8 - 944.0)).abs() < 0.1);
        assert!((world.body[0] - (1506.8 - 944.0)).abs() < 0.1);
    }

    #[test]
    fn editing_gives_the_field_the_sliders_place_and_a_bit_more() {
        // The field needs 300 where the slider and its reset button needed 255, so the name and the
        // value text give up some width.
        let normal = row_parts(927.0, false);
        let editing = row_parts(927.0, true);
        assert!(editing.name[1] < normal.name[1]);
        assert!(editing.value[1] - editing.value[0] < normal.value[1] - normal.value[0]);
        assert!(editing.body[1] - editing.body[0] > 300.0);
        assert!((editing.name[1] - 321.0).abs() < 1e-3, "{editing:?}");
        assert!((editing.value[0] - 336.0).abs() < 1e-3);
        assert!((editing.value[1] - 514.0).abs() < 1e-3);
        assert!((editing.body[0] - 529.0).abs() < 1e-3);
        assert!((editing.body[1] - 927.0).abs() < 1e-3, "the row stays full");
        // A narrower row shrinks everything towards the minimums together (490 wide at the least).
        let narrow = row_parts(510.0, true);
        assert!((narrow.body[1] - 510.0).abs() < 1e-3, "{narrow:?}");
        assert!((narrow.body[1] - narrow.body[0] - 300.0).abs() < 1e-3);
    }

    #[test]
    fn the_fields_text_has_liberation_sans_widths() {
        // 20 px: a digit is 1139 / 2048 of 20.
        assert!((text_width("0", 20.0) - 11.123).abs() < 0.01);
        assert!((text_width("12.5", 20.0) - (3.0 * 11.123 + 5.557)).abs() < 0.02);
        assert_eq!(text_width("", 20.0), 0.0);
    }
}
