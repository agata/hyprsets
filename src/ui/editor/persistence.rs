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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::config::{LayoutNode, SplitDirection, SplitNode, TabConfig, WindowSlot};

    fn temp_config_path(prefix: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hyprsets_test_{prefix}_{suffix}"));
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    fn sample_leaf(slot_id: u32, command: &str) -> LayoutNode {
        LayoutNode::Leaf(WindowSlot {
            slot_id,
            command: command.to_string(),
            cwd: None,
            env: Default::default(),
            wait_after_ms: None,
        })
    }

    fn cleanup(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn load_tab_state_selects_first_matching_tab() {
        let path = temp_config_path("load_tab_state");
        let cfg = AppConfig {
            version: 1,
            default_tab: None,
            show_all_tab: None,
            all_tab_position: None,
            tabs: vec![
                TabConfig {
                    id: "t1".into(),
                    label: "Tab1".into(),
                    worksets: vec!["w1".into()],
                    include_unassigned: false,
                },
                TabConfig {
                    id: "t2".into(),
                    label: "Tab2".into(),
                    worksets: vec!["w1".into(), "w2".into()],
                    include_unassigned: false,
                },
            ],
            worksets: vec![],
        };
        cfg.save(&path).unwrap();

        let (options, selected) = load_tab_state(&path, "w1");
        assert_eq!(options.len(), 2);
        assert_eq!(selected.as_deref(), Some("t1"));

        cleanup(&path);
    }

    #[test]
    fn persist_workset_replaces_saved_id_and_assigns_tab() {
        let path = temp_config_path("persist_workset");
        let existing = Workset {
            id: "old".into(),
            name: "Old".into(),
            desc: "".into(),
            workspace: None,
            commands: vec!["oldcmd".into()],
            cwd: None,
            env: Default::default(),
            layout: Some(sample_leaf(1, "oldcmd")),
        };
        let cfg = AppConfig {
            version: 1,
            default_tab: None,
            show_all_tab: None,
            all_tab_position: None,
            tabs: vec![
                TabConfig {
                    id: "tabA".into(),
                    label: "A".into(),
                    worksets: vec!["old".into()],
                    include_unassigned: false,
                },
                TabConfig {
                    id: "tabB".into(),
                    label: "B".into(),
                    worksets: vec![],
                    include_unassigned: false,
                },
            ],
            worksets: vec![existing],
        };
        cfg.save(&path).unwrap();

        let new_ws = Workset {
            id: "new".into(),
            name: "New".into(),
            desc: "d".into(),
            workspace: Some("ws".into()),
            commands: vec!["newcmd".into()],
            cwd: None,
            env: Default::default(),
            layout: Some(LayoutNode::Split(SplitNode {
                direction: SplitDirection::Vertical,
                ratio: 1.0,
                left: Box::new(sample_leaf(1, "a")),
                right: Box::new(sample_leaf(2, "b")),
            })),
        };
        persist_workset(&new_ws, "old", &Some("tabB".into()), &path).unwrap();

        let updated = AppConfig::load_or_init(&path).unwrap();
        assert_eq!(updated.version, 2);
        assert_eq!(updated.worksets.len(), 1);
        assert_eq!(updated.worksets[0].id, "new");
        let tab_a = updated.tabs.iter().find(|t| t.id == "tabA").unwrap();
        assert!(tab_a.worksets.is_empty());
        let tab_b = updated.tabs.iter().find(|t| t.id == "tabB").unwrap();
        assert_eq!(tab_b.worksets, vec!["new"]);

        cleanup(&path);
    }

    #[test]
    fn persist_workset_errors_on_conflicting_id() {
        let path = temp_config_path("persist_workset_conflict");
        let cfg = AppConfig {
            version: 1,
            default_tab: None,
            show_all_tab: None,
            all_tab_position: None,
            tabs: vec![],
            worksets: vec![Workset {
                id: "other".into(),
                name: "Other".into(),
                desc: "".into(),
                workspace: None,
                commands: vec![],
                cwd: None,
                env: Default::default(),
                layout: Some(sample_leaf(1, "x")),
            }],
        };
        cfg.save(&path).unwrap();

        let ws = Workset {
            id: "other".into(),
            name: "New".into(),
            desc: "".into(),
            workspace: None,
            commands: vec![],
            cwd: None,
            env: Default::default(),
            layout: Some(sample_leaf(2, "y")),
        };
        let err = persist_workset(&ws, "different", &None, &path).unwrap_err();
        assert!(format!("{err}").contains("ID already exists"));

        cleanup(&path);
    }

    #[test]
    fn load_tab_state_returns_empty_on_missing_file() {
        let path = temp_config_path("load_missing");
        let (opts, selected) = load_tab_state(&path, "whatever");
        assert!(opts.is_empty());
        assert!(selected.is_none());
        cleanup(&path);
    }
}
