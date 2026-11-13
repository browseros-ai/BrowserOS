//! The page handle ops hand back to the prelude, the output twin of `target::PageRef`.

use browseros_core::TabId;
use serde::Serialize;
use ts_rs::TS;

/// A tab-only output brand. The prelude caches one `Page` per tab, so popup events,
/// `context.pages()` and `page.opener()` agree on object identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(rename = "PageBrand")]
pub(crate) struct PageHandle {
    #[serde(rename = "$neo")]
    #[ts(type = "'page'")]
    brand: Brand,
    tab: TabId,
}

/// The only brand a page handle carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum Brand {
    #[serde(rename = "page")]
    Page,
}

impl PageHandle {
    /// The browser identity needed by host-side event reporting.
    pub(crate) fn tab(&self) -> TabId {
        self.tab
    }

    /// The handle for the browser tab.
    pub(crate) fn new(tab: TabId) -> Self {
        Self {
            brand: Brand::Page,
            tab,
        }
    }
}
