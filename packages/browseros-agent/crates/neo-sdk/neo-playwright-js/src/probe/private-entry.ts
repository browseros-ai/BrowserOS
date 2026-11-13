/**
 * The private entry's steps on a pinned box (protected-fill design sections 5.4 to 5.6), after
 * the look (private-look.ts) passed and Rust marked the entry sent. Rust types; this side
 * focuses, checks where the focus is, and compares what the box holds. Rust gets a state or a
 * yes or a no, never the box's value.
 *
 *   focus     marks the box as privately typed, listens to its input events, focuses it and
 *             selects what it holds (as fill does), then one microtask later answers whether
 *             the box holds its tree's focus and its document has the focus
 *   focused   the same check alone: before each key, and after Rust's Shift press; `moving`
 *             once a form submitted or the document began to unload since the focus step
 *   holds     whether the box holds exactly `text` now: all but the last key, seen
 *   readBack  whether the box holds the value, holds something else (then it marks the
 *             control holding the focus, as markFocused), or left its document, whether it held
 *             the whole value at its last input event, and whether the page is moving on; then
 *             forgets the value
 *   forget    drops the value and the listener: every exit of the entry ends with it
 *   markFocused  marks the text control that holds the focus in the box's document, when it is
 *             not the box: an entry stopped at focus_lost or value_not_taken may have typed
 *             into it, because the focus can move between Neo's check and the key (ruling R26).
 *             A focus that went to another document is not followed.
 *
 * The value the box should hold stays in this world from `focus` to `readBack` or `forget`,
 * beside the box, so the listener can compare at each input event. The box under entry is held
 * by its pin in a table of its own, which nothing evicts: the pin table drops old pins past its
 * bound, and an entry must always reach its box to read it back and to forget the value. Nothing
 * here writes to the page's markup (ruling R22): the mark and the listener live in Neo's isolated
 * world.
 */
import type {
  BoxState,
  FocusState,
  Pin,
  PrivateAnswer,
  PrivateRequest,
  PrivateStep,
  ReplyTo,
} from './protocol'
import type { World } from './world'

/** Boxes Neo typed a private value into, in this document (design section 10.4). */
const marked = new WeakSet<Element>()

/**
 * Whether this world ever marked a box. Until it has, a snapshot under the spec harness's
 * conformance switch installs no mask at all (snapshot.ts); once it has, that snapshot masks the
 * marked boxes, which no switch bypasses (ruling R44).
 */
let anyMarked = false

/**
 * A box under entry: the value it should hold, whether its last input event saw it, and what
 * says its page may be moving on since the focus step: the submit events seen (judged once
 * their dispatch is over, when a page's handler may have prevented them) and whether the
 * document began to unload.
 */
type Watched = {
  readonly text: string
  readonly spaced: boolean
  heldAtLastInput: boolean
  readonly submits: Event[]
  leaving: boolean
  readonly listener: (event: Event) => void
  readonly onSubmit: (event: Event) => void
  readonly onLeave: (event: Event) => void
}

/** The events that say the box's document is leaving. */
const LEAVES = ['beforeunload', 'pagehide'] as const

const watched = new WeakMap<Element, Watched>()

/** The boxes under entry, by pin: kept until `readBack` or `forget`, never evicted. */
const active = new Map<Pin, Element>()

/** Whether Neo typed a private value into `element` (the mask reads it, pull request 4). */
export function privatelyTyped(element: Element): boolean {
  return marked.has(element)
}

/** Whether this world marked any box yet (`anyMarked`). */
export function anyPrivatelyTyped(): boolean {
  return anyMarked
}

function mark(element: Element): void {
  marked.add(element)
  anyMarked = true
}

export function privateStep(
  world: World,
  request: PrivateRequest,
): ReplyTo<'private'> | Promise<ReplyTo<'private'>> {
  const { step, pin } = request
  // After `focus`, the box is reached through `active`, connected or not: a read-back must
  // reach a box that left its document, whose listener still knows what it held at its last
  // input. Before it, the box must be in its document.
  const box =
    active.get(pin) ??
    (step.kind === 'focus' ? world.pins.live(pin) : undefined)
  if (!box) return { kind: 'detached' }
  const answered = answer(world, pin, box, step)
  return answered instanceof Promise ? answered.then(reply) : reply(answered)
}

function reply(value: PrivateAnswer): ReplyTo<'private'> {
  return { kind: 'value', value }
}

function answer(
  world: World,
  pin: Pin,
  box: Element,
  step: PrivateStep,
): PrivateAnswer | Promise<PrivateAnswer> {
  switch (step.kind) {
    case 'focus':
      return focus(world, pin, box, step.text, step.spaced)
    case 'focused':
      return { kind: 'focus', focus: focusState(box) }
    case 'holds':
      return {
        kind: 'holds',
        holds: same(heldText(box), step.text, step.spaced),
      }
    case 'readBack':
      return readBack(pin, box)
    case 'forget':
      forget(pin, box)
      return { kind: 'forgot' }
    case 'markFocused':
      return { kind: 'markedFocused', marked: markFocused(box) }
  }
}

/** Marks the text control holding the focus in `box`'s document, down open shadow roots. */
function markFocused(box: Element): boolean {
  let holder = box.ownerDocument.activeElement
  while (holder?.shadowRoot?.activeElement)
    holder = holder.shadowRoot.activeElement
  if (!holder || holder === box) return false
  const name = holder.localName
  if (name !== 'input' && name !== 'textarea') return false
  mark(holder)
  return true
}

async function focus(
  world: World,
  pin: Pin,
  box: Element,
  text: string,
  spaced: boolean,
): Promise<PrivateAnswer> {
  // The mark comes before the first character, so a snapshot taken mid-entry is masked too.
  mark(box)
  listen(pin, box, text, spaced)
  if (world.injected.selectText(box) === 'error:notconnected')
    return { kind: 'focus', focus: 'gone' }
  // One microtask turn, so a swap a framework schedules from the focus event as a microtask
  // is seen (edit.ts keptFocus).
  await Promise.resolve()
  return { kind: 'focus', focus: focusState(box) }
}

/**
 * Playwright's own focused rule: the box is in its document, holds its tree's focus, and its
 * document has the focus. In a cross-site frame `focus()` sets the frame's active element
 * before the site's focus handlers run; they run when input arrives, so Rust presses Shift
 * once when only the document's focus is missing.
 */
function focusState(box: Element): FocusState {
  if (!box.isConnected) return 'gone'
  if (moving(watched.get(box))) return 'moving'
  if (!holdsFocus(box)) return 'elsewhere'
  return box.ownerDocument.hasFocus() ? 'held' : 'noDocumentFocus'
}

/** The box itself holds its tree's focus, down open shadow roots. */
function holdsFocus(box: Element): boolean {
  const root = box.getRootNode()
  const active: unknown = Reflect.get(root, 'activeElement')
  return active === box
}

/**
 * Whether the box's page is moving on: its document began to unload, or a form submitted and no
 * handler prevented it (a page that validates in its submit handler stays). Read after the
 * submit's dispatch is over, in a later call, so `defaultPrevented` is final. A form that
 * submits into another tab moves nothing here.
 */
function moving(entry: Watched | undefined): boolean {
  if (!entry) return false
  return (
    entry.leaving ||
    entry.submits.some(
      (event) =>
        !event.defaultPrevented &&
        Reflect.get(event.target ?? {}, 'target') !== '_blank',
    )
  )
}

function listen(pin: Pin, box: Element, text: string, spaced: boolean): void {
  forget(pin, box)
  const entry: Watched = {
    text,
    spaced,
    heldAtLastInput: false,
    submits: [],
    leaving: false,
    listener: () => {
      entry.heldAtLastInput = same(heldText(box), text, spaced)
    },
    onSubmit: (event) => {
      entry.submits.push(event)
    },
    onLeave: () => {
      entry.leaving = true
    },
  }
  watched.set(box, entry)
  active.set(pin, box)
  // Capture, so a page that stops the event before it bubbles still lets Neo see it.
  box.addEventListener('input', entry.listener, { capture: true })
  const view = box.ownerDocument.defaultView
  view?.addEventListener('submit', entry.onSubmit, { capture: true })
  for (const type of LEAVES)
    view?.addEventListener(type, entry.onLeave, { capture: true })
}

function forget(pin: Pin, box: Element): void {
  active.delete(pin)
  const entry = watched.get(box)
  if (!entry) return
  box.removeEventListener('input', entry.listener, { capture: true })
  const view = box.ownerDocument.defaultView
  view?.removeEventListener('submit', entry.onSubmit, { capture: true })
  for (const type of LEAVES)
    view?.removeEventListener(type, entry.onLeave, { capture: true })
  watched.delete(box)
}

/**
 * The read-back: what the box holds, whether it held the whole value at its last input event,
 * and whether its page is moving on (a submit not prevented, or the document unloading): a box
 * that still holds its value on a page that already submitted went out with the form, and Rust
 * answers it as moved, never as entered (ruling R38).
 */
function readBack(pin: Pin, box: Element): PrivateAnswer {
  const entry = watched.get(box)
  const heldAtLastInput = entry?.heldAtLastInput ?? false
  let state: BoxState
  if (!box.isConnected) state = 'left'
  else if (entry && same(heldText(box), entry.text, entry.spaced))
    state = 'holds'
  else state = 'other'
  // The text may have gone to the control that holds the focus now: it is marked before the
  // box is forgotten, which ends this entry's reach to the box (ruling R26).
  if (state === 'other') markFocused(box)
  const leaving = moving(entry)
  forget(pin, box)
  return { kind: 'readBack', state, heldAtLastInput, moving: leaving }
}

function heldText(box: Element): string {
  const value: unknown = Reflect.get(box, 'value')
  return typeof value === 'string' ? value : ''
}

/** Equal, or with `spaced` equal once spaces, dashes and slashes are set aside (`12 / 29`). */
function same(held: string, text: string, spaced: boolean): boolean {
  if (held === text) return true
  if (!spaced) return false
  const bare = (value: string) => value.replace(/[\s\-/]/g, '')
  return bare(held) === bare(text)
}
