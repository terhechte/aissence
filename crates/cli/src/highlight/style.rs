//! Frontend-independent syntax token + style types.
//!
//! Vendored (and trimmed) from VikerKit's `viker-core` (MIT,
//! <https://github.com/terhechte/VikerKit>). The `serde` derives and the
//! tree-sitter-specific paths were dropped — we only keep what the syntect
//! backend needs.

/// Frontend-independent semantic syntax token. Frontends map this into their
/// own theme at draw time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SyntaxToken {
    #[default]
    Text,
    Keyword,
    TypeName,
    Tag,
    Attribute,
    Constructor,
    Function,
    Method,
    Macro,
    StringLiteral,
    Escape,
    Character,
    NumberLiteral,
    BooleanLiteral,
    Constant,
    Comment,
    Variable,
    Parameter,
    Property,
    Module,
    Label,
    Punctuation,
    Operator,
    Heading,
    RawText,
    Link,
    LinkUrl,
    Emphasis,
    Strong,
    Unknown,
}

/// Frontend-independent syntax style. Convert this into the native render style
/// (e.g. [`egui::Color32`]) at draw time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyntaxStyle {
    pub fg: Option<RgbColor>,
    pub italic: bool,
}

/// A 24-bit RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor(pub u8, pub u8, pub u8);

/// A token together with its resolved style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyntaxHighlight {
    pub token: SyntaxToken,
    pub style: SyntaxStyle,
}

impl SyntaxHighlight {
    pub fn new(token: SyntaxToken, style: SyntaxStyle) -> Self {
        Self { token, style }
    }
}

impl SyntaxStyle {
    pub fn fg(mut self, color: RgbColor) -> Self {
        self.fg = Some(color);
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
}
