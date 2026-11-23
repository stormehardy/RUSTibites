//! Command-line argument parsing for `rustibites`.
//!
//! Hand-rolled (no dependency) so the CLI stays portable and auditable. The
//! parser is intentionally strict: unknown flags and missing values are hard
//! errors with actionable messages.

use std::fmt;
use std::path::PathBuf;

/// Usage text printed for `--help` and parse errors.
pub const USAGE: &str = "\
rustibites — headless simulation engine and benchmark harness for The Bibites

USAGE:
    rustibites <COMMAND> [OPTIONS]

COMMANDS:
    run <PATH>        Load a world/scenario/genome and simulate it
    bench             Run the benchmark harness
    calibrate         Time this machine: how many workers a tick should use and from what size of world the pool of workers pays
    info <PATH>       Inspect a world/scenario/genome/settings file
    check <PATH>      Validate a file and its round-trip
    braincheck <JSON> Emit Rust brain outputs for tools/braincheck (with -o)
    help              Show this help
    version           Show the version

RUN OPTIONS:
    --ticks <N>         Number of simulation ticks to run (default: 1000)
    --seed <N>          RNG seed (default: 0)
    --threads <N>       Worker threads; 0 = automatic (default: 0)
    --stats <FMT>       human | json | csv (default: human)
    --settings <PATH>   Scenario-independent settings override (.bb8settings)
    --save <PATH>       Write the world to this .zip when the run ends
    --capture <DIR>     Write a parity capture (docs/PARITY.md) while running
    --templates <DIR>   Load *.bb8template files for scenario spawn rows
    --no-state          Do not load/write checkpoint state (.bb8state sidecar)
    --quiet             Suppress per-checkpoint progress output

BENCH OPTIONS:
    --scenario <PATH>   Scenario to benchmark (default: built-in / testdata)
    --bibites <N>       Synthetic population size (default: 2000)
    --ticks <N>         Ticks per repetition (default: 600)
    --threads <N>       Worker threads; 0 = automatic (default: 0)
    --repeat <N>        Repetitions (default: 3)
    --json              Emit a machine-readable report

CALIBRATE OPTIONS:
    --quick             A shorter, rougher test (about a fifth of the time)
    --threads <N>       Threads of the machine to calibrate for; 0 = every one (default: 0)
    --save              Keep the result where the window's Settings > Calibration page reads it (./userdata/calibration.txt)
    --out <PATH>        Write the result to this file

Run `rustibites help` for this text.
";

/// Arguments of `calibrate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalibrateArgs {
    /// A shorter, rougher test.
    pub quick: bool,
    /// Threads of the machine to calibrate for (0 = every one it has).
    pub threads: usize,
    /// Keep the result where the window's Calibration page reads it.
    pub save: bool,
    /// Write the result to this file.
    pub out: Option<PathBuf>,
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Simulate a world/scenario/genome.
    Run(RunArgs),
    /// Benchmark throughput.
    Bench(BenchArgs),
    /// Calibrate the machine (how many workers a tick uses, from what size of world the pool pays).
    Calibrate(CalibrateArgs),
    /// Inspect a file.
    Info {
        /// File to inspect.
        input: PathBuf,
    },
    /// Validate a file and its round-trip.
    Check {
        /// File to validate.
        input: PathBuf,
    },
    /// Emit Rust brain-evaluator outputs for the independent Python checker.
    Braincheck {
        /// Fixture JSON (see tools/braincheck/README.md).
        fixture: PathBuf,
        /// Optional output JSON path; stdout when omitted.
        out: Option<PathBuf>,
    },
    /// Print usage.
    Help,
    /// Print the version.
    Version,
}

/// Output format for run statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsFormat {
    /// Human-readable summary.
    Human,
    /// JSON object.
    Json,
    /// CSV row.
    Csv,
}

impl StatsFormat {
    fn parse(value: &str) -> Result<Self, ParseError> {
        match value {
            "human" => Ok(Self::Human),
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            other => Err(ParseError(format!(
                "invalid --stats value '{other}' (expected human|json|csv)"
            ))),
        }
    }
}

/// Arguments for `run`.
#[derive(Debug, Clone, PartialEq)]
pub struct RunArgs {
    /// World, scenario or genome file.
    pub input: PathBuf,
    /// Number of ticks to simulate.
    pub ticks: u64,
    /// RNG seed.
    pub seed: u64,
    /// Worker threads; `0` means automatic.
    pub threads: usize,
    /// Statistics output format.
    pub stats: StatsFormat,
    /// Optional scenario-independent settings override.
    pub settings: Option<PathBuf>,
    /// Optional output save path.
    pub save: Option<PathBuf>,
    /// Optional parity capture directory (see docs/PARITY.md).
    pub capture: Option<PathBuf>,
    /// Optional directory of `.bb8template` files used to resolve scenario
    /// spawn rows (the game resolves templates from its user template folder).
    pub templates: Option<PathBuf>,
    /// Disable loading/writing the `.bb8state` checkpoint sidecar.
    pub no_state: bool,
    /// Suppress progress output.
    pub quiet: bool,
}

/// Arguments for `bench`.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchArgs {
    /// Optional scenario to benchmark.
    pub scenario: Option<PathBuf>,
    /// Synthetic population size.
    pub bibites: usize,
    /// Ticks per repetition.
    pub ticks: u64,
    /// Worker threads; `0` means automatic.
    pub threads: usize,
    /// Number of repetitions.
    pub repeat: u32,
    /// Emit JSON instead of human-readable output.
    pub json: bool,
}

/// A command-line parse failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

type ArgIter<I> = std::iter::Peekable<I>;

/// Parses command-line arguments (without the program name).
pub fn parse<I>(args: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = String>,
{
    let mut it = args.into_iter().peekable();
    let Some(command) = it.next() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" | "version" => Ok(Command::Version),
        "run" => parse_run(&mut it),
        "bench" => parse_bench(&mut it),
        "calibrate" => parse_calibrate(&mut it),
        "info" => parse_single_path("info", &mut it),
        "check" => parse_single_path("check", &mut it),
        "braincheck" => parse_braincheck(&mut it),
        other => Err(ParseError(format!("unknown command '{other}'"))),
    }
}

fn parse_single_path<I>(command: &str, it: &mut ArgIter<I>) -> Result<Command, ParseError>
where
    I: Iterator<Item = String>,
{
    let mut input: Option<PathBuf> = None;
    for arg in it.by_ref() {
        if arg.starts_with('-') {
            return Err(ParseError(format!(
                "unexpected option '{arg}' for '{command}'"
            )));
        }
        if input.is_some() {
            return Err(ParseError(format!(
                "'{command}' takes exactly one file argument"
            )));
        }
        input = Some(PathBuf::from(arg));
    }
    let input = input.ok_or_else(|| ParseError(format!("'{command}' needs a file argument")))?;
    Ok(if command == "info" {
        Command::Info { input }
    } else {
        Command::Check { input }
    })
}

fn parse_run<I>(it: &mut ArgIter<I>) -> Result<Command, ParseError>
where
    I: Iterator<Item = String>,
{
    let mut input: Option<PathBuf> = None;
    let mut args = RunArgs {
        input: PathBuf::new(),
        ticks: 1000,
        seed: 0,
        threads: 0,
        stats: StatsFormat::Human,
        settings: None,
        save: None,
        capture: None,
        templates: None,
        no_state: false,
        quiet: false,
    };

    while let Some(raw) = it.next() {
        let (flag, inline) = split_flag(&raw);
        match flag {
            "--ticks" => args.ticks = take_u64(it, flag, inline)?,
            "--seed" => args.seed = take_u64(it, flag, inline)?,
            "--threads" => args.threads = take_usize(it, flag, inline)?,
            "--stats" => args.stats = StatsFormat::parse(&take_value(it, flag, inline)?)?,
            "--settings" => args.settings = Some(PathBuf::from(take_value(it, flag, inline)?)),
            "--save" => args.save = Some(PathBuf::from(take_value(it, flag, inline)?)),
            "--capture" => args.capture = Some(PathBuf::from(take_value(it, flag, inline)?)),
            "--templates" => args.templates = Some(PathBuf::from(take_value(it, flag, inline)?)),
            "--no-state" => args.no_state = true,
            "--quiet" => args.quiet = true,
            other if other.starts_with("--") => {
                return Err(ParseError(format!("unknown option '{other}' for 'run'")));
            }
            _ => {
                if input.is_some() {
                    return Err(ParseError("'run' takes exactly one input file".into()));
                }
                input = Some(PathBuf::from(raw));
            }
        }
    }

    args.input = input.ok_or_else(|| ParseError("'run' needs an input file".into()))?;
    Ok(Command::Run(args))
}

fn parse_braincheck<I>(it: &mut ArgIter<I>) -> Result<Command, ParseError>
where
    I: Iterator<Item = String>,
{
    let mut fixture: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    while let Some(raw) = it.next() {
        let (flag, inline) = split_flag(&raw);
        match flag {
            "-o" | "--out" => out = Some(PathBuf::from(take_value(it, flag, inline)?)),
            other if other.starts_with('-') => {
                return Err(ParseError(format!(
                    "unknown option '{other}' for 'braincheck'"
                )));
            }
            _ => {
                if fixture.is_some() {
                    return Err(ParseError("'braincheck' takes exactly one fixture".into()));
                }
                fixture = Some(PathBuf::from(raw));
            }
        }
    }
    let fixture = fixture.ok_or_else(|| ParseError("'braincheck' needs a fixture file".into()))?;
    Ok(Command::Braincheck { fixture, out })
}

fn parse_calibrate<I>(it: &mut ArgIter<I>) -> Result<Command, ParseError>
where
    I: Iterator<Item = String>,
{
    let mut args = CalibrateArgs { quick: false, threads: 0, save: false, out: None };
    while let Some(raw) = it.next() {
        let (flag, inline) = split_flag(&raw);
        match flag {
            "--quick" => args.quick = true,
            "--threads" => args.threads = take_usize(it, flag, inline)?,
            "--save" => args.save = true,
            "--out" => args.out = Some(PathBuf::from(take_value(it, flag, inline)?)),
            other => {
                return Err(ParseError(format!("unknown option '{other}' for 'calibrate'")));
            }
        }
    }
    Ok(Command::Calibrate(args))
}

fn parse_bench<I>(it: &mut ArgIter<I>) -> Result<Command, ParseError>
where
    I: Iterator<Item = String>,
{
    let mut args = BenchArgs {
        scenario: None,
        bibites: 2000,
        ticks: 600,
        threads: 0,
        repeat: 3,
        json: false,
    };
    while let Some(raw) = it.next() {
        let (flag, inline) = split_flag(&raw);
        match flag {
            "--scenario" => {
                args.scenario = Some(PathBuf::from(take_value(it, flag, inline)?));
            }
            "--bibites" => args.bibites = take_usize(it, flag, inline)?,
            "--ticks" => args.ticks = take_u64(it, flag, inline)?,
            "--threads" => args.threads = take_usize(it, flag, inline)?,
            "--repeat" => {
                let repeat = take_u64(it, flag, inline)?;
                if repeat == 0 || repeat > u32::MAX as u64 {
                    return Err(ParseError("--repeat must be between 1 and 2^32-1".into()));
                }
                args.repeat = repeat as u32;
            }
            "--json" => args.json = true,
            other => {
                return Err(ParseError(format!("unknown option '{other}' for 'bench'")));
            }
        }
    }
    Ok(Command::Bench(args))
}

fn split_flag(raw: &str) -> (&str, Option<String>) {
    match raw.split_once('=') {
        Some((flag, value)) => (flag, Some(value.to_owned())),
        None => (raw, None),
    }
}

fn take_value<I>(
    it: &mut ArgIter<I>,
    flag: &str,
    inline: Option<String>,
) -> Result<String, ParseError>
where
    I: Iterator<Item = String>,
{
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(ParseError(format!("option '{flag}' needs a value")));
        }
        return Ok(value);
    }
    it.next()
        .ok_or_else(|| ParseError(format!("option '{flag}' needs a value")))
}

fn take_u64<I>(it: &mut ArgIter<I>, flag: &str, inline: Option<String>) -> Result<u64, ParseError>
where
    I: Iterator<Item = String>,
{
    let value = take_value(it, flag, inline)?;
    value.parse::<u64>().map_err(|_| {
        ParseError(format!(
            "option '{flag}' expects a non-negative integer, got '{value}'"
        ))
    })
}

fn take_usize<I>(
    it: &mut ArgIter<I>,
    flag: &str,
    inline: Option<String>,
) -> Result<usize, ParseError>
where
    I: Iterator<Item = String>,
{
    let value = take_value(it, flag, inline)?;
    value.parse::<usize>().map_err(|_| {
        ParseError(format!(
            "option '{flag}' expects a non-negative integer, got '{value}'"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(args: &[&str]) -> Result<Command, ParseError> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn empty_is_help() {
        assert_eq!(parse_str(&[]).unwrap(), Command::Help);
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_str(&["-h"]).unwrap(), Command::Help);
        assert_eq!(parse_str(&["--version"]).unwrap(), Command::Version);
    }

    #[test]
    fn run_defaults() {
        let Command::Run(args) = parse_str(&["run", "world.zip"]).unwrap() else {
            panic!("expected run");
        };
        assert_eq!(args.input, PathBuf::from("world.zip"));
        assert_eq!(args.ticks, 1000);
        assert_eq!(args.seed, 0);
        assert_eq!(args.threads, 0);
        assert_eq!(args.stats, StatsFormat::Human);
        assert!(!args.quiet);
    }

    #[test]
    fn run_flags_both_forms() {
        let Command::Run(args) = parse_str(&[
            "run",
            "w.zip",
            "--ticks=500",
            "--seed",
            "42",
            "--threads=8",
            "--stats",
            "json",
            "--save",
            "out.zip",
            "--capture",
            "capdir",
            "--templates",
            "templates",
            "--no-state",
            "--quiet",
        ])
        .unwrap() else {
            panic!("expected run");
        };
        assert_eq!(args.ticks, 500);
        assert_eq!(args.seed, 42);
        assert_eq!(args.threads, 8);
        assert_eq!(args.stats, StatsFormat::Json);
        assert_eq!(args.save, Some(PathBuf::from("out.zip")));
        assert_eq!(args.capture, Some(PathBuf::from("capdir")));
        assert_eq!(args.templates, Some(PathBuf::from("templates")));
        assert!(args.no_state);
        assert!(args.quiet);
    }

    #[test]
    fn bench_defaults_and_flags() {
        let Command::Bench(args) = parse_str(&["bench"]).unwrap() else {
            panic!("expected bench");
        };
        assert_eq!(args.bibites, 2000);
        assert_eq!(args.ticks, 600);
        assert_eq!(args.repeat, 3);

        let Command::Bench(args) = parse_str(&[
            "bench",
            "--scenario=ring.zip",
            "--bibites",
            "10000",
            "--ticks=2000",
            "--threads",
            "12",
            "--repeat=5",
            "--json",
        ])
        .unwrap() else {
            panic!("expected bench");
        };
        assert_eq!(args.scenario, Some(PathBuf::from("ring.zip")));
        assert_eq!(args.bibites, 10000);
        assert_eq!(args.ticks, 2000);
        assert_eq!(args.threads, 12);
        assert_eq!(args.repeat, 5);
        assert!(args.json);
    }

    #[test]
    fn info_and_check() {
        assert_eq!(
            parse_str(&["info", "a.bb8"]).unwrap(),
            Command::Info {
                input: PathBuf::from("a.bb8")
            }
        );
        assert_eq!(
            parse_str(&["check", "s.zip"]).unwrap(),
            Command::Check {
                input: PathBuf::from("s.zip")
            }
        );
    }

    #[test]
    fn braincheck_parses_fixture_and_out() {
        assert_eq!(
            parse_str(&["braincheck", "fixture.json"]).unwrap(),
            Command::Braincheck {
                fixture: PathBuf::from("fixture.json"),
                out: None
            }
        );
        assert_eq!(
            parse_str(&["braincheck", "f.json", "-o", "out.json"]).unwrap(),
            Command::Braincheck {
                fixture: PathBuf::from("f.json"),
                out: Some(PathBuf::from("out.json"))
            }
        );
        assert_eq!(
            parse_str(&["braincheck", "f.json", "--out=o.json"]).unwrap(),
            Command::Braincheck {
                fixture: PathBuf::from("f.json"),
                out: Some(PathBuf::from("o.json"))
            }
        );
        assert!(parse_str(&["braincheck"]).is_err());
        assert!(parse_str(&["braincheck", "a", "b"]).is_err());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_str(&["frobnicate"]).is_err());
        assert!(parse_str(&["run"]).is_err());
        assert_eq!(
            parse_str(&["calibrate"]).unwrap(),
            Command::Calibrate(CalibrateArgs { quick: false, threads: 0, save: false, out: None })
        );
        assert_eq!(
            parse_str(&["calibrate", "--quick", "--threads=4", "--save", "--out", "c.txt"]).unwrap(),
            Command::Calibrate(CalibrateArgs { quick: true, threads: 4, save: true, out: Some(PathBuf::from("c.txt")) })
        );
        assert!(parse_str(&["calibrate", "--ticks", "5"]).is_err());
        assert!(parse_str(&["run", "a.zip", "b.zip"]).is_err());
        assert!(parse_str(&["run", "a.zip", "--ticks"]).is_err());
        assert!(parse_str(&["run", "a.zip", "--ticks", "many"]).is_err());
        assert!(parse_str(&["run", "a.zip", "--stats", "xml"]).is_err());
        assert!(parse_str(&["bench", "--repeat", "0"]).is_err());
        assert!(parse_str(&["info"]).is_err());
        assert!(parse_str(&["info", "a", "b"]).is_err());
    }
}
