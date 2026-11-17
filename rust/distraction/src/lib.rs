//! Queue, audio-grid rendering, loopback control and supervised native adapter ownership.

pub mod ahead_pool;
pub mod cli;
pub mod control;
pub mod favicon;
pub mod player;
pub mod runtime;
pub mod supervisor;
#[cfg(test)]
mod tests;
mod worker;
pub mod write_loop;

use dt_core::{Error, Result};

/// Dispatch the CLI. Implemented control requests never call setup, start a server or read a token.
pub fn run(cli: cli::Cli) -> Result<()> {
    match cli.command {
        cli::Command::Ctl { command, arg, port } => {
            let reply =
                control::ControlClient::new(port)?.send(command, arg.as_deref().unwrap_or(""))?;
            let json = serde_json::to_string(&reply)
                .map_err(|_| Error::Protocol("invalid control JSON"))?;
            println!("{json}");
            Ok(())
        }
        cli::Command::Play { urls } => player::play(urls),
        cli::Command::Setup => runtime::setup_with_paths(dt_t3::T3Paths::discover()?),
        cli::Command::Service => supervisor::serve(),
        cli::Command::Idle => runtime::idle(),
    }
}
