//! A live tab, as Neo names it internally.

use browseros_core::pages::PageInfo;
use browseros_core::{PageId, TabId};
use serde::{Serialize, Serializer};

/// A tab resolved from core's cached page info, without a CDP call. The dispatcher resolves
/// every op's target to one before its rules run, so the handler, the rules and the trace all
/// see the same tab.
///
/// Rules, locks and records key by `tab`, the browser's id, which survives a Neo restart. `id`
/// is what core's calls take; it is process-local and restarts at 1 with Neo. A plain data bag.
///
/// It serializes as the page brand, `{"$neo":"page","id":7,"tab":1234}`: a method that returns
/// a page returns a `PageRef`, the bootstrap revives the brand into a `Page` object, and a later
/// op naming that page sends the same brand back (`dispatch::PageTarget`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PageRef {
    /// Core's process-local page id.
    pub(crate) id: PageId,
    /// The browser's tab id.
    pub(crate) tab: TabId,
}

impl From<&PageInfo> for PageRef {
    fn from(info: &PageInfo) -> Self {
        Self {
            id: info.page_id.clone(),
            tab: info.tab_id,
        }
    }
}

impl Serialize for PageRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut brand = serializer.serialize_struct("PageRef", 3)?;
        brand.serialize_field("$neo", "page")?;
        brand.serialize_field("id", &self.id.0)?;
        brand.serialize_field("tab", &self.tab.0)?;
        brand.end()
    }
}
