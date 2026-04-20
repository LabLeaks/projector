/**
@module PROJECTOR.EDGE.BROWSER_UI
Owns shared terminal-browser UI helpers reused across projector CLI browser flows.
*/
// @fileimplements PROJECTOR.EDGE.BROWSER_UI
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub(crate) fn format_unix_timestamp(timestamp_ms: u128) -> String {
    let secs = (timestamp_ms / 1_000) as u64;
    let system_time = UNIX_EPOCH + Duration::from_secs(secs);
    match system_time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => format!("unix={}s", duration.as_secs()),
        Err(_) => format!("unix={}s", secs),
    }
}

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

pub(crate) struct TerminalUiGuard {
    raw_mode_enabled: bool,
    alternate_screen_enabled: bool,
}

impl TerminalUiGuard {
    pub(crate) fn new() -> Self {
        Self {
            raw_mode_enabled: false,
            alternate_screen_enabled: false,
        }
    }

    pub(crate) fn enable_raw_mode(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        self.raw_mode_enabled = true;
        Ok(())
    }

    pub(crate) fn enter_alternate_screen<W: io::Write>(
        &mut self,
        writer: &mut W,
    ) -> io::Result<()> {
        execute!(writer, EnterAlternateScreen)?;
        self.alternate_screen_enabled = true;
        Ok(())
    }
}

impl Drop for TerminalUiGuard {
    fn drop(&mut self) {
        if self.alternate_screen_enabled {
            let mut stdout = io::stdout();
            let _ = execute!(stdout, LeaveAlternateScreen);
        }
        if self.raw_mode_enabled {
            let _ = disable_raw_mode();
        }
    }
}
