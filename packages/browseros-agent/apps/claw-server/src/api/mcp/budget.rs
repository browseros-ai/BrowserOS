//! The `run` reply budget: how many tokens one reply's JSON may cost, estimated from its
//! UTF-8 bytes, and the per-call `maxResponseTokens` override. `output.rs` spends it.

use serde_json::Value;

/// Neo: reply budget when a call names none. The owner's choice; clients that cap tool output
/// lower (Claude Code's `MAX_MCP_OUTPUT_TOKENS` defaults to 25 000) pass `maxResponseTokens`.
pub(super) const DEFAULT_TOKENS: usize = 100_000;
/// Neo: smallest `maxResponseTokens`. Its 4 000 bytes still hold page identities, an error
/// name and continuation links beside some text.
pub(super) const MIN_TOKENS: usize = 1_000;
/// Neo: largest `maxResponseTokens`, the owner's "200K context length".
pub(super) const MAX_TOKENS: usize = 200_000;
/// Neo: rough token math, `tokens = ceil(UTF-8 bytes / 4)`. English prose, JSON and aria
/// snapshots run near 4 bytes per token; denser text (CJK, base64, minified code) costs more
/// tokens per byte, so a client close to its own output cap should ask for less.
pub(super) const BYTES_PER_TOKEN: usize = 4;
/// Neo: smallest `limit` window. The ancestor `context` and window metadata must still leave
/// room for text; it is also the smallest budget's size.
pub(super) const LIMIT_MIN: usize = 4_000;
/// Neo: largest `limit` window (the med-fix-b decision asked for at least 100 000 bytes).
/// `limit` only narrows a window below the budget; raise `maxResponseTokens` for a wider one.
pub(super) const LIMIT_MAX: usize = 100_000;
/// Neo: the reply size the excerpt caps in `output.rs` were tuned for, the old fixed
/// 24 000-byte budget. Smaller budgets shrink them in proportion; larger ones keep them.
const TUNED_BYTES: usize = 24_000;

/// Estimated tokens of `text`, the inverse of [`Budget::bytes`].
pub(super) fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(BYTES_PER_TOKEN)
}

/// One call's reply budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Budget {
    tokens: usize,
    /// The call named it, so every `next` the reply returns repeats it: following `next`
    /// verbatim keeps the budget.
    requested: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            tokens: DEFAULT_TOKENS,
            requested: false,
        }
    }
}

impl Budget {
    /// Reads a call's `maxResponseTokens`: absent is the default; anything but an integer in
    /// `MIN_TOKENS..=MAX_TOKENS` is refused, never clamped.
    pub(super) fn parse(value: Option<&Value>) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        match value
            .as_u64()
            .and_then(|tokens| usize::try_from(tokens).ok())
        {
            Some(tokens) if (MIN_TOKENS..=MAX_TOKENS).contains(&tokens) => Ok(Self {
                tokens,
                requested: true,
            }),
            _ => Err(format!(
                "maxResponseTokens must be an integer from {MIN_TOKENS} to {MAX_TOKENS}, got {value}"
            )),
        }
    }

    /// Largest reply in bytes: `estimate_tokens(reply) <= tokens` exactly when
    /// `reply.len() <= bytes()`.
    pub(super) fn bytes(self) -> usize {
        self.tokens * BYTES_PER_TOKEN
    }

    /// Whether `reply`, compact JSON, fits this budget.
    pub(super) fn fits(self, reply: &str) -> bool {
        estimate_tokens(reply) <= self.tokens
    }

    /// The token count to repeat in `next` arguments, when the call named one.
    pub(super) fn requested(self) -> Option<usize> {
        self.requested.then_some(self.tokens)
    }

    /// Byte cap of one retrieval window: the whole budget, or a narrower `limit`. A `limit`
    /// outside its range or above the budget is refused, never clamped.
    pub(super) fn window(self, limit: Option<usize>) -> Result<usize, String> {
        let Some(limit) = limit else {
            return Ok(self.bytes());
        };
        if !(LIMIT_MIN..=LIMIT_MAX).contains(&limit) {
            return Err(format!("limit must be {LIMIT_MIN}..{LIMIT_MAX} bytes"));
        }
        if limit > self.bytes() {
            return Err(format!(
                "limit {limit} bytes exceeds the reply budget of {} bytes (maxResponseTokens {} x {BYTES_PER_TOKEN}); lower limit or raise maxResponseTokens",
                self.bytes(),
                self.tokens
            ));
        }
        Ok(limit)
    }
}

/// `cap`, an excerpt cap tuned for a 24 000-byte reply, for a reply of at most `bytes`: shrunk
/// in proportion below that, unchanged above it, so excerpts never crowd out a small budget.
pub(super) fn scaled(cap: usize, bytes: usize) -> usize {
    cap.min(cap * bytes / TUNED_BYTES)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "Assertions fail the test on malformed results"
)]
mod tests {
    use serde_json::{Value, json};

    use super::{Budget, DEFAULT_TOKENS, LIMIT_MAX, LIMIT_MIN, estimate_tokens, scaled};

    #[test]
    fn tests_that_tokens_are_utf8_bytes_over_four_rounded_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        // Multibyte text counts bytes, not characters: 🦀 is 4 bytes, 漢字 is 6.
        assert_eq!(estimate_tokens("🦀"), 1);
        assert_eq!(estimate_tokens("漢字"), 2);
        assert_eq!(estimate_tokens(&"x".repeat(400_001)), 100_001);
    }

    #[test]
    fn tests_that_max_response_tokens_is_validated_without_clamping() {
        let default = Budget::parse(None).expect("absent is the default");
        assert_eq!(default, Budget::default());
        assert_eq!(default.bytes(), DEFAULT_TOKENS * 4);
        assert_eq!(default.requested(), None);
        assert!(default.fits(&"x".repeat(400_000)) && !default.fits(&"x".repeat(400_001)));
        for tokens in [1_000, 25_000, 200_000] {
            let budget = Budget::parse(Some(&json!(tokens))).expect("in range");
            assert_eq!(budget.requested(), Some(tokens));
            assert_eq!(budget.bytes(), tokens * 4);
        }
        for bad in [
            json!(999),
            json!(200_001),
            json!(0),
            json!(-1),
            json!(1.5),
            json!(1000.0),
            json!("1000"),
            json!(true),
            json!([1000]),
            Value::Null,
        ] {
            let error = Budget::parse(Some(&bad)).expect_err("refused");
            assert!(
                error.contains("maxResponseTokens must be an integer from 1000 to 200000"),
                "{error}"
            );
        }
    }

    #[test]
    fn tests_that_a_window_is_the_budget_or_a_narrower_limit() {
        let small = Budget::parse(Some(&json!(1_000))).expect("in range");
        assert_eq!(small.window(None), Ok(4_000));
        assert_eq!(small.window(Some(LIMIT_MIN)), Ok(LIMIT_MIN));
        let error = small.window(Some(50_000)).expect_err("above the budget");
        assert!(
            error.contains("50000") && error.contains("4000") && error.contains("1000"),
            "{error}"
        );
        let default = Budget::default();
        assert_eq!(default.window(None), Ok(400_000));
        assert_eq!(default.window(Some(LIMIT_MAX)), Ok(LIMIT_MAX));
        for bad in [LIMIT_MIN - 1, LIMIT_MAX + 1, 0] {
            assert!(default.window(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn tests_that_excerpt_caps_shrink_only_below_the_tuned_size() {
        assert_eq!(scaled(2_000, 24_000), 2_000);
        assert_eq!(scaled(2_000, 800_000), 2_000);
        assert_eq!(scaled(4_000, 4_000), 666);
        assert_eq!(scaled(20, 4_000), 3);
    }
}
