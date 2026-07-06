//! The JSONC front-end: JSON with `//` line and `/* … */` block comments. Comments are stripped to
//! whitespace-preserving JSON, then parsed by the [`json`](crate::loader::json) loader, so a `.jsonc`
//! document lands in exactly the same [`serde_json::Value`] model as its `.json` twin.
//!
//! The stripper is **string-aware**: a `//` or `/*` inside a double-quoted string literal (e.g. the
//! `//` in `"https://example.com"`) is data, not a comment, so it must survive verbatim. Escapes
//! inside a string (`\"`) are tracked so an escaped quote does not prematurely close the string.

use serde_json::Value;

use tmx_core::error::RunError;

/// Parse `text` as JSONC: strip comments, then parse the residue as JSON.
pub(crate) fn parse(text: &str) -> Result<Value, RunError> {
    super::json::parse(&strip_comments(text))
}

/// Strip `//` line comments and `/* … */` block comments from `src`, leaving string literals intact.
///
/// Line comments collapse to a bare newline (preserving line counts for legible parse errors); block
/// comments collapse to nothing. A `/` that is not the start of a comment, and any character inside a
/// string, is copied verbatim.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push('"');
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            // Line comment: consume to the end of the line, keeping the newline.
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            // Block comment: consume to the closing `*/`.
            chars.next();
            let mut prev = '\0';
            for next in chars.by_ref() {
                if prev == '*' && next == '/' {
                    break;
                }
                prev = next;
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_and_block_comments_but_keeps_string_slashes() {
        // A `//` and a `/* */` outside strings are comments; the `//` inside the URL string is data.
        let src = r#"{
            // a leading line comment
            "url": "https://example.com", /* trailing block */
            "n": 1 // eol comment
        }"#;
        let value = parse(src).expect("jsonc parses after stripping");
        assert_eq!(
            value["url"], "https://example.com",
            "the // inside the string literal must survive stripping"
        );
        assert_eq!(
            value["n"], 1,
            "the value before an eol comment is preserved"
        );
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_string_early() {
        // The `\"` keeps us inside the string, so the following `//` is still string data, not a
        // comment — a stripper that mishandled the escape would truncate the value.
        let src = r#"{ "quote": "he said \"hi//there\"" }"#;
        let value = parse(src).expect("jsonc with an escaped quote parses");
        assert_eq!(
            value["quote"], r#"he said "hi//there""#,
            "an escaped quote keeps the // inside the string as data"
        );
    }

    #[test]
    fn a_malformed_residue_is_a_typed_error_not_a_panic() {
        // Stripping the comment leaves `{ "a": }` — invalid JSON — which must surface as a typed
        // parse error (negative space), never a panic.
        let err = parse("{ \"a\": /* oops */ }").expect_err("dangling value is invalid");
        assert_eq!(err.code, "source_parse_error", "a typed parse error");
        assert_eq!(
            err.category,
            tmx_core::error::ErrorCategory::Validation,
            "a parse failure is a validation-category error"
        );
    }
}
