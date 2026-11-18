//! Frozen JSON boundary for agent.js. Reports contain metadata, never field values.
#![allow(dead_code)]

use serde_json::{Value, json};

pub(super) struct Tap {
    pub at: u64,
    pub editable: bool,
}
pub(super) struct Focus {
    pub at: u64,
    pub editable: bool,
    pub inside: bool,
}
pub(super) struct Boxes {
    pub doc: u64,
    pub vp: Viewport,
    pub boxes: Vec<[f64; 4]>,
    pub focused: Option<usize>,
}
pub(super) struct Viewport {
    pub vv: [f64; 4],
    pub vv_scale: f64,
    pub inner: [f64; 2],
    pub dpr: f64,
    pub screen: [f64; 2],
    pub zoom: f64,
}
pub(super) enum Report {
    Tap(Tap),
    Focus(Focus),
    Input { at: u64 },
    Boxes(Boxes),
}

pub(super) fn parse_report(payload: &str) -> Option<Report> {
    if payload.len() > super::LIMIT {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    Some(match value["k"].as_str()? {
        "tap" => Report::Tap(Tap {
            at: value["at"].as_u64()?,
            editable: value["editable"].as_bool()?,
        }),
        "focus" => Report::Focus(Focus {
            at: value["at"].as_u64()?,
            editable: value["editable"].as_bool()?,
            inside: value["inside"].as_bool()?,
        }),
        "input" => Report::Input {
            at: value["at"].as_u64()?,
        },
        "boxes" => {
            let vp = &value["vp"];
            let vv = numbers(&vp["vv"])?;
            let inner = numbers(&vp["inner"])?;
            let screen = numbers(&vp["screen"])?;
            if vv[2] <= 0. || vv[3] <= 0. || inner.iter().chain(&screen).any(|n| *n <= 0.) {
                return None;
            }
            let raw_boxes = value["boxes"].as_array()?;
            if raw_boxes.len() > 128 {
                return None;
            }
            let boxes: Vec<[f64; 4]> = raw_boxes.iter().map(numbers).collect::<Option<_>>()?;
            if boxes.iter().any(|b| b[2] <= 0. || b[3] <= 0.) {
                return None;
            }
            let focused = match value.get("focused")? {
                Value::Null => None,
                i => {
                    let i = usize::try_from(i.as_u64()?).ok()?;
                    if i >= boxes.len() {
                        return None;
                    }
                    Some(i)
                }
            };
            Report::Boxes(Boxes {
                doc: value["doc"].as_u64()?,
                vp: Viewport {
                    vv,
                    vv_scale: positive(&vp["vv_scale"])?,
                    inner,
                    dpr: positive(&vp["dpr"])?,
                    screen,
                    zoom: positive(&vp["zoom"])?,
                },
                boxes,
                focused,
            })
        }
        _ => return None,
    })
}

fn numbers<const N: usize>(value: &Value) -> Option<[f64; N]> {
    let values = value.as_array()?;
    if values.len() != N {
        return None;
    }
    let mut out = [0.; N];
    for (dest, value) in out.iter_mut().zip(values) {
        *dest = value.as_f64().filter(|v| v.is_finite())?;
    }
    Some(out)
}

fn positive(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite() && *v > 0.)
}

pub(super) struct PrepareArg<'a> {
    pub id: u32,
    pub text: &'a str,
    pub refocus: bool,
    pub retry: bool,
}
impl PrepareArg<'_> {
    pub(super) fn to_value(&self) -> Value {
        json!({"id":self.id,"text":self.text,"refocus":self.refocus,"retry":self.retry})
    }
}
pub(super) struct VerifyArg {
    pub id: u32,
    pub window_ms: u32,
}
impl VerifyArg {
    pub(super) fn to_value(&self) -> Value {
        json!({"id":self.id,"windowMs":self.window_ms})
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Prepared {
    Ready,
    Handled,
    NotFocused,
    NoField { handled: bool },
}
impl Prepared {
    pub(super) fn parse(value: &Value) -> Option<Self> {
        Some(match value["state"].as_str()? {
            "ready" => Self::Ready,
            "handled" => Self::Handled,
            "notfocused" => Self::NotFocused,
            "nofield" => Self::NoField {
                handled: value["handled"].as_bool()?,
            },
            _ => return None,
        })
    }
}

pub(super) struct Evidence {
    pub changed: bool,
    pub before_input: u32,
    pub input: u32,
    pub focus_moved: bool,
    pub gone: bool,
}
impl Evidence {
    pub(super) fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            changed: value["changed"].as_bool()?,
            before_input: u32::try_from(value["beforeInput"].as_u64()?).ok()?,
            input: u32::try_from(value["input"].as_u64()?).ok()?,
            focus_moved: value["focusMoved"].as_bool()?,
            gone: value["gone"].as_bool()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tests_that_reports_keep_only_the_frozen_metadata() {
        assert!(matches!(
            parse_report(r#"{"k":"tap","at":42,"editable":true}"#),
            Some(Report::Tap(Tap {
                at: 42,
                editable: true
            }))
        ));
        assert!(matches!(
            parse_report(r#"{"k":"focus","at":43,"editable":false,"inside":true}"#),
            Some(Report::Focus(Focus {
                at: 43,
                editable: false,
                inside: true
            }))
        ));
        assert!(matches!(
            parse_report(r#"{"k":"input","at":44}"#),
            Some(Report::Input { at: 44 })
        ));
    }

    fn boxes() -> Value {
        json!({"k":"boxes","doc":7,
            "vp":{"vv":[1.5,2.5,390,844],"vv_scale":1,"inner":[390,844],"dpr":3,"screen":[390,844],"zoom":2},
            "boxes":[[10.5,20,30,40]],"focused":0})
    }

    #[test]
    fn tests_that_boxes_preserve_css_geometry_and_nullable_focus() {
        let mut value = boxes();
        let Some(Report::Boxes(parsed)) = parse_report(&value.to_string()) else {
            panic!("boxes rejected")
        };
        assert_eq!(parsed.doc, 7);
        assert_eq!(parsed.vp.vv, [1.5, 2.5, 390., 844.]);
        assert_eq!(parsed.vp.inner, [390., 844.]);
        assert_eq!(parsed.vp.screen, [390., 844.]);
        assert_eq!(
            (parsed.vp.vv_scale, parsed.vp.dpr, parsed.vp.zoom),
            (1., 3., 2.)
        );
        assert_eq!(parsed.boxes, [[10.5, 20., 30., 40.]]);
        assert_eq!(parsed.focused, Some(0));
        value["focused"] = Value::Null;
        assert!(matches!(
            parse_report(&value.to_string()),
            Some(Report::Boxes(Boxes { focused: None, .. }))
        ));
    }

    #[test]
    fn tests_that_malformed_reports_are_rejected() {
        for payload in [
            "",
            "null",
            "[]",
            "{}",
            r#"{"k":"unknown"}"#,
            r#"{"k":"tap","at":-1,"editable":true}"#,
            r#"{"k":"tap","at":1,"editable":1}"#,
            r#"{"k":"focus","at":1,"editable":true}"#,
            r#"{"k":"input","at":1.5}"#,
        ] {
            assert!(parse_report(payload).is_none());
        }
        for (key, bad) in [
            ("focused", json!(1)),
            ("focused", json!(-1)),
            ("doc", json!("7")),
            ("boxes", json!([[1, 2, 3]])),
            ("boxes", json!([[1, 2, -1, 4]])),
            ("boxes", json!(vec![[0, 0, 1, 1]; 129])),
        ] {
            let mut value = boxes();
            value[key] = bad;
            assert!(parse_report(&value.to_string()).is_none());
        }
        for (key, bad) in [
            ("vv", json!([0, 0, 10])),
            ("vv_scale", json!(0)),
            ("inner", json!([0, 844])),
            ("dpr", json!(-1)),
            ("zoom", json!(null)),
        ] {
            let mut value = boxes();
            value["vp"][key] = bad;
            assert!(parse_report(&value.to_string()).is_none());
        }
    }

    #[test]
    fn tests_that_call_arguments_and_results_match_javascript_names() {
        assert_eq!(
            PrepareArg {
                id: 9,
                text: "café",
                refocus: true,
                retry: false
            }
            .to_value(),
            json!({"id":9,"text":"café","refocus":true,"retry":false})
        );
        assert_eq!(
            VerifyArg {
                id: 9,
                window_ms: 300
            }
            .to_value(),
            json!({"id":9,"windowMs":300})
        );
        assert_eq!(
            Prepared::parse(&json!({"state":"ready"})),
            Some(Prepared::Ready)
        );
        assert_eq!(
            Prepared::parse(&json!({"state":"handled"})),
            Some(Prepared::Handled)
        );
        assert_eq!(
            Prepared::parse(&json!({"state":"notfocused"})),
            Some(Prepared::NotFocused)
        );
        assert_eq!(
            Prepared::parse(&json!({"state":"nofield","handled":true})),
            Some(Prepared::NoField { handled: true })
        );
        for value in [
            json!({}),
            json!({"state":"unknown"}),
            json!({"state":"nofield"}),
            json!({"state":"nofield","handled":1}),
        ] {
            assert!(Prepared::parse(&value).is_none());
        }
        let value =
            json!({"changed":true,"beforeInput":2,"input":3,"focusMoved":false,"gone":false});
        let e = Evidence::parse(&value).unwrap();
        assert!(e.changed);
        assert_eq!(
            (e.before_input, e.input, e.focus_moved, e.gone),
            (2, 3, false, false)
        );
        for (key, bad) in [
            ("beforeInput", json!(-1)),
            ("input", json!(u64::MAX)),
            ("changed", json!(1)),
            ("gone", Value::Null),
        ] {
            let mut bad_value = value.clone();
            bad_value[key] = bad;
            assert!(Evidence::parse(&bad_value).is_none());
        }
    }
}
