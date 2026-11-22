//! The proxy rule for every child the server starts: loopback goes direct, the rest keeps the
//! person's proxy.
//!
//! Both children talk to this machine over loopback HTTP with clients that follow the proxy
//! variables: the engine finds the browser's CDP at `127.0.0.1`, `localhost` or `[::1]`, and a
//! coding agent calls the private MCP endpoint at `127.0.0.1`. A proxy elsewhere cannot reach
//! this machine's loopback, so with `HTTP_PROXY` or `ALL_PROXY` set and loopback missing from
//! `NO_PROXY`, the engine never links and the coding agent has no browser. The proxy variables
//! themselves stay: the engine's CAPTCHA providers and a coding agent's model calls are outside
//! traffic.

/// The hosts a child must reach directly, as `NO_PROXY` entries. `localhost` also covers
/// `*.localhost`; `::1` matches `[::1]`. `*` would not do: reqwest applies it to names only.
const LOOPBACK: [&str; 3] = ["127.0.0.1", "localhost", "::1"];

/// `NO_PROXY` and `no_proxy` for a child, for `Command::envs`: the entries this process has in
/// either variable, then the loopback hosts that are missing.
///
/// Both get one value because readers disagree on which comes first: reqwest reads `NO_PROXY`
/// and stops at the first that is set, even when empty, while curl, undici and Python read
/// `no_proxy` first. Taking the entries of both keeps every host that went direct for any of
/// them.
pub(crate) fn loopback_no_proxy() -> [(&'static str, String); 2] {
    let value = merged(
        std::env::var("NO_PROXY").ok().as_deref(),
        std::env::var("no_proxy").ok().as_deref(),
    );
    [("NO_PROXY", value.clone()), ("no_proxy", value)]
}

fn merged(upper: Option<&str>, lower: Option<&str>) -> String {
    let mut entries: Vec<&str> = Vec::new();
    let inherited = upper.into_iter().chain(lower).flat_map(|v| v.split(','));
    for entry in inherited.map(str::trim).chain(LOOPBACK) {
        if !entry.is_empty() && !entries.iter().any(|e| e.eq_ignore_ascii_case(entry)) {
            entries.push(entry);
        }
    }
    entries.join(",")
}

#[cfg(test)]
mod tests {
    use super::merged;

    #[test]
    fn loopback_joins_the_entries_of_both_variables_once() {
        for (upper, lower, expected) in [
            (None, None, "127.0.0.1,localhost,::1"),
            (Some(""), None, "127.0.0.1,localhost,::1"),
            (
                Some("corp.example"),
                Some(".internal, corp.example"),
                "corp.example,.internal,127.0.0.1,localhost,::1",
            ),
            (
                None,
                Some(" LOCALHOST ,10.0.0.0/8,,"),
                "LOCALHOST,10.0.0.0/8,127.0.0.1,::1",
            ),
            (Some("*"), None, "*,127.0.0.1,localhost,::1"),
        ] {
            assert_eq!(merged(upper, lower), expected, "{upper:?} {lower:?}");
        }
    }
}
