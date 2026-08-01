//! aissence — a 3-pane desktop browser for a project's AI coding sessions,
//! correlated with its git commits.
//!
//! Start with `--folder <path>` (or pick a folder via a dialog when omitted).
//! The app parses every session for the folder and shows cached summaries
//! immediately. Summarization is **on demand**: selecting a commit offers
//! *Summarize* and *Ignore* buttons. Panes:
//!
//! - **left**: commits, each with a one-line summary (ignored ones faded),
//! - **middle**: Summarize/Ignore buttons + summarized user prompts and local
//!   previews for assistant text/changes,
//! - **right** (toggleable, hidden by default): syntax-highlighted code changes
//!   — the whole commit diff, or the edits of a selected "ai changed" message.
//!
//! Generated summaries, git metadata, and ignore flags persist to
//! `<folder>/.aissence`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use aissence::{
    CoalescedHistory, CommitGroup, CommitSummary, DiffView, MAX_CONCURRENT_GROUPS, MessageKind,
    SummarizationCost, Summarizer, SummaryStore, TokenCount, analyze, git, summarize,
};
use eframe::egui;
use elegance::{Accent, Button, Callout, CalloutTone, Card, Spinner, Theme};

// Per-kind accent colors (One Dark family).
const USER_COLOR: egui::Color32 = egui::Color32::from_rgb(97, 175, 239); // blue
const AI_COLOR: egui::Color32 = egui::Color32::from_rgb(152, 195, 121); // green
const CHANGE_COLOR: egui::Color32 = egui::Color32::from_rgb(229, 192, 123); // yellow
const DIM: egui::Color32 = egui::Color32::from_gray(140);
const DISABLED: egui::Color32 = egui::Color32::from_gray(90); // faded / ignored

fn main() -> eframe::Result {
    let meta_summaries = meta_summaries_from_args();
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
        Box::new(move |cc| Ok(Box::new(AissenceApp::new(cc, folder, meta_summaries)))),
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

fn meta_summaries_from_args() -> bool {
    std::env::args().any(|arg| arg == "--meta-summaries")
}

/// Parse lifecycle, shared between the worker task and the UI thread.
enum Phase {
    Loading(String),
    Failed(String),
    Ready(CoalescedHistory),
}

/// Result of one background summarization, sent back to the UI thread.
struct SummarizeResult {
    group_index: usize,
    batch_id: Option<u64>,
    cost: SummarizationCost,
    outcome: Result<CommitSummary, String>,
}

#[derive(Clone, Copy)]
struct QueuedSummary {
    group_index: usize,
    batch_id: u64,
}

struct SummaryBatch {
    id: u64,
    total_groups: usize,
    remaining_groups: usize,
    failed_groups: usize,
    cost: SummarizationCost,
}

struct AissenceApp {
    folder: PathBuf,
    runtime: tokio::runtime::Runtime,
    phase: Arc<Mutex<Phase>>,
    /// `None` when no valid local LLM configuration is available.
    summarizer: Option<Arc<Summarizer>>,
    meta_summaries_enabled: bool,
    /// On-disk cache of summaries, git metadata, and ignore flags.
    store: SummaryStore,

    // Populated once parsing finishes (moved out of `phase`).
    history: Option<CoalescedHistory>,
    notice: Option<String>,

    // Selection state.
    selected_group: Option<usize>,
    /// `None` = show the whole-commit diff; `Some(i)` = message `i`'s changes.
    selected_message: Option<usize>,
    /// Whether the optional right-hand git/code changes pane is visible.
    show_changes: bool,

    // Right-pane patch, recomputed only when the selection changes.
    current_patch: Option<String>,
    last_selection: (Option<usize>, Option<usize>),
    /// Cache of whole-commit git diffs by group index.
    patch_cache: HashMap<usize, Result<String, String>>,

    // On-demand summarization.
    in_progress: HashSet<usize>,
    /// Group indices queued by "Summarize all", pumped with bounded concurrency.
    queue: VecDeque<QueuedSummary>,
    next_batch_id: u64,
    active_batch: Option<SummaryBatch>,
    batch_report: Option<(String, CalloutTone)>,
    results_tx: Sender<SummarizeResult>,
    results_rx: Receiver<SummarizeResult>,
}

impl AissenceApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        folder: PathBuf,
        meta_summaries_enabled: bool,
    ) -> Self {
        Theme::slate().install(&cc.egui_ctx);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let phase = Arc::new(Mutex::new(Phase::Loading("Starting…".to_string())));

        // Parsing runs in the background; the store + summarizer live on the UI
        // thread (summaries are kicked off on demand).
        runtime.spawn(load(folder.clone(), phase.clone(), cc.egui_ctx.clone()));
        let store = SummaryStore::load(&folder);
        let summarizer = Summarizer::from_env()
            .ok()
            .map(|summarizer| summarizer.with_meta_summaries(meta_summaries_enabled))
            .map(Arc::new);
        let notice = summarizer.is_none().then(|| {
            "Configure an OpenRouter or OpenAI-compatible API to enable AI summaries.".to_string()
        });

        let (results_tx, results_rx) = std::sync::mpsc::channel();

        Self {
            folder,
            runtime,
            phase,
            summarizer,
            meta_summaries_enabled,
            store,
            history: None,
            notice,
            selected_group: None,
            selected_message: None,
            show_changes: false,
            current_patch: None,
            last_selection: (None, None),
            patch_cache: HashMap::new(),
            in_progress: HashSet::new(),
            queue: VecDeque::new(),
            next_batch_id: 1,
            active_batch: None,
            batch_report: None,
            results_tx,
            results_rx,
        }
    }

    fn on_ready(&mut self, mut history: CoalescedHistory) {
        // Apply summaries whose stable source digest still matches, then write
        // every committed source group so the file is uploadable even before
        // local summarization has run.
        for group in history.groups.iter_mut() {
            if let Some(cached) = self.store.get_for_group(group) {
                let summary = cached.clone();
                if !summarize::has_required_user_summaries(group, &summary) {
                    continue;
                }
                if !summarize::has_required_core_summaries(group, &summary) {
                    continue;
                }
                if self.meta_summaries_enabled
                    && (!summarize::has_required_extended_summaries(group, &summary)
                        || !summarize::has_required_meta_summary(&summary))
                {
                    continue;
                }
                group.summary = Some(summary);
            }
        }
        if let Err(err) = self.store.sync_groups(&history.groups) {
            self.notice = Some(format!("Could not write .aissence: {err}"));
        }

        // Default to the most recent group that actually has messages.
        self.selected_group = (0..history.groups.len())
            .rev()
            .find(|&i| !history.groups[i].messages.is_empty())
            .or(history.groups.len().checked_sub(1));
        self.selected_message = None;
        self.history = Some(history);
    }

    /// Drain finished summarizations, applying them to `history` and persisting.
    fn drain_results(&mut self, history: &mut CoalescedHistory) {
        while let Ok(result) = self.results_rx.try_recv() {
            self.in_progress.remove(&result.group_index);
            let failed = result.outcome.is_err();
            match result.outcome {
                Ok(summary) => {
                    if let Some(group) = history.groups.get_mut(result.group_index) {
                        group.summary = Some(summary.clone());
                        if let Err(err) = self.store.insert_and_save(group, summary) {
                            self.notice = Some(format!("Could not write .aissence: {err}"));
                        }
                    }
                }
                Err(err) => self.notice = Some(format!("Summarization failed: {err}")),
            }
            if let Some(batch_id) = result.batch_id {
                self.finish_batch_item(batch_id, result.cost, failed);
            }
        }
    }

    /// Spawn summarization for one group (also un-ignores it).
    fn start_summarize(
        &mut self,
        group_index: usize,
        group: &CommitGroup,
        batch_id: Option<u64>,
        ctx: egui::Context,
    ) {
        let Some(summarizer) = self.summarizer.clone() else {
            return;
        };
        if self.in_progress.contains(&group_index) {
            return;
        }

        let commit_id = group.commit.as_ref().map(|c| c.id.clone());
        if let Some(id) = &commit_id
            && let Err(err) = self.store.set_ignored(id, false)
        {
            self.notice = Some(format!("Could not write .aissence: {err}"));
        }

        let group = group.clone();
        let tx = self.results_tx.clone();
        self.in_progress.insert(group_index);
        self.runtime.spawn(async move {
            let (outcome, cost) = summarizer.summarize_group_with_cost(&group).await;
            let _ = tx.send(SummarizeResult {
                group_index,
                batch_id,
                cost,
                outcome: outcome.map_err(|e| e.to_string()),
            });
            ctx.request_repaint();
        });
    }

    fn finish_batch_item(&mut self, batch_id: u64, cost: SummarizationCost, failed: bool) {
        let Some(batch) = self
            .active_batch
            .as_mut()
            .filter(|batch| batch.id == batch_id)
        else {
            return;
        };
        batch.cost.add(cost);
        batch.remaining_groups = batch.remaining_groups.saturating_sub(1);
        batch.failed_groups += usize::from(failed);
        if batch.remaining_groups != 0 {
            return;
        }

        let batch = self.active_batch.take().expect("completed batch");
        self.batch_report = Some(batch_report(&batch));
    }

    /// Mark a commit ignored / un-ignored.
    fn set_commit_ignored(&mut self, group: &CommitGroup, ignored: bool) {
        if let Some(commit) = &group.commit
            && let Err(err) = self.store.set_ignored(&commit.id, ignored)
        {
            self.notice = Some(format!("Could not write .aissence: {err}"));
        }
    }

    /// Whether group `idx` can be summarized now: it has messages, isn't already
    /// summarized or running, and isn't ignored.
    fn is_summarizable(&self, idx: usize, group: &CommitGroup) -> bool {
        !group.messages.is_empty()
            && group.summary.is_none()
            && !self.in_progress.contains(&idx)
            && group
                .commit
                .as_ref()
                .is_none_or(|c| !self.store.is_ignored(&c.id))
    }

    /// Groups "Summarize all" would still enqueue (summarizable + not queued).
    fn pending_for_all(&self, history: &CoalescedHistory) -> Vec<usize> {
        history
            .groups
            .iter()
            .enumerate()
            .filter(|(i, g)| {
                self.is_summarizable(*i, g)
                    && !self.queue.iter().any(|queued| queued.group_index == *i)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Start summarizing queued commits, up to [`MAX_CONCURRENT_GROUPS`].
    fn pump_queue(&mut self, history: &CoalescedHistory, ctx: &egui::Context) {
        while self.in_progress.len() < MAX_CONCURRENT_GROUPS {
            let Some(queued) = self.queue.pop_front() else {
                break;
            };
            if let Some(group) = history.groups.get(queued.group_index)
                && self.is_summarizable(queued.group_index, group)
            {
                self.start_summarize(
                    queued.group_index,
                    group,
                    Some(queued.batch_id),
                    ctx.clone(),
                );
            } else {
                self.finish_batch_item(queued.batch_id, SummarizationCost::default(), false);
            }
        }
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
                if matches!(&*guard, Phase::Ready(_)) {
                    std::mem::replace(&mut *guard, Phase::Loading(String::new()))
                } else {
                    match &*guard {
                        Phase::Loading(message) => Phase::Loading(message.clone()),
                        Phase::Failed(e) => Phase::Failed(e.clone()),
                        Phase::Ready(_) => unreachable!(),
                    }
                }
            };

            match snapshot {
                Phase::Ready(history) => self.on_ready(history),
                Phase::Loading(message) => {
                    show_loading(ui, &message);
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
        let ctx = ui.ctx().clone();
        // Own the history for the frame so we can freely mutate selection state.
        let mut history = self.history.take().expect("history present");
        self.drain_results(&mut history);
        self.pump_queue(&history, &ctx);

        egui::Panel::top("topbar").show_inside(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("aissence");
                ui.label(egui::RichText::new(self.folder.display().to_string()).color(DIM));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let pending = self.pending_for_all(&history);
                    let enabled = self.summarizer.is_some()
                        && self.active_batch.is_none()
                        && !pending.is_empty();
                    let button = ui.add_enabled(
                        enabled,
                        Button::new(format!("Summarize all ({})", pending.len()))
                            .accent(Accent::Blue),
                    );
                    if button.clicked() {
                        let batch_id = self.next_batch_id;
                        self.next_batch_id = self.next_batch_id.wrapping_add(1);
                        self.active_batch = Some(SummaryBatch {
                            id: batch_id,
                            total_groups: pending.len(),
                            remaining_groups: pending.len(),
                            failed_groups: 0,
                            cost: SummarizationCost::default(),
                        });
                        self.batch_report = None;
                        self.queue
                            .extend(pending.into_iter().map(|group_index| QueuedSummary {
                                group_index,
                                batch_id,
                            }));
                    }
                    if self.summarizer.is_none() {
                        button.on_disabled_hover_text("Configure a local LLM API");
                    }
                    let active = self.in_progress.len();
                    let queued = self.queue.len();
                    let outstanding = queued + active;
                    if outstanding > 0 {
                        ui.add(Spinner::new().size(16.0));
                        ui.label(
                            egui::RichText::new(format!("{active} active · {queued} queued"))
                                .color(DIM),
                        );
                    }
                    let changes_label = if self.show_changes {
                        "Hide changes"
                    } else {
                        "Show changes"
                    };
                    if ui.add(Button::new(changes_label)).clicked() {
                        self.show_changes = !self.show_changes;
                    }
                });
            });
            if let Some(notice) = &self.notice {
                Callout::new(CalloutTone::Warning)
                    .body(notice.clone())
                    .show(ui, |_| {});
            }
            if let Some((report, tone)) = &self.batch_report {
                Callout::new(*tone).body(report.clone()).show(ui, |_| {});
            }
            ui.add_space(2.0);
        });

        egui::Panel::left("commits")
            .resizable(true)
            .default_size(300.0)
            .size_range(egui::Rangef::new(220.0, 460.0))
            .show_inside(ui, |ui| self.commits_pane(ui, &history));

        if self.show_changes {
            self.refresh_patch(&history);
            egui::Panel::left("messages")
                .resizable(true)
                .default_size(400.0)
                .size_range(egui::Rangef::new(280.0, 640.0))
                .show_inside(ui, |ui| self.messages_pane(ui, &history));

            // Central panel must be added last.
            egui::CentralPanel::default().show_inside(ui, |ui| self.changes_pane(ui));
        } else {
            // With changes hidden, let the messages view use all remaining
            // space instead of leaving an empty third pane.
            egui::CentralPanel::default().show_inside(ui, |ui| self.messages_pane(ui, &history));
        }

        // Keep animating / pumping while any commit is queued or summarizing.
        if !self.in_progress.is_empty() || !self.queue.is_empty() {
            ui.ctx().request_repaint();
        }

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
                    let ignored = group
                        .commit
                        .as_ref()
                        .is_some_and(|c| self.store.is_ignored(&c.id));
                    let summarized = group.summary.is_some();

                    let (headline, mut sub) = match &group.commit {
                        Some(commit) => {
                            let mut sub = format!(
                                "{}  ·  {}",
                                commit.short_id,
                                prompt_count_label(group.prompt_count())
                            );
                            if ignored {
                                sub.push_str("  ·  ignored");
                            } else if summarized {
                                sub.push_str("  ·  ✓");
                            }
                            (commit.summary.clone(), sub)
                        }
                        None => (
                            "Uncommitted work".to_string(),
                            format!("pending  ·  {}", prompt_count_label(group.prompt_count())),
                        ),
                    };
                    // Token counts (from session data — shown even before summarizing).
                    if let Some(tokens) = group.token_usage().filter(|t| !t.is_zero()) {
                        sub.push_str(&format!(
                            "  ·  ↑{} ↓{}",
                            fmt_count(tokens.input),
                            fmt_count(tokens.output)
                        ));
                    }

                    // Ignored commits render in a faded (disabled) color.
                    let headline_color = if ignored {
                        DISABLED
                    } else {
                        ui.visuals().text_color()
                    };
                    let selected = self.selected_group == Some(i);
                    let mut job = egui::text::LayoutJob::default();
                    job.append(
                        &headline,
                        0.0,
                        egui::TextFormat {
                            color: headline_color,
                            ..Default::default()
                        },
                    );
                    job.append(
                        &format!("\n{sub}"),
                        0.0,
                        egui::TextFormat {
                            color: if ignored { DISABLED } else { DIM },
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
        let ignored = group
            .commit
            .as_ref()
            .is_some_and(|c| self.store.is_ignored(&c.id));
        let busy = self.in_progress.contains(&group_idx);

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
                        // Provenance: which agent(s) + model(s) produced this commit.
                        let agents = aissence::agents_for(group);
                        if !agents.is_empty() {
                            let text = agents
                                .iter()
                                .map(|a| a.label())
                                .collect::<Vec<_>>()
                                .join(",   ");
                            ui.label(egui::RichText::new(text).color(USER_COLOR).size(11.0));
                        }
                        // Token usage for the whole conversation.
                        if let Some(tokens) = group.token_usage().filter(|t| !t.is_zero()) {
                            ui.label(
                                egui::RichText::new(token_detail(&tokens))
                                    .color(DIM)
                                    .size(11.0),
                            );
                        }
                    });
                }

                // Action row: Summarize / Ignore (or a spinner while running).
                ui.add_space(6.0);
                if busy {
                    ui.horizontal(|ui| {
                        ui.add(Spinner::new().size(18.0));
                        ui.label("Summarizing…");
                    });
                } else {
                    ui.horizontal(|ui| {
                        let can_summarize = self.summarizer.is_some() && !group.messages.is_empty();
                        let label = if group.summary.is_some() {
                            "Re-summarize"
                        } else {
                            "Summarize"
                        };
                        let summarize =
                            ui.add_enabled(can_summarize, Button::new(label).accent(Accent::Blue));
                        if summarize.clicked() {
                            self.start_summarize(group_idx, group, None, ui.ctx().clone());
                        }
                        if self.summarizer.is_none() {
                            summarize.on_disabled_hover_text("Configure a local LLM API");
                        }
                        if group.commit.is_some() {
                            let ignore_label = if ignored { "Un-ignore" } else { "Ignore" };
                            if ui
                                .add(Button::new(ignore_label).accent(Accent::Amber))
                                .clicked()
                            {
                                self.set_commit_ignored(group, !ignored);
                            }
                        }
                    });
                }

                // "All changes" selector → whole-commit diff on the right.
                ui.add_space(6.0);
                if ui
                    .selectable_label(self.selected_message.is_none(), "📦  All changes in this commit")
                    .clicked()
                {
                    self.selected_message = None;
                }
                ui.separator();

                if ignored {
                    ui.label(
                        egui::RichText::new(
                            "Ignored — not summarized. Click Summarize to include it.",
                        )
                        .italics()
                        .color(DISABLED),
                    );
                } else if let Some(summary) = &group.summary {
                    // Synthesized overview of the whole commit.
                    if !summary.meta.trim().is_empty() {
                        Card::new().heading("Overview").show(ui, |ui| {
                            ui.label(&summary.meta);
                        });
                        ui.add_space(8.0);
                    }
                    if summary.messages.is_empty() {
                        ui.label(egui::RichText::new("No conversation summarized.").color(DIM));
                    }
                    for (i, message) in summary.messages.iter().enumerate() {
                        self.message_row(ui, i, message);
                    }
                } else {
                    ui.label(
                        egui::RichText::new(
                            "Not summarized yet — click Summarize, or browse the raw messages below.",
                        )
                        .color(DIM),
                    );
                    ui.add_space(4.0);
                    raw_messages(ui, group);
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
            ui.label(
                egui::RichText::new(message.kind.label())
                    .color(color)
                    .strong(),
            );
            // Per-message token counts (turn-level, approximate).
            if let Some(tokens) = message.tokens.filter(|t| !t.is_zero()) {
                ui.label(
                    egui::RichText::new(format!(
                        "↑{} ↓{}",
                        fmt_count(tokens.input),
                        fmt_count(tokens.output)
                    ))
                    .color(DIM)
                    .size(11.0),
                );
            }
        });

        if message.kind == MessageKind::AiChanged {
            // Selectable: clicking shows this message's edits on the right.
            let mut seen = HashSet::new();
            let files: Vec<&str> = message
                .changes
                .iter()
                .map(|change| change.file.as_str())
                .filter(|file| seen.insert(*file))
                .collect();
            let activity = match files.len() {
                0 => "Editing activity".to_string(),
                1 => "Changed 1 file".to_string(),
                count => format!("Changed {count} files"),
            };
            let label = if files.is_empty() {
                activity
            } else {
                format!("{activity}\n— {}", files.join(", "))
            };
            if ui
                .selectable_label(self.selected_message == Some(index), label)
                .clicked()
            {
                self.selected_message = Some(index);
            }
        } else if message.kind == MessageKind::UserWrote {
            if let Some(summary) = message.summary.as_deref() {
                ui.label(summary);
            } else {
                let preview = message
                    .content
                    .as_deref()
                    .map(message_preview)
                    .unwrap_or_else(|| "Source text unavailable.".to_string());
                ui.label(preview);
            }
        } else {
            let preview = message
                .content
                .as_deref()
                .map(message_preview)
                .unwrap_or_else(|| "Source text unavailable.".to_string());
            ui.label(preview);
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

/// Compact token count: `950`, `12.3k`, `1.4M`.
fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Short, local preview for retained user and assistant source text.
fn message_preview(content: &str) -> String {
    const MAX_CHARS: usize = 320;
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }
    let mut preview: String = normalized.chars().take(MAX_CHARS).collect();
    preview.push('…');
    preview
}

fn batch_report(batch: &SummaryBatch) -> (String, CalloutTone) {
    let groups = if batch.total_groups == 1 {
        "1 commit".to_string()
    } else {
        format!("{} commits", batch.total_groups)
    };
    let requests = if batch.cost.request_count == 1 {
        "1 API request".to_string()
    } else {
        format!("{} API requests", batch.cost.request_count)
    };

    let mut report = format!("Summarize all finished for {groups}. ");
    if batch.cost.request_count == 0 {
        report.push_str("No API requests were needed; cost $0.");
    } else if batch.cost.unpriced_request_count == 0 {
        report.push_str(&format!("{requests} cost {}.", fmt_usd(batch.cost.usd)));
    } else if batch.cost.unpriced_request_count == batch.cost.request_count {
        report.push_str(&format!(
            "Cost unavailable: none of the {requests} returned billing data."
        ));
    } else {
        report.push_str(&format!(
            "Reported cost is {} across {requests}; {} did not return billing data, so the actual total may be higher.",
            fmt_usd(batch.cost.usd),
            batch.cost.unpriced_request_count
        ));
    }
    if batch.failed_groups > 0 {
        report.push_str(&format!(
            " {} failed before completing.",
            if batch.failed_groups == 1 {
                "1 commit".to_string()
            } else {
                format!("{} commits", batch.failed_groups)
            }
        ));
    }

    let tone = if batch.failed_groups > 0 {
        CalloutTone::Warning
    } else if batch.cost.unpriced_request_count > 0 {
        CalloutTone::Info
    } else {
        CalloutTone::Success
    };
    (report, tone)
}

fn fmt_usd(cost: f64) -> String {
    if cost >= 1.0 {
        format!("${cost:.4}")
    } else if cost >= 0.01 {
        format!("${cost:.6}")
    } else {
        format!("${cost:.8}")
    }
}

fn prompt_count_label(count: usize) -> String {
    if count == 1 {
        "1 prompt".to_string()
    } else {
        format!("{count} prompts")
    }
}

/// One-line token breakdown for a commit header, e.g.
/// `↑ 12.3k in · ↓ 4.1k out · cache 8.0k r / 1.2k w`.
fn token_detail(tokens: &TokenCount) -> String {
    let mut text = format!(
        "↑ {} in  ·  ↓ {} out",
        fmt_count(tokens.input),
        fmt_count(tokens.output)
    );
    let cache_read = tokens.cache_read.unwrap_or(0);
    let cache_create = tokens.cache_create.unwrap_or(0);
    if cache_read > 0 || cache_create > 0 {
        text.push_str(&format!(
            "  ·  cache {} r / {} w",
            fmt_count(cache_read),
            fmt_count(cache_create)
        ));
    }
    text
}

/// Raw message preview (role + first line), shown before a commit is summarized.
fn raw_messages(ui: &mut egui::Ui, group: &CommitGroup) {
    for message in &group.messages {
        let (color, who) = match message.role {
            aissence::ncs::Role::User => (USER_COLOR, "user"),
            aissence::ncs::Role::Assistant => (AI_COLOR, "assistant"),
            aissence::ncs::Role::Tool => (DIM, "tool"),
            aissence::ncs::Role::System => (DIM, "system"),
        };
        let text = message.text();
        let snippet: String = text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(140)
            .collect();
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

fn show_loading(ui: &mut egui::Ui, message: &str) {
    ui.add_space(120.0);
    ui.vertical_centered(|ui| {
        ui.add(Spinner::new().size(40.0));
        ui.add_space(16.0);
        ui.heading("Preparing your sessions");
        ui.add_space(6.0);
        ui.label(egui::RichText::new(message).color(DIM));
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

/// Background step: parse the folder's sessions + commits. Summarization is
/// kicked off on demand from the UI; cached summaries are applied in `on_ready`.
async fn load(folder: PathBuf, phase: Arc<Mutex<Phase>>, ctx: egui::Context) {
    *phase.lock().unwrap() = Phase::Loading("Parsing sessions and commits…".to_string());
    ctx.request_repaint();

    let analyzed = tokio::task::spawn_blocking(move || analyze(&folder)).await;
    *phase.lock().unwrap() = match analyzed {
        Ok(Ok(history)) => Phase::Ready(history),
        Ok(Err(err)) => Phase::Failed(format!("Analysis failed: {err}")),
        Err(err) => Phase::Failed(format!("Worker crashed: {err}")),
    };
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_report_shows_the_exact_reported_cost() {
        let batch = SummaryBatch {
            id: 1,
            total_groups: 2,
            remaining_groups: 0,
            failed_groups: 0,
            cost: SummarizationCost {
                usd: 0.001234,
                request_count: 7,
                unpriced_request_count: 0,
            },
        };

        let (report, _) = batch_report(&batch);
        assert_eq!(
            report,
            "Summarize all finished for 2 commits. 7 API requests cost $0.00123400."
        );
    }

    #[test]
    fn batch_report_warns_when_some_requests_have_no_price() {
        let batch = SummaryBatch {
            id: 1,
            total_groups: 1,
            remaining_groups: 0,
            failed_groups: 0,
            cost: SummarizationCost {
                usd: 0.02,
                request_count: 3,
                unpriced_request_count: 1,
            },
        };

        let (report, _) = batch_report(&batch);
        assert!(report.contains("Reported cost is $0.020000"));
        assert!(report.contains("actual total may be higher"));
    }

    #[test]
    fn message_preview_is_local_normalized_and_bounded() {
        assert_eq!(message_preview("first\n\nsecond"), "first second");

        let long = "x".repeat(321);
        let preview = message_preview(&long);
        assert_eq!(preview.chars().count(), 321);
        assert!(preview.ends_with('…'));
    }
}
