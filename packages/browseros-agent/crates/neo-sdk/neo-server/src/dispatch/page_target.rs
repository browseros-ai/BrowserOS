//! Pages as a script names them. The door resolves one to a `browser::PageRef`.

use browseros_core::TabId;
use serde::{Deserialize, Deserializer};

/// A script's tab-only page brand, or `{tab:null}` for its lazy default page. The door
/// resolves it before any policy or method runs; old `id` fields are rejected by the decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageTarget(pub(super) TargetHandle);

/// What a page brand claims, before the door checks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TargetHandle {
    /// A page the script holds a handle for.
    Page {
        /// The tab the handle was issued for.
        tab: TabId,
    },
    /// The run's default page: the task's tab, or one opened on first use.
    RunDefault,
}

impl PageTarget {
    /// Whether this names the run's own page before it has a tab (`tab: null`), which the door
    /// opens on first use. The prelude sends that brand wherever the page is named, so one input
    /// can carry it more than once.
    #[must_use]
    pub(crate) fn is_run_default(&self) -> bool {
        matches!(self.0, TargetHandle::RunDefault)
    }
}

/// The brand's wire form. Every field the prelude sends is named, so a typo is refused.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Brand {
    #[serde(rename = "$neo")]
    kind: BrandKind,
    // A missing field is not the lazy page: only an explicit null means unopened.
    #[serde(deserialize_with = "Option::deserialize")]
    tab: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum BrandKind {
    Page,
}

impl<'de> Deserialize<'de> for PageTarget {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Brand {
            kind: BrandKind::Page,
            tab,
        } = Brand::deserialize(deserializer)?;
        Ok(Self(match tab {
            Some(tab) => TargetHandle::Page { tab: TabId(tab) },
            None => TargetHandle::RunDefault,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{PageTarget, TargetHandle};
    use crate::browser::PageRef;
    use browseros_core::TabId;
    use serde_json::json;
    use std::error::Error;

    #[test]
    fn a_page_handle_round_trips_through_its_brand() -> Result<(), Box<dyn Error>> {
        let page = PageRef { tab: TabId(1234) };
        let brand = serde_json::to_value(&page)?;
        assert_eq!(brand, json!({ "$neo": "page", "tab": 1234 }));
        assert_eq!(
            serde_json::from_value::<PageTarget>(brand)?,
            PageTarget(TargetHandle::Page { tab: TabId(1234) })
        );
        Ok(())
    }

    #[test]
    fn explicit_null_tab_is_the_run_default() -> Result<(), Box<dyn Error>> {
        assert_eq!(
            serde_json::from_value::<PageTarget>(json!({ "$neo": "page", "tab": null }))?,
            PageTarget(TargetHandle::RunDefault)
        );
        Ok(())
    }

    #[test]
    fn malformed_and_obsolete_handles_are_refused() {
        for brand in [
            json!({ "$neo": "page" }),
            json!({ "$neo": "page", "id": 7, "tab": 1234 }),
            json!({ "$neo": "page", "id": null }),
            json!({ "$neo": "frame", "tab": 1234 }),
            json!({ "$neo": "page", "tab": 1234, "url": "https://example.com/" }),
            json!({ "tab": 1234 }),
            json!(7),
        ] {
            assert!(
                serde_json::from_value::<PageTarget>(brand.clone()).is_err(),
                "{brand} is not a tab-only page handle"
            );
        }
    }
}
