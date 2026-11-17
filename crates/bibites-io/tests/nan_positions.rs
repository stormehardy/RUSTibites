//! A save written by a build before the NaN fix of 2026-11-13 can hold bibites and eggs whose position is `null` (serde writes a NaN as
//! `null`): the loader used to fail on them ("invalid type: null, expected f32"). They are left out of the loaded world.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use bibites_io::load_world_contents;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bundles/save_min.zip")
}

/// Copies `save_min.zip` with `edit(entry name, bytes)` applied to every entry; returns the path of the copy.
fn rewritten(tag: &str, edit: impl Fn(&str, Vec<u8>) -> Vec<u8>) -> PathBuf {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(fixture()).unwrap()).unwrap();
    let out = std::env::temp_dir().join(format!("bibites_io_nan_{}_{tag}.zip", std::process::id()));
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let name = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        let bytes = edit(&name, bytes);
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&bytes).unwrap();
    }
    writer.finish().unwrap();
    out
}

fn replaced(bytes: Vec<u8>, old: &str, new: &str) -> Vec<u8> {
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains(old), "the fixture has no `{old}`");
    text.replacen(old, new, 1).into_bytes()
}

#[test]
fn bibites_and_eggs_with_a_null_position_are_left_out_not_a_load_error() {
    let plain = load_world_contents(fixture(), None).unwrap();
    assert_eq!((plain.bibites.len(), plain.eggs.len()), (1, 1), "the fixture holds a bibite and an egg");

    // Compact, as the port wrote it (`"position":[null,null]` at the start of the file): left out without the parse.
    let path = rewritten("compact", |name, bytes| match name {
        "bibites/bibite_0.bb8" => replaced(bytes, "\"position\": [\r\n      10.0,\r\n      -20.0\r\n    ]", "\"position\":[null,null]"),
        _ => bytes,
    });
    let contents = load_world_contents(&path, None).unwrap();
    assert_eq!(contents.bibites.len(), 0, "the bibite of NaN position is left out");
    assert_eq!(contents.eggs.len(), 1, "the egg is not touched");
    let _ = std::fs::remove_file(&path);

    // Spelled with spaces: parsed as usual, and still left out (the position reads as NaN).
    let path = rewritten("spaced", |name, bytes| match name {
        "bibites/bibite_0.bb8" => replaced(bytes, "\"position\": [\r\n      10.0,\r\n      -20.0\r\n    ]", "\"position\": [null, null]"),
        "eggs/egg_0.bb8" => replaced(bytes, "\"position\": [5.0, 6.0]", "\"position\": [null, null]"),
        _ => bytes,
    });
    let contents = load_world_contents(&path, None).unwrap();
    assert_eq!((contents.bibites.len(), contents.eggs.len()), (0, 0), "both are left out");
    let _ = std::fs::remove_file(&path);

    // A bibite whose size is not a number (`d2Size` null: a dead body of NaN size) is left out too; the rest of the world loads.
    let path = rewritten("size", |name, bytes| match name {
        "bibites/bibite_0.bb8" => {
            let text = String::from_utf8(bytes).unwrap();
            let at = text.find("\"d2Size\"").expect("the fixture has a d2Size");
            let end = text[at..].find(|c| c == ',' || c == '\r' || c == '\n').map(|n| at + n).unwrap();
            let mut edited = text.clone();
            edited.replace_range(at..end, "\"d2Size\": null");
            edited.into_bytes()
        }
        _ => bytes,
    });
    let contents = load_world_contents(&path, None).unwrap();
    assert_eq!((contents.bibites.len(), contents.eggs.len()), (0, 1), "the bibite of no size is left out");
    let _ = std::fs::remove_file(&path);
}
