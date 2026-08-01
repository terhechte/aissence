//! One Dark inspired color theme and syntect-scope → token mapping.
//!
//! Vendored from VikerKit's `viker-core` (MIT,
//! <https://github.com/terhechte/VikerKit>): `style_for_token`/`default_style`
//! from `highlight/theme.rs`, and the syntect scope classifier from
//! `highlight/mod.rs`. Tree-sitter capture mapping was dropped.

use syntect::parsing::ScopeStack;

use super::style::{RgbColor, SyntaxStyle, SyntaxToken};

/// Maps a semantic syntax token to the built-in One Dark inspired style.
pub fn style_for_token(token: SyntaxToken) -> SyntaxStyle {
    match token {
        SyntaxToken::Keyword => SyntaxStyle::default().fg(RgbColor(198, 120, 221)), // purple
        SyntaxToken::TypeName
        | SyntaxToken::Attribute
        | SyntaxToken::Constructor
        | SyntaxToken::Link
        | SyntaxToken::Module => SyntaxStyle::default().fg(RgbColor(229, 192, 123)), // yellow
        SyntaxToken::Tag
        | SyntaxToken::Strong
        | SyntaxToken::Variable
        | SyntaxToken::Parameter
        | SyntaxToken::Property => SyntaxStyle::default().fg(RgbColor(224, 108, 117)), // red
        SyntaxToken::Function | SyntaxToken::Method | SyntaxToken::Macro | SyntaxToken::Heading => {
            SyntaxStyle::default().fg(RgbColor(97, 175, 239)) // blue
        }
        SyntaxToken::StringLiteral | SyntaxToken::Character | SyntaxToken::RawText => {
            SyntaxStyle::default().fg(RgbColor(152, 195, 121)) // green
        }
        SyntaxToken::LinkUrl | SyntaxToken::Escape => {
            SyntaxStyle::default().fg(RgbColor(86, 182, 194)) // cyan
        }
        SyntaxToken::Emphasis => SyntaxStyle::default().fg(RgbColor(171, 178, 191)).italic(),
        SyntaxToken::NumberLiteral
        | SyntaxToken::BooleanLiteral
        | SyntaxToken::Constant
        | SyntaxToken::Label => SyntaxStyle::default().fg(RgbColor(209, 154, 102)), // orange
        SyntaxToken::Comment => SyntaxStyle::default().fg(RgbColor(92, 99, 112)).italic(), // gray italic
        SyntaxToken::Punctuation
        | SyntaxToken::Operator
        | SyntaxToken::Text
        | SyntaxToken::Unknown => default_style(),
    }
}

/// Default text color (used when no scope matches).
pub fn default_style() -> SyntaxStyle {
    SyntaxStyle::default().fg(RgbColor(171, 178, 191)) // light gray
}

/// Pick a token for the innermost matching syntect scope on the stack.
pub fn token_for_syntect_scope_stack(stack: &ScopeStack) -> SyntaxToken {
    stack
        .scopes
        .iter()
        .rev()
        .find_map(|scope| token_for_syntect_scope(&scope.to_string()))
        .unwrap_or(SyntaxToken::Text)
}

/// Classify a single TextMate-style syntect scope (e.g. `keyword.control.rust`)
/// into a semantic [`SyntaxToken`].
pub fn token_for_syntect_scope(scope: &str) -> Option<SyntaxToken> {
    if scope.starts_with("comment") {
        Some(SyntaxToken::Comment)
    } else if scope.starts_with("constant.character.escape") {
        Some(SyntaxToken::Escape)
    } else if scope.starts_with("constant.character") {
        Some(SyntaxToken::Character)
    } else if scope.starts_with("string") {
        Some(SyntaxToken::StringLiteral)
    } else if scope.starts_with("constant.numeric") {
        Some(SyntaxToken::NumberLiteral)
    } else if scope.starts_with("constant.language.boolean") {
        Some(SyntaxToken::BooleanLiteral)
    } else if scope.starts_with("constant") {
        Some(SyntaxToken::Constant)
    } else if scope.starts_with("keyword.operator") {
        Some(SyntaxToken::Operator)
    } else if scope.starts_with("keyword") || scope.starts_with("storage") {
        Some(SyntaxToken::Keyword)
    } else if scope.starts_with("entity.name.tag") {
        Some(SyntaxToken::Tag)
    } else if scope.starts_with("entity.other.attribute-name") {
        Some(SyntaxToken::Attribute)
    } else if scope.starts_with("entity.name.function") || scope.starts_with("support.function") {
        Some(SyntaxToken::Function)
    } else if scope.starts_with("entity.name.type")
        || scope.starts_with("entity.name.class")
        || scope.starts_with("support.type")
        || scope.starts_with("support.class")
    {
        Some(SyntaxToken::TypeName)
    } else if scope.starts_with("entity.name.namespace") {
        Some(SyntaxToken::Module)
    } else if scope.starts_with("entity.name.section") {
        Some(SyntaxToken::Label)
    } else if scope.starts_with("support.constant") {
        Some(SyntaxToken::Constant)
    } else if scope.starts_with("support.variable.property")
        || scope.starts_with("variable.other.member")
    {
        Some(SyntaxToken::Property)
    } else if scope.starts_with("variable.parameter") {
        Some(SyntaxToken::Parameter)
    } else if scope.starts_with("variable") {
        Some(SyntaxToken::Variable)
    } else if scope.starts_with("punctuation") {
        Some(SyntaxToken::Punctuation)
    } else if scope.starts_with("markup.heading") {
        Some(SyntaxToken::Heading)
    } else if scope.starts_with("markup.bold") {
        Some(SyntaxToken::Strong)
    } else if scope.starts_with("markup.italic") {
        Some(SyntaxToken::Emphasis)
    } else if scope.starts_with("markup.underline.link") {
        Some(SyntaxToken::LinkUrl)
    } else if scope.starts_with("markup.raw") {
        Some(SyntaxToken::RawText)
    } else {
        None
    }
}
