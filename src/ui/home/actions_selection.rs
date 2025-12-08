use std::time::{Duration, Instant};

use crate::ui::home::ToolbarAction;

use super::{HomeApp, NUMERIC_INPUT_TIMEOUT_MS, NumericInput};

impl HomeApp {
    pub(super) fn row_from_y(&self, y: u16, ui: &super::UiMeta) -> Option<usize> {
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

    pub(super) fn hit_toolbar(&self, x: u16, y: u16, ui: &super::UiMeta) -> Option<ToolbarAction> {
        ui.toolbar_hits
            .iter()
            .find(|hit| hit.y == y && x >= hit.x_start && x <= hit.x_end)
            .map(|hit| hit.action)
    }

    pub(super) fn hit_tab(&self, x: u16, y: u16, ui: &super::UiMeta) -> Option<super::TabHitKind> {
        ui.tab_hits
            .iter()
            .find(|hit| x >= hit.x_start && x <= hit.x_end && y >= hit.y_start && y <= hit.y_end)
            .map(|hit| hit.kind)
    }

    pub(super) fn ensure_offset(&mut self, visible_rows: usize) {
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

    pub(super) fn move_selection(&mut self, delta: isize, visible_rows: usize) {
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

    pub(super) fn select_index(&mut self, idx: usize, visible_rows: usize) {
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

    pub(super) fn is_double_click(&self, idx: usize) -> bool {
        if let Some(last) = &self.last_click {
            last.idx == idx && last.at.elapsed() <= Duration::from_millis(350)
        } else {
            false
        }
    }

    pub(super) fn handle_numeric_selection(&mut self, ch: char, visible_rows: usize) {
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

    pub(super) fn reset_numeric_input_if_stale(&mut self) {
        if let Some(input) = self.numeric_input.as_ref()
            && input.last_input.elapsed() > Duration::from_millis(NUMERIC_INPUT_TIMEOUT_MS)
        {
            self.numeric_input = None;
        }
    }

    pub(super) fn record_current_selection(&mut self) {
        let tab_id = self.current_tab_id().map(|t| t.to_string());
        if let (Some(tab_id), Some(id)) = (tab_id.clone(), self.current_id()) {
            self.tab_selected_ids.insert(tab_id, id);
        } else if let Some(tab_id) = tab_id {
            self.tab_selected_ids.remove(&tab_id);
        }
    }
}
