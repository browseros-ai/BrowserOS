/** The caller's explicit screenshot mask, resolved without waiting or strict matching. */
import type { ReplyTo, RequestOf } from './protocol'
import { queryAll } from './resolve'
import type { World } from './world'

export function screenshotMask(
  world: World,
  request: RequestOf<'screenshotMask'>,
  root: Node | undefined,
): ReplyTo<'screenshotMask'> {
  world.injected.addMaskedElements(
    queryAll(world, request.target, root),
    request.color,
  )
  return { kind: 'value', value: null }
}

/** Rust retains this world and its capture lease until the overlay is removed. */
export function screenshotMaskCleanup(
  world: World,
): ReplyTo<'screenshotMaskCleanup'> {
  world.injected.hideHighlight()
  return { kind: 'value', value: null }
}
