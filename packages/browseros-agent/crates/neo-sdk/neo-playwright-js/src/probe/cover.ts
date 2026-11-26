/**
 * A pointer action under a cover, in Neo's isolated world (covered-click spec, sections 1 and 2;
 * Rust's half is neo-playwright/src/action/cover.rs).
 *
 *   coverMark   at each covered check: which cover it is, for the pointer runner's count on
 *               Rust's clock. The root of the hit target's subtree as upstream's
 *               `expectHitTarget` finds it (injectedScript.ts), its number in this world, its
 *               box, how opaque it shows, and whether it says it is loading.
 *   cover       once, when an action ends covered: hit-tests the point again and, while the same
 *               cover is on top, answers its facts. Which dialog it is, its markup and what it
 *               draws, the controls it shows, and how it stands to the control (the kind that
 *               picks the answer's line, in Rust).
 *
 * The page finds; Rust names (apc-engine-design 8.2). This world has no ARIA engine: every
 * decision here reads the live page, roles through the stand-in's pure-DOM `computeAriaRole`
 * (stubs/roleUtils.ts, held against the vendored engine by its differential test) and words
 * through the page's own text and labelling attributes. What an answer prints (the cover's ref,
 * role, name and block, each control's, the control a label is for) comes back as pins, which
 * Rust names from a capture of the tab's tree, so every ref it prints is one a snapshot prints.
 *
 * Every text read runs under the request's `FailureOutput.tree`, so a privately typed box in the
 * cover prints as the mask prints it (R47). Nothing here sends an event or changes the page.
 *
 * The helpers below (`hitChain`, `coverRoot`, `whichDialog`, `dialogBehind`, `kindOf`,
 * `takingControls`, `listedControls`) are exported for sibling files of this folder only, such as
 * a survey of the covers over a page's controls: one rule for "what covers it" and "what is its
 * own", and no second hit test anywhere. They are never part of the probe's protocol.
 */
import { computeAriaRole } from '../stubs/roleUtils'
import type { FailureOutput } from './failure-output'
import { SECTION_ROLES } from './pointer-targets'
import { CONTROLS, drawingOf, labelOf, renderedText } from './private-look'
import type {
  CoverFacts,
  CoverKind,
  CoverMark,
  CoverRequest,
  OwnAllowed,
  OwnPress,
  Point,
  ReplyTo,
} from './protocol'
import { resolveTarget } from './resolve'
import { mayCarryRoot } from './shadow-roots'
import type { World } from './world'

/**
 * How the cover stands to the control, as the page decides it: `CoverKind` with the control on
 * top as an element where a `labels` cover names it. Only the `cover` request's answer pins it;
 * an arm or a survey reads the kind and names nothing.
 */
export type PageKind =
  | Exclude<CoverKind, { is: 'labels' }>
  | { is: 'labels'; control: Element }

/** The roles that answer "which dialog", before the section roles. */
const DIALOG_ROLES: ReadonlySet<string> = new Set([
  'dialog',
  'alertdialog',
  'menu',
  'listbox',
  'tooltip',
])

/**
 * A notice's roles: a toast is named as itself, never as the full-view carrier a library keeps its
 * toasts in. They name the cover beside the dialog roles, and never stand for the dialog a blank
 * layer dims.
 */
const NOTICE_ROLES: ReadonlySet<string> = new Set(['status', 'alert', 'log'])

/** The most controls an answer lists. */
const MAX_CONTROLS = 8

/** How much of a wordless cover's drawn text stands in for its snapshot. */
const MAX_TEXT = 300

/**
 * Each cover root's number, the same at every check while the element is the same. A weak map in
 * this document's probe, as the ref map keeps frame keys: a pin would be a new number at every
 * call and spend one of the world's pin entries.
 */
const coverNumbers = new WeakMap<Element, number>()
let nextCoverNumber = 1

function coverNumber(element: Element): number {
  let known = coverNumbers.get(element)
  if (known === undefined) {
    known = nextCoverNumber++
    coverNumbers.set(element, known)
  }
  return known
}

/** The composed tree's parent: the parent element, or a shadow root's host. */
export function parentOrHost(element: Element): Element | undefined {
  if (element.parentElement) return element.parentElement
  const root = element.parentNode
  return root && root.nodeType === 11 ? (root as ShadowRoot).host : undefined
}

/**
 * What is on top at `point` and its ancestors up to `target`, walked as upstream's
 * `expectHitTarget` walks them (injectedScript.ts at v1.64.1): `null` when `target`, or one of its
 * descendants, is on top. The roots are found from the target up, which works in closed shadow
 * roots, and each root's hit leads into the next.
 */
export function hitChain(target: Element, point: Point): Element[] | null {
  let hit = hitAt(target, point)
  const chain: Element[] = []
  while (hit && hit !== target) {
    chain.push(hit)
    hit = hit.assignedSlot ?? parentOrHost(hit)
  }
  return hit === target ? null : chain
}

/** What is on top at `point` in the target's own chain of roots, as `hitChain` reads it. */
function hitAt(target: Element, point: Point): Element | undefined {
  const roots: (Document | ShadowRoot)[] = []
  for (let at: Element | undefined = target; at; ) {
    const root = at.getRootNode()
    if (root.nodeType !== 9 && root.nodeType !== 11) break
    roots.push(root as Document | ShadowRoot)
    if (root.nodeType === 9) break
    at = (root as ShadowRoot).host
  }
  return hitAtRoots(roots, point)
}

/**
 * The click's one root traversal runs from the outer document inward. A survey sample supplies
 * only the document: it must not enter a shadow root on behalf of a subject it does not have.
 */
export function hitAtRoots(
  roots: readonly (Document | ShadowRoot)[],
  point: Point,
): Element | undefined {
  let hit: Element | undefined
  for (let index = roots.length - 1; index >= 0; index--) {
    const root = roots[index]
    if (!root) break
    const elements = root.elementsFromPoint(point.x, point.y)
    const single = root.elementFromPoint(point.x, point.y)
    if (
      single &&
      elements[0] &&
      parentOrHost(single) === elements[0] &&
      getComputedStyle(single).display === 'contents'
    )
      elements.unshift(single)
    const inner = elements[0]
    if (!inner) break
    hit = inner
    const next = roots[index - 1]
    if (index && next && inner !== (next as ShadowRoot).host) break
  }
  return hit
}

/**
 * The root of the cover's subtree: the topmost element of the hit chain that is not in the
 * target's own chain, as upstream names it in "… from <root> subtree". When what is on top is
 * one of the target's own ancestors, that ancestor.
 */
export function coverRoot(target: Element, chain: readonly Element[]): Element {
  for (let at: Element | undefined = target; at; at = parentOrHost(at)) {
    const index = chain.indexOf(at)
    if (index > 0) return chain[index - 1] ?? at
    if (index === 0) return at
  }
  return chain[chain.length - 1] ?? document.documentElement
}

/** Its own opacity times every ancestor's, which a fade changes. */
function shownOpacity(element: Element): number {
  let opacity = 1
  for (let at: Element | undefined = element; at; at = parentOrHost(at)) {
    const own = Number.parseFloat(getComputedStyle(at).opacity)
    if (Number.isFinite(own)) opacity *= own
  }
  return opacity
}

/** It says it is loading: `aria-busy="true"` on it or around it, or a progress bar in it. */
export function saysLoading(element: Element): boolean {
  return (
    element.closest('[aria-busy="true"]') !== null ||
    element.querySelector('[role="progressbar"], progress') !== null
  )
}

/**
 * Which cover a covered check found: `target` is the element the hit test was for (the action's
 * retargeted element, or a parent page's iframe) and `point` the point in this frame's pixels.
 * `undefined` when the target is on top after all.
 */
export function coverMark(
  target: Element,
  point: Point,
): CoverMark | undefined {
  const chain = hitChain(target, point)
  if (!chain) return undefined
  const root = coverRoot(target, chain)
  const box = root.getBoundingClientRect()
  return {
    id: coverNumber(root),
    rect: { x: box.x, y: box.y, width: box.width, height: box.height },
    opacity: shownOpacity(root),
    busy: saysLoading(root),
  }
}

/**
 * The `cover` request: the facts of an action that ended covered, while the cover numbered
 * `request.cover` is still the one on top at `request.point`. A different cover, or none, answers
 * `null`, and the action answers as it did before this request.
 */
export function cover(
  world: World,
  request: CoverRequest,
  root: Node | undefined,
  _nodes: readonly unknown[],
  output: FailureOutput,
): ReplyTo<'cover'> {
  const found = resolveTarget(world, request.target, 'firstMatch', root)
  if (found.kind !== 'one') return found
  const { injected } = world
  const element = found.element
  // The arm's hit test is for the element its interceptor retargets to; a parent page's
  // test is for its iframe as it is.
  const subject = request.above
    ? element
    : (injected.retarget(element, 'button-link') ?? element)
  const chain = hitChain(subject, request.point)
  if (!chain) return { kind: 'cover', facts: null }
  const top = coverRoot(subject, chain)
  if (coverNumber(top) !== request.cover) return { kind: 'cover', facts: null }
  return {
    kind: 'cover',
    facts: factsOf(world, output, subject, chain, top, request),
  }
}

function factsOf(
  world: World,
  output: FailureOutput,
  subject: Element,
  chain: readonly Element[],
  top: Element,
  request: CoverRequest,
): CoverFacts {
  const kind = kindOf(subject, chain, request.point, request.above)
  const layer = whichDialog(chain, top, request.point)
  const { shown, behind, closed } = shownLayer(
    world,
    output,
    layer,
    request.point,
    subject,
  )
  // A cover that holds the control (its wrapper, its label, the page's root) is the control's
  // own surroundings: its block would be the page, so Rust prints none.
  const surrounds = shown.contains(subject) || shown === document.body
  // Every control the cover takes the pointer with, read in full: the list the answer prints is
  // cut to eight, and what the cover is (frames only or not) is decided before that cut.
  const taking = surrounds ? [] : takingControls(world, shown)
  const controls = listedControls(world, shown, taking)
  const words = surrounds || output.tree(shown, () => holdsWords(world, shown))
  // What it draws, which Rust says beside a block that holds no words: the tree prints a
  // `role=textbox` div without its own text.
  const drawn = surrounds
    ? []
    : output.tree(shown, () => Array.from(renderedText(shown).trim()))
  const finalKind = frameKind(shown, kind, taking)
  const labelled = finalKind.is === 'labels' ? [finalKind.control] : []
  // One entry for every element the answer names, each once, so none is evicted before Rust has
  // keyed them all in its capture.
  const named = [...new Set([shown, ...controls, ...labelled])]
  const pinned = world.pins.addAll(named)
  const pinOf = new Map(named.map((element, at) => [element, pinned[at] ?? 0]))
  const pin = (element: Element) => pinOf.get(element) ?? 0
  return {
    layer: coverNumber(layer),
    pin: pin(shown),
    // A blank layer prints as itself, the dialog open over it after; a dialog shown for its
    // nameless layer prints as the dialog.
    markup: output.previewNode(behind ? layer : shown),
    ...(drawn.length > 0 ? { text: drawn.slice(0, MAX_TEXT).join('') } : {}),
    controls: controls.map(pin),
    surrounds,
    blank: !isFrame(shown) && !closed && controls.length === 0 && !words,
    behind,
    closed,
    notice: !surrounds && inLiveRegion(shown),
    kind:
      finalKind.is === 'labels'
        ? { is: 'labels', pin: pin(finalKind.control) }
        : finalKind,
  }
}

/** One naming decision for a click and the survey; the latter has no clicked subject. */
export function shownAs(
  world: World,
  output: FailureOutput,
  layer: Element,
  point: Point,
  subject?: Element,
): Element {
  return shownLayer(world, output, layer, point, subject).shown
}

/** Keep the blank reading beside the chosen element: the click needs both, without a
 * second control walk or words test just to decide which markup to print. */
function shownLayer(
  world: World,
  output: FailureOutput,
  layer: Element,
  point: Point,
  subject: Element | undefined,
): { shown: Element; behind: boolean; closed: boolean } {
  const holds = subject
    ? layer.contains(subject) || layer === world.injected.document.body
    : pageElement(layer)
  const closed = subject
    ? !layer.contains(subject) && inClosedPart(layer)
    : !holds && inClosedPart(layer)
  if (holds || closed) return { shown: layer, behind: false, closed }
  const blank = isBlank(world, output, layer)
  const shown =
    (blank ? dialogBehind(world, layer, point) : soleDialog(world, layer)) ??
    layer
  // Only a blank face names a sibling behind it; a sole child dialog is the layer's content.
  return { shown, behind: blank && shown !== layer, closed }
}

/** A frame contains a separate document, never an empty backdrop in this world. */
function isFrame(element: Element): boolean {
  return element.localName === 'iframe' || element.localName === 'frame'
}

/**
 * The kind against the cover named and every control it takes the pointer with (not the list the
 * answer prints, which is cut to eight), for the frame's line:
 * - a cover that is a frame keeps its kind and its line;
 * - a cover that lists nothing but frames, one or more, gets the frame's kind whatever lies at the
 *   point, with a line that points into the frame (`inside`): its way out is in there, as on a
 *   sign-in sheet whose Close is the frame's own;
 * - a frame at the point of a cover with controls of its own (a sign-in dialog that draws its form
 *   in a frame) is `page`: the frame's line would send the agent to a bot-check solver with the
 *   dialog's Close listed above.
 */
function frameKind(
  shown: Element,
  kind: PageKind,
  taking: readonly Element[],
): PageKind {
  if (isFrame(shown)) return kind
  if (kind.is !== 'page' && kind.is !== 'frame') return kind
  const frames = taking.filter(isFrame)
  if (frames.length > 0 && frames.length === taking.length)
    return { is: 'frame', inside: true }
  return kind.is === 'frame' && frames.length < taking.length
    ? { is: 'page' }
    : kind
}

/**
 * "Which dialog": walking up from what is on top to the cover's root, (1) the first element with a
 * dialog-like role or a notice's role; else (2) the face, the outermost one that is fixed or in the
 * top layer and takes the pointer itself (a fixed full-view carrier with `pointer-events: none` is
 * no face: what it carries is); else (3) the first with one of the snapshot's section roles; else
 * (4) the root. Never the page: an
 * element that is or holds the document's main landmark, the body or the root element is skipped,
 * and then what is on top is named. (A fixed banner inside the page's `main` makes `main` the root
 * for a control outside it; its face is the banner.) What is on top starts inside the shadow roots
 * of the hit this world can read: a consent tool draws its dialog in its host's root, and the
 * document's hit test names only the host.
 */
export function whichDialog(
  chain: readonly Element[],
  top: Element,
  point: Point,
): Element {
  const path = coverPath(chain, top, point)
  const main = top.ownerDocument.querySelector('main, [role="main"]')
  const named = path.filter((at) => !pageElement(at, main))
  const face = coverFace(named)
  return (
    named.find((at) => {
      const role = roleOf(at)
      return DIALOG_ROLES.has(role) || NOTICE_ROLES.has(role)
    }) ??
    face ??
    named.find((at) => SECTION_ROLES.has(roleOf(at))) ??
    (pageElement(top, main) ? (path[0] ?? top) : top)
  )
}

/**
 * The element's role for the page's decisions, `''` for none: the stand-in's pure-DOM role,
 * which needs no layout or name computation and answers what the vendored engine answers.
 */
export function roleOf(element: Element): string {
  return computeAriaRole(element) ?? ''
}

/** The click's shadow-aware hit path, shared before naming and face selection. */
export function coverPath(
  chain: readonly Element[],
  top: Element,
  point: Point,
): Element[] {
  const [hit] = chain
  let deepest: Element | undefined = hit
  for (let depth = 0; deepest?.shadowRoot && depth < MAX_DEPTH; depth++) {
    const inner = deepest.shadowRoot.elementFromPoint(point.x, point.y)
    if (!inner || inner === deepest) break
    deepest = inner
  }
  const path: Element[] = []
  for (
    let at: Element | undefined = deepest;
    at && at !== hit;
    at = at.assignedSlot ?? parentOrHost(at)
  )
    path.push(at)
  for (const at of chain) {
    path.push(at)
    if (at === top) break
  }
  return path
}

/** The page carriers the click never names; the survey adds its whole-document majority guard. */
export function pageElement(
  at: Element,
  main = at.ownerDocument.querySelector('main, [role="main"]'),
): boolean {
  const document = at.ownerDocument
  return (
    at === document.documentElement ||
    at === document.body ||
    (main !== null && at.contains(main))
  )
}

/** The outermost pointer-taking element carried by the path's fixed or top-layer element. */
export function coverFace(named: readonly Element[]): Element | undefined {
  const outermostFixed = named.findLastIndex(
    (at) => getComputedStyle(at).position === 'fixed' || inTopLayer(at),
  )
  // The path runs outwards: a pointer-free fixed carrier delegates its face to what it carries.
  return outermostFixed < 0
    ? undefined
    : named.slice(0, outermostFixed + 1).findLast(takesPointer)
}

/**
 * No control of its own takes the pointer, and it draws no words (`holdsWords`): a backdrop, a
 * blank layer. Both read the composed tree, the roots this world can read, so a dialog drawn in a
 * shadow root is not blank.
 */
export function isBlank(
  world: World,
  output: FailureOutput,
  element: Element,
): boolean {
  return (
    !isFrame(element) &&
    takingControls(world, element).length === 0 &&
    !output.tree(element, () => holdsWords(world, element))
  )
}

/** The attributes whose words a snapshot prints as a name: a labelled icon, an image's text. */
const WORD_ATTRIBUTES = ['aria-label', 'title', 'alt'] as const

/**
 * Whether the element draws words, as a snapshot of it would print some: rendered text in it or
 * in a shadow root it holds, or an `aria-label`, `title` or `alt` on a shown element of its
 * composed tree. The page's own test, where the engine's was "its snapshot holds a name or a
 * text"; read under the walk's one bound (MAX_WALKED).
 */
function holdsWords(world: World, element: Element): boolean {
  const { utils } = world.injected
  const blank = (text: string | null) => (text ?? '').trim() === ''
  return [element, ...composedDescendants(element)].some(
    (at) =>
      // A shadow root's top elements draw what the host's own text leaves out.
      ((at === element || at.parentNode?.nodeType === 11) &&
        !blank(renderedText(at))) ||
      (WORD_ATTRIBUTES.some((name) => !blank(at.getAttribute(name))) &&
        utils.isElementVisible(at)),
  )
}

/**
 * The dialog a blank cover dims. Many libraries render the backdrop and its dialog as two children
 * of `<body>`, so walking up from the backdrop never reaches the dialog: a shown element of the
 * composed tree with a dialog role or in the top layer, neither inside the backdrop nor around it,
 * standing on it (at the dialog's own centre the backdrop is under it), and whose own controls take
 * the pointer; with several, the one nearest `point`. A dialog beside the layer is not the one it
 * dims: a loading layer beside a cookie notice stays blank. `undefined` when there is none.
 */
export function dialogBehind(
  world: World,
  backdrop: Element,
  point: Point,
): Element | undefined {
  const { injected } = world
  // The document's own query, with no count: a portal renders its dialog at the end of `<body>`,
  // after thousands of elements on a real page (3,620 to 21,981 measured). Beside it, the dialogs
  // drawn in shadow roots.
  const document = backdrop.ownerDocument
  const candidates = [
    ...document.querySelectorAll(DIALOGS),
    ...inShadowRoots(document).filter((element) => element.matches(DIALOGS)),
  ]
  let nearest: { dialog: Element; distance: number } | undefined
  for (const candidate of candidates) {
    if (backdrop.contains(candidate) || candidate.contains(backdrop)) continue
    if (!injected.utils.isElementVisible(candidate)) continue
    if (!DIALOG_ROLES.has(roleOf(candidate)) && !inTopLayer(candidate)) continue
    if (!standsOn(candidate, backdrop)) continue
    if (takingControls(world, candidate).length === 0) continue
    const box = candidate.getBoundingClientRect()
    const dx = Math.max(box.left - point.x, 0, point.x - box.right)
    const dy = Math.max(box.top - point.y, 0, point.y - box.bottom)
    const distance = Math.hypot(dx, dy)
    if (!nearest || distance < nearest.distance)
      nearest = { dialog: candidate, distance }
  }
  return nearest?.dialog
}

/** Its own `pointer-events` lets the pointer take it. */
export function takesPointer(element: Element): boolean {
  return getComputedStyle(element).pointerEvents !== 'none'
}

/**
 * A notice that may go by itself: a live region on the cover or around it, by a notice's role or
 * by `aria-live`. Its answer says so, beside its controls.
 */
function inLiveRegion(cover: Element): boolean {
  for (
    let at: Element | undefined = cover;
    at;
    at = at.assignedSlot ?? parentOrHost(at)
  ) {
    if (NOTICE_ROLES.has(roleOf(at))) return true
    const live = at.getAttribute('aria-live')
    if (live !== null && live !== 'off') return true
  }
  return false
}

/**
 * At the dialog's own centre, the backdrop is among the elements under the pointer. The centre of
 * the dialog's part in view: a hit test reads nothing outside the viewport. The hit test is the
 * backdrop's own root's: a document's names a shadow root's host, never what the root holds, so
 * a backdrop drawn inside a consent widget's root would never be found under its dialog.
 */
function standsOn(dialog: Element, backdrop: Element): boolean {
  const box = dialog.getBoundingClientRect()
  const left = Math.max(box.left, 0)
  const right = Math.min(box.right, innerWidth)
  const top = Math.max(box.top, 0)
  const bottom = Math.min(box.bottom, innerHeight)
  if (right <= left || bottom <= top) return false
  const root = backdrop.getRootNode() as Document | ShadowRoot
  return root
    .elementsFromPoint((left + right) / 2, (top + bottom) / 2)
    .some((element) => element === backdrop || backdrop.contains(element))
}

/** What may be the dialog a blank layer dims. */
const DIALOGS =
  '[role="dialog"], [role="alertdialog"], [aria-modal="true"], dialog[open], [popover]'

/** The most elements a walk of a cover's composed tree, or of the document's shadow roots, reads. */
const MAX_WALKED = 2000

/** The most shadow roots a hit is followed into. */
const MAX_DEPTH = 32

/**
 * `root`'s descendants in the composed tree: its light children, and the content of every shadow
 * root this world can read, which is an open one or a closed one a marked snapshot handed it
 * (`host.shadowRoot` answers both in Neo's world, shadow-roots.ts), and the elements a slot
 * draws: a dialog in a root whose buttons are slotted from its host's light DOM has them only
 * through its `<slot>`. In document order within each scope, each element once, at most
 * MAX_WALKED.
 */
function composedDescendants(root: Element | Document): Element[] {
  const walk: Walk = { found: [], seen: new Set(), scopes: [root] }
  const own = root.nodeType === 1 ? (root as Element).shadowRoot : null
  if (own) walk.scopes.push(own)
  for (let index = 0; index < walk.scopes.length; index++) {
    const scope = walk.scopes[index]
    if (!scope) continue
    for (const element of scope.querySelectorAll('*')) {
      if (walk.found.length >= MAX_WALKED) return walk.found
      if (!take(walk, element)) continue
      if (element.localName === 'slot')
        takeSlotted(walk, element as HTMLSlotElement)
    }
  }
  return walk.found
}

/** A composed walk's state: what it took, in order, and the scopes it has still to read. */
type Walk = {
  found: Element[]
  seen: Set<Element>
  scopes: (Element | Document | ShadowRoot)[]
}

/** Takes `element` once, and queues its own root: what a slot draws keeps its own there too. */
function take(walk: Walk, element: Element): boolean {
  if (walk.seen.has(element)) return false
  walk.seen.add(element)
  walk.found.push(element)
  if (element.shadowRoot) walk.scopes.push(element.shadowRoot)
  return true
}

/**
 * The elements `slot` draws, each with its light descendants and its own root: a slotted
 * design-system button keeps its real button in its own root. Under the walk's one limit.
 */
function takeSlotted(walk: Walk, slot: HTMLSlotElement): void {
  for (const assigned of slot.assignedElements({ flatten: true })) {
    if (walk.found.length >= MAX_WALKED) return
    if (take(walk, assigned)) walk.scopes.push(assigned)
  }
}

/**
 * The elements inside the shadow roots this world can read under `document`, nested roots
 * included. Every host is found wherever it stands, since a host check is one property read per
 * element; only the roots' own content is capped, at MAX_WALKED.
 */
function inShadowRoots(document: Document): Element[] {
  const roots: ShadowRoot[] = []
  for (const element of document.querySelectorAll('*'))
    if (element.shadowRoot) roots.push(element.shadowRoot)
  const found: Element[] = []
  for (let index = 0; index < roots.length; index++) {
    for (const element of roots[index]?.querySelectorAll('*') ?? []) {
      if (found.length >= MAX_WALKED) return found
      found.push(element)
      if (element.shadowRoot) roots.push(element.shadowRoot)
    }
  }
  return found
}

/**
 * The cover is, or holds, a custom element that may carry a closed shadow root this world does
 * not know (`mayCarryRoot`): its content is there, unread, and a new snapshot shows it. A plain
 * `div` or `span` host looks the same as a blank layer from here and stays blank.
 */
export function inClosedPart(cover: Element): boolean {
  const elements = [cover, ...composedDescendants(cover)]
  return elements.some(
    (element) => element.localName.includes('-') && mayCarryRoot(element),
  )
}

/** A modal `<dialog>` or an open popover: the browser draws it above every other layer. */
export function inTopLayer(element: Element): boolean {
  try {
    return element.matches(':modal') || element.matches(':popover-open')
  } catch {
    return false
  }
}

/** The roles that make an element a control, written in its `role` attribute. */
const CONTROL_ROLES: ReadonlySet<string> = new Set([
  'button',
  'link',
  'checkbox',
  'radio',
  'switch',
  'tab',
  'menuitem',
  'option',
])

/**
 * A control by the role written in its `role` attribute, whatever its tabindex: a custom close
 * button is often a `div` with `role="button"` and nothing else. The attribute, not a computed
 * role, so only what the page declares counts. A role attribute lists fallbacks; the first is the
 * one a browser tries first.
 */
function explicitControl(element: Element): boolean {
  const [role] = (element.getAttribute('role') ?? '').trim().split(/\s+/)
  return CONTROL_ROLES.has(role ?? '')
}

/**
 * The one dialog a nameless cover holds: the cover has no dialog-like and no notice role and no
 * name of its own (`labelOf`, the page's labelling attributes, which is what names a plain
 * container), and its composed tree holds exactly one shown element whose role is `dialog` or
 * `alertdialog` (an open `<dialog>` counts; `aria-modal` alone does not). None, or two or more, a
 * dialog nested in a dialog included: `undefined`, and the cover is shown as itself. The count
 * runs inside the walk's bound (MAX_WALKED), a named limit: a second dialog past it is not seen.
 * Giving up at the bound would take the name from every cover that holds one large dialog, which
 * is commoner, and counting past it costs a second walk.
 */
function soleDialog(world: World, cover: Element): Element | undefined {
  const { utils } = world.injected
  const role = roleOf(cover)
  if (DIALOG_ROLES.has(role) || NOTICE_ROLES.has(role)) return undefined
  if (labelOf(cover) !== '') return undefined
  const dialogs = composedDescendants(cover).filter((element) => {
    const its = roleOf(element)
    return (
      (its === 'dialog' || its === 'alertdialog') &&
      utils.isElementVisible(element)
    )
  })
  return dialogs.length === 1 ? dialogs[0] : undefined
}

/** What is a control by itself: `CONTROLS` without its bare `[tabindex]`. */
const NATIVE_CONTROLS =
  'input, textarea, select, button, a[href], iframe, frame, [contenteditable]'

/**
 * A cover's control, by what it is: a native control, a frame, an editable region, or an element
 * whose role attribute names a control. An element with only a tabindex is not one: a dialog's
 * container, its focus guards, an off-screen focus target and its title span all carry one, and
 * an answer that lists them hides the way out among them.
 */
export function coverControl(element: Element): boolean {
  return element.matches(NATIVE_CONTROLS) || explicitControl(element)
}

/**
 * The cover's controls that take the pointer at their middle, in the composed tree's order, so a
 * dialog drawn in its host's shadow root has its buttons. One out of view counts, since the agent
 * can scroll to it: a long dialog's button below the fold is the one an answer must not lose. This
 * list also decides whether a cover is blank and which dialog a backdrop dims, so a control it
 * misses can turn a dialog into "a blank layer", whose advice ends in a reload.
 */
export function takingControls(world: World, shown: Element): Element[] {
  const { injected } = world
  return composedDescendants(shown).filter((control) => {
    if (!coverControl(control)) return false
    if (!injected.utils.isElementVisible(control)) return false
    const box = control.getBoundingClientRect()
    const middle = { x: box.x + box.width / 2, y: box.y + box.height / 2 }
    const inView =
      middle.x >= 0 &&
      middle.y >= 0 &&
      middle.x < innerWidth &&
      middle.y < innerHeight
    return !inView || hitChain(control, middle) === null
  })
}

/**
 * Up to eight of the cover's controls that take the pointer, in their order. A dialog puts its
 * buttons last, so when there are more than eight the buttons are kept first. The `cover` answer
 * and `coverSurvey` pin them for Rust to word from its tree.
 */
export function listedControls(
  world: World,
  shown: Element,
  taking: readonly Element[] = takingControls(world, shown),
): Element[] {
  const kept = new Set<Element>(
    taking
      .filter((control) => roleOf(control) === 'button')
      .slice(0, MAX_CONTROLS),
  )
  for (const control of taking) {
    if (kept.size >= MAX_CONTROLS) break
    kept.add(control)
  }
  return taking.filter((control) => kept.has(control))
}

/**
 * How the cover stands to the control: spec section 2's table, the first row that holds. `above`
 * when `subject` is a page's iframe and the cover is in that page, above the control's frame.
 */
export function kindOf(
  subject: Element,
  chain: readonly Element[],
  point: Point,
  above: boolean,
): PageKind {
  const onTop = chain[0]
  if (!onTop) return { is: 'nothing' }
  if (folded(subject, point)) return { is: 'folded' }
  if (onTop.localName === 'iframe' || onTop.localName === 'frame')
    return { is: 'frame' }
  if (above) return { is: 'page' }
  const label = ownLabel(subject, onTop)
  if (label) return labelKind(label)
  const labelled = labelledControl(subject, onTop)
  if (labelled) return { is: 'labels', control: labelled }
  const parent = parentOrHost(subject)
  // No name hint reaches a click: its target was resolved in the page, so the drawing test reads
  // the control's own label.
  if (
    onTop === parent ||
    (parent && onTop === parentOrHost(parent)) ||
    drawingOf(subject, onTop, false) !== null
  )
    return { is: 'wrapper' }
  if (onTop.contains(subject) || onTop === document.documentElement)
    return { is: 'nothing' }
  const clear = clearPoint(subject)
  if (clear) return { is: 'partial', position: clear }
  return { is: 'page' }
}

/**
 * After the action's scroll, a parent with `overflow: hidden` or `clip` still clips the point
 * away: a closed panel or menu holds the control, and what is drawn there is not over it.
 */
function folded(subject: Element, point: Point): boolean {
  for (let at = parentOrHost(subject); at; at = parentOrHost(at)) {
    const style = getComputedStyle(at)
    const clips = (value: string) => value === 'hidden' || value === 'clip'
    if (!clips(style.overflowX) && !clips(style.overflowY)) continue
    const box = at.getBoundingClientRect()
    if (
      point.x < box.left ||
      point.x >= box.right ||
      point.y < box.top ||
      point.y >= box.bottom
    )
      return true
  }
  return false
}

/** The control's own `<label>` that holds what is on top, if it is one. */
function ownLabel(subject: Element, onTop: Element): Element | undefined {
  const labels: unknown = Reflect.get(subject, 'labels')
  if (!(labels instanceof NodeList)) return undefined
  return Array.from(labels).find(
    (label): label is Element =>
      label instanceof Element && label.contains(onTop),
  )
}

/** The roles whose press point cannot matter: a clear point of one is pressed by itself. */
const POINT_ROLES: ReadonlySet<string> = new Set([
  'link',
  'button',
  'checkbox',
  'radio',
  'switch',
  'tab',
  'menuitem',
  'option',
])

/**
 * What of the target's own takes a press its covered point refused: the element the interceptor
 * arms on, and the frame point it expects, which the caller works out from `press.position` for
 * a clear point (`act.ts`'s `clickPoint`, as a script's `position` is read).
 */
export type OwnChoice = {
  element: Element
  point?: Point
  press: OwnPress
}

/**
 * Covered-click C, the presses that land by themselves: what of `target`'s own may take a press
 * the interceptor refused at `point`. It goes by the covered answer's own kinds (`kindOf`), so
 * Neo presses where its answer would have told the agent to:
 * - its own label: at `point` when the label or its plain content is on top there, else at the
 *   label's first plain point; a label that is all link or control takes no press;
 * - its own wrapper or drawing: only the cover `allowed.wrapper` names, and only for a checkbox, a
 *   radio or a switch, whose state shows what the press did. Over any other control a wrapper
 *   may be a blank veil, and a parent's text holds its button's own text, so no test of what
 *   the wrapper draws tells the two apart: the answer comes;
 * - a clear point of it, the one the answer prints, for a target whose point cannot matter.
 * Never a frame, another control's label, or anything foreign. `undefined`: the answer comes.
 */
export function ownPress(
  world: World,
  output: FailureOutput,
  target: Element,
  point: Point,
  allowed: OwnAllowed,
): OwnChoice | undefined {
  const chain = hitChain(target, point)
  const onTop = chain?.[0]
  if (!chain || !onTop) return undefined
  const kind = kindOf(target, chain, point, false)
  if (kind.is === 'label') {
    const label = ownLabel(target, onTop)
    if (!label) return undefined
    const at = ownSurface(label, onTop) ? point : labelPlainPoint(label)
    if (!at) return undefined
    return {
      element: label,
      point: at,
      press: { case: 'label', markup: output.previewNode(label) },
    }
  }
  if (kind.is === 'wrapper') {
    if (coverNumber(coverRoot(target, chain)) !== allowed.wrapper)
      return undefined
    // What takes the press as a wrapper is nothing in itself: the element pressed, and each parent
    // of it below the one that holds the control, is plain by `ownSurface`'s one list. A `span
    // role="button"` drawing the control's name, a radio card that is a link, or a component
    // whose root draws over the radio would run its own action instead.
    if (!plainElement(onTop)) return undefined
    for (
      let at = onTop.parentElement;
      at && !at.contains(target);
      at = at.parentElement
    )
      if (!plainElement(at)) return undefined
    const checked = checkedState(world, target)
    if (checked === undefined) return undefined
    return {
      element: onTop,
      point,
      press: { case: 'wrapper', markup: output.previewNode(onTop), checked },
    }
  }
  if (kind.is === 'partial') {
    if (!POINT_ROLES.has(roleOf(target)) && target.localName !== 'summary')
      return undefined
    return {
      element: target,
      press: { case: 'point', position: kind.position },
    }
  }
  return undefined
}

/**
 * A checkbox or radio that is not shown, and a label of its own that is: the label a click
 * through it goes to (a label's state two). Checkboxes and radios only. One with a plain point
 * in the view comes first; else the first shown one, which the attempt scrolls into the view
 * before `labelPlace` looks for its plain point (a terms box at the foot of a long form).
 */
export function shownOwnLabel(
  world: World,
  control: Element,
): Element | undefined {
  if (control.localName !== 'input') return undefined
  const type = control.getAttribute('type')?.toLowerCase()
  if (type !== 'checkbox' && type !== 'radio') return undefined
  const labels: unknown = Reflect.get(control, 'labels')
  if (!(labels instanceof NodeList)) return undefined
  const shown = Array.from(labels).filter(
    (label): label is Element =>
      label instanceof Element && world.injected.utils.isElementVisible(label),
  )
  return shown.find((label) => labelPlainPoint(label) !== undefined) ?? shown[0]
}

/**
 * Whether `hit` lies on `owner`'s own surface: a point is the owner's own when nothing between
 * what was hit there and the owner is a link, a control (native, with a tabindex, or by a
 * declared role), a frame, or a shadow host, since what a host draws is not seen from here. The
 * label, the clear point and the wrapper all ask it. The owner itself is not asked: a target that
 * is a host keeps its insides as its own. The walk stops at the owner, so a container around it,
 * such as a dialog's body with its tabindex, is not the owner's.
 */
function ownSurface(owner: Element, hit: Element): boolean {
  for (let at: Element | null = hit; at && at !== owner; at = at.parentElement)
    if (!plainElement(at)) return false
  return owner.contains(hit)
}

/**
 * What is on top at `point` when it is not `owner`'s own surface (`ownSurface`), hit-tested in
 * the owner's own chain of roots as `hitChain` reads it, never taken from an event's target,
 * which hides what is inside a closed root. `undefined` when the point is the owner's own. The
 * guard at delivery (`act.ts`'s `guardPress`) asks it where a press arrives.
 */
export function notOwnAt(owner: Element, point: Point): Element | undefined {
  const hit = hitAt(owner, point)
  if (!hit) return owner.ownerDocument.documentElement
  return ownSurface(owner, hit) ? undefined : hit
}

/**
 * `ownSurface`'s one list: no link, no control (native, with a tabindex, or by a declared role),
 * no frame (a frame is a control) and no shadow host.
 */
function plainElement(element: Element): boolean {
  return !(
    element.matches(`a[href], ${CONTROLS}`) ||
    explicitControl(element) ||
    drawsUnseen(element)
  )
}

/**
 * A shadow host: what it draws in its root (a link, perhaps) is not seen as plain, whether this
 * world reads that root or not. A root this world reads shows as `shadowRoot`; one it cannot is
 * known by `inClosedPart`'s test. Both a hit and the root's own hit answer the host for a point
 * inside its root, so the host is always on a walk from what was hit. Slotted text needs nothing:
 * its parent is the host.
 */
function drawsUnseen(element: Element): boolean {
  return (
    element.shadowRoot !== null ||
    (element.localName.includes('-') && mayCarryRoot(element))
  )
}

/** The label's first plain point (`pointOffLinks`), in the frame's pixels. */
export function labelPlainPoint(label: Element): Point | undefined {
  const position = pointOffLinks(label)
  return position && positionPoint(label, position).point
}

/** A checkbox's, radio's or switch's state; `undefined` for any other control. */
function checkedState(world: World, control: Element): boolean | undefined {
  const role = roleOf(control)
  if (role !== 'checkbox' && role !== 'radio' && role !== 'switch')
    return undefined
  try {
    return world.injected.elementState(control, 'checked', true).matches
  } catch {
    return undefined
  }
}

/** The label's links, and a point of it on none of them when it has some. */
function labelKind(label: Element): PageKind {
  const links = label.querySelectorAll('a[href]').length
  if (links === 0) return { is: 'label', links: 0 }
  const position = pointOffLinks(label)
  return position ? { is: 'label', links, position } : { is: 'label', links }
}

/**
 * A point of the label, from its padding edge, where the label itself or its plain content is on
 * top: no link and no control inside it. Tried on a 5 by 3 grid over its box.
 */
function pointOffLinks(label: Element): Point | undefined {
  // Hit-tested in the label's own root, so one inside a shadow root is not retargeted to its host.
  const root = label.getRootNode()
  const scope = root instanceof ShadowRoot ? root : document
  for (const fy of [0.5, 0.25, 0.75])
    for (const fx of [0.1, 0.3, 0.5, 0.7, 0.9]) {
      const position = samplePosition(label, fx, fy)
      const { point } = positionPoint(label, position)
      const chain = hitChain(label, point)
      if (chain !== null) continue
      const on = scope.elementFromPoint(point.x, point.y)
      if (on && !ownSurface(label, on)) continue
      return position
    }
  return undefined
}

/** The control on top, when the target is a `<label>` for it. */
function labelledControl(
  subject: Element,
  onTop: Element,
): Element | undefined {
  if (subject.localName !== 'label') return undefined
  const control: unknown = Reflect.get(subject, 'control')
  if (!(control instanceof Element)) return undefined
  return control === onTop || control.contains(onTop) ? control : undefined
}

/**
 * A script's `position` as a point of the frame: from the element's padding edge, its border
 * widths read as upstream reads them (`parseInt`). `clickPoint` converts a script's position with
 * it, and `clearPoint` and the label's plain point test the very point it gives, so the point
 * that is tested is the point that is pressed.
 */
export function positionPoint(
  element: Element,
  position: Point,
): { point: Point; borderOffset: Point } {
  const style = getComputedStyle(element)
  const box = element.getBoundingClientRect()
  const borderOffset = {
    x: Number.parseInt(style.borderLeftWidth || '0', 10) + position.x,
    y: Number.parseInt(style.borderTopWidth || '0', 10) + position.y,
  }
  return {
    point: { x: box.x + borderOffset.x, y: box.y + borderOffset.y },
    borderOffset,
  }
}

/** A sample of `element`'s box at the fractions `fx`, `fy`, as a whole-pixel position. */
function samplePosition(element: Element, fx: number, fy: number): Point {
  const box = element.getBoundingClientRect()
  const origin = positionPoint(element, { x: 0, y: 0 }).point
  return {
    x: Math.round(box.x + box.width * fx - origin.x),
    y: Math.round(box.y + box.height * fy - origin.y),
  }
}

/**
 * One of eight other points of the control where it is on top, on its own surface
 * (`ownSurface`: no link, control, frame or component nested in it), as `click({ position })`
 * takes it. Each sample is rounded to the whole-pixel position first, and the point that
 * position gives (`positionPoint`) is the one tested: in the view, on top, the target's own.
 * Both a press of the target's own and the covered answer's point come from here, so neither
 * lands on a control inside the target; one whose only clear parts are such controls has none.
 */
function clearPoint(subject: Element): Point | undefined {
  for (const [fx, fy] of [
    [0.25, 0.25],
    [0.5, 0.25],
    [0.75, 0.25],
    [0.25, 0.5],
    [0.75, 0.5],
    [0.25, 0.75],
    [0.5, 0.75],
    [0.75, 0.75],
  ] as const) {
    const position = samplePosition(subject, fx, fy)
    const { point } = positionPoint(subject, position)
    if (
      point.x < 0 ||
      point.y < 0 ||
      point.x >= innerWidth ||
      point.y >= innerHeight
    )
      continue
    const hit = hitAt(subject, point)
    if (hit && ownSurface(subject, hit)) return position
  }
  return undefined
}
