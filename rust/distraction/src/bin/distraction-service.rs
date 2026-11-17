//! Windowless service started at sign-in by `distraction install`. Runs the supervisor and starts
//! `distraction.exe` from the same folder for playback. Built for the Windows GUI subsystem so no
//! console window appears; errors go to `%LOCALAPPDATA%\GlowTube\service.log`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::Write;

fn main() -> std::process::ExitCode {
    let result = std::env::current_exe()
        .map_err(dt_core::Error::from)
        .and_then(|exe| {
            distraction::supervisor::serve_with(
                exe.with_file_name(distraction::install::PLAYER_EXE),
            )
        });
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if let Ok(state) = distraction::runtime::state_dir()
                && std::fs::create_dir_all(&state).is_ok()
                && let Ok(mut log) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(state.join("service.log"))
            {
                let _ = writeln!(log, "service stopped: {error}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
