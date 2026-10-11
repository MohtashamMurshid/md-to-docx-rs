//! TeX subset that `TexMathParser` in `src/renderers/mathRenderer.ts` accepts.
//!
//! `tex_to_omml` returns the `<m:oMath>` element the `docx` `Math` class writes
//! for those nodes. Positions follow JavaScript UTF-16 code units. Leading and
//! trailing whitespace follows `String.prototype.trim`.

use quick_xml::escape::escape;

use crate::error::MarkdownConversionError;

/// Characters and commands from `COMMAND_REPLACEMENTS` in `mathRenderer.ts`.
const COMMAND_REPLACEMENTS: &[(&str, &str)] = &[
    ("alpha", "\u{03B1}"),
    ("beta", "\u{03B2}"),
    ("gamma", "\u{03B3}"),
    ("delta", "\u{03B4}"),
    ("epsilon", "\u{03F5}"),
    ("varepsilon", "\u{03B5}"),
    ("theta", "\u{03B8}"),
    ("lambda", "\u{03BB}"),
    ("mu", "\u{03BC}"),
    ("pi", "\u{03C0}"),
    ("rho", "\u{03C1}"),
    ("sigma", "\u{03C3}"),
    ("phi", "\u{03D5}"),
    ("varphi", "\u{03C6}"),
    ("omega", "\u{03C9}"),
    ("Gamma", "\u{0393}"),
    ("Delta", "\u{0394}"),
    ("Theta", "\u{0398}"),
    ("Lambda", "\u{039B}"),
    ("Pi", "\u{03A0}"),
    ("Sigma", "\u{03A3}"),
    ("Phi", "\u{03A6}"),
    ("Omega", "\u{03A9}"),
    ("times", "\u{00D7}"),
    ("cdot", "\u{00B7}"),
    ("pm", "\u{00B1}"),
    ("mp", "\u{2213}"),
    ("le", "\u{2264}"),
    ("leq", "\u{2264}"),
    ("ge", "\u{2265}"),
    ("geq", "\u{2265}"),
    ("neq", "\u{2260}"),
    ("approx", "\u{2248}"),
    ("infty", "\u{221E}"),
    ("sum", "\u{2211}"),
    ("int", "\u{222B}"),
    ("sin", "sin"),
    ("cos", "cos"),
    ("tan", "tan"),
    ("log", "log"),
    ("ln", "ln"),
];

/// One OMML node, in the same shape `docx` builds for this TeX subset.
enum MathNode {
    Run(String),
    Sup {
        base: Box<MathNode>,
        sup: Vec<MathNode>,
    },
    Sub {
        base: Box<MathNode>,
        sub: Vec<MathNode>,
    },
    SubSup {
        base: Box<MathNode>,
        sub: Vec<MathNode>,
        sup: Vec<MathNode>,
    },
    Frac {
        num: Vec<MathNode>,
        den: Vec<MathNode>,
    },
    Sqrt {
        body: Vec<MathNode>,
    },
}

impl MathNode {
    fn write_omml(&self, out: &mut String) {
        match self {
            MathNode::Run(text) => {
                out.push_str("<m:r><m:t>");
                out.push_str(&escape(text.as_str()));
                out.push_str("</m:t></m:r>");
            }
            MathNode::Sup { base, sup } => {
                out.push_str("<m:sSup><m:sSupPr/>");
                write_children(out, "m:e", std::slice::from_ref(base.as_ref()));
                write_children(out, "m:sup", sup);
                out.push_str("</m:sSup>");
            }
            MathNode::Sub { base, sub } => {
                out.push_str("<m:sSub><m:sSubPr/>");
                write_children(out, "m:e", std::slice::from_ref(base.as_ref()));
                write_children(out, "m:sub", sub);
                out.push_str("</m:sSub>");
            }
            MathNode::SubSup { base, sub, sup } => {
                out.push_str("<m:sSubSup><m:sSubSupPr/>");
                write_children(out, "m:e", std::slice::from_ref(base.as_ref()));
                write_children(out, "m:sub", sub);
                write_children(out, "m:sup", sup);
                out.push_str("</m:sSubSup>");
            }
            MathNode::Frac { num, den } => {
                out.push_str("<m:f>");
                write_children(out, "m:num", num);
                write_children(out, "m:den", den);
                out.push_str("</m:f>");
            }
            MathNode::Sqrt { body } => {
                out.push_str("<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/>");
                write_children(out, "m:e", body);
                out.push_str("</m:rad>");
            }
        }
    }
}

fn write_children(out: &mut String, tag: &str, children: &[MathNode]) {
    out.push('<');
    out.push_str(tag);
    out.push('>');
    for child in children {
        child.write_omml(out);
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// Convert a TeX subset to an OMML `<m:oMath>` fragment.
///
/// Supported input is `COMMAND_REPLACEMENTS`, `^`, `_`, `\frac`, and `\sqrt`.
/// Anything else returns the same message `TexMathParser` throws.
pub fn tex_to_omml(src: &str) -> Result<String, crate::error::MarkdownConversionError> {
    let trimmed = js_trim(src);
    let units: Vec<u16> = trimmed.encode_utf16().collect();
    let nodes = Parser {
        source: &units,
        position: 0,
    }
    .parse()?;
    let mut body = String::new();
    for node in &nodes {
        node.write_omml(&mut body);
    }
    Ok(format!("<m:oMath>{body}</m:oMath>"))
}

struct Parser<'a> {
    source: &'a [u16],
    position: usize,
}

impl<'a> Parser<'a> {
    fn parse(&mut self) -> Result<Vec<MathNode>, MarkdownConversionError> {
        let children = self.parse_expression(None)?;
        self.skip_spaces();
        if self.position != self.source.len() {
            let token = unit_string(self.source, self.position);
            return Err(MarkdownConversionError::new(format!(
                "Unexpected token '{token}' at {}",
                self.position
            )));
        }
        if children.is_empty() {
            return Err(MarkdownConversionError::new("Math expression is empty"));
        }
        Ok(children)
    }

    fn parse_expression(
        &mut self,
        stop: Option<u16>,
    ) -> Result<Vec<MathNode>, MarkdownConversionError> {
        let mut children = Vec::new();
        while self.position < self.source.len() {
            if stop.is_some_and(|stop_unit| self.source[self.position] == stop_unit) {
                break;
            }
            children.push(self.parse_atom_with_scripts()?);
        }
        Ok(children)
    }

    fn parse_atom_with_scripts(&mut self) -> Result<MathNode, MarkdownConversionError> {
        let base = self.parse_atom()?;
        let mut subscript = None;
        let mut superscript = None;
        while matches!(self.current(), Some(unit) if unit == u16_from(b'_') || unit == u16_from(b'^'))
        {
            let marker = self.source[self.position];
            self.position += 1;
            let value = self.parse_script_argument()?;
            if marker == u16_from(b'_') {
                if subscript.is_some() {
                    return Err(MarkdownConversionError::new("Duplicate subscript"));
                }
                subscript = Some(value);
            } else if superscript.is_some() {
                return Err(MarkdownConversionError::new("Duplicate superscript"));
            } else {
                superscript = Some(value);
            }
        }
        Ok(match (subscript, superscript) {
            (Some(sub), Some(sup)) => MathNode::SubSup {
                base: Box::new(base),
                sub,
                sup,
            },
            (Some(sub), None) => MathNode::Sub {
                base: Box::new(base),
                sub,
            },
            (None, Some(sup)) => MathNode::Sup {
                base: Box::new(base),
                sup,
            },
            (None, None) => base,
        })
    }

    fn parse_atom(&mut self) -> Result<MathNode, MarkdownConversionError> {
        if self.position >= self.source.len() {
            return Err(MarkdownConversionError::new("Expected math token"));
        }
        let character = self.source[self.position];
        if character == u16_from(b'\\') {
            return self.parse_command();
        }
        if character == u16_from(b'{') {
            return Err(MarkdownConversionError::new("Unexpected group"));
        }
        if character == u16_from(b'}') {
            return Err(MarkdownConversionError::new("Unexpected closing group"));
        }
        if character == u16_from(b'_') || character == u16_from(b'^') {
            let shown = String::from_utf16_lossy(&[character]);
            return Err(MarkdownConversionError::new(format!(
                "Missing base before '{shown}'"
            )));
        }
        self.position += 1;
        Ok(MathNode::Run(String::from_utf16_lossy(&[character])))
    }

    fn parse_command(&mut self) -> Result<MathNode, MarkdownConversionError> {
        self.position += 1;
        let start = self.position;
        while self.current().is_some_and(is_ascii_letter) {
            self.position += 1;
        }
        if start == self.position {
            let Some(escaped) = self.current() else {
                return Err(MarkdownConversionError::new("Trailing escape"));
            };
            self.position += 1;
            return Ok(MathNode::Run(String::from_utf16_lossy(&[escaped])));
        }
        let command = String::from_utf16_lossy(&self.source[start..self.position]);
        if command == "frac" {
            let num = self.parse_required_group("fraction numerator")?;
            let den = self.parse_required_group("fraction denominator")?;
            return Ok(MathNode::Frac { num, den });
        }
        if command == "sqrt" {
            if self.current() == Some(u16_from(b'[')) {
                return Err(MarkdownConversionError::new(
                    "Root degree syntax is not supported",
                ));
            }
            let body = self.parse_required_group("square root body")?;
            return Ok(MathNode::Sqrt { body });
        }
        match command_replacement(&command) {
            Some(text) => Ok(MathNode::Run(text.to_string())),
            None => Err(MarkdownConversionError::new(format!(
                "Unsupported command \\{command}"
            ))),
        }
    }

    fn parse_required_group(
        &mut self,
        label: &str,
    ) -> Result<Vec<MathNode>, MarkdownConversionError> {
        self.skip_spaces();
        if self.current() != Some(u16_from(b'{')) {
            return Err(MarkdownConversionError::new(format!("Expected {label}")));
        }
        self.position += 1;
        let children = self.parse_expression(Some(u16_from(b'}')))?;
        if self.current() != Some(u16_from(b'}')) {
            return Err(MarkdownConversionError::new(format!("Unclosed {label}")));
        }
        self.position += 1;
        if children.is_empty() {
            return Err(MarkdownConversionError::new(format!("{label} is empty")));
        }
        Ok(children)
    }

    fn parse_script_argument(&mut self) -> Result<Vec<MathNode>, MarkdownConversionError> {
        if self.current() == Some(u16_from(b'{')) {
            self.parse_required_group("script")
        } else {
            Ok(vec![self.parse_atom()?])
        }
    }

    fn skip_spaces(&mut self) {
        while self.current().is_some_and(is_js_regexp_space) {
            self.position += 1;
        }
    }

    fn current(&self) -> Option<u16> {
        self.source.get(self.position).copied()
    }
}

fn command_replacement(command: &str) -> Option<&'static str> {
    COMMAND_REPLACEMENTS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, value)| *value)
}

fn u16_from(byte: u8) -> u16 {
    u16::from(byte)
}

fn is_ascii_letter(unit: u16) -> bool {
    matches!(unit, 0x41..=0x5A | 0x61..=0x7A)
}

fn unit_string(source: &[u16], index: usize) -> String {
    match source.get(index).copied() {
        Some(unit) => String::from_utf16_lossy(&[unit]),
        None => String::new(),
    }
}

/// `String.prototype.trim`: Unicode `White_Space`, line terminators, and U+FEFF.
fn js_trim(src: &str) -> &str {
    let start = src
        .char_indices()
        .find(|(_, ch)| !is_js_trim_whitespace(*ch))
        .map(|(index, _)| index)
        .unwrap_or(src.len());
    let end = src[start..]
        .char_indices()
        .rev()
        .find(|(_, ch)| !is_js_trim_whitespace(*ch))
        .map(|(index, ch)| start + index + ch.len_utf8())
        .unwrap_or(start);
    &src[start..end]
}

fn is_js_trim_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
            | '\u{2000}'..='\u{200A}'
    )
}

/// JavaScript `\s`, which does not include U+0085.
fn is_js_regexp_space(unit: u16) -> bool {
    matches!(
        unit,
        0x0009
            | 0x000A
            | 0x000B
            | 0x000C
            | 0x000D
            | 0x0020
            | 0x00A0
            | 0x1680
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
            | 0x2000..=0x200A
    )
}

#[cfg(test)]
mod tests {
    use super::{tex_to_omml, COMMAND_REPLACEMENTS};

    fn omml(src: &str) -> String {
        tex_to_omml(src).unwrap_or_else(|err| panic!("{src}: {err}"))
    }

    #[test]
    fn command_replacements_become_single_math_runs() {
        assert_eq!(COMMAND_REPLACEMENTS.len(), 41);
        for (name, value) in COMMAND_REPLACEMENTS {
            let xml = omml(&format!("\\{name}"));
            let expected = format!("<m:oMath><m:r><m:t>{value}</m:t></m:r></m:oMath>");
            assert_eq!(xml, expected, "\\{name}");
        }
        assert!(omml(r"\epsilon").contains('\u{03F5}'));
        assert!(!omml(r"\epsilon").contains('\u{03B5}'));
        assert!(omml(r"\varepsilon").contains('\u{03B5}'));
        assert!(omml(r"\phi").contains('\u{03D5}'));
        assert!(!omml(r"\phi").contains('\u{03C6}'));
        assert!(omml(r"\varphi").contains('\u{03C6}'));
        assert_eq!(
            omml(r"\alpha+\beta"),
            "<m:oMath><m:r><m:t>\u{03B1}</m:t></m:r><m:r><m:t>+</m:t></m:r><m:r><m:t>\u{03B2}</m:t></m:r></m:oMath>"
        );
    }

    #[test]
    fn superscript_subscript_and_both() {
        assert_eq!(
            omml("x^2"),
            "<m:oMath><m:sSup><m:sSupPr/><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath>"
        );
        assert_eq!(
            omml("x_i"),
            "<m:oMath><m:sSub><m:sSubPr/><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>i</m:t></m:r></m:sub></m:sSub></m:oMath>"
        );
        let both = "<m:oMath><m:sSubSup><m:sSubSupPr/><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>i</m:t></m:r></m:sub><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSubSup></m:oMath>";
        assert_eq!(omml("x_i^2"), both);
        assert_eq!(omml("x^2_i"), both);
        assert_eq!(
            omml("x^{10}"),
            "<m:oMath><m:sSup><m:sSupPr/><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>1</m:t></m:r><m:r><m:t>0</m:t></m:r></m:sup></m:sSup></m:oMath>"
        );
        assert_eq!(omml("  x^2  "), omml("x^2"));
    }

    #[test]
    fn fraction_and_square_root() {
        assert_eq!(
            omml(r"\frac{1}{2}"),
            "<m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath>"
        );
        assert_eq!(omml(r"\frac {1} {2}"), omml(r"\frac{1}{2}"));
        assert_eq!(
            omml(r"\sqrt{x}"),
            "<m:oMath><m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e><m:r><m:t>x</m:t></m:r></m:e></m:rad></m:oMath>"
        );
        assert_eq!(
            omml(r"\frac{x^2}{\sqrt{y}}"),
            "<m:oMath><m:f><m:num><m:sSup><m:sSupPr/><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:num><m:den><m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/><m:e><m:r><m:t>y</m:t></m:r></m:e></m:rad></m:den></m:f></m:oMath>"
        );
    }

    #[test]
    fn escapes_text_and_rejects_unsupported_input() {
        assert_eq!(
            omml(r"\&"),
            "<m:oMath><m:r><m:t>&amp;</m:t></m:r></m:oMath>"
        );
        assert_eq!(omml("<"), "<m:oMath><m:r><m:t>&lt;</m:t></m:r></m:oMath>");
        assert_eq!(
            tex_to_omml(r"\overline{x}").unwrap_err().message(),
            r"Unsupported command \overline"
        );
        assert_eq!(
            tex_to_omml("").unwrap_err().message(),
            "Math expression is empty"
        );
        assert_eq!(
            tex_to_omml("   ").unwrap_err().message(),
            "Math expression is empty"
        );
        assert_eq!(
            tex_to_omml("x_1_2").unwrap_err().message(),
            "Duplicate subscript"
        );
        assert_eq!(
            tex_to_omml("x^1^2").unwrap_err().message(),
            "Duplicate superscript"
        );
        assert_eq!(
            tex_to_omml(r"\sqrt[3]{x}").unwrap_err().message(),
            "Root degree syntax is not supported"
        );
        assert_eq!(
            tex_to_omml(r"\frac{}{1}").unwrap_err().message(),
            "fraction numerator is empty"
        );
        assert_eq!(
            tex_to_omml("^2").unwrap_err().message(),
            "Missing base before '^'"
        );
        assert_eq!(tex_to_omml(r"\").unwrap_err().message(), "Trailing escape");
        assert_eq!(
            tex_to_omml("{x}").unwrap_err().message(),
            "Unexpected group"
        );
    }
}
