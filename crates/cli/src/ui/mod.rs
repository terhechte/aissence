//! egui widgets built on the vendored syntect highlighter.
//!
//! - [`CodeView`] renders source with line numbers and syntax highlighting.
//! - [`DiffView`] renders a unified git patch with +/- gutters, line numbers,
//!   and syntax-highlighted content.
//!
//! Both are plain immediate-mode widgets — call `.show(ui)` inside any egui
//! layout. They depend only on `egui` (no `eframe`), so they embed anywhere.

pub mod code_view;
pub mod diff;

pub use code_view::CodeView;
pub use diff::{Diff, DiffLine, DiffLineKind, DiffView, FileDiff, Hunk, parse_unified_diff};

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};

use crate::highlight::{RgbColor, StyledLine, SyntaxStyle};

/// Fallback foreground (One Dark light gray), matching the vendored theme.
pub(crate) const DEFAULT_FG: Color32 = Color32::from_rgb(171, 178, 191);

/// Convert a vendored [`SyntaxStyle`] foreground into an egui color.
pub(crate) fn to_color32(style: SyntaxStyle) -> Color32 {
    match style.fg {
        Some(RgbColor(r, g, b)) => Color32::from_rgb(r, g, b),
        None => DEFAULT_FG,
    }
}

/// Build a non-wrapping monospace [`LayoutJob`] from styled segments.
pub(crate) fn layout_styled(line: &StyledLine, font_size: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    // Don't wrap: long lines scroll horizontally instead of breaking the grid.
    job.wrap.max_width = f32::INFINITY;
    for segment in line {
        job.append(
            &segment.text,
            0.0,
            TextFormat {
                font_id: FontId::monospace(font_size),
                color: to_color32(segment.style),
                italics: segment.style.italic,
                ..Default::default()
            },
        );
    }
    job
}
