//! clap surface for `play`, `ctl`, `setup`, `service` and `idle` from docs/RUST_PLAN.md.

use clap::{Parser, Subcommand, ValueEnum};
use dt_core::constants::CONTROL_PORT;

/// Distraction theme helper.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// Operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level operations.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Play URLs or local files.
    Play {
        /// Initial playlist; an empty list starts with the idle card.
        urls: Vec<String>,
    },
    /// Send a command to the running Python or Rust helper using the existing loopback protocol.
    Ctl {
        /// Same command names as glowtube.command; defaults to status.
        #[arg(value_enum, default_value_t = CtlCommand::Status)]
        command: CtlCommand,
        /// Optional argument encoded as query parameter v; quote URLs containing shell characters.
        #[arg(allow_hyphen_values = true)]
        arg: Option<String>,
        /// Helper's loopback port; honors Python's GLOWTUBE_PORT override.
        #[arg(long, env = "GLOWTUBE_PORT", default_value_t = CONTROL_PORT)]
        port: u16,
    },
    /// Prepare scoped token, project and selected theme.
    Setup,
    /// Supervise playback while T3 is running.
    Service,
    /// Write the idle theme.
    Idle,
}

/// Every command accepted by `glowtube.command`, with unchanged URL path spellings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum CtlCommand {
    /// Read the current JSON snapshot, without waking/rebuilding the theme.
    #[default]
    Status,
    /// Toggle playback.
    Toggle,
    /// Resume playback.
    Play,
    /// Pause playback.
    Pause,
    /// Advance to the next queue entry.
    Next,
    /// Go to the previous queue entry.
    Prev,
    /// Relative seek in seconds, default +10; negative values are allowed.
    Seek,
    /// Gain, clamped by server to 0..=1.5.
    Vol,
    /// Visual intensity, clamped by server to 0..=11.
    Chaos,
    /// Add a signed number of seconds to display offset.
    Offset,
    /// Add a link and play immediately.
    Add,
    /// Append a link to the queue.
    Queue,
    /// Move upcoming zero-based row offset to play next; Python defaults the argument to 1.
    #[value(name = "playnext")]
    PlayNext,
    /// Enable favicon video unless argument is exactly off.
    Favicon,
    /// Hold writes for the argument's seconds, default 2.
    #[value(name = "holdwrites")]
    HoldWrites,
    /// Stop the helper.
    Quit,
}

impl CtlCommand {
    /// Exact `glowtube.command` URL segment; contains no slash or user input.
    pub fn path(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Toggle => "toggle",
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Next => "next",
            Self::Prev => "prev",
            Self::Seek => "seek",
            Self::Vol => "vol",
            Self::Chaos => "chaos",
            Self::Offset => "offset",
            Self::Add => "add",
            Self::Queue => "queue",
            Self::PlayNext => "playnext",
            Self::Favicon => "favicon",
            Self::HoldWrites => "holdwrites",
            Self::Quit => "quit",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctl_status_is_the_default_and_cli_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
        let cli = Cli::try_parse_from(["distraction", "ctl"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Ctl {
                command: CtlCommand::Status,
                ..
            }
        ));
    }
}
