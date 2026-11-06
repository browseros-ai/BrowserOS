//! The call traits are sealed. The compile-fail half lives in `src/call.rs` as doc tests: an
//! outside impl of `Command` or `InjectedCall` does not compile (E0277, the missing seal). This
//! half checks that the generated calls stay usable through the traits, which is the only way
//! `neo-runtime` reaches them, and that each result decodes the way its spec says it crosses.

use neo_runtime_spec::injected::{
    InjectedAriaSnapshotParams, InjectedFrameOwnerParams, InjectedFrameOwnerResult,
    InjectedTextContentParams, InjectedTitleParams, InjectedTitleResult,
};
use neo_runtime_spec::serve::FrameTitleParams;
use neo_runtime_spec::{
    Command, Element, Handle, HandleId, InjectedCall, PageResult, Properties, ResultDecodeError,
};
use serde_json::{Value, json};

/// What `neo-runtime` sends for any page call: the glue key, the JSON params, the handle
/// arguments' object ids, and whether the result crosses by value.
fn page_call<C: InjectedCall>(
    params: &C,
) -> serde_json::Result<(&'static str, Value, Vec<String>, bool)> {
    let handles = params
        .handles()
        .iter()
        .map(|id| id.object_id().to_owned())
        .collect();
    let json = serde_json::to_value(params)?;
    Ok((C::NAME, json, handles, C::Result::BY_VALUE))
}

fn served<C: Command>() -> (&'static str, &'static str) {
    (C::META.interface, C::META.method)
}

fn glue_key<C: InjectedCall>() -> &'static str {
    C::NAME
}

/// The properties of a page result, as `neo-runtime` reads them with `Runtime.getProperties`.
fn properties(values: Value, handles: &[(&str, &str, Option<&str>)]) -> Properties {
    let Value::Object(values) = values else {
        panic!("values must be a JSON object");
    };
    let handles = handles
        .iter()
        .map(|(key, id, subtype)| {
            let handle = (HandleId::new((*id).to_owned()), subtype.map(str::to_owned));
            ((*key).to_owned(), handle)
        })
        .collect();
    Properties { values, handles }
}

fn frame_owner(properties: Properties) -> Result<InjectedFrameOwnerResult, String> {
    InjectedFrameOwnerResult::from_properties(properties).map_err(|error| error.to_string())
}

#[test]
fn generated_calls_are_usable_through_the_sealed_traits() {
    assert_eq!(
        page_call(&InjectedTitleParams {}).unwrap(),
        ("title", json!({}), Vec::new(), true)
    );
    let text_content = InjectedTextContentParams {
        selector: "#a".into(),
        strict: true,
    };
    assert_eq!(
        page_call(&text_content).unwrap(),
        (
            "textContent",
            json!({"selector": "#a", "strict": true}),
            Vec::new(),
            true
        )
    );
    // `frameOwner` returns an element, so its result crosses through `Runtime.getProperties`.
    let frame_owner = InjectedFrameOwnerParams {
        selector: "iframe".into(),
        strict: true,
    };
    assert_eq!(
        page_call(&frame_owner).unwrap(),
        (
            "frameOwner",
            json!({"selector": "iframe", "strict": true}),
            Vec::new(),
            false
        )
    );
    assert_eq!(glue_key::<InjectedAriaSnapshotParams>(), "ariaSnapshot");
    assert_eq!(served::<FrameTitleParams>(), ("Frame", "title"));
}

#[test]
fn a_handle_result_decodes_from_its_properties() {
    let found = frame_owner(properties(
        json!({}),
        &[("element", "-9083572.1.4", Some("node"))],
    ))
    .unwrap();
    let element = found.element.expect("the element");
    assert_eq!(element.id().object_id(), "-9083572.1.4");

    // The page returns `{element: undefined}` when nothing matched, and `neo-runtime` leaves
    // `undefined` out.
    assert_eq!(
        frame_owner(Properties::default()).unwrap(),
        InjectedFrameOwnerResult { element: None }
    );
    assert_eq!(
        frame_owner(properties(json!({"element": null}), &[])).unwrap(),
        InjectedFrameOwnerResult { element: None },
        "null reads as absent, as it does for an Option field decoded from JSON"
    );
}

#[test]
fn a_handle_result_rejects_what_its_spec_does_not_declare() {
    let array = frame_owner(properties(
        json!({}),
        &[("element", "-1.1.1", Some("array"))],
    ));
    assert_eq!(
        array.unwrap_err(),
        "page call frameOwner returned `element` as a page object of subtype Some(\"array\"), \
         not a `Element` (subtype Some(\"node\"))"
    );
    let value = frame_owner(properties(json!({"element": "iframe#main"}), &[]));
    assert_eq!(
        value.unwrap_err(),
        "page call frameOwner returned `element` as the value \"iframe#main\", not a page object"
    );
    let extra = frame_owner(properties(
        json!({"count": 1}),
        &[("element", "-1.1.1", Some("node"))],
    ));
    assert_eq!(
        extra.unwrap_err(),
        "page call frameOwner returned the unknown property `count`"
    );
}

#[test]
fn a_result_decodes_only_the_way_its_spec_crosses() {
    let by_value = InjectedFrameOwnerResult::from_value(json!({}));
    assert_eq!(
        by_value.map_err(|error| error.to_string()).unwrap_err(),
        "page call frameOwner returned a value, but its result holds a page object"
    );
    let as_properties = InjectedTitleResult::from_properties(Properties::default());
    let error: ResultDecodeError = as_properties.unwrap_err();
    assert_eq!(
        (error.call, error.to_string().as_str()),
        (
            "title",
            "page call title returned properties, but its result crosses by value"
        )
    );
    assert_eq!(
        InjectedTitleResult::from_value(json!({"value": "Example"})).unwrap(),
        InjectedTitleResult {
            value: "Example".into()
        }
    );
}

#[test]
fn a_handle_checks_its_kind_and_compares_by_object_id() {
    let id = HandleId::new("-9083572.1.4".into());
    assert!(
        Handle::<Element>::new(id.clone(), None).is_none(),
        "a plain object is not an element"
    );
    let element = Handle::<Element>::new(id, Some("node")).expect("a node is an element");
    let copy = element.clone();
    assert_eq!(copy, element);
    assert_eq!(format!("{copy:?}"), "Handle<Element>(\"-9083572.1.4\")");
}
