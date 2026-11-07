//! A fixture port of several Playwright sources, for the provenance gate's own tests.
//!
//! Playwright: `server/frames.ts` `Frame.click`
//! Playwright: `isomorphic/yaml.ts` `yamlStringNeedsQuotes`

use std::time::Duration;

/// The waits between click attempts.
/// Playwright: `server/frames.ts` `Frame.click`: `const waitTime = [0, 20, 100, 100, 500];`
const LOOP_B_WAITS_MS: [u64; 5] = [0, 20, 100, 100, 500];

/// Strings YAML reads as a boolean, null or `~`.
/// Playwright: `isomorphic/yaml.ts` `yamlStringNeedsQuotes`: `['y', 'n', 'yes', 'no', 'true', 'false', 'on', 'off', 'null', '~']`
const YAML_WORDS: [&str; 10] = [
    "y", "n", "yes", "no", "true", "false", "on", "off", "null", "~",
];

/// The most clicks one call may ask for.
/// Deviation: `server/frames.ts` `Frame.click`: `for (let retry = 0; ; ++retry) {`
/// Playwright has no cap; a script's `clickCount: 1e9` must not take the server's memory
/// with it.
const MAX_CLICK_COUNT: u32 = 1000;

/// Neo: how long a detach may take before the session is dropped anyway; no script waits on
/// it.
const DETACH_TIMEOUT: Duration = Duration::from_secs(1);

/// Unexplained: why the realm gets 64 MiB; no measurement is recorded.
const MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// Neo: the page script the fixture would install. Its quotes pair up only as a raw string's,
/// so a lexer that misreads raw strings sees `const hidden` as an untagged Rust constant.
const SCRIPT: &str = r#"
const label = "a;
const hidden: number = 1;
";
"#;

/// Platform: JavaScript prints lowercase hex digits (`Number.prototype.toString(16)`).
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Frame.click's retry loop, cut to one attempt.
/// Playwright: `server/frames.ts` `Frame.click`
/// Playwright: `protocol/spec/frame.yml` `Frame.commands.click`: `selector: string`
pub(crate) fn click(progress: &Progress, trial: bool) -> Result<(), PlaywrightError> {
    let trial = if trial { " (trial run)" } else { "" };
    progress.log(format!("attempting click action{trial}"));
    progress.log("attempting click action (trial run)");
    progress.log(format!("  waiting {}ms", LOOP_B_WAITS_MS[1]));
    // Deviation: `server/frames.ts` `Frame.click`: `for (let retry = 0; ; ++retry) {`
    // The fixture tries once and never retries.
    progress.log("tried once, without retrying");
    Err(PlaywrightError::error("Element is not visible"))
}

/// Playwright: `server/frames.ts` `Frame`
impl FixtureFrame {
    /// Playwright: `server/frames.ts` `Frame.ariaSnapshot`: `return { snapshot: renderAriaSnapshotAsYaml(snapshot) };`
    pub(crate) fn aria_snapshot(&self) -> String {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Playwright: `server/frames.ts` `Frame.nope`: `not checked in test code`
    const UNTAGGED: u32 = 7;

    #[test]
    fn waits() {
        let _ = Duration::from_millis(5);
    }
}
