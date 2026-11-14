/**
 * `privateShown`: whether this frame's document holds a box that the mask treats as private,
 * that is not empty, and that the browser paints in clear, such as a card number or a code
 * the page declares, typed by a script or by the owner. Neo's camera asks it in every frame of
 * the tab before and after a picture, and stores no picture while one does (protected-fill
 * ruling R45). A password box paints bullets (`-webkit-text-security`), so a sign-in page keeps
 * its pictures. Read outside any mask: the box's own value only decides empty or not, and
 * nothing of it leaves this world.
 *
 * It also answers whether the document holds a private box at all, empty or not (`boxes`):
 * only such a document, or one whose private box took input, is part of the camera's reading,
 * so an ad frame's coming and going changes nothing (ruling R54). And it answers how many
 * times such a box took input since the first reading in this document. A value typed and gone again between the camera's two readings (a single-page
 * checkout that unmounts its card form) leaves both readings clear while the capture between
 * them holds it; a count that moved tells the camera to drop that capture.
 *
 * A box inside a closed shadow root is out of reach of every script; this world reaches it
 * through the root's own elements, which Rust finds in Chromium's capture of the document. The
 * first reading in a document offers the camera one walk of its closed roots (`walkable`, with
 * the document's size), and no later reading does, whether Rust took it or not: a walk that found
 * nothing, failed or was skipped is never tried again at each reading. When Rust takes it, it
 * asks again with those elements (`walk`), and this world keeps their roots in a map of its own,
 * never the engine's (shadow-roots.ts), so a locator, an action or a snapshot reads nothing more
 * for it. A root the page attaches after that walk is seen once a marked snapshot opens it to
 * this world, a named limit. Input is counted inside the roots the camera walked too, by a
 * listener on each: at the document an event's path stops at a closed root's host. A value a
 * page's own script writes with `.value =` fires no input event, so one shown and cleared again
 * between the two readings is not counted.
 */
import { isBrowserOSUi } from './browseros-ui'
import { isPrivateField } from './private-field-values'
import type { PrivateShownRequest, Value } from './protocol'
import type { World } from './world'

/** Input events on a private box painted in clear, since the first reading in this document. */
let inputs = 0
/** Counts one input event on a private box painted in clear; set by the first reading. */
let countInput: ((event: Event) => void) | undefined

/**
 * The closed shadow roots the camera walked in this document, by host: its own, so a walk opens
 * nothing to the engine. A root never leaves its host, and a weak map lets a removed host go.
 */
const walkedRoots = new WeakMap<Element, ShadowRoot>()
/** Whether this document has offered the camera its one walk of its closed roots. */
let offered = false

export function privateShown(
  world: World,
  request: PrivateShownRequest,
  _root: Node | undefined,
  nodes: readonly unknown[],
): Value {
  const { document } = world.injected
  const view = document.defaultView
  if (!countInput) {
    const counted = (event: Event) => {
      const target = event.composedPath()[0]
      if (target instanceof Element && paintedInClear(target, view)) inputs += 1
    }
    countInput = counted
    // Capture phase on the document, in this isolated world: the page can neither see nor stop
    // it. The event's first target is the box itself, inside an open shadow root too.
    document.addEventListener('input', counted, true)
  }
  if (request.walk === true) walk(nodes, countInput)
  const { shown, boxes } = look(document, view)
  // The one offer goes with the document's size: Rust walks none whose capture could not finish
  // within the camera's budget, as a capture once sent cannot be called back.
  const offer = offered
    ? {}
    : {
        walkable: true,
        elements: document.getElementsByTagName('*').length,
      }
  offered = true
  return { kind: 'value', value: { shown, inputs, boxes, ...offer } }
}

/**
 * Keeps the closed root each of `seeds` sits in, with a listener that counts input inside it.
 * A root this world already reads through `shadowRoot` (an open one, or one a marked snapshot
 * opened) is walked by `look` anyway, and BrowserOS's own UI is never read. Rust sends one seed
 * per flat parent, so several seeds often name one root: the map keeps one entry.
 */
function walk(seeds: readonly unknown[], count: (event: Event) => void): void {
  for (const seed of seeds) {
    if (!(seed instanceof Node)) continue
    const root = seed.getRootNode()
    if (!(root instanceof ShadowRoot)) continue
    const host = root.host
    if (
      isBrowserOSUi(host) ||
      host.shadowRoot === root ||
      walkedRoots.has(host)
    )
      continue
    walkedRoots.set(host, root)
    // Inside the root, the event's path still starts at the box itself.
    root.addEventListener('input', count, true)
  }
}

/** Whether a private box shows its value, and whether the document holds a private box at all. */
function look(
  document: Document,
  view: Window | null,
): { shown: boolean; boxes: boolean } {
  const roots: (Document | ShadowRoot)[] = [document]
  let boxes = false
  for (const root of roots) {
    for (const field of root.querySelectorAll('input, textarea')) {
      if (!isPrivateField(field, { declared: true })) continue
      boxes = true
      if (paintedInClear(field, view)) return { shown: true, boxes }
    }
    for (const host of root.querySelectorAll('*')) {
      const shadow = host.shadowRoot ?? walkedRoots.get(host)
      if (shadow) roots.push(shadow)
    }
  }
  return { shown: false, boxes }
}

/** A private box that holds something and shows it as typed, where it renders at all. */
function paintedInClear(field: Element, view: Window | null): boolean {
  // The page's own declarations always count here, whatever scope a caller puts around it.
  if (!isPrivateField(field, { declared: true })) return false
  const value: unknown = Reflect.get(field, 'value')
  if (typeof value !== 'string' || value === '') return false
  const security = view
    ?.getComputedStyle(field)
    .getPropertyValue('-webkit-text-security')
  if (security && security !== 'none') return false
  // A box that does not render paints nothing; one off screen still counts, as a full-page
  // picture can show it.
  const visible: unknown = Reflect.get(field, 'checkVisibility')
  return (
    typeof visible !== 'function' ||
    Reflect.apply(visible, field, [
      { visibilityProperty: true, opacityProperty: true },
    ]) === true
  )
}
