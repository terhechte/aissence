//! Unified-diff parsing and the [`DiffView`] egui widget.
//!
//! [`parse_unified_diff`] turns a git patch (the output of `git show` /
//! `git diff`) into a structured [`Diff`]. [`DiffView`] renders it with +/-
//! gutters, old/new line numbers, and syntax-highlighted content.
//!
//! Highlighting note: each hunk is highlighted by reconstructing its *old* and
//! *new* side as contiguous text and running the syntect highlighter over each,
//! so multi-line constructs colorize correctly within a hunk. (Context spanning
//! across hunk boundaries is necessarily approximate — the full file isn't part
//! of a patch.)

use egui::{Color32, FontId, Margin, RichText, ScrollArea};
use syntect::parsing::SyntaxReference;

use super::code_view::digit_width;
use super::layout_styled;
use crate::highlight::{self, StyledLine};

// One Dark inspired diff palette.
const ADD_BG: Color32 = Color32::from_rgb(19, 48, 32);
const ADD_FG: Color32 = Color32::from_rgb(152, 195, 121);
const REMOVE_BG: Color32 = Color32::from_rgb(58, 26, 30);
const REMOVE_FG: Color32 = Color32::from_rgb(224, 108, 117);
const HUNK_BG: Color32 = Color32::from_rgb(30, 36, 46);
const HUNK_FG: Color32 = Color32::from_rgb(86, 182, 194);
const FILE_FG: Color32 = Color32::from_rgb(229, 192, 123);
const GUTTER_FG: Color32 = Color32::from_gray(110);

/// Whether a diff line was unchanged, added, or removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Add,
    Remove,
}

/// A single line within a hunk (marker stripped from `content`).
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    /// 1-based line number on the old side (`None` for additions).
    pub old_no: Option<usize>,
    /// 1-based line number on the new side (`None` for removals).
    pub new_no: Option<usize>,
}

/// One `@@ … @@` hunk.
#[derive(Debug, Clone)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// All hunks for a single file, plus its paths and any extra header lines.
#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub hunks: Vec<Hunk>,
    pub is_binary: bool,
    /// Informational header lines (`index …`, `new file mode …`, `rename …`).
    pub extra_headers: Vec<String>,
}

impl FileDiff {
    /// The path to use for syntax detection and display (prefers the new path,
    /// ignoring `/dev/null`).
    pub fn display_path(&self) -> &str {
        match (self.new_path.as_deref(), self.old_path.as_deref()) {
            (Some(n), _) if n != "/dev/null" && !n.is_empty() => n,
            (_, Some(o)) if o != "/dev/null" && !o.is_empty() => o,
            _ => "(unknown)",
        }
    }

    fn max_line_no(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter_map(|l| l.old_no.max(l.new_no))
            .max()
            .unwrap_or(1)
    }
}

/// A parsed unified diff: one entry per file.
#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub files: Vec<FileDiff>,
}

/// Parse unified-diff / `git show` text into a [`Diff`]. Tolerant of both
/// `git`-style (`diff --git`, `a/`…`b/` prefixes) and plain `diff -u` output.
pub fn parse_unified_diff(text: &str) -> Diff {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut current: Option<FileDiff> = None;
    let mut old_cur = 0usize;
    let mut new_cur = 0usize;

    for raw in text.lines() {
        if raw.starts_with("diff --git") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            let mut file = FileDiff::default();
            file.extra_headers.push(raw.to_string());
            current = Some(file);
            continue;
        }

        if let Some(rest) = raw.strip_prefix("--- ") {
            // A `---` after hunks (or with no current file) starts a new file.
            if current.as_ref().is_none_or(|f| !f.hunks.is_empty()) {
                if let Some(file) = current.take() {
                    files.push(file);
                }
                current = Some(FileDiff::default());
            }
            current.get_or_insert_with(FileDiff::default).old_path = Some(clean_path(rest));
            continue;
        }
        if let Some(rest) = raw.strip_prefix("+++ ") {
            current.get_or_insert_with(FileDiff::default).new_path = Some(clean_path(rest));
            continue;
        }

        if raw.starts_with("@@") {
            let file = current.get_or_insert_with(FileDiff::default);
            let (old_start, new_start) = parse_hunk_header(raw);
            old_cur = old_start;
            new_cur = new_start;
            file.hunks.push(Hunk {
                header: raw.to_string(),
                lines: Vec::new(),
            });
            continue;
        }

        let Some(file) = current.as_mut() else {
            continue; // preamble before any file
        };

        // Content lines only make sense inside a hunk.
        if let Some(hunk) = file.hunks.last_mut() {
            if let Some(content) = raw.strip_prefix('+') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Add,
                    content: content.to_string(),
                    old_no: None,
                    new_no: Some(new_cur),
                });
                new_cur += 1;
                continue;
            } else if let Some(content) = raw.strip_prefix('-') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Remove,
                    content: content.to_string(),
                    old_no: Some(old_cur),
                    new_no: None,
                });
                old_cur += 1;
                continue;
            } else if let Some(content) = raw.strip_prefix(' ') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Context,
                    content: content.to_string(),
                    old_no: Some(old_cur),
                    new_no: Some(new_cur),
                });
                old_cur += 1;
                new_cur += 1;
                continue;
            } else if raw.starts_with('\\') {
                continue; // "\ No newline at end of file"
            } else if raw.is_empty() {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Context,
                    content: String::new(),
                    old_no: Some(old_cur),
                    new_no: Some(new_cur),
                });
                old_cur += 1;
                new_cur += 1;
                continue;
            }
        }

        // Header line between the `diff --git` and the first hunk.
        if raw.starts_with("Binary files") || raw.contains("GIT binary patch") {
            file.is_binary = true;
        }
        file.extra_headers.push(raw.to_string());
    }

    if let Some(file) = current.take() {
        files.push(file);
    }
    Diff { files }
}

/// Strip `a/` / `b/` prefixes and any trailing tab-timestamp from a diff path.
fn clean_path(raw: &str) -> String {
    let path = raw.split('\t').next().unwrap_or(raw);
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    path.to_string()
}

/// Parse the starting old/new line numbers from `@@ -A,B +C,D @@`.
fn parse_hunk_header(header: &str) -> (usize, usize) {
    let mut old_start = 1;
    let mut new_start = 1;
    if let Some(body) = header.strip_prefix("@@") {
        for token in body.split_whitespace() {
            if let Some(rest) = token.strip_prefix('-') {
                old_start = parse_start(rest);
            } else if let Some(rest) = token.strip_prefix('+') {
                new_start = parse_start(rest);
            }
        }
    }
    (old_start, new_start)
}

fn parse_start(token: &str) -> usize {
    token
        .split(',')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
}

/// Reconstruct a hunk's old/new sides and highlight each, returning one styled
/// line per [`DiffLine`] (in hunk order).
fn highlight_hunk(hunk: &Hunk, syntax: &SyntaxReference) -> Vec<StyledLine> {
    let mut old_text = String::new();
    let mut new_text = String::new();
    for line in &hunk.lines {
        match line.kind {
            DiffLineKind::Context => {
                old_text.push_str(&line.content);
                old_text.push('\n');
                new_text.push_str(&line.content);
                new_text.push('\n');
            }
            DiffLineKind::Add => {
                new_text.push_str(&line.content);
                new_text.push('\n');
            }
            DiffLineKind::Remove => {
                old_text.push_str(&line.content);
                old_text.push('\n');
            }
        }
    }

    let old_styled = highlight::highlight_block(&old_text, syntax);
    let new_styled = highlight::highlight_block(&new_text, syntax);

    let mut result = Vec::with_capacity(hunk.lines.len());
    let (mut oi, mut ni) = (0usize, 0usize);
    for line in &hunk.lines {
        let styled = match line.kind {
            DiffLineKind::Context => {
                let s = new_styled.get(ni).cloned().unwrap_or_default();
                oi += 1;
                ni += 1;
                s
            }
            DiffLineKind::Add => {
                let s = new_styled.get(ni).cloned().unwrap_or_default();
                ni += 1;
                s
            }
            DiffLineKind::Remove => {
                let s = old_styled.get(oi).cloned().unwrap_or_default();
                oi += 1;
                s
            }
        };
        result.push(styled);
    }
    result
}

/// Renders a [`Diff`] with line numbers, +/- gutters, and highlighted content.
pub struct DiffView {
    diff: Diff,
    font_size: f32,
}

impl DiffView {
    /// Parse a unified-diff string and prepare it for display.
    pub fn from_patch(patch: &str) -> Self {
        Self::new(parse_unified_diff(patch))
    }

    /// Display an already-parsed [`Diff`].
    pub fn new(diff: Diff) -> Self {
        Self {
            diff,
            font_size: 13.0,
        }
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) {
        ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                if self.diff.files.is_empty() {
                    ui.label(RichText::new("(no changes)").italics().color(GUTTER_FG));
                }
                for file in &self.diff.files {
                    self.show_file(ui, file);
                }
            });
    }

    fn show_file(&self, ui: &mut egui::Ui, file: &FileDiff) {
        // File header.
        ui.add_space(4.0);
        let title = match (&file.old_path, &file.new_path) {
            (Some(old), Some(new)) if old != new => format!("{old}  →  {new}"),
            _ => file.display_path().to_string(),
        };
        ui.label(
            RichText::new(title)
                .monospace()
                .size(self.font_size + 1.0)
                .strong()
                .color(FILE_FG),
        );
        if file.is_binary {
            ui.label(RichText::new("Binary file").italics().color(GUTTER_FG));
            return;
        }

        let syntax = highlight::syntax_for_path(std::path::Path::new(file.display_path()));
        let gutter_chars = digit_width(file.max_line_no());

        for hunk in &file.hunks {
            // Hunk header.
            row_frame(ui, HUNK_BG, |ui| {
                ui.label(
                    RichText::new(&hunk.header)
                        .monospace()
                        .size(self.font_size)
                        .color(HUNK_FG),
                );
            });

            let styled = highlight_hunk(hunk, syntax);
            for (line, content) in hunk.lines.iter().zip(styled.iter()) {
                self.show_line(ui, gutter_chars, line, content);
            }
        }
    }

    fn show_line(
        &self,
        ui: &mut egui::Ui,
        gutter_chars: usize,
        line: &DiffLine,
        content: &StyledLine,
    ) {
        let (bg, marker, marker_color) = match line.kind {
            DiffLineKind::Add => (ADD_BG, '+', ADD_FG),
            DiffLineKind::Remove => (REMOVE_BG, '-', REMOVE_FG),
            DiffLineKind::Context => (Color32::TRANSPARENT, ' ', GUTTER_FG),
        };

        row_frame(ui, bg, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(gutter(line.old_no, gutter_chars, self.font_size));
            ui.label(gutter(line.new_no, gutter_chars, self.font_size));
            ui.label(
                RichText::new(marker)
                    .monospace()
                    .size(self.font_size)
                    .color(marker_color),
            );
            ui.label(layout_styled(content, self.font_size));
        });
    }
}

/// A full-width horizontal row with a background fill.
fn row_frame(ui: &mut egui::Ui, bg: Color32, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .fill(bg)
        .inner_margin(Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(add_contents);
        });
}

/// Right-aligned, dim line-number cell (blank when `None`).
fn gutter(number: Option<usize>, width: usize, font_size: f32) -> RichText {
    let text = match number {
        Some(n) => format!("{n:>width$}"),
        None => " ".repeat(width),
    };
    RichText::new(text)
        .font(FontId::monospace(font_size))
        .color(GUTTER_FG)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Raw string: leading spaces on context lines (the diff markers) must be
    // preserved exactly.
    const PATCH: &str = r#"diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,4 +10,5 @@ fn ctx() {
 let keep = 1;
-let old = 2;
+let new = 2;
+let extra = 3;
 let tail = 4;
"#;

    #[test]
    fn parses_paths_kinds_and_line_numbers() {
        let diff = parse_unified_diff(PATCH);
        assert_eq!(diff.files.len(), 1);
        let file = &diff.files[0];
        assert_eq!(file.old_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(file.new_path.as_deref(), Some("src/lib.rs"));
        assert_eq!(file.display_path(), "src/lib.rs");

        let hunk = &file.hunks[0];
        let kinds: Vec<_> = hunk.lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffLineKind::Context,
                DiffLineKind::Remove,
                DiffLineKind::Add,
                DiffLineKind::Add,
                DiffLineKind::Context,
            ]
        );

        // Old side numbers advance on context + removals only.
        let old: Vec<_> = hunk.lines.iter().map(|l| l.old_no).collect();
        assert_eq!(old, vec![Some(10), Some(11), None, None, Some(12)]);
        // New side numbers advance on context + additions only.
        let new: Vec<_> = hunk.lines.iter().map(|l| l.new_no).collect();
        assert_eq!(new, vec![Some(10), None, Some(11), Some(12), Some(13)]);
    }

    #[test]
    fn handles_added_file_against_dev_null() {
        let patch = "diff --git a/new.rs b/new.rs\n\
new file mode 100644\n\
--- /dev/null\n\
+++ b/new.rs\n\
@@ -0,0 +1,2 @@\n\
+fn a() {}\n\
+fn b() {}\n";
        let diff = parse_unified_diff(patch);
        let file = &diff.files[0];
        assert_eq!(file.old_path.as_deref(), Some("/dev/null"));
        // display_path skips /dev/null in favor of the real (new) path.
        assert_eq!(file.display_path(), "new.rs");
        assert_eq!(file.hunks[0].lines.len(), 2);
        assert!(
            file.hunks[0]
                .lines
                .iter()
                .all(|l| l.kind == DiffLineKind::Add)
        );
    }

    #[test]
    fn highlight_hunk_yields_one_styled_line_per_diff_line() {
        let diff = parse_unified_diff(PATCH);
        let hunk = &diff.files[0].hunks[0];
        let syntax = highlight::syntax_for_path(std::path::Path::new("src/lib.rs"));
        let styled = highlight_hunk(hunk, syntax);
        assert_eq!(styled.len(), hunk.lines.len());
    }
}
