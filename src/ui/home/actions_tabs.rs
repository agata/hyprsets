use std::io;

use anyhow::Result;
use crossterm::{cursor::Show, execute};
use rand::{Rng, distributions::Alphanumeric};

use super::{
    ALL_TAB_ID, HomeApp, ResolvedTab, TabAssignRow, TabAssignState, TabForm, TabFormField,
    TabRenameState,
};

impl HomeApp {
    pub(super) fn start_tab_new(&mut self) -> Result<()> {
        execute!(io::stdout(), Show)?;
        let form = TabForm {
            id: self.generate_tab_id(),
            label: String::new(),
            include_unassigned: false,
            focus: TabFormField::Label,
        };
        self.mode = super::Mode::TabNew(form);
        Ok(())
    }

    pub(super) fn start_tab_rename(&mut self) -> Result<()> {
        execute!(io::stdout(), Show)?;
        let Some(tab_id) = self.current_tab_id().filter(|id| *id != ALL_TAB_ID) else {
            self.message = Some("All tab cannot be renamed".into());
            self.hide_cursor()?;
            return Ok(());
        };
        let Some(tab) = self.cfg.tabs.iter().find(|t| t.id == tab_id) else {
            self.message = Some("Tab not found".into());
            self.hide_cursor()?;
            return Ok(());
        };
        let form = TabRenameState {
            tab_id: tab.id.clone(),
            label: tab.label.clone(),
        };
        self.mode = super::Mode::TabRename(form);
        Ok(())
    }

    pub(super) fn start_tab_delete(&mut self) -> Result<()> {
        if let Some(tab_id) = self.current_tab_id().filter(|id| *id != ALL_TAB_ID) {
            let tab_label = self
                .cfg
                .tabs
                .iter()
                .find(|t| t.id == tab_id)
                .map(|t| t.label.clone())
                .unwrap_or_else(|| tab_id.to_string());
            self.mode = super::Mode::ConfirmTabDelete {
                tab_id: tab_id.to_string(),
                tab_label,
            };
        } else {
            self.message = Some("All tab cannot be deleted".into());
        }
        Ok(())
    }

    pub(super) fn start_tab_assign(&mut self) -> Result<()> {
        let Some(ws) = self.current_workset() else {
            self.message = Some("Select a workset first".into());
            return Ok(());
        };
        if self.cfg.tabs.is_empty() {
            self.message = Some("No tabs to assign. Create a tab first.".into());
            return Ok(());
        }
        let mut memberships: Vec<TabAssignRow> = self
            .cfg
            .tabs
            .iter()
            .map(|tab| {
                let member = tab.worksets.iter().any(|id| id == &ws.id);
                TabAssignRow {
                    tab_id: tab.id.clone(),
                    label: tab.label.clone(),
                    member,
                }
            })
            .collect();

        let mut cursor = 0;
        let mut found_owner = false;
        for (idx, row) in memberships.iter_mut().enumerate() {
            if row.member {
                if found_owner {
                    row.member = false;
                } else {
                    found_owner = true;
                    cursor = idx;
                }
            }
        }

        self.mode = super::Mode::TabAssign(TabAssignState {
            workset_id: ws.id,
            cursor,
            memberships,
        });
        Ok(())
    }

    pub(super) fn start_tab_menu(&mut self) -> Result<()> {
        self.mode = super::Mode::TabMenu(super::TabMenuState { cursor: 0 });
        Ok(())
    }

    pub(super) fn generate_tab_id(&self) -> String {
        let mut rng = rand::thread_rng();
        loop {
            let suffix: String = (0..4)
                .map(|_| rng.sample(Alphanumeric) as char)
                .map(|c| c.to_ascii_lowercase())
                .collect();
            let candidate = format!("tab-{suffix}");
            if self.cfg.tabs.iter().all(|t| t.id != candidate) {
                return candidate;
            }
        }
    }

    pub(super) fn move_tab(&mut self, delta: isize) -> Result<()> {
        if self.tabs.len() <= 1 {
            return Ok(());
        }
        let len = self.tabs.len() as isize;
        let mut next = self.active_tab_idx as isize + delta;
        if next < 0 {
            next = len - 1;
        } else if next >= len {
            next = 0;
        }
        self.switch_tab(next as usize)?;
        Ok(())
    }

    pub(super) fn switch_tab(&mut self, idx: usize) -> Result<()> {
        if idx >= self.tabs.len() || idx == self.active_tab_idx {
            return Ok(());
        }
        self.record_current_selection();
        self.active_tab_idx = idx;
        let preferred = self
            .current_tab_id()
            .and_then(|tab_id| self.tab_selected_ids.get(tab_id).cloned());
        self.apply_selection_for_active_tab(preferred);
        self.persist_last_tab();
        Ok(())
    }

    pub(super) fn persist_last_tab(&mut self) {
        if let Some(id) = self.current_tab_id() {
            self.state.last_tab_id = Some(id.to_string());
            if let Err(err) = self.state.save(&self.state_path) {
                self.message = Some(format!("failed to save tab state: {err}"));
            }
        }
    }

    pub(super) fn current_tab(&self) -> Option<&ResolvedTab> {
        self.tabs.get(self.active_tab_idx)
    }

    pub(super) fn current_tab_id(&self) -> Option<&str> {
        self.current_tab().map(|t| t.id.as_str())
    }

    pub(super) fn tab_index_by_id(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub(super) fn current_tab_len(&self) -> usize {
        self.current_tab().map(|t| t.indices.len()).unwrap_or(0)
    }

    pub(super) fn index_in_current_tab(&self, workset_id: &str) -> Option<usize> {
        let tab = self.current_tab()?;
        tab.indices.iter().position(|idx| {
            self.cfg
                .worksets
                .get(*idx)
                .map(|ws| ws.id.as_str() == workset_id)
                .unwrap_or(false)
        })
    }

    pub(super) fn workset_at_tab_index(&self, tab_idx: usize) -> Option<&crate::config::Workset> {
        let tab = self.current_tab()?;
        let global_idx = *tab.indices.get(tab_idx)?;
        self.cfg.worksets.get(global_idx)
    }

    pub(super) fn current_user_tab_index(&self) -> Option<usize> {
        let id = self.current_tab_id()?;
        if id == ALL_TAB_ID {
            None
        } else {
            self.cfg.tabs.iter().position(|t| t.id == id)
        }
    }

    pub(super) fn move_active_tab(&mut self, delta: isize) -> Result<()> {
        let Some(idx) = self.current_user_tab_index() else {
            self.message = Some("Cannot reorder All tab".into());
            return Ok(());
        };
        if self.cfg.tabs.len() <= 1 {
            return Ok(());
        }
        let target = idx as isize + delta;
        if target < 0 || target >= self.cfg.tabs.len() as isize {
            self.message = Some("Cannot move tab further".into());
            return Ok(());
        }
        let id = self.cfg.tabs[idx].id.clone();
        self.cfg.tabs.swap(idx, target as usize);
        self.message = Some("Tab order updated".into());
        self.save()?;
        self.apply_tabs(Some(id), self.current_id());
        Ok(())
    }

    pub(super) fn create_tab(&mut self, form: &TabForm) -> Result<bool> {
        let id = form.id.trim();
        let label = form.label.trim();
        if id.is_empty() {
            self.message = Some("Tab ID is required".into());
            return Ok(false);
        }
        if label.is_empty() {
            self.message = Some("Tab label is required".into());
            return Ok(false);
        }
        if self.cfg.tabs.iter().any(|t| t.id == id) {
            self.message = Some("Tab ID already exists".into());
            return Ok(false);
        }
        self.ensure_tab_version();
        self.cfg.tabs.push(crate::config::TabConfig {
            id: id.to_string(),
            label: label.to_string(),
            worksets: Vec::new(),
            include_unassigned: form.include_unassigned,
        });
        self.message = Some(format!("Tab added: {id}"));
        self.save()?;
        self.apply_tabs(Some(id.to_string()), self.current_id());
        Ok(true)
    }

    pub(super) fn rename_tab(&mut self, form: &TabRenameState) -> Result<bool> {
        let label = form.label.trim();
        if label.is_empty() {
            self.message = Some("Tab label is required".into());
            return Ok(false);
        }
        let Some(tab) = self.cfg.tabs.iter_mut().find(|t| t.id == form.tab_id) else {
            self.message = Some("Tab not found".into());
            return Ok(false);
        };
        tab.label = label.to_string();
        self.message = Some("Tab renamed".into());
        self.save()?;
        self.apply_tabs(Some(form.tab_id.clone()), self.current_id());
        Ok(true)
    }

    pub(super) fn delete_tab(&mut self, tab_id: &str) -> Result<()> {
        let before = self.cfg.tabs.len();
        self.cfg.tabs.retain(|t| t.id != tab_id);
        if self.cfg.tabs.len() == before {
            self.message = Some("Tab not found".into());
            return Ok(());
        }
        self.message = Some(format!("Deleted tab: {tab_id}"));
        let selection = self.current_id();
        self.save()?;
        self.apply_tabs(None, selection);
        Ok(())
    }

    pub(super) fn assign_workset_to_tab(&mut self, workset_id: &str, tab_id: Option<&str>) -> bool {
        if self.cfg.tabs.is_empty() {
            return true;
        }
        self.ensure_tab_version();
        let mut previous_tab_id = None;
        let mut previous_pos = None;
        for tab in &self.cfg.tabs {
            if let Some(pos) = tab.worksets.iter().position(|id| id == workset_id) {
                previous_tab_id = Some(tab.id.clone());
                previous_pos = Some(pos);
                break;
            }
        }
        for tab in self.cfg.tabs.iter_mut() {
            tab.worksets.retain(|id| id != workset_id);
        }
        if let Some(target) = tab_id {
            if let Some(tab) = self.cfg.tabs.iter_mut().find(|t| t.id == target) {
                let insert_at = if previous_tab_id.as_deref() == Some(target) {
                    previous_pos.unwrap_or(tab.worksets.len())
                } else {
                    tab.worksets.len()
                }
                .min(tab.worksets.len());
                tab.worksets.insert(insert_at, workset_id.to_string());
            } else {
                self.message = Some("Tab not found".into());
                return false;
            }
        }
        true
    }

    pub(super) fn remove_workset_from_tabs(&mut self, workset_id: &str) {
        if self.cfg.tabs.is_empty() {
            return;
        }
        for tab in self.cfg.tabs.iter_mut() {
            tab.worksets.retain(|id| id != workset_id);
        }
    }

    pub(super) fn apply_tab_assignment(&mut self, state: &TabAssignState) -> Result<()> {
        self.ensure_tab_version();
        let target_tab = state
            .memberships
            .iter()
            .find(|row| row.member)
            .map(|row| row.tab_id.clone());

        self.assign_workset_to_tab(&state.workset_id, target_tab.as_deref());
        if self.message.is_none() {
            if target_tab.is_some() {
                self.message = Some("Tab assignment updated".into());
            } else {
                self.message = Some("Workset unassigned from tabs".into());
            }
        }
        let preferred_tab = self.current_tab_id().map(str::to_string);
        self.save()?;
        self.apply_tabs(preferred_tab, Some(state.workset_id.clone()));
        Ok(())
    }

    pub(super) fn reorder_in_tab(
        &mut self,
        from_idx: usize,
        to_idx: usize,
        tab: &ResolvedTab,
    ) -> Result<()> {
        let Some(tab_id) = self.current_tab_id().map(str::to_string) else {
            return Ok(());
        };
        let Some(tab_cfg) = self.cfg.tabs.iter_mut().find(|t| t.id == tab_id) else {
            self.message = Some("Tab not found".into());
            return Ok(());
        };

        let mut ordered_ids: Vec<String> = tab
            .indices
            .iter()
            .filter_map(|idx| self.cfg.worksets.get(*idx))
            .map(|ws| ws.id.clone())
            .collect();

        if from_idx >= ordered_ids.len() || to_idx >= ordered_ids.len() {
            return Ok(());
        }

        ordered_ids.swap(from_idx, to_idx);
        tab_cfg.worksets = ordered_ids;
        let direction = if to_idx < from_idx { "up" } else { "down" };
        self.message = Some(format!("Moved {} in tab", direction));
        Ok(())
    }

    pub(super) fn ensure_tab_version(&mut self) {
        if self.cfg.version < 2 {
            self.cfg.version = 2;
        }
    }

    pub(super) fn is_all_tab_active(&self) -> bool {
        matches!(self.current_tab_id(), Some(id) if id == ALL_TAB_ID)
    }

    pub(super) fn has_user_tabs(&self) -> bool {
        (self.cfg.version >= 2 || !self.cfg.tabs.is_empty()) && !self.cfg.tabs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::Mode;
    use super::HomeApp;
    use crate::{
        config::{AppConfig, TabConfig, Workset},
        state::AppState,
    };
    use ratatui::widgets::TableState;
    use std::path::PathBuf;

    fn sample_workset(id: &str) -> Workset {
        Workset {
            id: id.to_string(),
            name: format!("Workset {id}"),
            desc: String::new(),
            workspace: None,
            commands: vec![],
            cwd: None,
            env: Default::default(),
            layout: None,
        }
    }

    fn test_app(cfg: AppConfig) -> HomeApp {
        HomeApp {
            cfg,
            config_path: PathBuf::new(),
            state_path: PathBuf::new(),
            state: AppState::default(),
            table_state: TableState::default(),
            scroll: 0,
            mode: Mode::Normal,
            message: None,
            last_click: None,
            hover_toolbar: None,
            hover_tab: None,
            numeric_input: None,
            tabs: Vec::new(),
            active_tab_idx: 0,
            tab_selected_ids: Default::default(),
            workset_tab_labels: Default::default(),
        }
    }

    #[test]
    fn assign_workset_to_tab_preserves_order_when_same_tab() {
        let cfg = AppConfig {
            version: 2,
            default_tab: None,
            show_all_tab: None,
            all_tab_position: None,
            tabs: vec![TabConfig {
                id: "tabA".into(),
                label: "A".into(),
                worksets: vec!["w1".into(), "w2".into(), "w3".into()],
                include_unassigned: false,
            }],
            worksets: vec![
                sample_workset("w1"),
                sample_workset("w2"),
                sample_workset("w3"),
            ],
        };

        let mut app = test_app(cfg);
        let ok = app.assign_workset_to_tab("w2", Some("tabA"));
        assert!(ok);

        let tab = app.cfg.tabs.iter().find(|t| t.id == "tabA").unwrap();
        assert_eq!(tab.worksets, vec!["w1", "w2", "w3"]);
    }
}
