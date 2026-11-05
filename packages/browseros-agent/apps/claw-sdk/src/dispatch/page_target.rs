//! Pages as a script names them. The door resolves one to a `browser::PageRef`.

use browseros_core::{PageId, TabId};
use serde::{Deserialize, Deserializer};

/// A page as an op's input names it, from the page brand the script's `Page` objects carry:
/// `{"$neo":"page","id":7,"tab":1234}`, or `{"$neo":"page","id":null}` for the run's default
/// page before it has a tab.
///
/// Opaque outside `dispatch/`: a method hands it back through `Method::target` and receives
/// the resolved, checked tab from `OpCtx::page`, so no method can act on a handle the door has
/// not verified. Core's page ids restart at 1 when Neo restarts, so a handle pasted from an
/// old result could name another tab; the door refuses one whose `tab` no longer matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageTarget(pub(super) TargetHandle);

/// What a page brand claims, before the door checks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TargetHandle {
    /// A page the script holds a handle for.
    Page {
        /// Core's process-local page id.
        id: PageId,
        /// The tab the handle was issued for.
        tab: TabId,
    },
    /// The run's default page: the task's tab, or one opened on first use.
    RunDefault,
}

/// The brand's wire form. Every field the prelude sends is named, so a typo is refused.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Brand {
    #[serde(rename = "$neo")]
    kind: BrandKind,
    #[serde(default)]
    id: Option<u32>,
    #[serde(default)]
    tab: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum BrandKind {
    Page,
}

impl<'de> Deserialize<'de> for PageTarget {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let Brand {
            kind: BrandKind::Page,
            id,
            tab,
        } = Brand::deserialize(deserializer)?;
        match (id, tab) {
            (Some(id), Some(tab)) => Ok(Self(TargetHandle::Page {
                id: PageId(id),
                tab: TabId(tab),
            })),
            (None, None) => Ok(Self(TargetHandle::RunDefault)),
            (Some(_), None) | (None, Some(_)) => Err(D::Error::custom(
                "a page handle carries both its id and its tab; use a page from context.pages()",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::{PageId, TabId};
    use serde_json::json;

    use super::{PageTarget, TargetHandle};
    use crate::browser::PageRef;

    #[test]
    fn a_page_handle_round_trips_through_its_brand() -> Result<(), Box<dyn Error>> {
        let page = PageRef {
            id: PageId(7),
            tab: TabId(1234),
        };
        let brand = serde_json::to_value(&page)?;
        assert_eq!(
            brand,
            json!({ "$neo": "page", "id": 7, "tab": 1234 }),
            "the brand scripts get"
        );
        assert_eq!(
            serde_json::from_value::<PageTarget>(brand)?,
            PageTarget(TargetHandle::Page {
                id: PageId(7),
                tab: TabId(1234),
            }),
            "what a method returns is what a later op can name"
        );
        Ok(())
    }

    #[test]
    fn a_handle_without_an_id_is_the_run_default() -> Result<(), Box<dyn Error>> {
        for brand in [
            json!({ "$neo": "page" }),
            json!({ "$neo": "page", "id": null }),
        ] {
            assert_eq!(
                serde_json::from_value::<PageTarget>(brand)?,
                PageTarget(TargetHandle::RunDefault),
                "the `page` global before it has a tab"
            );
        }
        Ok(())
    }

    #[test]
    fn malformed_handles_are_refused() {
        for brand in [
            json!({ "$neo": "page", "id": 7 }),
            json!({ "$neo": "page", "tab": 1234 }),
            json!({ "$neo": "frame", "id": 7, "tab": 1234 }),
            json!({ "$neo": "page", "id": 7, "tab": 1234, "url": "https://example.com/" }),
            json!({ "id": 7, "tab": 1234 }),
            json!(7),
        ] {
            assert!(
                serde_json::from_value::<PageTarget>(brand.clone()).is_err(),
                "{brand} is not a page handle"
            );
        }
    }
}
