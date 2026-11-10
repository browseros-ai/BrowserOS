//! The `DOMStorage` domain as neo-runtime uses it: an origin's `localStorage`, read and replaced
//! in place over the session of a tab open on it, for `storageState` and `setStorageState`
//! (`browser_representations/cookies_and_storage.rs`).
//!
//! Neo: browseros-cdp's generated params and results, made from BrowserOS's pinned protocol,
//! under each method's name; a local type only where its doc says why.

use browseros_cdp::dom_storage as generated;
use generated::GetDOMStorageItemsResult;
use serde::Serialize;

use super::{Ignored, OnSession, command};

pub(crate) use generated::{
    ClearParams as Clear, GetDOMStorageItemsParams as GetDomStorageItems,
    SetDOMStorageItemParams as SetDomStorageItem, StorageId,
};

/// `DOMStorage.enable`. Local: it takes no params, and browseros-cdp generates no params type
/// for such a command.
#[derive(Serialize)]
pub(crate) struct Enable {}

command! {
    // Neo: `setStorageState` replaces an origin's `localStorage` in place over its tab's session, where Playwright evaluates a restore script in a page of that origin.
    Clear => "DOMStorage.clear", OnSession, Ignored;
    // Neo: the domain `storageState` and `setStorageState` read and write an origin's `localStorage` through.
    Enable => "DOMStorage.enable", OnSession, Ignored;
    // Neo: `storageState` reads an origin's `localStorage` in place over its tab's session, where Playwright evaluates a collect script in a page of that origin.
    GetDomStorageItems => "DOMStorage.getDOMStorageItems", OnSession, GetDOMStorageItemsResult;
    // Neo: `setStorageState` writes an origin's `localStorage` items in place, one at a time.
    SetDomStorageItem => "DOMStorage.setDOMStorageItem", OnSession, Ignored;
}
