//! Live MCP sessions, conversation-keyed page ownership and tab-group state, and the
//! persisted per-task browser target. `Sessions` owns lifetimes; its durable side
//! effects go through `seams::SessionJournal`.

mod manager;
mod session;
mod tab_ownership;
mod target_bindings;
mod usage;

#[doc(hidden)]
pub use manager::teardown_all;
pub use manager::{
    RetainedGroupAction, RetainedGroupHook, RetirementCause, SessionAuditFlushHook,
    SessionCompletionHook, Sessions,
};
pub use session::Session;
pub use tab_ownership::{
    PageOwnership, ScopedGroupKey, TabGroup, TabGroupColor, TabGroupState, TitleSync,
};
pub use target_bindings::{TargetBindings, TaskTargetStore};
pub use usage::{SessionUsageSnapshot, ToolUsageSnapshot};
