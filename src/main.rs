//! Demo CLI for the `aissence` library.
//!
//! Parses a folder's commits and AI session logs, then prints — for each commit
//! — the chat messages that led up to it. Pass `--summarize` to additionally
//! generate LLM summaries (reads `OPENROUTER_API_KEY` / `OPENROUTER_MODEL`).
//!
//! ```text
//! cargo run -- /path/to/project
//! cargo run -- /path/to/project --summarize
//! ```

use std::path::PathBuf;

use aissence::ncs;
use aissence::{CommitGroup, Summarizer, TimedMessage, analyze};

#[tokio::main]
async fn main() {
    let mut folder = None;
    let mut summarize = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--summarize" | "-s" => summarize = true,
            _ if folder.is_none() => folder = Some(PathBuf::from(arg)),
            _ => {}
        }
    }
    let folder = folder
        .or_else(|| std::env::current_dir().ok())
        .expect("could not determine folder to analyze");

    let mut history = match analyze(&folder) {
        Ok(history) => history,
        Err(err) => {
            eprintln!("error analyzing {}: {err}", folder.display());
            std::process::exit(1);
        }
    };

    let mut did_summarize = false;
    if summarize {
        match Summarizer::from_env() {
            Ok(summarizer) => {
                eprintln!("Summarizing with model `{}` …", summarizer.model());
                match summarizer.summarize_history(&mut history).await {
                    Ok(()) => did_summarize = true,
                    Err(err) => eprintln!("summarization error: {err}"),
                }
            }
            Err(err) => eprintln!("cannot summarize: {err}"),
        }
    }

    println!("Analyzed {}", history.folder.display());
    println!(
        "{} commit group(s), {} message(s) placed, {} undated",
        history.groups.len(),
        history.message_count(),
        history.undated.len(),
    );
    for warning in &history.warnings {
        eprintln!("warning: {warning}");
    }
    println!();

    for group in &history.groups {
        print_group(group, did_summarize);
    }
}

fn print_group(group: &CommitGroup, summarized: bool) {
    match &group.commit {
        Some(commit) => {
            println!(
                "● {} {}  ({}, {})",
                commit.short_id,
                commit.summary,
                commit.author_name,
                commit.time.format("%Y-%m-%d %H:%M"),
            );
        }
        None => println!("● (uncommitted — work in progress)"),
    }

    if let Some(summary) = &group.summary {
        print_summary_line("user requested", &summary.user_requested);
        print_summary_line("ai answered", &summary.ai_answered);
        print_summary_line("ai implemented", &summary.ai_implemented);
    } else if summarized {
        println!("    (no summary — no chat messages)");
    } else {
        // Raw message view (no summarization requested).
        if group.messages.is_empty() {
            println!("    · no chat messages");
        }
        for message in &group.messages {
            print_message(message);
        }
    }
    println!();
}

fn print_summary_line(label: &str, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    println!("    {label}:");
    for line in textwrap(text, 92) {
        println!("      {line}");
    }
}

fn print_message(message: &TimedMessage) {
    let role = role_label(&message.role);
    let when = message
        .timestamp
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "        ".to_string());

    let snippet = first_line(&message.text(), 100);
    let tools = message.tool_uses();

    if !snippet.is_empty() {
        println!("    {when} {role:>9}: {snippet}");
    }
    if !tools.is_empty() {
        println!("    {when} {role:>9}: [tools: {}]", tools.join(", "));
    }
    if snippet.is_empty() && tools.is_empty() {
        println!("    {when} {role:>9}: …");
    }
}

fn role_label(role: &ncs::Role) -> &'static str {
    match role {
        ncs::Role::User => "user",
        ncs::Role::Assistant => "assistant",
        ncs::Role::System => "system",
        ncs::Role::Tool => "tool",
    }
}

fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() > max {
        let truncated: String = line.chars().take(max).collect();
        format!("{truncated}…")
    } else {
        line.to_string()
    }
}

/// Minimal greedy word-wrap for terminal output.
fn textwrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
