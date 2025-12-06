use std::collections::{HashMap, HashSet};

use crate::state::AppState;

use super::{ALL_TAB_ID, HomeApp, ResolvedTab};

impl HomeApp {
    pub(super) fn initialize_tabs(&mut self, initial_selected_id: Option<String>) {
        let preferred_tab = self.state.last_tab_id.clone();
        self.apply_tabs(preferred_tab, initial_selected_id);
    }

    pub(super) fn apply_tabs(
        &mut self,
        preferred_tab: Option<String>,
        preferred_selection: Option<String>,
    ) {
        let (tabs, warnings, labels) = self.build_tabs();
        self.tabs = tabs;
        self.workset_tab_labels = labels;
        self.hover_tab = None;
        if let Some(warning) = warnings.last()
            && self.message.is_none()
        {
            self.message = Some(warning.clone());
        }
        if self.tabs.is_empty() {
            self.active_tab_idx = 0;
            self.table_state.select(None);
        } else {
            let tab_idx = self
                .resolve_tab_index(preferred_tab.as_deref())
                .min(self.tabs.len().saturating_sub(1));
            self.active_tab_idx = tab_idx;
            self.apply_selection_for_active_tab(preferred_selection);
            self.persist_last_tab();
        }
    }

    pub(super) fn build_tabs(&self) -> (Vec<ResolvedTab>, Vec<String>, HashMap<String, String>) {
        let mut warnings = Vec::new();
        let mut tabs = Vec::new();
        let mut labels: HashMap<String, String> = HashMap::new();
        let all_tab = ResolvedTab {
            id: ALL_TAB_ID.to_string(),
            label: "All".to_string(),
            indices: (0..self.cfg.worksets.len()).collect(),
        };

        let tab_enabled = self.cfg.version >= 2 || !self.cfg.tabs.is_empty();
        if self.cfg.tabs.is_empty() || !tab_enabled {
            tabs.push(all_tab);
            return (tabs, warnings, labels);
        }

        let index_map: HashMap<String, usize> = self
            .cfg
            .worksets
            .iter()
            .enumerate()
            .map(|(idx, ws)| (ws.id.clone(), idx))
            .collect();

        let mut assigned: HashSet<String> = HashSet::new();

        for tab in &self.cfg.tabs {
            let mut indices = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for id in &tab.worksets {
                if let Some(idx) = index_map.get(id) {
                    if assigned.contains(id) {
                        let msg = format!(
                            "workset '{}' already assigned to another tab; skipped in '{}'",
                            id, tab.id
                        );
                        eprintln!("warning: {msg}");
                        warnings.push(msg);
                        continue;
                    }
                    if seen.insert(id.clone()) {
                        indices.push(*idx);
                        labels.insert(id.clone(), tab.label.clone());
                        assigned.insert(id.clone());
                    }
                } else {
                    let msg = format!("tab '{}' references missing workset '{}'", tab.id, id);
                    eprintln!("warning: {msg}");
                    warnings.push(msg);
                }
            }
            if tab.include_unassigned {
                let mut extras: Vec<_> = index_map
                    .iter()
                    .filter(|(id, _)| {
                        !seen.contains::<str>(id.as_str()) && !assigned.contains::<str>(id.as_str())
                    })
                    .map(|(id, idx)| (id.clone(), *idx))
                    .collect();
                extras.sort_by(|a, b| a.0.cmp(&b.0));
                for (id, idx) in extras {
                    if seen.insert(id.clone()) {
                        indices.push(idx);
                        labels.insert(id.clone(), tab.label.clone());
                        assigned.insert(id);
                    }
                }
            }
            tabs.push(ResolvedTab {
                id: tab.id.clone(),
                label: tab.label.clone(),
                indices,
            });
        }

        tabs.push(all_tab);

        (tabs, warnings, labels)
    }

    pub(super) fn resolve_tab_index(&self, preferred: Option<&str>) -> usize {
        if let Some(id) = preferred
            && let Some(idx) = self.tab_index_by_id(id)
        {
            return idx;
        }
        if let Some(id) = self.state.last_tab_id.as_deref()
            && let Some(idx) = self.tab_index_by_id(id)
        {
            return idx;
        }
        if let Some(id) = self.cfg.default_tab.as_deref()
            && let Some(idx) = self.tab_index_by_id(id)
        {
            return idx;
        }
        0
    }

    pub(super) fn apply_selection_for_active_tab(&mut self, preferred: Option<String>) {
        let selected_idx = preferred
            .as_deref()
            .and_then(|id| self.index_in_current_tab(id))
            .or_else(|| {
                self.current_tab_id()
                    .and_then(|tab_id| self.tab_selected_ids.get(tab_id).cloned())
                    .and_then(|id| self.index_in_current_tab(&id))
            })
            .or_else(|| (self.current_tab_len() > 0).then_some(0));

        self.table_state.select(selected_idx);
        self.scroll = 0;
        self.record_current_selection();
    }

    pub(super) fn load_state(&mut self) {
        self.state = AppState::load(&self.state_path);
    }

    pub(super) fn load_and_apply_tabs(&mut self, initial_selected_id: Option<String>) {
        self.load_state();
        self.initialize_tabs(initial_selected_id);
    }
}
