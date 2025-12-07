use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use super::{
    ActiveDrag, EditorApp, EditorExit, Mode, Side, ToolbarAction, UiMeta,
    layout::{ratio_from_position, set_ratio},
    render::{hit_split, hit_toolbar, point_in_rect},
};

impl EditorApp {
    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Result<Option<EditorExit>> {
        match self.mode.clone() {
            Mode::Normal => self.handle_key_normal(key),
            Mode::EditCommand {
                mut buffer,
                mut cursor,
            } => {
                let out = self.handle_key_cmd(key, &mut buffer, &mut cursor);
                if matches!(self.mode, Mode::Normal) {
                    // stayed in normal
                } else {
                    self.mode = Mode::EditCommand { buffer, cursor };
                }
                out
            }
            Mode::EditWorkset { mut form } => {
                let out = self.handle_key_workset(key, &mut form);
                if matches!(self.mode, Mode::Normal) {
                    // stayed in normal
                } else {
                    self.mode = Mode::EditWorkset { form };
                }
                out
            }
            Mode::ConfirmDelete { target, slot_id } => {
                self.handle_key_confirm_delete(key, target, slot_id)
            }
        }
    }

    fn handle_key_normal(&mut self, key: KeyEvent) -> Result<Option<EditorExit>> {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return self.exit_saved(),
            KeyCode::Char('l') => self.move_tab(1),
            KeyCode::Char('k') => self.move_tab(-1),
            KeyCode::Char('j') => self.move_tab(1),
            KeyCode::Tab => self.move_tab(1),
            KeyCode::BackTab => self.move_tab(-1),
            KeyCode::Char('h') => self.split_selected(super::SplitDirection::Horizontal),
            KeyCode::Char('v') => self.split_selected(super::SplitDirection::Vertical),
            KeyCode::Char('s') => self.split_selected(super::SplitDirection::Vertical),
            KeyCode::Char('+') => self.bump_ratio(0.1),
            KeyCode::Char('-') => self.bump_ratio(-0.1),
            KeyCode::Char('x') => self.confirm_delete_selected(),
            KeyCode::Char('d') | KeyCode::Char('D') => self.confirm_delete_selected(),
            KeyCode::Enter | KeyCode::Char('c') => {
                self.start_edit_command();
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                self.start_edit_workset();
            }
            KeyCode::F(2) => {
                self.start_edit_workset();
            }
            KeyCode::Char('w') => {
                self.swap_with_next();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_cmd(
        &mut self,
        key: KeyEvent,
        buffer: &mut String,
        cursor: &mut usize,
    ) -> Result<Option<EditorExit>> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                if let Some(slot) = self.current_leaf_mut() {
                    slot.command = buffer.trim().to_string();
                }
                self.mark_changed();
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                if *cursor > 0
                    && let Some(prev) = prev_grapheme_start(buffer, *cursor)
                {
                    buffer.drain(prev..*cursor);
                    *cursor = prev;
                }
            }
            KeyCode::Char(ch) => {
                buffer.insert(*cursor, ch);
                *cursor += ch.len_utf8();
            }
            KeyCode::Left => {
                if let Some(prev) = prev_grapheme_start(buffer, *cursor) {
                    *cursor = prev;
                }
            }
            KeyCode::Right => {
                if let Some(next) = next_grapheme_end(buffer, *cursor) {
                    *cursor = next;
                }
            }
            KeyCode::Home => {
                *cursor = 0;
            }
            KeyCode::End => {
                *cursor = buffer.len();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_workset(
        &mut self,
        key: KeyEvent,
        form: &mut super::WorksetForm,
    ) -> Result<Option<EditorExit>> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Tab => {
                form.focus = next_field(form.focus);
                self.sync_workset_cursor(form);
            }
            KeyCode::BackTab => {
                form.focus = prev_field(form.focus);
                self.sync_workset_cursor(form);
            }
            KeyCode::Enter => {
                let new_id = form.id.trim();
                if new_id.is_empty() {
                    self.message = Some("ID cannot be empty".into());
                    return Ok(None);
                }
                if let Err(err) = self.validate_workset_id(new_id) {
                    self.message = Some(err.to_string());
                    return Ok(None);
                }
                self.workset.id = new_id.to_string();
                self.workset.name = form.name.trim().to_string();
                let workspace = form.workspace.trim();
                self.workset.workspace = if workspace.is_empty() {
                    None
                } else {
                    Some(workspace.to_string())
                };
                self.workset.desc = form.desc.trim().to_string();
                self.selected_tab_id = form.selected_tab.clone();
                self.mark_changed();
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => match form.focus {
                super::DialogField::Id => {
                    if form.cursor_id > 0
                        && let Some(prev) = prev_grapheme_start(&form.id, form.cursor_id)
                    {
                        form.id.drain(prev..form.cursor_id);
                        form.cursor_id = prev;
                    }
                }
                super::DialogField::Name => {
                    if form.cursor_name > 0
                        && let Some(prev) = prev_grapheme_start(&form.name, form.cursor_name)
                    {
                        form.name.drain(prev..form.cursor_name);
                        form.cursor_name = prev;
                    }
                }
                super::DialogField::Workspace => {
                    if form.cursor_workspace > 0
                        && let Some(prev) =
                            prev_grapheme_start(&form.workspace, form.cursor_workspace)
                    {
                        form.workspace.drain(prev..form.cursor_workspace);
                        form.cursor_workspace = prev;
                    }
                }
                super::DialogField::Desc => {
                    if form.cursor_desc > 0
                        && let Some(prev) = prev_grapheme_start(&form.desc, form.cursor_desc)
                    {
                        form.desc.drain(prev..form.cursor_desc);
                        form.cursor_desc = prev;
                    }
                }
                super::DialogField::Tab => {}
            },
            KeyCode::Char(ch) => match form.focus {
                super::DialogField::Id => {
                    form.id.insert(form.cursor_id, ch);
                    form.cursor_id += ch.len_utf8();
                }
                super::DialogField::Name => {
                    form.name.insert(form.cursor_name, ch);
                    form.cursor_name += ch.len_utf8();
                }
                super::DialogField::Workspace => {
                    form.workspace.insert(form.cursor_workspace, ch);
                    form.cursor_workspace += ch.len_utf8();
                }
                super::DialogField::Desc => {
                    form.desc.insert(form.cursor_desc, ch);
                    form.cursor_desc += ch.len_utf8();
                }
                super::DialogField::Tab => {
                    if matches!(ch, ' ' | 'l' | 'L' | 'h' | 'H') {
                        let delta = if matches!(ch, 'h' | 'H') { -1 } else { 1 };
                        self.cycle_tab_selection(form, delta);
                    }
                }
            },
            KeyCode::Left => match form.focus {
                super::DialogField::Id => {
                    if let Some(prev) = prev_grapheme_start(&form.id, form.cursor_id) {
                        form.cursor_id = prev;
                    }
                }
                super::DialogField::Name => {
                    if let Some(prev) = prev_grapheme_start(&form.name, form.cursor_name) {
                        form.cursor_name = prev;
                    }
                }
                super::DialogField::Workspace => {
                    if let Some(prev) = prev_grapheme_start(&form.workspace, form.cursor_workspace)
                    {
                        form.cursor_workspace = prev;
                    }
                }
                super::DialogField::Desc => {
                    if let Some(prev) = prev_grapheme_start(&form.desc, form.cursor_desc) {
                        form.cursor_desc = prev;
                    }
                }
                super::DialogField::Tab => self.cycle_tab_selection(form, -1),
            },
            KeyCode::Right => match form.focus {
                super::DialogField::Id => {
                    if let Some(next) = next_grapheme_end(&form.id, form.cursor_id) {
                        form.cursor_id = next;
                    }
                }
                super::DialogField::Name => {
                    if let Some(next) = next_grapheme_end(&form.name, form.cursor_name) {
                        form.cursor_name = next;
                    }
                }
                super::DialogField::Workspace => {
                    if let Some(next) = next_grapheme_end(&form.workspace, form.cursor_workspace) {
                        form.cursor_workspace = next;
                    }
                }
                super::DialogField::Desc => {
                    if let Some(next) = next_grapheme_end(&form.desc, form.cursor_desc) {
                        form.cursor_desc = next;
                    }
                }
                super::DialogField::Tab => self.cycle_tab_selection(form, 1),
            },
            KeyCode::Home => match form.focus {
                super::DialogField::Id => form.cursor_id = 0,
                super::DialogField::Name => form.cursor_name = 0,
                super::DialogField::Workspace => form.cursor_workspace = 0,
                super::DialogField::Desc => form.cursor_desc = 0,
                super::DialogField::Tab => {
                    form.selected_tab = None;
                }
            },
            KeyCode::End => match form.focus {
                super::DialogField::Id => form.cursor_id = form.id.len(),
                super::DialogField::Name => form.cursor_name = form.name.len(),
                super::DialogField::Workspace => form.cursor_workspace = form.workspace.len(),
                super::DialogField::Desc => form.cursor_desc = form.desc.len(),
                super::DialogField::Tab => {
                    if let Some(last) = self.tab_options.last() {
                        form.selected_tab = Some(last.id.clone());
                    }
                }
            },
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_confirm_delete(
        &mut self,
        key: KeyEvent,
        target: Vec<Side>,
        slot_id: Option<u32>,
    ) -> Result<Option<EditorExit>> {
        match key.code {
            KeyCode::Char('y') => {
                self.delete_at(&target);
                self.mode = Mode::Normal;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.mode = Mode::Normal;
                let label = slot_id
                    .map(|id| format!("Cancelled delete for #{}", id))
                    .unwrap_or_else(|| "Cancelled delete".into());
                self.message = Some(label);
            }
            _ => {}
        }
        Ok(None)
    }

    pub(super) fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        ui: &UiMeta,
    ) -> Result<Option<EditorExit>> {
        if !matches!(self.mode, Mode::Normal) {
            return Ok(None);
        }
        match mouse.kind {
            MouseEventKind::Moved => {
                let hit = hit_toolbar(mouse.column, mouse.row, ui);
                if hit != self.hover_toolbar {
                    self.hover_toolbar = hit;
                }
                self.hover_split = hit_split(mouse.column, mouse.row, ui).map(|h| h.path);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(action) = hit_toolbar(mouse.column, mouse.row, ui) {
                    self.hover_toolbar = Some(action);
                    if let Some(exit) = self.trigger_toolbar(action)? {
                        return Ok(Some(exit));
                    }
                    return Ok(None);
                }
                if let Some(hit) = hit_split(mouse.column, mouse.row, ui) {
                    self.hover_split = Some(hit.path.clone());
                    self.start_drag(hit);
                    return Ok(None);
                }
                if let Some(hit) = ui
                    .leaf_hits
                    .iter()
                    .find(|hit| point_in_rect(mouse.column, mouse.row, hit.rect))
                {
                    let double = self.is_double_click(&hit.path);
                    self.selected_path = hit.path.clone();
                    if double {
                        self.split_selected(super::SplitDirection::Horizontal);
                    }
                    self.last_click = Some((hit.path.clone(), Instant::now()));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.update_drag(mouse, ui);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.active_drag = None;
                self.hover_split = hit_split(mouse.column, mouse.row, ui).map(|h| h.path);
            }
            _ => {}
        }
        Ok(None)
    }

    fn start_drag(&mut self, hit: super::SplitHit) {
        self.active_drag = Some(ActiveDrag { path: hit.path });
        self.message = Some("Drag to adjust split ratio".into());
    }

    fn update_drag(&mut self, mouse: MouseEvent, ui: &UiMeta) {
        let Some(active) = &self.active_drag else {
            return;
        };
        let Some(hit) = ui.split_hits.iter().find(|h| h.path == active.path) else {
            return;
        };
        let Some(new_ratio) = ratio_from_position(hit.area, hit.direction, mouse.column, mouse.row)
        else {
            return;
        };

        if let Some((_, old, new)) = set_ratio(&mut self.root, &active.path, new_ratio)
            && (old - new).abs() > 0.0001
        {
            self.mark_changed();
        }

        // Update hover state after ratio adjustment
        self.hover_split = hit_split(mouse.column, mouse.row, ui).map(|h| h.path);
    }

    fn trigger_toolbar(&mut self, action: ToolbarAction) -> Result<Option<EditorExit>> {
        match action {
            ToolbarAction::Next => self.move_tab(1),
            ToolbarAction::SplitH => self.split_selected(super::SplitDirection::Horizontal),
            ToolbarAction::SplitV => self.split_selected(super::SplitDirection::Vertical),
            ToolbarAction::RatioUp => self.bump_ratio(0.1),
            ToolbarAction::RatioDown => self.bump_ratio(-0.1),
            ToolbarAction::Delete => self.confirm_delete_selected(),
            ToolbarAction::EditCmd => self.start_edit_command(),
            ToolbarAction::Swap => self.swap_with_next(),
            ToolbarAction::EditWorkset => self.start_edit_workset(),
            ToolbarAction::Cancel => return self.exit_saved(),
        }
        Ok(None)
    }

    fn is_double_click(&self, path: &[Side]) -> bool {
        if let Some((last, at)) = &self.last_click {
            last == path && at.elapsed() <= std::time::Duration::from_millis(350)
        } else {
            false
        }
    }
}

fn next_field(focus: super::DialogField) -> super::DialogField {
    match focus {
        super::DialogField::Id => super::DialogField::Name,
        super::DialogField::Name => super::DialogField::Tab,
        super::DialogField::Tab => super::DialogField::Workspace,
        super::DialogField::Workspace => super::DialogField::Desc,
        super::DialogField::Desc => super::DialogField::Id,
    }
}

fn prev_field(focus: super::DialogField) -> super::DialogField {
    match focus {
        super::DialogField::Id => super::DialogField::Desc,
        super::DialogField::Name => super::DialogField::Id,
        super::DialogField::Tab => super::DialogField::Name,
        super::DialogField::Workspace => super::DialogField::Tab,
        super::DialogField::Desc => super::DialogField::Workspace,
    }
}

fn prev_grapheme_start(text: &str, cursor: usize) -> Option<usize> {
    if cursor == 0 {
        return None;
    }
    text[..cursor].char_indices().last().map(|(i, _)| i)
}

fn next_grapheme_end(text: &str, cursor: usize) -> Option<usize> {
    if cursor >= text.len() {
        return None;
    }
    text[cursor..]
        .char_indices()
        .next()
        .map(|(_, ch)| cursor + ch.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_and_prev_field_cycle_through_form() {
        assert!(matches!(
            next_field(super::super::DialogField::Id),
            super::super::DialogField::Name
        ));
        assert!(matches!(
            prev_field(super::super::DialogField::Id),
            super::super::DialogField::Desc
        ));
        assert!(matches!(
            next_field(super::super::DialogField::Desc),
            super::super::DialogField::Id
        ));
    }

    #[test]
    fn grapheme_navigation_handles_multibyte() {
        let text = "aあい";
        // positions: 0 a, 1 あ, 4 い
        assert_eq!(next_grapheme_end(text, 0), Some(1));
        assert_eq!(next_grapheme_end(text, 1), Some(4));
        assert_eq!(next_grapheme_end(text, 4), Some(7));

        assert_eq!(prev_grapheme_start(text, 4), Some(1));
        assert_eq!(prev_grapheme_start(text, 1), Some(0));
        assert_eq!(prev_grapheme_start(text, 0), None);
    }
}
