/**
@module PROJECTOR.EDGE.REDACT_BROWSER
Owns the terminal retained-history browser for `projector redact`, including interactive match inspection and explicit confirmation before applying retained-history redaction.
*/
// @fileimplements PROJECTOR.EDGE.REDACT_BROWSER
use std::error::Error;
use std::io;
use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use projector_domain::DocumentBodyRedactionMatch;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::browser_ui::{centered_rect, format_unix_timestamp};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RedactBrowserExit {
    Apply { selected_seq: u64 },
    Cancelled { selected_seq: u64 },
}

pub(crate) fn browse_redaction_matches(
    requested_path: &Path,
    exact_text: &str,
    matches: &[DocumentBodyRedactionMatch],
) -> Result<RedactBrowserExit, Box<dyn Error>> {
    let mut browser = RedactBrowser::new(requested_path, exact_text, matches)?;
    browser.run()
}

#[derive(Clone, Debug)]
struct RedactBrowser<'a> {
    requested_path: &'a Path,
    exact_text: &'a str,
    matches: Vec<DocumentBodyRedactionMatch>,
    selected_idx: usize,
    detail_scroll: u16,
    mode: BrowserMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrowserMode {
    Browsing,
    Confirming,
}

impl<'a> RedactBrowser<'a> {
    fn new(
        requested_path: &'a Path,
        exact_text: &'a str,
        matches: &[DocumentBodyRedactionMatch],
    ) -> Result<Self, Box<dyn Error>> {
        if matches.is_empty() {
            return Err("redact browser requires at least one retained match".into());
        }

        Ok(Self {
            requested_path,
            exact_text,
            matches: matches.to_vec(),
            selected_idx: 0,
            detail_scroll: 0,
            mode: BrowserMode::Browsing,
        })
    }

    fn run(&mut self) -> Result<RedactBrowserExit, Box<dyn Error>> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen)?;
        let _guard = RedactTerminalGuard;
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
                KeyCode::PageUp => self.scroll_detail_up(10),
                KeyCode::PageDown => self.scroll_detail_down(10),
                KeyCode::Home => self.detail_scroll = 0,
                KeyCode::End => self.detail_scroll = self.max_detail_scroll(),
                KeyCode::Enter => match self.mode {
                    BrowserMode::Browsing => self.mode = BrowserMode::Confirming,
                    BrowserMode::Confirming => {
                        return Ok(RedactBrowserExit::Apply {
                            selected_seq: self.selected_match().seq,
                        });
                    }
                },
                KeyCode::Char('y') if self.mode == BrowserMode::Confirming => {
                    return Ok(RedactBrowserExit::Apply {
                        selected_seq: self.selected_match().seq,
                    });
                }
                KeyCode::Char('n') if self.mode == BrowserMode::Confirming => {
                    self.mode = BrowserMode::Browsing;
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    return Ok(RedactBrowserExit::Cancelled {
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
                Constraint::Length(6),
                Constraint::Min(12),
                Constraint::Length(4),
            ])
            .split(frame.area());

        let selected = self.selected_match();
        let header = Paragraph::new(vec![
            Line::raw(format!(
                "redact {}",
                match self.mode {
                    BrowserMode::Browsing => "browse",
                    BrowserMode::Confirming => "confirm",
                }
            )),
            Line::raw(format!("path: {}", self.requested_path.display())),
            Line::raw(format!("exact_text: {:?}", self.exact_text)),
            Line::raw(format!(
                "selected: seq={}  total matches={}",
                selected.seq,
                self.matches.len()
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title("Redact"));
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
                        "seq={} kind={} occs={}{}",
                        entry.seq, entry.history_kind, entry.occurrences, anchor
                    )),
                    Line::raw(format!("{ts} by {actor}")),
                ])
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Matching Revisions"),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, main[0], &mut list_state);

        let preview_lines = self
            .selected_match()
            .preview_lines
            .iter()
            .map(|line| Line::raw(line.as_str()))
            .collect::<Vec<_>>();
        let detail = Paragraph::new(preview_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Preview (+ after redaction, - before)"),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.detail_scroll, 0));
        frame.render_widget(detail, main[1]);

        let footer = Paragraph::new(match self.mode {
            BrowserMode::Browsing => vec![
                Line::raw("up/down or j/k: choose retained revision"),
                Line::raw("page up/down: scroll preview"),
                Line::raw("enter: confirm redaction  q/esc: cancel"),
            ],
            BrowserMode::Confirming => vec![
                Line::raw("apply retained-history redaction for all listed matches?"),
                Line::raw("enter or y: apply  n: back  q/esc: cancel"),
            ],
        })
        .block(Block::default().borders(Borders::ALL).title("Controls"));
        frame.render_widget(footer, layout[2]);

        if self.mode == BrowserMode::Confirming {
            let area = centered_rect(60, 20, frame.area());
            frame.render_widget(Clear, area);
            let confirm = Paragraph::new(vec![
                Line::raw("Apply retained-history redaction?"),
                Line::raw(format!("matches: {}", self.matches.len())),
                Line::raw("This rewrites retained history to [REDACTED]."),
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
            self.detail_scroll = 0;
        }
    }

    fn move_selection_down(&mut self) {
        if self.selected_idx + 1 < self.matches.len() {
            self.selected_idx += 1;
            self.detail_scroll = 0;
        }
    }

    fn scroll_detail_up(&mut self, amount: u16) {
        self.detail_scroll = self.detail_scroll.saturating_sub(amount);
    }

    fn scroll_detail_down(&mut self, amount: u16) {
        self.detail_scroll = self
            .detail_scroll
            .saturating_add(amount)
            .min(self.max_detail_scroll());
    }

    fn max_detail_scroll(&self) -> u16 {
        self.selected_match()
            .preview_lines
            .len()
            .saturating_sub(10)
            .min(u16::MAX as usize) as u16
    }

    fn selected_match(&self) -> &DocumentBodyRedactionMatch {
        &self.matches[self.selected_idx]
    }
}

struct RedactTerminalGuard;

impl Drop for RedactTerminalGuard {
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
    format_unix_timestamp(timestamp_ms)
}
