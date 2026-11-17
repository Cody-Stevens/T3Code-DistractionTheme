//! Executable entry point for the Rust helper.

use clap::Parser;

fn main() -> std::process::ExitCode {
    match distraction::run(distraction::cli::Cli::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("distraction: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
