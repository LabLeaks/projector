/**
@module PROJECTOR.EDGE.GET_BROWSER
Owns the terminal browser for `projector get`, including interactive remote sync-entry selection and metadata/preview rendering before local materialization.
*/
// @fileimplements PROJECTOR.EDGE.GET_BROWSER
use std::error::Error;
use std::io;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use projector_domain::SyncEntrySummary;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

#[derive(Clone, Debug)]
pub(crate) enum GetBrowserExit {
    Selected(SyncEntrySummary),
    Cancelled,
}

pub(crate) fn browse_sync_entries(
    entries: &[SyncEntrySummary],
) -> Result<GetBrowserExit, Box<dyn Error>> {
    let mut browser = GetBrowser::new(entries)?;
    browser.run()
}

struct GetBrowser {
    entries: Vec<SyncEntrySummary>,
    selected_idx: usize,
}

impl GetBrowser {
    fn new(entries: &[SyncEntrySummary]) -> Result<Self, Box<dyn Error>> {
        if entries.is_empty() {
            return Err("selected server profile has no available remote sync entries".into());
        }

        Ok(Self {
            entries: entries.to_vec(),
            selected_idx: 0,
        })
    }

    fn run(&mut self) -> Result<GetBrowserExit, Box<dyn Error>> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen)?;
        let _guard = GetTerminalGuard;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        loop {
            terminal.draw(|frame| self.render(frame))?;
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.move_selection_up(),
                KeyCode::Down | KeyCode::Char('j') => self.move_selection_down(),
                KeyCode::Enter => {
                    return Ok(GetBrowserExit::Selected(
                        self.entries[self.selected_idx].clone(),
                    ));
                }
                KeyCode::Esc | KeyCode::Char('q') => return Ok(GetBrowserExit::Cancelled),
                _ => {}
            }
        }
    }

    fn render(&self, frame: &mut Frame<'_>) {
        let selected = &self.entries[self.selected_idx];
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Min(12),
                Constraint::Length(3),
            ])
            .split(frame.area());

        let header = Paragraph::new(vec![
            Line::raw("get browse"),
            Line::raw(format!("selected id: {}", selected.sync_entry_id)),
            Line::raw(format!("default local path: {}", selected.remote_path)),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Remote Sync Entries"),
        );
        frame.render_widget(header, layout[0]);

        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(layout[1]);

        let mut list_state = ListState::default();
        list_state.select(Some(self.selected_idx));
        let items = self
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(vec![
                    Line::raw(format!(
                        "{} [{}]",
                        entry.remote_path,
                        format_sync_entry_kind(&entry.kind)
                    )),
                    Line::raw(format!(
                        "{}  {}",
                        abbreviate_id(&entry.sync_entry_id),
                        format_timestamp(entry.last_updated_ms)
                    )),
                ])
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Entries"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, main[0], &mut list_state);

        let details = Paragraph::new(vec![
            Line::raw(format!("id: {}", selected.sync_entry_id)),
            Line::raw(format!("path: {}", selected.remote_path)),
            Line::raw(format!("kind: {}", format_sync_entry_kind(&selected.kind))),
            Line::raw(format!(
                "source repo: {}",
                selected.source_repo_name.as_deref().unwrap_or("unknown")
            )),
            Line::raw(format!(
                "last updated: {}",
                format_timestamp(selected.last_updated_ms)
            )),
            Line::raw(""),
            Line::raw("preview:"),
            Line::raw(selected.preview.as_deref().unwrap_or("(no preview)")),
        ])
        .block(Block::default().borders(Borders::ALL).title("Details"))
        .wrap(Wrap { trim: false });
        frame.render_widget(details, main[1]);

        let footer = Paragraph::new("j/k or arrows: move  enter: get selected entry  q: cancel")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(footer, layout[2]);
    }

    fn move_selection_up(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    fn move_selection_down(&mut self) {
        if self.selected_idx + 1 < self.entries.len() {
            self.selected_idx += 1;
        }
    }
}

struct GetTerminalGuard;

impl Drop for GetTerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, LeaveAlternateScreen);
    }
}

fn format_sync_entry_kind(kind: &projector_domain::SyncEntryKind) -> &'static str {
    match kind {
        projector_domain::SyncEntryKind::File => "file",
        projector_domain::SyncEntryKind::Directory => "directory",
    }
}

fn abbreviate_id(value: &str) -> String {
    if value.len() <= 18 {
        value.to_owned()
    } else {
        format!("{}...", &value[..18])
    }
}

fn format_timestamp(timestamp_ms: Option<u128>) -> String {
    let Some(timestamp_ms) = timestamp_ms else {
        return "unknown".to_owned();
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    if now_ms <= timestamp_ms {
        return "just now".to_owned();
    }
    let delta = now_ms - timestamp_ms;
    if delta < 60_000 {
        format!("{}s ago", delta / 1_000)
    } else if delta < 3_600_000 {
        format!("{}m ago", delta / 60_000)
    } else if delta < 86_400_000 {
        format!("{}h ago", delta / 3_600_000)
    } else {
        format!("{}d ago", delta / 86_400_000)
    }
}
