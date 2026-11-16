/**
 * A whole page's snapshot surveys controls in the same isolated-world request as its tree.
 * The click module owns every hit and naming rule; this module owns grouping, reach and weak
 * per-document memory. It never reads a child frame or presses through a cover.
 */

import type { AriaSnapshotJSON } from '../prelude/vendored'
import {
  controlsOf,
  coverControl,
  coverFace,
  coverPath,
  coverRoot,
  hitChain,
  inClosedPart,
  inTopLayer,
  isBlank,
  kindOf,
  nameOf,
  pageElement,
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

type Face = { element: Element; named: Element; roots: Set<Element> }
type Remembered = {
  face: WeakRef<Element>
  named: WeakRef<Element>
  order: number
}
// A document's retained faces must not keep removed DOM alive. Navigation drops the key too.
const memory = new WeakMap<Document, Remembered[]>()

/** Facts are masked before leaving the page; Rust owns their public wording. */
export type SurveyLayer = {
  role: string
  name: string
  ref: string
  blank: boolean
  loading: boolean
  wholeView: boolean
  controls: ReturnType<typeof controlsOf>
  firstWords?: string
  firstRefs: string[]
}

/** No candidate count decides whether a layer is named; it is diagnostic evidence only. */
export function surveyCovers(
  world: World,
  output: FailureOutput,
  json: AriaSnapshotJSON,
): {
  layers: SurveyLayer[]
  tested: number
  elapsed: number
  marksSkipped: boolean
} {
  const started = performance.now()
  const { injected } = world
  const document = injected.document
  const engine = lastSnapshotRefs(injected)
  let linked: ReturnType<typeof lastSnapshotNodes> | undefined
  try {
    linked = lastSnapshotNodes(injected, json)
  } catch {
    /* The line still prints; no block moves on an unfamiliar engine shape. */
  }
  const elements =
    linked?.elements ??
    Array.from(new Set(Array.from(engine.values(), ({ element }) => element)))
  const controls = elements.filter((element) => coverControl(world, element))
  const inside = (root: Element, element: Element) =>
    injected.utils.isInsideScope(root, element)
  const page = (element: Element) => {
    // Dialogs and notices remain layers on a small page even when most controls belong to them.
    if (
      DIALOGS.has(injected.utils.getAriaRole(element) ?? '') ||
      inTopLayer(element)
    )
      return false
    if (pageElement(element)) return true
    return (
      controls.filter((control) => inside(element, control)).length >
      controls.length / 2
    )
  }
  const found = new Map<Element, Face>()
  const tested: { element: Element; clear: boolean }[] = []
  for (const control of controls) {
    if (
      tested.length >= MAX_CONTROLS ||
      performance.now() - started >= BUDGET_MS
    )
      break
    // This is the main frame. The engine's visibility API requires its actual frame visibility.
    if (
      !injected.elementState(control, 'visible', true).matches ||
      !injected.elementState(control, 'enabled', true).matches
    )
      continue
    const point = middle(control)
    if (!point) continue
    const subject = injected.retarget(control, 'button-link') ?? control
    const chain = hitChain(subject, point)
    const kind = chain
      ? kindOf(world, subject, chain, point, false).is
      : 'nothing'
    const covered = kind === 'page' || kind === 'frame'
    tested.push({ element: control, clear: !covered })
    if (!chain || !covered) continue
    const root = coverRoot(subject, chain)
    const face = coverFace(
      coverPath(chain, root, point).filter((element) => !pageElement(element)),
    )
    if (!face || edgeBar(face) || page(face)) continue
    const known = found.get(face)
    if (known) {
      if (!page(root)) known.roots.add(root)
    } else {
      found.set(face, {
        element: face,
        named: whichDialog(world, chain, root, point),
        roots: new Set(page(root) ? [] : [root]),
      })
    }
  }
  const remembered = memory.get(document) ?? []
  for (const entry of remembered) {
    const face = entry.face.deref()
    const named = entry.named.deref()
    if (!face || !stands(world, face) || page(face) || found.has(face)) continue
    found.set(face, {
      element: face,
      named:
        named?.isConnected &&
        injected.elementState(named, 'visible', true).matches
          ? named
          : face,
      roots: new Set(),
    })
  }
  const centre = { x: innerWidth / 2, y: innerHeight / 2 }
  // The shared shown-as rule chooses the nearest dialog at the viewport centre for a backdrop.
  for (const face of found.values())
    face.named = shownAs(world, output, face.named, centre)
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
    if (!joins.length) groups.push([face])
    else {
      const first = joins[0]
      if (!first) continue
      first.push(face)
      for (const joined of joins.slice(1)) {
        first.push(...joined)
        groups.splice(groups.indexOf(joined), 1)
      }
    }
  }
  const next: Remembered[] = []
  const notes = groups
    .map((group) => {
      // Prefer the backdrop as the remembered face: when its dialog closes the blank face stays.
      const face =
        group.find((face) => isBlank(world, output, face.element)) ?? group[0]
      if (!face) return undefined
      const previous = remembered.find((entry) =>
        group.some((face) => entry.face.deref() === face.element),
      )
      const order =
        previous?.order ??
        remembered.reduce((last, entry) => Math.max(last, entry.order + 1), 0) +
          next.length
      const named = face.named
      const faces = group.map((face) => face.element)
      const sharedRoots = Array.from(face.roots).filter(
        (root) => group.filter((face) => face.roots.has(root)).length > 1,
      )
      const held = sharedRoots.length ? sharedRoots : faces
      if (named !== face.element && !held.some((root) => inside(root, named)))
        held.push(named)
      const belongs = (element: Element) =>
        held.some((root) => inside(root, element))
      const allLayer = (element: Element) =>
        groups.some((group) =>
          group.some(
            (face) =>
              inside(face.element, element) || inside(face.named, element),
          ),
        )
      const wholeView =
        viewPoints().every((point) =>
          held.some((root) => hitChain(root, point) === null),
        ) &&
        tested.every(({ element, clear }) => !clear || allLayer(element)) &&
        tested.some(({ element, clear }) => !clear && !belongs(element))
      const own = Array.from(
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
        own.length === 0 &&
        !words &&
        isBlank(world, output, named)
      const reference = world.refs.forElement(named, world.seq)
      next.push({
        face: new WeakRef(face.element),
        named: new WeakRef(named),
        order,
      })
      return {
        order,
        held,
        note: {
          role:
            named.localName === 'iframe' || named.localName === 'frame'
              ? 'iframe'
              : (injected.utils.getAriaRole(named) ?? 'generic'),
          name: nameOf(world, output, named),
          ref: reference,
          blank,
          loading: held.some(saysLoading),
          wholeView,
          controls: own,
          ...(words
            ? { firstWords: Array.from(words).slice(0, 60).join('') }
            : {}),
          firstRefs:
            wholeView && !blank
              ? Array.from(engine, ([ref, { element }]) =>
                  belongs(element) ? ref : '',
                ).filter(Boolean)
              : [],
        },
      }
    })
    .filter((note) => note !== undefined)
  memory.set(document, next)
  notes.sort(
    (a, b) =>
      Number(b.note.wholeView) - Number(a.note.wholeView) || a.order - b.order,
  )
  const kept = notes.slice(0, MAX_LAYERS)
  // Mark the topmost nodes inside the holder in the request's original tree, ref or no ref.
  // A holder absent from the engine has only its retained descendant nodes to move.
  if (linked) {
    const pending = [...json]
    while (pending.length) {
      const node = pending.pop()
      if (!node) continue
      const element = linked.nodes.get(node)
      const layer =
        element &&
        kept.find(
          ({ held, note }) =>
            note.wholeView &&
            !note.blank &&
            held.some((root) => inside(root, element)),
        )
      if (layer) node.onTop = layer.note.ref
      else
        for (const child of node.children ?? [])
          if (typeof child !== 'string') pending.push(child)
    }
  }
  return {
    layers: kept.map(({ note }) => note),
    tested: tested.length,
    elapsed: performance.now() - started,
    marksSkipped: linked === undefined,
  }
}

/** A point in the drawn part of a control; a point outside the viewport cannot be tested. */
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

/** Thin fixed bars at an edge scroll clear; unlike a modal they do not describe the current view. */
function edgeBar(element: Element): boolean {
  if (inTopLayer(element)) return false
  const box = element.getBoundingClientRect()
  const atCentre =
    box.left <= innerWidth / 2 &&
    box.right > innerWidth / 2 &&
    box.top <= innerHeight / 2 &&
    box.bottom > innerHeight / 2
  return (
    getComputedStyle(element).position === 'sticky' ||
    (!atCentre && (box.width < innerWidth / 4 || box.height < innerHeight / 4))
  )
}

/** Retention uses the same hit rule at the face, never the centre of a pointer-free carrier. */
function stands(world: World, element: Element): boolean {
  if (
    !element.isConnected ||
    !takesPointer(element) ||
    !world.injected.elementState(element, 'visible', true).matches
  )
    return false
  let carried = false
  for (let at: Element | null = element; at; ) {
    carried ||= getComputedStyle(at).position === 'fixed' || inTopLayer(at)
    at =
      at.assignedSlot ??
      at.parentElement ??
      (at.getRootNode() instanceof ShadowRoot
        ? (at.getRootNode() as ShadowRoot).host
        : null)
  }
  const point = middle(element)
  return carried && point !== undefined && hitChain(element, point) === null
}
