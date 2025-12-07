use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::config::{LayoutNode, SplitDirection};

use super::{DialogField, EditorApp, Mode, Side, ToolbarAction, UiMeta, layout::split_area};

impl EditorApp {
    pub(super) fn draw(&mut self, f: &mut Frame) -> UiMeta {
        let area = f.size();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(8),
                Constraint::Length(2),
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
        hits: &mut Vec<super::LeafHit>,
        split_hits: &mut Vec<super::SplitHit>,
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
                hits.push(super::LeafHit {
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
                    split_hits.push(super::SplitHit {
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
    ) -> Vec<super::ButtonHit> {
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
            hits.push(super::ButtonHit {
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

    fn render_workset_dialog(&self, f: &mut Frame, area: Rect, form: &super::WorksetForm) {
        let popup = centered_rect(80, 50, area);
        f.render_widget(Clear, popup);
        let block = Block::default()
            .title("Edit Workset Info")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        let inner = block.inner(popup);

        let id_line = self.field_line("ID", &form.id, form.focus == DialogField::Id);
        let name_line = self.field_line("Name", &form.name, form.focus == DialogField::Name);
        let tab_label = if self.tab_options.is_empty() {
            "(no tabs)".to_string()
        } else {
            form.selected_tab
                .as_ref()
                .and_then(|id| {
                    self.tab_options
                        .iter()
                        .find(|opt| opt.id == *id)
                        .map(|opt| opt.label.clone())
                })
                .unwrap_or_else(|| "<none>".to_string())
        };

        let workspace_line = self.field_line(
            "Workspace",
            &form.workspace,
            form.focus == DialogField::Workspace,
        );
        let desc_line = self.field_line("Desc", &form.desc, form.focus == DialogField::Desc);
        let tab_line = self.field_line("Tab", &tab_label, form.focus == DialogField::Tab);

        let lines = vec![
            Line::from(id_line),
            Line::from(name_line),
            Line::from(tab_line),
            Line::from(workspace_line),
            Line::from(desc_line),
            Line::from(""),
            Line::from(
                "Enter: Save  Tab/Shift+Tab: Switch field  Left/Right/Space: Change tab  Esc: Cancel",
            ),
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
            DialogField::Tab => {
                let prefix_w = UnicodeWidthStr::width("Tab: ") as u16;
                let w = UnicodeWidthStr::width(tab_label.as_str()) as u16;
                (inner.x + prefix_w + w, inner.y + 2)
            }
            DialogField::Workspace => {
                let prefix_w = UnicodeWidthStr::width("Workspace: ") as u16;
                let w = width_up_to(&form.workspace, form.cursor_workspace);
                (inner.x + prefix_w + w, inner.y + 3)
            }
            DialogField::Desc => {
                let prefix_w = UnicodeWidthStr::width("Desc: ") as u16;
                let w = width_up_to(&form.desc, form.cursor_desc);
                (inner.x + prefix_w + w, inner.y + 4)
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
}

pub(super) fn hit_toolbar(x: u16, y: u16, ui: &UiMeta) -> Option<ToolbarAction> {
    ui.toolbar_hits
        .iter()
        .find(|hit| hit.y == y && x >= hit.x_start && x <= hit.x_end)
        .map(|hit| hit.action)
}

pub(super) fn hit_split(x: u16, y: u16, ui: &UiMeta) -> Option<super::SplitHit> {
    ui.split_hits
        .iter()
        .find(|hit| point_in_rect(x, y, hit.boundary))
        .cloned()
}

pub(super) fn point_in_rect(x: u16, y: u16, rect: Rect) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

fn render_split_highlight(f: &mut Frame, rect: Rect, _direction: SplitDirection, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let block = Block::default().style(Style::default().bg(color));
    f.render_widget(block, rect);
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
