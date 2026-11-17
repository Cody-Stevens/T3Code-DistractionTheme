//! CLI and orchestration boundary. Only `ctl` is implemented in phase one.
//! Other subcommands return a clear error before touching audio, files, T3 state or the helper.

pub mod ahead_pool;
pub mod cli;
pub mod control;
pub mod player;
pub mod supervisor;
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
        cli::Command::Setup => Err(Error::NotImplemented("setup")),
        cli::Command::Service => supervisor::serve(),
        cli::Command::Idle => Err(Error::NotImplemented("idle")),
    }
}
