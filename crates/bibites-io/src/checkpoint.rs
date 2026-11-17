//! Checkpoint sidecar (`<save>.zip.bb8state`) for bit-identical world resumes.
//!
//! The game save `.zip` is never modified: the runtime state that the game
//! format drops (RNG streams, clock/timers and phase-2 extensions) travels in a
//! sibling file. The payload is `bibites_core::sim::SimSnapshot::encode()`;
//! this module only adds an identity/checksum header.
//!
//! Header layout (little-endian, 88 bytes + payload + checksum):
//!
//! | field | bytes |
//! |---|---|
//! | magic `BB8STATE` | 8 |
//! | version (1) | u16 |
//! | flags (0) | u16 |
//! | base save SHA-256 | 32 |
//! | settings SHA-256 (bridge `/settings` canonical hash) | 32 |
//! | world seed | u64 |
//! | core payload length | u32 |
//! | core payload (`SimSnapshot::encode`) | len |
//! | FNV-1a 64 checksum over header through payload | u64 |
//!
//! Base hash + settings hash make "wrong sidecar for this save" a distinct
//! error from "corrupt sidecar/core".

use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use bibites_core::sim::{SimSnapshot, SnapshotError, SnapshotSection};
use bibites_core::{Error, Result};

use crate::settings::to_bridge_settings_json;

/// Sidecar magic bytes.
pub const BB8STATE_MAGIC: [u8; 8] = *b"BB8STATE";
/// Sidecar format version.
pub const BB8STATE_VERSION: u16 = 1;
/// Experimental chunked/streamed sidecar version (opt-in; v1 stays default).
pub const BB8STATE_VERSION_V2: u16 = 2;
/// Chunk size used by the experimental v2 writer (16 MiB).
pub const BB8STATE_V2_CHUNK: usize = 16 * 1024 * 1024;
/// Header bytes before the core payload.
pub const BB8STATE_HEADER_LEN: usize = 88;
/// Checksum bytes after the payload.
pub const BB8STATE_CHECKSUM_LEN: usize = 8;

/// Returns the sibling sidecar path for a save zip (`foo.zip` → `foo.zip.bb8state`).
#[must_use]
pub fn checkpoint_path(base_save: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bb8state", base_save.display()))
}

/// Writes the checkpoint sidecar for `base_save` with a seed of `0`.
///
/// Use [`write_checkpoint_state_with_seed`] when the CLI knows the run seed.
pub fn write_checkpoint_state(base_save: &Path, snapshot: &SimSnapshot) -> Result<()> {
    write_checkpoint_state_with_seed(base_save, snapshot, 0)
}

/// Writes the checkpoint sidecar for `base_save` with an explicit world seed.
pub fn write_checkpoint_state_with_seed(
    base_save: &Path,
    snapshot: &SimSnapshot,
    world_seed: u64,
) -> Result<()> {
    if v2_enabled() {
        return write_checkpoint_state_v2(base_save, snapshot, world_seed);
    }
    let path = checkpoint_path(base_save);
    let header = Header {
        version: BB8STATE_VERSION,
        flags: 0,
        base_sha256: sha256_file(base_save)?,
        settings_sha256: settings_sha256_of_save(base_save)?,
        world_seed,
    };
    let payload = snapshot
        .encode()
        .map_err(|error| Error::invalid_data(format!("checkpoint core: {error}")))?;
    let mut bytes = Vec::with_capacity(BB8STATE_HEADER_LEN + payload.len() + BB8STATE_CHECKSUM_LEN);
    bytes.extend_from_slice(&BB8STATE_MAGIC);
    bytes.extend_from_slice(&header.version.to_le_bytes());
    bytes.extend_from_slice(&header.flags.to_le_bytes());
    bytes.extend_from_slice(&header.base_sha256);
    bytes.extend_from_slice(&header.settings_sha256);
    bytes.extend_from_slice(&header.world_seed.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&payload);
    let checksum = fnv1a64(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    let tmp = path.with_extension("bb8state.tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Reads a sidecar; `Ok(None)` only when the sibling file is absent.
pub fn read_checkpoint_state(base_save: &Path) -> Result<Option<SimSnapshot>> {
    read_checkpoint_state_inner(base_save, None)
}

/// Reads a sidecar and verifies the recorded world seed too.
pub fn read_checkpoint_state_checked(
    base_save: &Path,
    expected_seed: u64,
) -> Result<Option<SimSnapshot>> {
    read_checkpoint_state_inner(base_save, Some(expected_seed))
}

fn read_checkpoint_state_inner(
    base_save: &Path,
    expected_seed: Option<u64>,
) -> Result<Option<SimSnapshot>> {
    if v2_enabled() {
        return read_checkpoint_state_v2(base_save, expected_seed);
    }
    let path = checkpoint_path(base_save);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::from(error)),
    };
    if bytes.len() < BB8STATE_HEADER_LEN + BB8STATE_CHECKSUM_LEN {
        return Err(Error::invalid_data("checkpoint header: truncated sidecar"));
    }
    if bytes[..8] != BB8STATE_MAGIC {
        return Err(Error::invalid_data("checkpoint header: bad magic"));
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != BB8STATE_VERSION {
        return Err(Error::invalid_data(format!(
            "checkpoint header: unsupported version {version}"
        )));
    }
    let flags = u16::from_le_bytes([bytes[10], bytes[11]]);
    if flags != 0 {
        return Err(Error::invalid_data(format!(
            "checkpoint header: unsupported flags {flags}"
        )));
    }
    let mut base_sha = [0u8; 32];
    base_sha.copy_from_slice(&bytes[12..44]);
    let mut settings_sha = [0u8; 32];
    settings_sha.copy_from_slice(&bytes[44..76]);
    let world_seed = u64::from_le_bytes(bytes[76..84].try_into().expect("u64"));
    let core_len = u32::from_le_bytes(bytes[84..88].try_into().expect("u32")) as usize;
    let end = BB8STATE_HEADER_LEN + core_len;
    if end + BB8STATE_CHECKSUM_LEN != bytes.len() {
        return Err(Error::invalid_data(
            "checkpoint header: length/trailing data mismatch",
        ));
    }
    let stored_checksum = u64::from_le_bytes(bytes[end..].try_into().expect("u64"));
    if stored_checksum != fnv1a64(&bytes[..end]) {
        return Err(Error::invalid_data("checkpoint header: checksum mismatch"));
    }
    if sha256_file(base_save)? != base_sha {
        return Err(Error::invalid_data("checkpoint base mismatch"));
    }
    if settings_sha256_of_save(base_save)? != settings_sha {
        return Err(Error::invalid_data("checkpoint settings mismatch"));
    }
    if let Some(expected) = expected_seed
        && world_seed != expected
    {
        return Err(Error::invalid_data("checkpoint seed mismatch"));
    }
    SimSnapshot::decode(&bytes[BB8STATE_HEADER_LEN..end])
        .map(Some)
        .map_err(|error: SnapshotError| Error::invalid_data(format!("checkpoint core: {error}")))
}

struct Header {
    version: u16,
    flags: u16,
    base_sha256: [u8; 32],
    settings_sha256: [u8; 32],
    world_seed: u64,
}

// ---------------------------------------------------------------------------
// Experimental v2: streamed sections (opt-in; v1 is the default)
// ---------------------------------------------------------------------------

/// Opt-in switch for the v2 streamed sidecar. Set `RUSTIBITES_CHECKPOINT_V2=1`
/// (or `v2`/`true`) to make the public [`write_checkpoint_state`] /
/// [`read_checkpoint_state`] APIs use v2. The v2 format is a sibling file; the
/// game zip is never modified.
fn v2_enabled() -> bool {
    std::env::var("RUSTIBITES_CHECKPOINT_V2").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "v2"
        )
    })
}

/// v2 header: magic/version/flags, base zip sha256, settings sha256, seed,
/// section count (88 bytes).
const V2_HEADER: usize = 8 + 2 + 2 + 32 + 32 + 8 + 4;
/// Per-section record: tag u16 + length u32 + FNV-1a checksum u64.
const V2_SECTION_PREFIX: usize = 2 + 4 + 8;

/// Writes an experimental v2 sidecar by streaming `SimSnapshot::encode_sections()`
/// straight to the file (control preamble + extension sections, including the
/// core's 64 MiB chunked sections). No contiguous v1 payload is built.
pub fn write_checkpoint_state_v2(
    base_save: &Path,
    snapshot: &SimSnapshot,
    world_seed: u64,
) -> Result<()> {
    use std::io::Write as _;
    let path = checkpoint_path(base_save);
    let base_sha = sha256_file(base_save)?;
    let settings_sha = settings_sha256_of_save(base_save)?;
    let sections: Vec<SnapshotSection<'_>> = snapshot
        .encode_sections()
        .map_err(|error| Error::invalid_data(format!("checkpoint core: {error}")))?
        .collect();
    let tmp = path.with_extension("bb8state.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(&BB8STATE_MAGIC)?;
    file.write_all(&BB8STATE_VERSION_V2.to_le_bytes())?;
    file.write_all(&0u16.to_le_bytes())?;
    file.write_all(&base_sha)?;
    file.write_all(&settings_sha)?;
    file.write_all(&world_seed.to_le_bytes())?;
    file.write_all(&(sections.len() as u32).to_le_bytes())?;
    for section in &sections {
        let data = section.data.as_ref();
        file.write_all(&section.tag.to_le_bytes())?;
        file.write_all(&(data.len() as u32).to_le_bytes())?;
        file.write_all(&fnv1a64(data).to_le_bytes())?;
        file.write_all(data)?;
    }
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Reads an experimental v2 sidecar and feeds its sections directly to
/// `SimSnapshot::decode_sections` (no concatenated v1 payload buffer). Verifies
/// magic/version/flags, base/settings/seed identity and every per-section
/// checksum; `Ok(None)` only when the sibling file is absent.
pub fn read_checkpoint_state_v2(
    base_save: &Path,
    expected_seed: Option<u64>,
) -> Result<Option<SimSnapshot>> {
    let path = checkpoint_path(base_save);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::from(error)),
    };
    if bytes.len() < V2_HEADER {
        return Err(Error::invalid_data(
            "checkpoint header: truncated v2 sidecar",
        ));
    }
    if bytes[..8] != BB8STATE_MAGIC {
        return Err(Error::invalid_data("checkpoint header: bad magic"));
    }
    if u16::from_le_bytes([bytes[8], bytes[9]]) != BB8STATE_VERSION_V2 {
        return Err(Error::invalid_data("checkpoint header: not a v2 sidecar"));
    }
    if u16::from_le_bytes([bytes[10], bytes[11]]) != 0 {
        return Err(Error::invalid_data("checkpoint header: unsupported flags"));
    }
    let mut base_sha = [0u8; 32];
    base_sha.copy_from_slice(&bytes[12..44]);
    let mut settings_sha = [0u8; 32];
    settings_sha.copy_from_slice(&bytes[44..76]);
    let world_seed = u64::from_le_bytes(bytes[76..84].try_into().expect("u64"));
    let section_count = u32::from_le_bytes(bytes[84..88].try_into().expect("u32")) as usize;
    if sha256_file(base_save)? != base_sha {
        return Err(Error::invalid_data("checkpoint base mismatch"));
    }
    if settings_sha256_of_save(base_save)? != settings_sha {
        return Err(Error::invalid_data("checkpoint settings mismatch"));
    }
    if let Some(expected) = expected_seed
        && world_seed != expected
    {
        return Err(Error::invalid_data("checkpoint seed mismatch"));
    }
    let mut offset = V2_HEADER;
    let mut sections = Vec::with_capacity(section_count);
    for _ in 0..section_count {
        if offset + V2_SECTION_PREFIX > bytes.len() {
            return Err(Error::invalid_data("checkpoint header: truncated section"));
        }
        let tag = u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16"));
        let len =
            u32::from_le_bytes(bytes[offset + 2..offset + 6].try_into().expect("u32")) as usize;
        let checksum = u64::from_le_bytes(bytes[offset + 6..offset + 14].try_into().expect("u64"));
        offset += V2_SECTION_PREFIX;
        if offset + len > bytes.len() {
            return Err(Error::invalid_data(
                "checkpoint header: truncated section data",
            ));
        }
        let data = &bytes[offset..offset + len];
        if fnv1a64(data) != checksum {
            return Err(Error::invalid_data("checkpoint chunk checksum mismatch"));
        }
        sections.push(SnapshotSection {
            tag,
            data: std::borrow::Cow::Borrowed(data),
        });
        offset += len;
    }
    if offset != bytes.len() {
        return Err(Error::invalid_data(
            "checkpoint header: length/trailing data mismatch",
        ));
    }
    SimSnapshot::decode_sections(sections)
        .map(Some)
        .map_err(|error: SnapshotError| Error::invalid_data(format!("checkpoint core: {error}")))
}

fn sha256_file(path: &Path) -> Result<[u8; 32]> {
    // Stream the save zip: at 100k-entity sizes it can be hundreds of MiB and a
    // whole-file read is a pure extra allocation (the real identity check only
    // needs the digest).
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

/// Settings hash of the save: the same canonical bridge-settings digest the
/// parity capture emits (`docs/PARITY.md` §6.3), full 32-byte digest.
fn settings_sha256_of_save(base_save: &Path) -> Result<[u8; 32]> {
    // Only the settings member is needed. Parsing the full bundle here would
    // deserialize every entity/pellet/species section just to hash 92 values.
    use std::io::Read as _;
    let file = std::fs::File::open(base_save)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        Error::invalid_data(format!("checkpoint settings: invalid zip: {error}"))
    })?;
    let mut bytes = None;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| Error::invalid_data(format!("checkpoint settings: {error}")))?;
        let name = entry.name().replace('\\', "/");
        if name == "settings.bb8settings" || name == "settings.json" {
            let mut data = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut data)
                .map_err(|error| Error::Io(error.to_string()))?;
            bytes = Some(data);
            break;
        }
    }
    let bytes = bytes
        .ok_or_else(|| Error::invalid_data("checkpoint settings: save has no settings section"))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| Error::invalid_data(format!("checkpoint settings: not UTF-8: {error}")))?;
    let settings = crate::settings::parse_settings_json(text)?.effective_settings();
    Ok(settings_sha256(&to_bridge_settings_json(&settings)))
}

fn settings_sha256(settings: &Value) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for scope in ["scenario", "independent"] {
        hasher.update(scope.as_bytes());
        hasher.update(b"{");
        if let Some(values) = settings.get(scope).and_then(Value::as_object) {
            let mut keys: Vec<&String> = values.keys().collect();
            keys.sort_unstable();
            for key in keys {
                let value = &values[key.as_str()];
                hasher.update(key.as_bytes());
                hasher.update(b"=");
                if let Some(flag) = value.as_bool() {
                    hasher.update(b"b");
                    hasher.update([u8::from(flag)]);
                } else if let Some(integer) = value.as_i64() {
                    hasher.update(b"i");
                    hasher.update((integer as i32).to_le_bytes());
                } else {
                    hasher.update(b"f");
                    hasher.update((value.as_f64().unwrap_or(0.0) as f32).to_le_bytes());
                }
                hasher.update(b";");
            }
        }
        hasher.update(b"}");
    }
    hasher.finalize().into()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
