/**
@module PROJECTOR.EDGE.PURGE_BROWSER
Owns the terminal retained-history browser for `projector purge`, including interactive revision inspection and explicit confirmation before clearing retained body content.
*/
// @fileimplements PROJECTOR.EDGE.PURGE_BROWSER
use std::error::Error;
use std::io;
use std::path::Path;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use projector_domain::DocumentBodyPurgeMatch;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PurgeBrowserExit {
    Apply { selected_seq: u64 },
    Cancelled { selected_seq: u64 },
}

pub(crate) fn browse_purge_matches(
    requested_path: &Path,
    matches: &[DocumentBodyPurgeMatch],
) -> Result<PurgeBrowserExit, Box<dyn Error>> {
    let mut browser = PurgeBrowser::new(requested_path, matches)?;
    browser.run()
}

#[derive(Clone, Debug)]
struct PurgeBrowser<'a> {
    requested_path: &'a Path,
    matches: Vec<DocumentBodyPurgeMatch>,
    selected_idx: usize,
    mode: BrowserMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrowserMode {
    Browsing,
    Confirming,
}

impl<'a> PurgeBrowser<'a> {
    fn new(
        requested_path: &'a Path,
        matches: &[DocumentBodyPurgeMatch],
    ) -> Result<Self, Box<dyn Error>> {
        if matches.is_empty() {
            return Err("purge browser requires at least one retained match".into());
        }

        Ok(Self {
            requested_path,
            matches: matches.to_vec(),
            selected_idx: 0,
            mode: BrowserMode::Browsing,
        })
    }

    fn run(&mut self) -> Result<PurgeBrowserExit, Box<dyn Error>> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen)?;
        let _guard = PurgeTerminalGuard;
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
                KeyCode::Enter => match self.mode {
                    BrowserMode::Browsing => self.mode = BrowserMode::Confirming,
                    BrowserMode::Confirming => {
                        return Ok(PurgeBrowserExit::Apply {
                            selected_seq: self.selected_match().seq,
                        });
                    }
                },
                KeyCode::Char('y') if self.mode == BrowserMode::Confirming => {
                    return Ok(PurgeBrowserExit::Apply {
                        selected_seq: self.selected_match().seq,
                    });
                }
                KeyCode::Char('n') if self.mode == BrowserMode::Confirming => {
                    self.mode = BrowserMode::Browsing;
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    return Ok(PurgeBrowserExit::Cancelled {
                        selected_seq: self.selected_match().seq,
                    });
                }
                _ => {}
            }
        }
    }

    fn render(&self, frame: &mut Frame<'_>) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Min(12),
                Constraint::Length(4),
            ])
            .split(frame.area());

        let selected = self.selected_match();
        let header = Paragraph::new(vec![
            Line::raw(format!(
                "purge {}",
                match self.mode {
                    BrowserMode::Browsing => "browse",
                    BrowserMode::Confirming => "confirm",
                }
            )),
            Line::raw(format!("path: {}", self.requested_path.display())),
            Line::raw(format!(
                "selected: seq={}  clearable revisions={}",
                selected.seq,
                self.matches.len()
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title("Purge"));
        frame.render_widget(header, layout[0]);

        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(layout[1]);

        let mut list_state = ListState::default();
        list_state.select(Some(self.selected_idx));
        let items = self
            .matches
            .iter()
            .map(|entry| {
                let actor = abbreviate_actor(&entry.actor_id);
                let ts = format_timestamp(entry.timestamp_ms);
                let anchor = entry
                    .checkpoint_anchor_seq
                    .map(|seq| format!(" anchor={seq}"))
                    .unwrap_or_default();
                ListItem::new(vec![
                    Line::raw(format!(
                        "seq={} kind={} body_len={}{}",
                        entry.seq, entry.history_kind, entry.body_len, anchor
                    )),
                    Line::raw(format!("{ts} by {actor}")),
                ])
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Clearable Revisions"),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, main[0], &mut list_state);

        let detail = Paragraph::new(vec![
            Line::raw(format!("seq: {}", selected.seq)),
            Line::raw(format!("history_kind: {}", selected.history_kind)),
            Line::raw(format!("body_len: {}", selected.body_len)),
            Line::raw(format!(
                "checkpoint_anchor_seq: {}",
                selected
                    .checkpoint_anchor_seq
                    .map(|seq| seq.to_string())
                    .unwrap_or_else(|| "-".to_owned())
            )),
            Line::raw(format!("actor_id: {}", selected.actor_id)),
            Line::raw(format!("document_id: {}", selected.document_id)),
            Line::raw(format!("timestamp_ms: {}", selected.timestamp_ms)),
            Line::raw(""),
            Line::raw("Purge clears retained body content for every listed revision."),
            Line::raw("Live current document state is left unchanged."),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Selected Revision"),
        )
        .wrap(Wrap { trim: false });
        frame.render_widget(detail, main[1]);

        let footer = Paragraph::new(match self.mode {
            BrowserMode::Browsing => vec![
                Line::raw("up/down or j/k: choose retained revision"),
                Line::raw("enter: confirm purge  q/esc: cancel"),
            ],
            BrowserMode::Confirming => vec![
                Line::raw("apply retained-history purge for all listed revisions?"),
                Line::raw("enter or y: apply  n: back  q/esc: cancel"),
            ],
        })
        .block(Block::default().borders(Borders::ALL).title("Controls"));
        frame.render_widget(footer, layout[2]);

        if self.mode == BrowserMode::Confirming {
            let area = centered_rect(60, 20, frame.area());
            frame.render_widget(Clear, area);
            let confirm = Paragraph::new(vec![
                Line::raw("Apply retained-history purge?"),
                Line::raw(format!("clearable revisions: {}", self.matches.len())),
                Line::raw("This clears retained body content only."),
                Line::raw(""),
                Line::raw("enter / y: apply   n: back   q/esc: cancel"),
            ])
            .block(Block::default().borders(Borders::ALL).title("Confirm"))
            .wrap(Wrap { trim: false });
            frame.render_widget(confirm, area);
        }
    }

    fn move_selection_up(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
        }
    }

    fn move_selection_down(&mut self) {
        if self.selected_idx + 1 < self.matches.len() {
            self.selected_idx += 1;
        }
    }

    fn selected_match(&self) -> &DocumentBodyPurgeMatch {
        &self.matches[self.selected_idx]
    }
}

struct PurgeTerminalGuard;

impl Drop for PurgeTerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn abbreviate_actor(actor_id: &str) -> String {
    const LIMIT: usize = 24;
    if actor_id.len() <= LIMIT {
        actor_id.to_owned()
    } else {
        format!("{}...", &actor_id[..LIMIT - 3])
    }
}

fn format_timestamp(timestamp_ms: u128) -> String {
    let secs = (timestamp_ms / 1_000) as u64;
    let system_time = UNIX_EPOCH + Duration::from_secs(secs);
    match system_time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => format!("unix={}s", duration.as_secs()),
        Err(_) => format!("unix={}s", secs),
    }
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
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
