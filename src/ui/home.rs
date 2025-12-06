use std::{
    collections::{HashMap, HashSet},
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
    state::{AppState, default_state_path},
};

const ALL_TAB_ID: &str = "all";

#[derive(Debug, Clone)]
pub enum HomeExit {
    Quit,
    Run { id: String, preconfirm_clean: bool },
    Edit(String),
}

#[derive(Debug, Clone)]
struct ResolvedTab {
    id: String,
    label: String,
    indices: Vec<usize>,
}

#[derive(Debug, Clone)]
struct TabHit {
    x_start: u16,
    x_end: u16,
    y: u16,
    idx: usize,
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

    result
}

struct HomeApp {
    cfg: AppConfig,
    config_path: PathBuf,
    state_path: PathBuf,
    state: AppState,
    table_state: TableState,
    scroll: usize,
    mode: Mode,
    message: Option<String>,
    last_click: Option<LastClick>,
    hover_toolbar: Option<ToolbarAction>,
    hover_tab: Option<usize>,
    numeric_input: Option<NumericInput>,
    tabs: Vec<ResolvedTab>,
    active_tab_idx: usize,
    tab_selected_ids: HashMap<String, String>,
    workset_tab_labels: HashMap<String, String>,
}

#[derive(Debug, Clone)]
enum Mode {
    Normal,
    ConfirmDelete { idx: usize },
    ConfirmRun(ConfirmRunState),
    NewDialog(NewDialogState),
    TabNew(TabForm),
    TabRename(TabRenameState),
    ConfirmTabDelete { tab_id: String, tab_label: String },
    TabAssign(TabAssignState),
    TabMenu(TabMenuState),
}

#[derive(Debug, Clone)]
struct NewDialogState {
    id: String,
    name: String,
    desc: String,
    selected_tab: Option<String>,
    focus: DialogField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogField {
    Name,
    Desc,
    Id,
    Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabFormField {
    Id,
    Label,
    IncludeUnassigned,
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
    TabAssign,
    TabMenu,
}

#[derive(Default, Debug, Clone)]
struct UiMeta {
    data_start_y: u16,
    visible_rows: usize,
    visible_offset: usize,
    toolbar_hits: Vec<ButtonHit>,
    tab_hits: Vec<TabHit>,
}

#[derive(Debug, Clone)]
struct ButtonHit {
    x_start: u16,
    x_end: u16,
    y: u16,
    action: ToolbarAction,
}

#[derive(Debug, Clone)]
struct NumericInput {
    buffer: String,
    last_input: Instant,
}

#[derive(Debug, Clone)]
struct TabForm {
    id: String,
    label: String,
    include_unassigned: bool,
    focus: TabFormField,
}

#[derive(Debug, Clone)]
struct TabRenameState {
    tab_id: String,
    label: String,
}

#[derive(Debug, Clone)]
struct TabAssignState {
    workset_id: String,
    cursor: usize,
    memberships: Vec<TabAssignRow>,
}

#[derive(Debug, Clone)]
struct TabAssignRow {
    tab_id: String,
    label: String,
    member: bool,
}

#[derive(Debug, Clone)]
struct TabMenuState {
    cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabMenuItem {
    Add,
    Rename,
    Delete,
    MoveLeft,
    MoveRight,
    Exit,
}

const NUMERIC_INPUT_TIMEOUT_MS: u64 = 900;

impl HomeApp {
    fn new(cfg: AppConfig, config_path: PathBuf, initial_selected_id: Option<String>) -> Self {
        let mut app = Self {
            cfg,
            config_path,
            state_path: default_state_path(),
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
            tab_selected_ids: HashMap::new(),
            workset_tab_labels: HashMap::new(),
        };
        app.load_state();
        app.initialize_tabs(initial_selected_id);
        app
    }

    fn load_state(&mut self) {
        self.state = AppState::load(&self.state_path);
    }

    fn initialize_tabs(&mut self, initial_selected_id: Option<String>) {
        let preferred_tab = self.state.last_tab_id.clone();
        self.apply_tabs(preferred_tab, initial_selected_id);
    }

    fn apply_tabs(&mut self, preferred_tab: Option<String>, preferred_selection: Option<String>) {
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

    fn build_tabs(&self) -> (Vec<ResolvedTab>, Vec<String>, HashMap<String, String>) {
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

    fn resolve_tab_index(&self, preferred: Option<&str>) -> usize {
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

    fn apply_selection_for_active_tab(&mut self, preferred: Option<String>) {
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
                Constraint::Length(1),
                Constraint::Min(8),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        let tab_area = chunks[0];
        let list_area = chunks[1];
        let status_area = chunks[2];
        let toolbar_area = chunks[3];

        let visible_rows = list_area.height.saturating_sub(3) as usize; // border + header
        let mut ui_meta = UiMeta {
            visible_rows: visible_rows.max(1),
            visible_offset: self.scroll,
            data_start_y: list_area.y.saturating_add(2), // border + header
            ..UiMeta::default()
        };

        ui_meta.tab_hits = self.render_tabs(f, tab_area, self.hover_tab);
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
            Mode::TabNew(form) => {
                self.render_tab_new_modal(f, area, form);
            }
            Mode::TabRename(form) => {
                self.render_tab_rename_modal(f, area, form);
            }
            Mode::ConfirmTabDelete { tab_id, tab_label } => {
                self.render_tab_delete_modal(f, area, tab_id, tab_label);
            }
            Mode::TabAssign(state) => {
                self.render_tab_assign_modal(f, area, state);
            }
            Mode::TabMenu(state) => {
                self.render_tab_menu_modal(f, area, state);
            }
            Mode::Normal => {}
        }

        ui_meta
    }

    fn render_tabs(&self, f: &mut Frame, area: Rect, hover: Option<usize>) -> Vec<TabHit> {
        let mut spans: Vec<Span> = Vec::new();
        let mut hits = Vec::new();
        let mut cursor_x = area.x.saturating_add(1);

        for (idx, tab) in self.tabs.iter().enumerate() {
            if idx > 0 {
                spans.push(Span::raw(" "));
                cursor_x = cursor_x.saturating_add(1);
            }
            let label = format!("[{}]", tab.label);
            let width = UnicodeWidthStr::width(label.as_str()) as u16;
            let active = idx == self.active_tab_idx;
            let style = if Some(idx) == hover {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
            } else if active {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(label.clone(), style));
            hits.push(TabHit {
                x_start: cursor_x,
                x_end: cursor_x + width.saturating_sub(1),
                y: area.y,
                idx,
            });
            cursor_x = cursor_x.saturating_add(width);
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line);
        f.render_widget(paragraph, area);
        hits
    }

    fn render_table(&mut self, f: &mut Frame, area: Rect, visible_rows: usize) {
        let total = self.current_tab_len();
        if total == 0 {
            let block = Block::default()
                .borders(Borders::ALL)
                .title("HyprSets — Worksets");
            f.render_widget(block.clone(), area);
            let inner = block.inner(area);
            let msg = Paragraph::new("No worksets in this tab. Press 'n' to create.")
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
            .current_tab()
            .into_iter()
            .flat_map(|tab| {
                tab.indices
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(visible_rows)
                    .filter_map(|(local_idx, global_idx)| {
                        self.cfg.worksets.get(*global_idx).map(|ws| {
                            let workspace = ws
                                .workspace
                                .as_deref()
                                .map(str::trim)
                                .filter(|w| !w.is_empty())
                                .unwrap_or("")
                                .to_string();
                            let tab_label = self
                                .workset_tab_labels
                                .get(&ws.id)
                                .cloned()
                                .unwrap_or_default();
                            Row::new(vec![
                                Cell::from(format!("{:>3}", local_idx + 1)),
                                Cell::from(ws.name.clone()),
                                Cell::from(ws.desc.clone()),
                                Cell::from(tab_label),
                                Cell::from(workspace),
                            ])
                        })
                    })
            })
            .collect();

        let selected_str = self
            .table_state
            .selected()
            .map(|i| format!("{}/{}", i + 1, total))
            .unwrap_or_else(|| format!("0/{}", total));

        let header = Row::new(vec!["No", "Name", "Description", "Tab", "Workspace"]).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

        let widths = [
            Constraint::Length(5),
            Constraint::Length(20),
            Constraint::Min(10),
            Constraint::Length(14),
            Constraint::Length(18),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(format!(
                        "HyprSets — Worksets [{}]   {}",
                        self.current_tab()
                            .map(|t| t.label.as_str())
                            .unwrap_or("All"),
                        selected_str
                    )))
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
        let total = self.current_tab_len();
        let pos = if let Some(selected) = self.table_state.selected() {
            format!("{}/{}", selected + 1, total)
        } else {
            format!("0/{}", total)
        };

        let tab_label = self
            .current_tab()
            .map(|t| t.label.as_str())
            .unwrap_or("All");
        let msg_body = self.message.as_deref().unwrap_or("");
        let left = if msg_body.is_empty() {
            format!("[Tab: {tab_label}]")
        } else {
            format!("[Tab: {tab_label}] {msg_body}")
        };

        let msg_width = UnicodeWidthStr::width(left.as_str());
        let pos_width = UnicodeWidthStr::width(pos.as_str());
        let total_width = area.width as usize;
        let spacing = total_width.saturating_sub(msg_width + pos_width + 1);
        let line = format!("{left}{pad}{pos}", pad = " ".repeat(spacing));
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
            (ToolbarAction::MoveUp, "[Shift+K MoveUp]"),
            (ToolbarAction::MoveDown, "[Shift+J MoveDown]"),
            (ToolbarAction::Delete, "[d Delete]"),
            (ToolbarAction::TabAssign, "[a AssignTab]"),
            (ToolbarAction::TabMenu, "[t TabMenu]"),
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
        let Some(ws) = self.workset_at_tab_index(idx) else {
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

        let id_label = if form.focus == DialogField::Id {
            Span::styled(
                format!("ID: {}", form.id),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(format!("ID: {}", form.id))
        };

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

        let tab_value = if self.cfg.tabs.is_empty() {
            "(no tabs)".to_string()
        } else {
            form.selected_tab
                .as_ref()
                .and_then(|id| {
                    self.cfg
                        .tabs
                        .iter()
                        .find(|t| t.id == *id)
                        .map(|t| t.label.clone())
                })
                .unwrap_or_else(|| "<none>".to_string())
        };

        let tab_label = if form.focus == DialogField::Tab {
            Span::styled(
                format!("Tab: {}", tab_value),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(format!("Tab: {}", tab_value))
        };

        let help = Line::from(vec![
            Span::raw("Enter: Create and return to list  "),
            Span::raw("Tab/Shift+Tab: Switch field  "),
            Span::raw("Left/Right/Space: Change tab  "),
            Span::raw("Esc: Cancel"),
        ]);

        let lines = vec![
            Line::from(id_label),
            Line::from(name_label),
            Line::from(desc_label),
            Line::from(tab_label),
            Line::from(""),
            help,
        ];

        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);

        let (cursor_x, cursor_y) = match form.focus {
            DialogField::Id => {
                let prefix_w = UnicodeWidthStr::width("ID: ") as u16;
                let text_w = UnicodeWidthStr::width(form.id.as_str()) as u16;
                (inner.x + prefix_w + text_w, inner.y)
            }
            DialogField::Name => {
                let prefix_w = UnicodeWidthStr::width("Name: ") as u16;
                let text_w = UnicodeWidthStr::width(form.name.as_str()) as u16;
                (inner.x + prefix_w + text_w, inner.y + 1)
            }
            DialogField::Desc => {
                let prefix_w = UnicodeWidthStr::width("Desc: ") as u16;
                let text_w = UnicodeWidthStr::width(form.desc.as_str()) as u16;
                (inner.x + prefix_w + text_w, inner.y + 2)
            }
            DialogField::Tab => {
                let prefix_w = UnicodeWidthStr::width("Tab: ") as u16;
                let text_w = UnicodeWidthStr::width(tab_value.as_str()) as u16;
                (inner.x + prefix_w + text_w, inner.y + 3)
            }
        };
        f.set_cursor(cursor_x, cursor_y);
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

    fn render_tab_new_modal(&self, f: &mut Frame, area: Rect, form: &TabForm) {
        let block = Block::default()
            .title("New Tab")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let popup_area = centered_rect(70, 50, area);
        let inner = block.inner(popup_area);

        let id_label = if form.focus == TabFormField::Id {
            Span::styled(
                format!("ID: {}", form.id),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(format!("ID: {}", form.id))
        };

        let label_label = if form.focus == TabFormField::Label {
            Span::styled(
                format!("Label: {}", form.label),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(format!("Label: {}", form.label))
        };

        let include_label = if form.focus == TabFormField::IncludeUnassigned {
            Span::styled(
                format!(
                    "Include unassigned: {}",
                    if form.include_unassigned { "yes" } else { "no" }
                ),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(format!(
                "Include unassigned: {}",
                if form.include_unassigned { "yes" } else { "no" }
            ))
        };

        let help = Line::from(vec![
            Span::raw("Enter: Create  "),
            Span::raw("Tab/Shift+Tab: Switch field  "),
            Span::raw("Space: Toggle include  "),
            Span::raw("Esc: Cancel"),
        ]);

        let lines = vec![
            Line::from(id_label),
            Line::from(label_label),
            Line::from(include_label),
            Line::from(""),
            help,
        ];

        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);

        let (cursor_x, cursor_y) = match form.focus {
            TabFormField::Id => {
                let prefix_w = UnicodeWidthStr::width("ID: ") as u16;
                let text_w = UnicodeWidthStr::width(form.id.as_str()) as u16;
                (inner.x + prefix_w + text_w, inner.y)
            }
            TabFormField::Label => {
                let prefix_w = UnicodeWidthStr::width("Label: ") as u16;
                let text_w = UnicodeWidthStr::width(form.label.as_str()) as u16;
                (inner.x + prefix_w + text_w, inner.y + 1)
            }
            TabFormField::IncludeUnassigned => {
                let prefix_w = UnicodeWidthStr::width("Include unassigned: ") as u16;
                let text_w =
                    UnicodeWidthStr::width(if form.include_unassigned { "yes" } else { "no" })
                        as u16;
                (inner.x + prefix_w + text_w, inner.y + 2)
            }
        };
        f.set_cursor(cursor_x, cursor_y);
    }

    fn render_tab_rename_modal(&self, f: &mut Frame, area: Rect, form: &TabRenameState) {
        let block = Block::default()
            .title("Rename Tab")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let popup_area = centered_rect(60, 40, area);
        let inner = block.inner(popup_area);
        let lines = vec![
            Line::from(format!("ID: {}", form.tab_id)),
            Line::from(format!("Label: {}", form.label)),
            Line::from(""),
            Line::from("Enter: Save   Esc: Cancel"),
        ];
        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);

        let prefix_w = UnicodeWidthStr::width("Label: ") as u16;
        let text_w = UnicodeWidthStr::width(form.label.as_str()) as u16;
        f.set_cursor(inner.x + prefix_w + text_w, inner.y + 1);
    }

    fn render_tab_delete_modal(&self, f: &mut Frame, area: Rect, tab_id: &str, tab_label: &str) {
        let block = Block::default()
            .title("Delete tab?")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let popup_area = centered_rect(55, 35, area);
        let inner = block.inner(popup_area);
        let lines = vec![
            Line::from(format!("Delete tab '{tab_label}' (id: {tab_id})?")),
            Line::from("Are you sure you want to remove this tab?"),
            Line::from(" "),
            Line::from("[y Yes]   [n No]"),
        ];
        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
    }

    fn render_tab_assign_modal(&self, f: &mut Frame, area: Rect, state: &TabAssignState) {
        let block = Block::default()
            .title("Assign to tabs")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let popup_area = centered_rect(65, 60, area);
        let inner = block.inner(popup_area);

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(format!("Workset: {}", state.workset_id)));
        lines.push(Line::from("Space: toggle  Enter: save  Esc: cancel"));
        lines.push(Line::from(""));

        for (idx, row) in state.memberships.iter().enumerate() {
            let marker = if row.member { "[x]" } else { "[ ]" };
            let text = format!("{marker} {}", row.label);
            let style = if idx == state.cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::styled(text, style));
        }

        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
    }

    fn render_tab_menu_modal(&self, f: &mut Frame, area: Rect, state: &TabMenuState) {
        let block = Block::default()
            .title("Tab menu")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let popup_area = centered_rect(50, 45, area);
        let inner = block.inner(popup_area);

        let items = [
            (TabMenuItem::Add, "Add tab"),
            (TabMenuItem::Rename, "Rename tab"),
            (TabMenuItem::Delete, "Delete tab"),
            (TabMenuItem::MoveLeft, "Move tab left"),
            (TabMenuItem::MoveRight, "Move tab right"),
            (TabMenuItem::Exit, "Close menu"),
        ];

        let mut lines = Vec::new();
        lines.push(Line::from(
            "Use ↑/↓ or j/k to select, Enter to confirm, Esc to close",
        ));
        lines.push(Line::from(""));
        for (idx, (_, label)) in items.iter().enumerate() {
            let prefix = if idx == state.cursor { "> " } else { "  " };
            let style = if idx == state.cursor {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::styled(format!("{prefix}{label}"), style));
        }

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

    fn start_new_dialog(&mut self) -> Result<()> {
        execute!(io::stdout(), Show)?;
        let state = self.new_dialog_state();
        self.mode = Mode::NewDialog(state);
        Ok(())
    }

    fn new_dialog_state(&self) -> NewDialogState {
        NewDialogState {
            id: self.generate_id(),
            name: String::new(),
            desc: String::new(),
            selected_tab: self.default_new_tab(),
            focus: DialogField::Name,
        }
    }

    fn default_new_tab(&self) -> Option<String> {
        self.current_tab_id()
            .filter(|id| *id != ALL_TAB_ID)
            .map(str::to_string)
    }

    fn start_tab_new(&mut self) -> Result<()> {
        execute!(io::stdout(), Show)?;
        let form = TabForm {
            id: self.generate_tab_id(),
            label: String::new(),
            include_unassigned: false,
            focus: TabFormField::Label,
        };
        self.mode = Mode::TabNew(form);
        Ok(())
    }

    fn start_tab_rename(&mut self) -> Result<()> {
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
        self.mode = Mode::TabRename(form);
        Ok(())
    }

    fn start_tab_delete(&mut self) -> Result<()> {
        if let Some(tab_id) = self.current_tab_id().filter(|id| *id != ALL_TAB_ID) {
            let tab_label = self
                .cfg
                .tabs
                .iter()
                .find(|t| t.id == tab_id)
                .map(|t| t.label.clone())
                .unwrap_or_else(|| tab_id.to_string());
            self.mode = Mode::ConfirmTabDelete {
                tab_id: tab_id.to_string(),
                tab_label,
            };
        } else {
            self.message = Some("All tab cannot be deleted".into());
        }
        Ok(())
    }

    fn start_tab_assign(&mut self) -> Result<()> {
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

        self.mode = Mode::TabAssign(TabAssignState {
            workset_id: ws.id,
            cursor,
            memberships,
        });
        Ok(())
    }

    fn start_tab_menu(&mut self) -> Result<()> {
        self.mode = Mode::TabMenu(TabMenuState { cursor: 0 });
        Ok(())
    }

    fn hide_cursor(&self) -> Result<()> {
        execute!(io::stdout(), Hide)?;
        Ok(())
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
        if idx < self.current_tab_len() {
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

    fn hit_tab(&self, x: u16, y: u16, ui: &UiMeta) -> Option<usize> {
        ui.tab_hits
            .iter()
            .find(|hit| hit.y == y && x >= hit.x_start && x <= hit.x_end)
            .map(|hit| hit.idx)
    }

    fn ensure_offset(&mut self, visible_rows: usize) {
        if let Some(selected) = self.table_state.selected() {
            if selected < self.scroll {
                self.scroll = selected;
            } else if selected >= self.scroll + visible_rows {
                self.scroll = selected + 1 - visible_rows;
            }
            *self.table_state.offset_mut() = self.scroll;
        } else if self.current_tab_len() > 0 {
            self.table_state.select(Some(0));
            self.scroll = 0;
            *self.table_state.offset_mut() = 0;
        }
    }

    fn move_selection(&mut self, delta: isize, visible_rows: usize) {
        let len = self.current_tab_len();
        if len == 0 {
            return;
        }
        let len_isize = len as isize;
        let current = self.table_state.selected().map(|i| i as isize).unwrap_or(0);

        let next = if delta == 1 && current == len_isize - 1 {
            0
        } else if delta == -1 && current == 0 {
            len_isize - 1
        } else {
            let mut next = current + delta;
            if next < 0 {
                next = 0;
            } else if next >= len_isize {
                next = len_isize - 1;
            }
            next
        };

        self.select_index(next as usize, visible_rows);
    }

    fn move_tab(&mut self, delta: isize) -> Result<()> {
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

    fn switch_tab(&mut self, idx: usize) -> Result<()> {
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

    fn persist_last_tab(&mut self) {
        if let Some(id) = self.current_tab_id() {
            self.state.last_tab_id = Some(id.to_string());
            if let Err(err) = self.state.save(&self.state_path) {
                self.message = Some(format!("failed to save tab state: {err}"));
            }
        }
    }

    fn select_index(&mut self, idx: usize, visible_rows: usize) {
        let len = self.current_tab_len();
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let bounded = idx.min(len - 1);
        self.table_state.select(Some(bounded));
        self.ensure_offset(visible_rows);
        self.record_current_selection();
    }

    fn is_double_click(&self, idx: usize) -> bool {
        if let Some(last) = &self.last_click {
            last.idx == idx && last.at.elapsed() <= Duration::from_millis(350)
        } else {
            false
        }
    }

    fn begin_run_selected(&mut self) -> Result<Option<HomeExit>> {
        let Some(ws) = self.current_workset() else {
            return Ok(None);
        };
        self.begin_run(ws)
    }

    fn begin_run(&mut self, ws: Workset) -> Result<Option<HomeExit>> {
        let status = workspace_cleanup_status(Some(&ws))?;
        let id = ws.id.clone();
        let name = ws.name.clone();
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

    fn handle_numeric_selection(&mut self, ch: char, visible_rows: usize) {
        let timeout = Duration::from_millis(NUMERIC_INPUT_TIMEOUT_MS);
        let input = self.numeric_input.get_or_insert_with(|| NumericInput {
            buffer: String::new(),
            last_input: Instant::now(),
        });

        if input.last_input.elapsed() > timeout {
            input.buffer.clear();
        }

        input.buffer.push(ch);
        input.last_input = Instant::now();

        let value = if input.buffer == "0" {
            10
        } else {
            match input.buffer.parse::<usize>() {
                Ok(num) if num > 0 => num,
                _ => return,
            }
        };

        let idx = value - 1;
        if idx < self.current_tab_len() {
            self.select_index(idx, visible_rows);
        } else {
            self.message = Some(format!("Workset {} not available in this tab", value));
        }
    }

    fn reset_numeric_input_if_stale(&mut self) {
        if let Some(input) = self.numeric_input.as_ref()
            && input.last_input.elapsed() > Duration::from_millis(NUMERIC_INPUT_TIMEOUT_MS)
        {
            self.numeric_input = None;
        }
    }

    fn delete_at(&mut self, idx: usize) -> Result<()> {
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

    fn clone_selected(&mut self) -> Result<()> {
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

    fn move_workset(&mut self, delta: isize, _visible_rows: usize) -> Result<()> {
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

    fn reorder_in_tab(&mut self, from_idx: usize, to_idx: usize, tab: &ResolvedTab) -> Result<()> {
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

    fn create_new(&mut self, form: &NewDialogState) -> Result<bool> {
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

    fn generate_tab_id(&self) -> String {
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

    fn current_tab(&self) -> Option<&ResolvedTab> {
        self.tabs.get(self.active_tab_idx)
    }

    fn current_tab_id(&self) -> Option<&str> {
        self.current_tab().map(|t| t.id.as_str())
    }

    fn tab_index_by_id(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn current_tab_len(&self) -> usize {
        self.current_tab().map(|t| t.indices.len()).unwrap_or(0)
    }

    fn index_in_current_tab(&self, workset_id: &str) -> Option<usize> {
        let tab = self.current_tab()?;
        tab.indices.iter().position(|idx| {
            self.cfg
                .worksets
                .get(*idx)
                .map(|ws| ws.id.as_str() == workset_id)
                .unwrap_or(false)
        })
    }

    fn workset_at_tab_index(&self, tab_idx: usize) -> Option<&Workset> {
        let tab = self.current_tab()?;
        let global_idx = *tab.indices.get(tab_idx)?;
        self.cfg.worksets.get(global_idx)
    }

    fn selected_global_index(&self) -> Option<usize> {
        let tab = self.current_tab()?;
        let selected = self.table_state.selected()?;
        tab.indices.get(selected).copied()
    }

    fn current_workset(&self) -> Option<Workset> {
        self.selected_global_index()
            .and_then(|idx| self.cfg.worksets.get(idx))
            .cloned()
    }

    fn current_id(&self) -> Option<String> {
        self.current_workset().map(|ws| ws.id)
    }

    fn record_current_selection(&mut self) {
        let tab_id = self.current_tab_id().map(|t| t.to_string());
        if let (Some(tab_id), Some(id)) = (tab_id.clone(), self.current_id()) {
            self.tab_selected_ids.insert(tab_id, id);
        } else if let Some(tab_id) = tab_id {
            self.tab_selected_ids.remove(&tab_id);
        }
    }

    fn is_all_tab_active(&self) -> bool {
        matches!(self.current_tab_id(), Some(id) if id == ALL_TAB_ID)
    }

    fn has_user_tabs(&self) -> bool {
        (self.cfg.version >= 2 || !self.cfg.tabs.is_empty()) && !self.cfg.tabs.is_empty()
    }

    fn ensure_tab_version(&mut self) {
        if self.cfg.version < 2 {
            self.cfg.version = 2;
        }
    }

    fn assign_workset_to_tab(&mut self, workset_id: &str, tab_id: Option<&str>) -> bool {
        if self.cfg.tabs.is_empty() {
            return true;
        }
        self.ensure_tab_version();
        for tab in self.cfg.tabs.iter_mut() {
            tab.worksets.retain(|id| id != workset_id);
        }
        if let Some(target) = tab_id {
            if let Some(tab) = self.cfg.tabs.iter_mut().find(|t| t.id == target) {
                tab.worksets.push(workset_id.to_string());
            } else {
                self.message = Some("Tab not found".into());
                return false;
            }
        }
        true
    }

    fn remove_workset_from_tabs(&mut self, workset_id: &str) {
        if self.cfg.tabs.is_empty() {
            return;
        }
        for tab in self.cfg.tabs.iter_mut() {
            tab.worksets.retain(|id| id != workset_id);
        }
    }

    fn create_tab(&mut self, form: &TabForm) -> Result<bool> {
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

    fn rename_tab(&mut self, form: &TabRenameState) -> Result<bool> {
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

    fn delete_tab(&mut self, tab_id: &str) -> Result<()> {
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

    fn current_user_tab_index(&self) -> Option<usize> {
        let id = self.current_tab_id()?;
        if id == ALL_TAB_ID {
            None
        } else {
            self.cfg.tabs.iter().position(|t| t.id == id)
        }
    }

    fn move_active_tab(&mut self, delta: isize) -> Result<()> {
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

    fn apply_tab_assignment(&mut self, state: &TabAssignState) -> Result<()> {
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
}

impl Default for NewDialogState {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            desc: String::new(),
            selected_tab: None,
            focus: DialogField::Name,
        }
    }
}

fn cycle_new_dialog_tab(
    form: &mut NewDialogState,
    tabs: &[crate::config::TabConfig],
    delta: isize,
) {
    if tabs.is_empty() {
        form.selected_tab = None;
        return;
    }
    let options_len = tabs.len() + 1; // +1 for unassigned
    let current_idx = form
        .selected_tab
        .as_ref()
        .and_then(|id| tabs.iter().position(|t| t.id == *id))
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
        tabs.get(next - 1).map(|t| t.id.clone())
    };
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
