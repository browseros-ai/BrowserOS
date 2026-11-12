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
import {
  asPointerTargets,
  type EngineReads,
  leafTargets,
} from './pointer-targets'
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

/**
 * `nodes`, the call's nodes after its root, are the elements Rust found clickable without a
 * pointer cursor (clickable marking, pointer-targets.ts); Rust sends them only in ai mode, and
 * never under the spec harness's conformance switch, so without them this is upstream's call.
 */
export function snapshot(
  world: World,
  request: SnapshotRequest,
  root: Node | undefined,
  nodes: readonly unknown[],
): ReplyTo<'snapshot'> {
  // A locator's element is strict in both modes, as upstream's locator.ariaSnapshot is.
  const found =
    request.root === 'document'
      ? frameRoot(world)
      : resolveTarget(world, request.root, 'strict', root)
  if (found.kind !== 'one') return found
  const { injected } = world
  const element = found.element
  const plain = () =>
    injected.ariaSnapshotJSON(element, {
      mode: request.mode,
      depth: request.depth,
      boxes: request.boxes,
    })
  const engine: EngineReads = {
    roleOf: (of) => injected.utils.getAriaRole(of),
    isVisible: (of) => injected.utils.isElementVisible(of),
  }
  const { result, skipped } = marked(nodes, engine, plain)
  const { json, iframeRefs, iframeDepths } = result
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
    ...(skipped ? { marksSkipped: true } : {}),
  }
}

/**
 * `snapshot` with clickable marking's targets, or plainly without any. Marking must never fail a
 * snapshot: if the guard or the marked engine call throws on a page shape nobody foresaw, the
 * snapshot is taken again as upstream takes it, with every read real, and `skipped` tells Rust,
 * which notes it on the op's call log.
 */
function marked<T>(
  nodes: readonly unknown[],
  engine: EngineReads,
  snapshot: () => T,
): { result: T; skipped: boolean } {
  const elements = nodes.filter((node) => node instanceof Element)
  if (elements.length === 0) return { result: snapshot(), skipped: false }
  try {
    return {
      result: asPointerTargets(leafTargets(elements, engine), snapshot),
      skipped: false,
    }
  } catch {
    return { result: snapshot(), skipped: true }
  }
}
