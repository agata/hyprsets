use anyhow::{Result, bail};

use crate::config::{AppConfig, LayoutNode, WindowSlot};

use super::{
    EditorApp, EditorExit, Mode, Side, WorksetForm,
    layout::{
        adjust_ratio, collect_commands, leaf_at_path, next_slot_id, remove_leaf,
        replace_leaf_with_split, set_leaf_at_path,
    },
    persistence::persist_workset,
};

impl EditorApp {
    pub(super) fn exit_saved(&mut self) -> Result<Option<EditorExit>> {
        // 保存は各アクションのオートセーブで行うため、ここでは追加保存しない
        Ok(Some(EditorExit::Saved(self.workset.clone())))
    }

    pub(super) fn validate_workset_id(&self, candidate: &str) -> Result<()> {
        let cfg = AppConfig::load_or_init(&self.config_path)?;
        if cfg
            .worksets
            .iter()
            .any(|ws| ws.id == candidate && ws.id != self.saved_id)
        {
            bail!("ID already exists");
        }
        Ok(())
    }

    pub(super) fn sync_workset_cursor(&self, form: &mut WorksetForm) {
        match form.focus {
            super::DialogField::Id => form.cursor_id = form.id.len(),
            super::DialogField::Name => form.cursor_name = form.name.len(),
            super::DialogField::Workspace => form.cursor_workspace = form.workspace.len(),
            super::DialogField::Desc => form.cursor_desc = form.desc.len(),
            super::DialogField::Tab => {}
        }
    }

    pub(super) fn cycle_tab_selection(&self, form: &mut WorksetForm, delta: isize) {
        if self.tab_options.is_empty() {
            form.selected_tab = None;
            return;
        }
        let options_len = self.tab_options.len() + 1; // +1 for unassigned
        let current_idx = form
            .selected_tab
            .as_ref()
            .and_then(|id| self.tab_options.iter().position(|o| o.id == *id))
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let mut next = (current_idx as isize + delta).rem_euclid(options_len as isize);
        if next < 0 {
            next += options_len as isize;
        }
        let next = next as usize;
        form.selected_tab = if next == 0 {
            None
        } else {
            self.tab_options.get(next - 1).map(|opt| opt.id.clone())
        };
    }

    pub(super) fn current_leaf(&self) -> Option<&WindowSlot> {
        let mut node = &self.root;
        for side in &self.selected_path {
            match node {
                LayoutNode::Split(split) => {
                    node = if matches!(side, Side::Left) {
                        &split.left
                    } else {
                        &split.right
                    }
                }
                LayoutNode::Leaf(_) => break,
            }
        }
        if let LayoutNode::Leaf(slot) = node {
            Some(slot)
        } else {
            None
        }
    }

    pub(super) fn current_leaf_mut(&mut self) -> Option<&mut WindowSlot> {
        let mut node = &mut self.root;
        for side in self.selected_path.clone() {
            match node {
                LayoutNode::Split(split) => {
                    node = if matches!(side, Side::Left) {
                        &mut split.left
                    } else {
                        &mut split.right
                    }
                }
                LayoutNode::Leaf(_) => break,
            }
        }
        if let LayoutNode::Leaf(slot) = node {
            Some(slot)
        } else {
            None
        }
    }

    pub(super) fn current_slot_id(&self) -> Option<u32> {
        self.current_leaf().map(|s| s.slot_id)
    }

    pub(super) fn start_edit_command(&mut self) {
        let current_cmd = self
            .current_leaf()
            .map(|slot| slot.command.clone())
            .unwrap_or_default();
        let cursor = current_cmd.len();
        self.mode = Mode::EditCommand {
            buffer: current_cmd,
            cursor,
        };
    }

    pub(super) fn start_edit_workset(&mut self) {
        self.mode = Mode::EditWorkset {
            form: WorksetForm {
                id: self.workset.id.clone(),
                name: self.workset.name.clone(),
                workspace: self.workset.workspace.clone().unwrap_or_default(),
                desc: self.workset.desc.clone(),
                selected_tab: self.selected_tab_id.clone(),
                focus: super::DialogField::Name,
                cursor_id: self.workset.id.len(),
                cursor_name: self.workset.name.len(),
                cursor_workspace: self
                    .workset
                    .workspace
                    .as_ref()
                    .map(|s| s.len())
                    .unwrap_or(0),
                cursor_desc: self.workset.desc.len(),
            },
        };
    }

    pub(super) fn confirm_delete_selected(&mut self) {
        let target = self.selected_path.clone();
        let slot_id = self.current_slot_id();
        self.mode = Mode::ConfirmDelete { target, slot_id };
    }

    pub(super) fn move_tab(&mut self, delta: isize) {
        if self.leaf_paths.is_empty() {
            return;
        }
        let len = self.leaf_paths.len() as isize;
        let current_idx = self
            .leaf_paths
            .iter()
            .position(|p| p == &self.selected_path)
            .unwrap_or(0) as isize;
        let mut next = current_idx + delta;
        while next < 0 {
            next += len;
        }
        next %= len;
        if let Some(path) = self.leaf_paths.get(next as usize) {
            self.selected_path = path.clone();
        }
    }

    pub(super) fn split_selected(&mut self, direction: super::SplitDirection) {
        let next_id = next_slot_id(&self.root);
        let path = self.selected_path.clone();
        let replaced = replace_leaf_with_split(&mut self.root, &path, direction, next_id);
        if replaced {
            self.mark_changed();
        }
    }

    pub(super) fn bump_ratio(&mut self, delta: f32) {
        if let Some((_, old, new)) = adjust_ratio(&mut self.root, &self.selected_path, delta)
            && (old - new).abs() > 0.0001
        {
            self.mark_changed();
        }
    }

    pub(super) fn delete_at(&mut self, path: &[Side]) {
        if path.is_empty() {
            if let LayoutNode::Leaf(slot) = &mut self.root {
                slot.command.clear();
                self.message = Some("Cleared root leaf".into());
                self.mark_changed();
            }
            return;
        }
        if remove_leaf(&mut self.root, path) {
            self.selected_path = path[..path.len().saturating_sub(1)].to_vec();
            self.message = Some("Deleted slot".into());
            self.mark_changed();
        }
    }

    pub(super) fn swap_with_next(&mut self) {
        if self.leaf_paths.len() < 2 {
            self.message = Some("No adjacent slot to swap".into());
            return;
        }

        let current_idx = self
            .leaf_paths
            .iter()
            .position(|p| p == &self.selected_path)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.leaf_paths.len();
        let next_path = &self.leaf_paths[next_idx];

        let Some(current_leaf) = leaf_at_path(&self.root, &self.selected_path).cloned() else {
            return;
        };
        let Some(next_leaf) = leaf_at_path(&self.root, next_path).cloned() else {
            return;
        };

        if set_leaf_at_path(&mut self.root, &self.selected_path, next_leaf)
            && set_leaf_at_path(&mut self.root, next_path, current_leaf)
        {
            self.message = Some("Swapped with adjacent slot".into());
            self.mark_changed();
        }
    }

    pub(super) fn commit_workset(&mut self) {
        let mut commands = Vec::new();
        collect_commands(&self.root, &mut commands);
        self.workset.commands = commands;
        self.workset.layout = Some(self.root.clone());
    }

    pub(super) fn save_current(&mut self) -> Result<()> {
        self.commit_workset();
        persist_workset(
            &self.workset,
            &self.saved_id,
            &self.selected_tab_id,
            &self.config_path,
        )?;
        self.saved_id = self.workset.id.clone();
        Ok(())
    }

    pub(super) fn autosave(&mut self) {
        if let Err(err) = self.save_current() {
            self.message = Some(format!("Autosave failed: {err}"));
        }
    }

    pub(super) fn mark_changed(&mut self) {
        self.autosave();
    }
}
