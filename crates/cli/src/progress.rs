//! Terminal progress dashboard for LLM summarization.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, IsTerminal, Write};
use std::time::Instant;

use aissence::{SummarizationProgress, SummarizationRequestKind};
use terminal_size::{Width, terminal_size};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[38;5;81m";
const GREEN: &str = "\x1b[38;5;114m";
const YELLOW: &str = "\x1b[38;5;221m";
const RED: &str = "\x1b[38;5;203m";
const BLUE: &str = "\x1b[38;5;75m";
const MUTED: &str = "\x1b[38;5;244m";
const MAX_VISIBLE_REQUESTS: usize = aissence::summarize::MAX_CONCURRENT_REQUESTS;
const MIN_CELL_WIDTH: usize = 38;
const MAX_GRID_COLUMNS: usize = 4;
const CELL_GAP: &str = "  ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestStatus {
    InFlight,
    Retrying(usize),
    Completed,
    Failed,
}

#[derive(Debug)]
struct RequestState {
    kind: SummarizationRequestKind,
    status: RequestStatus,
    tokens: usize,
    started: Instant,
}

/// An animated dashboard for the requests emitted by the summarizer.
///
/// The dashboard is deliberately terminal-only. Piped/non-interactive runs
/// retain normal command output and never receive cursor-control sequences.
pub(crate) struct LiveProgress {
    interactive: bool,
    color: bool,
    frame: usize,
    rendered_lines: usize,
    cursor_hidden: bool,
    title: String,
    model: String,
    requests: BTreeMap<usize, RequestState>,
    slots: Vec<Option<usize>>,
    recent_failures: VecDeque<(usize, String)>,
}

impl LiveProgress {
    /// Create a progress dashboard with a title and model label.
    pub(crate) fn new(title: &str, model: &str) -> Self {
        Self {
            interactive: io::stderr().is_terminal(),
            color: std::env::var_os("NO_COLOR").is_none(),
            frame: 0,
            rendered_lines: 0,
            cursor_hidden: false,
            title: title.to_string(),
            model: model.to_string(),
            requests: BTreeMap::new(),
            slots: vec![None; MAX_VISIBLE_REQUESTS],
            recent_failures: VecDeque::new(),
        }
    }

    /// Apply one progress event emitted by the summarizer.
    pub(crate) fn handle(&mut self, event: SummarizationProgress) {
        match event {
            SummarizationProgress::Started { id, kind } => {
                self.assign_slot(id);
                self.requests
                    .entry(id)
                    .and_modify(|state| {
                        state.status = RequestStatus::InFlight;
                        state.kind = kind;
                    })
                    .or_insert(RequestState {
                        kind,
                        status: RequestStatus::InFlight,
                        tokens: 0,
                        started: Instant::now(),
                    });
            }
            SummarizationProgress::TokensReceived { id, tokens } => {
                if let Some(state) = self.requests.get_mut(&id) {
                    state.tokens = tokens;
                }
            }
            SummarizationProgress::Retrying {
                id,
                kind,
                attempt,
                error: _,
            } => {
                self.assign_slot(id);
                self.requests
                    .entry(id)
                    .and_modify(|state| {
                        state.status = RequestStatus::Retrying(attempt);
                        state.kind = kind;
                    })
                    .or_insert(RequestState {
                        kind,
                        status: RequestStatus::Retrying(attempt),
                        tokens: 0,
                        started: Instant::now(),
                    });
            }
            SummarizationProgress::Completed { id, kind, usage } => {
                self.assign_slot(id);
                let state = self.requests.entry(id).or_insert(RequestState {
                    kind,
                    status: RequestStatus::Completed,
                    tokens: 0,
                    started: Instant::now(),
                });
                state.kind = kind;
                state.status = RequestStatus::Completed;
                if let Some(usage) = usage {
                    state.tokens = usage.completion_tokens as usize;
                }
            }
            SummarizationProgress::Failed { id, kind, error } => {
                self.assign_slot(id);
                let state = self.requests.entry(id).or_insert(RequestState {
                    kind,
                    status: RequestStatus::Failed,
                    tokens: 0,
                    started: Instant::now(),
                });
                state.kind = kind;
                state.status = RequestStatus::Failed;
                self.recent_failures.push_front((id, error));
                self.recent_failures.truncate(3);
            }
        }
    }

    /// Render the current dashboard, if stderr is an interactive terminal.
    pub(crate) fn render(&mut self) {
        if !self.interactive {
            return;
        }

        let lines = self.dashboard_lines(terminal_width());
        self.write_dashboard(&lines);
        self.frame = self.frame.wrapping_add(1);
    }

    /// Render the final state and restore the terminal cursor.
    pub(crate) fn finish(&mut self, success: bool) {
        if !self.interactive {
            return;
        }

        let lines = self.dashboard_lines_for(terminal_width(), Some(success));
        self.write_dashboard(&lines);
        self.show_cursor();
        self.rendered_lines = 0;
    }

    fn dashboard_lines(&self, width: usize) -> Vec<String> {
        self.dashboard_lines_for(width, None)
    }

    fn dashboard_lines_for(&self, width: usize, finished: Option<bool>) -> Vec<String> {
        let width = width.max(20);
        let active = self.active_count();
        let completed = self.completed_count();
        let failed = self.failed_count();
        let total_tokens = self.total_tokens();
        let spinner = SPINNER[self.frame % SPINNER.len()];
        let title = format!(" {} ", self.title);
        let model = format!(" {} ", self.model);
        let rule_width = width.saturating_sub(
            UnicodeWidthStr::width(title.as_str()) + UnicodeWidthStr::width(model.as_str()) + 2,
        );
        let mut lines = vec![render_segments(
            &[
                (BLUE, "─".to_string()),
                (CYAN, title),
                (BLUE, "─".to_string()),
                (MUTED, model),
                (BLUE, "─".repeat(rule_width)),
            ],
            width,
            self.color,
        )];

        let columns = (width + CELL_GAP.len())
            .checked_div(MIN_CELL_WIDTH + CELL_GAP.len())
            .unwrap_or(1)
            .clamp(1, MAX_GRID_COLUMNS);
        let cell_width = (width.saturating_sub(CELL_GAP.len() * (columns - 1))) / columns;
        let last_visible = self.slots.iter().rposition(Option::is_some);
        if let Some(last_visible) = last_visible {
            for row in self.slots[..=last_visible].chunks(columns) {
                let cells = row
                    .iter()
                    .map(|slot| {
                        slot.and_then(|id| {
                            self.requests
                                .get(&id)
                                .map(|state| self.request_cell(id, state, cell_width))
                        })
                        .unwrap_or_else(|| " ".repeat(cell_width))
                    })
                    .collect::<Vec<_>>()
                    .join(CELL_GAP);
                lines.push(cells);
            }
        } else {
            lines.push(render_segments(
                &[(MUTED, "  waiting for the first request…".to_string())],
                width,
                self.color,
            ));
        }

        if !self.recent_failures.is_empty() {
            let details = self
                .recent_failures
                .iter()
                .map(|(id, error)| format!("#{id:03} {}", compact_error(error, 48)))
                .collect::<Vec<_>>()
                .join("  ·  ");
            lines.push(render_segments(
                &[(RED, format!("  ✕ {details}"))],
                width,
                self.color,
            ));
        }

        let (status_icon, status_label, status_color) = match finished {
            Some(true) => ('✓', "Complete", GREEN),
            Some(false) => ('■', "Stopped", RED),
            None => (spinner, "Working…", GREEN),
        };
        let metrics = format!(
            "{active} active · {completed} done · {failed} errors · {} tok",
            format_count(total_tokens)
        );
        let left = format!(" {status_icon} {status_label} ");
        let divider_width = width.saturating_sub(
            UnicodeWidthStr::width(left.as_str()) + UnicodeWidthStr::width(metrics.as_str()) + 2,
        );
        lines.push(render_segments(
            &[
                (status_color, left),
                (BLUE, "─".repeat(divider_width)),
                (MUTED, format!("  {metrics}")),
            ],
            width,
            self.color,
        ));
        lines
    }

    fn request_cell(&self, id: usize, state: &RequestState, width: usize) -> String {
        let elapsed = state.started.elapsed().as_secs_f64();
        let bar_width = match width {
            0..=31 => 4,
            32..=43 => 6,
            44..=55 => 8,
            _ => 10,
        };
        let (icon, color, detail) = match state.status {
            RequestStatus::InFlight => (
                SPINNER[self.frame % SPINNER.len()],
                GREEN,
                if state.tokens == 0 {
                    "streaming".to_string()
                } else {
                    format!("~{} tok", format_count(state.tokens))
                },
            ),
            RequestStatus::Retrying(attempt) => ('↻', YELLOW, format!("retry {attempt}")),
            RequestStatus::Completed => ('✓', GREEN, format!("{} tok", format_count(state.tokens))),
            RequestStatus::Failed => ('✕', RED, "failed".to_string()),
        };
        let segments = vec![
            (BLUE, format!("{id:03}")),
            (MUTED, " [".to_string()),
            (color, progress_bar(state.status, self.frame, bar_width)),
            (MUTED, "] ".to_string()),
            (color, icon.to_string()),
            ("", format!(" {}", request_kind_label(state.kind))),
            (MUTED, format!(" · {detail} · {elapsed:.1}s")),
        ];
        render_segments(&segments, width, self.color)
    }

    fn active_count(&self) -> usize {
        self.requests
            .values()
            .filter(|state| {
                matches!(
                    state.status,
                    RequestStatus::InFlight | RequestStatus::Retrying(_)
                )
            })
            .count()
    }

    fn completed_count(&self) -> usize {
        self.requests
            .values()
            .filter(|state| state.status == RequestStatus::Completed)
            .count()
    }

    fn failed_count(&self) -> usize {
        self.requests
            .values()
            .filter(|state| state.status == RequestStatus::Failed)
            .count()
    }

    fn total_tokens(&self) -> usize {
        self.requests.values().map(|state| state.tokens).sum()
    }

    fn assign_slot(&mut self, id: usize) {
        if self.slots.contains(&Some(id)) {
            return;
        }
        if let Some(empty) = self.slots.iter_mut().find(|slot| slot.is_none()) {
            *empty = Some(id);
            return;
        }
        let reusable = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let visible_id = (*slot)?;
                let state = self.requests.get(&visible_id)?;
                (!matches!(
                    state.status,
                    RequestStatus::InFlight | RequestStatus::Retrying(_)
                ))
                .then_some((index, visible_id))
            })
            .min_by_key(|(_, visible_id)| *visible_id)
            .map(|(index, _)| index);
        if let Some(index) = reusable {
            self.slots[index] = Some(id);
        }
    }

    fn write_dashboard(&mut self, lines: &[String]) {
        let previous_lines = self.rendered_lines;
        let slots = previous_lines.max(lines.len());
        let mut stderr = io::stderr().lock();
        if !self.cursor_hidden {
            let _ = write!(stderr, "\x1b[?25l");
            self.cursor_hidden = true;
        }
        if previous_lines > 0 {
            let _ = write!(stderr, "\x1b[{previous_lines}A");
        }
        for index in 0..slots {
            let _ = write!(stderr, "\x1b[2K\r");
            if let Some(line) = lines.get(index) {
                let _ = write!(stderr, "{line}");
            }
            let _ = writeln!(stderr);
        }
        let _ = stderr.flush();
        self.rendered_lines = slots;
    }

    fn show_cursor(&mut self) {
        if self.cursor_hidden {
            let _ = write!(io::stderr().lock(), "\x1b[?25h");
            self.cursor_hidden = false;
        }
    }
}

impl Drop for LiveProgress {
    fn drop(&mut self) {
        if self.interactive {
            self.show_cursor();
        }
    }
}

fn terminal_width() -> usize {
    terminal_size()
        .map(|(Width(width), _)| usize::from(width))
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|columns| columns.parse().ok())
        })
        .unwrap_or(120)
}

fn progress_bar(status: RequestStatus, frame: usize, width: usize) -> String {
    match status {
        RequestStatus::Completed => "⣿".repeat(width),
        RequestStatus::Failed => "×".repeat(width),
        RequestStatus::Retrying(_) => (0..width)
            .map(|index| {
                if (index + frame / 2).is_multiple_of(2) {
                    '⣶'
                } else {
                    '·'
                }
            })
            .collect(),
        RequestStatus::InFlight => {
            let head = frame % (width + 3);
            (0..width)
                .map(|index| match head.checked_sub(index) {
                    Some(0) => '⣿',
                    Some(1) => '⣷',
                    Some(2) => '⣤',
                    _ => '·',
                })
                .collect()
        }
    }
}

/// Render independently styled pieces into an exact terminal-cell width.
/// ANSI sequences never participate in truncation or padding calculations.
fn render_segments(segments: &[(&str, String)], width: usize, color: bool) -> String {
    let mut rendered = String::new();
    let mut used = 0;
    for (style, text) in segments {
        if used >= width {
            break;
        }
        let (text, text_width) = truncate_to_width(text, width - used);
        if text.is_empty() {
            continue;
        }
        if color && !style.is_empty() {
            rendered.push_str(style);
        }
        rendered.push_str(&text);
        if color && !style.is_empty() {
            rendered.push_str(RESET);
        }
        used += text_width;
    }
    rendered.push_str(&" ".repeat(width.saturating_sub(used)));
    rendered
}

fn truncate_to_width(text: &str, max_width: usize) -> (String, usize) {
    let mut result = String::new();
    let mut width = 0;
    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > max_width {
            break;
        }
        result.push(character);
        width += character_width;
    }
    (result, width)
}

fn request_kind_label(kind: SummarizationRequestKind) -> String {
    match kind {
        SummarizationRequestKind::UserBatch { entries } => format!("user×{entries}"),
        SummarizationRequestKind::PublicStory => "public".to_string(),
    }
}

fn compact_error(error: &str, max_chars: usize) -> String {
    let one_line = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max_chars {
        return one_line;
    }
    let mut shortened = one_line
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    shortened.push('…');
    shortened
}

fn format_count(count: usize) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aissence::SummarizationRequestUsage;

    fn test_live_progress() -> LiveProgress {
        LiveProgress {
            interactive: false,
            color: false,
            frame: 0,
            rendered_lines: 0,
            cursor_hidden: false,
            title: "Aissence".to_string(),
            model: "vendor/model".to_string(),
            requests: BTreeMap::new(),
            slots: vec![None; MAX_VISIBLE_REQUESTS],
            recent_failures: VecDeque::new(),
        }
    }

    #[test]
    fn live_progress_replaces_estimates_with_exact_tokens_and_keeps_failures_visible() {
        let mut live = test_live_progress();
        live.handle(SummarizationProgress::Started {
            id: 1,
            kind: SummarizationRequestKind::UserBatch { entries: 4 },
        });
        live.handle(SummarizationProgress::TokensReceived {
            id: 1,
            tokens: 1234,
        });
        let lines = live.dashboard_lines(120);
        assert!(lines.last().unwrap().contains("1 active"));
        assert!(lines[1].contains("~1.2k tok"));

        live.handle(SummarizationProgress::Completed {
            id: 1,
            kind: SummarizationRequestKind::UserBatch { entries: 4 },
            usage: Some(SummarizationRequestUsage {
                prompt_tokens: 900,
                completion_tokens: 42,
                total_tokens: 942,
                cost: None,
            }),
        });
        live.handle(SummarizationProgress::Failed {
            id: 2,
            kind: SummarizationRequestKind::PublicStory,
            error: "429: provider rate limit".into(),
        });
        let lines = live.dashboard_lines(120);
        let footer = lines.last().unwrap();
        assert!(footer.contains("0 active"));
        assert!(footer.contains("1 done"));
        assert!(footer.contains("1 errors"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("#002 429: provider rate limit"))
        );
        assert_eq!(live.total_tokens(), 42);
    }

    #[test]
    fn live_progress_recycles_one_finished_slot_without_shifting_its_neighbors() {
        let mut live = test_live_progress();
        live.slots = vec![None; 2];
        for id in [1, 2] {
            live.handle(SummarizationProgress::Started {
                id,
                kind: SummarizationRequestKind::PublicStory,
            });
        }
        live.handle(SummarizationProgress::Completed {
            id: 1,
            kind: SummarizationRequestKind::PublicStory,
            usage: None,
        });
        live.handle(SummarizationProgress::Started {
            id: 3,
            kind: SummarizationRequestKind::PublicStory,
        });

        assert_eq!(live.slots, vec![Some(3), Some(2)]);
        let grid = &live.dashboard_lines(100)[1];
        assert!(grid.find("003").unwrap() < grid.find("002").unwrap());
    }

    #[test]
    fn dashboard_rows_fit_the_terminal_width_with_unicode_model_names() {
        let mut live = test_live_progress();
        live.model = "模型/model".to_string();
        live.handle(SummarizationProgress::Started {
            id: 1,
            kind: SummarizationRequestKind::UserBatch { entries: 12 },
        });

        for line in live.dashboard_lines(73) {
            assert_eq!(UnicodeWidthStr::width(line.as_str()), 73);
        }
    }
}
