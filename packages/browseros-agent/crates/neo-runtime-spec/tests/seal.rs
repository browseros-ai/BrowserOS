//! The call traits are sealed. The compile-fail half lives in `src/call.rs` as doc tests: an
//! outside impl of `Command` or `InjectedCall` does not compile (E0277, the missing seal). This
//! half checks that the generated calls stay usable through the traits, which is the only way
//! `neo-runtime` reaches them, and that each result decodes the way its spec says it crosses.

use neo_runtime_spec::injected::{
    ElementState, ElementStatesOutcome, HitTargetBlocked, HitTargetOutcome, HitTargetSetup,
    InjectedAriaSnapshotParams, InjectedCheckElementStatesParams, InjectedCheckElementStatesResult,
    InjectedFrameOwnerParams, InjectedFrameOwnerResult, InjectedResolveResult,
    InjectedSetupHitTargetInterceptorParams, InjectedSetupHitTargetInterceptorResult,
    InjectedStopHitTargetInterceptorParams, InjectedStopHitTargetInterceptorResult,
    InjectedTextContentParams, InjectedTitleParams, InjectedTitleResult, MissingState,
    NotConnected, Point,
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
        json!({"visible": true}),
        &[("element", "-9083572.1.4", Some("node"))],
    ))
    .unwrap();
    let element = found.element.expect("the element");
    assert_eq!(element.id().object_id(), "-9083572.1.4");

    // The page returns `{element: undefined}` when nothing matched, and `neo-runtime` leaves
    // `undefined` out.
    assert_eq!(
        frame_owner(properties(json!({"visible": false}), &[])).unwrap(),
        InjectedFrameOwnerResult {
            element: None,
            visible: false
        }
    );
    assert_eq!(
        frame_owner(properties(json!({"element": null, "visible": false}), &[])).unwrap(),
        InjectedFrameOwnerResult {
            element: None,
            visible: false
        },
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
        json!({"count": 1, "visible": true}),
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

/// An element handle, as `resolve` hands one out.
fn element(id: &str) -> Option<Handle<Element>> {
    Handle::new(HandleId::new(id.to_owned()), Some("node"))
}

#[test]
fn a_click_resolves_one_element_and_passes_it_by_handle() {
    let resolved = InjectedResolveResult::from_properties(properties(
        json!({"count": 2, "preview": "<button>Submit</button>"}),
        &[("element", "-7.1.9", Some("node"))],
    ))
    .unwrap();
    assert_eq!(
        resolved,
        InjectedResolveResult {
            element: element("-7.1.9"),
            count: 2,
            preview: Some("<button>Submit</button>".into()),
        }
    );
    let missing_count = InjectedResolveResult::from_properties(Properties::default());
    assert_eq!(
        missing_count.unwrap_err().to_string(),
        "page call resolve returned no `count`"
    );

    // The handle crosses as an argument after the params, never inside them.
    let states = InjectedCheckElementStatesParams {
        element: resolved.element.expect("resolve found the element"),
        states: vec![
            ElementState::Visible,
            ElementState::Stable,
            ElementState::NonInert,
        ],
        frame_visible: true,
    };
    assert_eq!(
        page_call(&states).unwrap(),
        (
            "checkElementStates",
            json!({"states": ["visible", "stable", "non-inert"], "frameVisible": true}),
            vec!["-7.1.9".to_owned()],
            true
        )
    );
    let not_ready = InjectedCheckElementStatesResult::from_value(
        json!({"outcome": {"missingState": "stable"}}),
    )
    .unwrap();
    let missing = MissingState {
        missing_state: ElementState::Stable,
    };
    assert_eq!(
        not_ready.outcome,
        Some(ElementStatesOutcome::Missing(Box::new(missing)))
    );
    let detached =
        InjectedCheckElementStatesResult::from_value(json!({"outcome": "error:notconnected"}))
            .unwrap();
    assert_eq!(
        detached.outcome,
        Some(ElementStatesOutcome::NotConnected(
            NotConnected::ErrorNotconnected
        ))
    );
}

#[test]
fn an_armed_interceptor_is_the_handle_its_stop_consumes() {
    let setup = InjectedSetupHitTargetInterceptorParams {
        element: element("-7.1.9").unwrap(),
        hit_point: Some(Point { x: 40.5, y: 18.0 }),
        block_all_events: false,
    };
    assert_eq!(
        page_call(&setup).unwrap(),
        (
            "setupHitTargetInterceptor",
            json!({"hitPoint": {"x": 40.5, "y": 18.0}, "blockAllEvents": false}),
            vec!["-7.1.9".to_owned()],
            false
        )
    );

    // Armed: `{stop}` is a plain object, which CDP reports with no subtype.
    let armed = InjectedSetupHitTargetInterceptorResult::from_properties(properties(
        json!({}),
        &[("outcome", "-7.1.12", None)],
    ))
    .unwrap();
    let HitTargetSetup::Armed(interceptor) = armed.outcome else {
        panic!("an object outcome is the armed interceptor: {armed:?}");
    };
    let stop = InjectedStopHitTargetInterceptorParams { interceptor };
    assert_eq!(
        page_call(&stop).unwrap(),
        (
            "stopHitTargetInterceptor",
            json!({}),
            vec!["-7.1.12".to_owned()],
            true
        )
    );
    let blocked = InjectedStopHitTargetInterceptorResult::from_value(
        json!({"outcome": {"hitTargetDescription": "<div id=\"overlay\"></div>"}}),
    )
    .unwrap();
    let overlay = HitTargetBlocked {
        hit_target_description: "<div id=\"overlay\"></div>".into(),
    };
    assert_eq!(
        blocked.outcome,
        HitTargetOutcome::Blocked(Box::new(overlay))
    );

    // Not armed: the description of what covers the point arrives as a value.
    let covered = InjectedSetupHitTargetInterceptorResult::from_properties(properties(
        json!({"outcome": "<div id=\"overlay\"></div>"}),
        &[],
    ))
    .unwrap();
    assert_eq!(
        covered.outcome,
        HitTargetSetup::Description("<div id=\"overlay\"></div>".into())
    );
    // An element is not an interceptor, even though both are page objects.
    let element_outcome = InjectedSetupHitTargetInterceptorResult::from_properties(properties(
        json!({}),
        &[("outcome", "-7.1.9", Some("node"))],
    ));
    assert_eq!(
        element_outcome.unwrap_err().to_string(),
        "page call setupHitTargetInterceptor returned `outcome` as a page object of subtype \
         Some(\"node\"), not a `HitTargetInterceptor` (subtype None)"
    );
    let number = InjectedSetupHitTargetInterceptorResult::from_properties(properties(
        json!({"outcome": 3}),
        &[],
    ));
    assert_eq!(
        number.unwrap_err().to_string(),
        "page call setupHitTargetInterceptor returned `outcome` as the value 3, not a string or a \
         `HitTargetInterceptor` page object"
    );
}
