use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub last_tab_id: Option<String>,
}

pub fn default_state_path() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".config").join("hyprsets").join("state.toml")
}

impl AppState {
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_else(|err| {
                eprintln!(
                    "warning: failed to parse state file {}: {}",
                    path.display(),
                    err
                );
                Self::default()
            }),
            Err(err) if err.kind() == ErrorKind::NotFound => Self::default(),
            Err(err) => {
                eprintln!(
                    "warning: failed to read state file {}: {}",
                    path.display(),
                    err
                );
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create state dir: {}", parent.display()))?;
        }
        let serialized = toml::to_string_pretty(self).context("failed to serialize state")?;
        fs::write(path, serialized)
            .with_context(|| format!("failed to write state file: {}", path.display()))
    }
}
