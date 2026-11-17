//! Benchmark and run report formatting.
//!
//! The numbers are computed by the engine; this module only turns them into
//! stable human/JSON/CSV output so benchmark results can be compared between
//! runs and machines.

use std::fmt;

/// Result of one benchmark repetition (or of a `run`).
#[derive(Debug, Clone, PartialEq)]
pub struct RunReport {
    /// Input/scenario label.
    pub label: String,
    /// Simulated ticks.
    pub ticks: u64,
    /// Population at the start of the measured run.
    pub bibites_start: usize,
    /// Population at the end.
    pub bibites_end: usize,
    /// Wall-clock seconds.
    pub wall_secs: f64,
    /// Simulated seconds (`ticks * dt`).
    pub sim_secs: f64,
    /// Worker threads used (0 = serial).
    pub threads: usize,
    /// Peak resident bytes, if known.
    pub peak_bytes: Option<u64>,
    /// Deterministic world-state fingerprint after the run (FNV-1a over entity
    /// state), for same-seed determinism checks and CI.
    pub state_fingerprint: Option<u64>,
}

impl RunReport {
    /// Simulation ticks per wall-clock second.
    pub fn ticks_per_second(&self) -> f64 {
        if self.wall_secs > 0.0 {
            self.ticks as f64 / self.wall_secs
        } else {
            0.0
        }
    }

    /// Simulated time divided by wall-clock time (`1.0` = real time).
    pub fn realtime_factor(&self) -> f64 {
        if self.wall_secs > 0.0 {
            self.sim_secs / self.wall_secs
        } else {
            0.0
        }
    }

    /// Bibite-updates per wall-clock second (the primary throughput metric).
    pub fn bibite_updates_per_second(&self) -> f64 {
        self.ticks_per_second() * self.bibites_start as f64
    }

    /// One-line human-readable summary.
    pub fn summary(&self) -> String {
        format!(
            "{}: {} bibites, {} ticks in {:.3}s = {:.0} ticks/s, {:.1}x realtime, {:.0} bibite-updates/s{}",
            self.label,
            self.bibites_start,
            self.ticks,
            self.wall_secs,
            self.ticks_per_second(),
            self.realtime_factor(),
            self.bibite_updates_per_second(),
            if self.threads > 0 {
                format!(", {} threads", self.threads)
            } else {
                String::new()
            }
        )
    }

    /// JSON object (stable field order, one line).
    pub fn to_json(&self) -> String {
        format!(
            "{{\"label\":\"{}\",\"ticks\":{},\"bibites_start\":{},\"bibites_end\":{},\
             \"wall_secs\":{:.6},\"sim_secs\":{:.6},\"ticks_per_second\":{:.3},\
             \"realtime_factor\":{:.3},\"bibite_updates_per_second\":{:.1},\
             \"threads\":{},\"peak_bytes\":{},\"state_fingerprint\":{}}}",
            json_escape(&self.label),
            self.ticks,
            self.bibites_start,
            self.bibites_end,
            self.wall_secs,
            self.sim_secs,
            self.ticks_per_second(),
            self.realtime_factor(),
            self.bibite_updates_per_second(),
            self.threads,
            self.peak_bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "null".to_owned()),
            self.state_fingerprint
                .map(|fingerprint| fingerprint.to_string())
                .unwrap_or_else(|| "null".to_owned())
        )
    }

    /// CSV row (header is [`RunReport::CSV_HEADER`]).
    pub fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{:.6},{:.6},{:.3},{:.3},{:.1},{},{},{}",
            csv_escape(&self.label),
            self.ticks,
            self.bibites_start,
            self.bibites_end,
            self.wall_secs,
            self.sim_secs,
            self.ticks_per_second(),
            self.realtime_factor(),
            self.bibite_updates_per_second(),
            self.threads,
            self.peak_bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "null".to_owned()),
            self.state_fingerprint
                .map(|fingerprint| fingerprint.to_string())
                .unwrap_or_else(|| "null".to_owned())
        )
    }

    /// CSV header matching [`RunReport::to_csv`].
    pub const CSV_HEADER: &'static str = "label,ticks,bibites_start,bibites_end,wall_secs,sim_secs,ticks_per_second,realtime_factor,bibite_updates_per_second,threads,peak_bytes,state_fingerprint";
}

impl fmt::Display for RunReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn csv_escape(input: &str) -> String {
    if input.contains(',') || input.contains('"') || input.contains('\n') {
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> RunReport {
        RunReport {
            label: "default".into(),
            ticks: 4000,
            bibites_start: 1000,
            bibites_end: 1234,
            wall_secs: 2.0,
            sim_secs: 100.0,
            threads: 12,
            peak_bytes: Some(1024),
            state_fingerprint: Some(0xDEAD_BEEF),
        }
    }

    #[test]
    fn math_is_consistent() {
        let r = report();
        assert!((r.ticks_per_second() - 2000.0).abs() < 1e-9);
        assert!((r.realtime_factor() - 50.0).abs() < 1e-9);
        assert!((r.bibite_updates_per_second() - 2_000_000.0).abs() < 1e-6);
    }

    #[test]
    fn zero_wall_time_does_not_divide_by_zero() {
        let mut r = report();
        r.wall_secs = 0.0;
        assert_eq!(r.ticks_per_second(), 0.0);
        assert_eq!(r.realtime_factor(), 0.0);
        assert_eq!(r.bibite_updates_per_second(), 0.0);
    }

    #[test]
    fn json_is_well_formed_and_escaped() {
        let mut r = report();
        r.label = "a\"b\\c".into();
        let json = r.to_json();
        assert!(json.starts_with('{') && json.ends_with('}'));
        assert!(json.contains("\"label\":\"a\\\"b\\\\c\""));
        assert!(json.contains("\"ticks\":4000"));
    }

    #[test]
    fn csv_escaping() {
        let r = RunReport {
            label: "a,b".into(),
            ..report()
        };
        assert!(r.to_csv().starts_with("\"a,b\","));
        assert!(!report().to_csv().starts_with('"'));
    }

    #[test]
    fn summary_contains_key_numbers() {
        let text = report().summary();
        assert!(text.contains("2000 ticks/s"));
        assert!(text.contains("50.0x realtime"));
        assert!(text.contains("12 threads"));
    }
}
