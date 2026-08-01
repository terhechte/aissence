//! Compare an OpenRouter model by summarizing one complete project history.
//!
//! Outputs `<model>.aissence` and `<model>-stats.md` in the directory from
//! which the command is launched, independently of the analyzed source path.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use aissence::{
    CoalescedHistory, CommitSummary, MessageKind, SummarizationCost, SummarizationProgress,
    SummarizationRequestKind, Summarizer, SummaryStore, analyze,
};
use tokio::sync::mpsc;

const USAGE: &str = "\
Usage:
  aissence-benchmark --model <MODEL> --source-folder <FOLDER> [--meta-summaries]

Options:
  --meta-summaries     Also generate User requested, AI implemented, and Overview

Environment:
  OPENROUTER_API_KEY    Required OpenRouter API key
  OPENROUTER_PROVIDER   Optional preferred provider
";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    model: String,
    source_folder: PathBuf,
    meta_summaries: bool,
}

const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[38;5;81m";
const GREEN: &str = "\x1b[38;5;114m";
const YELLOW: &str = "\x1b[38;5;221m";
const RED: &str = "\x1b[38;5;203m";
const DIM: &str = "\x1b[2m";

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

/// A single animated dashboard for the requests emitted by the summarizer.
///
/// The dashboard is deliberately terminal-only. Piped/non-interactive runs
/// retain the benchmark's normal machine-readable output and never receive
/// cursor-control sequences.
struct LiveProgress {
    interactive: bool,
    frame: usize,
    rendered_lines: usize,
    requests: BTreeMap<usize, RequestState>,
    recent_failures: VecDeque<(usize, String)>,
}

impl LiveProgress {
    fn new() -> Self {
        Self {
            interactive: io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
            frame: 0,
            rendered_lines: 0,
            requests: BTreeMap::new(),
            recent_failures: VecDeque::new(),
        }
    }

    fn handle(&mut self, event: SummarizationProgress) {
        match event {
            SummarizationProgress::Started { id, kind } => {
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
                if let Some(state) = self.requests.get_mut(&id) {
                    state.kind = kind;
                    state.status = RequestStatus::Completed;
                    if let Some(usage) = usage {
                        state.tokens = usage.completion_tokens as usize;
                    }
                }
            }
            SummarizationProgress::Failed { id, kind, error } => {
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

    fn render(&mut self) {
        if !self.interactive {
            return;
        }

        let lines = self.dashboard_lines();
        self.write_dashboard(&lines);
        self.frame = self.frame.wrapping_add(1);
    }

    fn finish(&mut self, success: bool) {
        if !self.interactive {
            return;
        }

        let completed = self.completed_count();
        let failed = self.failed_count();
        let total_tokens = self.total_tokens();
        let (icon, label, color) = if success {
            ('✓', "benchmark complete", GREEN)
        } else {
            ('■', "benchmark stopped", RED)
        };
        let line = format!(
            "{color}{icon}{RESET} {label} · {completed} requests · {} output tokens · {failed} errors",
            format_count(total_tokens)
        );
        self.write_dashboard(&[line]);
        self.rendered_lines = 0;
    }

    fn dashboard_lines(&self) -> Vec<String> {
        let active: Vec<_> = self
            .requests
            .iter()
            .filter(|(_, state)| {
                matches!(
                    state.status,
                    RequestStatus::InFlight | RequestStatus::Retrying(_)
                )
            })
            .collect();
        let completed = self.completed_count();
        let failed = self.failed_count();
        let total_tokens = self.total_tokens();
        let spinner = SPINNER[self.frame % SPINNER.len()];

        let mut lines = vec![format!(
            "{CYAN}{spinner}{RESET} summarizing · {} active · {completed} done · {failed} errors · {} tokens received",
            active.len(),
            format_count(total_tokens)
        )];

        if active.is_empty() {
            lines.push(format!("  {DIM}waiting for the next request…{RESET}"));
        } else {
            for row in active.chunks(4) {
                let cells = row
                    .iter()
                    .map(|(id, state)| self.request_cell(**id, state))
                    .collect::<Vec<_>>()
                    .join("   ");
                lines.push(format!("  {cells}"));
            }
        }

        let failures = if self.recent_failures.is_empty() {
            format!("  {DIM}recent errors: none{RESET}")
        } else {
            let details = self
                .recent_failures
                .iter()
                .map(|(id, error)| format!("{RED}✖{RESET} #{id:03} {}", compact_error(error, 48)))
                .collect::<Vec<_>>()
                .join("   ");
            format!("  recent errors: {details}")
        };
        lines.push(failures);
        lines
    }

    fn request_cell(&self, id: usize, state: &RequestState) -> String {
        let elapsed = state.started.elapsed().as_secs_f64();
        let token_label = format!("~{}tok", format_count(state.tokens));
        let (icon, status) = match state.status {
            RequestStatus::InFlight => ('▸', "live".to_string()),
            RequestStatus::Retrying(attempt) => ('↻', format!("retry {attempt}")),
            RequestStatus::Completed | RequestStatus::Failed => unreachable!(),
        };
        format!(
            "{YELLOW}{icon}{RESET} #{id:03} {} {token_label} {DIM}{elapsed:.1}s {status}{RESET}",
            request_kind_label(state.kind)
        )
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

    fn write_dashboard(&mut self, lines: &[String]) {
        let previous_lines = self.rendered_lines;
        let slots = previous_lines.max(lines.len());
        let mut stderr = io::stderr().lock();
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
}

fn request_kind_label(kind: SummarizationRequestKind) -> String {
    match kind {
        SummarizationRequestKind::UserBatch { entries } => format!("user×{entries}"),
        SummarizationRequestKind::CoreOutcome => "core".to_string(),
        SummarizationRequestKind::MetaSummary => "meta".to_string(),
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

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let args = match parse_args(std::env::args().skip(1))? {
        Some(args) => args,
        None => {
            print!("{USAGE}");
            return Ok(());
        }
    };
    let output_folder = std::env::current_dir()
        .map_err(|error| format!("cannot determine current folder: {error}"))?;
    let file_label = model_file_label(&args.model);
    let cache_path = output_folder.join(format!("{file_label}.aissence"));
    let stats_path = output_folder.join(format!("{file_label}-stats.md"));

    eprintln!("Analyzing {} …", args.source_folder.display());
    let mut history = analyze(&args.source_folder).map_err(|error| error.to_string())?;
    for warning in &history.warnings {
        eprintln!("warning: {warning}");
    }

    let api_key = std::env::var(aissence::summarize::ENV_API_KEY).map_err(|_| {
        format!(
            "missing environment variable: {}",
            aissence::summarize::ENV_API_KEY
        )
    })?;
    let provider = std::env::var(aissence::summarize::ENV_PROVIDER)
        .ok()
        .map(|provider| provider.trim().to_string())
        .filter(|provider| !provider.is_empty());
    let summarizer = Summarizer::new_with_provider(api_key, &args.model, provider)
        .map_err(|error| error.to_string())?
        .with_meta_summaries(args.meta_summaries);

    eprintln!(
        "Summarizing {} group(s) and {} prompt(s) with `{}` …",
        history
            .groups
            .iter()
            .filter(|group| !group.messages.is_empty())
            .count(),
        history.prompt_count(),
        args.model
    );
    let started = Instant::now();
    let (progress_sender, mut progress_receiver) = mpsc::unbounded_channel();
    let summarizer = summarizer.with_progress(move |event| {
        let _ = progress_sender.send(event);
    });
    let mut live_progress = LiveProgress::new();
    let mut tick = tokio::time::interval(Duration::from_millis(80));
    let (outcome, cost) = {
        let summarize = summarizer.summarize_history_with_cost(&mut history);
        tokio::pin!(summarize);
        loop {
            tokio::select! {
                result = &mut summarize => {
                    while let Ok(event) = progress_receiver.try_recv() {
                        live_progress.handle(event);
                    }
                    live_progress.render();
                    break result;
                }
                event = progress_receiver.recv() => {
                    if let Some(event) = event {
                        live_progress.handle(event);
                    }
                }
                _ = tick.tick() => live_progress.render(),
            }
        }
    };
    let elapsed = started.elapsed();
    live_progress.finish(outcome.is_ok());
    if let Err(error) = outcome {
        return Err(format!(
            "summarization failed after {} ({} incurred): {error}",
            format_duration(elapsed),
            format_cost(&cost)
        ));
    }

    let mut store = SummaryStore::empty_at_for_folder(&cache_path, &args.source_folder);
    store
        .replace_from_groups(&history.groups)
        .map_err(|error| format!("cannot write {}: {error}", cache_path.display()))?;

    let stats = stats_markdown(&args.model, &args.source_folder, elapsed, cost, &history);
    write_atomic(&stats_path, &stats)
        .map_err(|error| format!("cannot write {}: {error}", stats_path.display()))?;

    println!("Wrote {}", cache_path.display());
    println!("Wrote {}", stats_path.display());
    println!(
        "Summarization took {} and {}.",
        format_duration(elapsed),
        format_cost(&cost)
    );
    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let mut model = None;
    let mut source_folder = None;
    let mut meta_summaries = false;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--meta-summaries" => meta_summaries = true,
            "--model" | "-m" => {
                model = Some(
                    args.next()
                        .ok_or_else(|| format!("missing value for {arg}\n\n{USAGE}"))?,
                );
            }
            "--source-folder" | "-s" => {
                source_folder =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        format!("missing value for {arg}\n\n{USAGE}")
                    })?));
            }
            _ if arg.starts_with("--model=") => {
                model = Some(arg["--model=".len()..].to_string());
            }
            _ if arg.starts_with("--source-folder=") => {
                source_folder = Some(PathBuf::from(&arg["--source-folder=".len()..]));
            }
            _ => return Err(format!("unknown argument: {arg}\n\n{USAGE}")),
        }
    }

    let model = model
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| format!("--model is required\n\n{USAGE}"))?;
    let source_folder =
        source_folder.ok_or_else(|| format!("--source-folder is required\n\n{USAGE}"))?;
    Ok(Some(Args {
        model,
        source_folder,
        meta_summaries,
    }))
}

fn model_file_label(model: &str) -> String {
    let mut label = String::with_capacity(model.len());
    let mut last_was_separator = false;
    for character in model.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            label.push(character);
            last_was_separator = false;
        } else if !last_was_separator {
            label.push('-');
            last_was_separator = true;
        }
    }
    let label = label.trim_matches(['.', '_', '-']);
    if label.is_empty() {
        "model".to_string()
    } else {
        label.to_string()
    }
}

fn stats_markdown(
    model: &str,
    source_folder: &Path,
    elapsed: Duration,
    cost: SummarizationCost,
    history: &CoalescedHistory,
) -> String {
    let summarized_groups = history
        .groups
        .iter()
        .filter(|group| group.summary.is_some())
        .count();
    let mut markdown = String::new();
    writeln!(markdown, "# Model benchmark").unwrap();
    writeln!(markdown).unwrap();
    writeln!(markdown, "- Model: `{}`", escape_inline_code(model)).unwrap();
    writeln!(
        markdown,
        "- Source folder: `{}`",
        escape_inline_code(&source_folder.display().to_string())
    )
    .unwrap();
    writeln!(
        markdown,
        "- Summarization time: {}",
        format_duration(elapsed)
    )
    .unwrap();
    writeln!(markdown, "- {}", markdown_cost(&cost)).unwrap();
    writeln!(markdown, "- API requests: {}", cost.request_count).unwrap();
    writeln!(markdown, "- Summarized groups: {summarized_groups}").unwrap();
    writeln!(markdown, "- User prompts: {}", history.prompt_count()).unwrap();

    append_sample(&mut markdown, "First", find_prompt_sample(history, false));
    append_sample(&mut markdown, "Last", find_prompt_sample(history, true));
    markdown
}

struct PromptSample<'a> {
    prompt: &'a str,
    group: &'a CommitSummary,
    group_label: String,
}

fn find_prompt_sample(history: &CoalescedHistory, reverse: bool) -> Option<PromptSample<'_>> {
    let groups: Box<dyn Iterator<Item = _>> = if reverse {
        Box::new(history.groups.iter().rev())
    } else {
        Box::new(history.groups.iter())
    };
    for group in groups {
        let Some(summary) = &group.summary else {
            continue;
        };
        let prompt = if reverse {
            summary.messages.iter().rev().find_map(prompt_summary)
        } else {
            summary.messages.iter().find_map(prompt_summary)
        };
        if let Some(prompt) = prompt {
            let group_label = group
                .commit
                .as_ref()
                .map(|commit| format!("{} — {}", commit.short_id, commit.summary))
                .unwrap_or_else(|| "uncommitted work in progress".to_string());
            return Some(PromptSample {
                prompt,
                group: summary,
                group_label,
            });
        }
    }
    None
}

fn prompt_summary(message: &aissence::MessageSummary) -> Option<&str> {
    (message.kind == MessageKind::UserWrote)
        .then_some(message.summary.as_deref())
        .flatten()
        .filter(|summary| !summary.trim().is_empty())
}

fn append_sample(markdown: &mut String, position: &str, sample: Option<PromptSample<'_>>) {
    writeln!(markdown, "\n## {position} prompt summary").unwrap();
    let Some(sample) = sample else {
        writeln!(markdown, "\n_No summarized user prompt was available._").unwrap();
        return;
    };

    writeln!(
        markdown,
        "\nGroup: `{}`",
        escape_inline_code(&sample.group_label)
    )
    .unwrap();
    append_quote(markdown, sample.prompt);

    writeln!(markdown, "\n### {position} aggregate summaries").unwrap();
    if !sample.group.user_requested.trim().is_empty() {
        append_named_summary(markdown, "User requested", &sample.group.user_requested);
    }
    append_named_summary(markdown, "AI answered", &sample.group.ai_answered);
    if !sample.group.ai_implemented.trim().is_empty() {
        append_named_summary(markdown, "AI implemented", &sample.group.ai_implemented);
    }
    if !sample.group.meta.trim().is_empty() {
        append_named_summary(markdown, "Overview", &sample.group.meta);
    }
}

fn append_named_summary(markdown: &mut String, heading: &str, text: &str) {
    writeln!(markdown, "\n#### {heading}").unwrap();
    if text.trim().is_empty() {
        writeln!(markdown, "\n_None._").unwrap();
    } else {
        append_quote(markdown, text);
    }
}

fn append_quote(markdown: &mut String, text: &str) {
    writeln!(markdown).unwrap();
    for line in text.trim().lines() {
        writeln!(markdown, "> {line}").unwrap();
    }
}

fn format_duration(duration: Duration) -> String {
    format!("{:.3} seconds", duration.as_secs_f64())
}

fn format_cost(cost: &SummarizationCost) -> String {
    if cost.unpriced_request_count == 0 {
        format!("cost ${:.8} USD", cost.usd)
    } else {
        format!(
            "reported cost ${:.8} USD ({} of {} requests unpriced)",
            cost.usd, cost.unpriced_request_count, cost.request_count
        )
    }
}

fn markdown_cost(cost: &SummarizationCost) -> String {
    if cost.unpriced_request_count == 0 {
        format!("Cost: ${:.8} USD", cost.usd)
    } else {
        format!(
            "Reported cost: ${:.8} USD ({} of {} requests returned no billing data; actual cost may be higher)",
            cost.usd, cost.unpriced_request_count, cost.request_count
        )
    }
}

fn escape_inline_code(text: &str) -> String {
    text.replace('`', "\\`")
}

fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("aissence-stats.md");
    let temporary = path.with_file_name(format!("{file_name}.tmp"));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aissence::SummarizationRequestUsage;

    fn test_live_progress() -> LiveProgress {
        LiveProgress {
            interactive: false,
            frame: 0,
            rendered_lines: 0,
            requests: BTreeMap::new(),
            recent_failures: VecDeque::new(),
        }
    }

    #[test]
    fn parses_named_parameters() {
        let args = parse_args(
            ["--model", "vendor/model", "--source-folder", "../project"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            args,
            Args {
                model: "vendor/model".to_string(),
                source_folder: PathBuf::from("../project"),
                meta_summaries: false,
            }
        );
    }

    #[test]
    fn meta_summaries_are_opt_in() {
        let args = parse_args(
            [
                "--model",
                "vendor/model",
                "--source-folder",
                "../project",
                "--meta-summaries",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap()
        .unwrap();
        assert!(args.meta_summaries);
    }

    #[test]
    fn makes_openrouter_model_ids_safe_for_one_file_name() {
        assert_eq!(
            model_file_label("inclusionai/ling-2.6-flash"),
            "inclusionai-ling-2.6-flash"
        );
        assert_eq!(model_file_label("../../"), "model");
    }

    #[test]
    fn reports_unpriced_requests_without_claiming_the_total_is_exact() {
        let cost = SummarizationCost {
            usd: 0.00125,
            request_count: 3,
            unpriced_request_count: 1,
        };
        assert!(markdown_cost(&cost).contains("actual cost may be higher"));
        assert!(markdown_cost(&cost).contains("$0.00125000"));
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
        assert!(live.dashboard_lines()[0].contains("1 active"));
        assert!(live.dashboard_lines()[1].contains("~1.2ktok"));

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
            kind: SummarizationRequestKind::CoreOutcome,
            error: "429: provider rate limit".into(),
        });
        let lines = live.dashboard_lines();
        assert!(lines[0].contains("0 active"));
        assert!(lines[0].contains("1 done"));
        assert!(lines[0].contains("1 errors"));
        assert!(lines[2].contains("#002 429: provider rate limit"));
        assert_eq!(live.total_tokens(), 42);
    }

    #[test]
    fn stats_include_first_and_last_prompt_and_every_group_summary() {
        let summary = CommitSummary {
            user_requested: "Aggregate request".into(),
            ai_answered: "Aggregate answer".into(),
            ai_implemented: "Aggregate implementation".into(),
            meta: "Complete overview".into(),
            messages: vec![
                aissence::MessageSummary {
                    kind: MessageKind::UserWrote,
                    summary: Some("First prompt".into()),
                    content: None,
                    changes: vec![],
                    timestamp: None,
                    tokens: None,
                },
                aissence::MessageSummary {
                    kind: MessageKind::UserWrote,
                    summary: Some("Last prompt".into()),
                    content: None,
                    changes: vec![],
                    timestamp: None,
                    tokens: None,
                },
            ],
            ..Default::default()
        };
        let history = CoalescedHistory {
            folder: PathBuf::from("/source"),
            groups: vec![aissence::CommitGroup {
                commit: None,
                messages: vec![],
                summary: Some(summary),
            }],
            undated: vec![],
            warnings: vec![],
        };

        let stats = stats_markdown(
            "vendor/model",
            Path::new("/source"),
            Duration::from_millis(1250),
            SummarizationCost::default(),
            &history,
        );

        assert!(stats.contains("> First prompt"));
        assert!(stats.contains("> Last prompt"));
        assert!(stats.contains("> Aggregate request"));
        assert!(stats.contains("> Aggregate answer"));
        assert!(stats.contains("> Aggregate implementation"));
        assert!(stats.contains("> Complete overview"));
    }

    #[test]
    fn stats_omit_disabled_extended_summary_sections() {
        let summary = CommitSummary {
            ai_answered: "Aggregate answer".into(),
            messages: vec![aissence::MessageSummary {
                kind: MessageKind::UserWrote,
                summary: Some("Prompt summary".into()),
                content: None,
                changes: vec![],
                timestamp: None,
                tokens: None,
            }],
            ..Default::default()
        };
        let history = CoalescedHistory {
            folder: PathBuf::from("/source"),
            groups: vec![aissence::CommitGroup {
                commit: None,
                messages: vec![],
                summary: Some(summary),
            }],
            undated: vec![],
            warnings: vec![],
        };

        let stats = stats_markdown(
            "vendor/model",
            Path::new("/source"),
            Duration::ZERO,
            SummarizationCost::default(),
            &history,
        );

        assert!(stats.contains("#### AI answered"));
        assert!(!stats.contains("#### User requested"));
        assert!(!stats.contains("#### AI implemented"));
        assert!(!stats.contains("#### Overview"));
    }
}
