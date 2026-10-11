//! Optional fenced-code colors.
//!
//! Syntect 5.3.0 parses the fence. Each scope prefix maps to a hex color from
//! `DEFAULT_CODE_HIGHLIGHT_THEME` in `src/utils/codeHighlight.ts`. Those keys
//! are the highlight.js class names without the `hljs-` prefix. Token splits
//! follow syntect's syntax definitions.

use std::sync::OnceLock;

use syntect::parsing::{
    ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxDefinition, SyntaxReference, SyntaxSet,
    SyntaxSetBuilder,
};
use syntect::util::LinesWithEndings;

const DEFAULT_COLOR: &str = "24292E";
const MAX_LANGUAGE_NAME_LENGTH: usize = 64;

/// Scope prefix, then the theme hex for that `hljs-*` key.
const SCOPE_THEME: &[(&str, &str)] = &[
    ("comment", "6A737D"),
    ("keyword.control", "D73A49"),
    ("keyword.operator", "D73A49"),
    ("keyword", "D73A49"),
    ("storage.type", "6F42C1"),
    ("storage.modifier", "D73A49"),
    ("support.function", "005CC5"),
    ("support.type", "6F42C1"),
    ("support.class", "6F42C1"),
    ("support.constant", "005CC5"),
    ("constant.numeric", "005CC5"),
    ("constant.language", "005CC5"),
    ("constant.character", "032F62"),
    ("constant.other.symbol", "005CC5"),
    ("string.regexp", "032F62"),
    ("string", "032F62"),
    ("variable.language", "D73A49"),
    ("variable.other.constant", "005CC5"),
    ("variable.parameter", "24292E"),
    ("variable", "E36209"),
    ("entity.name.function", "6F42C1"),
    ("entity.name.class", "6F42C1"),
    ("entity.name.type", "6F42C1"),
    ("entity.name.tag", "22863A"),
    ("entity.name.section", "005CC5"),
    ("entity.other.attribute-name", "6F42C1"),
    ("entity.name", "6F42C1"),
    ("punctuation.definition.comment", "6A737D"),
    ("punctuation.definition.string", "032F62"),
    ("punctuation", "24292E"),
    ("markup.deleted", "B31D28"),
    ("markup.inserted", "22863A"),
    ("meta.annotation", "6A737D"),
    ("meta.preprocessor", "6A737D"),
];

struct ScopeColor {
    prefix: Scope,
    color: &'static str,
}

/// One colored slice of a code block.
///
/// `color` is a theme hex without `#`. `None` means the fence language is
/// unknown, or tokenization failed, and the caller should keep the plain run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub text: String,
    pub color: Option<String>,
}

/// Splits `code` into theme-colored spans for `language`.
///
/// A blank name, a name longer than 64 characters, or a name outside the
/// bundled syntax set returns one span whose `color` is `None`. A parse
/// failure uses that same span.
pub fn highlight_code(code: &str, language: &str) -> Vec<HighlightSpan> {
    let Some((syntax, set)) = find_syntax(language) else {
        return vec![plain_span(code)];
    };
    match highlight_with_syntax(code, syntax, set) {
        Some(spans) if code.is_empty() || !spans.is_empty() => spans,
        _ => vec![plain_span(code)],
    }
}

fn plain_span(code: &str) -> HighlightSpan {
    HighlightSpan {
        text: code.to_string(),
        color: None,
    }
}

fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn typescript_syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut builder = SyntaxSetBuilder::new();
        for source in [
            include_str!("../syntaxes/TypeScript.sublime-syntax"),
            include_str!("../syntaxes/TypeScriptReact.sublime-syntax"),
        ] {
            // These definitions are bundled and validated by the syntax tests.
            // Loading with newlines matches LinesWithEndings below.
            builder.add(
                SyntaxDefinition::load_from_str(source, true, None)
                    .expect("bundled TypeScript syntax must be valid"),
            );
        }
        builder.build()
    })
}

fn scope_colors() -> &'static [ScopeColor] {
    static COLORS: OnceLock<Vec<ScopeColor>> = OnceLock::new();
    COLORS.get_or_init(|| {
        let mut items = Vec::new();
        for (name, color) in SCOPE_THEME {
            if let Ok(prefix) = Scope::new(name) {
                items.push(ScopeColor { prefix, color });
            }
        }
        items.sort_by_key(|item| std::cmp::Reverse(item.prefix.len()));
        items
    })
}

fn find_syntax(language: &str) -> Option<(&'static SyntaxReference, &'static SyntaxSet)> {
    let normalized = language.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized.chars().count() > MAX_LANGUAGE_NAME_LENGTH {
        return None;
    }
    let token = match normalized.as_str() {
        "typescript" => "ts",
        "typescriptreact" | "typescript-react" => "tsx",
        token => token,
    };
    let set = match token {
        "ts" | "tsx" | "mts" | "cts" => typescript_syntax_set(),
        _ => syntax_set(),
    };
    set.find_syntax_by_token(token).map(|syntax| (syntax, set))
}

fn highlight_with_syntax(
    code: &str,
    syntax: &SyntaxReference,
    set: &SyntaxSet,
) -> Option<Vec<HighlightSpan>> {
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::default();
    let mut spans = Vec::new();
    for line in LinesWithEndings::from(code) {
        let ops = state.parse_line(line, set).ok()?;
        if !push_line(line, &ops, &mut stack, &mut spans) {
            return None;
        }
    }
    Some(spans)
}

fn push_line(
    line: &str,
    ops: &[(usize, ScopeStackOp)],
    stack: &mut ScopeStack,
    spans: &mut Vec<HighlightSpan>,
) -> bool {
    let mut index = 0;
    let mut op_index = 0;
    while op_index < ops.len() {
        let pos = ops[op_index].0;
        if pos < index || pos > line.len() {
            return false;
        }
        if pos > index {
            let Some(text) = line.get(index..pos) else {
                return false;
            };
            push_span(spans, text, color_for_stack(stack));
            index = pos;
        }
        while op_index < ops.len() && ops[op_index].0 == pos {
            if stack.apply(&ops[op_index].1).is_err() {
                return false;
            }
            op_index += 1;
        }
    }
    if index > line.len() {
        return false;
    }
    if index < line.len() {
        let Some(text) = line.get(index..) else {
            return false;
        };
        push_span(spans, text, color_for_stack(stack));
    }
    true
}

fn color_for_stack(stack: &ScopeStack) -> &'static str {
    for scope in stack.as_slice().iter().rev() {
        if let Some(color) = color_for_scope(*scope) {
            return color;
        }
    }
    DEFAULT_COLOR
}

fn color_for_scope(scope: Scope) -> Option<&'static str> {
    for item in scope_colors() {
        if item.prefix.is_prefix_of(scope) {
            return Some(item.color);
        }
    }
    None
}

fn push_span(spans: &mut Vec<HighlightSpan>, text: &str, color: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.color.as_deref() == Some(color) {
            last.text.push_str(text);
            return;
        }
    }
    spans.push(HighlightSpan {
        text: text.to_string(),
        color: Some(color.to_string()),
    });
}

#[cfg(test)]
mod tests {
    use super::highlight_code;

    #[test]
    fn known_language_uses_theme_colors() {
        let code = "fn main() {\n    let value = 1;\n}\n";
        let spans = highlight_code(code, "rust");
        let joined: String = spans.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(joined, code);
        assert!(spans.len() > 1, "spans: {spans:?}");
        assert!(
            spans.iter().all(|span| span.color.is_some()),
            "spans: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|span| span.color.as_deref() != Some("24292E")),
            "expected a theme color other than default, spans: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text == "fn" && span.color.as_deref() == Some("6F42C1")),
            "expected type color on fn, spans: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text == "=" && span.color.as_deref() == Some("D73A49")),
            "expected keyword.operator color, spans: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text == "1" && span.color.as_deref() == Some("005CC5")),
            "expected number color, spans: {spans:?}"
        );
    }

    #[test]
    fn unknown_language_returns_one_uncolored_span() {
        let code = "fn main() {}";
        let spans = highlight_code(code, "not-a-language");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, code);
        assert_eq!(spans[0].color, None);

        let blank = highlight_code(code, "   ");
        assert_eq!(blank.len(), 1);
        assert_eq!(blank[0].text, code);
        assert_eq!(blank[0].color, None);
    }

    #[test]
    fn typescript_aliases_color_types_and_preserve_source() {
        let code = "interface User<T> { readonly value: T; }\n\
                    type Result = number | null;\n\
                    const count: number = 42;\n\
                    // café 🦀\n";
        let expected = highlight_code(code, "ts");
        for alias in ["ts", "typescript", " TS ", "TypeScript", "mts", "cts"] {
            let spans = highlight_code(code, alias);
            assert_eq!(spans, expected, "alias: {alias}");
            let joined: String = spans.iter().map(|span| span.text.as_str()).collect();
            assert_eq!(joined, code);
            for token in ["interface", "readonly", "type", "number"] {
                assert!(
                    spans.iter().any(|span| span.text.contains(token)
                        && span.color.as_deref().is_some_and(|color| color != "24292E")),
                    "expected TypeScript color for {token}, spans: {spans:?}"
                );
            }
            assert!(spans.windows(2).all(|pair| pair[0].color != pair[1].color));
        }
    }

    #[test]
    fn tsx_colors_tags_attributes_and_embedded_expressions() {
        let code = "const view = <div><Widget title=\"hello\">{42}</Widget></div>;\n";
        let expected = highlight_code(code, "tsx");
        for alias in ["tsx", "typescriptreact", "typescript-react"] {
            let spans = highlight_code(code, alias);
            assert_eq!(spans, expected);
            assert_eq!(
                spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>(),
                code
            );
            for (token, color) in [
                ("div", "22863A"),
                ("Widget", "6F42C1"),
                ("title", "6F42C1"),
                ("42", "005CC5"),
            ] {
                assert!(
                    spans
                        .iter()
                        .any(|span| span.text.contains(token)
                            && span.color.as_deref() == Some(color)),
                    "expected {color} on {token}, spans: {spans:?}"
                );
            }
        }
    }
}
