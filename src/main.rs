//! Demo CLI for the `aissence` library.
//!
//! Parses a folder's commits and AI session logs, then prints — for each commit
//! — the chat messages that led up to it. This exists to exercise the library
//! ahead of the egui front-end; run it with:
//!
//! ```text
//! cargo run -- /path/to/project
//! ```

use std::path::PathBuf;

use aissence::ncs;
use aissence::{CommitGroup, TimedMessage, analyze};

fn main() {
    let folder = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .expect("could not determine folder to analyze");

    let history = match analyze(&folder) {
        Ok(history) => history,
        Err(err) => {
            eprintln!("error analyzing {}: {err}", folder.display());
            std::process::exit(1);
        }
    };

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
        print_group(group);
    }
}

fn print_group(group: &CommitGroup) {
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

    if group.messages.is_empty() {
        println!("    · no chat messages");
    }
    for message in &group.messages {
        print_message(message);
    }
    println!();
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
