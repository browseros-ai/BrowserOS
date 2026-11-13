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
import {
  withMarkedFieldsMasked,
  withMaskedFieldValues,
} from './private-field-values'
import type { ReplyTo, SnapshotIframe, SnapshotRequest } from './protocol'
import { type Found, resolveTarget } from './resolve'
import { openClosedRoots } from './shadow-roots'
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
 * `nodes`, the call's nodes after its root, come from clickable marking's read of the frame.
 * Rust sends them only in ai mode and never under the spec harness's conformance switch. The
 * first `closedRoots` sit inside closed shadow roots, opened to this world's engine before
 * resolving the root or marking so both read inside them, now and in every later call
 * (shadow-roots.ts). The rest are the elements Chromium reports clickable without a pointer
 * cursor (pointer-targets.ts). Value masking is independent of marks: every normal tree,
 * including marking's fallback, is masked before it can leave this frame's isolated world.
 */
export function snapshot(
  world: World,
  request: SnapshotRequest,
  root: Node | undefined,
  nodes: readonly unknown[],
): ReplyTo<'snapshot'> {
  const seeds = request.closedRoots ?? 0
  openClosedRoots(nodes.slice(0, seeds))
  const targets = nodes.slice(seeds)
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
  const take = () => marked(targets, engine, plain)
  // Under the spec harness's conformance switch the page's own private fields print as upstream
  // prints them, but a box Neo typed a host's secret into stays masked: the mark has no bypass
  // (ruling R44). Until this world marks a box, that snapshot installs nothing.
  const { result, skipped } = request.conformance
    ? withMarkedFieldsMasked(take)
    : withMaskedFieldValues(
        {
          root: element,
          // Rust's topology includes frames hidden from window.frames by shadow roots.
          // Unknown hints still discover foreign prototypes; false only skips that search.
          mayHaveOtherFrame: request.mayHaveOtherFrame !== false,
        },
        take,
      )
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
 * snapshot is taken again without marks, still inside value masking's scope outside
 * conformance. `skipped` tells Rust, which notes it on the op's call log.
 */
function marked<T>(
  targets: readonly unknown[],
  engine: EngineReads,
  snapshot: () => T,
): { result: T; skipped: boolean } {
  const elements = targets.filter((node) => node instanceof Element)
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
