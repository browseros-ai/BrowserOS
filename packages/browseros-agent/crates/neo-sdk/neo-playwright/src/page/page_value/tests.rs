use std::error::Error;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{MAX_DEPTH, PageValue, Unreadable, depth_of, js_number};

type TestResult = Result<(), Box<dyn Error>>;

/// tests/data/in-page-values.json, which scripts/in-page-values.ts writes from the vendored
/// serializer.
#[derive(Deserialize)]
struct Golden {
    values: Vec<GoldenValue>,
}

#[derive(Deserialize)]
struct GoldenValue {
    name: String,
    /// What the page's serializer wrote.
    page: String,
    /// What the parse must write back for the prelude to read.
    result: Value,
}

fn golden() -> Result<Golden, serde_json::Error> {
    serde_json::from_str(include_str!("../../../tests/data/in-page-values.json"))
}

/// `text` parsed and written back, or why it cannot leave.
fn read(text: &str) -> Result<Value, String> {
    match PageValue::parse(text) {
        Ok(value) => Ok(value.into_json()),
        Err(Unreadable::TooDeep) => Err("too deep".to_owned()),
        Err(Unreadable::Shape) => Err("not the format".to_owned()),
    }
}

#[test]
fn tests_that_every_serialized_value_reads_and_leaves_as_the_golden_says() -> TestResult {
    for case in &golden()?.values {
        assert_eq!(
            read(&case.page)?,
            case.result,
            "{}: {}",
            case.name,
            case.page
        );
    }
    Ok(())
}

#[test]
fn tests_that_no_text_is_undefined() {
    assert_eq!(
        PageValue::undefined().into_json(),
        json!({ "v": "undefined" })
    );
}

/// `depth` nested `{o: [{k, v}]}` objects around 0: three JSON levels each, numbered from the
/// outside in, as the serializer numbers them.
fn nested_objects(depth: usize) -> String {
    let open = r#"{"o":[{"k":"a","v":"#.repeat(depth);
    let close = (1..=depth)
        .rev()
        .map(|id| format!(r#"}}],"id":{id}}}"#))
        .collect::<Vec<_>>()
        .concat();
    format!("{open}0{close}")
}

/// `depth` nested `{a: [...]}` arrays around 0: two JSON levels each.
fn nested_arrays(depth: usize) -> String {
    let open = r#"{"a":["#.repeat(depth);
    let close = (1..=depth)
        .rev()
        .map(|id| format!(r#"],"id":{id}}}"#))
        .collect::<Vec<_>>()
        .concat();
    format!("{open}0{close}")
}

/// S-depth's measured boundary: upstream returns 99 nested objects (297 levels) and fails 149
/// nested arrays (298). A 10,000-level value is refused before anything recurses over it.
#[test]
fn tests_that_a_value_deeper_than_upstreams_limit_is_too_deep() {
    assert_eq!(depth_of(&nested_objects(99)), MAX_DEPTH);
    assert!(read(&nested_objects(99)).is_ok(), "297 levels come back");
    assert!(read(&nested_arrays(148)).is_ok(), "296 levels come back");
    for deep in [
        nested_arrays(149),
        nested_objects(100),
        nested_arrays(10_000),
    ] {
        assert_eq!(read(&deep), Err("too deep".to_owned()));
    }
}

#[test]
fn tests_that_depth_counts_brackets_outside_strings_only() {
    assert_eq!(depth_of(r#""[[{{""#), 0);
    assert_eq!(depth_of(r#"["a\"[[", {"b": ["]"]}]"#), 3);
    assert_eq!(depth_of("42"), 0);
}

/// Shapes the serializer never writes for a result, as a page that replaced its globals could:
/// each fails rather than reaching the agent.
#[test]
fn tests_that_anything_outside_the_format_fails() {
    for text in [
        "null",
        "[1]",
        r#"{"m":[],"id":1}"#,
        r#"{"s":[],"id":1}"#,
        r#"{"h":0}"#,
        r#"{"v":"bogus"}"#,
        r#"{"v":"NaN","x":1}"#,
        r#"{"a":[1]}"#,
        r#"{"a":{},"id":1}"#,
        r#"{"o":[{"k":1,"v":2}],"id":1}"#,
        r#"{"o":[{"k":"a","v":null}],"id":1}"#,
        r#"{"o":[{"k":"a","v":1,"x":2}],"id":1}"#,
        r#"{"o":[],"id":-1}"#,
        r#"{"bi":"12x"}"#,
        r#"{"bi":""}"#,
        r#"{"d":5}"#,
        r#"{"e":{"n":1,"m":"","s":""}}"#,
        r#"{"e":{"n":"Error","m":""}}"#,
        r#"{"r":{"p":"a"}}"#,
        r#"{"ta":{"b":"AQ==","k":"u8"}}"#,
        r#"{"ta":{"b":"!!!!","k":"ui8"}}"#,
        r#"{"ta":{"b":"AQID","k":"i32"}}"#,
        r#"{"ab":{"b":"AQ="}}"#,
        "1 2",
        "",
    ] {
        assert_eq!(read(text), Err("not the format".to_owned()), "{text}");
    }
}

#[test]
fn tests_that_a_number_prints_as_javascript_prints_it() {
    assert_eq!(js_number(42.0), "42");
    assert_eq!(js_number(1.5), "1.5");
    assert_eq!(js_number(-0.25), "-0.25");
    assert_eq!(js_number(1e21), "1e+21");
    assert_eq!(js_number(123_456_789_012.0), "123456789012");
    assert_eq!(js_number(1e-7), "1e-7");
    assert_eq!(js_number(0.000_001), "0.000001");
    assert_eq!(js_number(-0.0), "0");
    assert_eq!(js_number(f64::NAN), "NaN");
    assert_eq!(js_number(f64::NEG_INFINITY), "-Infinity");
    assert_eq!(js_number(f64::from(0.1_f32)), "0.10000000149011612");
}

/// A page that replaced the serializer's globals chooses its ids. They are renumbered in order of
/// appearance, as upstream's server writes them again, and every ref follows.
#[test]
fn tests_that_ids_and_refs_are_renumbered_in_order_of_appearance() -> TestResult {
    let tampered = r#"{"a":[{"o":[{"k":"self","v":{"ref":482913}}],"id":482913},{"ref":482913},{"a":[],"id":7}],"id":99}"#;
    assert_eq!(
        read(tampered)?,
        json!({ "a": [
            { "o": [{ "k": "self", "v": { "ref": 2 } }], "id": 2 },
            { "ref": 2 },
            { "a": [], "id": 3 },
        ], "id": 1 })
    );
    assert_eq!(
        read(r#"{"a":[{"ab":{"b":"AQI="}},{"o":[],"id":1}],"id":5}"#)?,
        json!({ "a": [{ "o": [], "id": 2 }, { "o": [], "id": 3 }], "id": 1 }),
        "an ArrayBuffer's object takes its place in the count, as upstream's server gives it"
    );
    Ok(())
}

/// A ref the page wrote before its object, or to an id two objects claim, or to the id an
/// `ArrayBuffer` stands in with, names nothing upstream's serializer could have written.
#[test]
fn tests_that_a_ref_to_an_id_not_yet_seen_fails() {
    for text in [
        r#"{"a":[{"ref":2},{"o":[],"id":2}],"id":1}"#,
        r#"{"a":[{"ref":9}],"id":1}"#,
        r#"{"a":[{"o":[],"id":1}],"id":1}"#,
        r#"{"a":[{"ab":{"b":""}},{"ref":0}],"id":1}"#,
        r#"{"ref":1}"#,
    ] {
        assert_eq!(read(text), Err("not the format".to_owned()), "{text}");
    }
}
