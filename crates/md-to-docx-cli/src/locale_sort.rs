//! Directory entry order for `--batch`.
//!
//! Node's `localeCompare` in the Jest CLI tests uses the `en-US` collator:
//! letters by base character, then accents, then lowercase before uppercase.
//! Digits stay in code-point order, so `file10` sorts before `file2`.

use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;

pub fn js_locale_compare(left: &str, right: &str) -> Ordering {
    let left = sort_key(left);
    let right = sort_key(right);
    left.primary
        .cmp(&right.primary)
        .then(left.secondary.cmp(&right.secondary))
        .then(left.tertiary.cmp(&right.tertiary))
}

struct SortKey {
    primary: Vec<u32>,
    secondary: Vec<u32>,
    tertiary: Vec<u32>,
}

fn sort_key(text: &str) -> SortKey {
    let chars: Vec<char> = text.nfd().collect();
    let mut key = SortKey {
        primary: Vec::new(),
        secondary: Vec::new(),
        tertiary: Vec::new(),
    };
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if is_combining_mark(ch) {
            key.secondary.push(ch as u32);
            index += 1;
            continue;
        }
        index += 1;
        let mut marks = Vec::new();
        while index < chars.len() && is_combining_mark(chars[index]) {
            marks.push(chars[index] as u32);
            index += 1;
        }
        let (base, case_rank) = primary_and_case(ch);
        key.primary.push(base);
        key.secondary.extend(marks);
        key.secondary.push(0);
        key.tertiary.push(case_rank);
    }
    key
}

fn primary_and_case(ch: char) -> (u32, u32) {
    let folded: String = ch.to_lowercase().collect();
    let base = folded.chars().next().unwrap_or(ch) as u32;
    let rank = if ch.is_uppercase() { 2 } else { 1 };
    (base, rank)
}

fn is_combining_mark(ch: char) -> bool {
    matches!(
        ch,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

#[cfg(test)]
mod tests {
    use super::js_locale_compare;
    use std::cmp::Ordering;

    #[test]
    fn matches_node_en_us_locale_compare() {
        let mut names = vec![
            "two.md", "nested", "One.md", "a.md", "B.md", "file10.md", "file2.md", "é.md", "e.md",
        ];
        names.sort_by(|left, right| js_locale_compare(left, right));
        assert_eq!(
            names,
            vec![
                "a.md", "B.md", "e.md", "é.md", "file10.md", "file2.md", "nested", "One.md",
                "two.md",
            ]
        );
    }

    #[test]
    fn shorter_prefix_sorts_first() {
        assert_eq!(js_locale_compare(".", ".."), Ordering::Less);
        assert_eq!(js_locale_compare("a", "A"), Ordering::Less);
        assert_eq!(js_locale_compare("file10", "file2"), Ordering::Less);
    }
}
