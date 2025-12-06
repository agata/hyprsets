use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, widgets::TableState};

use crate::{
    config::AppConfig,
    state::{AppState, default_state_path},
};

mod actions;
mod actions_selection;
mod actions_tabs;
mod actions_worksets;
mod events;
mod render;
mod tabs;

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
        app.load_and_apply_tabs(initial_selected_id);
        app
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
