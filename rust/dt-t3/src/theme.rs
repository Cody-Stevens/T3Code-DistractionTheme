//! Atomic theme/settings replacement from `glowtube.write_theme` and `service.ensure_theme`.

use dt_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

/// Headroom below T3's 50,000-character hard limit, shared with the render budget.
pub const MAX_CSS_CHARS: usize = 49_500;

/// Port `glowtube.write_theme`'s JSON document; defaults are supplied by `glowtube` below.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThemeDocument {
    /// Theme schema version (1).
    pub version: u32,
    /// Display name, at most 48 Unicode characters.
    pub name: String,
    /// `dark` for GlowTube.
    pub appearance: String,
    /// Base canvas color (#07060b).
    pub canvas: String,
    /// Accent color (#ff2bd6).
    pub accent: String,
    /// Full stylesheet, at most 49,500 Unicode characters with no backslashes in values.
    pub css: String,
}

impl ThemeDocument {
    /// Port `write_theme`'s constant document fields; validate CSS size before constructing.
    pub fn glowtube(css: String) -> Result<Self> {
        let document = Self {
            version: 1,
            name: dt_core::constants::DISPLAY_NAME.into(),
            appearance: "dark".into(),
            canvas: "#07060b".into(),
            accent: "#ff2bd6".into(),
            css,
        };
        document.validate()?;
        Ok(document)
    }

    /// Check limits again at write time, since callers can construct or edit public fields.
    pub fn validate(&self) -> Result<()> {
        // T3 is JavaScript: count UTF-16 code units so astral Unicode cannot bypass its limit.
        if self.name.encode_utf16().count() > 48 || self.name.trim().is_empty() {
            return Err(Error::InvalidInput(
                "theme name must contain 1..48 characters",
            ));
        }
        if self.css.encode_utf16().count() > MAX_CSS_CHARS {
            return Err(Error::InvalidInput("theme CSS exceeds 49,500 characters"));
        }
        Ok(())
    }
}

/// Port `glowtube.write_theme` with an injected directory, safe for temporary-directory tests.
#[derive(Clone, Debug)]
pub struct ThemeWriter {
    /// Theme directory; construction does not create it or write anything.
    pub themes_dir: PathBuf,
}

impl ThemeWriter {
    /// Port `write_theme`: write a complete temp JSON beside glowtube.json, then atomically replace.
    /// Retry Windows sharing violations up to 20 times at 5 ms; report exhausted retries.
    /// Caller enforces >=150 ms gaps / 250 ms audio grid and favicon quiet holds. No in-place writes.
    pub fn write(&self, document: &ThemeDocument) -> Result<()> {
        document.validate()?;
        std::fs::create_dir_all(&self.themes_dir)?;
        atomic_json(&self.themes_dir.join("glowtube.json"), document)
    }
}

/// Port `service.ensure_theme`'s settings update: preserve unknown fields, set defaultTheme,
/// defaultAppearance=dark and their UTC timestamps, then atomically replace beside settings.json.
/// This function is an explicit mutation and must only be called by setup, using injected paths.
pub fn select_theme(settings_file: &Path, theme_id: &str) -> Result<()> {
    if theme_id.is_empty() {
        return Err(Error::InvalidInput("empty theme ID"));
    }
    let mut settings: serde_json::Value = match std::fs::read(settings_file) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| Error::Protocol("invalid T3 settings JSON"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.into()),
    };
    let settings = settings
        .as_object_mut()
        .ok_or(Error::Protocol("T3 settings must be an object"))?;
    // Match Python's no-op when the theme is already selected.
    if settings
        .get("defaultTheme")
        .and_then(serde_json::Value::as_str)
        == Some(theme_id)
    {
        return Ok(());
    }
    let timestamp = time::OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .map_err(|_| Error::Protocol("invalid UTC timestamp"))?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| Error::Protocol("could not format UTC timestamp"))?;
    let timestamp = timestamp.replace('Z', ".000Z");
    settings.insert("defaultTheme".into(), theme_id.into());
    settings.insert("defaultThemeSetAt".into(), timestamp.clone().into());
    settings.insert("defaultAppearance".into(), "dark".into());
    settings.insert("defaultAppearanceSetAt".into(), timestamp.into());
    let parent = settings_file
        .parent()
        .ok_or(Error::InvalidInput("settings path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    atomic_json(settings_file, settings)
}

pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or(Error::InvalidInput("atomic destination has no parent"))?,
    )?;
    serde_json::to_writer(&mut temp, value)
        .map_err(|_| Error::Protocol("could not serialize T3 JSON"))?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    atomic_persist(temp, path)
}

pub(crate) fn atomic_persist(mut temp: tempfile::NamedTempFile, path: &Path) -> Result<()> {
    for attempt in 0..20 {
        match temp.persist(path) {
            Ok(_) => return Ok(()),
            Err(error) => {
                let retry = cfg!(windows)
                    && (error.error.kind() == std::io::ErrorKind::PermissionDenied
                        || matches!(error.error.raw_os_error(), Some(32 | 33)));
                if !retry || attempt == 19 {
                    return Err(error.error.into());
                }
                temp = error.file;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    unreachable!("replacement loop always returns")
}
