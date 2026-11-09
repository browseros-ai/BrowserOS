//! The `DOM` domain as neo-runtime uses it: an element's geometry and scrolling it into view,
//! a frame's owner element, adopting a node into a world, and a file input's files, by the
//! object id or backend node id of what Neo holds.
//!
//! Neo: browseros-cdp's generated params under each method's name. Replies stay JSON, read as
//! leniently as each caller reads them, or ignored.

use browseros_cdp::dom as generated;
use serde_json::Value;

use super::{Ignored, OnSession, command};

pub(crate) use generated::{
    DescribeNodeParams as DescribeNode, GetBoxModelParams as GetBoxModel,
    GetContentQuadsParams as GetContentQuads, GetFrameOwnerParams as GetFrameOwner, Rect,
    ResolveNodeParams as ResolveNode, ScrollIntoViewIfNeededParams as ScrollIntoViewIfNeeded,
    SetFileInputFilesParams as SetFileInputFiles,
};

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._getOwnerFrame`: `const nodeInfo = await this._client.send('DOM.describeNode', {`
    DescribeNode => "DOM.describeNode", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._getBoundingBox`: `const result = await this._client._sendMayFail('DOM.getBoxModel', {`
    GetBoxModel => "DOM.getBoxModel", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._getContentQuads`: `const result = await this._client._sendMayFail('DOM.getContentQuads', {`
    GetContentQuads => "DOM.getContentQuads", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `CRPage.getFrameElement`: `const { backendNodeId } = await parentSession._client.send('DOM.getFrameOwner', { frameId: frame._id }).catch(e => {`
    GetFrameOwner => "DOM.getFrameOwner", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._adoptBackendNodeId`: `const result = await this._client._sendMayFail('DOM.resolveNode', {`
    ResolveNode => "DOM.resolveNode", OnSession, Value;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._scrollRectIntoViewIfNeeded`: `return await this._client.send('DOM.scrollIntoViewIfNeeded', {`
    ScrollIntoViewIfNeeded => "DOM.scrollIntoViewIfNeeded", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `CRPage.setInputFilePaths`: `await progress.race(parentSession._client.send('DOM.setFileInputFiles', {`
    SetFileInputFiles => "DOM.setFileInputFiles", OnSession, Ignored;
}
