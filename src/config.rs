use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    #[serde(rename = "workset")]
    pub worksets: Vec<Workset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workset {
    pub id: String,
    pub name: String,
    pub desc: String,
    #[serde(default)]
    pub workspace: Option<String>,
    pub commands: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub layout: Option<LayoutNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LayoutNode {
    Leaf(WindowSlot),
    Split(SplitNode),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowSlot {
    pub slot_id: u32,
    pub command: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitNode {
    pub direction: SplitDirection,
    pub ratio: f32,
    pub left: Box<LayoutNode>,
    pub right: Box<LayoutNode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

impl AppConfig {
    /// Load config from path. If the file is missing or empty, write and return a default template.
    pub fn load_or_init(path: &Path) -> Result<Self> {
        if path.exists() {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read config file: {}", path.display()))?;
            if raw.trim().is_empty() {
                return Self::write_default(path);
            }
            let cfg: Self = toml::from_str(&raw)
                .with_context(|| format!("failed to parse config file: {}", path.display()))?;
            return Ok(cfg);
        }

        Self::write_default(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.ensure_parent(path)?;
        let serialized = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(path, serialized)
            .with_context(|| format!("failed to write config file: {}", path.display()))
    }

    fn write_default(path: &Path) -> Result<Self> {
        let cfg = Self::default_template();
        cfg.save(path)?;
        Ok(cfg)
    }

    fn ensure_parent(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config dir: {}", parent.display()))?;
        }
        Ok(())
    }

    fn default_template() -> Self {
        let layout = LayoutNode::Split(SplitNode {
            direction: SplitDirection::Horizontal,
            ratio: 1.2,
            left: Box::new(LayoutNode::Leaf(WindowSlot {
                slot_id: 1,
                command: String::from(r#"code -n "$HOME/ws/demo""#),
                cwd: None,
                env: HashMap::new(),
            })),
            right: Box::new(LayoutNode::Leaf(WindowSlot {
                slot_id: 2,
                command: String::from(
                    r#"omarchy-launch-browser --new-window "https://example.com""#,
                ),
                cwd: None,
                env: HashMap::new(),
            })),
        });

        let workset = Workset {
            id: "sample".into(),
            name: "Sample Workset".into(),
            desc: "Code + Browser example".into(),
            workspace: Some(String::new()),
            commands: vec![
                String::from(r#"code -n "$HOME/ws/demo""#),
                String::from(r#"omarchy-launch-browser --new-window "https://example.com""#),
                String::from(r#"hyprctl dispatch movefocus l"#),
                String::from(r#"hyprctl dispatch splitratio exact 1.2"#),
            ],
            cwd: None,
            env: HashMap::new(),
            layout: Some(layout),
        };

        Self {
            version: 1,
            worksets: vec![workset],
        }
    }
}

pub fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".config").join("hyprsets").join("hyprsets.toml")
}
