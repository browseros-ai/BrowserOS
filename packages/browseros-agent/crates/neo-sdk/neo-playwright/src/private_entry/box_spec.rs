//! One box as the host names it for a private entry: a ref, the site it must be on, and the facts
//! the host approved. No value: this is what may be recorded.

use serde::{Deserialize, Deserializer, Serialize, de};
use ts_rs::TS;

/// What the host approved about a box. The look refuses the box when a fact the host sent no
/// longer holds (`box_changed`), so the box Neo types into is the box the host approved.
///
/// The look in the page reads the same shape (probe request `look`), so the facts the host
/// approved travel to the box's isolated world unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "BoxExpect")]
pub struct Expect {
    /// The origin of the box's own frame, `https://pay.example`. Required: Neo never types
    /// "wherever the ref ended up".
    pub origin: String,
    /// The input's `type` attribute as the DOM reports it (`password`, `text`, ...).
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<String>,
    /// The `autocomplete` attribute, exactly as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub autocomplete: Option<String>,
    /// The box's `maxlength`, or `-1` for a box that must have none (ruling R49), the DOM's own
    /// word for a box without one. Absent, it is not checked; `0` is a limit of zero. Any other
    /// negative number is refused as a malformed body.
    #[serde(
        default,
        deserialize_with = "max_length",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional)]
    pub max_length: Option<i32>,
    /// The `inputmode` attribute, exactly as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub input_mode: Option<String>,
    /// The `name` attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// The `id` attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
}

/// `expect.maxLength`: `-1`, or a limit of 0 or more.
fn max_length<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: Deserializer<'de>,
{
    match Option::<i64>::deserialize(deserializer)? {
        None => Ok(None),
        Some(-1) => Ok(Some(-1)),
        Some(limit) => i32::try_from(limit)
            .ok()
            .filter(|limit| *limit >= 0)
            .map(Some)
            .ok_or_else(|| {
                de::Error::custom("maxLength is -1, for a box with none, or a limit of 0 or more")
            }),
    }
}

/// One box of a private entry or a look, without its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoxSpec {
    /// The box's ref, as a snapshot printed it or `locator.ref()` minted it: `e5`, or `f2e4`
    /// for a box in frame document 2. A ref names one node and never re-binds; a gone node
    /// answers `element_detached`.
    #[serde(rename = "ref")]
    pub box_ref: String,
    /// The facts the box must still have.
    pub expect: Expect,
    /// Accept a value the box's own `maxlength` cuts: Neo types the part that fits.
    #[serde(default)]
    pub cut: bool,
    /// Compare the read-back with spaces, dashes and slashes set aside, for a box that groups
    /// digits (`4242 4242 …`) or formats an expiry (`12 / 29`).
    #[serde(default)]
    pub spaced: bool,
    /// How the value is typed. `false`, the default: one insertion, as `fill` types, for a
    /// password, a username, a card number or a code in one box; it needs a box that shows.
    /// `true`: key by key, with the focus checked before each character, for a code with one
    /// character in each box, a box with no size, or a masked box that needs key events
    /// (design section 5.5). A form that submits on its first input event takes only one
    /// insertion.
    #[serde(default)]
    pub keys: bool,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Expect;

    /// Ruling R49: `maxLength` takes -1 for a box with none, 0 and any limit; absent stays
    /// unchecked; any other negative number is a malformed body.
    #[test]
    fn tests_that_max_length_takes_minus_one_zero_and_limits_only() {
        let expect = |max_length: serde_json::Value| {
            serde_json::from_value::<Expect>(
                json!({ "origin": "https://a.example", "maxLength": max_length }),
            )
        };
        assert_eq!(expect(json!(-1)).ok().and_then(|e| e.max_length), Some(-1));
        assert_eq!(expect(json!(0)).ok().and_then(|e| e.max_length), Some(0));
        assert_eq!(expect(json!(19)).ok().and_then(|e| e.max_length), Some(19));
        assert!(expect(json!(-2)).is_err(), "-2 is malformed");
        assert!(expect(json!(-1.5)).is_err(), "not an integer");
        let absent = serde_json::from_value::<Expect>(json!({ "origin": "https://a.example" }));
        assert_eq!(absent.ok().and_then(|e| e.max_length), None);
    }
}
