//! Neo: Defense in depth for hosts offering CAPTCHA. Provider files must still be unreadable
//! by the browser process for isolation; existing file: documents are outside this URL guard.

use crate::progress::PlaywrightError;
use url::Url;

/// Deviation: `server/frames.ts` `Frame.goto` Neo refuses local-file navigation while its host offers a CAPTCHA solver because file documents could read provider key files.
const MESSAGE: &str = "Neo refuses file: URLs while it offers neo.solveCaptcha: a file: page could read its provider key files";

pub(crate) fn refuses(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        matches!(parsed.scheme(), "file" | "view-source" | "filesystem")
    } else {
        // WHATWG URL parsing removes embedded ASCII tabs/newlines and folds the scheme.
        // Retain that protection even when a malformed URL cannot be parsed in full.
        let clean = url
            .trim()
            .replace(['\t', '\n', '\r'], "")
            .to_ascii_lowercase();
        ["file:", "view-source:", "filesystem:"]
            .iter()
            .any(|scheme| clean.starts_with(scheme))
    }
}

pub(crate) fn check(enabled: bool, url: &str) -> Result<(), PlaywrightError> {
    if enabled && refuses(url) {
        return Err(PlaywrightError::error(MESSAGE));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn captcha_file_navigation_scheme_table() {
        for url in [
            "file:///x",
            "FILE:///x",
            " file:///x",
            "fi\tle:///x",
            "fi\nle:///x",
            "view-source:file:///x",
            "filesystem:file:///x",
            "VIEW-SOURCE:http://x",
            " Fi\rle:???",
        ] {
            assert!(super::refuses(url), "{url:?}");
            assert!(super::check(true, url).is_err());
            assert!(super::check(false, url).is_ok());
        }
        for url in [
            "http://x",
            "https://x",
            "about:blank",
            "data:text/html,x",
            "blob:https://x/id",
            "x",
        ] {
            assert!(!super::refuses(url), "{url:?}");
        }
    }
}
