//! What an op acts on, as the prelude sends it. Decoded once at the op boundary with
//! `deny_unknown_fields`; everything downstream trusts these types, and a bad input is
//! `script / invalid_input` before anything is sent.
//!
//! The prelude builds Playwright's selector string with the vendored `locatorUtils` (byte for
//! byte what Playwright's client builds), splits it with the vendored `splitSelectorByFrame`
//! and sends the **parsed** chunks. Rust never parses selector syntax. It reads one fact from
//! each chunk, whether its first part is a ref (`aria-ref=f7e3` or `aria-ref=e5`), which makes
//! the resolver jump to the document that printed it, as upstream's
//! `_jumpToAriaRefFrameIfNeeded` does.
//!
//! Why parsed chunks and not chunk strings: `parseSelector` on the whole selector strips a
//! nested selector's frame prefix when it equals the outer one (`frameLocator('#f')
//! .locator('tr').filter({ has: frameLocator('#f').getByText('Dana') })`). A chunk string
//! parsed on its own keeps the prefix, and its nested `enter-frame` then never matches.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the resolver and the families decode their targets with these"
    )
)]

use std::{fmt, iter};

use browseros_core::{FrameId, TabId};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::{self, RawValue};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::world::{BackendNodeId, FrameSeq};

/// A tab-only page handle. The host resolves a lazy `{tab:null}` before invoking the crate;
/// null or an obsolete `id` field therefore fails decoding here, before any input is sent.
/// The admitted core id is looked up for one operation and never retained in this brand.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "PageHandle")]
pub(crate) struct PageRef {
    #[serde(rename = "$neo")]
    #[ts(type = "'page'")]
    _brand: PageBrand,
    // The door replaces the unopened page's null before Rust decodes it. TypeScript must
    // accept that input while Rust continues requiring the admitted tab id here.
    #[ts(type = "number | null")]
    tab: TabId,
}

impl PageRef {
    /// The browser tab this handle names.
    pub(crate) fn tab(&self) -> TabId {
        self.tab
    }
}

/// The only brand a page handle carries.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
enum PageBrand {
    #[serde(rename = "page")]
    Page,
}

/// What an element op acts on.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "$neo", rename_all = "lowercase")]
pub(crate) enum Target {
    /// Re-resolved on every attempt, as a Playwright `Locator` is.
    Locator(LocatorTarget),
    /// One node pinned in one document, as a Playwright `ElementHandle` is.
    Element(ElementTarget),
}

impl Target {
    /// The page the target is on, whichever kind of target it is.
    pub(crate) fn page(&self) -> &PageRef {
        match self {
            Self::Locator(locator) => locator.page(),
            Self::Element(element) => element.page(),
        }
    }
}

/// `{"$neo":"locator","page":{"$neo":"page","tab":1234},"frame":null,"chunks":[...],
/// "display":"getByRole('button')"}`.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocatorTarget {
    /// The page the locator was built on.
    pub(crate) page: PageRef,
    /// The frame the locator was built from (`frame.locator(..)`); `None` is the main frame.
    #[serde(default)]
    pub(crate) frame: Option<FrameId>,
    /// One chunk per frame. Every chunk but the last selects the `<iframe>` to enter.
    #[ts(type = "import('../../vendored').ParsedSelector[]")]
    pub(crate) chunks: NonEmpty<Chunk>,
    /// The element handle a query is rooted at (`elementHandle.$(..)`, `elementHandle.$$(..)`):
    /// the first chunk runs from that node, in its own document, and `frame` plays no part. A
    /// leading ref jumps away from it, as upstream drops the scope once the frame changes.
    #[serde(default)]
    #[ts(optional)]
    pub(crate) root: Option<RootHandle>,
    /// Playwright's `asLocator` rendering, for messages and the call log.
    pub(crate) display: String,
}

impl LocatorTarget {
    /// The page the locator is on. Ops read it here rather than from the field, so the page's
    /// wire shape can change in one place.
    pub(crate) fn page(&self) -> &PageRef {
        &self.page
    }
}

/// The handle a query is rooted at, with its `$neo` tag, which must say `element`.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "$neo", rename_all = "lowercase")]
#[ts(rename = "ElementBrand")]
pub(crate) enum RootHandle {
    Element(ElementTarget),
}

/// `{"$neo":"element","page":{"$neo":"page","tab":1234},"seq":4,"node":1234,
/// "display":"<button>Save</button>"}`. Once document `seq` is gone, the handle is detached for
/// good.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct ElementTarget {
    /// The page the element was pinned on.
    // An element is already pinned to an opened page; unlike locator inputs its page can
    // never be the lazy null brand. Reuse the output-side page type for this stronger promise.
    #[ts(as = "crate::page::PageHandle")]
    pub(crate) page: PageRef,
    pub(crate) seq: FrameSeq,
    pub(crate) node: BackendNodeId,
    /// Playwright's preview of the element, for messages.
    pub(crate) display: String,
}

impl ElementTarget {
    /// The page the handle's node is on; see [`LocatorTarget::page`].
    pub(crate) fn page(&self) -> &PageRef {
        &self.page
    }
}

/// One frame's part of a selector: a vendored `ParsedSelector`, passed to the page untouched.
#[derive(Clone, Debug)]
pub(crate) struct Chunk {
    /// The `ParsedSelector` JSON, which the probe hands to `InjectedScript.querySelectorAll`.
    pub(crate) parsed: Box<RawValue>,
    /// Set when the chunk's first part is a ref.
    pub(crate) jump: Option<AriaRefJump>,
}

/// A ref that leads a frame's selector. The resolver goes straight to the document that
/// printed it, so a ref names the element it was printed for or fails at once.
///
/// Neo also jumps for a bare `e<n>`, which upstream resolves in the current frame: in Neo a
/// bare ref is seq 0, the main frame's first document, and once that document is gone the ref
/// must fail (`element_detached`) instead of being looked up in whatever the frame shows now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AriaRefJump {
    /// The document that printed the ref.
    pub(crate) seq: FrameSeq,
    /// The element's number in that document.
    pub(crate) element: u32,
}

impl AriaRefJump {
    /// Reads a ref exactly as Neo prints one: `e<n>` or `f<seq>e<n>`, decimal, with no sign,
    /// space or leading zero. Seq 0 prints bare, so `f0e5` is not a ref, and refs count from
    /// `e1`. Anything else names no element any snapshot printed.
    pub(crate) fn parse(body: &str) -> Option<Self> {
        let (seq, element) = match body.strip_prefix('f') {
            Some(rest) => {
                let (seq, element) = rest.split_once('e')?;
                (positive(seq)?, element)
            }
            None => (0, body.strip_prefix('e')?),
        };
        Some(Self {
            seq: FrameSeq(seq),
            element: positive(element)?,
        })
    }
}

/// A positive decimal `u32` without a sign or leading zero.
fn positive(digits: &str) -> Option<u32> {
    let mut chars = digits.chars();
    let first = chars.next()?;
    if !('1'..='9').contains(&first) || !chars.all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

impl fmt::Display for AriaRefJump {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.seq {
            FrameSeq(0) => write!(f, "e{}", self.element),
            FrameSeq(seq) => write!(f, "f{seq}e{}", self.element),
        }
    }
}

/// The vendored `ParsedSelector` shape: `{ parts: [{ name, body, source }], capture? }`.
/// Bodies stay opaque; only a leading `aria-ref` part's body is read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParsedSelectorShape {
    parts: Vec<SelectorPartShape>,
    #[serde(rename = "capture", default)]
    _capture: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectorPartShape {
    name: String,
    body: Value,
    #[serde(rename = "source")]
    _source: String,
}

impl ParsedSelectorShape {
    /// The ref jump this chunk asks for, if it starts with a ref.
    fn jump(&self) -> Result<Option<AriaRefJump>, String> {
        let Some(first) = self.parts.first() else {
            return Err("a selector chunk has no parts".to_owned());
        };
        if first.name != "aria-ref" {
            return Ok(None);
        }
        let Value::String(body) = &first.body else {
            return Err("an aria-ref part's body must be a string".to_owned());
        };
        AriaRefJump::parse(body).map(Some).ok_or_else(|| {
            format!(
                "aria-ref={body:?} is not a ref; refs look like e5 or f2e5, as \
                 page.ariaSnapshot({{ mode: 'ai' }}) prints them"
            )
        })
    }
}

impl<'de> Deserialize<'de> for Chunk {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        // Through `Value`, not straight into `RawValue`: a chunk inside the `$neo`-tagged
        // `Target` reaches here through serde's buffered content, which cannot produce a
        // `RawValue`.
        let parsed = Value::deserialize(deserializer)?;
        let shape = ParsedSelectorShape::deserialize(&parsed).map_err(D::Error::custom)?;
        let jump = shape.jump().map_err(D::Error::custom)?;
        let parsed = value::to_raw_value(&parsed).map_err(D::Error::custom)?;
        Ok(Self { parsed, jump })
    }
}

impl Chunk {
    /// `aria-ref=<ref>` for a ref a snapshot printed, as the prelude's `splitSelectorByFrame`
    /// parses it: the chunk a snapshot hops through into the frame behind an iframe it printed,
    /// as upstream's `aria-ref=<ref> >> internal:control=enter-frame` does. Decoded like the
    /// prelude's chunks, so it jumps to the document that printed the ref.
    pub(crate) fn aria_ref(r#ref: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_value(
            json!({ "parts": [{ "name": "aria-ref", "body": r#ref, "source": r#ref }] }),
        )
    }
}

/// Opens every op input: Playwright's name for the call and its timeout.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct CallHeader {
    /// The method the script called (`page.click`, `frame.click`), which every message from
    /// the op starts with. Absent means the op's own name.
    pub(crate) api: Option<String>,
    /// The op's Playwright timeout, which the prelude resolved from the call's option,
    /// `setDefaultTimeout` and Playwright's defaults (Rust holds no defaults). `0` or absent
    /// means none: the op waits until the host's run deadline drops it.
    pub(crate) timeout_ms: Option<u64>,
}

/// Whether a resolution that matches several elements fails. The probe protocol carries it
/// with the same names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Strictness {
    /// Locator methods: a second match throws `strict mode violation` at once.
    Strict,
    /// Page and frame shortcuts (`page.click(selector)`): the first match, unless the script
    /// passed `{ strict: true }`.
    FirstMatch,
}

/// A list with at least one item, so "the last chunk" needs no `Option`.
#[derive(Clone, Debug)]
pub(crate) struct NonEmpty<T> {
    first: T,
    rest: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// The last item.
    pub(crate) fn last(&self) -> &T {
        self.rest.last().unwrap_or(&self.first)
    }

    /// Every item but the last: the frame hops before a locator's final chunk.
    pub(crate) fn hops(&self) -> impl Iterator<Item = &T> {
        let hops = self.rest.len();
        iter::once(&self.first).chain(&self.rest).take(hops)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for NonEmpty<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let mut items = Vec::<T>::deserialize(deserializer)?.into_iter();
        let Some(first) = items.next() else {
            return Err(D::Error::invalid_length(0, &"at least one item"));
        };
        Ok(Self {
            first,
            rest: items.collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::{FrameId, TabId};
    use serde_json::{Value, json};

    use super::{AriaRefJump, CallHeader, Chunk, PageRef, Strictness, Target};
    use crate::world::{BackendNodeId, FrameSeq};

    /// A chunk whose first part is `name=body`, shaped as the vendored `parseSelector` emits it.
    fn chunk(name: &str, body: &str) -> Value {
        json!({ "parts": [{ "name": name, "body": body, "source": body }] })
    }

    fn jump(value: Value) -> Result<Option<AriaRefJump>, serde_json::Error> {
        serde_json::from_value::<Chunk>(value).map(|chunk| chunk.jump)
    }

    #[test]
    fn tests_that_a_frame_ref_jumps_to_its_document() -> Result<(), Box<dyn Error>> {
        let jumped = jump(chunk("aria-ref", "f7e3"))?;
        assert_eq!(
            jumped,
            Some(AriaRefJump {
                seq: FrameSeq(7),
                element: 3
            })
        );
        assert_eq!(jumped.map(|jump| jump.to_string()).as_deref(), Some("f7e3"));
        Ok(())
    }

    #[test]
    fn tests_that_a_bare_ref_jumps_to_the_first_main_document() -> Result<(), Box<dyn Error>> {
        let jumped = jump(chunk("aria-ref", "e5"))?;
        assert_eq!(
            jumped,
            Some(AriaRefJump {
                seq: FrameSeq(0),
                element: 5
            }),
            "e5 was printed by seq 0, and must fail once that document is gone"
        );
        assert_eq!(jumped.map(|jump| jump.to_string()).as_deref(), Some("e5"));
        Ok(())
    }

    /// The vendored parser keeps what follows `=` as the body, spaces included, so
    /// `aria-ref= e5` arrives with the body ` e5`. No snapshot prints that ref, and Neo's
    /// engine would never match it, so it fails at once instead of waiting out a timeout.
    #[test]
    fn tests_that_a_ref_with_spaces_is_invalid_input() {
        for body in [" e5", " f7e3", "e5 "] {
            let error = jump(chunk("aria-ref", body))
                .err()
                .map(|error| error.to_string());
            assert!(
                error
                    .as_deref()
                    .is_some_and(|error| error.contains("is not a ref")),
                "aria-ref={body:?} must be refused with a hint, got {error:?}"
            );
        }
    }

    #[test]
    fn tests_that_only_refs_neo_prints_are_refs() {
        for body in [
            "f0e5", "e0", "e05", "f07e3", "E5", "f7", "fe3", "e", "", "e5e6", "e+5",
        ] {
            assert!(
                jump(chunk("aria-ref", body)).is_err(),
                "aria-ref={body:?} names no printed element"
            );
        }
        assert!(
            jump(chunk("aria-ref", "e4294967296")).is_err(),
            "beyond u32 is no ref"
        );
    }

    #[test]
    fn tests_that_a_chunk_without_a_leading_ref_does_not_jump() -> Result<(), Box<dyn Error>> {
        assert_eq!(jump(chunk("internal:role", "button[name=\"e5\"i]"))?, None);
        let css =
            json!({ "parts": [{ "name": "css", "body": [{ "simples": [] }], "source": "#pay" }] });
        assert_eq!(jump(css)?, None, "a CSS body stays opaque");
        let later_ref = json!({ "parts": [
            { "name": "internal:attr", "body": "[id=\"row\"s]", "source": "[id=\"row\"s]" },
            { "name": "aria-ref", "body": "f7e3", "source": "f7e3" },
        ] });
        assert_eq!(
            jump(later_ref)?,
            None,
            "only a chunk's first part jumps, as upstream"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_chunk_keeps_the_parsed_selector_for_the_page() -> Result<(), Box<dyn Error>> {
        let parsed = json!({ "parts": [{ "name": "aria-ref", "body": "e5", "source": "e5" }], "capture": 0 });
        let chunk: Chunk = serde_json::from_value(parsed.clone())?;
        let sent: Value = serde_json::from_str(chunk.parsed.get())?;
        assert_eq!(
            sent, parsed,
            "the page receives the prelude's ParsedSelector untouched"
        );
        Ok(())
    }

    /// `tests/frames_live.rs` pins the same shape to the vendored parser's output for
    /// `aria-ref=f7e3`.
    #[test]
    fn tests_that_a_snapshot_hop_chunk_is_the_prelude_s_and_jumps_to_its_printer()
    -> Result<(), Box<dyn Error>> {
        let hop = Chunk::aria_ref("f7e3")?;
        let sent: Value = serde_json::from_str(hop.parsed.get())?;
        assert_eq!(sent, chunk("aria-ref", "f7e3"));
        assert_eq!(
            hop.jump,
            Some(AriaRefJump {
                seq: FrameSeq(7),
                element: 3
            })
        );
        assert!(
            Chunk::aria_ref("#4").is_err(),
            "only printed refs, never a frame key"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_malformed_chunk_is_refused() {
        for bad in [
            json!({ "parts": [] }),
            json!({ "parts": [{ "name": "css", "body": "#a" }] }),
            json!({ "parts": [{ "name": "css", "body": "#a", "source": "#a", "extra": 1 }] }),
            json!({ "parts": [{ "name": "aria-ref", "body": { "parsed": {} }, "source": "x" }] }),
            json!("#pay"),
        ] {
            assert!(
                serde_json::from_value::<Chunk>(bad.clone()).is_err(),
                "{bad} is not a chunk"
            );
        }
    }

    #[test]
    fn tests_that_a_locator_target_decodes_its_frames() -> Result<(), Box<dyn Error>> {
        let target: Target = serde_json::from_value(json!({
            "$neo": "locator",
            "page": { "$neo": "page", "tab": 1007 },
            "frame": "F1",
            "chunks": [
                chunk("internal:attr", "[id=\"pay\"s]"),
                chunk("internal:role", "button[name=\"Apply\"i]"),
            ],
            "display": "locator('#pay').contentFrame().getByRole('button', { name: 'Apply' })",
        }))?;
        let Target::Locator(locator) = target else {
            panic!("a locator brand is a locator target");
        };
        assert_eq!(locator.page.tab, TabId(1007));
        assert_eq!(locator.frame, Some(FrameId("F1".to_owned())));
        let (hops, last) = locator.hops_and_last_bodies();
        assert_eq!(hops, ["[id=\"pay\"s]"], "the iframe chunk is a hop");
        assert_eq!(
            last, "button[name=\"Apply\"i]",
            "the button chunk runs in the frame"
        );
        assert!(locator.display.starts_with("locator('#pay')"));
        Ok(())
    }

    #[test]
    fn tests_that_an_element_target_names_its_document_and_node() -> Result<(), Box<dyn Error>> {
        let page = json!({ "$neo": "page", "tab": 1007 });
        let target: Target = serde_json::from_value(json!({
            "$neo": "element", "page": page, "seq": 4, "node": 1234, "display": "<button>Save</button>",
        }))?;
        let Target::Element(element) = target else {
            panic!("an element brand is an element target");
        };
        assert_eq!(
            serde_json::to_value(&element.page)?,
            page,
            "an element handle hands its page back as it came"
        );
        assert_eq!(
            (
                element.page.tab,
                element.seq,
                element.node,
                element.display.as_str()
            ),
            (
                TabId(1007),
                FrameSeq(4),
                BackendNodeId(1234),
                "<button>Save</button>"
            )
        );
        Ok(())
    }

    #[test]
    fn tests_that_targets_refuse_what_they_do_not_know() {
        let page = json!({ "$neo": "page", "tab": 1007 });
        for bad in [
            json!({ "$neo": "locator", "page": page, "chunks": [], "display": "x" }),
            json!({ "$neo": "locator", "page": page, "chunks": [chunk("css", "a")], "display": "x", "steps": [] }),
            json!({ "$neo": "page", "id": 7 }),
            json!({ "page": page, "chunks": [chunk("css", "a")], "display": "x" }),
            json!({ "$neo": "locator", "page": 7, "chunks": [chunk("css", "a")], "display": "x" }),
            json!({ "$neo": "element", "page": 7, "seq": 4, "node": 1234, "display": "x" }),
        ] {
            assert!(
                serde_json::from_value::<Target>(bad.clone()).is_err(),
                "{bad} is no target"
            );
        }
    }

    /// The door resolves the run's default page once, checks its policies on that tab, and
    /// hands the crate that tab. A target still naming the unresolved page would let the crate
    /// pick a tab of its own, so it never decodes, at the top or nested in a target.
    #[test]
    fn tests_that_the_unresolved_default_page_never_decodes() {
        let lazy = json!({ "$neo": "page", "tab": null });
        assert!(serde_json::from_value::<PageRef>(lazy.clone()).is_err());
        for target in [
            json!({ "$neo": "locator", "page": lazy, "chunks": [chunk("css", "a")], "display": "x" }),
            json!({ "$neo": "element", "page": lazy, "seq": 4, "node": 1234, "display": "x" }),
        ] {
            assert!(
                serde_json::from_value::<Target>(target.clone()).is_err(),
                "{target} names no tab the door cleared"
            );
        }
    }

    #[test]
    fn tests_that_a_page_handle_carries_only_its_tab() -> Result<(), Box<dyn Error>> {
        let page: PageRef = serde_json::from_value(json!({ "$neo": "page", "tab": 1234 }))?;
        assert_eq!(page.tab, TabId(1234));
        for bad in [
            json!({ "$neo": "page", "id": 7, "tab": 1234 }),
            json!({ "$neo": "page", "tab": null }),
            json!({ "$neo": "locator", "id": 7, "tab": 1234 }),
            json!({ "$neo": "page", "id": 7 }),
            json!({ "$neo": "page", "tab": 1234, "url": "" }),
        ] {
            assert!(
                serde_json::from_value::<PageRef>(bad.clone()).is_err(),
                "{bad} is no page handle"
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_the_call_header_and_strictness_decode_strictly() -> Result<(), Box<dyn Error>> {
        let header: CallHeader =
            serde_json::from_value(json!({ "api": "page.click", "timeoutMs": 5000 }))?;
        assert_eq!(header.api.as_deref(), Some("page.click"));
        assert_eq!(header.timeout_ms, Some(5000));
        assert!(serde_json::from_value::<CallHeader>(json!({ "timeout": 5000 })).is_err());
        let strictness: Vec<Strictness> = serde_json::from_value(json!(["strict", "firstMatch"]))?;
        assert_eq!(strictness, [Strictness::Strict, Strictness::FirstMatch]);
        Ok(())
    }

    impl super::LocatorTarget {
        /// The hop chunks' and the last chunk's first bodies, for asserting chunk order.
        fn hops_and_last_bodies(&self) -> (Vec<String>, String) {
            let body = |chunk: &Chunk| -> String {
                let parsed: Value = serde_json::from_str(chunk.parsed.get()).unwrap_or_default();
                parsed["parts"][0]["body"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            };
            (
                self.chunks.hops().map(body).collect(),
                body(self.chunks.last()),
            )
        }
    }
}
