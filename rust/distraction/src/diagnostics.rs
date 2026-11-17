//! Opt-in timing events. Never include tokens, request URLs or API response bodies.
use std::{
    fmt,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

/// Write an event to stderr when GLOWTUBE_DEBUG is nonempty.
pub fn log(message: fmt::Arguments<'_>) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if *ENABLED.get_or_init(|| std::env::var_os("GLOWTUBE_DEBUG").is_some_and(|v| !v.is_empty())) {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        eprintln!("[glowtube wall={wall:.3}] {message}");
    }
}
