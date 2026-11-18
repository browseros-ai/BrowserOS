/**
 * A whole page surveys covers in the same isolated-world request as its snapshot tree.
 * The click module owns hits, faces, control eligibility and names. This module owns grouping,
 * reach and weak per-document memory; child frames and pressing are outside its scope.
 */
import type { AriaSnapshotJSON } from '../prelude/vendored'
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
import type { Point } from './protocol'
import { lastSnapshotNodes, lastSnapshotRefs, type World } from './world'

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

/** Page facts are masked here; the shared Rust formatter owns their public words. */
export type SurveyLayer = {
  role: string
  name: string
  ref: string
  blank: boolean
  loading: boolean
  wholeView: boolean
  controls: ReturnType<typeof controlsOf>
  firstWords?: string
}

type Phases = {
  hits: number
  count: number
  naming: number
  linking: number
  marks: number
  stopped: boolean
}
type Layer = { order: number; held: Element[]; note: SurveyLayer }
type Group = { faces: Face[]; held: Element[]; controlsFrom: Element[] }

export function surveyCovers(
  world: World,
  output: FailureOutput,
  json: AriaSnapshotJSON,
): {
  layers: SurveyLayer[]
  tested: number
  elapsed: number
  marksSkipped: boolean
  phases: Phases
} {
  const { injected } = world
  const document = injected.document
  const refs = lastSnapshotRefs(injected)
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
    refs,
    samples,
    started,
    phases,
  )
  phases.hits = performance.now() - started
  // The engine's scope walk crosses hosts but not slots. Layer membership follows the
  // rendered flat tree so a panel owns its slotted controls and content as well.
  const inside: Inside = (root, element) => {
    for (
      let at: Element | undefined = element;
      at;
      at = at.assignedSlot ?? parentOrHost(at)
    )
      if (at === root) return true
    return false
  }
  const page = pageGuard(world, refs, phases, main)
  const resolving = performance.now()
  const found = facesFromHits(world, hits, page)
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
    return {
      layers: [],
      tested: tested.length,
      elapsed: performance.now() - started,
      marksSkipped: false,
      phases,
    }
  }
  const naming = performance.now()
  // Isolate guard time spent while naming: a carrier's guard belongs to count, not both phases.
  const countedBeforeNaming = phases.count
  phases.count = 0
  const centre = { x: innerWidth / 2, y: innerHeight / 2 }
  for (const face of found.values())
    face.named = shownAs(world, output, face.named, centre)
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
  const marksSkipped = markBlocks(world, json, layers, inside, phases)
  return {
    layers: layers.map(({ note }) => note),
    tested: tested.length,
    elapsed: performance.now() - started,
    marksSkipped,
    phases,
  }
}

/** Samples seed candidates; only a foreign hit on an outside control makes one a layer. */
function findFaces(
  world: World,
  refs: ReturnType<typeof lastSnapshotRefs>,
  samples: Sample[],
  started: number,
  phases: Phases,
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
  // The engine's ref map has tree order. Filtering and testing in one pass means a budget
  // stop leaves a tested prefix, never a selected list whose tail was silently untested.
  for (const { element } of refs.values()) {
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
      ? kindOf(world, subject, chain, point, false).is
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
    const named = whichDialog(world, hit.chain, root, hit.point)
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
  world: World,
  refs: ReturnType<typeof lastSnapshotRefs>,
  phases: Phases,
  main: Element | null,
): (element: Element) => boolean {
  let controls: Element[] | undefined
  const cached = new Map<Element, boolean>()
  // Dialog ancestry shields its controls; a menu or listbox does not stop its app shell
  // from being the page. Keep the broader naming exemption separate from that test.
  const meanings = new Map<Element, { dialog: boolean; exempt: boolean }>()
  const meaning = (element: Element) => {
    let known = meanings.get(element)
    if (!known) {
      const role = world.injected.utils.getAriaRole(element) ?? ''
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
      controls ??= Array.from(refs.values(), ({ element }) => element).filter(
        (element) => coverControl(element),
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

function describeLayer(
  world: World,
  output: FailureOutput,
  group: Group,
  groups: Group[],
  remembered: Remembered[],
  next: Remembered[],
  tested: Tested[],
  samples: Sample[],
  inside: Inside,
): Layer | undefined {
  const { faces, held, controlsFrom } = group
  const face =
    faces.find((face) => isBlank(world, output, face.element)) ?? faces[0]
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
        .flatMap((root) => controlsOf(world, output, root))
        .map((control) => [control.ref, control]),
    ).values(),
  )
  const words = output.tree(named, () =>
    renderedText(named).trim().replace(/\s+/g, ' '),
  )
  const blank =
    !inClosedPart(named) &&
    controls.length === 0 &&
    !words &&
    isBlank(world, output, named)
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
      role: controlRole(world, named),
      name: nameOf(world, output, named),
      ref: world.refs.forElement(named, world.seq),
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
  world: World,
  json: AriaSnapshotJSON,
  layers: Layer[],
  inside: Inside,
  phases: Phases,
): boolean {
  if (!layers.some(({ note }) => note.wholeView && !note.blank)) return false
  const started = performance.now()
  let linked: ReturnType<typeof lastSnapshotNodes>
  try {
    linked = lastSnapshotNodes(world.injected, json)
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
    const element = linked.nodes.get(node)
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
