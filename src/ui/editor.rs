use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::Result;
use crossterm::{
    cursor::Show,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::config::{LayoutNode, SplitDirection, Workset};

mod actions;
mod events;
mod layout;
mod persistence;
mod render;

use self::{
    layout::{ensure_layout, first_leaf_path},
    persistence::load_tab_state,
};

const RATIO_MIN: f32 = 0.3;
const RATIO_MAX: f32 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone)]
struct LeafHit {
    rect: ratatui::layout::Rect,
    path: Vec<Side>,
}

#[derive(Debug, Clone)]
struct ButtonHit {
    x_start: u16,
    x_end: u16,
    y: u16,
    action: ToolbarAction,
}

#[derive(Debug, Clone)]
struct SplitHit {
    boundary: ratatui::layout::Rect,
    area: ratatui::layout::Rect,
    path: Vec<Side>,
    direction: SplitDirection,
}

#[derive(Debug, Clone)]
struct ActiveDrag {
    path: Vec<Side>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogField {
    Id,
    Name,
    Workspace,
    Desc,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Normal,
    EditCommand {
        buffer: String,
        cursor: usize,
    },
    EditWorkset {
        form: WorksetForm,
    },
    ConfirmDelete {
        target: Vec<Side>,
        slot_id: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolbarAction {
    Next,
    SplitH,
    SplitV,
    RatioUp,
    RatioDown,
    Delete,
    EditCmd,
    Swap,
    EditWorkset,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorksetForm {
    id: String,
    name: String,
    workspace: String,
    desc: String,
    selected_tab: Option<String>,
    focus: DialogField,
    cursor_id: usize,
    cursor_name: usize,
    cursor_workspace: usize,
    cursor_desc: usize,
}

#[derive(Debug)]
pub enum EditorExit {
    Saved(Workset),
}

#[derive(Debug, Default, Clone)]
struct UiMeta {
    leaf_hits: Vec<LeafHit>,
    toolbar_hits: Vec<ButtonHit>,
    split_hits: Vec<SplitHit>,
}

#[derive(Debug, Clone)]
struct TabOption {
    id: String,
    label: String,
}

pub fn run_editor(workset: Workset, config_path: &Path) -> Result<EditorExit> {
    let mut app = EditorApp::new(workset, config_path.to_path_buf());
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.show_cursor()?;

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;

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

struct EditorApp {
    workset: Workset,
    saved_id: String,
    root: LayoutNode,
    selected_path: Vec<Side>,
    message: Option<String>,
    mode: Mode,
    last_click: Option<(Vec<Side>, Instant)>,
    leaf_paths: Vec<Vec<Side>>,
    config_path: PathBuf,
    hover_toolbar: Option<ToolbarAction>,
    active_drag: Option<ActiveDrag>,
    hover_split: Option<Vec<Side>>,
    tab_options: Vec<TabOption>,
    selected_tab_id: Option<String>,
}

impl EditorApp {
    fn new(workset: Workset, config_path: PathBuf) -> Self {
        let root = ensure_layout(workset.clone());
        let selected_path = first_leaf_path(&root).unwrap_or_default();
        let saved_id = workset.id.clone();
        let (tab_options, selected_tab_id) = load_tab_state(&config_path, &workset.id);
        Self {
            workset,
            saved_id,
            root,
            selected_path,
            message: None,
            mode: Mode::Normal,
            last_click: None,
            leaf_paths: Vec::new(),
            config_path,
            hover_toolbar: None,
            active_drag: None,
            hover_split: None,
            tab_options,
            selected_tab_id,
        }
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<EditorExit> {
        loop {
            let mut ui_meta = UiMeta::default();
            terminal.draw(|f| {
                ui_meta = self.draw(f);
            })?;
            self.leaf_paths = ui_meta.leaf_hits.iter().map(|h| h.path.clone()).collect();
            if (self.selected_path.is_empty() || !self.leaf_paths.contains(&self.selected_path))
                && let Some(first) = self.leaf_paths.first()
            {
                self.selected_path = first.clone();
            }

            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Some(exit) = self.handle_key(key)? {
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
