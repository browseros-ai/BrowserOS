//! One pure scan of JSON text, shared by the engine’s reply reader and the runtime’s settle
//! path. It lives below both callers so a runtime can bound a value without depending on a
//! browser-driving crate; each caller owns its own cap, and this module makes no browser call.

/// The deepest `[`/`{` nesting in `text` outside its strings, in one pass over the bytes. A
/// multi-byte character never holds a quote, a backslash or a bracket byte.
#[must_use]
pub fn json_value_depth(text: &str) -> usize {
    let (mut depth, mut deepest) = (0_usize, 0_usize);
    let (mut in_string, mut escaped) = (false, false);
    for byte in text.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

#[cfg(test)]
mod tests {
    use super::json_value_depth;

    #[test]
    fn tests_that_depth_counts_brackets_outside_strings_only() {
        assert_eq!(json_value_depth(r#""[[{{""#), 0);
        assert_eq!(json_value_depth(r#"["a\"[[", {"b": ["]"]}]"#), 3);
        assert_eq!(json_value_depth("42"), 0);
    }

    #[test]
    fn scans_deep_text_without_a_recursive_value_walk() {
        for depth in [0, 1, 297, 298, 10_000] {
            let text = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(json_value_depth(&text), depth);
        }
    }
}
