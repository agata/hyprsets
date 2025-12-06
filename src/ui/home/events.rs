use std::time::Instant;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};

use super::{
    DialogField, HomeApp, HomeExit, LastClick, Mode, TabFormField, TabMenuItem, ToolbarAction,
    UiMeta, cycle_new_dialog_tab,
};

impl HomeApp {
    pub(super) fn handle_key(&mut self, key: KeyEvent, ui: &UiMeta) -> Result<Option<HomeExit>> {
        match self.mode.clone() {
            Mode::Normal => self.handle_key_normal(key, ui),
            Mode::ConfirmDelete { idx } => {
                match key.code {
                    KeyCode::Char('y') => {
                        self.mode = Mode::Normal;
                        self.delete_at(idx)?;
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        self.message = Some("Deletion cancelled".into());
                    }
                    _ => {}
                }
                Ok(None)
            }
            Mode::ConfirmRun(state) => match key.code {
                KeyCode::Char('y') => {
                    self.mode = Mode::Normal;
                    Ok(Some(HomeExit::Run {
                        id: state.id,
                        preconfirm_clean: true,
                    }))
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    self.message = Some("Launch cancelled".into());
                    Ok(None)
                }
                _ => Ok(None),
            },
            Mode::NewDialog(_) => self.handle_key_new_dialog(key),
            Mode::TabNew(_) => self.handle_key_tab_new(key),
            Mode::TabRename(_) => self.handle_key_tab_rename(key),
            Mode::ConfirmTabDelete { tab_id, .. } => {
                match key.code {
                    KeyCode::Char('y') => {
                        self.mode = Mode::Normal;
                        self.delete_tab(&tab_id)?;
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        self.message = Some("Tab deletion cancelled".into());
                    }
                    _ => {}
                }
                Ok(None)
            }
            Mode::TabAssign(_) => self.handle_key_tab_assign(key),
            Mode::TabMenu(_) => self.handle_key_tab_menu(key),
        }
    }

    fn handle_key_normal(&mut self, key: KeyEvent, ui: &UiMeta) -> Result<Option<HomeExit>> {
        self.reset_numeric_input_if_stale();

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(Some(HomeExit::Quit)),
            KeyCode::Enter => {
                return self.begin_run_selected();
            }
            KeyCode::Char('e') => {
                if let Some(id) = self.current_id() {
                    return Ok(Some(HomeExit::Edit(id)));
                }
            }
            KeyCode::Char(ch @ '0'..='9') => {
                self.handle_numeric_selection(ch, ui.visible_rows);
            }
            KeyCode::Char('n') => {
                self.start_new_dialog()?;
                return Ok(None);
            }
            KeyCode::Char('c') => {
                self.clone_selected()?;
            }
            KeyCode::Char('d') => {
                if let Some(idx) = self.table_state.selected() {
                    self.mode = Mode::ConfirmDelete { idx };
                }
            }
            KeyCode::Char('a') => {
                self.start_tab_assign()?;
            }
            KeyCode::Char('t') => {
                self.start_tab_menu()?;
            }
            KeyCode::Left => self.move_tab(-1)?,
            KeyCode::Right => self.move_tab(1)?,
            KeyCode::Tab => self.move_tab(1)?,
            KeyCode::BackTab => self.move_tab(-1)?,
            KeyCode::Up => self.move_selection(-1, ui.visible_rows),
            KeyCode::Down => self.move_selection(1, ui.visible_rows),
            KeyCode::Char('k') => self.move_selection(-1, ui.visible_rows),
            KeyCode::Char('j') => self.move_selection(1, ui.visible_rows),
            KeyCode::Char('K') => self.move_workset(-1, ui.visible_rows)?,
            KeyCode::Char('J') => self.move_workset(1, ui.visible_rows)?,
            KeyCode::PageUp => self.move_selection(-(ui.visible_rows as isize), ui.visible_rows),
            KeyCode::PageDown => self.move_selection(ui.visible_rows as isize, ui.visible_rows),
            KeyCode::Home => self.select_index(0, ui.visible_rows),
            KeyCode::End => {
                if self.current_tab_len() > 0 {
                    self.select_index(self.current_tab_len() - 1, ui.visible_rows);
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_new_dialog(&mut self, key: KeyEvent) -> Result<Option<HomeExit>> {
        let tabs_snapshot = self.cfg.tabs.clone();
        let Mode::NewDialog(ref mut form) = self.mode else {
            return Ok(None);
        };

        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.hide_cursor()?;
                self.message = Some("Creation cancelled".into());
            }
            KeyCode::Tab => {
                form.focus = match form.focus {
                    DialogField::Name => DialogField::Desc,
                    DialogField::Desc => DialogField::Id,
                    DialogField::Id => DialogField::Tab,
                    DialogField::Tab => DialogField::Name,
                };
            }
            KeyCode::BackTab => {
                form.focus = match form.focus {
                    DialogField::Name => DialogField::Tab,
                    DialogField::Desc => DialogField::Name,
                    DialogField::Id => DialogField::Desc,
                    DialogField::Tab => DialogField::Id,
                };
            }
            KeyCode::Backspace => match form.focus {
                DialogField::Id => {
                    form.id.pop();
                }
                DialogField::Name => {
                    form.name.pop();
                }
                DialogField::Desc => {
                    form.desc.pop();
                }
                DialogField::Tab => {}
            },
            KeyCode::Enter => {
                let form_clone = form.clone();
                if self.create_new(&form_clone)? {
                    self.mode = Mode::Normal;
                    self.hide_cursor()?;
                }
            }
            KeyCode::Char(ch) => match form.focus {
                DialogField::Id => form.id.push(ch),
                DialogField::Name => form.name.push(ch),
                DialogField::Desc => form.desc.push(ch),
                DialogField::Tab => {
                    if matches!(ch, ' ' | 'l' | 'L' | 'h' | 'H') {
                        let delta = if matches!(ch, 'h' | 'H') { -1 } else { 1 };
                        cycle_new_dialog_tab(form, &tabs_snapshot, delta);
                    }
                }
            },
            KeyCode::Left => {
                if form.focus == DialogField::Tab {
                    cycle_new_dialog_tab(form, &tabs_snapshot, -1);
                }
            }
            KeyCode::Right => {
                if form.focus == DialogField::Tab {
                    cycle_new_dialog_tab(form, &tabs_snapshot, 1);
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_tab_new(&mut self, key: KeyEvent) -> Result<Option<HomeExit>> {
        let Mode::TabNew(ref mut form) = self.mode else {
            return Ok(None);
        };
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.hide_cursor()?;
                self.message = Some("Tab creation cancelled".into());
            }
            KeyCode::Tab => {
                form.focus = match form.focus {
                    TabFormField::Id => TabFormField::Label,
                    TabFormField::Label => TabFormField::IncludeUnassigned,
                    TabFormField::IncludeUnassigned => TabFormField::Id,
                };
            }
            KeyCode::BackTab => {
                form.focus = match form.focus {
                    TabFormField::Id => TabFormField::IncludeUnassigned,
                    TabFormField::Label => TabFormField::Id,
                    TabFormField::IncludeUnassigned => TabFormField::Label,
                };
            }
            KeyCode::Backspace => match form.focus {
                TabFormField::Id => {
                    form.id.pop();
                }
                TabFormField::Label => {
                    form.label.pop();
                }
                TabFormField::IncludeUnassigned => {}
            },
            KeyCode::Enter => {
                let form_clone = form.clone();
                if self.create_tab(&form_clone)? {
                    self.mode = Mode::Normal;
                    self.hide_cursor()?;
                }
            }
            KeyCode::Char(ch) => match form.focus {
                TabFormField::Id => form.id.push(ch),
                TabFormField::Label => form.label.push(ch),
                TabFormField::IncludeUnassigned => {
                    if ch == ' ' {
                        form.include_unassigned = !form.include_unassigned;
                    }
                }
            },
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_tab_rename(&mut self, key: KeyEvent) -> Result<Option<HomeExit>> {
        let Mode::TabRename(ref mut form) = self.mode else {
            return Ok(None);
        };
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.hide_cursor()?;
                self.message = Some("Tab rename cancelled".into());
            }
            KeyCode::Backspace => {
                form.label.pop();
            }
            KeyCode::Enter => {
                let form_clone = form.clone();
                if self.rename_tab(&form_clone)? {
                    self.mode = Mode::Normal;
                    self.hide_cursor()?;
                }
            }
            KeyCode::Char(ch) => form.label.push(ch),
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_tab_assign(&mut self, key: KeyEvent) -> Result<Option<HomeExit>> {
        let Mode::TabAssign(ref mut state) = self.mode else {
            return Ok(None);
        };
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.message = Some("Tab assignment cancelled".into());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.cursor + 1 < state.memberships.len() {
                    state.cursor += 1;
                }
            }
            KeyCode::Char(' ') | KeyCode::Char('x') => {
                let already_member = state
                    .memberships
                    .get(state.cursor)
                    .map(|row| row.member)
                    .unwrap_or(false);
                if already_member {
                    if let Some(row) = state.memberships.get_mut(state.cursor) {
                        row.member = false;
                    }
                } else {
                    for entry in state.memberships.iter_mut() {
                        entry.member = false;
                    }
                    if let Some(row) = state.memberships.get_mut(state.cursor) {
                        row.member = true;
                    }
                }
            }
            KeyCode::Enter => {
                let state_clone = state.clone();
                self.apply_tab_assignment(&state_clone)?;
                self.mode = Mode::Normal;
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_tab_menu(&mut self, key: KeyEvent) -> Result<Option<HomeExit>> {
        let Mode::TabMenu(ref mut state) = self.mode else {
            return Ok(None);
        };
        let items = [
            TabMenuItem::Add,
            TabMenuItem::Rename,
            TabMenuItem::Delete,
            TabMenuItem::MoveLeft,
            TabMenuItem::MoveRight,
            TabMenuItem::Exit,
        ];
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.message = Some("Tab menu closed".into());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.cursor + 1 < items.len() {
                    state.cursor += 1;
                }
            }
            KeyCode::Enter => match items.get(state.cursor).copied() {
                Some(TabMenuItem::Add) => {
                    self.mode = Mode::Normal;
                    self.start_tab_new()?;
                }
                Some(TabMenuItem::Rename) => {
                    self.mode = Mode::Normal;
                    self.start_tab_rename()?;
                }
                Some(TabMenuItem::Delete) => {
                    self.mode = Mode::Normal;
                    self.start_tab_delete()?;
                }
                Some(TabMenuItem::MoveLeft) => {
                    self.move_active_tab(-1)?;
                }
                Some(TabMenuItem::MoveRight) => {
                    self.move_active_tab(1)?;
                }
                Some(TabMenuItem::Exit) | None => {
                    self.mode = Mode::Normal;
                }
            },
            _ => {}
        }
        Ok(None)
    }

    pub(super) fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        ui: &UiMeta,
    ) -> Result<Option<HomeExit>> {
        if !matches!(self.mode, Mode::Normal) {
            return Ok(None);
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.move_selection(-1, ui.visible_rows);
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(1, ui.visible_rows);
            }
            MouseEventKind::Moved => {
                let tab_hit = self.hit_tab(mouse.column, mouse.row, ui);
                self.hover_tab = tab_hit;

                if let Some(idx) = self.row_from_y(mouse.row, ui) {
                    self.select_index(idx, ui.visible_rows);
                }
                let hit = self.hit_toolbar(mouse.column, mouse.row, ui);
                if hit != self.hover_toolbar {
                    self.hover_toolbar = hit;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(tab_idx) = self.hit_tab(mouse.column, mouse.row, ui) {
                    self.switch_tab(tab_idx)?;
                } else if let Some(idx) = self.row_from_y(mouse.row, ui) {
                    let double = self.is_double_click(idx);
                    self.select_index(idx, ui.visible_rows);
                    if double {
                        return self.begin_run_selected();
                    }
                    self.last_click = Some(LastClick {
                        idx,
                        at: Instant::now(),
                    });
                } else if let Some(action) = self.hit_toolbar(mouse.column, mouse.row, ui) {
                    self.hover_toolbar = Some(action);
                    return self.trigger_toolbar(action, ui.visible_rows);
                } else {
                    self.hover_toolbar = None;
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn trigger_toolbar(
        &mut self,
        action: ToolbarAction,
        visible_rows: usize,
    ) -> Result<Option<HomeExit>> {
        match action {
            ToolbarAction::Run => {
                return self.begin_run_selected();
            }
            ToolbarAction::Edit => {
                if let Some(id) = self.current_id() {
                    return Ok(Some(HomeExit::Edit(id)));
                }
            }
            ToolbarAction::New => {
                self.start_new_dialog()?;
            }
            ToolbarAction::Clone => {
                self.clone_selected()?;
            }
            ToolbarAction::MoveUp => self.move_workset(-1, visible_rows)?,
            ToolbarAction::MoveDown => self.move_workset(1, visible_rows)?,
            ToolbarAction::Delete => {
                if let Some(idx) = self.table_state.selected() {
                    self.mode = Mode::ConfirmDelete { idx };
                }
            }
            ToolbarAction::TabAssign => {
                self.start_tab_assign()?;
            }
            ToolbarAction::TabMenu => {
                self.start_tab_menu()?;
            }
            ToolbarAction::Quit => return Ok(Some(HomeExit::Quit)),
        }
        self.ensure_offset(visible_rows);
        Ok(None)
    }
}
