//! The `DOMStorage` domain as neo-runtime uses it: an origin's `localStorage`, read and replaced
//! in place over the session of a tab open on it, for `storageState` and `setStorageState`
//! (`browser_representations/cookies_and_storage.rs`).
//!
//! Neo: local structs, since browseros-cdp's surface has no `DOMStorage` domain.

use serde::Serialize;
use serde_json::Value;

use super::{Ignored, OnSession, command};

/// CDP's `DOMStorage.StorageId` of an origin's `localStorage`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageId {
    pub(crate) security_origin: String,
    pub(crate) is_local_storage: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Clear {
    pub(crate) storage_id: StorageId,
}

#[derive(Serialize)]
pub(crate) struct Enable {}

/// `DOMStorage.getDOMStorageItems`. Its reply stays JSON: `cookies_and_storage.rs` decodes the
/// entries with its own error.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetDomStorageItems {
    pub(crate) storage_id: StorageId,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetDomStorageItem {
    pub(crate) storage_id: StorageId,
    pub(crate) key: String,
    pub(crate) value: String,
}

command! {
    // Neo: `setStorageState` replaces an origin's `localStorage` in place over its tab's session, where Playwright evaluates a restore script in a page of that origin.
    Clear => "DOMStorage.clear", OnSession, Ignored;
    // Neo: the domain `storageState` and `setStorageState` read and write an origin's `localStorage` through.
    Enable => "DOMStorage.enable", OnSession, Ignored;
    // Neo: `storageState` reads an origin's `localStorage` in place over its tab's session, where Playwright evaluates a collect script in a page of that origin.
    GetDomStorageItems => "DOMStorage.getDOMStorageItems", OnSession, Value;
    // Neo: `setStorageState` writes an origin's `localStorage` items in place, one at a time.
    SetDomStorageItem => "DOMStorage.setDOMStorageItem", OnSession, Ignored;
}
