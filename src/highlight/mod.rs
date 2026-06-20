//! Lightweight, syntect-only syntax highlighting.
//!
//! This is a trimmed adaptation of VikerKit's highlighting (MIT,
//! <https://github.com/terhechte/VikerKit>). We keep only the **syntect**
//! backend — no tree-sitter, no ropey, no editor — and expose it as a simple
//! "highlight a string into styled line segments" API suitable for rendering in
//! egui (see [`crate::ui`]).
//!
//! ```
//! use aissence::highlight;
//! let syntax = highlight::syntax_for_path(std::path::Path::new("a.rs"));
//! let lines = highlight::highlight_block("fn main() {}", syntax);
//! assert_eq!(lines.len(), 1);
//! ```

pub mod style;
pub mod theme;

use std::sync::LazyLock;

use syntect::easy::ScopeRegionIterator;
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

pub use style::{RgbColor, SyntaxHighlight, SyntaxStyle, SyntaxToken};
use theme::{style_for_token, token_for_syntect_scope_stack};

/// The default syntax set (newline-aware variant, required for line-by-line
/// stateful parsing). Loaded once, lazily.
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// A run of characters sharing one style, as a ready-to-render text piece.
#[derive(Debug, Clone)]
pub struct StyledSegment {
    pub text: String,
    pub style: SyntaxStyle,
}

/// One source line, tiled left-to-right by [`StyledSegment`]s (no gaps). An
/// empty line yields an empty `Vec`.
pub type StyledLine = Vec<StyledSegment>;

/// Access the shared syntax set.
pub fn syntax_set() -> &'static SyntaxSet {
    &SYNTAX_SET
}

/// Resolve a syntect syntax for `path` by extension, then by file name, falling
/// back to plain text. Never returns `None` so callers can highlight anything.
pub fn syntax_for_path(path: &std::path::Path) -> &'static SyntaxReference {
    let set = syntax_set();
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(|ext| set.find_syntax_by_extension(ext))
        .or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .and_then(|name| set.find_syntax_by_token(name))
        })
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

/// Resolve a syntax by a bare token (extension or name, e.g. `"rust"`, `"rs"`,
/// `"toml"`), falling back to plain text.
pub fn syntax_for_token(token: &str) -> &'static SyntaxReference {
    let set = syntax_set();
    set.find_syntax_by_token(token)
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

/// Highlight a multi-line block of source into per-line styled segments.
///
/// State (string/comment scopes) is carried across lines, so multi-line
/// constructs highlight correctly within the block.
pub fn highlight_block(code: &str, syntax: &SyntaxReference) -> Vec<StyledLine> {
    let set = syntax_set();
    let mut parse_state = ParseState::new(syntax);
    let mut scope_stack = ScopeStack::new();
    let mut lines = Vec::new();

    for line in LinesWithEndings::from(code) {
        let mut segments: StyledLine = Vec::new();
        let Ok(ops) = parse_state.parse_line(line, set) else {
            lines.push(segments);
            continue;
        };

        for (segment, op) in ScopeRegionIterator::new(&ops, line) {
            if scope_stack.apply(op).is_err() {
                break;
            }
            // Drop the trailing newline from the visible text.
            let text = segment.trim_end_matches(['\r', '\n']);
            if text.is_empty() {
                continue;
            }
            let token = token_for_syntect_scope_stack(&scope_stack);
            segments.push(StyledSegment {
                text: text.to_string(),
                style: style_for_token(token),
            });
        }

        lines.push(segments);
    }

    // `LinesWithEndings` yields nothing for an empty input; represent it as one
    // empty line so the caller still renders a (blank) row.
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

/// Highlight a single line in isolation (fresh parser state).
pub fn highlight_line(line: &str, syntax: &SyntaxReference) -> StyledLine {
    highlight_block(line, syntax).into_iter().next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn find_colored<'a>(line: &'a StyledLine, needle: &str) -> Option<&'a StyledSegment> {
        line.iter().find(|s| s.text.contains(needle))
    }

    #[test]
    fn rust_keyword_and_string_get_theme_colors() {
        let syntax = syntax_for_path(Path::new("demo.rs"));
        let lines = highlight_block("fn main() { let s = \"hi\"; }", syntax);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];

        // `fn` is a keyword → One Dark purple.
        let kw = find_colored(line, "fn").expect("a segment containing `fn`");
        assert_eq!(kw.style.fg, Some(RgbColor(198, 120, 221)));

        // The string literal → One Dark green.
        let string = find_colored(line, "hi").expect("a segment containing the string");
        assert_eq!(string.style.fg, Some(RgbColor(152, 195, 121)));
    }

    #[test]
    fn unknown_extension_falls_back_to_plain_text() {
        let syntax = syntax_for_path(Path::new("file.unknownext"));
        // Plain text still produces one styled line per input line.
        let lines = highlight_block("just some text\nmore", syntax);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn empty_input_is_one_blank_line() {
        let syntax = syntax_for_token("rust");
        assert_eq!(highlight_block("", syntax).len(), 1);
    }
}
