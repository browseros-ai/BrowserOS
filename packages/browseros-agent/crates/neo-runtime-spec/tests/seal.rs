//! The call traits are sealed. The compile-fail half lives in `src/call.rs` as doc tests: an
//! outside impl of `Command` or `InjectedCall` does not compile (E0277, the missing seal). This
//! half checks that the generated calls stay usable through the traits, which is the only way
//! `neo-runtime` reaches them, and that each page call has the call kind its spec declares.

use neo_runtime_spec::injected::{
    InjectedAriaSnapshotParams, InjectedFrameOwnerParams, InjectedTextContentParams,
    InjectedTitleParams,
};
use neo_runtime_spec::serve::FrameTitleParams;
use neo_runtime_spec::{Command, ElementCall, InjectedCall, RemoteElement, ValueCall};

/// How `neo-runtime` calls a page function by value: one generic path for every `ValueCall`.
fn by_value<C: ValueCall>(_: &C) -> &'static str {
    C::NAME
}

/// How `neo-runtime` calls a page function that returns an element.
fn by_element<C: ElementCall>(_: &C) -> &'static str {
    C::NAME
}

fn served<C: Command>() -> (&'static str, &'static str) {
    (C::META.interface, C::META.method)
}

fn glue_key<C: InjectedCall>() -> &'static str {
    C::NAME
}

#[test]
fn generated_calls_are_usable_through_the_sealed_traits() {
    assert_eq!(by_value(&InjectedTitleParams {}), "title");
    let text_content = InjectedTextContentParams {
        selector: "#a".into(),
        strict: true,
    };
    assert_eq!(by_value(&text_content), "textContent");
    // `by_value(&frame_owner)` does not compile: `frameOwner` returns an element.
    let frame_owner = InjectedFrameOwnerParams {
        selector: "iframe".into(),
        strict: true,
    };
    assert_eq!(by_element(&frame_owner), "frameOwner");
    assert_eq!(glue_key::<InjectedAriaSnapshotParams>(), "ariaSnapshot");
    assert_eq!(served::<FrameTitleParams>(), ("Frame", "title"));
}

#[test]
fn an_element_result_is_a_remote_object_id() {
    let element = RemoteElement::new("-9083572.1.4".into());
    assert_eq!(element.object_id(), "-9083572.1.4");
}
