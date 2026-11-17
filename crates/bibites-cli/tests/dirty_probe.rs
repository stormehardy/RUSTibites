//! A layout probe, run by hand: how often each field and each cache line of a bibite record is written by a tick.
//!
//! `cargo test --profile perf -p bibites-cli --test dirty_probe -- --ignored --nocapture`
//! (`PROBE_N` bibites, default 600, `PROBE_TICKS` measured ticks, default 120.) Not a test of anything: the numbers it prints are
//! what the layout of `BibiteRecord` is argued from (`docs/BENCHMARKS.md`).

use std::collections::BTreeMap;
use std::mem::size_of;

use bibites_core::body::{BibiteRecord, BibiteSpawn, SpawnVitals};
use bibites_core::brain::Brain;
use bibites_core::genome::{Genome, GenomeMeta};
use bibites_core::ids::BibiteId;
use bibites_core::math::Vec2;
use bibites_core::settings::SimSettings;
use bibites_core::world::{ScenarioData, World};

/// Flattens the pretty `Debug` text of a value into `(path, text)` pairs, one per leaf.
fn flatten(text: &str) -> Vec<(String, String)> {
    struct Open {
        name: String,
        array: bool,
        next: usize,
    }
    let mut leaves = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    for line in &lines[1..lines.len().saturating_sub(1)] {
        let depth = (line.len() - line.trim_start().len()) / 4;
        let trimmed = line.trim();
        stack.truncate(depth - 1);
        if trimmed.starts_with('}') || trimmed.starts_with(']') || trimmed.starts_with(')') {
            continue;
        }
        let in_array = stack.last().is_some_and(|open| open.array);
        let (name, rest) = if in_array {
            let open = stack.last_mut().unwrap();
            let name = open.next.to_string();
            open.next += 1;
            (name, trimmed)
        } else if let Some((name, rest)) = trimmed.split_once(": ") {
            (name.to_string(), rest)
        } else {
            // A struct or tuple element without a name.
            let open = stack.last_mut().unwrap();
            let name = open.next.to_string();
            open.next += 1;
            (name, trimmed)
        };
        let path = stack
            .iter()
            .map(|open| open.name.as_str())
            .chain(std::iter::once(name.as_str()))
            .collect::<Vec<_>>()
            .join(".");
        if rest.ends_with('{') || rest.ends_with('[') || rest.ends_with('(') {
            stack.push(Open {
                name,
                array: !rest.ends_with('{'),
                next: 0,
            });
        } else {
            leaves.push((path, rest.trim_end_matches(',').to_string()));
        }
    }
    leaves
}

/// The fields of a record (its brain left out: the vectors it points to are not in the record).
fn describe(record: &BibiteRecord) -> BTreeMap<String, String> {
    let mut copy = record.clone();
    copy.brain = Brain::starter();
    flatten(&format!("{copy:#?}")).into_iter().collect()
}

/// The bytes of a record, padding included.
fn raw(record: &BibiteRecord) -> Vec<u8> {
    let mut bytes = vec![0u8; size_of::<BibiteRecord>()];
    // SAFETY: a diagnostic copy of the record's bytes into a buffer of the same size.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (record as *const BibiteRecord).cast::<u8>(),
            bytes.as_mut_ptr(),
            bytes.len(),
        );
    }
    bytes
}

#[test]
#[ignore = "layout probe: prints how often each field and each cache line of a record is written by a tick"]
fn which_fields_a_tick_writes() {
    let n = std::env::var("PROBE_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600usize);
    let ticks = std::env::var("PROBE_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120usize);
    let mut settings = SimSettings::game_defaults();
    let scale = (n as f32 / 2_000.0).sqrt();
    settings.independents.simulation_size = (2_000.0 * scale).min(20_000.0);
    let scenario = ScenarioData::with_default_zone_group();
    let mut world = World::new(settings, scenario, 0x5255_5354).unwrap();
    let _ = world.seed_scenario_world();
    let genome = Genome::starter();
    let brain = Brain::starter();
    for index in 0..n {
        let mut rng = world
            .rng
            .fork(0xB1B1_7E50 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let min = world.bounds.min();
        let max = world.bounds.max();
        let margin = (max - min).length() * 0.05;
        let low = min + Vec2::splat(margin);
        let high = max - Vec2::splat(margin);
        let mut spawn = BibiteSpawn::new(genome.clone(), GenomeMeta::default(), brain.clone());
        spawn.pos = Vec2::new(rng.f32_range(low.x, high.x), rng.f32_range(low.y, high.y));
        spawn.rot = rng.f32_range(0.0, std::f32::consts::TAU);
        spawn.vitals = SpawnVitals::Adult;
        let id = world.bibites.spawn(spawn).unwrap();
        world.register_bibite(id);
    }
    for _ in 0..60 {
        world.tick();
    }
    let lines = size_of::<BibiteRecord>() / 64;
    let mut field_changes: BTreeMap<String, u64> = BTreeMap::new();
    let mut line_changes = vec![0u64; lines];
    let mut word_changes = vec![0u64; lines * 8];
    let mut dirty_lines_total = 0u64;
    let mut samples = 0u64;
    for _ in 0..ticks {
        let before: Vec<(BibiteId, BTreeMap<String, String>, Vec<u8>)> = world
            .bibites
            .iter()
            .map(|(id, record)| (id, describe(record), raw(record)))
            .collect();
        world.tick();
        for (id, fields, bytes) in &before {
            let Some(record) = world.bibites.get(*id) else {
                continue;
            };
            samples += 1;
            let after = describe(record);
            for (path, value) in &after {
                if fields.get(path) != Some(value) {
                    *field_changes.entry(path.clone()).or_default() += 1;
                }
            }
            let now = raw(record);
            let mut dirty_lines = 0;
            for line in 0..lines {
                let mut any = false;
                for word in 0..8 {
                    let at = line * 64 + word * 8;
                    if bytes[at..at + 8] != now[at..at + 8] {
                        word_changes[line * 8 + word] += 1;
                        any = true;
                    }
                }
                if any {
                    line_changes[line] += 1;
                    dirty_lines += 1;
                }
            }
            dirty_lines_total += dirty_lines;
        }
    }
    eprintln!("samples (bibite, tick): {samples}");
    eprintln!(
        "expected dirty cache lines per bibite per tick: {:.2} of {lines}",
        dirty_lines_total as f64 / samples as f64
    );
    for line in 0..lines {
        let words: String = (0..8)
            .map(|word| {
                let rate = word_changes[line * 8 + word] as f64 / samples as f64;
                if rate == 0.0 {
                    "  .".to_string()
                } else {
                    format!("{:>3.0}", rate * 100.0)
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!(
            "line {line:>2}: dirty {:>5.1}%   words(8 B) % [{words} ]",
            line_changes[line] as f64 / samples as f64 * 100.0
        );
    }
    let mut rows: Vec<(&String, &u64)> = field_changes.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    eprintln!("fields that changed in at least 0.5% of the samples:");
    for (path, count) in rows {
        let rate = *count as f64 / samples as f64 * 100.0;
        if rate >= 0.5 {
            eprintln!("  {rate:>6.1}%  {path}");
        }
    }
}

/// Which byte of an `Option<BibiteRecord>` says "empty": every `entry.as_ref()` over the store's slots reads it, so its cache
/// line is touched by every pass over the records.
#[test]
#[ignore = "layout probe: prints where the niche of Option<BibiteRecord> lies"]
fn where_the_niche_of_an_empty_slot_lies() {
    use std::mem::{MaybeUninit, size_of};
    println!(
        "size_of BibiteRecord {}, Option<BibiteRecord> {}",
        size_of::<BibiteRecord>(),
        size_of::<Option<BibiteRecord>>()
    );
    let mut buffer = MaybeUninit::<Option<BibiteRecord>>::zeroed();
    // SAFETY: writes `None` into a zeroed buffer of the right size; only the bytes that say "empty" change.
    unsafe {
        buffer.as_mut_ptr().write(None);
        let bytes = std::slice::from_raw_parts(
            buffer.as_ptr().cast::<u8>(),
            size_of::<Option<BibiteRecord>>(),
        );
        let set: Vec<(usize, u8)> = bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte != 0)
            .map(|(offset, byte)| (offset, *byte))
            .collect();
        println!("bytes that are not zero in None: {set:?}");
        for (offset, _) in &set {
            println!("  offset {offset} is on line {}", offset / 64);
        }
    }
}
