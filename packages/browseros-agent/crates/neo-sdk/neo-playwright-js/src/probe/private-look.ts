/**
 * The private entry's look at one box (protected-fill design section 5.3): before Neo types a
 * host's secret, every box is checked here, in Neo's isolated world of the box's own frame,
 * and Rust gets the box's facts or the first check it fails. The look reads no value: only
 * whether the box is empty. Rust has already checked that the ref names one element (step 1)
 * and reads the tab's address itself (the rest of step 3).
 *
 *   2 an <input> whose type takes text       notAnInput
 *   3 its frame on the host's origin         wrongOrigin
 *   4 every fact the host approved holds     wrongType, or boxChanged with the fact
 *   5 enabled and not read-only              boxNotEditable
 *   6 the box shows, and nothing else takes  boxHidden, boxCovered
 *     the pointer at its centre              (main's covered rule, below); a box with no
 *                                            size is hidden unless typed key by key; the
 *                                            field's own drawing over its box does not
 *                                            cover it (rulings R55, R56, below)
 *   7 the value fits its maxlength, or the   valueTooLong
 *     host allowed a cut
 *
 * Neo does not wait here: a box that is hidden, covered or read-only now is refused now. The
 * host approved a box in one state, so a different state is a refusal, never a wait.
 */
import type {
  BoxAsk,
  BoxExpect,
  BoxFact,
  BoxFacts,
  Looked,
  LookRefused,
  LookRequest,
  ReplyTo,
} from './protocol'
import type { World } from './world'

/** The input types that take typed text; the DOM reports `text` for a missing or unknown one. */
const TEXT_TYPES = new Set([
  'text',
  'password',
  'tel',
  'email',
  'search',
  'url',
  'number',
])

/**
 * The types the browser applies `maxlength` to. A `number` box ignores it, so its code is never
 * refused as too long; its facts still report `maxLength` as the DOM gives it.
 */
const MAXLENGTH_TYPES = new Set([
  'text',
  'search',
  'url',
  'tel',
  'email',
  'password',
])

export function look(world: World, request: LookRequest): ReplyTo<'look'> {
  const element = world.pins.live(request.pin)
  if (!element) return { kind: 'detached' }
  return { kind: 'value', value: lookAt(world, element, request.look) }
}

/**
 * Steps 2 to 7 on one element, in order. The answer carries the box's facts whenever it is a
 * text input, and the first check it fails: a covered box answers both. Without `expect` (a
 * check call before the host approved anything) no origin and no fact is checked.
 */
export function lookAt(world: World, element: Element, ask: BoxAsk): Looked {
  // Every look starts without the wrapper an earlier one kept: a box with no size is reached by
  // focus and never hit-tested, so a wrapper from a look when it had a size must not carry over.
  coverWrappers.delete(element)
  if (!isTextInput(element)) return { refusal: { code: 'notAnInput' } }
  const facts = factsOf(element)
  const refusal = firstRefusal(world, element, facts, ask)
  return refusal ? { facts, refusal } : { facts }
}

function firstRefusal(
  world: World,
  element: HTMLInputElement,
  facts: BoxFacts,
  ask: BoxAsk,
): LookRefused | undefined {
  const { expect } = ask
  if (expect && facts.origin !== expect.origin) return { code: 'wrongOrigin' }
  if (expect?.type !== undefined && expect.type !== facts.type)
    return { code: 'wrongType' }
  const fact = expect && changedFact(facts, expect)
  if (fact) return { code: 'boxChanged', fact }
  if (element.disabled || element.readOnly || element.matches(':disabled'))
    return { code: 'boxNotEditable' }
  // One insertion needs a box that shows, as fill does. A field typed key by key may have no
  // size (a code field under a site's own drawing): it has nothing to show or to cover, and
  // is reached by focus alone.
  if (!facts.sized) {
    if (!ask.keys) return { code: 'boxHidden' }
  } else if (
    !world.injected.elementState(element, 'visible', ask.frameVisible).matches
  )
    return { code: 'boxHidden' }
  else if (blocked(world, element, ask.keys)) return { code: 'boxCovered' }
  if (
    ask.length !== undefined &&
    !ask.cut &&
    facts.maxLength !== null &&
    MAXLENGTH_TYPES.has(facts.type) &&
    ask.length > facts.maxLength
  )
    return { code: 'valueTooLong' }
  return undefined
}

/** The box's facts: what the host approves and sends back as `expect`. */
export function factsOf(input: HTMLInputElement): BoxFacts {
  const rect = input.getBoundingClientRect()
  return {
    origin: originOf(input.ownerDocument),
    type: input.type,
    autocomplete: input.getAttribute('autocomplete') ?? '',
    // The DOM's -1 is "no maxlength".
    maxLength: input.maxLength >= 0 ? input.maxLength : null,
    inputMode: input.getAttribute('inputmode') ?? '',
    name: input.getAttribute('name') ?? '',
    id: input.getAttribute('id') ?? '',
    sized: rect.width > 0 && rect.height > 0,
    empty: input.value === '',
  }
}

/**
 * The origin of the box's own document. A frame's `window.origin` is the document's origin
 * (an `about:blank` or `srcdoc` frame inherits its parent's), where `location.origin` would
 * read `null` for such a frame.
 */
function originOf(document: Document): string {
  const origin: unknown = document.defaultView
    ? Reflect.get(document.defaultView, 'origin')
    : undefined
  return typeof origin === 'string' ? origin : document.location.origin
}

function changedFact(facts: BoxFacts, expect: BoxExpect): BoxFact | undefined {
  if (
    expect.autocomplete !== undefined &&
    expect.autocomplete !== facts.autocomplete
  )
    return 'autocomplete'
  if (expect.maxLength !== undefined && expect.maxLength !== facts.maxLength)
    return 'maxLength'
  if (expect.inputMode !== undefined && expect.inputMode !== facts.inputMode)
    return 'inputMode'
  if (expect.name !== undefined && expect.name !== facts.name) return 'name'
  if (expect.id !== undefined && expect.id !== facts.id) return 'id'
  return undefined
}

function isTextInput(element: Element): element is HTMLInputElement {
  return (
    element.localName === 'input' &&
    element.namespaceURI === 'http://www.w3.org/1999/xhtml' &&
    TEXT_TYPES.has(String(Reflect.get(element, 'type')))
  )
}

/**
 * Main's covered rule, ported as it is: `HIT_TEST_BLOCKER_JS` in
 * crates/browseros-core/src/input/geometry.rs, which main's fill ran before typing
 * (input/fill.rs). tests/private_look_live.rs reads that text out of browseros-core and runs
 * both on the same pages, so the two cannot drift apart without a red test.
 *
 * Main scrolled the box into view, took its centre in its own target's coordinates, and asked
 * which element is there from the topmost document the box's frame can reach (`frameElement`
 * is null across origins), following the hit into same-origin frames. Not covered when that
 * element is the box or inside it, an ancestor of it (through shadow hosts), inside a <label>
 * whose control is the box or that holds the box, or inside the box's own wrapping label.
 * Anything else covers it.
 *
 * Kept as main, with its three blind spots (design section 5.3): an ancestor drawn over the box
 * counts as the box; a page's overlay over a cross-site frame is not seen; a cover inside the
 * box's own shadow root reads as its shadow host, an ancestor. Where the port differs:
 *
 * - A hit test that throws counts as covered, where main typed on.
 * - The point is the centre of the box's border box (`getBoundingClientRect`), where main took
 *   the centre of its first content quad (`DOM.getContentQuads`). They differ for a box with
 *   unequal borders or padding, and for a transformed box.
 * - For a frame on another origin of the same site, main's point was in the page's coordinates
 *   (the frame shares the page's target) while its hit test ran in the frame's own document,
 *   where `frameElement` stops the climb. Here the point stays in the frame's own coordinates
 *   for that document, so the hit lands on the box's centre.
 */
function blockerOf(box: Element): Element | null | 'unread' {
  try {
    scrollIntoViewIfNeeded(box)
    const rect = box.getBoundingClientRect()
    let document = box.ownerDocument
    let x = rect.left + rect.width / 2
    let y = rect.top + rect.height / 2
    // Main's point came from the protocol in the box's own target's coordinates: the page's
    // for a same-origin frame, the frame's own for a cross-site one. Climbing same-origin
    // frames to the topmost reachable document gives the same point.
    for (
      let frame = document.defaultView?.frameElement;
      frame;
      frame = document.defaultView?.frameElement
    ) {
      const frameRect = frame.getBoundingClientRect()
      x += frameRect.x + frame.clientLeft
      y += frameRect.y + frame.clientTop
      document = frame.ownerDocument
    }
    return blockerAt(document, box, x, y)
  } catch {
    return 'unread'
  }
}

/**
 * The box's wrapper, by box, when its last look let it through under its field's own drawing
 * (rulings R55, R56): the private entry reads the wrapper's text before the first character and
 * at the read-back, to see whether the site draws the typed value outside the box
 * (private-entry.ts). Each look sets or clears it, so a later look with no drawing left forgets
 * it.
 */
const coverWrappers = new WeakMap<Element, Element>()

/** The wrapper of the drawing the box's last look let it through under, if it did. */
export function coverWrapper(box: Element): Element | undefined {
  return coverWrappers.get(box)
}

/**
 * Whether the box is covered: main's rule, except that the field's own drawing over its box does
 * not cover it ([`drawingOf`], rulings R55 and R56). tests/private_look_live.rs holds the look
 * against main's own script on every page, and lists the pages where the two differ.
 */
function blocked(world: World, box: Element, keys: boolean): boolean {
  const blocker = blockerOf(box)
  if (blocker === null) return false
  if (blocker === 'unread') return true
  const wrapper = drawingOf(world, box, blocker, keys)
  if (!wrapper) return true
  coverWrappers.set(box, wrapper)
  return false
}

/** An element that can take input or the focus, in a cover or as the cover itself. */
const CONTROLS =
  'input, textarea, select, button, a[href], iframe, frame, [contenteditable], [tabindex]'

/**
 * Rulings R55 and R56: the field's own drawing over its box, which is no `<label>`, such as a
 * floating label or a code field's drawn digits, so main's rule calls it a cover. It does not
 * cover when every point holds, and then the answer is its wrapper; otherwise `null`.
 *
 * 1. The blocker is in the box's own document.
 * 2. It is inside the box's parent, or that parent's parent: the wrapper.
 * 3. The cover, the largest element under the wrapper that holds the blocker and not the box,
 *    holds no control and is none; a `<label>` for another control counts as one (`labelsAnother`).
 * 4. The cover lies within the box's rectangle grown by half its width on each side and half its
 *    height above and below.
 * 5. The cover draws the box's own name: its rendered text and the box's accessible name, as the
 *    snapshot engine computes it, are the same once folded (`sameName`); the box is typed either
 *    way. Or it draws nothing (its rendered text is empty once folded), as a code field's drawn
 *    digits do before any is typed; then only key by key (`keys`), as a host types a code.
 *    Other words cover the box.
 */
function drawingOf(
  world: World,
  box: Element,
  blocker: Element,
  keys: boolean,
): Element | null {
  if (blocker.ownerDocument !== box.ownerDocument) return null
  const parent = box.parentElement
  const grand = parent?.parentElement
  const wrapper = parent?.contains(blocker)
    ? parent
    : grand?.contains(blocker)
      ? grand
      : null
  if (!wrapper) return null
  let cover = blocker
  while (
    cover.parentElement &&
    cover.parentElement !== wrapper &&
    !cover.parentElement.contains(box)
  )
    cover = cover.parentElement
  if (
    cover.contains(box) ||
    cover.matches(CONTROLS) ||
    cover.querySelector(CONTROLS) ||
    labelsAnother(cover, box)
  )
    return null
  const own = box.getBoundingClientRect()
  const drawn = cover.getBoundingClientRect()
  const dx = own.width / 2
  const dy = own.height / 2
  if (
    drawn.left < own.left - dx ||
    drawn.right > own.right + dx ||
    drawn.top < own.top - dy ||
    drawn.bottom > own.bottom + dy
  )
    return null
  const text = renderedText(cover)
  if (fold(text) === '') return keys ? wrapper : null
  const name = world.injected.utils.getElementAccessibleNameText(box, false)
  return sameName(text, name) ? wrapper : null
}

/**
 * Whether the cover is, or holds, a `<label>` for another control than the box: the page itself
 * says whose it is, and a person's click there goes to that field. A label that labels nothing
 * (no `for`, no control inside it) stays a drawing: floating-label kits draw with one.
 */
function labelsAnother(cover: Element, box: Element): boolean {
  const labels = [cover, ...cover.querySelectorAll('label')]
  return labels.some((label) => {
    if (label.localName !== 'label') return false
    const control: unknown = Reflect.get(label, 'control')
    return isNode(control) && control !== box
  })
}

/**
 * What an element draws as text: `innerText`, which leaves out what does not render, so an
 * overlay that hides the name and shows nothing draws nothing. An element with no `innerText`
 * (an SVG or MathML one) draws nothing the look can read: its `textContent` would count a
 * `<title>` that never shows.
 */
export function renderedText(element: Element): string {
  const text: unknown = Reflect.get(element, 'innerText')
  return typeof text === 'string' ? text : ''
}

/**
 * Two names are the same once folded (whitespace collapsed, lower case, a trailing `*` or `:`
 * dropped), both non-empty, and equal, or the longer holds the shorter and the shorter has at
 * least three characters (`Email address` drawn over a box named `Email`).
 */
function sameName(drawn: string, name: string): boolean {
  const a = fold(drawn)
  const b = fold(name)
  if (!a || !b) return false
  if (a === b) return true
  const [shorter, longer] = a.length < b.length ? [a, b] : [b, a]
  return shorter.length >= 3 && longer.includes(shorter)
}

/** Whitespace collapsed, lower case, a trailing `*` or `:` dropped. */
function fold(text: string): string {
  return text
    .replace(/\s+/g, ' ')
    .trim()
    .toLowerCase()
    .replace(/[\s*:]+$/, '')
}

/** Main's `DOM.scrollIntoViewIfNeeded` before its hit test: a no-op for a box in view. */
function scrollIntoViewIfNeeded(box: Element): void {
  const scroll: unknown = Reflect.get(box, 'scrollIntoViewIfNeeded')
  if (typeof scroll === 'function') Reflect.apply(scroll, box, [true])
  else box.scrollIntoView({ block: 'center', inline: 'center' })
}

/** Main's `blockerAt`, line for line, answering with the blocker, or `null` when there is none. */
function blockerAt(
  document: Document,
  box: Element,
  pointX: number,
  pointY: number,
): Element | null {
  let currentDocument = document
  let x = pointX
  let y = pointY
  let hit = currentDocument.elementFromPoint(x, y)
  while (
    hit &&
    (hit.tagName === 'IFRAME' || hit.tagName === 'FRAME') &&
    hit !== box
  ) {
    let child: unknown = null
    try {
      child = Reflect.get(hit, 'contentDocument')
    } catch {
      child = null
    }
    // A frame's document lives in that frame's realm, so `instanceof Document` (this realm's)
    // would stop the descent one frame down; main's script checks for a document at all.
    if (!isDocument(child)) break
    const frameRect = hit.getBoundingClientRect()
    x -= frameRect.x + hit.clientLeft
    y -= frameRect.y + hit.clientTop
    currentDocument = child
    hit = currentDocument.elementFromPoint(x, y)
  }
  if (!hit || hit === box || reaches(hit, box) || reaches(box, hit)) return null
  const hitLabel = hit.closest('label')
  if (hitLabel && (hitLabel.control === box || hitLabel.contains(box)))
    return null
  const boxLabel = box.closest('label')
  return (boxLabel?.contains(hit) ?? false) ? null : hit
}

/**
 * Main's `up`: the parent node, or across a shadow root to its host. Realm-free, as the hit can
 * come from a same-origin child frame's document.
 */
function up(node: Node): Node | null {
  const next: unknown =
    node.parentNode ??
    Reflect.get(node, 'host') ??
    Reflect.get(node.getRootNode(), 'host')
  return isNode(next) ? next : null
}

function isNode(value: unknown): value is Node {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof Reflect.get(value, 'nodeType') === 'number'
  )
}

function isDocument(value: unknown): value is Document {
  return isNode(value) && value.nodeType === Node.DOCUMENT_NODE
}

/** Main's `reaches`: `to` is `from` or one of its ancestors through shadow hosts. */
function reaches(from: Node, to: Node): boolean {
  for (let node: Node | null = from; node; node = up(node))
    if (node === to) return true
  return false
}
