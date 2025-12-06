use std::collections::HashMap;

use anyhow::Result;
use rand::{Rng, distributions::Alphanumeric};

use crate::{config::Workset, run::workspace_cleanup_status};

use super::HomeApp;

impl HomeApp {
    pub(super) fn begin_run_selected(&mut self) -> Result<Option<super::HomeExit>> {
        let Some(ws) = self.current_workset() else {
            return Ok(None);
        };
        self.begin_run(ws)
    }

    pub(super) fn begin_run(&mut self, ws: Workset) -> Result<Option<super::HomeExit>> {
        let status = workspace_cleanup_status(Some(&ws))?;
        let id = ws.id.clone();
        let name = ws.name.clone();
        if status.closable_windows == 0 {
            return Ok(Some(super::HomeExit::Run {
                id,
                preconfirm_clean: false,
            }));
        }

        self.mode = super::Mode::ConfirmRun(super::ConfirmRunState {
            id,
            workset_name: name,
            workspace_name: status.workspace_name,
            window_count: status.closable_windows,
        });
        Ok(None)
    }

    pub(super) fn delete_at(&mut self, idx: usize) -> Result<()> {
        let Some(global_idx) = self
            .current_tab()
            .and_then(|tab| tab.indices.get(idx))
            .copied()
        else {
            return Ok(());
        };
        let next_selection = self
            .current_tab()
            .and_then(|tab| {
                tab.indices
                    .get(idx + 1)
                    .or_else(|| idx.checked_sub(1).and_then(|i| tab.indices.get(i)))
            })
            .and_then(|next_idx| self.cfg.worksets.get(*next_idx))
            .map(|ws| ws.id.clone());

        let removed = self.cfg.worksets.remove(global_idx);
        self.remove_workset_from_tabs(&removed.id);
        self.message = Some(format!("Deleted: {}", removed.name));
        self.table_state.select(None);
        self.scroll = 0;
        self.save()?;
        let current_tab = self.current_tab_id().map(str::to_string);
        self.apply_tabs(current_tab, next_selection);
        Ok(())
    }

    pub(super) fn clone_selected(&mut self) -> Result<()> {
        let Some(sel) = self.selected_global_index() else {
            return Ok(());
        };
        let Some(ws) = self.cfg.worksets.get(sel) else {
            return Ok(());
        };
        let new_id = self.generate_id();
        let mut new_ws = ws.clone();
        new_ws.id = new_id.clone();
        new_ws.name = format!("{} (copy)", ws.name);
        self.cfg.worksets.push(new_ws);
        self.message = Some(format!("Duplicated: {}", new_id));
        self.save()?;
        let tab_id = self.current_tab_id().map(str::to_string);
        self.apply_tabs(tab_id, Some(new_id));
        Ok(())
    }

    pub(super) fn move_workset(&mut self, delta: isize, _visible_rows: usize) -> Result<()> {
        let Some(selected_idx) = self.table_state.selected() else {
            return Ok(());
        };
        let Some(tab) = self.current_tab().cloned() else {
            return Ok(());
        };

        if tab.indices.len() <= 1 {
            return Ok(());
        }

        let target = selected_idx as isize + delta;
        if target < 0 || target >= tab.indices.len() as isize {
            self.message = Some("Cannot move further".into());
            return Ok(());
        }

        let selected_global = tab.indices[selected_idx];
        let selected_id = self
            .cfg
            .worksets
            .get(selected_global)
            .map(|ws| ws.id.clone());

        if self.is_all_tab_active() || !self.has_user_tabs() {
            let target_global = tab.indices[target as usize];
            self.reorder_in_all_tab(selected_global, target_global)?;
        } else {
            self.reorder_in_tab(selected_idx, target as usize, &tab)?;
        }

        self.save()?;
        let tab_id = self.current_tab_id().map(str::to_string);
        self.apply_tabs(tab_id, selected_id);
        Ok(())
    }

    fn reorder_in_all_tab(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= self.cfg.worksets.len() || to >= self.cfg.worksets.len() {
            return Ok(());
        }
        self.cfg.worksets.swap(from, to);
        let direction = if to < from { "up" } else { "down" };
        self.message = Some(format!("Moved {}", direction));
        Ok(())
    }

    pub(super) fn create_new(&mut self, form: &super::NewDialogState) -> Result<bool> {
        let id = form.id.trim();
        let name = form.name.trim();
        let desc = form.desc.trim();

        if id.is_empty() {
            self.message = Some("Please enter an id".into());
            return Ok(false);
        }
        if self.cfg.worksets.iter().any(|existing| existing.id == id) {
            self.message = Some("ID already exists".into());
            return Ok(false);
        }
        if name.is_empty() {
            self.message = Some("Please enter a name".into());
            return Ok(false);
        }

        let ws = Workset {
            id: id.to_string(),
            name: name.to_string(),
            desc: desc.to_string(),
            workspace: None,
            commands: vec![],
            cwd: None,
            env: HashMap::new(),
            layout: None,
        };
        self.cfg.worksets.push(ws);
        let assigned = self.assign_workset_to_tab(id, form.selected_tab.as_deref());
        if assigned {
            if let Some(tab_id) = form.selected_tab.as_deref() {
                self.message = Some(format!("Added: {} (tab: {})", id, tab_id));
            } else {
                self.message = Some(format!("Added: {}", id));
            }
        }
        self.save()?;
        let tab_id = self.current_tab_id().map(str::to_string);
        self.apply_tabs(tab_id, Some(id.to_string()));
        Ok(true)
    }

    pub(super) fn generate_id(&self) -> String {
        let mut rng = rand::thread_rng();
        loop {
            let suffix: String = (0..6)
                .map(|_| rng.sample(Alphanumeric) as char)
                .map(|c| c.to_ascii_lowercase())
                .collect();
            let candidate = format!("ws-{suffix}");
            if self.cfg.worksets.iter().all(|w| w.id != candidate) {
                return candidate;
            }
        }
    }

    pub(super) fn selected_global_index(&self) -> Option<usize> {
        let tab = self.current_tab()?;
        let selected = self.table_state.selected()?;
        tab.indices.get(selected).copied()
    }

    pub(super) fn current_workset(&self) -> Option<Workset> {
        self.selected_global_index()
            .and_then(|idx| self.cfg.worksets.get(idx))
            .cloned()
    }

    pub(super) fn current_id(&self) -> Option<String> {
        self.current_workset().map(|ws| ws.id)
    }
}
