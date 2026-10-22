pub mod browser;
pub mod capabilities;
pub mod connection;
pub mod content_markdown;
pub mod credentials;
pub mod error;
pub mod frames;
pub mod input;
pub mod navigation;
pub mod observer;
pub mod page_signals;
pub mod pages;
pub mod profiles;
pub mod screenshot;
pub mod settle;
pub mod snapshot;
pub mod target;
pub mod timeouts;
pub mod types;
pub mod windows;

pub use browser::Browser;
pub use capabilities::{BrowserOsVersion, NativeCapabilities, NativeFeature};
pub use connection::{CdpConnection, ProtocolSession};
pub use error::{CoreError, CoveredElementTarget, TargetError};
pub use page_signals::{ConsoleEntry, ConsoleLevel, PageDialog, PageSignals};
pub use profiles::{
    BrowserProfileInfo, DEFAULT_PROFILE_DIRECTORY, LoadProfileOutcome, ProfileCandidate,
    ProfileMeta, ProfileSelector, default_profile, resolve_profile,
};
pub use session::{BrowserSession, BrowserSessionHooks};
pub use target::{
    BrowserView, CapturedScope, PageAdmission, PageListing, TargetBinding, TargetIntent,
    TargetSource, TargetStatus, WindowSelection,
};
pub use types::{
    BrowserContextId, BrowserProfileId, FrameId, PageId, Ref, SessionId, TabId, TargetId, WindowId,
};

mod session;
#[cfg(test)]
mod test_support;
