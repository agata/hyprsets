use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table, Tabs},
};
use unicode_width::UnicodeWidthStr;

use super::{
    ButtonHit, ConfirmRunState, DialogField, HomeApp, Mode, NewDialogState, TabAssignState,
    TabForm, TabFormField, TabHit, TabHitKind, TabMenuItem, TabMenuState, TabRenameState,
    ToolbarAction, UiMeta,
};

impl HomeApp {
    pub(super) fn draw(&mut self, f: &mut Frame) -> UiMeta {
        let area = f.size();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        let tab_area = chunks[0];
        let list_area = chunks[1];
        let status_area = chunks[2];
        let toolbar_area = chunks[3];

        let visible_rows = list_area.height.saturating_sub(2) as usize; // header + bottom border
        let mut ui_meta = UiMeta {
            visible_rows: visible_rows.max(1),
            visible_offset: self.scroll,
            data_start_y: list_area.y.saturating_add(1), // header row
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

    fn render_tabs(&self, f: &mut Frame, area: Rect, hover: Option<TabHitKind>) -> Vec<TabHit> {
        let tab_block = Block::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .padding(Padding::horizontal(1));
        let inner = tab_block.inner(area);
        let divider = "│";
        let divider_width = UnicodeWidthStr::width(divider) as u16;
        let padding_width = 2; // default Tabs padding: 1 space on each side

        let mut hits = Vec::new();
        let mut cursor_x = inner.x;
        let y_start = inner.y;
        let y_end = inner.y;

        let mut titles: Vec<Line> = Vec::new();
        for (idx, tab) in self.tabs.iter().enumerate() {
            if idx > 0 {
                cursor_x = cursor_x.saturating_add(divider_width);
            }

            let style = if matches!(hover, Some(TabHitKind::Tab(h)) if h == idx) {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let label = format!(" {} ", tab.label);
            let width = UnicodeWidthStr::width(label.as_str()) as u16 + padding_width;
            hits.push(TabHit {
                x_start: cursor_x,
                x_end: cursor_x + width.saturating_sub(1),
                y_start,
                y_end,
                kind: TabHitKind::Tab(idx),
            });
            cursor_x = cursor_x.saturating_add(width);

            titles.push(Line::styled(label, style));
        }

        if !self.tabs.is_empty() {
            cursor_x = cursor_x.saturating_add(divider_width);
        }
        let add_style = if matches!(hover, Some(TabHitKind::AddButton)) {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let add_label = " + Add";
        let add_width = UnicodeWidthStr::width(add_label) as u16 + padding_width;
        hits.push(TabHit {
            x_start: cursor_x,
            x_end: cursor_x + add_width.saturating_sub(1),
            y_start,
            y_end,
            kind: TabHitKind::AddButton,
        });
        titles.push(Line::styled(add_label, add_style));

        let tabs = Tabs::new(titles)
            .block(tab_block)
            .divider(Span::raw(divider))
            .select(self.active_tab_idx)
            .style(Style::default())
            .highlight_style(
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            );

        f.render_widget(tabs, area);
        if area.width > 0 && area.height > 0 {
            let bottom_y = area.y + area.height - 1;
            let mut bottom_chars = vec!['─'; area.width as usize];
            if let Some(first) = bottom_chars.first_mut() {
                *first = '│';
            }
            if let Some(last) = bottom_chars.last_mut() {
                *last = '│';
            }
            let bottom_line: String = bottom_chars.into_iter().collect();
            let bottom = Paragraph::new(Span::raw(bottom_line));
            let bottom_area = Rect {
                x: area.x,
                y: bottom_y,
                width: area.width,
                height: 1,
            };
            f.render_widget(bottom, bottom_area);
        }
        hits
    }

    fn render_table(&mut self, f: &mut Frame, area: Rect, visible_rows: usize) {
        let total = self.current_tab_len();
        if total == 0 {
            let block = Block::default().borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM);
            f.render_widget(block.clone(), area);
            let inner = block.inner(area);
            let msg = Paragraph::new("No worksets in this tab. Press 'n' to create.")
                .alignment(Alignment::Center);
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
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM))
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
        let msg_body = self.message.as_deref().unwrap_or("").to_string();
        let left = if msg_body.is_empty() {
            format!("Tab: {tab_label}")
        } else {
            format!("Tab: {tab_label} — {msg_body}")
        };
        let left_width = UnicodeWidthStr::width(left.as_str());
        let pos_width = UnicodeWidthStr::width(pos.as_str());
        let total_width = area.width as usize;
        let spacing = total_width.saturating_sub(left_width + pos_width + 1);
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
