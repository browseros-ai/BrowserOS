//! The `DOM` domain as neo-runtime uses it: an element's geometry and scrolling it into view,
//! a frame's owner element, adopting a node into a world, and a file input's files, by the
//! object id or backend node id of what Neo holds.
//!
//! Neo: browseros-cdp's generated params and results, made from BrowserOS's pinned protocol,
//! under each method's name.

use browseros_cdp::dom as generated;
use generated::{
    DescribeNodeResult, GetBoxModelResult, GetContentQuadsResult, GetFrameOwnerResult,
    ResolveNodeResult,
};

use super::{Ignored, OnSession, command};

pub(crate) use generated::{
    DescribeNodeParams as DescribeNode, GetBoxModelParams as GetBoxModel,
    GetContentQuadsParams as GetContentQuads, GetFrameOwnerParams as GetFrameOwner,
    ResolveNodeParams as ResolveNode, ScrollIntoViewIfNeededParams as ScrollIntoViewIfNeeded,
    SetFileInputFilesParams as SetFileInputFiles,
};

command! {
    // Playwright: `server/chromium/crPage.ts` `FrameSession._getOwnerFrame`: `const nodeInfo = await this._client.send('DOM.describeNode', {`
    DescribeNode => "DOM.describeNode", OnSession, DescribeNodeResult;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._getBoundingBox`: `const result = await this._client._sendMayFail('DOM.getBoxModel', {`
    GetBoxModel => "DOM.getBoxModel", OnSession, GetBoxModelResult;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._getContentQuads`: `const result = await this._client._sendMayFail('DOM.getContentQuads', {`
    GetContentQuads => "DOM.getContentQuads", OnSession, GetContentQuadsResult;
    // Playwright: `server/chromium/crPage.ts` `CRPage.getFrameElement`: `const { backendNodeId } = await parentSession._client.send('DOM.getFrameOwner', { frameId: frame._id }).catch(e => {`
    GetFrameOwner => "DOM.getFrameOwner", OnSession, GetFrameOwnerResult;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._adoptBackendNodeId`: `const result = await this._client._sendMayFail('DOM.resolveNode', {`
    ResolveNode => "DOM.resolveNode", OnSession, ResolveNodeResult;
    // Playwright: `server/chromium/crPage.ts` `FrameSession._scrollRectIntoViewIfNeeded`: `return await this._client.send('DOM.scrollIntoViewIfNeeded', {`
    ScrollIntoViewIfNeeded => "DOM.scrollIntoViewIfNeeded", OnSession, Ignored;
    // Playwright: `server/chromium/crPage.ts` `CRPage.setInputFilePaths`: `await progress.race(parentSession._client.send('DOM.setFileInputFiles', {`
    SetFileInputFiles => "DOM.setFileInputFiles", OnSession, Ignored;
}
