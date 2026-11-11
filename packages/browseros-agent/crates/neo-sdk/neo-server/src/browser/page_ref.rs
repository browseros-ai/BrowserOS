//! A live tab, as Neo names it internally.

use browseros_core::TabId;
use browseros_core::pages::PageInfo;
use serde::{Serialize, Serializer};

/// A live browser tab resolved by the dispatcher before policies run. This is the only page
/// identity on the wire; core's private cache id is held separately for one operation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PageRef {
    /// The browser's tab id.
    pub(crate) tab: TabId,
}

impl From<&PageInfo> for PageRef {
    fn from(info: &PageInfo) -> Self {
        Self { tab: info.tab_id }
    }
}

impl Serialize for PageRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut brand = serializer.serialize_struct("PageRef", 2)?;
        brand.serialize_field("$neo", "page")?;
        brand.serialize_field("tab", &self.tab.0)?;
        brand.end()
    }
}
