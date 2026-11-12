//! A live tab, as Neo names it internally.

use browseros_core::TabId;
use browseros_core::pages::PageInfo;
use serde::{Serialize, Serializer};
use ts_rs::TS;

/// A live browser tab resolved by the dispatcher before policies run. This is the only page
/// identity on the wire, where it is the page brand `{"$neo": "page", "tab": …}`; core's private
/// cache id is held separately for one operation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, TS)]
#[ts(as = "PageBrand")]
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
        PageBrand {
            brand: Brand::Page,
            tab: self.tab,
        }
        .serialize(serializer)
    }
}

/// A [`PageRef`] on the wire: the page brand the bootstrap revives into the prelude's `Page`
/// for that tab.
#[derive(Serialize, TS)]
pub(crate) struct PageBrand {
    #[serde(rename = "$neo")]
    #[ts(type = "'page'")]
    brand: Brand,
    tab: TabId,
}

/// The only brand a page carries.
#[derive(Clone, Copy, Serialize)]
enum Brand {
    #[serde(rename = "page")]
    Page,
}
