//! `install` / `uninstall`: copy the helper to `%LOCALAPPDATA%\GlowTube\bin`, run the windowless
//! service now and at every sign-in (HKCU Run entry), and stop it cleanly.
//!
//! The service owns the player inside a kill-on-close job, so killing the service would cut the
//! player off before it blanks the favicons and writes the idle theme. Instead a stop request is a
//! file in the state folder; the service sees it within 50 ms, stops the player gracefully and
//! exits, which releases its single-instance mutex.

use crate::{runtime, supervisor};
use dt_core::{Error, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Name of the start-at-sign-in entry.
pub const RUN_VALUE: &str = "GlowTube";
/// The command-line helper (player, `ctl`, setup).
pub const PLAYER_EXE: &str = "distraction.exe";
/// The windowless supervisor started at sign-in.
pub const SERVICE_EXE: &str = "distraction-service.exe";
/// Stop-request file name inside the state folder.
pub const STOP_FILE: &str = "service.stop";
/// T3 theme id the helper writes and the service requires to be selected.
pub const THEME_ID: &str = "glowtube";

const STOP_TIMEOUT: Duration = Duration::from_secs(25);

/// Where `install` puts both executables.
pub fn bin_dir() -> Result<PathBuf> {
    Ok(runtime::state_dir()?.join("bin"))
}

/// The value written to the Run key: the quoted service path.
pub fn run_command(service: &Path) -> String {
    format!("\"{}\"", service.display())
}

/// Set up T3 (token, deck projects, theme selection), copy both executables to [`bin_dir`],
/// register the service to start at sign-in and start it now.
pub fn install() -> Result<()> {
    let source = std::env::current_exe()?;
    let source_dir = source
        .parent()
        .ok_or(Error::InvalidInput("executable has no folder"))?;
    let service_source = source_dir.join(SERVICE_EXE);
    if !service_source.is_file() {
        return Err(Error::InvalidInput(
            "distraction-service.exe must be in the same folder as distraction.exe",
        ));
    }
    let paths = dt_t3::T3Paths::discover()?;
    if !supervisor::t3_running(&paths) {
        return Err(Error::InvalidInput(
            "start T3 Code first: setup needs its server",
        ));
    }
    runtime::setup_with_paths(paths)?;
    println!("Setup done: token, GlowTube decks and theme (selected).");

    if stop_service(STOP_TIMEOUT)? {
        println!("Stopped the running service to replace it.");
    }
    let dir = bin_dir()?;
    fs::create_dir_all(&dir)?;
    let player = dir.join(PLAYER_EXE);
    let service = dir.join(SERVICE_EXE);
    copy_unless_same(&source, &player)?;
    copy_unless_same(&service_source, &service)?;
    dt_platform::autostart::enable(RUN_VALUE, &run_command(&service))?;
    start_detached(&service)?;
    println!("Installed to {}", dir.display());
    println!(
        "The service now runs at every sign-in and plays while T3 Code runs with the GlowTube"
    );
    println!(
        "theme selected. Pick another theme in T3 to turn it off; `distraction uninstall` removes it."
    );
    Ok(())
}

/// Remove the sign-in entry and stop the running service (which stops the player gracefully).
pub fn uninstall() -> Result<()> {
    let removed = dt_platform::autostart::disable(RUN_VALUE)?;
    let stopped = stop_service(STOP_TIMEOUT)?;
    println!(
        "{}",
        match (removed, stopped) {
            (true, true) => "Removed the sign-in entry and stopped the service.",
            (true, false) => "Removed the sign-in entry; the service was not running.",
            (false, true) => "Stopped the service; there was no sign-in entry.",
            (false, false) => "Nothing to do: no sign-in entry and no running service.",
        }
    );
    println!(
        "The copied files stay in {}; delete that folder to remove them.",
        bin_dir()?.display()
    );
    Ok(())
}

/// True while some process holds the service mutex.
pub fn service_running() -> Result<bool> {
    // Creating the mutex succeeds only when nobody holds it; the probe guard drops at once.
    Ok(dt_platform::process::single_instance(dt_platform::process::SERVICE_MUTEX)?.is_none())
}

/// Ask a running service to stop and wait for it to exit. Returns false when none was running.
pub fn stop_service(timeout: Duration) -> Result<bool> {
    if !service_running()? {
        return Ok(false);
    }
    let state = runtime::state_dir()?;
    fs::create_dir_all(&state)?;
    let stop = supervisor::stop_file(&state);
    fs::write(&stop, b"stop")?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !service_running()? {
            let _ = fs::remove_file(&stop);
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = fs::remove_file(&stop);
    Err(Error::Protocol("the service did not stop in time"))
}

fn copy_unless_same(from: &Path, to: &Path) -> Result<()> {
    let same = match (fs::canonicalize(from), fs::canonicalize(to)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        fs::copy(from, to)?;
    }
    Ok(())
}

/// Start the service outside the caller's console and job, so closing the terminal that ran
/// `install` doesn't stop it.
fn start_detached(service: &Path) -> Result<()> {
    let spawn = |flags: u32| {
        let mut command = Command::new(service);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(flags);
        }
        #[cfg(not(windows))]
        let _ = flags;
        command.spawn()
    };
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    // Breaking away fails when the caller's job forbids it; then start it inside the job.
    match spawn(base | CREATE_BREAKAWAY_FROM_JOB) {
        Ok(_) => Ok(()),
        Err(_) => spawn(base).map(drop).map_err(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_command_quotes_paths_with_spaces() {
        assert_eq!(
            run_command(Path::new(
                r"C:\Users\A B\AppData\Local\GlowTube\bin\distraction-service.exe"
            )),
            r#""C:\Users\A B\AppData\Local\GlowTube\bin\distraction-service.exe""#
        );
    }

    #[test]
    fn theme_selection_gates_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        assert!(!supervisor::theme_selected(&settings));
        fs::write(&settings, br#"{"defaultTheme":"glowtube","other":1}"#).unwrap();
        assert!(supervisor::theme_selected(&settings));
        fs::write(&settings, br#"{"defaultTheme":"t3-dark"}"#).unwrap();
        assert!(!supervisor::theme_selected(&settings));
        fs::write(&settings, b"not json").unwrap();
        assert!(!supervisor::theme_selected(&settings));
    }

    #[test]
    fn stop_file_lives_in_the_state_folder() {
        assert_eq!(
            supervisor::stop_file(Path::new("state")),
            Path::new("state").join(STOP_FILE)
        );
    }
}
