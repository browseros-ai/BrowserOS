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
 *
 * The mark is kept on the box itself (ruling R53): a non-enumerable property on the box's
 * wrapper in this isolated world, under a registered symbol. A frame's copy of this probe is a
 * module of its own, so a set held here would not follow a box the page moves into another
 * same-origin frame; the wrapper does, and the other frame's probe reads the same property
 * there. The page's own world holds a wrapper of its own and never sees it. A named limit: under
 * the spec harness's conformance switch a world masks only once it marked a box itself
 * (`anyPrivatelyTyped`), so a marked box moved into another frame's document is not masked in
 * that frame's tree walk there; no host fill happens under the switch.
 */

import { coverWrapper, renderedText } from './private-look'
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

/**
 * The key of the mark on a box Neo typed a private value into (design section 10.4). Registered
 * (`Symbol.for`), so every frame's probe in this renderer names the same key: a symbol made with
 * `Symbol()` would be one per frame's module and miss a box marked in another frame.
 */
const MARK = Symbol.for('neo.privatelyTyped')

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
  /**
   * For a box its look let through under its field's own drawing (rulings R55, R56): the
   * drawing's wrapper, whether its text already held the value before the first character, and
   * the value it is compared with.
   */
  readonly cover?: {
    readonly wrapper: Element
    readonly drewBefore: boolean
    readonly text: string
  }
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

/**
 * The wrapper whose text drew the value at a box's read-back (ruling R55), by pin, until the
 * `drawn` step asks whether it still does after Rust emptied the box. Kept apart from the box's
 * entry, which the read-back ends and the emptying's focus step starts again.
 */
const drawnWrappers = new Map<Pin, Element>()

/** Whether Neo typed a private value into `element` (the mask reads it, pull request 4). */
export function privatelyTyped(element: Element): boolean {
  return Object.hasOwn(element, MARK)
}

/** Whether this world marked any box yet (`anyMarked`). */
export function anyPrivatelyTyped(): boolean {
  return anyMarked
}

function mark(element: Element): void {
  // Neither enumerable, writable nor configurable: no walk lists it, and nothing unmarks a box.
  if (!Object.hasOwn(element, MARK))
    Object.defineProperty(element, MARK, { value: true })
  anyMarked = true
}

export function privateStep(
  world: World,
  request: PrivateRequest,
): ReplyTo<'private'> | Promise<ReplyTo<'private'>> {
  const { step, pin } = request
  // Asked after the emptying, whatever became of the box: only the wrapper is read. A pin with
  // no wrapper kept answers that it still draws, so Neo never says the page was cleared when it
  // cannot tell.
  if (step.kind === 'drawn') {
    const wrapper = drawnWrappers.get(pin)
    drawnWrappers.delete(pin)
    return reply({
      kind: 'drawn',
      drawn: wrapper ? draws(wrapper, step.text) : true,
    })
  }
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
  step: Exclude<PrivateStep, { kind: 'drawn' }>,
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
  const wrapper = coverWrapper(box)
  const entry: Watched = {
    text,
    spaced,
    ...(wrapper && {
      cover: {
        wrapper,
        drewBefore: draws(wrapper, text),
        text,
      },
    }),
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
 * answers it as moved, never as entered (ruling R38). Also how the box's field drawing figured
 * (`drawing`, rulings R55 and R56): `none` without one, `under` for a box entered under it, and
 * `drawn` when its wrapper now draws the value outside the box, where it did not before the first
 * character. Rust empties the box of a long drawn value and keeps a short one, a code's digits.
 * The value is compared here and nothing of it leaves this world.
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
  const cover = entry?.cover
  const drawn =
    cover !== undefined && !cover.drewBefore && draws(cover.wrapper, cover.text)
  if (cover && drawn) drawnWrappers.set(pin, cover.wrapper)
  forget(pin, box)
  const drawing = !cover ? 'none' : drawn ? 'drawn' : 'under'
  return { kind: 'readBack', state, heldAtLastInput, moving: leaving, drawing }
}

/**
 * Whether `wrapper` draws `text` as text outside its box: its rendered text, which holds no
 * box's value, contains it once whitespace, dashes and slashes are set aside on both sides and
 * both are lowered, whatever the field's `spaced` says: a site draws a code one digit to a cell,
 * which its text separates by line breaks, a card number in its own groups (`4242 4242 …`), and
 * a value through `text-transform`, which the rendered text gives as shown while the page's text
 * holds it as typed. A false alarm would need the drawing to gain the same characters in order
 * after the typing, which is the value. Text it held before the first character does not count
 * (`drewBefore`). `spaced` stays the box's own read-back's (`same`).
 */
function draws(wrapper: Element, text: string): boolean {
  const bare = (value: string) => value.replace(/[\s\-/]/g, '').toLowerCase()
  const value = bare(text)
  return value !== '' && bare(renderedText(wrapper)).includes(value)
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
