//! GFM autolink literals, the way `remark-gfm` produces them.
//!
//! Two passes run over every text node that is not already inside a link.
//! The first follows `micromark-extension-gfm-autolink-literal`: `www.`,
//! `http://`/`https://` and email literals with its domain, path and trail
//! rules. The second follows `mdast-util-gfm-autolink-literal`'s
//! find-and-replace regexes over whatever text is left. Each linked span and
//! each remaining piece of text becomes its own mdast node, so each becomes
//! its own run.

use std::sync::OnceLock;

use regex::Regex;

/// One piece of a text node after autolink detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    /// Set for a linked span: the mdast `url`.
    pub url: Option<String>,
}

/// Split `text` into plain and linked segments. `previous` is the character
/// before this text node in its paragraph, if any.
pub fn split_autolinks(text: &str, previous: Option<char>) -> Vec<Segment> {
    if !text.contains(['.', '@', ':']) {
        return vec![plain(text)];
    }
    let mut out = Vec::new();
    for segment in tokenizer_pass(text, previous) {
        if segment.url.is_some() {
            out.push(segment);
            continue;
        }
        for piece in regex_url_pass(&segment.text) {
            if piece.url.is_some() {
                out.push(piece);
            } else {
                out.extend(regex_email_pass(&piece.text));
            }
        }
    }
    out.retain(|segment| !segment.text.is_empty());
    if out.is_empty() {
        out.push(plain(""));
    }
    out
}

/// Literal tokenizer spans in the Markdown source, before emphasis and entity
/// decoding. The caller excludes code, existing links, HTML and image labels.
pub(crate) fn source_links(source: &str) -> Vec<(std::ops::Range<usize>, Segment)> {
    let mut offset = 0;
    tokenizer_pass(source, None)
        .into_iter()
        .filter_map(|segment| {
            let start = offset;
            offset += segment.text.len();
            segment.url.as_ref()?;
            Some((start..offset, segment))
        })
        .collect()
}

fn plain(text: &str) -> Segment {
    Segment {
        text: text.to_string(),
        url: None,
    }
}

fn linked(text: &str, url: String) -> Segment {
    Segment {
        text: text.to_string(),
        url: Some(url),
    }
}

// ---------------------------------------------------------------------------
// Character classes from `micromark-util-character`.

/// JavaScript `\s`.
pub fn js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `\p{P}|\p{S}`.
fn unicode_punctuation(ch: char) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| Regex::new(r"^[\p{P}\p{S}]$").expect("valid regex"));
    let mut buffer = [0u8; 4];
    pattern.is_match(ch.encode_utf8(&mut buffer))
}

/// `markdownLineEndingOrSpace`.
fn line_ending_or_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r')
}

fn ascii_control(ch: char) -> bool {
    ch < ' ' || ch == '\u{7f}'
}

/// Space, line ending, or Unicode whitespace: an end for domains and paths.
fn ends_domain(ch: Option<char>) -> bool {
    match ch {
        None => true,
        Some(ch) => line_ending_or_space(ch) || js_whitespace(ch),
    }
}

/// `gfmAtext`: `+`, `-`, `.`, `_`, or ASCII alphanumeric.
fn gfm_atext(ch: char) -> bool {
    matches!(ch, '+' | '-' | '.' | '_') || ch.is_ascii_alphanumeric()
}

// ---------------------------------------------------------------------------
// Pass 1: the micromark tokenizer.

fn tokenizer_pass(text: &str, previous: Option<char>) -> Vec<Segment> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut index = 0;
    while index < chars.len() {
        let prev = if index == 0 {
            previous
        } else {
            Some(chars[index - 1])
        };
        let ch = chars[index];
        let mut matched: Option<(usize, String)> = None;
        if gfm_atext(ch) {
            matched = email_autolink(&chars, index, prev);
        }
        if matched.is_none() && matches!(ch, 'w' | 'W') {
            matched = www_autolink(&chars, index, prev);
        }
        if matched.is_none() && matches!(ch, 'h' | 'H') {
            matched = protocol_autolink(&chars, index, prev);
        }
        if let Some((end, url)) = matched {
            if plain_start < index {
                out.push(plain(&chars[plain_start..index].iter().collect::<String>()));
            }
            let display: String = chars[index..end].iter().collect();
            out.push(linked(&display, url));
            plain_start = end;
            index = end;
            continue;
        }
        index += 1;
    }
    if plain_start < chars.len() {
        out.push(plain(&chars[plain_start..].iter().collect::<String>()));
    }
    out
}

/// `previousWww`.
fn previous_www(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(ch) => matches!(ch, '(' | '*' | '_' | '[' | ']' | '~') || line_ending_or_space(ch),
    }
}

fn www_autolink(chars: &[char], start: usize, prev: Option<char>) -> Option<(usize, String)> {
    if !previous_www(prev) {
        return None;
    }
    // `www.` followed by anything at all.
    if chars.len() < start + 5 {
        return None;
    }
    if !chars[start..start + 3]
        .iter()
        .all(|ch| matches!(ch, 'w' | 'W'))
        || chars[start + 3] != '.'
    {
        return None;
    }
    let domain_end = scan_domain(chars, start)?;
    let end = scan_path(chars, domain_end);
    let display: String = chars[start..end].iter().collect();
    Some((end, format!("http://{display}")))
}

fn protocol_autolink(chars: &[char], start: usize, prev: Option<char>) -> Option<(usize, String)> {
    if prev.is_some_and(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    let mut index = start;
    let mut scheme = String::new();
    while index < chars.len() && chars[index].is_ascii_alphabetic() && scheme.len() < 5 {
        scheme.push(chars[index]);
        index += 1;
    }
    if chars.get(index) != Some(&':') {
        return None;
    }
    let lower = scheme.to_ascii_lowercase();
    if lower != "http" && lower != "https" {
        return None;
    }
    index += 1;
    if chars.get(index) != Some(&'/') || chars.get(index + 1) != Some(&'/') {
        return None;
    }
    index += 2;
    let ch = chars.get(index)?;
    if ascii_control(*ch)
        || line_ending_or_space(*ch)
        || js_whitespace(*ch)
        || unicode_punctuation(*ch)
    {
        return None;
    }
    let domain_end = scan_domain(chars, index)?;
    let end = scan_path(chars, domain_end);
    let display: String = chars[start..end].iter().collect();
    Some((end, display))
}

/// `previousEmail`: anything but `/` and atext characters.
fn previous_email(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(ch) => ch != '/' && !gfm_atext(ch),
    }
}

fn email_autolink(chars: &[char], start: usize, prev: Option<char>) -> Option<(usize, String)> {
    if !previous_email(prev) {
        return None;
    }
    let mut index = start;
    while index < chars.len() && gfm_atext(chars[index]) {
        index += 1;
    }
    if chars.get(index) != Some(&'@') {
        return None;
    }
    index += 1;
    let mut data = false;
    let mut dot = false;
    loop {
        match chars.get(index) {
            Some('.') => {
                // A dot is a trail unless an alphanumeric follows it.
                if chars
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_alphanumeric())
                {
                    dot = true;
                    index += 1;
                } else {
                    break;
                }
            }
            Some(ch) if *ch == '-' || *ch == '_' || ch.is_ascii_alphanumeric() => {
                data = true;
                index += 1;
            }
            _ => break,
        }
    }
    let last = chars.get(index.wrapping_sub(1)).copied();
    if !(data && dot && last.is_some_and(|ch| ch.is_ascii_alphabetic())) {
        return None;
    }
    let display: String = chars[start..index].iter().collect();
    Some((index, format!("mailto:{display}")))
}

/// `tokenizeDomain`. Returns the end of the domain.
fn scan_domain(chars: &[char], start: usize) -> Option<usize> {
    let mut index = start;
    let mut seen = false;
    let mut underscore_last = false;
    let mut underscore_last_last = false;
    loop {
        let ch = chars.get(index).copied();
        match ch {
            Some('.') | Some('_') => {
                if is_trail(chars, index) {
                    break;
                }
                if ch == Some('_') {
                    underscore_last = true;
                } else {
                    underscore_last_last = underscore_last;
                    underscore_last = false;
                }
                index += 1;
            }
            None => break,
            Some(ch) => {
                if ends_domain(Some(ch)) || (ch != '-' && unicode_punctuation(ch)) {
                    break;
                }
                seen = true;
                index += 1;
            }
        }
    }
    if underscore_last || underscore_last_last || !seen {
        return None;
    }
    Some(index)
}

/// `tokenizePath`. Returns the end of the path.
fn scan_path(chars: &[char], start: usize) -> usize {
    let mut index = start;
    let mut open = 0usize;
    let mut close = 0usize;
    while let Some(ch) = chars.get(index).copied() {
        if ch == '(' {
            open += 1;
            index += 1;
            continue;
        }
        if ch == ')' && close < open {
            close += 1;
            index += 1;
            continue;
        }
        if matches!(
            ch,
            '!' | '"'
                | '&'
                | '\''
                | ')'
                | '*'
                | ','
                | '.'
                | ':'
                | ';'
                | '<'
                | '?'
                | ']'
                | '_'
                | '~'
        ) {
            if is_trail(chars, index) {
                break;
            }
            if ch == ')' {
                close += 1;
            }
            index += 1;
            continue;
        }
        if ends_domain(Some(ch)) {
            break;
        }
        index += 1;
    }
    index
}

/// `tokenizeTrail`: true when everything from `index` to the next end is
/// trailing punctuation that stays outside the link.
fn is_trail(chars: &[char], mut index: usize) -> bool {
    loop {
        match chars.get(index).copied() {
            Some('!' | '"' | '\'' | ')' | '*' | ',' | '.' | ':' | ';' | '?' | '_' | '~') => {
                index += 1;
            }
            Some('&') => {
                index += 1;
                if !chars.get(index).is_some_and(|ch| ch.is_ascii_alphabetic()) {
                    return false;
                }
                while chars.get(index).is_some_and(|ch| ch.is_ascii_alphabetic()) {
                    index += 1;
                }
                if chars.get(index) == Some(&';') {
                    index += 1;
                } else {
                    return false;
                }
            }
            Some(']') => {
                index += 1;
                match chars.get(index).copied() {
                    None | Some('(') | Some('[') => return true,
                    Some(ch) if ends_domain(Some(ch)) => return true,
                    _ => {}
                }
            }
            Some('<') | None => return true,
            Some(ch) if ends_domain(Some(ch)) => return true,
            Some(_) => return false,
        }
    }
}

// ---------------------------------------------------------------------------
// Pass 2: `mdast-util-gfm-autolink-literal`'s find-and-replace.

/// `/(https?:\/\/|www(?=\.))([-.\w]+)([^ \t\r\n]*)/gi` with `findUrl`.
fn regex_url_pass(text: &str) -> Vec<Segment> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut index = 0;
    while index < chars.len() {
        let Some((protocol_len, is_www)) = url_prefix_at(&chars, index) else {
            index += 1;
            continue;
        };
        let domain_start = index + protocol_len;
        let mut domain_end = domain_start;
        while domain_end < chars.len() && is_js_word_dot_dash(chars[domain_end]) {
            domain_end += 1;
        }
        if domain_end == domain_start {
            index += 1;
            continue;
        }
        let mut path_end = domain_end;
        while path_end < chars.len() && !matches!(chars[path_end], ' ' | '\t' | '\r' | '\n') {
            path_end += 1;
        }
        // The regex match is fixed; `findUrl` may still decline it.
        let match_end = path_end;
        let replacement = (|| {
            let prev = if index == 0 {
                None
            } else {
                Some(chars[index - 1])
            };
            if !regex_previous(prev, false) {
                return None;
            }
            let protocol: String = chars[index..domain_start].iter().collect();
            let mut domain: String = chars[domain_start..domain_end].iter().collect();
            let mut prefix = "";
            let mut protocol_text = protocol;
            if is_www {
                domain = format!("{protocol_text}{domain}");
                protocol_text = String::new();
                prefix = "http://";
            }
            if !is_correct_domain(&domain) {
                return None;
            }
            let path: String = chars[domain_end..path_end].iter().collect();
            let (url, trail) = split_url(&format!("{domain}{path}"));
            if url.is_empty() {
                return None;
            }
            Some((
                format!("{prefix}{protocol_text}{url}"),
                format!("{protocol_text}{url}"),
                trail,
            ))
        })();
        match replacement {
            Some((url, display, trail)) => {
                if plain_start < index {
                    out.push(plain(&chars[plain_start..index].iter().collect::<String>()));
                }
                out.push(linked(&display, url));
                if !trail.is_empty() {
                    out.push(plain(&trail));
                }
                plain_start = match_end;
                index = match_end;
            }
            None => {
                // A declined match still consumes its span in `findAndReplace`.
                index = match_end.max(index + 1);
            }
        }
    }
    if plain_start < chars.len() {
        out.push(plain(&chars[plain_start..].iter().collect::<String>()));
    }
    merge_adjacent_plain(out)
}

fn url_prefix_at(chars: &[char], index: usize) -> Option<(usize, bool)> {
    let lower = |offset: usize| chars.get(index + offset).map(|ch| ch.to_ascii_lowercase());
    if lower(0) == Some('h')
        && lower(1) == Some('t')
        && lower(2) == Some('t')
        && lower(3) == Some('p')
    {
        if lower(4) == Some(':') && lower(5) == Some('/') && lower(6) == Some('/') {
            return Some((7, false));
        }
        if lower(4) == Some('s')
            && lower(5) == Some(':')
            && lower(6) == Some('/')
            && lower(7) == Some('/')
        {
            return Some((8, false));
        }
    }
    if lower(0) == Some('w')
        && lower(1) == Some('w')
        && lower(2) == Some('w')
        && lower(3) == Some('.')
    {
        return Some((3, true));
    }
    None
}

/// `[-.\w]` with JavaScript's ASCII `\w`.
fn is_js_word_dot_dash(ch: char) -> bool {
    ch == '-' || ch == '.' || ch == '_' || ch.is_ascii_alphanumeric()
}

/// `previous(match, email)`.
fn regex_previous(prev: Option<char>, email: bool) -> bool {
    let ok = match prev {
        None => true,
        Some(ch) => js_whitespace(ch) || unicode_punctuation(ch),
    };
    ok && (!email || prev != Some('/'))
}

/// `isCorrectDomain`.
fn is_correct_domain(domain: &str) -> bool {
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let bad = |part: &str| {
        !part.is_empty()
            && (part.contains('_') || !part.chars().any(|ch| ch.is_ascii_alphanumeric()))
    };
    if bad(parts[parts.len() - 1]) || bad(parts[parts.len() - 2]) {
        return false;
    }
    true
}

/// `splitUrl`: strip trailing punctuation, giving back balanced parens.
fn split_url(url: &str) -> (String, String) {
    let trail_chars = |ch: char| {
        matches!(
            ch,
            '!' | '"' | '&' | '\'' | ')' | ',' | '.' | ':' | ';' | '<' | '>' | '?' | ']' | '}'
        )
    };
    let trimmed = url.trim_end_matches(trail_chars);
    if trimmed.len() == url.len() {
        return (url.to_string(), String::new());
    }
    let mut kept = trimmed.to_string();
    let mut trail = url[trimmed.len()..].to_string();
    let opening = kept.matches('(').count();
    let mut closing = kept.matches(')').count();
    while let Some(position) = trail.find(')') {
        if opening <= closing {
            break;
        }
        kept.push_str(&trail[..=position]);
        trail = trail[position + 1..].to_string();
        closing += 1;
    }
    (kept, trail)
}

/// `/(?<=^|\s|\p{P}|\p{S})([-.\w+]+)@([-\w]+(?:\.[-\w]+)+)/gu` with `findEmail`.
fn regex_email_pass(text: &str) -> Vec<Segment> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut search = 0;
    let atext = |ch: char| matches!(ch, '-' | '.' | '+' | '_') || ch.is_ascii_alphanumeric();
    let word = |ch: char| ch == '-' || ch == '_' || ch.is_ascii_alphanumeric();
    while let Some(at) = chars[search..]
        .iter()
        .position(|ch| *ch == '@')
        .map(|p| p + search)
    {
        // Leftmost start whose look-behind holds.
        let mut run_start = at;
        while run_start > search && atext(chars[run_start - 1]) {
            run_start -= 1;
        }
        let start = (run_start..at).find(|&p| {
            p == 0 || {
                let prev = chars[p - 1];
                js_whitespace(prev) || unicode_punctuation(prev)
            }
        });
        let Some(start) = start else {
            search = at + 1;
            continue;
        };
        // Domain: `[-\w]+(?:\.[-\w]+)+`.
        let mut end = at + 1;
        let mut segments = 0;
        loop {
            let segment_start = end;
            while end < chars.len() && word(chars[end]) {
                end += 1;
            }
            if end == segment_start {
                if segments > 0 {
                    end = segment_start - 1;
                }
                break;
            }
            segments += 1;
            if chars.get(end) == Some(&'.') {
                end += 1;
                continue;
            }
            break;
        }
        if segments < 2 {
            search = at + 1;
            continue;
        }
        let label: String = chars[at + 1..end].iter().collect();
        let prev = if start == 0 {
            None
        } else {
            Some(chars[start - 1])
        };
        if !regex_previous(prev, true)
            || label.ends_with(|ch: char| ch == '-' || ch == '_' || ch.is_ascii_digit())
        {
            search = end;
            continue;
        }
        if plain_start < start {
            out.push(plain(&chars[plain_start..start].iter().collect::<String>()));
        }
        let display: String = chars[start..end].iter().collect();
        out.push(linked(&display, format!("mailto:{display}")));
        plain_start = end;
        search = end;
    }
    if plain_start < chars.len() {
        out.push(plain(&chars[plain_start..].iter().collect::<String>()));
    }
    merge_adjacent_plain(out)
}

fn merge_adjacent_plain(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for segment in segments {
        if segment.url.is_none() {
            if let Some(last) = out.last_mut().filter(|last| last.url.is_none()) {
                last.text.push_str(&segment.text);
                continue;
            }
        }
        out.push(segment);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(text: &str) -> Vec<(String, Option<String>)> {
        split_autolinks(text, None)
            .into_iter()
            .map(|segment| (segment.text, segment.url))
            .collect()
    }

    #[test]
    fn www_and_protocol_links() {
        assert_eq!(
            links("see www.commonmark.org now"),
            vec![
                ("see ".to_string(), None),
                (
                    "www.commonmark.org".to_string(),
                    Some("http://www.commonmark.org".to_string())
                ),
                (" now".to_string(), None),
            ]
        );
        assert_eq!(
            links("Visit https://encrypted.google.com/search?q=Markup+(business)"),
            vec![
                ("Visit ".to_string(), None),
                (
                    "https://encrypted.google.com/search?q=Markup+(business)".to_string(),
                    Some("https://encrypted.google.com/search?q=Markup+(business)".to_string())
                ),
            ]
        );
    }

    #[test]
    fn trailing_punctuation_and_parens() {
        assert_eq!(
            links("www.google.com/search?q=Markup+(business)))"),
            vec![
                (
                    "www.google.com/search?q=Markup+(business)".to_string(),
                    Some("http://www.google.com/search?q=Markup+(business)".to_string())
                ),
                ("))".to_string(), None),
            ]
        );
        assert_eq!(
            links("www.google.com/search?q=commonmark&hl;"),
            vec![
                (
                    "www.google.com/search?q=commonmark".to_string(),
                    Some("http://www.google.com/search?q=commonmark".to_string())
                ),
                ("&hl;".to_string(), None),
            ]
        );
    }

    #[test]
    fn email_links() {
        assert_eq!(
            links("foo@bar.baz"),
            vec![(
                "foo@bar.baz".to_string(),
                Some("mailto:foo@bar.baz".to_string())
            )]
        );
        assert_eq!(
            links("a.b-c_d@a.b-"),
            vec![("a.b-c_d@a.b-".to_string(), None)]
        );
        assert_eq!(
            links("a.b-c_d@a.b_"),
            vec![("a.b-c_d@a.b_".to_string(), None)]
        );
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(
            links("hello world."),
            vec![("hello world.".to_string(), None)]
        );
        assert_eq!(links("www"), vec![("www".to_string(), None)]);
    }
}
