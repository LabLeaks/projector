/**
@module PROJECTOR.EDGE.BROWSER_UI
Owns shared terminal-browser UI helpers reused across projector CLI browser flows.
*/
// @fileimplements PROJECTOR.EDGE.BROWSER_UI
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
