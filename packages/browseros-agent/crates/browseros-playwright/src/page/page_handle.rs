//! The page handle ops hand back to the prelude, the output twin of `target::PageRef`.

use browseros_core::{PageId, TabId};
use serde::Serialize;

/// `{"$neo":"page","id":7,"tab":1234}` in an op's output, the skeleton's page brand. The
/// prelude's `page` brand turns it into the one `Page` object it keeps for that id, so a popup,
/// `context.pages()` and `page.opener()` that name the same tab give the script the same object.
/// `tab` lets a later op that names the page be refused once the id names another tab.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct PageHandle {
    #[serde(rename = "$neo")]
    brand: Brand,
    id: PageId,
    tab: TabId,
}

/// The only brand a page handle carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
enum Brand {
    #[serde(rename = "page")]
    Page,
}

impl PageHandle {
    /// The handle for tab `tab`, which core calls `id`.
    pub(crate) fn new(id: PageId, tab: TabId) -> Self {
        Self {
            brand: Brand::Page,
            id,
            tab,
        }
    }
}
