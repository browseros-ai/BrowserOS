//! Which pages Neo leaves alone, read from their URL's scheme, for the host's door and the
//! target watcher alike. The door (neo-server's `refused_page`) refuses an op on a tab that shows
//! such a page, and the watcher sends such a tab nothing of a page domain
//! (`events::target_watcher`). One rule in one place, so the two never disagree.

/// The scheme of `url` as the URL Standard reads it: leading and trailing C0 controls and spaces
/// dropped, tabs and newlines removed wherever they are, then an ASCII letter and letters,
/// digits, `+`, `-` or `.` up to the first `:`, lowercased. `None` when there is none. Chromium
/// trims, strips and lowercases the same way, so ` JavaScript:` and `java\tscript:` are
/// `javascript:` to it too; a scheme with any other character is no URL it navigates to.
#[must_use]
pub fn url_scheme(url: &str) -> Option<String> {
    let mut scheme = String::new();
    let chars = url
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'));
    for c in chars {
        match c {
            ':' if !scheme.is_empty() => return Some(scheme),
            'a'..='z' | 'A'..='Z' => scheme.push(c.to_ascii_lowercase()),
            '0'..='9' | '+' | '-' | '.' if !scheme.is_empty() => scheme.push(c),
            _ => return None,
        }
    }
    None
}

/// Whether Neo refuses a page at `url`: a `file:` page, however its scheme is spelled (decision
/// of 2026-11-12). A script cannot open one, the door refuses an op on a tab that shows one, and
/// the target watcher sends such a tab nothing of a page domain.
#[must_use]
pub fn refuses_page(url: &str) -> bool {
    url_scheme(url).as_deref() == Some("file")
}

#[cfg(test)]
mod tests {
    #[test]
    fn tests_that_a_scheme_is_read_as_chromium_reads_it() {
        for (url, scheme) in [
            ("https://bank.example/", Some("https")),
            ("HTTP://bank.example/", Some("http")),
            (" \u{1}java\tscr\nipt:void(0)", Some("javascript")),
            ("view-source:https://bank.example/", Some("view-source")),
            ("about:blank", Some("about")),
            ("bank.example/path", None),
            ("1http://bank.example/", None),
            ("", None),
        ] {
            assert_eq!(super::url_scheme(url).as_deref(), scheme, "{url:?}");
        }
    }

    #[test]
    fn tests_that_only_a_file_page_is_refused_however_its_scheme_is_spelled() {
        for url in [
            "file:///etc/hosts",
            "FILE:///etc/hosts",
            " \u{1}fi\tl\ne:///etc/hosts",
        ] {
            assert!(super::refuses_page(url), "{url:?} is refused");
        }
        for url in [
            "https://bank.example/file:///etc/hosts",
            "about:blank",
            "data:text/html,x",
            "",
        ] {
            assert!(!super::refuses_page(url), "{url:?} is not refused");
        }
    }
}
