use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use rand::{Rng, distributions::Alphanumeric};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    config::{AppConfig, Workset},
    run::workspace_cleanup_status,
};

#[derive(Debug, Clone)]
pub enum HomeExit {
    Quit,
    Run { id: String, preconfirm_clean: bool },
    Edit(String),
}

pub fn run_home(
    cfg: AppConfig,
    config_path: &Path,
    initial_selected_id: Option<String>,
) -> Result<HomeExit> {
    let mut app = HomeApp::new(cfg, config_path.to_path_buf(), initial_selected_id);
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, Hide)?;
    let result = app.event_loop(&mut terminal);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        Show
    )?;

    if app.dirty {
        app.save()?;
    }

    result
}

struct HomeApp {
    cfg: AppConfig,
    config_path: PathBuf,
    table_state: TableState,
    scroll: usize,
    mode: Mode,
    message: Option<String>,
    last_click: Option<LastClick>,
    dirty: bool,
    hover_toolbar: Option<ToolbarAction>,
}

#[derive(Debug, Clone)]
enum Mode {
    Normal,
    ConfirmDelete { idx: usize },
    ConfirmRun(ConfirmRunState),
    NewDialog(NewDialogState),
}

#[derive(Debug, Clone)]
struct NewDialogState {
    name: String,
    desc: String,
    focus: DialogField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogField {
    Name,
    Desc,
}

#[derive(Debug, Clone)]
struct ConfirmRunState {
    id: String,
    workset_name: String,
    workspace_name: String,
    window_count: usize,
}

#[derive(Debug, Clone)]
struct LastClick {
    idx: usize,
    at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarAction {
    Run,
    Edit,
    New,
    Clone,
    Delete,
    MoveUp,
    MoveDown,
    Quit,
}

#[derive(Default, Debug, Clone)]
struct UiMeta {
    data_start_y: u16,
    visible_rows: usize,
    visible_offset: usize,
    toolbar_hits: Vec<ButtonHit>,
}

#[derive(Debug, Clone)]
struct ButtonHit {
    x_start: u16,
    x_end: u16,
    y: u16,
    action: ToolbarAction,
}

impl HomeApp {
    fn new(cfg: AppConfig, config_path: PathBuf, initial_selected_id: Option<String>) -> Self {
        let mut table_state = TableState::default();
        if let Some(idx) = initial_selected_id
            .and_then(|id| cfg.worksets.iter().position(|w| w.id == id))
            .or_else(|| (!cfg.worksets.is_empty()).then_some(0))
        {
            table_state.select(Some(idx));
        }
        Self {
            cfg,
            config_path,
            table_state,
            scroll: 0,
            mode: Mode::Normal,
            message: None,
            last_click: None,
            dirty: false,
            hover_toolbar: None,
        }
    }

    fn save(&self) -> Result<()> {
        self.cfg
            .save(&self.config_path)
            .with_context(|| format!("failed to write config to {}", self.config_path.display()))
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<HomeExit> {
        loop {
            let mut ui_meta = UiMeta::default();
            terminal.draw(|f| {
                ui_meta = self.draw(f);
            })?;

            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Some(exit) = self.handle_key(key, &ui_meta)? {
                        return Ok(exit);
                    }
                }
                Event::Mouse(mouse) => {
                    if let Some(exit) = self.handle_mouse(mouse, &ui_meta)? {
                        return Ok(exit);
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    fn draw(&mut self, f: &mut Frame) -> UiMeta {
        let area = f.size();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        let list_area = chunks[0];
        let status_area = chunks[1];
        let toolbar_area = chunks[2];

        let visible_rows = list_area.height.saturating_sub(3) as usize; // border + header
        let mut ui_meta = UiMeta {
            visible_rows: visible_rows.max(1),
            visible_offset: self.scroll,
            data_start_y: list_area.y.saturating_add(2), // border + header
            ..UiMeta::default()
        };

        self.render_table(f, list_area, visible_rows);
        self.render_status(f, status_area);
        ui_meta.toolbar_hits = self.render_toolbar(f, toolbar_area, self.hover_toolbar);

        match &self.mode {
            Mode::ConfirmDelete { idx } => {
                self.render_delete_modal(f, area, *idx);
            }
            Mode::ConfirmRun(state) => {
                self.render_run_modal(f, area, state);
            }
            Mode::NewDialog(form) => {
                self.render_new_modal(f, area, form);
            }
            Mode::Normal => {}
        }

        ui_meta
    }

    fn render_table(&mut self, f: &mut Frame, area: Rect, visible_rows: usize) {
        let total = self.cfg.worksets.len();
        if total == 0 {
            let block = Block::default()
                .borders(Borders::ALL)
                .title("HyprSets — Worksets");
            f.render_widget(block.clone(), area);
            let inner = block.inner(area);
            let msg = Paragraph::new("No worksets. Press 'n' to create.")
                .alignment(ratatui::layout::Alignment::Center);
            f.render_widget(msg, inner);
            return;
        }

        if self.table_state.selected().is_none() {
            self.table_state.select(Some(0));
        }

        self.ensure_offset(visible_rows);

        let start = self.scroll;

        let rows: Vec<Row> = self
            .cfg
            .worksets
            .iter()
            .enumerate()
            .skip(start)
            .take(visible_rows)
            .map(|(idx, ws)| {
                Row::new(vec![
                    Cell::from(format!("{:>3}", idx + 1)),
                    Cell::from(ws.name.clone()),
                    Cell::from(ws.desc.clone()),
                ])
            })
            .collect();

        let selected_str = self
            .table_state
            .selected()
            .map(|i| format!("{}/{}", i + 1, total))
            .unwrap_or_else(|| format!("0/{}", total));

        let header = Row::new(vec!["No", "Name", "Description"]).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

        let widths = [
            Constraint::Length(5),
            Constraint::Length(22),
            Constraint::Min(10),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("HyprSets — Worksets   {}", selected_str)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ")
            .column_spacing(1);

        *self.table_state.offset_mut() = self.scroll;
        f.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let pos = if let Some(selected) = self.table_state.selected() {
            format!("{}/{}", selected + 1, self.cfg.worksets.len())
        } else {
            format!("0/{}", self.cfg.worksets.len())
        };

        let msg = self.message.as_deref().unwrap_or("");
        let msg_width = UnicodeWidthStr::width(msg);
        let pos_width = UnicodeWidthStr::width(pos.as_str());
        let total_width = area.width as usize;
        let spacing = total_width.saturating_sub(msg_width + pos_width + 1);
        let line = format!("{msg}{pad}{pos}", pad = " ".repeat(spacing));
        let paragraph = Paragraph::new(Line::from(line));
        f.render_widget(paragraph, area);
    }

    fn render_toolbar(
        &self,
        f: &mut Frame,
        area: Rect,
        hover: Option<ToolbarAction>,
    ) -> Vec<ButtonHit> {
        let buttons = [
            (ToolbarAction::Run, "[↵ Run]"),
            (ToolbarAction::Edit, "[e Edit]"),
            (ToolbarAction::New, "[n New]"),
            (ToolbarAction::Clone, "[c Copy]"),
            (ToolbarAction::MoveUp, "[Shift+K Move↑]"),
            (ToolbarAction::MoveDown, "[Shift+J Move↓]"),
            (ToolbarAction::Delete, "[d Delete]"),
            (ToolbarAction::Quit, "[q Quit]"),
        ];

        let mut spans: Vec<Span> = Vec::new();
        let mut hits = Vec::new();
        let mut cursor_x = area.x;
        for (idx, (action, label)) in buttons.iter().enumerate() {
            if idx == 0 {
                cursor_x = area.x + 1;
            } else {
                spans.push(Span::raw(" "));
                cursor_x += 1;
            }
            let width = UnicodeWidthStr::width(*label) as u16;
            let start = cursor_x;
            let end = cursor_x + width.saturating_sub(1);
            hits.push(ButtonHit {
                x_start: start,
                x_end: end,
                y: area.y,
                action: *action,
            });
            let style = if Some(*action) == hover {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(*label, style));
            cursor_x += width;
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line).block(Block::default());
        f.render_widget(paragraph, area);
        hits
    }

    fn render_delete_modal(&self, f: &mut Frame, area: Rect, idx: usize) {
        let Some(ws) = self.cfg.worksets.get(idx) else {
            return;
        };
        let block = Block::default()
            .title("Delete workset?")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let popup_area = centered_rect(60, 40, area);
        let inner = block.inner(popup_area);
        let lines = vec![
            Line::from(format!("Delete \"{}\" (id: {}) ?", ws.name, ws.id)),
            Line::from("Are you sure you want to delete this workset?"),
            Line::from(" "),
            Line::from("[y Yes]   [n No]"),
        ];
        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
    }

    fn render_new_modal(&self, f: &mut Frame, area: Rect, form: &NewDialogState) {
        let block = Block::default()
            .title("New Workset")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let popup_area = centered_rect(70, 50, area);
        let inner = block.inner(popup_area);

        let name_label = if form.focus == DialogField::Name {
            Span::styled(
                format!("Name: {}", form.name),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(format!("Name: {}", form.name))
        };

        let desc_label = if form.focus == DialogField::Desc {
            Span::styled(
                format!("Desc: {}", form.desc),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(format!("Desc: {}", form.desc))
        };

        let help = Line::from(vec![
            Span::raw("Enter: Create and return to list  "),
            Span::raw("Tab: Switch field  "),
            Span::raw("Esc: Cancel"),
        ]);

        let lines = vec![
            Line::from(name_label),
            Line::from(desc_label),
            Line::from(""),
            help,
        ];

        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
    }

    fn render_run_modal(&self, f: &mut Frame, area: Rect, state: &ConfirmRunState) {
        let block = Block::default()
            .title("Close existing windows?")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let popup_area = centered_rect(65, 40, area);
        let inner = block.inner(popup_area);
        let lines = vec![
            Line::from(format!("Workset: {}", state.workset_name)),
            Line::from(format!(
                "Workspace '{}' has {} window(s).",
                state.workspace_name, state.window_count
            )),
            Line::from("Close all of them before launching this workset?"),
            Line::from(" "),
            Line::from("[y Yes]   [n No]"),
        ];
        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
    }

    fn handle_key(&mut self, key: KeyEvent, ui: &UiMeta) -> Result<Option<HomeExit>> {
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
        }
    }

    fn handle_key_normal(&mut self, key: KeyEvent, ui: &UiMeta) -> Result<Option<HomeExit>> {
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
            KeyCode::Char('n') => {
                self.mode = Mode::NewDialog(NewDialogState::default());
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
            KeyCode::Up => self.move_selection(-1, ui.visible_rows),
            KeyCode::Down => self.move_selection(1, ui.visible_rows),
            KeyCode::Char('k') => self.move_selection(-1, ui.visible_rows),
            KeyCode::Char('j') => self.move_selection(1, ui.visible_rows),
            KeyCode::Char('K') => self.move_workset(-1, ui.visible_rows),
            KeyCode::Char('J') => self.move_workset(1, ui.visible_rows),
            KeyCode::PageUp => self.move_selection(-(ui.visible_rows as isize), ui.visible_rows),
            KeyCode::PageDown => self.move_selection(ui.visible_rows as isize, ui.visible_rows),
            KeyCode::Home => self.select_index(0, ui.visible_rows),
            KeyCode::End => {
                if !self.cfg.worksets.is_empty() {
                    self.select_index(self.cfg.worksets.len() - 1, ui.visible_rows);
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_key_new_dialog(&mut self, key: KeyEvent) -> Result<Option<HomeExit>> {
        let Mode::NewDialog(ref mut form) = self.mode else {
            return Ok(None);
        };

        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.message = Some("Creation cancelled".into());
            }
            KeyCode::Tab => {
                form.focus = match form.focus {
                    DialogField::Name => DialogField::Desc,
                    DialogField::Desc => DialogField::Name,
                };
            }
            KeyCode::Backspace => match form.focus {
                DialogField::Name => {
                    form.name.pop();
                }
                DialogField::Desc => {
                    form.desc.pop();
                }
            },
            KeyCode::Enter => {
                let form_clone = form.clone();
                self.create_new(&form_clone)?;
                self.mode = Mode::Normal;
            }
            KeyCode::Char(ch) => match form.focus {
                DialogField::Name => form.name.push(ch),
                DialogField::Desc => form.desc.push(ch),
            },
            _ => {}
        }
        Ok(None)
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, ui: &UiMeta) -> Result<Option<HomeExit>> {
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
                if let Some(idx) = self.row_from_y(mouse.row, ui) {
                    self.select_index(idx, ui.visible_rows);
                }
                let hit = self.hit_toolbar(mouse.column, mouse.row, ui);
                if hit != self.hover_toolbar {
                    self.hover_toolbar = hit;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(idx) = self.row_from_y(mouse.row, ui) {
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
                self.mode = Mode::NewDialog(NewDialogState::default());
            }
            ToolbarAction::Clone => {
                self.clone_selected()?;
            }
            ToolbarAction::MoveUp => self.move_workset(-1, visible_rows),
            ToolbarAction::MoveDown => self.move_workset(1, visible_rows),
            ToolbarAction::Delete => {
                if let Some(idx) = self.table_state.selected() {
                    self.mode = Mode::ConfirmDelete { idx };
                }
            }
            ToolbarAction::Quit => return Ok(Some(HomeExit::Quit)),
        }
        self.ensure_offset(visible_rows);
        Ok(None)
    }

    fn row_from_y(&self, y: u16, ui: &UiMeta) -> Option<usize> {
        if y < ui.data_start_y {
            return None;
        }
        let rel = y - ui.data_start_y;
        if rel as usize >= ui.visible_rows {
            return None;
        }
        let idx = ui.visible_offset + rel as usize;
        if idx < self.cfg.worksets.len() {
            Some(idx)
        } else {
            None
        }
    }

    fn hit_toolbar(&self, x: u16, y: u16, ui: &UiMeta) -> Option<ToolbarAction> {
        ui.toolbar_hits
            .iter()
            .find(|hit| hit.y == y && x >= hit.x_start && x <= hit.x_end)
            .map(|hit| hit.action)
    }

    fn ensure_offset(&mut self, visible_rows: usize) {
        if let Some(selected) = self.table_state.selected() {
            if selected < self.scroll {
                self.scroll = selected;
            } else if selected >= self.scroll + visible_rows {
                self.scroll = selected + 1 - visible_rows;
            }
            *self.table_state.offset_mut() = self.scroll;
        } else if !self.cfg.worksets.is_empty() {
            self.table_state.select(Some(0));
            self.scroll = 0;
            *self.table_state.offset_mut() = 0;
        }
    }

    fn move_selection(&mut self, delta: isize, visible_rows: usize) {
        if self.cfg.worksets.is_empty() {
            return;
        }
        let len = self.cfg.worksets.len() as isize;
        let current = self.table_state.selected().map(|i| i as isize).unwrap_or(0);

        let next = if delta == 1 && current == len - 1 {
            0
        } else if delta == -1 && current == 0 {
            len - 1
        } else {
            let mut next = current + delta;
            if next < 0 {
                next = 0;
            } else if next >= len {
                next = len - 1;
            }
            next
        };

        self.select_index(next as usize, visible_rows);
    }

    fn select_index(&mut self, idx: usize, visible_rows: usize) {
        if self.cfg.worksets.is_empty() {
            self.table_state.select(None);
            return;
        }
        let bounded = idx.min(self.cfg.worksets.len() - 1);
        self.table_state.select(Some(bounded));
        self.ensure_offset(visible_rows);
    }

    fn is_double_click(&self, idx: usize) -> bool {
        if let Some(last) = &self.last_click {
            last.idx == idx && last.at.elapsed() <= Duration::from_millis(350)
        } else {
            false
        }
    }

    fn current_workset_info(&self) -> Option<(String, String)> {
        self.table_state
            .selected()
            .and_then(|idx| self.cfg.worksets.get(idx))
            .map(|ws| (ws.id.clone(), ws.name.clone()))
    }

    fn begin_run_selected(&mut self) -> Result<Option<HomeExit>> {
        let Some((id, name)) = self.current_workset_info() else {
            return Ok(None);
        };
        self.begin_run(id, name)
    }

    fn begin_run(&mut self, id: String, name: String) -> Result<Option<HomeExit>> {
        let status = workspace_cleanup_status()?;
        if status.closable_windows == 0 {
            return Ok(Some(HomeExit::Run {
                id,
                preconfirm_clean: false,
            }));
        }

        self.mode = Mode::ConfirmRun(ConfirmRunState {
            id,
            workset_name: name,
            workspace_name: status.workspace_name,
            window_count: status.closable_windows,
        });
        Ok(None)
    }

    fn current_id(&self) -> Option<String> {
        self.current_workset_info().map(|(id, _)| id)
    }

    fn delete_at(&mut self, idx: usize) -> Result<()> {
        if idx >= self.cfg.worksets.len() {
            return Ok(());
        }
        let removed = self.cfg.worksets.remove(idx);
        self.dirty = true;
        self.message = Some(format!("Deleted: {}", removed.name));

        if self.cfg.worksets.is_empty() {
            self.table_state.select(None);
            self.scroll = 0;
        } else if idx >= self.cfg.worksets.len() {
            self.table_state.select(Some(self.cfg.worksets.len() - 1));
            self.scroll = self.scroll.saturating_sub(1);
        }
        Ok(())
    }

    fn clone_selected(&mut self) -> Result<()> {
        let Some(sel) = self.table_state.selected() else {
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
        self.dirty = true;
        self.message = Some(format!("Duplicated: {}", new_id));
        let last = self.cfg.worksets.len() - 1;
        self.table_state.select(Some(last));
        Ok(())
    }

    fn move_workset(&mut self, delta: isize, visible_rows: usize) {
        let Some(sel) = self.table_state.selected() else {
            return;
        };
        let len = self.cfg.worksets.len();
        if len <= 1 {
            return;
        }

        let target = sel as isize + delta;
        if target < 0 || target >= len as isize {
            self.message = Some("Cannot move further".into());
            return;
        }

        self.cfg.worksets.swap(sel, target as usize);
        self.table_state.select(Some(target as usize));
        self.dirty = true;
        self.ensure_offset(visible_rows);
        let direction = if delta < 0 { "up" } else { "down" };
        self.message = Some(format!("Moved {}", direction));
    }

    fn create_new(&mut self, form: &NewDialogState) -> Result<()> {
        if form.name.trim().is_empty() {
            self.message = Some("Please enter a name".into());
            return Ok(());
        }
        let id = self.generate_id();
        let ws = Workset {
            id: id.clone(),
            name: form.name.trim().to_string(),
            desc: form.desc.trim().to_string(),
            commands: vec![],
            cwd: None,
            env: HashMap::new(),
            layout: None,
        };
        self.cfg.worksets.push(ws);
        self.dirty = true;
        self.message = Some(format!("Added: {}", id));
        let last = self.cfg.worksets.len() - 1;
        self.table_state.select(Some(last));
        Ok(())
    }

    fn generate_id(&self) -> String {
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
}

impl Default for NewDialogState {
    fn default() -> Self {
        Self {
            name: String::new(),
            desc: String::new(),
            focus: DialogField::Name,
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(r);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1]);

    horizontal[1]
}
