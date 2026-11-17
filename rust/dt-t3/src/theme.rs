//! Atomic theme/settings replacement from `glowtube.write_theme` and `service.ensure_theme`.

use dt_core::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    pub fn glowtube(_css: String) -> Result<Self> {
        todo!("construct validated theme document")
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
    pub fn write(&self, _document: &ThemeDocument) -> Result<()> {
        todo!("atomic theme replacement")
    }
}

/// Port `service.ensure_theme`'s settings update: preserve unknown fields, set defaultTheme,
/// defaultAppearance=dark and their UTC timestamps, then atomically replace beside settings.json.
/// This function is an explicit mutation and must only be called by setup, using injected paths.
pub fn select_theme(_settings_file: &Path, _theme_id: &str) -> Result<()> {
    todo!("select theme atomically")
}
