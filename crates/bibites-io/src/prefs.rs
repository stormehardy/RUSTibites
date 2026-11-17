//! The installed game's `PlayerPrefs`, read-only.
//!
//! The game keeps its user settings, warnings, achievements and challenge
//! results in Unity `PlayerPrefs`, which on Windows live in the registry key
//! `HKCU\Software\The Bibites\The Bibites` (value names carry a `_h<hash>`
//! suffix; floats are 8-byte values holding a double, strings are NUL-terminated
//! UTF-8 binaries). RUSTibites never writes there: it only reads the values so the
//! port starts with the same settings, results and unlocks as the player's game.
//!
//! Reading uses a one-shot PowerShell child (no `unsafe`, no registry crate); on
//! other platforms, or when the key is missing, the result is empty.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// One stored preference.
#[derive(Clone, Debug, PartialEq)]
pub enum PrefValue {
    /// `PlayerPrefs.SetInt` (bool / enum / int settings).
    Int(i64),
    /// `PlayerPrefs.SetFloat` (float / colour channel settings).
    Float(f64),
    /// `PlayerPrefs.SetString`.
    Text(String),
}

/// A snapshot of the game's preferences, keyed by the original name (without the
/// registry's `_h<hash>` suffix).
#[derive(Clone, Debug, Default)]
pub struct GamePrefs {
    values: BTreeMap<String, PrefValue>,
    /// This machine's UTC offset in minutes, reported by the same reader (not a preference).
    utc_offset_minutes: Option<i64>,
}

static GLOBAL: OnceLock<GamePrefs> = OnceLock::new();

impl GamePrefs {
    /// Parses the tab-separated dump produced by the reader script: one
    /// `kind<TAB>name<TAB>value` line per value (`i` int, `f` float, `s` string;
    /// `z` is the machine's UTC offset in minutes).
    #[must_use]
    pub fn parse(dump: &str) -> Self {
        let mut values = BTreeMap::new();
        let mut utc_offset_minutes = None;
        for line in dump.lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(kind), Some(name), Some(raw)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let raw = raw.trim_end_matches('\r');
            let value = match kind {
                "i" => raw.trim().parse::<i64>().ok().map(PrefValue::Int),
                "f" => raw.trim().parse::<f64>().ok().map(PrefValue::Float),
                "s" => Some(PrefValue::Text(raw.to_owned())),
                "z" => {
                    utc_offset_minutes = raw.trim().parse::<i64>().ok();
                    None
                }
                _ => None,
            };
            if let Some(value) = value {
                values.insert(name.to_owned(), value);
            }
        }
        Self {
            values,
            utc_offset_minutes,
        }
    }

    /// The machine's local-time offset from UTC in minutes (`DateTime.Now` in the game),
    /// when the reader could report it.
    #[must_use]
    pub fn utc_offset_minutes(&self) -> Option<i64> {
        self.utc_offset_minutes
    }

    /// Reads the registry (Windows only); empty elsewhere or on any failure.
    #[must_use]
    pub fn load() -> Self {
        // Deterministic runs (CI, golden screenshots) can ignore the machine's game.
        if std::env::var_os("RUSTIBITES_IGNORE_GAME_PREFS").is_some() {
            return Self::default();
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let script = READER_SCRIPT;
            let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let output = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-EncodedCommand",
                    &base64(&utf16),
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            if let Ok(output) = output {
                return Self::parse(&String::from_utf8_lossy(&output.stdout));
            }
        }
        Self::default()
    }

    /// Process-wide snapshot (read once; blocks while another thread is reading).
    #[must_use]
    pub fn global() -> &'static Self {
        GLOBAL.get_or_init(Self::load)
    }

    /// Starts reading the registry on a background thread so a later
    /// [`GamePrefs::global`] does not wait.
    pub fn prefetch() {
        std::thread::spawn(|| {
            let _ = GamePrefs::global();
        });
    }

    /// Raw value.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&PrefValue> {
        self.values.get(name)
    }

    /// Integer value (`PlayerPrefs.GetInt`).
    #[must_use]
    pub fn int(&self, name: &str) -> Option<i64> {
        match self.values.get(name)? {
            PrefValue::Int(value) => Some(*value),
            PrefValue::Float(value) => Some(*value as i64),
            PrefValue::Text(_) => None,
        }
    }

    /// Float value (`PlayerPrefs.GetFloat`).
    #[must_use]
    pub fn float(&self, name: &str) -> Option<f64> {
        match self.values.get(name)? {
            PrefValue::Float(value) => Some(*value),
            PrefValue::Int(value) => Some(*value as f64),
            PrefValue::Text(_) => None,
        }
    }

    /// Boolean value (`GetInt(..) == 1`).
    #[must_use]
    pub fn flag(&self, name: &str) -> Option<bool> {
        self.int(name).map(|value| value == 1)
    }

    /// String value (`PlayerPrefs.GetString`).
    #[must_use]
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.values.get(name)? {
            PrefValue::Text(value) => Some(value),
            _ => None,
        }
    }

    /// Number of stored values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when nothing could be read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// PowerShell that prints every value of the game's key as `kind<TAB>name<TAB>value`.
#[cfg(windows)]
const READER_SCRIPT: &str = r"
$inv = [Globalization.CultureInfo]::InvariantCulture
'z' + [char]9 + 'utc' + [char]9 + ([int][TimeZoneInfo]::Local.GetUtcOffset([DateTime]::UtcNow).TotalMinutes).ToString($inv)
$k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\The Bibites\The Bibites')
if ($k) {
  foreach ($n in $k.GetValueNames()) {
    $v = $k.GetValue($n)
    $b = $n -replace '_h\d+$', ''
    if ($v -is [int]) { 'i' + [char]9 + $b + [char]9 + $v.ToString($inv) }
    elseif ($v -is [long]) { 'f' + [char]9 + $b + [char]9 + [BitConverter]::Int64BitsToDouble($v).ToString('R', $inv) }
    elseif ($v -is [byte[]]) { 's' + [char]9 + $b + [char]9 + ([Text.Encoding]::UTF8.GetString($v).TrimEnd([char]0) -replace '[\r\n]', ' ') }
  }
}
";

#[cfg(windows)]
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dump_lines() {
        let prefs = GamePrefs::parse(
            "i\tAutoSave\t1\nf\tFertilityColorR\t0.90737247467041\ns\tSavePath\tC:\\Users\\x\\Downloads\nx\tbad\t1\nbroken\n",
        );
        assert_eq!(prefs.flag("AutoSave"), Some(true));
        assert!((prefs.float("FertilityColorR").unwrap() - 0.907_372_474_670_41).abs() < 1e-12);
        assert_eq!(prefs.text("SavePath"), Some("C:\\Users\\x\\Downloads"));
        assert_eq!(prefs.len(), 3);
        assert!(prefs.get("bad").is_none());
        assert_eq!(prefs.utc_offset_minutes(), None);
    }

    #[test]
    fn parses_the_utc_offset_line_without_counting_it_as_a_preference() {
        let prefs = GamePrefs::parse("z\tutc\t-300\ni\tAutoSave\t1\n");
        assert_eq!(prefs.utc_offset_minutes(), Some(-300));
        assert_eq!(prefs.len(), 1);
        assert!(!prefs.is_empty());
        assert!(GamePrefs::parse("z\tutc\t330\n").is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
