//! Locator input for live tests, written by hand the way the prelude will send it: each part
//! is a vendored `ParsedSelectorPart` (`{ name, body, source }`, a CSS body parsed into its
//! simple-selector list), and `internal:control=enter-frame` splits the parts into one chunk
//! per frame, as the vendored `splitSelectorByFrame` does. `frames_live.rs` checks these
//! shapes against the vendored parser's own output.

use serde_json::{Value, json};

// The tab type of the live helper the test drives, `live_browser`'s or `live_run`'s, as the
// test's root imports it. Both name a tab by the page brand's `id` and `tab`.
use crate::TestPage;

/// A part whose body is the source, as every non-CSS engine's is.
fn part(name: &str, body: &str) -> Value {
    json!({ "name": name, "body": body, "source": body })
}

/// `css=<selector>`, for a selector without Playwright's own pseudo-classes.
pub(crate) fn css(selector: &str) -> Value {
    json!({
        "name": "css",
        "body": [{ "simples": [{ "selector": { "css": selector, "functions": [] }, "combinator": "" }] }],
        "source": selector,
    })
}

/// `internal:attr=<body>`, as `getByTitle` and friends build it: `[title="a1"s]`.
pub(crate) fn attr(body: &str) -> Value {
    part("internal:attr", body)
}

/// `internal:role=<body>`, as `getByRole` builds it: `button[name="Go"i]`.
pub(crate) fn role(body: &str) -> Value {
    part("internal:role", body)
}

/// `internal:text=<body>`, as `getByText` builds it: `"Go"i`.
pub(crate) fn text(body: &str) -> Value {
    part("internal:text", body)
}

/// `aria-ref=<ref>`, as `getByRef` builds it.
pub(crate) fn aria_ref(body: &str) -> Value {
    part("aria-ref", body)
}

/// `internal:control=enter-frame`, which `frameLocator` and `contentFrame()` add.
pub(crate) fn enter_frame() -> Value {
    part("internal:control", "enter-frame")
}

/// The locator target `{"$neo":"locator",...}` on `page` over `parts`, split into chunks at
/// each enter-frame part.
pub(crate) fn locator(page: &TestPage, parts: &[Value], display: &str) -> Value {
    let mut chunks = vec![Vec::new()];
    for part in parts {
        if *part == enter_frame() {
            chunks.push(Vec::new());
        } else if let Some(chunk) = chunks.last_mut() {
            chunk.push(part.clone());
        }
    }
    let chunks: Vec<Value> = chunks
        .into_iter()
        .map(|parts| json!({ "parts": parts }))
        .collect();
    let page = json!({ "$neo": "page", "tab": page.tab.0 });
    json!({ "$neo": "locator", "page": page, "chunks": chunks, "display": display })
}
