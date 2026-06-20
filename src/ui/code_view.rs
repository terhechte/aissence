//! [`CodeView`]: syntax-highlighted source with line numbers.

use std::path::Path;

use egui::{RichText, ScrollArea};
use syntect::parsing::SyntaxReference;

use super::layout_styled;
use crate::highlight::{self, StyledLine};

/// Dim color for the line-number gutter.
const GUTTER_FG: egui::Color32 = egui::Color32::from_gray(110);

/// A read-only, syntax-highlighted code view with an optional line-number
/// gutter. Construct, configure with the builder methods, then `.show(ui)`.
pub struct CodeView<'a> {
    code: &'a str,
    syntax: &'a SyntaxReference,
    font_size: f32,
    show_line_numbers: bool,
    first_line: usize,
}

impl<'a> CodeView<'a> {
    /// Create a view for `code` highlighted as `syntax`
    /// (see [`highlight::syntax_for_path`] / [`highlight::syntax_for_token`]).
    pub fn new(code: &'a str, syntax: &'a SyntaxReference) -> Self {
        Self {
            code,
            syntax,
            font_size: 13.0,
            show_line_numbers: true,
            first_line: 1,
        }
    }

    /// Create a view, choosing the syntax from a file path's extension/name.
    pub fn from_path(code: &'a str, path: &Path) -> Self {
        Self::new(code, highlight::syntax_for_path(path))
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn line_numbers(mut self, show: bool) -> Self {
        self.show_line_numbers = show;
        self
    }

    /// Line number assigned to the first rendered line (default `1`).
    pub fn first_line(mut self, first: usize) -> Self {
        self.first_line = first;
        self
    }

    pub fn show(self, ui: &mut egui::Ui) {
        let lines = highlight::highlight_block(self.code, self.syntax);
        let gutter_chars = digit_width(self.first_line + lines.len());

        ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                for (i, line) in lines.iter().enumerate() {
                    self.row(ui, self.first_line + i, gutter_chars, line);
                }
            });
    }

    fn row(&self, ui: &mut egui::Ui, number: usize, gutter_chars: usize, line: &StyledLine) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if self.show_line_numbers {
                ui.label(
                    RichText::new(format!("{number:>gutter_chars$}"))
                        .monospace()
                        .size(self.font_size)
                        .color(GUTTER_FG),
                );
            }
            ui.label(layout_styled(line, self.font_size));
        });
    }
}

/// Number of decimal digits needed to print `max`, at least 3.
pub(crate) fn digit_width(max: usize) -> usize {
    max.to_string().len().max(3)
}
