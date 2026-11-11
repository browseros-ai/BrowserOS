/**
 * `expect`: one poll of one retrying matcher, matched by Playwright's own per-element and
 * per-array checks (world.ts `expectChecks`). Rust's loop owns the timing, `.not`, the failure
 * and its message; a poll only reports what it saw, and never waits.
 */
import type { FrameExpectParams } from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'
import type { ExpectPoll, ExpectRequest, ReplyTo } from './protocol'
import { queryAll, resolveTarget } from './resolve'
import { type ExpectCheck, expectChecks, type World } from './world'

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
    return poll(checks.every(elements, params), count)
  }
  const found = resolveTarget(world, request.target, request.strictness, root)
  if (found.kind !== 'one') return found
  const { element } = found
  const resolved = world.injected.previewNode(element)
  const check = await checks.single(element, params, request.frameVisible)
  return poll(check, resolved)
}

function poll(check: ExpectCheck, resolved: string): ExpectPoll {
  return {
    kind: 'expect',
    matches: check.matches,
    resolved,
    received: check.received,
  }
}
