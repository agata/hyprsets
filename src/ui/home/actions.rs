use std::io;

use anyhow::Result;
use crossterm::{
    cursor::{Hide, Show},
    execute,
};

use super::{ALL_TAB_ID, HomeApp};

impl HomeApp {
    pub(super) fn start_new_dialog(&mut self) -> Result<()> {
        execute!(io::stdout(), Show)?;
        let state = self.new_dialog_state();
        self.mode = super::Mode::NewDialog(state);
        Ok(())
    }

    pub(super) fn new_dialog_state(&self) -> super::NewDialogState {
        super::NewDialogState {
            id: self.generate_id(),
            name: String::new(),
            desc: String::new(),
            selected_tab: self.default_new_tab(),
            focus: super::DialogField::Name,
        }
    }

    pub(super) fn default_new_tab(&self) -> Option<String> {
        self.current_tab_id()
            .filter(|id| *id != ALL_TAB_ID)
            .map(str::to_string)
    }

    pub(super) fn hide_cursor(&self) -> Result<()> {
        execute!(io::stdout(), Hide)?;
        Ok(())
    }
}
