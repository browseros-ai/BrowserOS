//! Element refs have three spellings in the wild and exactly one on the wire.
//!
//! A snapshot prints `[ref=e12]`, the `act` tool takes `ref: "e12"`, and
//! agent-browser users type `@e12`. All three normalize here so nobody has to
//! think about it, and a ref pasted straight out of a snapshot works unedited.

use crate::exit::Failure;

/// Normalizes `@e12`, `e12` and `[ref=e12]` to `e12`.
///
/// # Errors
/// Returns a usage failure naming all three accepted spellings.
pub fn normalize(raw: &str) -> Result<String, Failure> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix("[ref=")
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(trimmed);
    let bare = inner.strip_prefix('@').unwrap_or(inner);

    let digits = bare.strip_prefix('e').unwrap_or("");
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(bare.to_string());
    }
    Err(Failure::usage(format!(
        "not an element ref: {raw}\n  refs look like e12, and @e12 or [ref=e12] are accepted too.\n  Take one from: browseros snapshot -i"
    )))
}

/// Splits `@e1=value` into its two halves for `fill --fields`.
///
/// # Errors
/// Returns a usage failure when there is no `=`, or when the left side is not a ref.
pub fn field_pair(raw: &str) -> Result<(String, String), Failure> {
    let (left, right) = raw.split_once('=').ok_or_else(|| {
        Failure::usage(format!(
            "not a field assignment: {raw}\n  fields look like @e1=\"some text\""
        ))
    })?;
    Ok((normalize(left)?, right.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_spellings_normalize_to_one() -> anyhow::Result<()> {
        for spelling in ["e12", "@e12", "[ref=e12]", "  @e12  "] {
            assert_eq!(normalize(spelling)?, "e12", "{spelling}");
        }
        Ok(())
    }

    #[test]
    fn a_css_selector_is_a_usage_error_that_says_where_to_get_a_ref() {
        let error = normalize(".row > button").unwrap_or_else(|failure| failure.message);
        assert!(error.contains("snapshot -i"), "{error}");
    }

    #[test]
    fn a_bare_number_is_not_a_ref() {
        assert!(normalize("12").is_err());
        assert!(normalize("e").is_err());
        assert!(normalize("ex1").is_err());
    }

    #[test]
    fn a_field_pair_keeps_everything_after_the_first_equals() -> anyhow::Result<()> {
        let (reference, value) = field_pair("@e3=a=b")?;
        assert_eq!(reference, "e3");
        assert_eq!(value, "a=b");
        Ok(())
    }
}
