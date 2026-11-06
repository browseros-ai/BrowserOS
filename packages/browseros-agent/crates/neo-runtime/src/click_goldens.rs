//! Playwright's recorded clicks (`neo-sdk/tests/goldens/click-goldens.json`), for unit tests.
//! A golden records what the script saw: the error's name and its full message, call log
//! included. The raw call logs Playwright's server wrote are not recorded, so [`raw_log`]
//! rebuilds them from Playwright's code; the folding tests prove each one folds into the
//! recorded message byte for byte.

use std::env;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

/// One recorded click case.
#[derive(Debug, Deserialize)]
pub(crate) struct Golden {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) error: Option<GoldenError>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GoldenError {
    /// `e.message` as the script read it, call log included.
    pub(crate) message: String,
}

#[derive(Deserialize)]
struct Goldens {
    cases: Vec<Golden>,
}

/// The locator every golden clicks.
const LOCATOR: &str = "getByRole('button', { name: 'Submit' })";

/// The client's prefix on every golden message: each golden calls `locator.click`.
const API_PREFIX: &str = "locator.click: ";

/// Playwright's `formatCallLog` separator between the server's message and the folded log.
const CALL_LOG: &str = "\nCall log:\n";

/// Playwright's waits between click attempts (`dom.ts` `_retryAction`).
const RETRY_WAITS_MS: [u32; 5] = [0, 20, 100, 100, 500];

/// Read at run time through `CARGO_MANIFEST_DIR`, so a test binary from a shared build dir reads
/// this checkout's goldens.
pub(crate) fn golden(name: &str) -> Golden {
    let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let path = crate_dir.join("../neo-sdk/tests/goldens/click-goldens.json");
    let text = fs::read_to_string(path).expect("the click goldens are committed");
    let goldens: Goldens = serde_json::from_str(&text).expect("click-goldens.json parses");
    goldens
        .cases
        .into_iter()
        .find(|golden| golden.name == name)
        .unwrap_or_else(|| panic!("no click golden named {name}"))
}

impl Golden {
    fn error(&self) -> &GoldenError {
        self.error
            .as_ref()
            .unwrap_or_else(|| panic!("the {} golden resolved", self.name))
    }

    /// The folded call log the script saw, one line per entry.
    pub(crate) fn call_log(&self) -> Vec<String> {
        let (_, log) = self
            .error()
            .message
            .split_once(CALL_LOG)
            .unwrap_or_else(|| panic!("the {} golden has a call log", self.name));
        log.strip_suffix('\n')
            .expect("a call log ends with a newline")
            .split('\n')
            .map(str::to_string)
            .collect()
    }

    /// The message Playwright's server replied with: the script's message without the client's
    /// `apiName: ` prefix and call log.
    pub(crate) fn server_message(&self) -> &str {
        let message = self
            .error()
            .message
            .strip_prefix(API_PREFIX)
            .expect("every golden calls locator.click");
        message
            .split_once(CALL_LOG)
            .map_or(message, |(server, _)| server)
    }

    /// The click's `timeout` option, from the golden's source.
    pub(crate) fn timeout_ms(&self) -> Option<f64> {
        let (_, rest) = self.source.split_once("timeout: ")?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }
}

/// The raw call log Playwright's server wrote for a golden, rebuilt from its code at the
/// recorded version:
/// - `frames.ts` `_retryWithProgressIfNotConnected` logs `waiting for <locator>`, then
///   `  locator resolved to <preview>` once the selector matches one element;
/// - `dom.ts` `_retryAction` logs `attempting click action` before the first attempt, and
///   `retrying click action` plus `  waiting <ms>ms` (when nonzero) before each later one;
/// - `_performPointerAction` logs each attempt's lines, ending in why it failed.
///
/// The attempt counts are the ones the recorded messages fold from; each timeout fired while
/// waiting before the next attempt.
pub(crate) fn raw_log(name: &str) -> Vec<String> {
    let waiting = format!("waiting for {LOCATOR}");
    match name {
        "strict" | "hidden" => vec![waiting],
        "covered" => attempts_log(
            waiting,
            "  locator resolved to <button>Submit</button>",
            &[
                "  waiting for element to be visible, enabled and stable",
                "  element is visible, enabled and stable",
                "  scrolling into view if needed",
                "  done scrolling",
                "  <div id=\"overlay\"></div> intercepts pointer events",
            ],
            6,
        ),
        "disabled" => attempts_log(
            waiting,
            "  locator resolved to <button disabled>Submit</button>",
            &[
                "  waiting for element to be visible, enabled and stable",
                "  element is not enabled",
            ],
            5,
        ),
        _ => panic!("no raw log for the {name} golden"),
    }
}

fn attempts_log(waiting: String, resolved: &str, attempt: &[&str], attempts: usize) -> Vec<String> {
    let mut log = vec![waiting, resolved.to_string()];
    for retry in 0..=attempts {
        if retry == 0 {
            log.push("attempting click action".to_string());
        } else {
            log.push("retrying click action".to_string());
            let wait = RETRY_WAITS_MS[(retry - 1).min(RETRY_WAITS_MS.len() - 1)];
            if wait > 0 {
                log.push(format!("  waiting {wait}ms"));
            }
        }
        if retry < attempts {
            log.extend(attempt.iter().map(|line| (*line).to_string()));
        }
    }
    log
}
