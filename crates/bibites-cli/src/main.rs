//! `rustibites` — headless simulation runner and benchmark harness.

mod args;
mod braincheck;
mod engine;
// Parity capture emitter is wired to `run --capture` by the Lead; keep it
// compiled and tested until then.
#[allow(dead_code)]
mod capture;
// Parity digests are wired to the capture emitter at integration; keep them
// compiled and tested until then.
#[allow(dead_code)]
mod parity;
// Reporting is wired to the engine in the integration milestone; keep it
// compiled and tested until then.
#[allow(dead_code)]
mod report;

use std::process::ExitCode;

use args::Command;

fn main() -> ExitCode {
    match args::parse(std::env::args().skip(1)) {
        Ok(Command::Help) => {
            print!("{}", args::USAGE);
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("rustibites {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(command) => match dispatch(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("error: {error}\n");
            eprint!("{}", args::USAGE);
            ExitCode::from(2)
        }
    }
}

fn dispatch(command: Command) -> Result<(), String> {
    match command {
        Command::Run(run) => engine::run(&run),
        Command::Bench(bench) => engine::bench(&bench),
        Command::Info { input } => engine::info(&input),
        Command::Check { input } => engine::check(&input),
        Command::Braincheck { fixture, out } => engine::braincheck(&fixture, out.as_deref()),
        Command::Help | Command::Version => Ok(()),
    }
}
