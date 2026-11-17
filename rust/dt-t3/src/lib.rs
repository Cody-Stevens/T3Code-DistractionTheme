//! T3 API, files, first-run setup and pure two-deck favicon scheduling.
//! No token, runtime file, settings, theme, or project is accessed by merely constructing a value.

pub mod api;
pub mod favicon;
pub mod setup;
pub mod theme;

use dt_core::{Error, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// File locations ported from `t3api.T3_HOME/RUNTIME_FILE/TOKEN_FILE` and `service.SETTINGS`.
#[derive(Clone, Debug)]
pub struct T3Paths {
    /// T3 home, default ~/.t3 or T3CODE_HOME.
    pub home: PathBuf,
    /// userdata/server-runtime.json.
    pub runtime_file: PathBuf,
    /// ~/.secrets/t3code-distraction-theme/t3-token or DISTRACTION_T3_TOKEN_FILE.
    /// This is a path only; no API may print its contents.
    pub token_file: PathBuf,
    /// userdata/themes or T3_THEMES_DIR.
    pub themes_dir: PathBuf,
    /// userdata/settings.json.
    pub settings_file: PathBuf,
    /// ~/.t3/bin/t3.cmd on Windows, t3 elsewhere.
    pub cli: PathBuf,
}

impl T3Paths {
    /// Port Python's path defaults/environment overrides; do not read or create files.
    /// Tests should pass explicit temporary paths instead of discovering the user's environment.
    pub fn discover() -> Result<Self> {
        Self::from_environment(|name| std::env::var_os(name))
    }

    fn from_environment(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<Self> {
        let user_home = get(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(Error::InvalidInput("user home is unavailable"))?;
        let home = get("T3CODE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home.join(".t3"));
        Ok(Self {
            runtime_file: home.join("userdata/server-runtime.json"),
            token_file: get("DISTRACTION_T3_TOKEN_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| user_home.join(".secrets/t3code-distraction-theme/t3-token")),
            themes_dir: get("T3_THEMES_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("userdata/themes")),
            settings_file: home.join("userdata/settings.json"),
            cli: home.join(if cfg!(windows) {
                "bin/t3.cmd"
            } else {
                "bin/t3"
            }),
            home,
        })
    }
}

/// Server location/liveness from `t3api.origin` and `service.t3_running`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeInfo {
    /// HTTP(S) server origin; validate before attaching an authorization header.
    pub origin: String,
    /// Server process ID.
    pub pid: u32,
}

/// Port `t3api.origin`: read the runtime file on each call so T3 restarts change the destination.
/// Missing/invalid file means T3 is unavailable; no token is read by this function.
pub fn read_runtime(paths: &T3Paths) -> Result<RuntimeInfo> {
    let runtime: RuntimeInfo = serde_json::from_slice(&std::fs::read(&paths.runtime_file)?)
        .map_err(|_| Error::Protocol("invalid T3 runtime file"))?;
    validate_origin(&runtime.origin)?;
    if runtime.pid == 0 {
        return Err(Error::Protocol("invalid T3 server PID"));
    }
    Ok(runtime)
}

fn validate_origin(origin: &str) -> Result<Url> {
    let url = Url::parse(origin).map_err(|_| Error::Protocol("invalid T3 origin"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Protocol("invalid T3 origin"));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides_resolve_without_accessing_user_files() {
        let paths = T3Paths::from_environment(|key| match key {
            "HOME" | "USERPROFILE" => Some("fake-home".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(paths.home, PathBuf::from("fake-home/.t3"));
        assert_eq!(
            paths.runtime_file,
            paths.home.join("userdata/server-runtime.json")
        );
        assert_eq!(
            paths.settings_file,
            paths.home.join("userdata/settings.json")
        );
        assert_eq!(paths.themes_dir, paths.home.join("userdata/themes"));
        assert_eq!(
            paths.token_file,
            PathBuf::from("fake-home/.secrets/t3code-distraction-theme/t3-token")
        );
        assert_eq!(
            paths.cli,
            paths.home.join(if cfg!(windows) {
                "bin/t3.cmd"
            } else {
                "bin/t3"
            })
        );
        let overrides = T3Paths::from_environment(|key| {
            Some(
                match key {
                    "T3CODE_HOME" => "t3-override",
                    "DISTRACTION_T3_TOKEN_FILE" => "token-override",
                    "T3_THEMES_DIR" => "theme-override",
                    _ => "fake-home",
                }
                .into(),
            )
        })
        .unwrap();
        assert_eq!(overrides.home, PathBuf::from("t3-override"));
        assert_eq!(overrides.token_file, PathBuf::from("token-override"));
        assert_eq!(overrides.themes_dir, PathBuf::from("theme-override"));
        assert!(T3Paths::from_environment(|_| None).is_err());
    }
}
