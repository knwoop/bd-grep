//! Matching and snippet extraction for `bd-grep`.
//!
//! Input is one issue per line as produced by `bd export`. Fields that are
//! empty are omitted from the export, so every field lookup is optional.

use regex::{Regex, RegexBuilder};
use serde_json::Value;

/// Text fields searched, in the order they are tried. Comments come last.
pub const FIELDS: [&str; 6] = [
    "title",
    "description",
    "design",
    "acceptance_criteria",
    "notes",
    "close_reason",
];

/// Characters of context kept on each side of a match.
pub const CONTEXT: usize = 30;

/// The first field of an issue that matched, with a one-line snippet.
#[derive(Debug, PartialEq, Eq)]
pub struct Hit {
    pub field: &'static str,
    pub snippet: String,
}

/// Builds a case-insensitive matcher. Without `regex`, `pattern` is a fixed string.
pub fn build_regex(pattern: &str, regex: bool) -> Result<Regex, regex::Error> {
    let source = if regex {
        pattern.to_owned()
    } else {
        regex::escape(pattern)
    };
    RegexBuilder::new(&source).case_insensitive(true).build()
}

/// Returns true for issue records. Other record types (memories, infra
/// beads) only appear with `bd export --all`.
pub fn is_issue(record: &Value) -> bool {
    record
        .get("_type")
        .and_then(Value::as_str)
        .is_none_or(|t| t == "issue")
}

/// Returns a string field, or "" when it is missing or not a string.
pub fn str_field<'a>(record: &'a Value, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Finds the first field of `issue` that matches `re`.
pub fn find_hit(issue: &Value, re: &Regex) -> Option<Hit> {
    let fields = FIELDS.iter().map(|&f| (f, str_field(issue, f)));
    let comments = issue
        .get("comments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| ("comments", str_field(c, "text")));

    fields.chain(comments).find_map(|(field, text)| {
        let m = re.find(text)?;
        Some(Hit {
            field,
            snippet: snippet(text, m.start(), m.end(), CONTEXT),
        })
    })
}

/// Cuts `text[start..end]` plus up to `ctx` characters on each side, marks
/// truncated ends with "…", and flattens it to one line. `start` and `end`
/// must be char boundaries; context is counted in chars, so multibyte text
/// is never split.
pub fn snippet(text: &str, start: usize, end: usize, ctx: usize) -> String {
    let from = if ctx == 0 {
        start
    } else {
        text[..start]
            .char_indices()
            .rev()
            .nth(ctx - 1)
            .map_or(0, |(i, _)| i)
    };
    let to = text[end..]
        .char_indices()
        .nth(ctx)
        .map_or(text.len(), |(i, _)| end + i);

    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    out.push_str(&text[from..to]);
    if to < text.len() {
        out.push('…');
    }
    flatten(&out)
}

/// Replaces each run of tabs, CRs and newlines with a single space so the
/// value fits in one TSV cell.
pub fn flatten(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_break = false;
    for c in s.chars() {
        if matches!(c, '\t' | '\r' | '\n') {
            if !in_break {
                out.push(' ');
            }
            in_break = true;
        } else {
            out.push(c);
            in_break = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hit(issue: &Value, pattern: &str, regex: bool) -> Option<Hit> {
        find_hit(issue, &build_regex(pattern, regex).unwrap())
    }

    #[test]
    fn fixed_string_is_case_insensitive_and_literal() {
        let issue = json!({"title": "Fix a.b(c) PARSER"});
        assert!(hit(&issue, "parser", false).is_some());
        assert!(hit(&issue, "a.b(c)", false).is_some());
        assert!(hit(&issue, "axb", false).is_none());
        assert!(hit(&issue, "a.b", true).is_some());
    }

    #[test]
    fn first_field_in_order_wins() {
        let issue = json!({
            "notes": "kite",
            "description": "a kite",
            "comments": [{"text": "kite"}],
        });
        assert_eq!(hit(&issue, "kite", false).unwrap().field, "description");
    }

    #[test]
    fn comments_are_searched_after_fields() {
        let issue = json!({"title": "x", "comments": [{"text": "no"}, {"text": "the KITE"}]});
        let h = hit(&issue, "kite", false).unwrap();
        assert_eq!(h.field, "comments");
        assert_eq!(h.snippet, "the KITE");
    }

    #[test]
    fn missing_and_non_string_fields_are_skipped() {
        let issue = json!({"title": "x", "notes": null, "design": 3, "comments": [{"id": 1}]});
        assert!(hit(&issue, "3", false).is_none());
    }

    #[test]
    fn snippet_keeps_multibyte_chars_whole() {
        let text = "あいうえおかきくけこ検索さしすせそ";
        let start = text.find("検索").unwrap();
        let s = snippet(text, start, start + "検索".len(), 3);
        assert_eq!(s, "…くけこ検索さしす…");
    }

    #[test]
    fn snippet_without_truncation_has_no_ellipsis() {
        assert_eq!(snippet("ab kite cd", 3, 7, 30), "ab kite cd");
    }

    #[test]
    fn snippet_is_flattened() {
        assert_eq!(snippet("a\r\n\tb kite\nc", 6, 10, 30), "a b kite c");
    }

    #[test]
    fn non_issue_records_are_detected() {
        assert!(is_issue(&json!({"_type": "issue"})));
        assert!(is_issue(&json!({})));
        assert!(!is_issue(&json!({"_type": "memory"})));
    }
}
