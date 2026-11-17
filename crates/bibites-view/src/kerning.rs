//! Pair kerning of the game's TextMeshPro font assets.
//!
//! TextMeshPro moves a character's successor by the pair adjustments of the font asset's `fontFeatureTable` ("To" sits 1.3 px closer at
//! 25 px than the sum of the two advances in `Roboto-Bold SDF NoOutline`). Only two of the game's assets that hold characters have such a
//! table: `LiberationSans SDF` (104 pairs) and `Roboto-Bold SDF NoOutline` (505). `Roboto-Regular SDF` (the body face of most of the UI),
//! `Roboto-Bold SDF WhiteOutline`, `Boston Black SDF` and `Inconsolata-SemiBold SDF` have none: their text is not kerned at all (a captured
//! inspector title "Tag:" in `Roboto-Regular SDF` is the plain sum of its advances, "Total Damages Suffered:" in Liberation is 2.6 px
//! narrower than that).
//!
//! The dump's `kerningTable` is the legacy one and is empty. The pair tables were read from the running game by the parity plugin
//! (`/d/font_kerning`) into `<asset>_kerning.json` next to the asset's dump: `[first, second, first.xAdvance]` per pair, in the asset's
//! point-size units (what TMP scales by `fontSize / pointSize` like an advance).

use std::collections::HashMap;
use std::path::Path;

/// The kerning of every pair of the font asset whose dump is `json_path`, in ems: what is added to the first character's advance. Empty
/// when the asset has no pair table (no `<asset>_kerning.json` next to its dump).
#[must_use]
pub fn pairs_for_dump(json_path: &Path) -> HashMap<(char, char), f32> {
    let Some(stem) = json_path.file_stem().and_then(|stem| stem.to_str()) else {
        return HashMap::new();
    };
    let Ok(text) = std::fs::read_to_string(json_path.with_file_name(format!("{stem}_kerning.json"))) else {
        return HashMap::new();
    };
    parse(&text)
}

/// The pairs of one `_kerning.json` file, in ems.
fn parse(text: &str) -> HashMap<(char, char), f32> {
    let mut table = HashMap::new();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return table;
    };
    let point_size = value
        .get("pointSize")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0) as f32;
    if point_size <= 0.0 {
        return table;
    }
    let Some(pairs) = value.get("pairs").and_then(serde_json::Value::as_array) else {
        return table;
    };
    for pair in pairs {
        let Some(row) = pair.as_array() else {
            continue;
        };
        let character = |index: usize| {
            let mut chars = row.get(index)?.as_str()?.chars();
            let first = chars.next()?;
            chars.next().is_none().then_some(first)
        };
        if let (Some(first), Some(second), Some(advance)) = (
            character(0),
            character(1),
            row.get(2).and_then(serde_json::Value::as_f64),
        ) && advance != 0.0
        {
            table.insert((first, second), advance as f32 / point_size);
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("assets/game/ui_fonts")
    }

    #[test]
    fn a_pair_file_gives_its_values_in_ems() {
        let table = parse(r#"{"pointSize":90.0,"pairs":[["T","o",-3.5],["A","V",-4.5],["x","y",0.0],["ab","c",-1.0]]}"#);
        assert_eq!(table.len(), 2, "a zero adjustment and a multi-character key are left out: {table:?}");
        assert!((table[&('T', 'o')] - -3.5 / 90.0).abs() < 1e-6);
        assert!((table[&('A', 'V')] - -0.05).abs() < 1e-6);
        assert!(parse("not json").is_empty());
        assert!(parse(r#"{"pairs":[["T","o",-3.5]]}"#).is_empty(), "no point size");
    }

    #[test]
    fn only_the_assets_the_game_gave_a_pair_table_kern() {
        let dir = fonts_dir();
        if !dir.join("Roboto-Bold_SDF_NoOutline_kerning.json").is_file() {
            eprintln!("kerning tables not present; skipping");
            return;
        }
        // Roboto Bold (point size 90): the o sits 3.47 units under a capital T; letters that sit apart have no pair.
        let bold = pairs_for_dump(&dir.join("Roboto-Bold_SDF_NoOutline.json"));
        assert!((bold[&('T', 'o')] - -3.468_75 / 90.0).abs() < 1e-5, "{:?}", bold.get(&('T', 'o')));
        assert!(!bold.contains_key(&('i', 'l')));
        // Liberation (point size 86).
        let liberation = pairs_for_dump(&dir.join("LiberationSans_SDF.json"));
        assert!((liberation[&('T', 'a')] - -9.531_25 / 86.0).abs() < 1e-5);
        assert!((liberation[&('f', 'f')] - -1.546_875 / 86.0).abs() < 1e-5);
        assert!(!liberation.contains_key(&('D', 'a')));
        // The assets without a table do not kern (the plain Roboto one included).
        for name in [
            "Roboto-Regular_SDF.json",
            "Roboto-Bold_SDF_WhiteOutline.json",
            "Boston_Black_SDF.json",
        ] {
            assert!(pairs_for_dump(&dir.join(name)).is_empty(), "{name}");
        }
    }
}
