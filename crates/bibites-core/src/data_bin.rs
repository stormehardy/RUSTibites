//! The framing of a save's `data.bin` (`SaveableBinStack`, `Here/ScriptHelpers/SaveableBinStack.cs`).
//!
//! The file is a stack of blocks, each a little-endian `i32` length and that many bytes: the `DataLogger`
//! (the Graphs panel's streams), the `BibiteTracker`, the `GlobalLineageManager` (the species logs) and the
//! `ZoneManager` (the zones' logs). The core only reads and writes the species and the zone blocks; the
//! others are kept as they were loaded.
//!
//! The species block is a list of `i64` species id, `i32` length and the species' log
//! ([`crate::species_log::SpeciesLog::save_state_bin`]); the zone block one of `i32` zone id, `i32` length and
//! the zone's log ([`crate::zone_log::ZoneLog::save_state_bin`]).

const BLANK_DATA_LOGGER: [u8; 188] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x04, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x14, 0x00,
    0x00, 0x00, 0x23, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x04, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

const BLANK_BIBITE_TRACKER: [u8; 46] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// The block of the Graphs streams (`DataLogger`).
pub const LOGGER_BLOCK: usize = 0;

/// The block of the tracker (`BibiteTracker`): the births and deaths since the last point and the estimator of the ages of death.
pub const TRACKER_BLOCK: usize = 1;

/// The first game version whose logger and tracker blocks are in the format the core reads (`DataParallelStreamGroup.LoadStateBin`
/// has another one before `0.6a9`).
const LOGGER_HISTORY_SINCE: Version = Version {
    numbers: [0, 6, 0, 0],
    alpha: 9,
};

/// The block of the species logs (`GlobalLineageManager`).
pub const SPECIES_BLOCK: usize = 2;

/// The block of the zone logs (`ZoneManager`).
pub const ZONE_BLOCK: usize = 3;

/// The first game version that reads the zone block (`SaveableBinElement(ZoneManager.instance, new
/// Version(0, 6, 0, 10))`: 0.6.0a10).
const ZONE_BLOCK_SINCE: Version = Version {
    numbers: [0, 6, 0, 0],
    alpha: 10,
};

/// The first game version that reads the species block (`SaveableBinElement(GlobalLineageManager.Instance,
/// new Version(0, 6, 0, 15))`: 0.6.0a15).
const SPECIES_BLOCK_SINCE: Version = Version {
    numbers: [0, 6, 0, 0],
    alpha: 15,
};

/// `Utility.Version`: four numbers and an alpha counter (0 for a release).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    /// `V0`..`V3`.
    pub numbers: [i32; 4],
    /// `Alpha`: `0.6.3a3` has alpha 3, `0.6.3` has 0 and is newer than any alpha of it.
    pub alpha: i32,
}

impl Version {
    /// `Version.Parse`: every run of digits is a number; with an `a` in the text the last one is the alpha.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut runs: Vec<i32> = Vec::new();
        let mut current: Option<i32> = None;
        for c in text.chars() {
            if let Some(digit) = c.to_digit(10) {
                current = Some(
                    current
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(digit as i32),
                );
            } else if let Some(number) = current.take() {
                runs.push(number);
            }
        }
        if let Some(number) = current {
            runs.push(number);
        }
        if runs.is_empty() {
            return None;
        }
        let mut version = Self {
            numbers: [0; 4],
            alpha: 0,
        };
        let mut numbers = runs.as_slice();
        if text.contains(['a', 'A']) {
            version.alpha = *numbers.last()?;
            numbers = &numbers[..numbers.len() - 1];
        }
        for (slot, number) in version.numbers.iter_mut().zip(numbers) {
            *slot = *number;
        }
        Some(version)
    }

    /// `v1 < v2`.
    fn is_older_than(&self, other: &Self) -> bool {
        match self.numbers.cmp(&other.numbers) {
            std::cmp::Ordering::Less => true,
            std::cmp::Ordering::Greater => false,
            std::cmp::Ordering::Equal => {
                (other.alpha == 0 || self.alpha < other.alpha) && self.alpha > 0
            }
        }
    }
}

/// Whether a save of this game version (`scene.bb8scene`'s `version`) has the species block.
#[must_use]
pub fn has_species_block(version: &str) -> bool {
    Version::parse(version).is_some_and(|version| !version.is_older_than(&SPECIES_BLOCK_SINCE))
}

/// Whether the Graphs history of a save of this game version can be read.
#[must_use]
pub fn has_logger_history(version: &str) -> bool {
    Version::parse(version).is_some_and(|version| !version.is_older_than(&LOGGER_HISTORY_SINCE))
}

/// Whether a save of this game version has the zone block.
#[must_use]
pub fn has_zone_block(version: &str) -> bool {
    Version::parse(version).is_some_and(|version| !version.is_older_than(&ZONE_BLOCK_SINCE))
}

/// The blocks of a `data.bin`, or `None` when a length runs past the end of the file.
#[must_use]
pub fn split_blocks(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let mut blocks = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let length = i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
        let length = usize::try_from(length).ok()?;
        blocks.push(bytes.get(at + 4..at + 4 + length)?);
        at += 4 + length;
    }
    Some(blocks)
}

/// Puts blocks together again.
#[must_use]
pub fn join_blocks(blocks: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(blocks.iter().map(|block| 4 + block.len()).sum());
    for block in blocks {
        out.extend_from_slice(&(block.len() as i32).to_le_bytes());
        out.extend_from_slice(block);
    }
    out
}

/// The species block's entries: the species id and the bytes of its log. Stops at the first entry that
/// does not fit.
#[must_use]
pub fn species_entries(block: &[u8]) -> Vec<(i64, &[u8])> {
    let mut entries = Vec::new();
    let mut at = 0;
    while at + 12 <= block.len() {
        let id = i64::from_le_bytes(block[at..at + 8].try_into().unwrap_or_default());
        let length = i32::from_le_bytes(block[at + 8..at + 12].try_into().unwrap_or_default());
        let Ok(length) = usize::try_from(length) else {
            break;
        };
        let Some(payload) = block.get(at + 12..at + 12 + length) else {
            break;
        };
        entries.push((id, payload));
        at += 12 + length;
    }
    entries
}

/// The zone block's entries: the zone id and the bytes of its log. Stops at the first entry that does not
/// fit (`ZoneManager.LoadStateBin` walks the same way).
#[must_use]
pub fn zone_entries(block: &[u8]) -> Vec<(i32, &[u8])> {
    let mut entries = Vec::new();
    let mut at = 0;
    while at + 8 <= block.len() {
        let id = i32::from_le_bytes(block[at..at + 4].try_into().unwrap_or_default());
        let length = i32::from_le_bytes(block[at + 4..at + 8].try_into().unwrap_or_default());
        let Ok(length) = usize::try_from(length) else {
            break;
        };
        let Some(payload) = block.get(at + 8..at + 8 + length) else {
            break;
        };
        entries.push((id, payload));
        at += 8 + length;
    }
    entries
}

/// Builds the zone block from `(zone id, log bytes)`.
#[must_use]
pub fn zone_block(entries: &[(i32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, payload) in entries {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        out.extend_from_slice(payload);
    }
    out
}

/// Builds the species block from `(species id, log bytes)`.
#[must_use]
pub fn species_block(entries: &[(i64, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, payload) in entries {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        out.extend_from_slice(payload);
    }
    out
}

/// A `data.bin` of a world with no history yet (what the game writes for a fresh one): the Graphs streams and
/// the tracker empty, the given species and zone blocks.
#[must_use]
pub fn blank_with(species: &[u8], zones: &[u8]) -> Vec<u8> {
    join_blocks(&[&BLANK_DATA_LOGGER, &BLANK_BIBITE_TRACKER, species, zones])
}

/// A `data.bin` of a world with history: the given logger, tracker, species and zone blocks.
#[must_use]
pub fn assemble(logger: &[u8], tracker: &[u8], species: &[u8], zones: &[u8]) -> Vec<u8> {
    join_blocks(&[logger, tracker, species, zones])
}

/// `data` with its block number `index` replaced; `None` when `data` has no such block.
#[must_use]
pub fn replace_block(data: &[u8], index: usize, replacement: &[u8]) -> Option<Vec<u8>> {
    let mut blocks = split_blocks(data)?;
    if blocks.len() <= index {
        return None;
    }
    blocks[index] = replacement;
    Some(join_blocks(&blocks))
}

/// `data` with its species block replaced; `None` when `data` has no block for it.
#[must_use]
pub fn replace_species_block(data: &[u8], species: &[u8]) -> Option<Vec<u8>> {
    replace_block(data, SPECIES_BLOCK, species)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_the_games_do() {
        assert!(has_species_block("0.6.3a3"));
        assert!(has_species_block("0.6.0a15"));
        assert!(has_species_block("0.6.0"));
        assert!(has_species_block("0.7.0a1"));
        assert!(!has_species_block("0.6.0a14"));
        assert!(!has_species_block("0.5.2"));
        assert!(!has_species_block(""));
        let v = Version::parse("0.6.3a3").unwrap();
        assert_eq!((v.numbers, v.alpha), ([0, 6, 3, 0], 3));
        assert_eq!(Version::parse("1.2.3.4").unwrap().numbers, [1, 2, 3, 4]);
    }

    #[test]
    fn blocks_split_and_join_back() {
        let data = blank_with(&species_block(&[(7, vec![1, 2, 3])]), &[]);
        let blocks = split_blocks(&data).expect("splits");
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].len(), 188);
        assert_eq!(blocks[1].len(), 46);
        assert_eq!(blocks[3].len(), 0);
        assert_eq!(join_blocks(&blocks), data);
        assert_eq!(
            species_entries(blocks[SPECIES_BLOCK]),
            vec![(7, &[1_u8, 2, 3][..])]
        );
        // A length past the end of the file is not a data.bin.
        assert!(split_blocks(&[9, 0, 0, 0, 1]).is_none());
        assert!(split_blocks(&[]).is_some_and(|blocks| blocks.is_empty()));
    }

    #[test]
    fn the_species_block_can_be_replaced_without_touching_the_others() {
        let data = blank_with(&species_block(&[(1, vec![0xaa; 5])]), &[]);
        let other = species_block(&[(2, vec![0xbb; 9]), (3, vec![0xcc; 2])]);
        let replaced = replace_species_block(&data, &other).expect("replaced");
        let (a, b) = (
            split_blocks(&data).unwrap(),
            split_blocks(&replaced).unwrap(),
        );
        assert_eq!(a[0], b[0]);
        assert_eq!(a[1], b[1]);
        assert_eq!(a[3], b[3]);
        assert_eq!(species_entries(b[SPECIES_BLOCK]).len(), 2);
        assert!(replace_species_block(&[], &other).is_none());
    }

    #[test]
    fn the_zone_block_is_a_list_of_zone_logs() {
        let zones = zone_block(&[(0, vec![1, 2, 3]), (4, vec![9; 20])]);
        assert_eq!(
            zone_entries(&zones),
            vec![(0, &[1_u8, 2, 3][..]), (4, &[9_u8; 20][..])]
        );
        // An entry that does not fit ends the list, as the game stops reading there.
        assert_eq!(zone_entries(&zones[..zones.len() - 1]).len(), 1);
        assert!(zone_entries(&[]).is_empty());
        let data = blank_with(&[], &zones);
        let blocks = split_blocks(&data).expect("splits");
        assert_eq!(zone_entries(blocks[ZONE_BLOCK]).len(), 2);
        let replaced =
            replace_block(&data, ZONE_BLOCK, &zone_block(&[(7, vec![5])])).expect("replaced");
        let blocks = split_blocks(&replaced).expect("splits");
        assert_eq!(zone_entries(blocks[ZONE_BLOCK]), vec![(7, &[5_u8][..])]);
        assert_eq!(blocks[0].len(), 188, "the other blocks are untouched");
    }

    #[test]
    fn a_new_logger_and_tracker_write_the_blocks_of_a_blank_game() {
        // The two constants are the blocks of a blank game's file: the logger's serialization is the game's down to the byte.
        let mut logger = crate::datalog::DataLogger::new();
        assert_eq!(logger.save_state_bin(), BLANK_DATA_LOGGER);
        assert_eq!(logger.save_tracker_bin(0, 0), BLANK_BIBITE_TRACKER);
        let mut loaded = crate::datalog::DataLogger::new();
        loaded.load_state_bin(&BLANK_DATA_LOGGER);
        assert!(!loaded.has_history());
        assert_eq!(loaded.load_tracker_bin(&BLANK_BIBITE_TRACKER), Some((0, 0)));
        let data = assemble(
            &logger.save_state_bin(),
            &logger.save_tracker_bin(0, 0),
            &[1, 2],
            &[],
        );
        assert_eq!(data, blank_with(&[1, 2], &[]));
    }

    #[test]
    fn the_history_blocks_are_read_from_alpha_nine() {
        assert!(has_logger_history("0.6.0a9"));
        assert!(has_logger_history("0.6.3a3"));
        assert!(has_logger_history("0.6.0"));
        assert!(!has_logger_history("0.6.0a8"));
        assert!(!has_logger_history("0.5.2"));
        assert!(!has_logger_history(""));
    }

    #[test]
    fn zone_logs_exist_from_alpha_ten() {
        assert!(has_zone_block("0.6.0a10"));
        assert!(has_zone_block("0.6.3a3"));
        assert!(has_zone_block("0.6.0"));
        assert!(!has_zone_block("0.6.0a9"));
        assert!(!has_zone_block("0.5.2"));
        assert!(!has_zone_block(""));
    }
}
