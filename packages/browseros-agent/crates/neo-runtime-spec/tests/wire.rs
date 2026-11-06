//! What crosses each seam on the wire, through the generated types.

use std::env;
use std::fs;
use std::path::Path;

use neo_runtime_spec::injected::{
    AriaChild, AriaInvalid, AriaInvalidKind, AriaRole, AriaTristate, AriaTristateMixed,
    InjectedAriaSnapshotResult, InjectedTextContentResult, TextMatch,
};
use neo_runtime_spec::serve::{
    FrameCall, FrameGotoParams, FrameGotoResult, FrameTextContentParams, FrameTextContentResult,
    FrameTitleResult, LifecycleEvent, Metadata,
};
use neo_runtime_spec::{CallDecodeError, Command, MethodFlag, MethodGroup, SPEC_SHA256};
use serde_json::json;

#[test]
fn optional_fields_are_absent_never_null() {
    let result = FrameTextContentResult { value: None };
    assert_eq!(serde_json::to_value(&result).unwrap(), json!({}));
    let params = FrameGotoParams {
        url: "https://a.test".into(),
        wait_until: None,
        referer: None,
    };
    assert_eq!(
        serde_json::to_value(&params).unwrap(),
        json!({"url": "https://a.test"})
    );
    let no_match = InjectedTextContentResult { matched: None };
    assert_eq!(serde_json::to_value(&no_match).unwrap(), json!({}));
}

#[test]
fn served_calls_decode_by_method_name() {
    let call = FrameCall::decode(
        "textContent",
        Some(json!({"selector": "#a", "strict": true})),
    );
    let expected = FrameTextContentParams {
        selector: "#a".into(),
        strict: Some(true),
    };
    assert_eq!(call.unwrap(), FrameCall::TextContent(expected));

    // Playwright's client sends no params for a command that declares none.
    let title = FrameCall::decode("title", None).unwrap();
    assert!(matches!(title, FrameCall::Title(_)), "{title:?}");
    assert_eq!(title.meta().group, Some(MethodGroup::Getter));

    let unknown = FrameCall::decode("click", Some(json!({"selector": "#a"}))).unwrap_err();
    assert!(
        matches!(&unknown, CallDecodeError::UnknownMethod { interface: "Frame", method } if method == "click"),
        "{unknown:?}"
    );
    let invalid = FrameCall::decode("goto", Some(json!({"url": "x", "waitUntil": "later"})));
    let message = invalid.unwrap_err().to_string();
    assert!(
        message.starts_with("invalid params for Frame.goto: unknown variant `later`"),
        "{message}"
    );
    let missing = FrameCall::decode("goto", None).unwrap_err().to_string();
    assert!(missing.contains("missing field `url`"), "{missing}");
}

#[test]
fn replies_are_typed_by_their_command() {
    let params = FrameGotoParams {
        url: "https://a.test".into(),
        wait_until: Some(LifecycleEvent::Load),
        referer: None,
    };
    // `params.reply(&FrameTitleResult { .. })` does not compile: the reply type is the command's.
    assert_eq!(
        params.reply(&FrameGotoResult { response: None }).unwrap(),
        json!({})
    );
    let meta = <FrameGotoParams as Command>::META;
    assert_eq!(
        (meta.interface, meta.method, meta.title),
        ("Frame", "goto", Some("Navigate"))
    );
    assert!(
        meta.has(MethodFlag::Snapshot) && !meta.has(MethodFlag::Input),
        "{meta:?}"
    );
    let title = FrameTitleResult {
        value: "Example".into(),
    };
    assert_eq!(
        serde_json::to_value(&title).unwrap(),
        json!({"value": "Example"})
    );
}

#[test]
fn goto_answers_without_a_response_until_neo_implements_one() {
    // `ResponseChannel` is uninhabited: `None` is the only value, and no wire value decodes.
    let decoded: FrameGotoResult = serde_json::from_value(json!({})).unwrap();
    assert_eq!(decoded, FrameGotoResult { response: None });
    let with_response =
        serde_json::from_value::<FrameGotoResult>(json!({"response": {"guid": "r1"}}));
    assert!(
        with_response.is_err(),
        "a Response reference decoded: {with_response:?}"
    );
}

#[test]
fn served_types_ignore_unknown_fields_like_playwright() {
    // Playwright's validator drops unknown keys, so a newer client's extra field is not an error.
    let metadata: Metadata =
        serde_json::from_value(json!({"timeout": 30000, "apiName": "page.goto"})).unwrap();
    assert_eq!(metadata.timeout, Some(30000.0));
}

#[test]
fn page_results_reject_unknown_fields() {
    let matched: InjectedTextContentResult =
        serde_json::from_value(json!({"matched": {"value": "", "preview": "<p></p>"}})).unwrap();
    let expected = TextMatch {
        value: String::new(),
        preview: "<p></p>".into(),
    };
    assert_eq!(matched.matched, Some(expected));
    let drift = serde_json::from_value::<InjectedTextContentResult>(json!({"matched": {
        "value": "x", "preview": "<p></p>", "extra": 1
    }}));
    let message = drift.unwrap_err().to_string();
    assert!(message.contains("unknown field `extra`"), "{message}");
}

#[test]
fn unions_decode_by_json_kind_and_encode_untagged() {
    let wire = json!({"snapshot": {
        "nodes": [{"role": "list", "children": [
            "loose text",
            {"role": "listitem", "checked": "mixed", "invalid": true, "ref": "e2",
             "box": {"x": 0, "y": 8, "width": 10, "height": 4}}
        ]}],
        "iframes": [{"ref": "e5", "depth": 2}]
    }});
    let result: InjectedAriaSnapshotResult = serde_json::from_value(wire.clone()).unwrap();
    let snapshot = result.snapshot.as_ref().unwrap();
    let children = snapshot.nodes[0].children.as_ref().unwrap();
    assert_eq!(children[0], AriaChild::Text("loose text".into()));
    let AriaChild::Node(item) = &children[1] else {
        panic!("expected a node: {children:?}");
    };
    assert_eq!(item.role, AriaRole::Listitem);
    assert_eq!(
        item.checked,
        Some(AriaTristate::Mixed(AriaTristateMixed::Mixed))
    );
    assert_eq!(item.invalid, Some(AriaInvalid::Flag(true)));
    assert_eq!(item.r#ref.as_deref(), Some("e2"));
    assert_eq!(snapshot.iframes[0].r#ref, "e5");

    // Encoding writes each member's own JSON, so a round trip is the identity.
    assert_eq!(serde_json::to_value(&result).unwrap(), wire);
    let spelling = serde_json::to_value(AriaInvalid::Kind(AriaInvalidKind::Spelling)).unwrap();
    assert_eq!(spelling, json!("spelling"));

    let wrong_literal = serde_json::from_value::<AriaTristate>(json!("partly")).unwrap_err();
    assert!(
        wrong_literal
            .to_string()
            .contains("unknown variant `partly`"),
        "{wrong_literal}"
    );
    let wrong_kind = serde_json::from_value::<AriaChild>(json!(3)).unwrap_err();
    assert!(
        wrong_kind
            .to_string()
            .contains("an AriaNode object or a string"),
        "{wrong_kind}"
    );
    let unknown_role = serde_json::from_value::<AriaChild>(json!({"role": "spaceship"}));
    assert!(unknown_role.is_err(), "{unknown_role:?}");
}

#[test]
fn int_fields_take_integral_numbers_only() {
    // `int` is `i64`, which relies on CDP writing integral doubles as JSON integers (see the
    // crate docs on `-0`).
    let snapshot: InjectedAriaSnapshotResult = serde_json::from_value(json!({"snapshot": {
        "nodes": [{"role": "img", "box": {"x": -4, "y": 1, "width": 2, "height": 3}}], "iframes": []
    }}))
    .unwrap();
    let node = &snapshot.snapshot.unwrap().nodes[0];
    assert_eq!(node.r#box.as_ref().map(|rect| rect.x), Some(-4));
    let fractional = serde_json::from_value::<InjectedAriaSnapshotResult>(json!({"snapshot": {
        "nodes": [{"role": "img", "box": {"x": 0.5, "y": 1, "width": 2, "height": 3}}], "iframes": []
    }}));
    assert!(
        fractional.is_err(),
        "a fractional box decoded: {fractional:?}"
    );
}

#[test]
fn spec_hash_is_stamped_in_both_languages() {
    assert!(
        SPEC_SHA256.len() == 64
            && SPEC_SHA256
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
        "{SPEC_SHA256}"
    );
    // Read at run time, never with `env!`: worktrees share one Cargo build dir, so this binary may
    // have been compiled in another worktree, and a compile-time path would read that one.
    let crate_dir = env::var_os("CARGO_MANIFEST_DIR").expect("cargo test sets CARGO_MANIFEST_DIR");
    let ts = fs::read_to_string(Path::new(&crate_dir).join("ts/spec-hash.ts")).unwrap();
    assert!(
        ts.contains(&format!("export const SPEC_SHA256 = '{SPEC_SHA256}'")),
        "{ts}"
    );
}
