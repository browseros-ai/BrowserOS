/**
 * The survey behind a snapshot's `# on top:` lines: which layers cover the page's controls.
 * The click module owns hits, faces, control eligibility and names. This module owns grouping,
 * reach and weak per-document memory; child frames and pressing are outside its scope.
 *
 * One survey, two answers (apc-engine-design 8.2):
 *   surveyCovers  inside a `snapshot` request, over the engine snapshot it just took: the layers
 *                 worded by role, name and ref, and the snapshot's whole-view nodes marked
 *   coverSurvey   a request of its own, for a page whose snapshot comes from APC and so has no
 *                 engine snapshot under it: the page's own controls are tested, and every element
 *                 the answer names comes back pinned, for Rust to word from its tree
 */
import type { AriaNodeJSON, AriaSnapshotJSON } from '../prelude/vendored'
import { composedElements } from './composed-tree'
import {
  controlRole,
  controlsOf,
  coverControl,
  coverFace,
  coverPath,
  coverRoot,
  hitAtRoots,
  hitChain,
  inClosedPart,
  inTopLayer,
  isBlank,
  kindOf,
  listedControls,
  nameOf,
  pageElement,
  parentOrHost,
  saysLoading,
  shownAs,
  takesPointer,
  whichDialog,
} from './cover'
import type { FailureOutput } from './failure-output'
import { renderedText } from './private-look'
import type {
  CoverControl,
  CoverSurveyRequest,
  PinnedLayer,
  Point,
  ReplyTo,
} from './protocol'
import { type SnapshotAria, snapshotAriaFor } from './snapshot-aria'
import type { World } from './world'

const MAX_CONTROLS = 300
const BUDGET_MS = 20
const MAX_LAYERS = 3
const DIALOGS = new Set([
  'dialog',
  'alertdialog',
  'menu',
  'listbox',
  'tooltip',
  'status',
  'alert',
  'log',
])
type Inside = (root: Element, element: Element) => boolean
type Sample = { point: Point; hit: Element | undefined; deepest?: Element }
/** One cached click path can be reclassified after the lazy guard without another page hit. */
type Hit = {
  path: Element[]
  chain: Element[]
  root: Element
  point: Point
  face: Element
  observed: boolean
  subject?: Element
}
type Face = {
  element: Element
  named: Element
  roots: Set<Element>
  observed: boolean
}
type Tested = { element: Element; clear: boolean }
type Remembered = {
  face: WeakRef<Element>
  named: WeakRef<Element>
  roots: WeakRef<Element>[]
  order: number
}
const memory = new WeakMap<Document, Remembered[]>()

/** What a layer is called by in the snapshot's answer. */
type RefName = { role: string; name: string; ref: string }

/**
 * A layer's facts, `N` naming it and `C` each of its listed controls. Page facts are masked
 * here; the shared Rust formatter owns their public words.
 */
type Note<N, C> = N & {
  blank: boolean
  loading: boolean
  wholeView: boolean
  controls: C[]
  firstWords?: string
}

export type SurveyLayer = Note<RefName, CoverControl>

/**
 * How an answer names what a layer holds. The survey asks in a fixed order (a layer's controls,
 * then what names it, layer by layer), which the snapshot's answer depends on: its refs are
 * minted as they are first named, and the agent reads their numbers.
 */
type Wording<N, C> = {
  /** A root's listed controls, the identity that dedupes them across roots, and what names it. */
  controls(root: Element): C[]
  same(control: C): unknown
  named(element: Element): N
}

type Phases = {
  hits: number
  count: number
  naming: number
  linking: number
  marks: number
  stopped: boolean
}
type Layer<N, C> = { order: number; held: Element[]; note: Note<N, C> }
type Group = { faces: Face[]; held: Element[]; controlsFrom: Element[] }

/** The survey of a `snapshot` request, over the engine snapshot `json` it took. */
export function surveyCovers(
  world: World,
  output: FailureOutput,
  json: AriaSnapshotJSON,
  aria: SnapshotAria = snapshotAriaFor(world),
): {
  layers: SurveyLayer[]
  tested: number
  marksSkipped: boolean
  phases: Phases
} {
  const { layers, tested, phases } = surveyLayers(
    world,
    output,
    aria.surveyElements(),
    aria,
    {
      controls: (root) => controlsOf(world, output, root, undefined, aria),
      same: (control) => control.ref,
      named: (element) => ({
        role: controlRole(world, element, aria),
        name: nameOf(world, output, element, aria),
        ref: aria.refOf(element),
      }),
    },
  )
  const marksSkipped = markBlocks(json, layers, inside, phases, aria)
  return {
    layers: layers.map(({ note }) => note),
    tested,
    marksSkipped,
    phases,
  }
}

/**
 * `coverSurvey`: the survey of a page whose snapshot comes from APC. It tests the page's own
 * controls instead of an engine snapshot's refs, and names each layer by pins: the element it is
 * named by, its listed controls and, for a whole-view layer with words, the elements that hold
 * it, under which Rust marks the tree's top-most nodes. Decisions read the same SnapshotAria
 * provider as the snapshot's survey; only names and refs are left to Rust.
 */
export function coverSurvey(
  world: World,
  _request: CoverSurveyRequest,
  _root: Node | undefined,
  _nodes: readonly unknown[],
  output: FailureOutput,
): ReplyTo<'coverSurvey'> {
  const aria = snapshotAriaFor(world)
  const { layers, tested, phases } = surveyLayers(
    world,
    output,
    { [Symbol.iterator]: () => pageControls(world) },
    aria,
    {
      controls: (root) => listedControls(world, root, undefined, aria),
      same: (control) => control,
      named: (element) => ({ named: element }),
    },
  )
  const holdersOf = ({ held, note }: (typeof layers)[number]) =>
    note.wholeView && !note.blank ? held : []
  // One entry for everything the answer names, each element once, so none is evicted before
  // Rust has described them all.
  const named = [
    ...new Set(
      layers.flatMap((layer) => [
        layer.note.named,
        ...layer.note.controls,
        ...holdersOf(layer),
      ]),
    ),
  ]
  const pins = world.pins.addAll(named)
  const pinOf = new Map(
    named.map((element, index) => [element, pins[index] ?? 0]),
  )
  const pin = (element: Element) => pinOf.get(element) ?? 0
  return {
    kind: 'surveyed',
    layers: layers.map(
      (layer): PinnedLayer => ({
        pin: pin(layer.note.named),
        blank: layer.note.blank,
        loading: layer.note.loading,
        wholeView: layer.note.wholeView,
        controls: layer.note.controls.map(pin),
        ...(layer.note.firstWords === undefined
          ? {}
          : { firstWords: layer.note.firstWords }),
        holders: holdersOf(layer).map(pin),
      }),
    ),
    tested,
    // What the snapshot's answer reports as its survey time: hits and the tested prefix.
    surveyMs: phases.hits,
    phases,
  }
}

/**
 * The page's own controls in composed-tree order, filtered as a snapshot's ref is (apc-engine-
 * design 3.6, upstream's `computeAriaRef`): box-visible and taking the pointer, a test an iframe
 * always passes there. The refs the engine's snapshot would have held, without its aria walk.
 */
function* pageControls(world: World): Generator<Element> {
  const { injected } = world
  const root = injected.document.documentElement
  if (!root) return
  for (const element of composedElements(root))
    if (
      coverControl(element) &&
      (isFrame(element) || takesPointer(element)) &&
      injected.utils.isElementVisible(element)
    )
      yield element
}

function isFrame(element: Element): boolean {
  return element.nodeName === 'IFRAME' || element.nodeName === 'FRAME'
}

/**
 * Layer membership follows the rendered flat tree, so a panel owns its slotted controls and
 * content as well; the engine's scope walk crosses hosts but not slots.
 */
function inside(root: Element, element: Element): boolean {
  for (
    let at: Element | undefined = element;
    at;
    at = at.assignedSlot ?? parentOrHost(at)
  )
    if (at === root) return true
  return false
}

/**
 * The survey both answers share, over `elements` in tree order. It finds, groups and names the
 * layers, layer by layer through `wording`, and keeps the document's memory of them.
 */
function surveyLayers<N, C>(
  world: World,
  output: FailureOutput,
  elements: Iterable<Element>,
  aria: SnapshotAria,
  wording: Wording<N, C>,
): { layers: Layer<N, C>[]; tested: number; phases: Phases } {
  const { injected } = world
  const document = injected.document
  const phases: Phases = {
    hits: 0,
    count: 0,
    naming: 0,
    linking: 0,
    marks: 0,
    stopped: false,
  }
  // Five document-only hits always precede the budgeted prefix of controls. A shadow host
  // remains a host here: only the click's own root list can take a hit into that root.
  const started = performance.now()
  const samples: Sample[] = viewPoints().map((point) => ({
    point,
    hit: hitAtRoots([document], point),
  }))
  const { hits, tested, main } = findFaces(
    world,
    elements,
    samples,
    started,
    phases,
    aria,
  )
  phases.hits = performance.now() - started
  const page = pageGuard(elements, phases, main, aria)
  const resolving = performance.now()
  const found = facesFromHits(world, hits, page, aria)
  // This is the first use of the guard, so its entire count time belongs to this resolution.
  // Charge that work to counting rather than to naming.
  if (hits.length)
    phases.naming += Math.max(0, performance.now() - resolving - phases.count)
  const remembered = memory.get(document) ?? []
  retainFaces(world, found, remembered, page)
  for (const [element, face] of found) {
    if (!face.observed || page(element)) found.delete(element)
    else for (const root of face.roots) if (page(root)) face.roots.delete(root)
  }
  if (!found.size) {
    memory.delete(document)
    return { layers: [], tested: tested.length, phases }
  }
  const naming = performance.now()
  // Isolate guard time spent while naming: a carrier's guard belongs to count, not both phases.
  const countedBeforeNaming = phases.count
  phases.count = 0
  const centre = { x: innerWidth / 2, y: innerHeight / 2 }
  for (const face of found.values())
    face.named = shownAs(world, output, face.named, centre, undefined, aria)
  const groups = joinFaces(found, inside).map((faces) => ({
    faces,
    ...holdersOf(faces, inside, page),
  }))
  const next: Remembered[] = []
  const layers = groups
    .map((group) =>
      describeLayer(
        world,
        output,
        group,
        groups,
        remembered,
        next,
        tested,
        samples,
        inside,
        aria,
        wording,
      ),
    )
    .filter((layer) => layer !== undefined)
    .sort(
      (a, b) =>
        Number(b.note.wholeView) - Number(a.note.wholeView) ||
        a.order - b.order,
    )
    .slice(0, MAX_LAYERS)
  memory.set(document, next)
  phases.naming += Math.max(0, performance.now() - naming - phases.count)
  phases.count += countedBeforeNaming
  return { layers, tested: tested.length, phases }
}

/** Samples seed candidates; only a foreign hit on an outside control makes one a layer. */
function findFaces(
  world: World,
  elements: Iterable<Element>,
  samples: Sample[],
  started: number,
  phases: Phases,
  aria: SnapshotAria,
) {
  const main = world.injected.document.querySelector('main, [role="main"]')
  const hits: Hit[] = []
  const tested: Tested[] = []
  for (const sample of samples) {
    const { point, hit } = sample
    const chain: Element[] = []
    for (let at = hit; at; at = at.assignedSlot ?? parentOrHost(at))
      chain.push(at)
    const top = chain.at(-1)
    if (top) sample.deepest = rememberHit(hits, chain, top, point, false, main)
  }
  // The provider yields candidates in tree order. Filtering and testing in one pass means a
  // budget stop leaves a tested prefix, never a selected list whose tail was silently untested.
  for (const element of elements) {
    if (
      tested.length >= MAX_CONTROLS ||
      performance.now() - started >= BUDGET_MS
    ) {
      phases.stopped = true
      break
    }
    if (!coverControl(element)) continue
    if (
      !world.injected.elementState(element, 'visible', true).matches ||
      !world.injected.elementState(element, 'enabled', true).matches
    )
      continue
    const point = middle(element)
    if (!point) continue
    const subject = world.injected.retarget(element, 'button-link') ?? element
    const chain = hitChain(subject, point)
    const kind = chain
      ? kindOf(world, subject, chain, point, false, aria).is
      : 'nothing'
    const covered = kind === 'page' || kind === 'frame'
    tested.push({ element, clear: !covered })
    if (chain && covered)
      rememberHit(
        hits,
        chain,
        coverRoot(subject, chain),
        point,
        true,
        main,
        subject,
      )
  }
  return { hits, tested, main }
}

/** Retain the click's path so a page face can be peeled after the lazy guard, without a new hit. */
function rememberHit(
  hits: Hit[],
  chain: Element[],
  root: Element,
  point: Point,
  observed: boolean,
  main: Element | null,
  subject?: Element,
): Element | undefined {
  const path = coverPath(chain, root, point)
  const filtered = path.filter((element) => !pageElement(element, main))
  const face = coverFace(filtered)
  if (face && !edgeBar(face))
    hits.push({ path: filtered, chain, root, point, face, observed, subject })
  return path[0]
}

/** A page shell can carry another fixed layer. Peeling it preserves that layer's own root. */
function facesFromHits(
  world: World,
  hits: Hit[],
  page: (element: Element) => boolean,
  aria: Pick<SnapshotAria, 'roleOf'>,
): Map<Element, Face> {
  const found = new Map<Element, Face>()
  for (const hit of hits) {
    // Samples provide reach only. Without a covered control, even a fixed app shell must
    // not trigger the lazy document count or become a layer.
    if (!hit.observed) continue
    let path = hit.path
    let element: Element | undefined = hit.face
    let peeled = false
    while (element && page(element)) {
      path = path.slice(0, path.indexOf(element))
      element = coverFace(path)
      peeled = true
    }
    if (!element || edgeBar(element)) continue
    const root = peeled
      ? hit.subject
        ? coverRoot(hit.subject, path)
        : (path.at(-1) ?? element)
      : hit.root
    const known = found.get(element)
    if (known) {
      known.observed = true
      if (!page(root)) known.roots.add(root)
      continue
    }
    const named = whichDialog(world, hit.chain, root, hit.point, aria)
    found.set(element, {
      element,
      named: page(named) ? element : named,
      roots: new Set(!page(root) ? [root] : []),
      observed: true,
    })
  }
  return found
}

/** The expensive majority clause is lazy: ordinary pages never count the whole document. */
function pageGuard(
  elements: Iterable<Element>,
  phases: Phases,
  main: Element | null,
  aria: Pick<SnapshotAria, 'roleOf'>,
): (element: Element) => boolean {
  let controls: Element[] | undefined
  const cached = new Map<Element, boolean>()
  // Dialog ancestry shields its controls; a menu or listbox does not stop its app shell
  // from being the page. Keep the broader naming exemption separate from that test.
  const meanings = new Map<Element, { dialog: boolean; exempt: boolean }>()
  const meaning = (element: Element) => {
    let known = meanings.get(element)
    if (!known) {
      const role = aria.roleOf(element) ?? ''
      const top = inTopLayer(element)
      known = {
        dialog: role === 'dialog' || role === 'alertdialog' || top,
        exempt: DIALOGS.has(role) || top,
      }
      meanings.set(element, known)
    }
    return known
  }
  // Ref-less dialog ancestors count too. Membership and the intervening dialog are one
  // flat-tree walk; no descendant query or second snapshot is needed to find either.
  const ownControl = (root: Element, control: Element) => {
    let inDialog = false
    for (
      let at: Element | undefined = control;
      at;
      at = at.assignedSlot ?? parentOrHost(at)
    ) {
      if (at === root) return !inDialog
      inDialog ||= meaning(at).dialog
    }
    return false
  }
  return (element) => {
    const known = cached.get(element)
    if (known !== undefined) return known
    const started = performance.now()
    let page = false
    try {
      if (meaning(element).exempt) return false
      for (
        let at = element.assignedSlot ?? parentOrHost(element);
        at;
        at = at.assignedSlot ?? parentOrHost(at)
      )
        if (meaning(at).dialog) return false
      if (pageElement(element, main)) {
        page = true
        return true
      }
      controls ??= Array.from(elements).filter((element) =>
        coverControl(element),
      )
      // A dialog's controls cannot turn its carrier into the page. The denominator remains
      // the whole document, so a fixed app shell is still judged by its own controls.
      page =
        controls.filter((control) => ownControl(element, control)).length >
        controls.length / 2
      return page
    } finally {
      cached.set(element, page)
      phases.count += performance.now() - started
    }
  }
}

/** Weak element identity survives a scroll, while detached faces and hidden dialogs disappear. */
function retainFaces(
  world: World,
  found: Map<Element, Face>,
  remembered: Remembered[],
  page: (element: Element) => boolean,
): void {
  for (const entry of remembered) {
    const element = entry.face.deref()
    if (!element || !stands(world, element) || page(element)) continue
    const known = found.get(element)
    if (known) {
      known.observed = true
      continue
    }
    const named = entry.named.deref()
    found.set(element, {
      element,
      named:
        named?.isConnected &&
        world.injected.elementState(named, 'visible', true).matches
          ? named
          : element,
      roots: new Set(
        entry.roots.flatMap((root) => {
          const live = root.deref()
          return live?.isConnected && !page(live) ? [live] : []
        }),
      ),
      observed: true,
    })
  }
}

function joinFaces(found: Map<Element, Face>, inside: Inside): Face[][] {
  const groups: Face[][] = []
  for (const face of found.values()) {
    const joins = groups.filter((group) =>
      group.some(
        (other) =>
          Array.from(face.roots).some((root) => other.roots.has(root)) ||
          face.named === other.element ||
          other.named === face.element ||
          (face.named !== face.element && inside(face.named, other.element)) ||
          (other.named !== other.element && inside(other.named, face.element)),
      ),
    )
    const first = joins[0]
    if (!first) {
      groups.push([face])
      continue
    }
    first.push(face)
    for (const joined of joins.slice(1)) {
      first.push(...joined)
      groups.splice(groups.indexOf(joined), 1)
    }
  }
  return groups
}

/**
 * A joined portal owns its shared holder. When a backdrop names an inner dialog, the dialog's
 * pointer-taking fixed carrier belongs to the same layer, so the gap beside the dialog still
 * counts as covered. The page guard peels app shells rather than adopting their content.
 * Controls and loading facts keep their original holders, as the shared covered answer does.
 */
function holdersOf(
  faces: Face[],
  inside: Inside,
  page: (element: Element) => boolean,
): { held: Element[]; controlsFrom: Element[] } {
  const shared = [...new Set(faces.flatMap((face) => [...face.roots]))].filter(
    (root) => faces.filter((face) => face.roots.has(root)).length > 1,
  )
  const held = shared.length ? shared : faces.map((face) => face.element)
  const controlsFrom = [...held]
  for (const { named } of faces) {
    if (!controlsFrom.some((root) => inside(root, named)))
      controlsFrom.push(named)
    if (held.some((root) => inside(root, named))) continue
    const path: Element[] = []
    for (
      let at: Element | undefined = named;
      at;
      at = at.assignedSlot ?? parentOrHost(at)
    )
      path.push(at)
    let carrier = coverFace(path)
    while (carrier && page(carrier)) {
      path.splice(path.indexOf(carrier))
      carrier = coverFace(path)
    }
    held.push(carrier ?? named)
  }
  return { held, controlsFrom }
}

function describeLayer<N, C>(
  world: World,
  output: FailureOutput,
  group: Group,
  groups: Group[],
  remembered: Remembered[],
  next: Remembered[],
  tested: Tested[],
  samples: Sample[],
  inside: Inside,
  aria: SnapshotAria,
  wording: Wording<N, C>,
): Layer<N, C> | undefined {
  const { faces, held, controlsFrom } = group
  const face =
    faces.find((face) => isBlank(world, output, face.element, aria)) ?? faces[0]
  if (!face) return undefined
  const old = remembered.find((entry) =>
    faces.some((face) => entry.face.deref() === face.element),
  )
  const order =
    old?.order ??
    remembered.reduce((last, entry) => Math.max(last, entry.order + 1), 0) +
      next.length
  const named = face.named
  const belongs = (element: Element) =>
    held.some((root) => inside(root, element))
  const ownOfAny = (element: Element) =>
    groups.some(({ held }) => held.some((root) => inside(root, element)))
  const wholeView =
    samples.every(({ deepest }) => deepest && belongs(deepest)) &&
    tested.every(({ element, clear }) => !clear || ownOfAny(element)) &&
    tested.some(({ element, clear }) => !clear && !belongs(element))
  const controls = Array.from(
    new Map(
      controlsFrom
        .flatMap((root) => wording.controls(root))
        .map((control) => [wording.same(control), control]),
    ).values(),
  )
  const words = output.tree(named, () =>
    renderedText(named).trim().replace(/\s+/g, ' '),
  )
  const blank =
    !inClosedPart(named) &&
    controls.length === 0 &&
    !words &&
    isBlank(world, output, named, aria)
  for (const member of faces)
    next.push({
      face: new WeakRef(member.element),
      named: new WeakRef(member.named),
      roots: Array.from(member.roots, (root) => new WeakRef(root)),
      order,
    })
  return {
    order,
    held,
    note: {
      ...wording.named(named),
      blank,
      loading: controlsFrom.some(saysLoading),
      wholeView,
      controls,
      ...(words ? { firstWords: Array.from(words).slice(0, 61).join('') } : {}),
    },
  }
}

/** Link and mark only an actual whole-view layer, after its reach is known. */
function markBlocks(
  json: AriaSnapshotJSON,
  layers: Layer<RefName, CoverControl>[],
  inside: Inside,
  phases: Phases,
  aria: Pick<SnapshotAria, 'snapshotNodes'>,
): boolean {
  if (!layers.some(({ note }) => note.wholeView && !note.blank)) return false
  const started = performance.now()
  let nodes: ReadonlyMap<AriaNodeJSON, Element>
  try {
    nodes = aria.snapshotNodes(json)
  } catch {
    phases.linking = performance.now() - started
    return true
  }
  phases.linking = performance.now() - started
  const marking = performance.now()
  const pending = [...json]
  while (pending.length) {
    const node = pending.pop()
    if (!node) continue
    const element = nodes.get(node)
    const layer =
      element &&
      layers.find(
        ({ held, note }) =>
          note.wholeView &&
          !note.blank &&
          held.some((root) => inside(root, element)),
      )
    if (layer) {
      node.onTop = layer.note.ref
    } else
      for (const child of node.children ?? [])
        if (typeof child !== 'string') pending.push(child)
  }
  phases.marks = performance.now() - marking
  return false
}

function middle(element: Element): Point | undefined {
  const box = element.getBoundingClientRect()
  const left = Math.max(0, box.left),
    right = Math.min(innerWidth, box.right)
  const top = Math.max(0, box.top),
    bottom = Math.min(innerHeight, box.bottom)
  return right > left && bottom > top
    ? { x: (left + right) / 2, y: (top + bottom) / 2 }
    : undefined
}
function viewPoints(): Point[] {
  return [
    [1 / 2, 1 / 2],
    [1 / 6, 1 / 6],
    [5 / 6, 1 / 6],
    [1 / 6, 5 / 6],
    [5 / 6, 5 / 6],
  ].map(([x = 0, y = 0]) => ({ x: x * innerWidth, y: y * innerHeight }))
}
/** Thin edge bars scroll clear and never describe the current view. */
function edgeBar(element: Element): boolean {
  if (element.matches(':modal')) return false
  const box = element.getBoundingClientRect()
  const centre =
    box.left <= innerWidth / 2 &&
    box.right > innerWidth / 2 &&
    box.top <= innerHeight / 2 &&
    box.bottom > innerHeight / 2
  return (
    getComputedStyle(element).position === 'sticky' ||
    (!centre && (box.width < innerWidth / 4 || box.height < innerHeight / 4))
  )
}
function stands(world: World, element: Element): boolean {
  if (
    !element.isConnected ||
    !takesPointer(element) ||
    !world.injected.elementState(element, 'visible', true).matches
  )
    return false
  let carried = false
  for (
    let at: Element | undefined = element;
    at;
    at = at.assignedSlot ?? parentOrHost(at)
  )
    carried ||= getComputedStyle(at).position === 'fixed' || inTopLayer(at)
  const point = middle(element)
  return carried && point !== undefined && hitChain(element, point) === null
}
