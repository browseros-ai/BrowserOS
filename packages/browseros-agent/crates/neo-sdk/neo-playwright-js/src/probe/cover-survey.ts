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
type Group = { faces: Face[]; held: Element[] }

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
  const { found, tested, main } = findFaces(
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
  const page = pageGuard(world, refs, inside, phases, main)
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
  const centre = { x: innerWidth / 2, y: innerHeight / 2 }
  for (const face of found.values())
    face.named = shownAs(world, output, face.named, centre)
  const groups = joinFaces(found, inside).map((faces) => ({
    faces,
    held: holdersOf(faces, inside),
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
  phases.naming = performance.now() - naming
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
  const found = new Map<Element, Face>()
  const tested: Tested[] = []
  for (const sample of samples) {
    const { point, hit } = sample
    const chain: Element[] = []
    for (let at = hit; at; at = at.assignedSlot ?? parentOrHost(at))
      chain.push(at)
    const top = chain.at(-1)
    if (top)
      sample.deepest = addFace(world, found, chain, top, point, false, main)
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
    if (!coverControl(world, element)) continue
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
      addFace(world, found, chain, coverRoot(subject, chain), point, true, main)
  }
  return { found, tested, main }
}

function addFace(
  world: World,
  found: Map<Element, Face>,
  chain: Element[],
  root: Element,
  point: Point,
  observed: boolean,
  main: Element | null,
): Element | undefined {
  // Reuse the click's existing shadow descent for reach; the raw document sample is a host.
  const path = coverPath(chain, root, point)
  const deepest = path[0]
  const element = coverFace(
    path.filter((element) => !pageElement(element, main)),
  )
  if (!element || edgeBar(element)) return deepest
  const known = found.get(element)
  if (known) {
    known.observed ||= observed
    if (observed) known.roots.add(root)
    return deepest
  }
  found.set(element, {
    element,
    named: whichDialog(world, chain, root, point),
    roots: new Set(observed ? [root] : []),
    observed,
  })
  return deepest
}

/** The expensive majority clause is lazy: ordinary pages never count the whole document. */
function pageGuard(
  world: World,
  refs: ReturnType<typeof lastSnapshotRefs>,
  inside: Inside,
  phases: Phases,
  main: Element | null,
): (element: Element) => boolean {
  let controls: Element[] | undefined
  const cached = new Map<Element, boolean>()
  return (element) => {
    if (
      DIALOGS.has(world.injected.utils.getAriaRole(element) ?? '') ||
      inTopLayer(element)
    )
      return false
    if (pageElement(element, main)) return true
    const known = cached.get(element)
    if (known !== undefined) return known
    const started = performance.now()
    controls ??= Array.from(refs.values(), ({ element }) => element).filter(
      (element) => coverControl(world, element),
    )
    const page =
      controls.filter((control) => inside(element, control)).length >
      controls.length / 2
    phases.count += performance.now() - started
    cached.set(element, page)
    return page
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

/** A joined portal owns the shared holder, including controls beside its fixed faces. */
function holdersOf(faces: Face[], inside: Inside): Element[] {
  const shared = [...new Set(faces.flatMap((face) => [...face.roots]))].filter(
    (root) => faces.filter((face) => face.roots.has(root)).length > 1,
  )
  const held = shared.length ? shared : faces.map((face) => face.element)
  for (const { named } of faces)
    if (!held.some((root) => inside(root, named))) held.push(named)
  return held
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
  const { faces, held } = group
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
      held
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
      loading: held.some(saysLoading),
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
