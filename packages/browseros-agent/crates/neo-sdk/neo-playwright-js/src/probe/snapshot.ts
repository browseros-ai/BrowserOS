/**
 * The `snapshot` request: one frame's half of upstream's server algorithm
 * (server/page.ts `ariaSnapshotJSONForFrame` at v1.64.1). It runs Playwright's own
 * `ariaSnapshotJSON` over this frame's document or one element, and returns the iframes it
 * rendered for Rust to hop into. Rust stitches the frames; nothing here reads a child frame.
 *
 * In ai mode the engine mints refs under this world's prefix (`f<seq>e<n>`, bare `e<n>` at
 * seq 0, because `createEngine` passed the seq), and every ref goes into Neo's per-document ref
 * map. The map accumulates, so a ref printed by an earlier or a scoped snapshot still names its
 * element; iframe refs are in it too, which is how Rust's hop finds each iframe again.
 */
import { parseSelector } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type { ReplyTo, SnapshotIframe, SnapshotRequest } from './protocol'
import { type Found, resolveTarget } from './resolve'
import { frameSource, lastSnapshotRefs, type World } from './world'

/**
 * Upstream's frame root. Non-strict and first match, because a page of nested framesets has
 * several `frameset` elements.
 */
const FRAME_ROOT = parseSelector('body,frameset')

/** The element a frame-level snapshot starts at; none while the document has no body yet. */
function frameRoot(world: World): Found {
  const { injected } = world
  const element = injected.querySelector(FRAME_ROOT, injected.document, false)
  return element ? { kind: 'one', element } : { kind: 'none' }
}

export function snapshot(
  world: World,
  request: SnapshotRequest,
  root: Node | undefined,
): ReplyTo<'snapshot'> {
  // A locator's element is strict in both modes, as upstream's locator.ariaSnapshot is.
  const found =
    request.root === 'document'
      ? frameRoot(world)
      : resolveTarget(world, request.root, 'strict', root)
  if (found.kind !== 'one') return found
  const { injected } = world
  const { json, iframeRefs, iframeDepths } = injected.ariaSnapshotJSON(
    found.element,
    { mode: request.mode, depth: request.depth, boxes: request.boxes },
  )
  const iframes: SnapshotIframe[] = []
  if (request.mode === 'ai') {
    const minted = lastSnapshotRefs(injected)
    world.refs.remember(minted)
    // Only the iframes the depth limit rendered, as upstream's `renderedIframeRefs`.
    for (const ref of iframeRefs) {
      const depth = iframeDepths[ref]
      const element = minted.get(ref)?.element
      if (depth === undefined || element === undefined) continue
      iframes.push({ ref, depth, src: frameSource(element) ?? '' })
    }
  }
  return {
    kind: 'snapshot',
    json,
    iframes,
  }
}
