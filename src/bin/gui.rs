//! aissence — a 3-pane desktop browser for a project's AI coding sessions,
//! correlated with its git commits.
//!
//! Start with `--folder <path>` (or pick a folder via a dialog when omitted).
//! The app parses every session for the folder, summarizes each commit's
//! conversation with an LLM (showing progress), then presents:
//!
//! - **left**: commits, each with a one-line summary,
//! - **middle**: the per-message summaries for the selected commit
//!   (user wrote / ai wrote / ai changed),
//! - **right**: syntax-highlighted code changes — the whole commit diff, or the
//!   edits of a selected "ai changed" message.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aissence::{CoalescedHistory, DiffView, MessageKind, SummaryStore, Summarizer, analyze, git};
use eframe::egui;
use elegance::{Accent, Callout, CalloutTone, Card, ProgressBar, Spinner, Theme};

// Per-kind accent colors (One Dark family).
const USER_COLOR: egui::Color32 = egui::Color32::from_rgb(97, 175, 239); // blue
const AI_COLOR: egui::Color32 = egui::Color32::from_rgb(152, 195, 121); // green
const CHANGE_COLOR: egui::Color32 = egui::Color32::from_rgb(229, 192, 123); // yellow
const DIM: egui::Color32 = egui::Color32::from_gray(140);

fn main() -> eframe::Result {
    let folder = match folder_from_args() {
        Some(folder) => folder,
        None => match rfd::FileDialog::new()
            .set_title("Open a project folder")
            .pick_folder()
        {
            Some(folder) => folder,
            None => {
                eprintln!("No folder selected.");
                return Ok(());
            }
        },
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 840.0])
            .with_title("aissence"),
        ..Default::default()
    };
    eframe::run_native(
        "aissence",
        options,
        Box::new(move |cc| Ok(Box::new(AissenceApp::new(cc, folder)))),
    )
}

/// Parse `--folder <path>` / `--folder=<path>` from the CLI.
fn folder_from_args() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--folder" {
            return args.next().map(PathBuf::from);
        }
        if let Some(rest) = arg.strip_prefix("--folder=") {
            return Some(PathBuf::from(rest));
        }
    }
    None
}

/// Background-load lifecycle, shared between the worker task and the UI thread.
enum Phase {
    Loading {
        message: String,
        fraction: Option<f32>,
    },
    Failed(String),
    Ready {
        history: CoalescedHistory,
        notice: Option<String>,
    },
}

struct AissenceApp {
    folder: PathBuf,
    _runtime: tokio::runtime::Runtime,
    phase: Arc<Mutex<Phase>>,

    // Populated once loading finishes (moved out of `phase`).
    history: Option<CoalescedHistory>,
    notice: Option<String>,

    // Selection state.
    selected_group: Option<usize>,
    /// `None` = show the whole-commit diff; `Some(i)` = message `i`'s changes.
    selected_message: Option<usize>,

    // Right-pane patch, recomputed only when the selection changes.
    current_patch: Option<String>,
    last_selection: (Option<usize>, Option<usize>),
    /// Cache of whole-commit git diffs by group index.
    patch_cache: HashMap<usize, Result<String, String>>,
}

impl AissenceApp {
    fn new(cc: &eframe::CreationContext<'_>, folder: PathBuf) -> Self {
        Theme::slate().install(&cc.egui_ctx);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let phase = Arc::new(Mutex::new(Phase::Loading {
            message: "Starting…".to_string(),
            fraction: None,
        }));

        runtime.spawn(load(folder.clone(), phase.clone(), cc.egui_ctx.clone()));

        Self {
            folder,
            _runtime: runtime,
            phase,
            history: None,
            notice: None,
            selected_group: None,
            selected_message: None,
            current_patch: None,
            last_selection: (None, None),
            patch_cache: HashMap::new(),
        }
    }

    fn on_ready(&mut self, history: CoalescedHistory, notice: Option<String>) {
        // Default to the most recent group that actually has messages.
        self.selected_group = (0..history.groups.len())
            .rev()
            .find(|&i| !history.groups[i].messages.is_empty())
            .or(history.groups.len().checked_sub(1));
        self.selected_message = None;
        self.notice = notice;
        self.history = Some(history);
    }

    /// Recompute [`Self::current_patch`] when the selection changed.
    fn refresh_patch(&mut self, history: &CoalescedHistory) {
        let selection = (self.selected_group, self.selected_message);
        if selection == self.last_selection && self.current_patch.is_some() {
            return;
        }
        self.last_selection = selection;
        self.current_patch = self.compute_patch(history);
    }

    fn compute_patch(&mut self, history: &CoalescedHistory) -> Option<String> {
        let group_idx = self.selected_group?;
        let group = history.groups.get(group_idx)?;

        // A specific "ai changed" message → its synthesized edit diffs.
        if let Some(message_idx) = self.selected_message {
            let message = group.summary.as_ref()?.messages.get(message_idx)?;
            if message.changes.is_empty() {
                return None;
            }
            return Some(join_diffs(message.changes.iter().map(|c| c.diff.as_str())));
        }

        // Whole-commit view.
        match &group.commit {
            Some(commit) => {
                let commit_id = commit.id.clone();
                if !self.patch_cache.contains_key(&group_idx) {
                    let result =
                        git::commit_patch(&self.folder, &commit_id).map_err(|e| e.to_string());
                    self.patch_cache.insert(group_idx, result);
                }
                self.patch_cache
                    .get(&group_idx)
                    .and_then(|r| r.as_ref().ok().cloned())
            }
            // Pending group: combine all synthesized changes (no git commit yet).
            None => {
                let combined = group.summary.as_ref().map(|s| {
                    join_diffs(
                        s.messages
                            .iter()
                            .flat_map(|m| m.changes.iter())
                            .map(|c| c.diff.as_str()),
                    )
                })?;
                (!combined.is_empty()).then_some(combined)
            }
        }
    }
}

impl eframe::App for AissenceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.history.is_none() {
            // Peek the phase; consume it if loading finished.
            let snapshot = {
                let mut guard = self.phase.lock().unwrap();
                if matches!(&*guard, Phase::Ready { .. }) {
                    std::mem::replace(
                        &mut *guard,
                        Phase::Loading {
                            message: String::new(),
                            fraction: None,
                        },
                    )
                } else {
                    match &*guard {
                        Phase::Loading { message, fraction } => Phase::Loading {
                            message: message.clone(),
                            fraction: *fraction,
                        },
                        Phase::Failed(e) => Phase::Failed(e.clone()),
                        Phase::Ready { .. } => unreachable!(),
                    }
                }
            };

            match snapshot {
                Phase::Ready { history, notice } => self.on_ready(history, notice),
                Phase::Loading { message, fraction } => {
                    show_loading(ui, &message, fraction);
                    ui.ctx().request_repaint(); // animate the spinner
                    return;
                }
                Phase::Failed(error) => {
                    show_failed(ui, &error);
                    return;
                }
            }
        }

        self.show_main(ui);
    }
}

impl AissenceApp {
    fn show_main(&mut self, ui: &mut egui::Ui) {
        // Own the history for the frame so we can freely mutate selection state.
        let history = self.history.take().expect("history present");
        self.refresh_patch(&history);

        egui::Panel::top("topbar").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("aissence");
                ui.label(egui::RichText::new(self.folder.display().to_string()).color(DIM));
            });
            if let Some(notice) = &self.notice {
                Callout::new(CalloutTone::Warning)
                    .body(notice.clone())
                    .show(ui, |_| {});
            }
            ui.add_space(2.0);
        });

        egui::Panel::left("commits")
            .resizable(true)
            .default_size(300.0)
            .size_range(egui::Rangef::new(220.0, 460.0))
            .show_inside(ui, |ui| self.commits_pane(ui, &history));

        egui::Panel::left("messages")
            .resizable(true)
            .default_size(400.0)
            .size_range(egui::Rangef::new(280.0, 640.0))
            .show_inside(ui, |ui| self.messages_pane(ui, &history));

        // Central panel must be added last.
        egui::CentralPanel::default().show_inside(ui, |ui| self.changes_pane(ui));

        self.history = Some(history);
    }

    fn commits_pane(&mut self, ui: &mut egui::Ui, history: &CoalescedHistory) {
        ui.add_space(6.0);
        ui.heading("Commits");
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, group) in history.groups.iter().enumerate() {
                    let (headline, sub) = match &group.commit {
                        Some(commit) => (
                            commit.summary.clone(),
                            format!("{}  ·  {} msg", commit.short_id, group.messages.len()),
                        ),
                        None => (
                            "Uncommitted work".to_string(),
                            format!("pending  ·  {} msg", group.messages.len()),
                        ),
                    };

                    let selected = self.selected_group == Some(i);
                    let mut job = egui::text::LayoutJob::default();
                    job.append(
                        &headline,
                        0.0,
                        egui::TextFormat {
                            color: ui.visuals().text_color(),
                            ..Default::default()
                        },
                    );
                    job.append(
                        &format!("\n{sub}"),
                        0.0,
                        egui::TextFormat {
                            color: DIM,
                            font_id: egui::FontId::proportional(11.0),
                            ..Default::default()
                        },
                    );

                    if ui.selectable_label(selected, job).clicked() {
                        self.selected_group = Some(i);
                        self.selected_message = None;
                    }
                    ui.add_space(2.0);
                }
            });
    }

    fn messages_pane(&mut self, ui: &mut egui::Ui, history: &CoalescedHistory) {
        ui.add_space(6.0);
        ui.heading("Messages");
        ui.add_space(4.0);

        let Some(group_idx) = self.selected_group else {
            ui.label(egui::RichText::new("Select a commit on the left.").color(DIM));
            return;
        };
        let Some(group) = history.groups.get(group_idx) else {
            return;
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Commit header.
                if let Some(commit) = &group.commit {
                    Card::new().show(ui, |ui| {
                        ui.strong(&commit.summary);
                        ui.label(
                            egui::RichText::new(format!(
                                "{} · {} · {}",
                                commit.short_id,
                                commit.author_name,
                                commit.time.format("%Y-%m-%d %H:%M")
                            ))
                            .color(DIM)
                            .size(11.0),
                        );
                    });
                }

                // "All changes" selector → whole-commit diff on the right.
                ui.add_space(4.0);
                if ui
                    .selectable_label(self.selected_message.is_none(), "📦  All changes in this commit")
                    .clicked()
                {
                    self.selected_message = None;
                }
                ui.separator();

                match &group.summary {
                    Some(summary) => {
                        if summary.messages.is_empty() {
                            ui.label(
                                egui::RichText::new("No conversation summarized.").color(DIM),
                            );
                        }
                        for (i, message) in summary.messages.iter().enumerate() {
                            self.message_row(ui, i, message);
                        }
                    }
                    None => raw_messages(ui, group),
                }
            });
    }

    fn message_row(&mut self, ui: &mut egui::Ui, index: usize, message: &aissence::MessageSummary) {
        let (color, prefix) = match message.kind {
            MessageKind::UserWrote => (USER_COLOR, "▸"),
            MessageKind::AiWrote => (AI_COLOR, "▸"),
            MessageKind::AiChanged => (CHANGE_COLOR, "✎"),
        };

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(prefix).color(color).strong());
            ui.label(egui::RichText::new(message.kind.label()).color(color).strong());
        });

        if message.kind == MessageKind::AiChanged {
            // Selectable: clicking shows this message's edits on the right.
            let files: Vec<&str> = message.changes.iter().map(|c| c.file.as_str()).collect();
            let label = if files.is_empty() {
                message.summary.clone()
            } else {
                format!("{}\n— {}", message.summary, files.join(", "))
            };
            if ui
                .selectable_label(self.selected_message == Some(index), label)
                .clicked()
            {
                self.selected_message = Some(index);
            }
        } else {
            ui.label(&message.summary);
        }
    }

    fn changes_pane(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Changes");
        ui.add_space(4.0);

        match &self.current_patch {
            Some(patch) => DiffView::from_patch(patch).show(ui),
            None => {
                let hint = match (self.selected_group, self.selected_message) {
                    (None, _) => "Select a commit to see its changes.",
                    (Some(_), Some(_)) => "This message made no code changes.",
                    (Some(_), None) => "No code changes for this commit.",
                };
                ui.label(egui::RichText::new(hint).color(DIM));
            }
        }
    }
}

/// Join several unified-diff strings into one patch.
fn join_diffs<'a>(diffs: impl Iterator<Item = &'a str>) -> String {
    diffs.collect::<Vec<_>>().join("\n")
}

/// Fallback message list when no LLM summary is available (e.g. no API key).
fn raw_messages(ui: &mut egui::Ui, group: &aissence::CommitGroup) {
    ui.label(
        egui::RichText::new("No AI summary — showing raw messages.")
            .italics()
            .color(DIM),
    );
    for message in &group.messages {
        let (color, who) = match message.role {
            aissence::ncs::Role::User => (USER_COLOR, "user"),
            aissence::ncs::Role::Assistant => (AI_COLOR, "assistant"),
            aissence::ncs::Role::Tool => (DIM, "tool"),
            aissence::ncs::Role::System => (DIM, "system"),
        };
        let text = message.text();
        let snippet: String = text.lines().next().unwrap_or("").chars().take(140).collect();
        if snippet.trim().is_empty() {
            continue;
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(who).color(color).strong());
            ui.label(snippet);
        });
    }
}

fn show_loading(ui: &mut egui::Ui, message: &str, fraction: Option<f32>) {
    ui.add_space(120.0);
    ui.vertical_centered(|ui| {
        ui.add(Spinner::new().size(40.0));
        ui.add_space(16.0);
        ui.heading("Preparing your sessions");
        ui.add_space(6.0);
        ui.label(egui::RichText::new(message).color(DIM));
        if let Some(fraction) = fraction {
            ui.add_space(12.0);
            ui.add(
                ProgressBar::new(fraction.clamp(0.0, 1.0))
                    .desired_width(320.0)
                    .accent(Accent::Blue)
                    .text(format!("{:.0}%", fraction * 100.0)),
            );
        }
    });
}

fn show_failed(ui: &mut egui::Ui, error: &str) {
    ui.add_space(80.0);
    Callout::new(CalloutTone::Danger)
        .title("Could not load this folder")
        .body(error.to_string())
        .multiline()
        .show(ui, |_| {});
}

/// The background pipeline: analyze, then summarize each commit with progress.
async fn load(folder: PathBuf, phase: Arc<Mutex<Phase>>, ctx: egui::Context) {
    let set = |phase: &Mutex<Phase>, p: Phase| *phase.lock().unwrap() = p;

    set(
        &phase,
        Phase::Loading {
            message: "Parsing sessions and commits…".to_string(),
            fraction: None,
        },
    );
    ctx.request_repaint();

    let folder_for_analyze = folder.clone();
    let analyzed = tokio::task::spawn_blocking(move || analyze(&folder_for_analyze)).await;
    let mut history = match analyzed {
        Ok(Ok(history)) => history,
        Ok(Err(err)) => {
            set(&phase, Phase::Failed(format!("Analysis failed: {err}")));
            ctx.request_repaint();
            return;
        }
        Err(err) => {
            set(&phase, Phase::Failed(format!("Worker crashed: {err}")));
            ctx.request_repaint();
            return;
        }
    };

    // Resume: apply any summaries cached in `<folder>/.aissence`. This works
    // even without an API key, so prior runs' work is shown immediately.
    let mut store = SummaryStore::load(&folder);
    for group in history.groups.iter_mut() {
        let Some(commit_id) = group.commit.as_ref().map(|c| c.id.clone()) else {
            continue; // never cache the live (uncommitted) group
        };
        let signature = aissence::persist::signature(group);
        if let Some(cached) = store.get(&commit_id, signature) {
            group.summary = Some(cached.clone());
        }
    }

    let mut notice = None;
    match Summarizer::from_env() {
        Ok(summarizer) => {
            // Commits still needing a summary (skip empty + already-cached).
            let pending: Vec<usize> = history
                .groups
                .iter()
                .enumerate()
                .filter(|(_, g)| !g.messages.is_empty() && g.summary.is_none())
                .map(|(i, _)| i)
                .collect();
            let already = history
                .groups
                .iter()
                .filter(|g| !g.messages.is_empty() && g.summary.is_some())
                .count();
            let total = (pending.len() + already).max(1);

            for (offset, &idx) in pending.iter().enumerate() {
                let done = already + offset;
                set(
                    &phase,
                    Phase::Loading {
                        message: format!("Summarizing commit {}/{} …", done + 1, total),
                        fraction: Some(done as f32 / total as f32),
                    },
                );
                ctx.request_repaint();

                match summarizer.summarize_group(&history.groups[idx]).await {
                    Ok(summary) => {
                        history.groups[idx].summary = Some(summary.clone());
                        // Persist immediately so a quit resumes from here.
                        if let Some(commit_id) =
                            history.groups[idx].commit.as_ref().map(|c| c.id.clone())
                        {
                            let signature = aissence::persist::signature(&history.groups[idx]);
                            if let Err(err) = store.insert_and_save(&commit_id, signature, summary) {
                                notice = Some(format!("Could not write .aissence cache: {err}"));
                            }
                        }
                    }
                    Err(err) => notice = Some(format!("Some summaries failed: {err}")),
                }
            }
        }
        Err(err) => {
            notice = Some(if store.is_empty() {
                format!("AI summaries unavailable ({err}). Showing commits and diffs only.")
            } else {
                format!("AI summaries unavailable ({err}); showing cached summaries.")
            });
        }
    }

    *phase.lock().unwrap() = Phase::Ready { history, notice };
    ctx.request_repaint();
}
