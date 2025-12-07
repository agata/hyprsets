use std::path::Path;

use anyhow::{Result, bail};

use crate::config::{AppConfig, Workset};

use super::TabOption;

pub(super) fn load_tab_state(
    config_path: &Path,
    workset_id: &str,
) -> (Vec<TabOption>, Option<String>) {
    match AppConfig::load_or_init(config_path) {
        Ok(cfg) => {
            let mut selected = None;
            let mut options = Vec::new();
            for tab in cfg.tabs {
                if selected.is_none() && tab.worksets.iter().any(|id| id == workset_id) {
                    selected = Some(tab.id.clone());
                }
                options.push(TabOption {
                    id: tab.id.clone(),
                    label: tab.label.clone(),
                });
            }
            (options, selected)
        }
        Err(err) => {
            eprintln!("warning: failed to load tabs: {err}");
            (Vec::new(), None)
        }
    }
}

pub(super) fn persist_workset(
    ws: &Workset,
    saved_id: &str,
    selected_tab: &Option<String>,
    config_path: &Path,
) -> Result<()> {
    let mut cfg = AppConfig::load_or_init(config_path)?;

    if cfg
        .worksets
        .iter()
        .any(|existing| existing.id == ws.id && existing.id != saved_id)
    {
        bail!("ID already exists");
    }

    if let Some(idx) = cfg.worksets.iter().position(|w| w.id == saved_id) {
        cfg.worksets[idx] = ws.clone();
    } else if let Some(idx) = cfg.worksets.iter().position(|w| w.id == ws.id) {
        cfg.worksets[idx] = ws.clone();
    } else {
        cfg.worksets.push(ws.clone());
    }

    // apply tab assignment (single tab membership)
    for tab in cfg.tabs.iter_mut() {
        tab.worksets.retain(|id| id != saved_id && id != &ws.id);
    }
    if let Some(target_tab) = selected_tab {
        if let Some(tab) = cfg.tabs.iter_mut().find(|t| &t.id == target_tab) {
            if cfg.version < 2 {
                cfg.version = 2;
            }
            tab.worksets.push(ws.id.clone());
        } else {
            eprintln!("warning: selected tab '{target_tab}' not found; skipping assignment");
        }
    }

    cfg.save(config_path)
}
