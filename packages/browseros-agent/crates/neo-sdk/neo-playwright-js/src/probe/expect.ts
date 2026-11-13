/**
 * `expect`: one poll of one retrying matcher, matched by Playwright's own per-element and
 * per-array checks (world.ts `expectChecks`). Rust's loop owns the timing, `.not`, the failure
 * and its message; a poll only reports what it saw, and never waits.
 */
import type { FrameExpectParams } from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'
import { withMarkedFieldsMasked } from './private-field-values'
import type { ExpectPoll, ExpectRequest, ReplyTo } from './protocol'
import { type Found, queryAll, resolveTarget } from './resolve'
import {
  type ExpectCheck,
  expectChecks,
  expectFailureSnapshot,
  type World,
} from './world'

/**
 * Where `expect(page).toMatchAriaSnapshot` matches its template: upstream's `body`
 * (Frame._expectInternal's effectiveSelector), read as the document's own body. The selector
 * also pierces shadow roots, and on BrowserOS the claw app mounts its takeover bar in a shadow
 * root with a body of its own, so the selector would be a strict violation on every page. A
 * frameset document's `document.body` is its frameset, which `body` does not match; `localName`
 * holds for an XHTML document too, where `nodeName` is lowercase.
 */
function pageAriaRoot(world: World): Found {
  const body = world.injected.document.body
  return body?.localName === 'body'
    ? { kind: 'one', element: body }
    : { kind: 'none' }
}

export async function expect(
  world: World,
  request: ExpectRequest,
  root: Node | undefined,
): Promise<ReplyTo<'expect'>> {
  const params: FrameExpectParams = request.params
  const checks = expectChecks(world.injected)
  // Upstream's split (InjectedScript.expect): count and the array forms read every match,
  // and zero matches is a value for them, not a missing element.
  if (
    params.expression === 'to.have.count' ||
    params.expression.endsWith('.array')
  ) {
    const elements = queryAll(world, request.target, root)
    const count = `${elements.length} element${elements.length === 1 ? '' : 's'}`
    return poll(
      withMarkedFieldsMasked(() => checks.every(elements, params)),
      count,
    )
  }
  // A page matcher's element is the document's root (upstream's `:root`), except an aria
  // template's.
  const found =
    params.expression === 'to.match.aria' && request.target === 'document'
      ? pageAriaRoot(world)
      : resolveTarget(world, request.target, request.strictness, root)
  if (found.kind !== 'one') return found
  const { element } = found
  // Every read Neo makes of a box it typed a host's secret into answers bullets: the preview,
  // the received value or aria tree, and the failure's snapshot (ruling R47). The mask is
  // synchronous: the check's promise is made inside it, and upstream's expectSingleElement reads
  // the element before its first await (the only await is `to.be.in.viewport`'s, which reads
  // geometry). So `toHaveValue` compares against bullets, as `inputValue()` reads.
  const resolved = withMarkedFieldsMasked(() =>
    world.injected.previewNode(element),
  )
  const check = await withMarkedFieldsMasked(() =>
    checks.single(element, params, request.frameVisible),
  )
  const reply = poll(check, resolved)
  // Upstream's InjectedScript.expect snapshots only a poll that fails the matcher, so a passing
  // poll costs no aria tree.
  if (check.matches === params.isNot) {
    const ariaSnapshot = withMarkedFieldsMasked(() =>
      expectFailureSnapshot(
        world.injected,
        element,
        params,
        request.frameVisible,
      ),
    )
    if (ariaSnapshot !== undefined) reply.ariaSnapshot = ariaSnapshot
  }
  return reply
}

function poll(check: ExpectCheck, resolved: string): ExpectPoll {
  return {
    kind: 'expect',
    matches: check.matches,
    resolved,
    received: check.received,
  }
}
