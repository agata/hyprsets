use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::{AppConfig, LayoutNode, SplitDirection, SplitNode, WindowSlot, Workset};

const RATIO_MIN: f32 = 0.3;
const RATIO_MAX: f32 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone)]
struct LeafHit {
    rect: Rect,
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
    boundary: Rect,
    area: Rect,
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
    Desc,
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
    desc: String,
    focus: DialogField,
    cursor_id: usize,
    cursor_name: usize,
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
}

impl EditorApp {
    fn new(workset: Workset, config_path: PathBuf) -> Self {
        let root = ensure_layout(workset.clone());
        let selected_path = first_leaf_path(&root).unwrap_or_default();
        let saved_id = workset.id.clone();
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

    fn draw(&mut self, f: &mut Frame) -> UiMeta {
        let area = f.size();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(8),
                Constraint::Length(2),
                Constraint::Length(2),
            ])
            .split(area);

        let mut header_spans = vec![Span::styled(
            format!("HyprSets — Editing: {}", self.workset.name),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        if !self.workset.desc.trim().is_empty() {
            header_spans.push(Span::raw("  ("));
            header_spans.push(Span::raw(&self.workset.desc));
            header_spans.push(Span::raw(")"));
        }
        let header = Paragraph::new(Line::from(header_spans));
        f.render_widget(header, chunks[0]);

        let mut parent_split = None;
        let mut leaf_hits = Vec::new();
        let mut split_hits = Vec::new();
        self.render_layout(
            f,
            chunks[1],
            &self.root,
            &mut Vec::new(),
            &mut leaf_hits,
            &mut split_hits,
            &mut parent_split,
            &self.selected_path,
        );
        self.render_status(f, chunks[2], parent_split);
        let toolbar_hits = self.render_toolbar(f, chunks[3], self.hover_toolbar);

        match &self.mode {
            Mode::EditCommand { buffer, cursor } => {
                self.render_cmd_dialog(f, area, buffer, *cursor)
            }
            Mode::EditWorkset { form } => self.render_workset_dialog(f, area, form),
            Mode::ConfirmDelete { target: _, slot_id } => {
                self.render_confirm_delete(f, area, *slot_id)
            }
            Mode::Normal => {}
        }

        UiMeta {
            leaf_hits,
            toolbar_hits,
            split_hits,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_layout(
        &self,
        f: &mut Frame,
        area: Rect,
        node: &LayoutNode,
        path: &mut Vec<Side>,
        hits: &mut Vec<LeafHit>,
        split_hits: &mut Vec<SplitHit>,
        parent_split: &mut Option<(SplitDirection, f32)>,
        selected_path: &[Side],
    ) {
        match node {
            LayoutNode::Leaf(slot) => {
                let is_selected = *path == self.selected_path;
                let title = format!("#{}  {}", slot.slot_id, slot.command);
                let block = Block::default().borders(Borders::ALL).title(Span::styled(
                    title,
                    if is_selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ));
                let inner = block.inner(area);
                f.render_widget(block, area);
                let text = Paragraph::new(" ").alignment(Alignment::Center);
                f.render_widget(text, inner);
                hits.push(LeafHit {
                    rect: area,
                    path: path.clone(),
                });
            }
            LayoutNode::Split(split) => {
                let (first_area, second_area) = split_area(area, split.direction, split.ratio);
                let boundary = match split.direction {
                    SplitDirection::Horizontal => {
                        let x = area.x.saturating_add(first_area.width.min(area.width));
                        let width = if area.width > 1 && x < area.x + area.width {
                            1
                        } else {
                            0
                        };
                        Rect {
                            x,
                            y: area.y,
                            width,
                            height: area.height,
                        }
                    }
                    SplitDirection::Vertical => {
                        let y = area.y.saturating_add(first_area.height.min(area.height));
                        let height = if area.height > 1 && y < area.y + area.height {
                            1
                        } else {
                            0
                        };
                        Rect {
                            x: area.x,
                            y,
                            width: area.width,
                            height,
                        }
                    }
                };

                let mut split_path = path.clone();
                split_path.push(Side::Left);
                let is_hover = self.hover_split.as_ref() == Some(&split_path);
                let is_active = self.active_drag.as_ref().map(|d| &d.path) == Some(&split_path);

                if boundary.width > 0 && boundary.height > 0 {
                    split_hits.push(SplitHit {
                        boundary,
                        area,
                        path: split_path.clone(),
                        direction: split.direction,
                    });
                }

                path.push(Side::Left);
                if selected_path.starts_with(path) {
                    *parent_split = Some((split.direction, split.ratio));
                }
                self.render_layout(
                    f,
                    first_area,
                    &split.left,
                    path,
                    hits,
                    split_hits,
                    parent_split,
                    selected_path,
                );
                path.pop();
                path.push(Side::Right);
                if selected_path.starts_with(path) {
                    *parent_split = Some((split.direction, split.ratio));
                }
                self.render_layout(
                    f,
                    second_area,
                    &split.right,
                    path,
                    hits,
                    split_hits,
                    parent_split,
                    selected_path,
                );
                path.pop();

                if (is_hover || is_active) && boundary.width > 0 && boundary.height > 0 {
                    render_split_highlight(f, boundary, split.direction, Color::Yellow);
                }
            }
        }
    }

    fn render_status(
        &self,
        f: &mut Frame,
        area: Rect,
        parent_split: Option<(SplitDirection, f32)>,
    ) {
        let mode_label = match self.mode {
            Mode::Normal => "Normal",
            Mode::EditCommand { .. } => "Cmd",
            Mode::EditWorkset { .. } => "Workset",
            Mode::ConfirmDelete { .. } => "Delete?",
        };
        let split_info = if let Some((dir, ratio)) = parent_split {
            let dir_str = match dir {
                SplitDirection::Horizontal => "horizontal",
                SplitDirection::Vertical => "vertical",
            };
            format!("split: {dir_str}, ratio = {:.2}", ratio)
        } else {
            "split: <root>".into()
        };
        let msg = self.message.as_deref().unwrap_or("");
        let line = Line::from(vec![
            Span::styled(
                format!(" [{mode_label}] "),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(split_info),
            Span::raw("  "),
            Span::styled("[Autosave]", Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::raw(msg),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn render_toolbar(
        &self,
        f: &mut Frame,
        area: Rect,
        hover: Option<ToolbarAction>,
    ) -> Vec<ButtonHit> {
        let mut spans: Vec<Span> = Vec::new();
        let mut hits = Vec::new();
        let mut cursor_x = area.x + 1;

        let buttons = [
            (ToolbarAction::Next, "[Tab Next]"),
            (ToolbarAction::SplitH, "[h Split ←→]"),
            (ToolbarAction::SplitV, "[v Split ↑↓]"),
            (ToolbarAction::RatioUp, "[+ Ratio+]"),
            (ToolbarAction::RatioDown, "[- Ratio-]"),
            (ToolbarAction::Delete, "[D Delete]"),
            (ToolbarAction::EditCmd, "[↵ Edit Cmd]"),
            (ToolbarAction::Swap, "[w Swap]"),
            (ToolbarAction::EditWorkset, "[e Title/Desc]"),
            (ToolbarAction::Cancel, "[Esc Close]"),
        ];

        for (action, label) in buttons {
            spans.push(Span::raw(" "));
            cursor_x += 1;
            let width = UnicodeWidthStr::width(label) as u16;
            let start = cursor_x;
            let end = cursor_x + width.saturating_sub(1);
            let style = if Some(action) == hover {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default()
            };
            spans.push(Span::styled(label, style));
            hits.push(ButtonHit {
                x_start: start,
                x_end: end,
                y: area.y,
                action,
            });
            cursor_x += width;
        }

        let line = Line::from(spans);
        f.render_widget(Paragraph::new(line), area);
        hits
    }

    fn render_cmd_dialog(&self, f: &mut Frame, area: Rect, buffer: &str, cursor: usize) {
        let popup = centered_rect(80, 30, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title("Edit Command")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let inner = block.inner(popup);

        f.render_widget(block, popup);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(3)])
            .split(inner);

        let hint = Paragraph::new(Line::from(format!(
            "Enter: Save  Esc: Cancel  (editing #{})",
            self.current_slot_id().unwrap_or(0)
        )));
        f.render_widget(hint, chunks[0]);

        let input_block = Block::default().borders(Borders::ALL).title("Command");
        let input_area = input_block.inner(chunks[1]);
        let (wrapped_lines, cursor_col, cursor_line) =
            wrap_input_text(buffer, cursor, input_area.width);
        let scroll = if input_area.height > 0 {
            cursor_line.saturating_sub(input_area.height.saturating_sub(1))
        } else {
            0
        };

        let cmd_text = Paragraph::new(wrapped_lines.join("\n")).scroll((scroll, 0));
        f.render_widget(input_block, chunks[1]);
        f.render_widget(cmd_text, input_area);

        if input_area.width > 0 && input_area.height > 0 {
            let cursor_x = input_area.x + cursor_col;
            let cursor_y = input_area
                .y
                .saturating_add(cursor_line.saturating_sub(scroll));
            f.set_cursor(cursor_x, cursor_y);
        }
    }

    fn render_workset_dialog(&self, f: &mut Frame, area: Rect, form: &WorksetForm) {
        let popup = centered_rect(80, 50, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title("Edit Workset Info")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let inner = block.inner(popup);

        let id_line = self.field_line("ID", &form.id, form.focus == DialogField::Id);
        let name_line = self.field_line("Name", &form.name, form.focus == DialogField::Name);
        let desc_line = self.field_line("Desc", &form.desc, form.focus == DialogField::Desc);

        let lines = vec![
            Line::from(id_line),
            Line::from(name_line),
            Line::from(desc_line),
            Line::from(""),
            Line::from("Enter: Save  Tab/Shift+Tab: Switch field  Esc: Cancel"),
        ];

        f.render_widget(block, popup);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);

        let (cursor_x, cursor_y) = match form.focus {
            DialogField::Id => {
                let prefix_w = UnicodeWidthStr::width("ID: ") as u16;
                let w = width_up_to(&form.id, form.cursor_id);
                (inner.x + prefix_w + w, inner.y)
            }
            DialogField::Name => {
                let prefix_w = UnicodeWidthStr::width("Name: ") as u16;
                let w = width_up_to(&form.name, form.cursor_name);
                (inner.x + prefix_w + w, inner.y + 1)
            }
            DialogField::Desc => {
                let prefix_w = UnicodeWidthStr::width("Desc: ") as u16;
                let w = width_up_to(&form.desc, form.cursor_desc);
                (inner.x + prefix_w + w, inner.y + 2)
            }
        };
        f.set_cursor(cursor_x, cursor_y);
    }

    fn render_confirm_delete(&self, f: &mut Frame, area: Rect, slot_id: Option<u32>) {
        let popup = centered_rect(50, 30, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title("Delete slot?")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let inner = block.inner(popup);
        let target = slot_id
            .map(|id| format!("#{}", id))
            .unwrap_or_else(|| "selected".into());
        let lines = vec![
            Line::from(format!("Delete slot {target}?")),
            Line::from("This cannot be undone after saving."),
            Line::from(""),
            Line::from("[y Yes]   [n No]"),
        ];
        f.render_widget(block, popup);
        f.render_widget(Paragraph::new(Text::from(lines)), inner);
    }

    fn field_line(&self, label: &str, val: &str, focused: bool) -> Span<'static> {
        if focused {
            Span::styled(
                format!("{label}: {val}"),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )
        } else {
            Span::raw(format!("{label}: {val}"))
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<EditorExit>> {
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
            KeyCode::Char('h') => self.split_selected(SplitDirection::Horizontal),
            KeyCode::Char('v') => self.split_selected(SplitDirection::Vertical),
            KeyCode::Char('s') => self.split_selected(SplitDirection::Vertical),
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

    fn exit_saved(&mut self) -> Result<Option<EditorExit>> {
        // 保存は各アクションのオートセーブで行うため、ここでは追加保存しない
        Ok(Some(EditorExit::Saved(self.workset.clone())))
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
        form: &mut WorksetForm,
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
                self.workset.desc = form.desc.trim().to_string();
                self.mark_changed();
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => match form.focus {
                DialogField::Id => {
                    if form.cursor_id > 0
                        && let Some(prev) = prev_grapheme_start(&form.id, form.cursor_id)
                    {
                        form.id.drain(prev..form.cursor_id);
                        form.cursor_id = prev;
                    }
                }
                DialogField::Name => {
                    if form.cursor_name > 0
                        && let Some(prev) = prev_grapheme_start(&form.name, form.cursor_name)
                    {
                        form.name.drain(prev..form.cursor_name);
                        form.cursor_name = prev;
                    }
                }
                DialogField::Desc => {
                    if form.cursor_desc > 0
                        && let Some(prev) = prev_grapheme_start(&form.desc, form.cursor_desc)
                    {
                        form.desc.drain(prev..form.cursor_desc);
                        form.cursor_desc = prev;
                    }
                }
            },
            KeyCode::Char(ch) => match form.focus {
                DialogField::Id => {
                    form.id.insert(form.cursor_id, ch);
                    form.cursor_id += ch.len_utf8();
                }
                DialogField::Name => {
                    form.name.insert(form.cursor_name, ch);
                    form.cursor_name += ch.len_utf8();
                }
                DialogField::Desc => {
                    form.desc.insert(form.cursor_desc, ch);
                    form.cursor_desc += ch.len_utf8();
                }
            },
            KeyCode::Left => match form.focus {
                DialogField::Id => {
                    if let Some(prev) = prev_grapheme_start(&form.id, form.cursor_id) {
                        form.cursor_id = prev;
                    }
                }
                DialogField::Name => {
                    if let Some(prev) = prev_grapheme_start(&form.name, form.cursor_name) {
                        form.cursor_name = prev;
                    }
                }
                DialogField::Desc => {
                    if let Some(prev) = prev_grapheme_start(&form.desc, form.cursor_desc) {
                        form.cursor_desc = prev;
                    }
                }
            },
            KeyCode::Right => match form.focus {
                DialogField::Id => {
                    if let Some(next) = next_grapheme_end(&form.id, form.cursor_id) {
                        form.cursor_id = next;
                    }
                }
                DialogField::Name => {
                    if let Some(next) = next_grapheme_end(&form.name, form.cursor_name) {
                        form.cursor_name = next;
                    }
                }
                DialogField::Desc => {
                    if let Some(next) = next_grapheme_end(&form.desc, form.cursor_desc) {
                        form.cursor_desc = next;
                    }
                }
            },
            KeyCode::Home => match form.focus {
                DialogField::Id => form.cursor_id = 0,
                DialogField::Name => form.cursor_name = 0,
                DialogField::Desc => form.cursor_desc = 0,
            },
            KeyCode::End => match form.focus {
                DialogField::Id => form.cursor_id = form.id.len(),
                DialogField::Name => form.cursor_name = form.name.len(),
                DialogField::Desc => form.cursor_desc = form.desc.len(),
            },
            _ => {}
        }
        Ok(None)
    }

    fn validate_workset_id(&self, candidate: &str) -> Result<()> {
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

    fn sync_workset_cursor(&self, form: &mut WorksetForm) {
        match form.focus {
            DialogField::Id => form.cursor_id = form.id.len(),
            DialogField::Name => form.cursor_name = form.name.len(),
            DialogField::Desc => form.cursor_desc = form.desc.len(),
        }
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

    fn handle_mouse(&mut self, mouse: MouseEvent, ui: &UiMeta) -> Result<Option<EditorExit>> {
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
                        self.split_selected(SplitDirection::Horizontal);
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

    fn start_drag(&mut self, hit: SplitHit) {
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
            ToolbarAction::SplitH => self.split_selected(SplitDirection::Horizontal),
            ToolbarAction::SplitV => self.split_selected(SplitDirection::Vertical),
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
            last == path && at.elapsed() <= Duration::from_millis(350)
        } else {
            false
        }
    }

    fn move_tab(&mut self, delta: isize) {
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

    fn split_selected(&mut self, direction: SplitDirection) {
        let next_id = next_slot_id(&self.root);
        let path = self.selected_path.clone();
        let replaced = replace_leaf_with_split(&mut self.root, &path, direction, next_id);
        if replaced {
            self.mark_changed();
        }
    }

    fn bump_ratio(&mut self, delta: f32) {
        if let Some((_, old, new)) = adjust_ratio(&mut self.root, &self.selected_path, delta)
            && (old - new).abs() > 0.0001
        {
            self.mark_changed();
        }
    }

    fn delete_at(&mut self, path: &[Side]) {
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

    fn current_leaf(&self) -> Option<&WindowSlot> {
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

    fn current_leaf_mut(&mut self) -> Option<&mut WindowSlot> {
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

    fn current_slot_id(&self) -> Option<u32> {
        self.current_leaf().map(|s| s.slot_id)
    }

    fn start_edit_command(&mut self) {
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

    fn start_edit_workset(&mut self) {
        self.mode = Mode::EditWorkset {
            form: WorksetForm {
                id: self.workset.id.clone(),
                name: self.workset.name.clone(),
                desc: self.workset.desc.clone(),
                focus: DialogField::Name,
                cursor_id: self.workset.id.len(),
                cursor_name: self.workset.name.len(),
                cursor_desc: self.workset.desc.len(),
            },
        };
    }

    fn confirm_delete_selected(&mut self) {
        let target = self.selected_path.clone();
        let slot_id = self.current_slot_id();
        self.mode = Mode::ConfirmDelete { target, slot_id };
    }

    fn swap_with_next(&mut self) {
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

    fn commit_workset(&mut self) {
        let mut commands = Vec::new();
        collect_commands(&self.root, &mut commands);
        self.workset.commands = commands;
        self.workset.layout = Some(self.root.clone());
    }

    fn save_current(&mut self) -> Result<()> {
        self.commit_workset();
        persist_workset(&self.workset, &self.saved_id, &self.config_path)?;
        self.saved_id = self.workset.id.clone();
        Ok(())
    }

    fn autosave(&mut self) {
        if let Err(err) = self.save_current() {
            self.message = Some(format!("Autosave failed: {err}"));
        }
    }

    fn mark_changed(&mut self) {
        self.autosave();
    }
}

fn ensure_layout(ws: Workset) -> LayoutNode {
    if let Some(layout) = ws.layout.clone() {
        layout
    } else {
        let cmd = ws.commands.first().cloned().unwrap_or_default();
        LayoutNode::Leaf(WindowSlot {
            slot_id: 1,
            command: cmd,
            cwd: None,
            env: Default::default(),
        })
    }
}

fn split_area(area: Rect, dir: SplitDirection, ratio: f32) -> (Rect, Rect) {
    match dir {
        SplitDirection::Horizontal => {
            let total = area.width as f32;
            let left_w = ((ratio / (ratio + 1.0)) * total).max(1.0) as u16;
            let right_w = area.width.saturating_sub(left_w);
            let left = Rect {
                x: area.x,
                y: area.y,
                width: left_w,
                height: area.height,
            };
            let right = Rect {
                x: area.x + left_w,
                y: area.y,
                width: right_w,
                height: area.height,
            };
            (left, right)
        }
        SplitDirection::Vertical => {
            let total = area.height as f32;
            let top_h = ((ratio / (ratio + 1.0)) * total).max(1.0) as u16;
            let bottom_h = area.height.saturating_sub(top_h);
            let top = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: top_h,
            };
            let bottom = Rect {
                x: area.x,
                y: area.y + top_h,
                width: area.width,
                height: bottom_h,
            };
            (top, bottom)
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ]
            .as_ref(),
        )
        .split(area);

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
        .split(vertical[1]);

    horizontal[1]
}

fn point_in_rect(x: u16, y: u16, rect: Rect) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

fn hit_toolbar(x: u16, y: u16, ui: &UiMeta) -> Option<ToolbarAction> {
    ui.toolbar_hits
        .iter()
        .find(|hit| hit.y == y && x >= hit.x_start && x <= hit.x_end)
        .map(|hit| hit.action)
}

fn hit_split(x: u16, y: u16, ui: &UiMeta) -> Option<SplitHit> {
    ui.split_hits
        .iter()
        .find(|hit| point_in_rect(x, y, hit.boundary))
        .cloned()
}

fn render_split_highlight(f: &mut Frame, rect: Rect, _direction: SplitDirection, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let block = Block::default().style(Style::default().bg(color));
    f.render_widget(block, rect);
}

fn next_field(focus: DialogField) -> DialogField {
    match focus {
        DialogField::Id => DialogField::Name,
        DialogField::Name => DialogField::Desc,
        DialogField::Desc => DialogField::Id,
    }
}

fn prev_field(focus: DialogField) -> DialogField {
    match focus {
        DialogField::Id => DialogField::Desc,
        DialogField::Name => DialogField::Id,
        DialogField::Desc => DialogField::Name,
    }
}

fn next_slot_id(root: &LayoutNode) -> u32 {
    let mut max_id = 0;
    collect_slot_ids(root, &mut max_id);
    max_id + 1
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

fn wrap_input_text(text: &str, cursor: usize, max_width: u16) -> (Vec<String>, u16, u16) {
    if max_width == 0 {
        return (vec![String::new()], 0, 0);
    }

    let target = cursor.min(text.len());
    let mut lines = vec![String::new()];
    let mut widths = vec![0u16];
    let mut cursor_col = 0;
    let mut cursor_line = 0;
    let mut cursor_set = target == 0;

    for (idx, ch) in text.char_indices() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        let current_width = *widths.last().unwrap_or(&0);
        let needs_wrap =
            current_width + ch_width > max_width && lines.last().is_some_and(|l| !l.is_empty());

        if needs_wrap {
            lines.push(String::new());
            widths.push(0);
        }

        let line = lines.last_mut().unwrap();
        line.push(ch);
        let width_ref = widths.last_mut().unwrap();
        *width_ref = width_ref.saturating_add(ch_width);

        if !cursor_set && target == idx + ch.len_utf8() {
            cursor_set = true;
            cursor_col = (*width_ref).min(max_width.saturating_sub(1));
            cursor_line = (lines.len() - 1) as u16;
        }
    }

    if !cursor_set {
        cursor_col = widths
            .last()
            .copied()
            .unwrap_or(0)
            .min(max_width.saturating_sub(1));
        cursor_line = (lines.len() - 1) as u16;
    }

    (lines, cursor_col, cursor_line)
}

fn width_up_to(text: &str, cursor: usize) -> u16 {
    let idx = cursor.min(text.len());
    UnicodeWidthStr::width(&text[..idx]) as u16
}

fn collect_slot_ids(node: &LayoutNode, max_id: &mut u32) {
    match node {
        LayoutNode::Leaf(slot) => {
            *max_id = (*max_id).max(slot.slot_id);
        }
        LayoutNode::Split(split) => {
            collect_slot_ids(&split.left, max_id);
            collect_slot_ids(&split.right, max_id);
        }
    }
}

fn first_leaf_path(node: &LayoutNode) -> Option<Vec<Side>> {
    match node {
        LayoutNode::Leaf(_) => Some(vec![]),
        LayoutNode::Split(split) => {
            let mut left_path = vec![Side::Left];
            if let Some(rest) = first_leaf_path(&split.left) {
                left_path.extend(rest);
                return Some(left_path);
            }
            let mut right_path = vec![Side::Right];
            if let Some(rest) = first_leaf_path(&split.right) {
                right_path.extend(rest);
                return Some(right_path);
            }
            None
        }
    }
}

fn replace_leaf_with_split(
    node: &mut LayoutNode,
    path: &[Side],
    direction: SplitDirection,
    new_slot_id: u32,
) -> bool {
    if path.is_empty() {
        if let LayoutNode::Leaf(existing) = node {
            let new_leaf = LayoutNode::Leaf(WindowSlot {
                slot_id: new_slot_id,
                command: String::new(),
                cwd: None,
                env: Default::default(),
            });
            let old_leaf = LayoutNode::Leaf(existing.clone());
            let split = SplitNode {
                direction,
                ratio: if matches!(direction, SplitDirection::Horizontal) {
                    1.2
                } else {
                    1.0
                },
                left: Box::new(old_leaf),
                right: Box::new(new_leaf),
            };
            *node = LayoutNode::Split(split);
            return true;
        }
        return false;
    }

    match node {
        LayoutNode::Leaf(_) => false,
        LayoutNode::Split(split) => {
            let (first, rest) = path.split_first().unwrap();
            let child = if matches!(first, Side::Left) {
                &mut split.left
            } else {
                &mut split.right
            };
            replace_leaf_with_split(child, rest, direction, new_slot_id)
        }
    }
}

fn remove_leaf(node: &mut LayoutNode, path: &[Side]) -> bool {
    if path.is_empty() {
        return false;
    }
    if let LayoutNode::Split(split) = node {
        let (first, rest) = path.split_first().unwrap();
        let target = if matches!(first, Side::Left) {
            &mut split.left
        } else {
            &mut split.right
        };

        if rest.is_empty() {
            let sibling = if matches!(first, Side::Left) {
                *split.right.clone()
            } else {
                *split.left.clone()
            };
            *node = sibling;
            return true;
        } else {
            return remove_leaf(target, rest);
        }
    }
    false
}

fn collect_commands(node: &LayoutNode, commands: &mut Vec<String>) {
    match node {
        LayoutNode::Leaf(slot) => commands.push(slot.command.clone()),
        LayoutNode::Split(split) => {
            collect_commands(&split.left, commands);
            collect_commands(&split.right, commands);
        }
    }
}

fn leaf_at_path<'a>(node: &'a LayoutNode, path: &[Side]) -> Option<&'a WindowSlot> {
    let mut cur = node;
    for side in path {
        match cur {
            LayoutNode::Split(split) => {
                cur = if matches!(side, Side::Left) {
                    &split.left
                } else {
                    &split.right
                };
            }
            LayoutNode::Leaf(_) => break,
        }
    }
    if let LayoutNode::Leaf(slot) = cur {
        Some(slot)
    } else {
        None
    }
}

fn set_leaf_at_path(node: &mut LayoutNode, path: &[Side], slot: WindowSlot) -> bool {
    if path.is_empty() {
        if let LayoutNode::Leaf(target) = node {
            *target = slot;
            return true;
        }
        return false;
    }

    match node {
        LayoutNode::Split(split) => {
            let (first, rest) = path.split_first().unwrap();
            let child = if matches!(first, Side::Left) {
                &mut split.left
            } else {
                &mut split.right
            };
            set_leaf_at_path(child, rest, slot)
        }
        LayoutNode::Leaf(_) => false,
    }
}

fn adjust_ratio(
    node: &mut LayoutNode,
    path: &[Side],
    delta: f32,
) -> Option<(SplitDirection, f32, f32)> {
    match node {
        LayoutNode::Split(split) => {
            let (first, rest) = path.split_first()?;
            if rest.is_empty() {
                let old = split.ratio;
                let new = clamp_ratio(old + delta);
                split.ratio = new;
                Some((split.direction, old, new))
            } else {
                let child = if matches!(first, Side::Left) {
                    &mut split.left
                } else {
                    &mut split.right
                };
                adjust_ratio(child, rest, delta)
            }
        }
        LayoutNode::Leaf(_) => None,
    }
}

fn set_ratio(
    node: &mut LayoutNode,
    path: &[Side],
    new_ratio: f32,
) -> Option<(SplitDirection, f32, f32)> {
    match node {
        LayoutNode::Split(split) => {
            let (first, rest) = path.split_first()?;
            if rest.is_empty() {
                let old = split.ratio;
                let new = clamp_ratio(new_ratio);
                split.ratio = new;
                Some((split.direction, old, new))
            } else {
                let child = if matches!(first, Side::Left) {
                    &mut split.left
                } else {
                    &mut split.right
                };
                set_ratio(child, rest, new_ratio)
            }
        }
        LayoutNode::Leaf(_) => None,
    }
}

fn ratio_from_position(area: Rect, direction: SplitDirection, x: u16, y: u16) -> Option<f32> {
    match direction {
        SplitDirection::Horizontal => {
            if area.width <= 1 {
                return None;
            }
            let pos = x
                .saturating_sub(area.x)
                .min(area.width.saturating_sub(1))
                .max(1);
            let left = pos as f32;
            let right = (area.width as f32 - left).max(1.0);
            Some(clamp_ratio(left / right))
        }
        SplitDirection::Vertical => {
            if area.height <= 1 {
                return None;
            }
            let pos = y
                .saturating_sub(area.y)
                .min(area.height.saturating_sub(1))
                .max(1);
            let top = pos as f32;
            let bottom = (area.height as f32 - top).max(1.0);
            Some(clamp_ratio(top / bottom))
        }
    }
}

fn clamp_ratio(val: f32) -> f32 {
    val.clamp(RATIO_MIN, RATIO_MAX)
}

fn persist_workset(ws: &Workset, saved_id: &str, config_path: &Path) -> Result<()> {
    let mut cfg = AppConfig::load_or_init(config_path)?;

    if cfg
        .worksets
        .iter()
        .any(|existing| existing.id == ws.id && existing.id != saved_id)
    {
        bail!("ID already exists");
    }

    if let Some(idx) = cfg.worksets.iter().position(|w| w.id == saved_id) {
        cfg.worksets[idx] = ws.clone();
    } else if let Some(idx) = cfg.worksets.iter().position(|w| w.id == ws.id) {
        cfg.worksets[idx] = ws.clone();
    } else {
        cfg.worksets.push(ws.clone());
    }

    cfg.save(config_path)
}
