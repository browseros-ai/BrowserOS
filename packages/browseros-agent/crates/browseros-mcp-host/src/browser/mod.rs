//! The CDP link to BrowserOS: one `BrowserSession` per connection epoch, reattached
//! forever, plus the deterministic tab-group palette agents are colored with.

mod connection;
mod tab_groups;

pub use connection::{BrowserConnectionState, BrowserLinkStatus, BrowserService};
pub use tab_groups::{TabGroupColor, color_for_slug, hex_for_slug};
