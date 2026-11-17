//! `.bb8state` checkpoint sidecar (task-75 P3): round-trip and rejection tests.

use std::path::{Path, PathBuf};

use bibites_core::sim::SimSnapshot;
use bibites_io::{
    checkpoint_path, read_checkpoint_state, read_checkpoint_state_checked,
    read_checkpoint_state_v2, write_checkpoint_state, write_checkpoint_state_v2,
    write_checkpoint_state_with_seed,
};

fn base_save() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/real/saves/lol.zip");
    path.is_file().then_some(path)
}

fn scratch(name: &str) -> PathBuf {
    // Unique per test: tests run in parallel and remove their own directory.
    let dir = std::env::temp_dir().join(format!(
        "rustibites_bb8state_{}_{}",
        std::process::id(),
        name.replace(&['.', '/', '\\'][..], "_")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn snapshot() -> SimSnapshot {
    SimSnapshot {
        tick: 123,
        simulated_time: 4.1,
        sim_tps: 30,
        ..SimSnapshot::default()
    }
}

#[test]
fn sidecar_round_trip_and_base_zip_untouched() {
    let Some(base) = base_save() else {
        eprintln!("skipping: lol.zip absent");
        return;
    };
    let copy = scratch("base.zip");
    std::fs::copy(&base, &copy).unwrap();
    let before = std::fs::read(&copy).unwrap();
    let sidecar = checkpoint_path(&copy);
    assert_eq!(
        sidecar.to_string_lossy(),
        format!("{}.bb8state", copy.display())
    );

    let original = snapshot();
    write_checkpoint_state_with_seed(&copy, &original, 12_345).unwrap();
    assert!(sidecar.is_file());
    let after = std::fs::read(&copy).unwrap();
    assert_eq!(
        before, after,
        "game zip must not change when writing a sidecar"
    );

    let loaded = read_checkpoint_state_checked(&copy, 12_345)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.encode().unwrap(), original.encode().unwrap());
    assert_eq!(
        loaded.encode().unwrap(),
        read_checkpoint_state(&copy)
            .unwrap()
            .unwrap()
            .encode()
            .unwrap()
    );
    let _ = std::fs::remove_dir_all(copy.parent().unwrap());
}

#[test]
fn missing_sidecar_returns_none() {
    let Some(base) = base_save() else {
        return;
    };
    let copy = scratch("none.zip");
    std::fs::copy(&base, &copy).unwrap();
    assert!(read_checkpoint_state(&copy).unwrap().is_none());
    assert!(read_checkpoint_state_checked(&copy, 1).unwrap().is_none());
    let _ = std::fs::remove_dir_all(copy.parent().unwrap());
}

#[test]
fn wrong_base_and_wrong_seed_are_rejected() {
    let Some(base) = base_save() else {
        return;
    };
    let copy = scratch("wrong.zip");
    std::fs::copy(&base, &copy).unwrap();
    write_checkpoint_state_with_seed(&copy, &snapshot(), 7).unwrap();

    let error = read_checkpoint_state_checked(&copy, 8)
        .unwrap_err()
        .to_string();
    assert!(error.contains("seed"), "{error}");

    let mut bytes = std::fs::read(&copy).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    std::fs::write(&copy, bytes).unwrap();
    let error = read_checkpoint_state(&copy).unwrap_err().to_string();
    assert!(error.contains("base"), "{error}");
    let _ = std::fs::remove_dir_all(copy.parent().unwrap());
}

#[test]
fn corrupt_and_truncated_sidecars_are_rejected() {
    let Some(base) = base_save() else {
        return;
    };
    let copy = scratch("corrupt.zip");
    std::fs::copy(&base, &copy).unwrap();
    write_checkpoint_state_with_seed(&copy, &snapshot(), 42).unwrap();
    let sidecar = checkpoint_path(&copy);

    let mut bytes = std::fs::read(&sidecar).unwrap();
    let payload_byte = 88 + 4; // inside the core payload
    bytes[payload_byte] ^= 0x01;
    std::fs::write(&sidecar, &bytes).unwrap();
    let error = read_checkpoint_state(&copy).unwrap_err().to_string();
    assert!(
        error.contains("checksum") || error.contains("core"),
        "{error}"
    );

    let truncated = &bytes[..40];
    std::fs::write(&sidecar, truncated).unwrap();
    let error = read_checkpoint_state(&copy).unwrap_err().to_string();
    assert!(
        error.contains("truncated") || error.contains("header"),
        "{error}"
    );
    let _ = std::fs::remove_dir_all(copy.parent().unwrap());
}

#[test]
fn wrong_settings_hash_is_rejected() {
    let Some(base) = base_save() else {
        return;
    };
    let copy = scratch("settings.zip");
    std::fs::copy(&base, &copy).unwrap();
    write_checkpoint_state_with_seed(&copy, &snapshot(), 5).unwrap();
    let sidecar = checkpoint_path(&copy);
    let mut bytes = std::fs::read(&sidecar).unwrap();
    // Flip one byte inside the settings_sha256 field, then re-sign the sidecar
    // so the dedicated settings check (not the checksum) rejects it.
    bytes[44] ^= 0x01;
    let core_len = u32::from_le_bytes(bytes[84..88].try_into().unwrap()) as usize;
    let end = 88 + core_len;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in &bytes[..end] {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    bytes[end..].copy_from_slice(&hash.to_le_bytes());
    std::fs::write(&sidecar, &bytes).unwrap();
    let error = read_checkpoint_state(&copy).unwrap_err().to_string();
    assert!(error.contains("settings"), "{error}");
    let _ = std::fs::remove_dir_all(copy.parent().unwrap());
}

#[test]
fn absent_base_sidecar_api_is_safe() {
    let bogus = scratch("bogus.zip");
    std::fs::write(&bogus, b"not a save").unwrap();
    assert!(write_checkpoint_state(&bogus, &snapshot()).is_err());
    let _ = std::fs::remove_dir_all(bogus.parent().unwrap());
}

#[test]
fn v2_streamed_sidecar_round_trips_and_rejects_corruption() {
    let Some(base) = base_save() else {
        return;
    };
    let copy = scratch("v2.zip");
    std::fs::copy(&base, &copy).unwrap();
    let before = std::fs::read(&copy).unwrap();
    let original = snapshot();
    write_checkpoint_state_v2(&copy, &original, 999).unwrap();
    let loaded = read_checkpoint_state_v2(&copy, Some(999)).unwrap().unwrap();
    assert_eq!(loaded.encode().unwrap(), original.encode().unwrap());
    assert_eq!(
        std::fs::read(&copy).unwrap(),
        before,
        "game zip must not change"
    );
    assert!(read_checkpoint_state_checked(&copy, 999).is_err());
    let sidecar = checkpoint_path(&copy);
    let mut bytes = std::fs::read(&sidecar).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&sidecar, &bytes).unwrap();
    let error = read_checkpoint_state_v2(&copy, None)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("checksum") || error.contains("core"),
        "{error}"
    );
    std::fs::write(&sidecar, &bytes[..40]).unwrap();
    assert!(read_checkpoint_state_v2(&copy, None).is_err());
    let _ = std::fs::remove_dir_all(copy.parent().unwrap());
}

#[test]
fn v2_streams_chunked_sections_and_round_trips() {
    let Some(base) = base_save() else {
        return;
    };
    let copy = scratch("v2_sections.zip");
    std::fs::copy(&base, &copy).unwrap();
    let mut snapshot = snapshot();
    // Larger than the core's 8 MiB chunk size: exercises multiple chunk sections.
    let payload = vec![7u8; 9 * 1024 * 1024];
    snapshot
        .push_section(0x0101, payload.clone())
        .expect("push section");
    write_checkpoint_state_v2(&copy, &snapshot, 77).unwrap();
    let loaded = read_checkpoint_state_v2(&copy, Some(77)).unwrap().unwrap();
    assert_eq!(
        loaded.extension_payload(0x0101).expect("chunked payload"),
        payload.as_slice()
    );
    let _ = std::fs::remove_dir_all(copy.parent().unwrap());
}
