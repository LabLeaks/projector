/**
@module PROJECTOR.EDGE.RESTORE_BROWSER
Owns the terminal revision browser for `projector restore`, including interactive revision selection and diff preview rendering for human-driven PITR.
*/
// @fileimplements PROJECTOR.EDGE.RESTORE_BROWSER
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
use projector_domain::DocumentBodyRevision;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::browser_ui::centered_rect;

#[derive(Clone, Debug)]
pub(crate) struct RestoreSelection {
    pub(crate) seq: u64,
}

#[derive(Clone, Debug)]
pub(crate) enum RestoreBrowserExit {
    Selected(RestoreSelection),
    Cancelled { selected_seq: u64 },
}

pub(crate) fn browse_restore_revisions(
    requested_path: &Path,
    current_text: &str,
    revisions: &[DocumentBodyRevision],
    default_seq: u64,
    confirm: bool,
) -> Result<RestoreBrowserExit, Box<dyn Error>> {
    let mut browser = RestoreBrowser::new(
        requested_path,
        current_text,
        revisions,
        default_seq,
        confirm,
    )?;
    browser.run()
}

pub(crate) fn simple_line_diff_with_labels(
    current_label: &str,
    restored_label: &str,
    current_text: &str,
    restored_text: &str,
) -> Vec<String> {
    let current = split_lines_for_diff(current_text);
    let restored = split_lines_for_diff(restored_text);
    let lcs = build_lcs_table(&current, &restored);
    let mut lines = vec![
        format!("--- {current_label}"),
        format!("+++ {restored_label}"),
        "@@".to_owned(),
    ];
    lines.extend(render_lcs_diff(&current, &restored, &lcs, 0, 0));
    lines
}

pub(crate) fn simple_line_diff(current_text: &str, restored_text: &str) -> Vec<String> {
    simple_line_diff_with_labels("current", "restored", current_text, restored_text)
}

#[derive(Clone, Debug)]
struct BrowserRevision {
    seq: u64,
    actor_id: String,
    timestamp_ms: u128,
    conflicted: bool,
    diff_lines: Vec<String>,
}

struct RestoreBrowser<'a> {
    requested_path: &'a Path,
    revisions: Vec<BrowserRevision>,
    selected_idx: usize,
    diff_scroll: u16,
    mode: BrowserMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrowserMode {
    Browsing,
    Confirming,
}

impl<'a> RestoreBrowser<'a> {
    fn new(
        requested_path: &'a Path,
        current_text: &str,
        revisions: &[DocumentBodyRevision],
        default_seq: u64,
        confirm: bool,
    ) -> Result<Self, Box<dyn Error>> {
        if revisions.is_empty() {
            return Err("restore requires at least one body revision".into());
        }

        let revisions = revisions
            .iter()
            .map(|revision| BrowserRevision {
                seq: revision.seq,
                actor_id: revision.actor_id.clone(),
                timestamp_ms: revision.timestamp_ms,
                conflicted: revision.conflicted,
                diff_lines: simple_line_diff(current_text, &revision.body_text),
            })
            .collect::<Vec<_>>();

        let selected_idx = revisions
            .iter()
            .position(|revision| revision.seq == default_seq)
            .unwrap_or(revisions.len().saturating_sub(1));

        Ok(Self {
            requested_path,
            revisions,
            selected_idx,
            diff_scroll: 0,
            mode: if confirm {
                BrowserMode::Confirming
            } else {
                BrowserMode::Browsing
            },
        })
    }

    fn run(&mut self) -> Result<RestoreBrowserExit, Box<dyn Error>> {
        let mut stdout = io::stdout();
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen)?;
        let _guard = RestoreTerminalGuard;
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
                KeyCode::PageUp => self.scroll_diff_up(10),
                KeyCode::PageDown => self.scroll_diff_down(10),
                KeyCode::Home => self.diff_scroll = 0,
                KeyCode::End => self.diff_scroll = self.max_diff_scroll(),
                KeyCode::Enter => match self.mode {
                    BrowserMode::Browsing => self.mode = BrowserMode::Confirming,
                    BrowserMode::Confirming => {
                        let revision = &self.revisions[self.selected_idx];
                        return Ok(RestoreBrowserExit::Selected(RestoreSelection {
                            seq: revision.seq,
                        }));
                    }
                },
                KeyCode::Char('y') if self.mode == BrowserMode::Confirming => {
                    let revision = &self.revisions[self.selected_idx];
                    return Ok(RestoreBrowserExit::Selected(RestoreSelection {
                        seq: revision.seq,
                    }));
                }
                KeyCode::Char('n') if self.mode == BrowserMode::Confirming => {
                    self.mode = BrowserMode::Browsing;
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    return Ok(RestoreBrowserExit::Cancelled {
                        selected_seq: self.revisions[self.selected_idx].seq,
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

        let selected = self.selected_revision();
        let selected_tags = self.revision_tags(selected);
        let header = Paragraph::new(vec![
            Line::raw(format!(
                "restore {}",
                match self.mode {
                    BrowserMode::Browsing => "browse",
                    BrowserMode::Confirming => "confirm",
                }
            )),
            Line::raw(format!("path: {}", self.requested_path.display())),
            Line::raw(format!(
                "selected: seq={}{}  total revisions={}",
                selected.seq,
                format_tags_inline(&selected_tags),
                self.revisions.len()
            )),
        ])
        .block(Block::default().borders(Borders::ALL).title("Restore"));
        frame.render_widget(header, layout[0]);

        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(layout[1]);

        let mut list_state = ListState::default();
        list_state.select(Some(self.selected_idx));
        let revisions = self
            .revisions
            .iter()
            .map(|revision| {
                let actor = abbreviate_actor(&revision.actor_id);
                let ts = format_timestamp(revision.timestamp_ms);
                let tags = self.revision_tags(revision);
                ListItem::new(vec![
                    Line::raw(format!("seq={}{}", revision.seq, format_tags_inline(&tags))),
                    Line::raw(format!("{} by {}", ts, actor)),
                ])
            })
            .collect::<Vec<_>>();
        let list = List::new(revisions)
            .block(Block::default().borders(Borders::ALL).title("Revisions"))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> ");
        frame.render_stateful_widget(list, main[0], &mut list_state);

        let revision = &self.revisions[self.selected_idx];
        let diff_text = if revision.diff_lines.is_empty() {
            vec![Line::raw("(no content change)")]
        } else {
            revision
                .diff_lines
                .iter()
                .map(|line| Line::raw(line.as_str()))
                .collect()
        };
        let diff = Paragraph::new(diff_text)
            .block(Block::default().borders(Borders::ALL).title(format!(
                "Diff for seq={}{}",
                revision.seq,
                format_tags_inline(&selected_tags)
            )))
            .wrap(Wrap { trim: false })
            .scroll((self.diff_scroll, 0));
        frame.render_widget(diff, main[1]);

        let footer_hint = if self.mode == BrowserMode::Confirming {
            "j/k or arrows: move  pgup/pgdn: scroll diff\ny or enter: apply selected revision  n: back  q: cancel"
        } else {
            "j/k or arrows: move  pgup/pgdn: scroll diff\nenter: choose selected revision  q: cancel"
        };
        let footer =
            Paragraph::new(footer_hint).block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(footer, layout[2]);

        if self.mode == BrowserMode::Confirming {
            let popup = centered_rect(55, 20, frame.area());
            frame.render_widget(Clear, popup);
            let revision = &self.revisions[self.selected_idx];
            let confirmation = Paragraph::new(vec![
                Line::raw(format!("Apply restore for seq={}?", revision.seq)),
                Line::raw(""),
                Line::raw("y or enter: apply"),
                Line::raw("n: return to browser"),
                Line::raw("q: cancel restore"),
            ])
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Confirm restore"),
            )
            .wrap(Wrap { trim: false });
            frame.render_widget(confirmation, popup);
        }
    }

    fn move_selection_up(&mut self) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
            self.diff_scroll = 0;
        }
    }

    fn move_selection_down(&mut self) {
        if self.selected_idx + 1 < self.revisions.len() {
            self.selected_idx += 1;
            self.diff_scroll = 0;
        }
    }

    fn scroll_diff_up(&mut self, amount: u16) {
        self.diff_scroll = self.diff_scroll.saturating_sub(amount);
    }

    fn scroll_diff_down(&mut self, amount: u16) {
        self.diff_scroll = self
            .diff_scroll
            .saturating_add(amount)
            .min(self.max_diff_scroll());
    }

    fn max_diff_scroll(&self) -> u16 {
        self.revisions[self.selected_idx]
            .diff_lines
            .len()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16
    }
}

impl RestoreBrowser<'_> {
    fn selected_revision(&self) -> &BrowserRevision {
        &self.revisions[self.selected_idx]
    }

    fn revision_tags(&self, revision: &BrowserRevision) -> Vec<&'static str> {
        let mut tags = Vec::new();
        if Some(revision.seq) == self.latest_seq() {
            tags.push("current");
        } else if Some(revision.seq) == self.previous_seq() {
            tags.push("previous");
        }
        if revision.conflicted {
            tags.push("conflicted");
        }
        tags
    }

    fn latest_seq(&self) -> Option<u64> {
        self.revisions.last().map(|revision| revision.seq)
    }

    fn previous_seq(&self) -> Option<u64> {
        if self.revisions.len() >= 2 {
            Some(self.revisions[self.revisions.len() - 2].seq)
        } else {
            None
        }
    }
}

struct RestoreTerminalGuard;

impl Drop for RestoreTerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn abbreviate_actor(actor_id: &str) -> String {
    let suffix = actor_id
        .split_once('-')
        .map(|(_, suffix)| suffix)
        .unwrap_or(actor_id);
    if suffix.len() <= 8 {
        suffix.to_owned()
    } else {
        format!("…{}", &suffix[suffix.len() - 6..])
    }
}

fn format_tags_inline(tags: &[&str]) -> String {
    if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", tags.join(", "))
    }
}

fn format_timestamp(timestamp_ms: u128) -> String {
    let seconds = (timestamp_ms / 1000) as u64;
    let timestamp = UNIX_EPOCH + Duration::from_secs(seconds);
    let Ok(age) = SystemTime::now().duration_since(timestamp) else {
        return format!("{seconds}s");
    };
    let seconds = age.as_secs();
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else {
        format!("{}h ago", seconds / 3600)
    }
}

fn split_lines_for_diff(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n')
        .map(|line| line.trim_end_matches('\n').to_owned())
        .collect()
}

fn build_lcs_table(left: &[String], right: &[String]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0; right.len() + 1]; left.len() + 1];
    for left_idx in (0..left.len()).rev() {
        for right_idx in (0..right.len()).rev() {
            table[left_idx][right_idx] = if left[left_idx] == right[right_idx] {
                table[left_idx + 1][right_idx + 1] + 1
            } else {
                table[left_idx + 1][right_idx].max(table[left_idx][right_idx + 1])
            };
        }
    }
    table
}

fn render_lcs_diff(
    current: &[String],
    restored: &[String],
    lcs: &[Vec<usize>],
    current_idx: usize,
    restored_idx: usize,
) -> Vec<String> {
    let mut current_idx = current_idx;
    let mut restored_idx = restored_idx;
    let mut rendered = Vec::new();
    while current_idx < current.len() && restored_idx < restored.len() {
        if current[current_idx] == restored[restored_idx] {
            rendered.push(format!(" {}", current[current_idx]));
            current_idx += 1;
            restored_idx += 1;
        } else if lcs[current_idx + 1][restored_idx] >= lcs[current_idx][restored_idx + 1] {
            rendered.push(format!("-{}", current[current_idx]));
            current_idx += 1;
        } else {
            rendered.push(format!("+{}", restored[restored_idx]));
            restored_idx += 1;
        }
    }
    while current_idx < current.len() {
        rendered.push(format!("-{}", current[current_idx]));
        current_idx += 1;
    }
    while restored_idx < restored.len() {
        rendered.push(format!("+{}", restored[restored_idx]));
        restored_idx += 1;
    }
    rendered
}
