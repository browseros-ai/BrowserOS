/**
 * The page half of pointer actions (crates/neo-sdk/neo-playwright/src/action/pointer_action.rs).
 *
 *   act            resolve or reuse a pin, check states, scroll, measure; frame-local pixels
 *   act { arm }    re-measure; while the rect still equals Rust's baseline, hit-test and arm
 *                  Playwright's hit-target interceptor (Rust sends the trusted input next)
 *   moves          how many trusted pointer moves reached this frame. Rust counts before and
 *                  after its approach and presses only once the count grew, because the browser
 *                  routes input by layout it may not have yet; forced actions too
 *   verdict        stop that interceptor: did the trusted events reach the element? A miss
 *                  is the interceptor's own description of what the events hit
 *   disarm         stop whatever is armed here, for cleanup on every other exit
 *   frameBox       for a parent frame: where its iframe's content box sits, and whether the
 *                  iframe receives events at a point
 *
 * Nothing here waits. Stability is two `act`s at least 16 ms apart on Rust's clock, because a
 * background tab never runs requestAnimationFrame. An armed interceptor blocks the page's own
 * pointer events after a miss, so every exit disarms it (Rust's ArmGuard), the next arming
 * request replaces it, and it expires on its own if Rust never comes back: Rust sets the
 * expiry past the last event its op can still send, so an expiry never lets an event of the
 * action through.
 */
import type { FailureOutput } from './failure-output'
import type {
  ActRequest,
  DisarmRequest,
  FrameBoxRequest,
  Hit,
  MovesRequest,
  Point,
  Rect,
  ReplyTo,
  ScrollPlan,
  VerdictRequest,
} from './protocol'
import { lookFrameBox } from './private-frame'
import { resolutionOf, resolveTarget } from './resolve'
import { ProbeError, type World } from './world'

/** What `setupHitTargetInterceptor` hands back once armed. */
type Interception = { stop(): Hit }

/**
 * An interceptor's verdict. `expired` when its expiry stopped it, not a verdict: events after
 * that went unchecked.
 */
type Stopped = {
  token: number
  hit: Hit
  expired: boolean
}

type Slot = {
  armed?: {
    token: number
    interception: Interception
    expiry: ReturnType<typeof setTimeout>
  }
  /**
   * The last interceptor stopped by its verdict or its expiry, until the next arming, so a
   * verdict is answered for its token as often as Rust asks for it.
   */
  stopped?: Stopped
}

/**
 * One slot per booted world. A WeakMap instead of a World field, so a re-keyed world (a new
 * World object) starts empty and the dead one's slot is collected with it.
 */
const slots = new WeakMap<World, Slot>()

function slotOf(world: World): Slot {
  let slot = slots.get(world)
  if (!slot) {
    slot = {}
    slots.set(world, slot)
  }
  return slot
}

/** Trusted pointer moves this document received, for `moves`. */
let trustedMoves = 0

function countMove(event: Event): void {
  if (event.isTrusted) trustedMoves++
}

/**
 * Counts this document's trusted pointer moves. Boot calls it before it builds an engine, so
 * it runs ahead of the interceptor's own window listeners, which stop every move of a trial
 * hover. Adding it again is a no-op, so it can follow the engine's listeners back after a
 * document.open() of the page's own drops the window's listeners.
 */
export function listenForMoves(): void {
  for (const type of ['pointermove', 'mousemove'])
    window.addEventListener(type, countMove, { capture: true, passive: true })
}

/** Stops the armed interceptor, if any, and returns what it saw. */
function stopArmed(slot: Slot): Stopped | undefined {
  const armed = slot.armed
  if (!armed) return undefined
  slot.armed = undefined
  clearTimeout(armed.expiry)
  return {
    token: armed.token,
    hit: armed.interception.stop(),
    expired: false,
  }
}

export function act(
  world: World,
  request: ActRequest,
  root: Node | undefined,
  _nodes: readonly unknown[],
  output: FailureOutput,
): ReplyTo<'act'> {
  const found = resolveTarget(world, request.target, request.strictness, root)
  if (found.kind !== 'one') return found
  const { element } = found
  const { injected } = world
  for (const state of request.states) {
    let result: ReturnType<World['injected']['elementState']>
    try {
      result = injected.elementState(element, state, request.frameVisible)
    } catch (error) {
      // A state the element cannot have (`editable` on a <div>) throws; upstream has logged
      // what the locator resolved to by then, so the error reply carries it (index.ts).
      if (error instanceof Error)
        Object.assign(error, resolutionOf(world, found))
      throw error
    }
    if (result.received === 'error:notconnected') return { kind: 'detached' }
    if (!result.matches)
      return { kind: 'state', missing: state, ...resolutionOf(world, found) }
  }
  const pin =
    typeof request.target === 'object' && 'pin' in request.target
      ? request.target.pin
      : world.pins.add(element)
  const { arm } = request
  // An arming request runs under Rust's page input lock, so a stale interceptor found here is
  // never another op's live one: an earlier op of this world that lost its verdict.
  if (arm) stopArmed(slotOf(world))
  else scroll(element, request.scroll)
  // Upstream's stability rect: the retargeted element's bounding box (InjectedScript's
  // _checkElementIsStable).
  const rect = injected
    .retarget(element, 'no-follow-label')
    ?.getBoundingClientRect()
  if (!rect) return { kind: 'detached' }
  const box = rectOf(rect)
  const place = clickPoint(element, request.position)
  const resolved = resolutionOf(world, found)
  if (!place) return { kind: 'state', missing: 'visible', ...resolved }
  const measured = {
    kind: 'measured',
    pin,
    rect: box,
    point: place.point,
    inView: place.inView,
    transformed: isTransformed(element),
    ...(place.borderOffset ? { borderOffset: place.borderOffset } : {}),
    ...resolved,
  } as const
  if (!arm || (!arm.browserMapped && !place.inView)) return measured
  if (arm.expectRect && !sameRect(box, arm.expectRect)) return measured
  // A transformed frame can map a different clipped centroid, or scale a subpixel
  // local target into view. Rust checked that geometry; events establish its hit.
  const interception = output.arm(
    element,
    arm.action,
    arm.browserMapped ? undefined : place.point,
    arm.blockAll,
  )
  if (interception === 'error:notconnected') return { kind: 'detached' }
  if (typeof interception === 'string')
    return { kind: 'covered', by: interception }
  const slot = slotOf(world)
  slot.stopped = undefined
  slot.armed = {
    token: arm.token,
    interception,
    // Only a Rust that lost the page lets this fire: it bounds how long the page's own
    // clicks stay blocked after a lost connection.
    expiry: setTimeout(() => {
      const stopped = stopArmed(slot)
      if (stopped) slot.stopped = { ...stopped, expired: true }
    }, arm.expiryMs),
  }
  return { kind: 'armed', rect: box, point: place.point, token: arm.token }
}

export function verdict(
  world: World,
  request: VerdictRequest,
): ReplyTo<'verdict'> {
  const slot = slotOf(world)
  if (slot.armed?.token === request.token) slot.stopped = stopArmed(slot)
  const stopped = slot.stopped
  // Rust asks only for a token it armed in this world, and nothing else stops it here while
  // the op holds the page's input. Answering `done` would vouch for input Neo never checked.
  if (stopped?.token !== request.token)
    throw new ProbeError(
      'internal',
      `no interceptor is armed with token ${request.token}`,
    )
  if (stopped.expired)
    throw new ProbeError(
      'internal',
      `the interceptor armed with token ${request.token} expired before its verdict, so the input's later events went unchecked`,
    )
  return { kind: 'verdict', hit: stopped.hit }
}

/**
 * How many trusted pointer moves the browser delivered to this document. The browser picks the
 * frame for each event by its own copy of the layout, which can trail a scroll in another
 * process by a frame or more (pointer_action.rs, PointerRunner::route), so Rust presses only
 * once this grew past its count from before the approach. It reads no interceptor: a forced
 * action, which arms none, checks its route the same way.
 */
export function moves(_world: World, _request: MovesRequest): ReplyTo<'moves'> {
  return { kind: 'moves', moves: trustedMoves }
}

/** Idempotent: with nothing armed it answers `done`. */
export function disarm(
  world: World,
  _request: DisarmRequest,
): ReplyTo<'disarm'> {
  const slot = slotOf(world)
  const stopped = stopArmed(slot)
  slot.stopped = undefined
  return { kind: 'verdict', hit: stopped?.hit ?? 'done' }
}

/**
 * One parent frame of an action's element. `origin` is where the iframe's content box starts
 * in this frame's viewport (null when a transform makes it no single offset); with `point`, in
 * this frame's pixels, `hit` says whether the iframe receives events there (upstream's
 * `_checkFrameIsHitTarget`, one level). With `look`, the private look's rules instead
 * (`private-frame.ts`, ruling R60).
 */
export function frameBox(
  world: World,
  request: FrameBoxRequest,
  root: Node | undefined,
): ReplyTo<'frameBox'> {
  const found = resolveTarget(world, request.iframe, 'firstMatch', root)
  if (found.kind !== 'one') return found
  const iframe = found.element
  if (request.look) return lookFrameBox(world, iframe, request.point)
  const style = world.injected.describeIFrameStyle(iframe)
  if (style === 'error:notconnected') return { kind: 'detached' }
  const box = iframe.getBoundingClientRect()
  return {
    kind: 'frameBox',
    origin:
      style === 'transformed'
        ? null
        : { x: box.x + style.left, y: box.y + style.top },
    hit: request.point
      ? world.injected.expectHitTarget(request.point, iframe)
      : null,
    unseen: false,
  }
}

/**
 * Upstream's scroll rotation by attempt: the protocol's scroll-if-needed, which Rust sends as
 * `DOM.scrollIntoViewIfNeeded` before this request (protocol_scroll.rs) because it is instant
 * where the page's own `scrollIntoViewIfNeeded` follows `scroll-behavior: smooth`; then
 * bottom, center and top alignment, for sticky headers and footers. Every scroll is instant
 * and, like the protocol's, scrolls every ancestor frame, cross-site ones included.
 */
function scroll(element: Element, plan: ScrollPlan): void {
  switch (plan) {
    case 'none':
    case 'protocol':
      return
    case 'end':
    case 'center':
    case 'start':
      element.scrollIntoView({ block: plan, inline: plan, behavior: 'instant' })
  }
}

/**
 * Upstream's point for a pointer action, in this frame's pixels. Without `position`, the
 * middle of the element's first box that covers more than one pixel of the viewport
 * (`_clickablePoint`); `inView` is false when no box does. With `position`, the offset from
 * the border box's padding edge (`_offsetPoint`), which upstream never checks against the
 * viewport. Undefined when the element has no box at all: it is not visible.
 */
function clickPoint(
  element: Element,
  position: Point | undefined,
): { point: Point; inView: boolean; borderOffset?: Point } | undefined {
  const boxes = Array.from(element.getClientRects())
  const [first] = boxes
  if (!first) return undefined
  if (position) {
    const style = getComputedStyle(element)
    const box = element.getBoundingClientRect()
    const borderOffset = {
      x: parseInt(style.borderLeftWidth || '0', 10) + position.x,
      y: parseInt(style.borderTopWidth || '0', 10) + position.y,
    }
    return {
      point: { x: box.x + borderOffset.x, y: box.y + borderOffset.y },
      inView: true,
      borderOffset,
    }
  }
  const width = innerWidth
  const height = innerHeight
  for (const box of boxes) {
    // Clamp every corner as upstream does. Two inverted offscreen spans otherwise have
    // positive area and admit a point outside the viewport for trusted input.
    const left = Math.min(Math.max(box.left, 0), width)
    const right = Math.min(Math.max(box.right, 0), width)
    const top = Math.min(Math.max(box.top, 0), height)
    const bottom = Math.min(Math.max(box.bottom, 0), height)
    if ((right - left) * (bottom - top) > 0.99)
      return {
        point: { x: (left + right) / 2, y: (top + bottom) / 2 },
        inView: true,
      }
  }
  return {
    point: { x: first.x + first.width / 2, y: first.y + first.height / 2 },
    inView: false,
  }
}

/** A CSS transform on the element or any ancestor: its boxes are bounding boxes, not quads. */
function isTransformed(element: Element): boolean {
  for (let at: Element | null = element; at; at = at.parentElement) {
    if (getComputedStyle(at).transform !== 'none') return true
  }
  return false
}

function rectOf(rect: DOMRect): Rect {
  return { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
}

function sameRect(a: Rect, b: Rect): boolean {
  return (
    a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height
  )
}
