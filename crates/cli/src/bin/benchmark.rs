//! Compare an OpenRouter model by summarizing one complete project history.
//!
//! Outputs `<model>.aissence` and `<model>-stats.md` in the directory from
//! which the command is launched, independently of the analyzed source path.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use aissence::{
    CoalescedHistory, CommitSummary, SummarizationCost, Summarizer, SummaryStore, analyze,
};
use tokio::sync::mpsc;

#[path = "../progress.rs"]
mod progress;

use progress::LiveProgress;

const USAGE: &str = "\
Usage:
  aissence-benchmark --model <MODEL> --source-folder <FOLDER>

Environment:
  OPENROUTER_API_KEY    Required OpenRouter API key
  OPENROUTER_PROVIDER   Optional preferred provider
";

#[derive(Debug, PartialEq, Eq)]
struct Args {
    model: String,
    source_folder: PathBuf,
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
        .map_err(|error| error.to_string())?;

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
    let mut live_progress = LiveProgress::new("Aissence benchmark", &args.model);
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
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
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

    let summaries = history
        .groups
        .iter()
        .filter_map(|group| group.summary.as_ref().map(|summary| (group, summary)))
        .collect::<Vec<_>>();
    let fallback_count = summaries
        .iter()
        .filter(|(_, summary)| summary.public.origin != aissence::SummaryOrigin::Model)
        .count();
    let zero_beat_count = summaries
        .iter()
        .filter(|(_, summary)| summary.public.beats.is_empty())
        .count();
    writeln!(markdown, "\n## Quality signals").unwrap();
    writeln!(
        markdown,
        "\n- Metadata/privacy/parse fallbacks: {fallback_count}"
    )
    .unwrap();
    writeln!(
        markdown,
        "- Stories with zero representative beats: {zero_beat_count}"
    )
    .unwrap();
    writeln!(
        markdown,
        "- Summary policy version: {}",
        aissence::SUMMARIZER_VERSION
    )
    .unwrap();

    append_sample(&mut markdown, "First", find_prompt_sample(history, false));
    append_sample(&mut markdown, "Last", find_prompt_sample(history, true));
    writeln!(markdown, "\n## Complete summary review").unwrap();
    for (group, summary) in summaries {
        let label = group
            .commit
            .as_ref()
            .map(|commit| format!("{} — {}", commit.short_id, commit.summary))
            .unwrap_or_else(|| "uncommitted work in progress".to_string());
        writeln!(markdown, "\n### `{}`", escape_inline_code(&label)).unwrap();
        writeln!(markdown, "\n- Origin: `{:?}`", summary.public.origin).unwrap();
        append_quote(&mut markdown, &summary.public.summary);
        for beat in &summary.public.beats {
            writeln!(markdown, "\n- **{}:** {}", beat.label, beat.text).unwrap();
        }
    }
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
        if !summary.public.summary.trim().is_empty() {
            let group_label = group
                .commit
                .as_ref()
                .map(|commit| format!("{} — {}", commit.short_id, commit.summary))
                .unwrap_or_else(|| "uncommitted work in progress".to_string());
            return Some(PromptSample {
                prompt: &summary.public.summary,
                group: summary,
                group_label,
            });
        }
    }
    None
}

fn append_sample(markdown: &mut String, position: &str, sample: Option<PromptSample<'_>>) {
    writeln!(markdown, "\n## {position} public story").unwrap();
    let Some(sample) = sample else {
        writeln!(markdown, "\n_No public story was available._").unwrap();
        return;
    };

    writeln!(
        markdown,
        "\nGroup: `{}`",
        escape_inline_code(&sample.group_label)
    )
    .unwrap();
    append_quote(markdown, sample.prompt);

    if !sample.group.public.beats.is_empty() {
        writeln!(markdown, "\n### Representative beats").unwrap();
        for beat in &sample.group.public.beats {
            writeln!(markdown, "\n- **{}:** {}", beat.label, beat.text).unwrap();
        }
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
            }
        );
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
            ..SummarizationCost::default()
        };
        assert!(markdown_cost(&cost).contains("actual cost may be higher"));
        assert!(markdown_cost(&cost).contains("$0.00125000"));
    }

    #[test]
    fn stats_include_public_story_and_beats() {
        let summary = CommitSummary {
            public: aissence::PublicStory {
                subline: String::new(),
                summary: "Complete public overview".into(),
                beats: vec![aissence::PublicBeat {
                    message_index: 0,
                    label: "Initial request".into(),
                    text: "A privacy-safe request paraphrase.".into(),
                }],
                origin: aissence::SummaryOrigin::Model,
            },
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

        assert!(stats.contains("> Complete public overview"));
        assert!(stats.contains("**Initial request:** A privacy-safe request paraphrase."));
    }
}
