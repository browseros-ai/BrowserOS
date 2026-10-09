mod connection;
mod tab_groups;
mod tab_registry;

pub use connection::{BrowserConnectionState, BrowserService};
pub use tab_groups::{TabGroupColor, color_for_slug, hex_for_slug};
pub use tab_registry::TabRegistry;
