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
import {
  coverMark,
  labelPlainPoint,
  notOwnAt,
  ownPress,
  positionPoint,
  shownOwnLabel,
} from './cover'
import type { FailureOutput } from './failure-output'
import { lookFrameBox } from './private-frame'
import type {
  ActRequest,
  DisarmRequest,
  FrameBoxRequest,
  Hit,
  MovesRequest,
  OwnPress,
  Point,
  Rect,
  ReplyTo,
  ScrollPlan,
  VerdictRequest,
} from './protocol'
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
    /** Neo's own check at delivery, for a press of the target's own (`guardPress`). */
    guard?: PressGuard
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

/** The events of a press the vendored interceptor checks for a mouse action, and so does Neo's guard. */
const PRESS_EVENTS = [
  'pointerdown',
  'mousedown',
  'pointerup',
  'mouseup',
  'click',
  'auxclick',
  'dblclick',
  'contextmenu',
] as const

/** What `guardPress` refused, if anything, and how its listeners go. */
type PressGuard = { refused(): string | undefined; stop(): void }

/**
 * Neo's own check at delivery of a press of the target's own (covered-click C). The vendored
 * interceptor swallows a press that lands outside the armed element, but accepts anything inside
 * it: a link nested in the label, a quick-add button a card shows under the pointer on hover after
 * Neo chose the point. At the press's first trusted event this asks the one predicate of what is on
 * top at the event's point (`notOwnAt`); a no swallows every event of the press, and the verdict
 * reports a miss with that element's preview, which costs a retry. Its listeners are added at
 * arming, so they run after the vendored dispatcher's (a press it swallowed never reaches them)
 * and after any the page added to the window before; `stopArmed` removes them on every path.
 */
function guardPress(owner: Element, output: FailureOutput): PressGuard {
  let asked = false
  let refused: string | undefined
  const listener = (event: Event) => {
    if (!event.isTrusted) return
    if (!asked) {
      asked = true
      const off =
        event instanceof MouseEvent
          ? notOwnAt(owner, { x: event.clientX, y: event.clientY })
          : undefined
      if (off) refused = output.previewNode(off)
    }
    if (refused === undefined) return
    event.preventDefault()
    event.stopImmediatePropagation()
  }
  for (const type of PRESS_EVENTS)
    window.addEventListener(type, listener, { capture: true })
  return {
    refused: () => refused,
    stop: () => {
      for (const type of PRESS_EVENTS)
        window.removeEventListener(type, listener, { capture: true })
    },
  }
}

/**
 * Stops the armed interceptor, if any, and returns what it saw. A press of the target's own that
 * its guard refused is a miss, whatever the vendored interceptor saw.
 */
function stopArmed(slot: Slot): Stopped | undefined {
  const armed = slot.armed
  if (!armed) return undefined
  slot.armed = undefined
  clearTimeout(armed.expiry)
  const seen = armed.interception.stop()
  armed.guard?.stop()
  const refused = armed.guard?.refused()
  return {
    token: armed.token,
    hit:
      refused !== undefined && seen === 'done'
        ? { hitTargetDescription: refused }
        : seen,
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
  const checked = checkStates(world, request, found, output)
  if ('kind' in checked) return checked
  const { subject, own } = checked
  const pin =
    typeof request.target === 'object' && 'pin' in request.target
      ? request.target.pin
      : world.pins.add(element)
  const { arm } = request
  // An arming request runs under Rust's page input lock, so a stale interceptor found here is
  // never another op's live one: an earlier op of this world that lost its verdict.
  if (arm) stopArmed(slotOf(world))
  else scroll(subject, request.scroll)
  // Upstream's stability rect: the retargeted element's bounding box (InjectedScript's
  // _checkElementIsStable), or the label's that takes the press.
  const rect = (
    own ? subject : injected.retarget(element, 'no-follow-label')
  )?.getBoundingClientRect()
  if (!rect) return { kind: 'detached' }
  const box = rectOf(rect)
  const place = own
    ? labelPlace(subject)
    : clickPoint(element, request.position)
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
    subject,
    arm.action,
    arm.browserMapped ? undefined : place.point,
    arm.blockAll,
  )
  if (interception === 'error:notconnected') return { kind: 'detached' }
  if (typeof interception !== 'string')
    return armSlot(
      world,
      arm,
      interception,
      box,
      place.point,
      own,
      own ? guardPress(subject, output) : undefined,
    )
  if (own) return coveredReply(world, element, place.point, interception)
  return armOwn(world, request, output, element, place.point, box, interception)
}

/**
 * The request's states on the element. A checkbox or radio that is not shown, whose own label
 * is, is the label's to press (covered-click C, a label's state two): the waits after the states
 * and the press then go to that label. Otherwise the first state that does not hold answers.
 */
function checkStates(
  world: World,
  request: ActRequest,
  found: Extract<ReturnType<typeof resolveTarget>, { kind: 'one' }>,
  output: FailureOutput,
): { subject: Element; own?: OwnPress } | ReplyTo<'act'> {
  const { element } = found
  let label: Element | undefined
  for (const state of request.states) {
    let result: ReturnType<World['injected']['elementState']>
    try {
      result = world.injected.elementState(element, state, request.frameVisible)
    } catch (error) {
      // A state the element cannot have (`editable` on a <div>) throws; upstream has logged
      // what the locator resolved to by then, so the error reply carries it (index.ts).
      if (error instanceof Error)
        Object.assign(error, resolutionOf(world, found))
      throw error
    }
    if (result.received === 'error:notconnected') return { kind: 'detached' }
    if (result.matches) continue
    label =
      state === 'visible' && request.own
        ? shownOwnLabel(world, element)
        : undefined
    if (!label)
      return { kind: 'state', missing: state, ...resolutionOf(world, found) }
  }
  return label
    ? {
        subject: label,
        own: { case: 'label', markup: output.previewNode(label) },
      }
    : { subject: element }
}

/** Keeps `taken` armed for the verdict, and answers where the input must go. */
function armSlot(
  world: World,
  arm: NonNullable<ActRequest['arm']>,
  taken: Interception,
  box: Rect,
  point: Point,
  press: OwnPress | undefined,
  guard?: PressGuard,
): ReplyTo<'act'> {
  const slot = slotOf(world)
  slot.stopped = undefined
  slot.armed = {
    token: arm.token,
    interception: taken,
    ...(guard ? { guard } : {}),
    // Only a Rust that lost the page lets this fire: it bounds how long the page's own
    // clicks stay blocked after a lost connection.
    expiry: setTimeout(() => {
      const stopped = stopArmed(slot)
      if (stopped) slot.stopped = { ...stopped, expired: true }
    }, arm.expiryMs),
  }
  return {
    kind: 'armed',
    rect: box,
    point,
    token: arm.token,
    ...(press ? { own: press } : {}),
  }
}

/**
 * Covered-click C: the same interceptor, armed on what is the target's own at that case's point
 * (`ownPress`), so nothing is pressed through anything foreign. A transformed frame maps its own
 * point, so it gets the covered answer. The interceptor tests the element it retargets to, so
 * what of the target's own may take the press, and the cover's mark, go by it too.
 */
function armOwn(
  world: World,
  request: ActRequest,
  output: FailureOutput,
  element: Element,
  point: Point,
  box: Rect,
  interception: string,
): ReplyTo<'act'> {
  const { arm } = request
  const retargeted = world.injected.retarget(element, 'button-link') ?? element
  const choice =
    arm && request.own && !arm.browserMapped
      ? ownPress(world, output, retargeted, point, request.own)
      : undefined
  const at =
    choice &&
    (choice.point ?? clickPoint(choice.element, choice.press.position)?.point)
  if (arm && choice && at) {
    const again = output.arm(choice.element, arm.action, at, arm.blockAll)
    if (again === 'error:notconnected') return { kind: 'detached' }
    if (typeof again !== 'string')
      return armSlot(
        world,
        arm,
        again,
        box,
        at,
        choice.press,
        guardPress(choice.element, output),
      )
  }
  return coveredReply(world, element, point, interception)
}

/** The covered answer, with which cover this is for the runner's count. */
function coveredReply(
  world: World,
  element: Element,
  point: Point,
  interception: string,
): ReplyTo<'act'> {
  const retargeted = world.injected.retarget(element, 'button-link') ?? element
  const mark = coverMark(retargeted, point)
  return {
    kind: 'covered',
    by: interception,
    ...(mark ? { cover: mark } : {}),
  }
}

/** Where a press on a label that takes it goes: its first plain point, as a click point. */
function labelPlace(
  label: Element,
): { point: Point; inView: boolean; borderOffset?: Point } | undefined {
  const point = labelPlainPoint(label)
  if (!point) return undefined
  const inView =
    point.x >= 0 &&
    point.y >= 0 &&
    point.x < innerWidth &&
    point.y < innerHeight
  return { point, inView }
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
  const hit = request.point
    ? world.injected.expectHitTarget(request.point, iframe)
    : null
  // A click's miss names its cover, for the runner's count, as the element's own frame does.
  const mark =
    request.point && hit !== null && hit !== 'done'
      ? coverMark(iframe, request.point)
      : undefined
  return {
    kind: 'frameBox',
    origin:
      style === 'transformed'
        ? null
        : { x: box.x + style.left, y: box.y + style.top },
    hit,
    unseen: false,
    ...(mark ? { cover: mark } : {}),
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
    const { point, borderOffset } = positionPoint(element, position)
    return { point, inView: true, borderOffset }
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
