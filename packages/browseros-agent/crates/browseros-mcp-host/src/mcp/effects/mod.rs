//! Built-in dispatch effects. They run after every catalog tool in
//! `dispatch::DEFAULT_EFFECTS` order, each seeing the previous one's result: ownership
//! claims and the tabs-list view first, then tab activity and grouping, then the
//! appended notes (rename nudge, page ownership, unapplied target, helpers).

pub mod ownership_claims;
pub mod page_ownership_notice;
pub mod scope_notice;
pub mod session_naming;
pub mod tab_activity;
pub mod tab_groups;
pub mod tabs_list_view;
