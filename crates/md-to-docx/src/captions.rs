//! Caption bookmark ids.
//!
//! `cross_reference_bookmark_id` matches `crossReferenceBookmarkId` in
//! `src/utils/bookmarkUtils.ts`. The hash is FNV-1a over UTF-16 code units.

/// Word bookmark name for a user caption id.
///
/// The label is `sanitizeForBookmarkId` cut to 24 characters. The suffix is
/// the FNV-1a hash of the original id. The whole name is cut to 40 characters.
pub fn cross_reference_bookmark_id(id: &str) -> String {
    let safe: String = sanitize_for_bookmark_id(id).chars().take(24).collect();
    let hash = stable_bookmark_hash(id);
    let full = format!("mdxref_{safe}_{hash}");
    full.chars().take(40).collect()
}

/// `sanitizeForBookmarkId` from `src/utils/bookmarkUtils.ts`.
fn sanitize_for_bookmark_id(text: &str) -> String {
    let mut kept = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || is_js_whitespace(ch) {
            kept.push(ch);
        }
    }
    let mut collapsed = String::new();
    let mut in_space = false;
    for ch in kept.chars() {
        if is_js_whitespace(ch) {
            if !in_space {
                collapsed.push('_');
                in_space = true;
            }
        } else {
            in_space = false;
            collapsed.push(ch);
        }
    }
    let starts_ok = collapsed
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_');
    if !starts_ok {
        collapsed.insert(0, '_');
    }
    collapsed.chars().take(40).collect()
}

/// FNV-1a, offset `0x811c9dc5`, prime `0x01000193`.
///
/// Each step XORs one UTF-16 code unit, then multiplies with the same
/// low-32-bit product as `Math.imul`. The unsigned value is base 36, padded
/// to 7 characters, then cut to the last 7.
fn stable_bookmark_hash(value: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for unit in value.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let encoded = encode_base36(hash);
    let mut padded = String::new();
    for _ in encoded.len()..7 {
        padded.push('0');
    }
    padded.push_str(&encoded);
    let start = padded.len() - 7;
    padded[start..].to_string()
}

fn encode_base36(mut value: u32) -> String {
    const DIGITS: [char; 36] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h',
        'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
    ];
    if value == 0 {
        return "0".to_string();
    }
    let mut out = String::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.chars().rev().collect()
}

fn is_js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::{cross_reference_bookmark_id, sanitize_for_bookmark_id, stable_bookmark_hash};

    #[test]
    fn hash_matches_javascript_fnv1a() {
        assert_eq!(stable_bookmark_hash(""), "0ztntfp");
        assert_eq!(stable_bookmark_hash("fig:overview"), "1ec4keq");
        assert_eq!(stable_bookmark_hash("tbl:results"), "1me3ew5");
        assert_eq!(stable_bookmark_hash("🚀id"), "0wslqqt");
        assert_eq!(
            stable_bookmark_hash(&format!("{}cap", "😀".repeat(3))),
            "0btmbg8"
        );
        assert_eq!(stable_bookmark_hash("[\n\t]"), "18ndb3m");
        assert_ne!(stable_bookmark_hash("[\n\t]"), stable_bookmark_hash(""));
        assert_eq!(
            cross_reference_bookmark_id("fig:overview"),
            "mdxref_figoverview_1ec4keq"
        );
        assert_eq!(
            cross_reference_bookmark_id("tbl:results"),
            "mdxref_tblresults_1me3ew5"
        );
    }

    #[test]
    fn bookmark_id_is_capped_at_forty_characters() {
        let long = "a".repeat(80);
        assert_eq!(sanitize_for_bookmark_id(&long), "a".repeat(40));
        let id = cross_reference_bookmark_id(&long);
        assert_eq!(id, "mdxref_aaaaaaaaaaaaaaaaaaaaaaaa_0mjqhn9");
        assert_eq!(id.chars().count(), 39);
        assert!(id.chars().count() <= 40);

        let digits = "9".repeat(50);
        assert_eq!(
            sanitize_for_bookmark_id(&digits),
            format!("_{}", "9".repeat(39))
        );
        assert_eq!(sanitize_for_bookmark_id(&digits).chars().count(), 40);
        let digit_id = cross_reference_bookmark_id(&digits);
        assert_eq!(digit_id, "mdxref__99999999999999999999999_1ryq0w7");
        assert!(digit_id.chars().count() <= 40);

        let full = "A".repeat(25);
        let cut = "A".repeat(24);
        assert_eq!(
            cross_reference_bookmark_id(&full),
            "mdxref_AAAAAAAAAAAAAAAAAAAAAAAA_0o8msac"
        );
        assert_eq!(
            cross_reference_bookmark_id(&cut),
            "mdxref_AAAAAAAAAAAAAAAAAAAAAAAA_1d8kmv1"
        );
        assert!(cross_reference_bookmark_id(&full).chars().count() <= 40);
    }
}
