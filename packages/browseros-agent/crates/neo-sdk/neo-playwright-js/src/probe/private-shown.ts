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
 * times such a box took input since the first reading in this document. A value typed and gone
 * again between the camera's two readings (a single-page checkout that unmounts its card form)
 * leaves both readings clear while the capture between them holds it; a count that moved tells
 * the camera to drop that capture.
 *
 * A box inside a closed shadow root is out of reach of every script, so the camera finds the
 * root from its host, on demand (ruling R65):
 *
 * ```text
 *  typing reaches a closed root ─ a trusted input or beforeinput event whose first target takes
 *    no input itself: a closed boundary retargeted it to the host ─ counted at once, host waiting
 *  a reading finds the focus on such a host ─ host waiting
 *  the answer says `hosts` ─ Rust asks `privateHosts` (below) for them, each once,
 *    reads each host's closed root from the browser (DOM.describeNode) and resolves it into this
 *    world ─ `privateShown` again with those roots (`walk`) ─ kept in `walkedRoots`, read by
 *    `look`, a listener on each, and the focus followed into them to a nested host
 * ```
 *
 * The roots the camera found stay in a map of this module's own, never the engine's
 * (shadow-roots.ts), so a locator, an action or a snapshot reads nothing more for them. Input is
 * counted inside every closed root this world reads, the camera's or a marked snapshot's, by a
 * listener on each: at the document an event's path stops at a closed root's host.
 *
 * What it does not see, named in the contract: a value inside a closed root that came with no
 * event and never held the focus (a page's own script writes `.value =`, which fires nothing);
 * a value typed before the first reading in this document, whose box no longer holds the focus
 * at that reading (the listeners are added at that reading); a page that stops the event at the
 * window before this world's listener; and a box whose page takes the keys itself and cancels
 * them, so no input event fires, which is known only while it holds the focus at a reading. A key
 * is not listened for: one that types nothing fires no input event, so PageDown, an arrow key or
 * Tab never moves a reading (ruling R65's third condition).
 */
import { isBrowserOSUi } from './browseros-ui'
import { isPrivateField } from './private-field-values'
import type {
  PrivateHostsRequest,
  PrivateShownRequest,
  Value,
} from './protocol'
import type { World } from './world'

/** Input events on a private box painted in clear, since the first reading in this document. */
let inputs = 0
/** Hears every input and beforeinput event; set by the first reading in this document. */
let heard: ((event: Event) => void) | undefined
/**
 * The closed shadow roots that hear input already: at the document an event's path stops at a
 * closed root's host, so only a listener on the root sees the box. `look` adds one to every closed
 * root it enters, whichever way this world came to know it (the camera's walk, or a marked
 * snapshot through the engine's map). Open roots get none: their input reaches the document's
 * listener with the box as its first target, and a second listener would count it twice.
 */
const listened = new WeakSet<ShadowRoot>()

/**
 * The closed shadow roots the camera found in this document, by host: its own, so the camera
 * opens nothing to the engine. A root never leaves its host, and a weak map lets a removed host go.
 */
const walkedRoots = new WeakMap<Element, ShadowRoot>()

/**
 * Hosts pointed at (by an event or the focus) and not handed to Rust yet. Weak, so a host the page
 * removes before the next reading goes; each is pointed at once (`pointed`).
 */
const waiting = new Set<WeakRef<Element>>()
/** Every host ever pointed at, waiting or handed over: each is pointed at once. */
const pointed = new WeakSet<Element>()
/** Each waiting host's entry in `waiting`, so a host whose root is kept leaves it. */
const entries = new WeakMap<Element, WeakRef<Element>>()
/**
 * The hosts a trusted input event named. An input event fires on what takes input, so a first
 * target that takes none is a closed boundary's retargeting: a real host. Handed over first, never
 * dropped from the wait, and while one waits unasked the camera's reading is not clear (`named`).
 */
const certain = new WeakSet<Element>()
/**
 * The hosts done with: handed to Rust and read, or handed and not readable (a host the focus
 * pointed at, at once; one an input event named, after `NAMED_TRIES` readings), or gone from the
 * document. Their events are no longer counted here and they are never asked again.
 */
const handed = new WeakSet<Element>()
/**
 * For a host an input event named and not yet read: how many readings it was handed over at, and
 * the last one. Handed over again at each reading until its root is kept, the reading unknown
 * meanwhile, and as on main after the fifth, which bounds what a host that can never be read costs.
 */
const tries = new WeakMap<Element, { count: number; at: number }>()
const NAMED_TRIES = 5
/** The camera's readings in this document: each first question of one counts. */
let readings = 0

/**
 * The most hosts that wait at once. A host only the focus named is not added past it (nor marked
 * pointed, so a later reading can add it); a host an input event named always is.
 */
const WAITING_MOST = 64
/** How deep the focus is followed through known roots at one reading (rule 6's bound). */
const FOCUS_DEPTH = 32

/**
 * The tags HTML lets carry an author shadow root (`attachShadow` and declarative shadow DOM
 * refuse every other), beside custom elements. A root the browser makes itself (a video's
 * controls, a `details` element, a text box) is on other tags, so such elements are never asked.
 */
const SHADOW_HOSTS = new Set([
  'article',
  'aside',
  'blockquote',
  'body',
  'div',
  'footer',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'header',
  'main',
  'nav',
  'p',
  'section',
  'span',
])
const XHTML = 'http://www.w3.org/1999/xhtml'

export function privateShown(
  world: World,
  request: PrivateShownRequest,
  _root: Node | undefined,
  nodes: readonly unknown[],
): Value {
  const { document } = world.injected
  const view = document.defaultView
  if (!heard) {
    heard = (event: Event) => {
      const first = event.composedPath()[0]
      if (!(first instanceof Element)) return
      // A closed root this world knows (a marked snapshot opened it after the last reading) but
      // no reading has entered yet has no listener. The event has not reached the root, so one
      // added now still hears it.
      const known = knownRoot(first)
      if (known) listen(known)
      if (event.type === 'input' && paintedInClear(first, view)) inputs += 1
      // A trusted input event whose first target takes no input came out of a closed root this
      // world does not know: counted at once, so the picture around it is dropped whether or not
      // the root is then found, until its host is handed over. beforeinput too, for a page that
      // cancels it and writes the value itself.
      if (!event.isTrusted || !unknownHost(first) || handed.has(first)) return
      inputs += 1
      point(first, true)
    }
    // Capture phase on the document, in this isolated world: the page can neither see nor stop
    // it below the window. The event's first target is the box itself, inside an open shadow
    // root too, and a closed root's host otherwise.
    document.addEventListener('beforeinput', heard, true)
    document.addEventListener('input', heard, true)
  }
  if (request.walk === true) keep(nodes)
  else {
    readings += 1
    sweep()
  }
  pointFocus(document)
  const { shown, boxes } = look(document)
  // `hosts`: some wait for the camera's walk; `named`: one of them an input event named, which
  // the camera must read before the reading may be clear.
  const hosts = waiting.size > 0 ? { hosts: true } : {}
  const named = namedWaiting() ? { named: true } : {}
  return { kind: 'value', value: { shown, inputs, boxes, ...hosts, ...named } }
}

/**
 * `privateHosts`: up to `limit` of the hosts waiting, the ones an input event named first, handed
 * to Rust by deep serialization (a by-handle request). A host the focus pointed at is done with as
 * it goes, so it is asked once whether or not a root comes back. One an input event named is
 * handed over once a reading and stays waiting until its root is kept (`keep`) or its tries run
 * out (`sweep`).
 */
export function privateHosts(request: PrivateHostsRequest): Element[] {
  const hosts: Element[] = []
  for (const first of [true, false]) {
    for (const ref of waiting) {
      if (hosts.length >= request.limit) return hosts
      const host = ref.deref()
      if (!host || !host.isConnected) {
        drop(ref, host)
        continue
      }
      if (certain.has(host) !== first) continue
      if (first) {
        const tried = tries.get(host) ?? { count: 0, at: 0 }
        if (tried.at === readings) continue
        tries.set(host, { count: tried.count + 1, at: readings })
      } else {
        drop(ref, host)
      }
      hosts.push(host)
    }
  }
  return hosts
}

/**
 * At each reading: a host gone from the document is dropped, so it stops counting and costs no
 * picture; one an input event named whose fifth reading has passed is as on main from now on.
 */
function sweep(): void {
  for (const ref of waiting) {
    const host = ref.deref()
    const spent = host && (tries.get(host)?.count ?? 0) >= NAMED_TRIES
    if (!host || !host.isConnected || spent) drop(ref, host)
  }
}

/** Takes a host out of the wait, done with. */
function drop(ref: WeakRef<Element>, host: Element | undefined): void {
  waiting.delete(ref)
  if (host) handed.add(host)
}

/** Whether a host an input event named waits unread, in the document. */
function namedWaiting(): boolean {
  for (const ref of waiting) {
    const host = ref.deref()
    if (host?.isConnected && certain.has(host)) return true
  }
  return false
}

/**
 * Keeps each closed root Rust resolved into this world from a host's own description, so `look`
 * reads it and gives it its listener. Only a root the browser lists as `closed` reaches here (a
 * root the browser made itself also reads `mode: 'closed'` from script, so Rust decides), and
 * BrowserOS's own UI is never read.
 */
function keep(roots: readonly unknown[]): void {
  for (const root of roots) {
    if (!(root instanceof ShadowRoot) || root.mode !== 'closed') continue
    const host = root.host
    if (isBrowserOSUi(host) || walkedRoots.has(host)) continue
    walkedRoots.set(host, root)
    // Read: out of the wait for good, its input counted by the root's own listener from now on.
    const entry = entries.get(host)
    if (entry) drop(entry, host)
  }
}

/** The shadow root this world can read on `host`: the camera's, or the engine's getter's. */
function knownRoot(host: Element): ShadowRoot | null {
  return walkedRoots.get(host) ?? host.shadowRoot
}

/**
 * Whether `element` may stand for a box inside a closed root this world does not know: it takes
 * no input itself (not an input, a textarea, a select or an editable region; measured, the body
 * of a document in design mode, which its input events name, is editable), it may carry an author
 * shadow root, and no root of it is known here.
 */
function unknownHost(element: Element): boolean {
  if (element.namespaceURI !== XHTML || isBrowserOSUi(element)) return false
  const name = element.localName
  if (!SHADOW_HOSTS.has(name) && !name.includes('-')) return false
  if (Reflect.get(element, 'isContentEditable') === true) return false
  return knownRoot(element) === null
}

function point(host: Element, byInput = false): void {
  if (byInput) certain.add(host)
  if (pointed.has(host)) return
  if (!byInput && waiting.size >= WAITING_MOST) return
  pointed.add(host)
  const entry = new WeakRef(host)
  entries.set(host, entry)
  waiting.add(entry)
}

/** Gives a closed root the input and beforeinput listener, once (`listened`). */
function listen(root: ShadowRoot): void {
  if (root.mode !== 'closed' || listened.has(root) || !heard) return
  listened.add(root)
  // Inside the root, a box's event starts at the box, and a nested closed root's at its host.
  root.addEventListener('beforeinput', heard, true)
  root.addEventListener('input', heard, true)
}

/**
 * The element that holds the focus at this reading, followed from root to root through the ones
 * this world knows: a host there whose root is unknown is pointed at, so a value typed before
 * the listeners existed, or into a root nested in a found one, is read once its box has the
 * focus. `:focus` on the holder tells a focused element from a document with no focus, whose
 * active element is its body.
 */
function pointFocus(document: Document): void {
  let holder = document.activeElement
  for (let depth = 0; holder && depth < FOCUS_DEPTH; depth += 1) {
    const inner = knownRoot(holder)?.activeElement
    if (!inner) break
    holder = inner
  }
  if (holder?.matches(':focus') && unknownHost(holder)) point(holder)
}

/**
 * Whether a private box shows its value, and whether the document holds a private box at all.
 * Every closed root it enters gets its listener (`listen`).
 */
function look(document: Document): { shown: boolean; boxes: boolean } {
  const view = document.defaultView
  const roots: (Document | ShadowRoot)[] = [document]
  let boxes = false
  for (const root of roots) {
    if (root instanceof ShadowRoot) listen(root)
    for (const field of root.querySelectorAll('input, textarea')) {
      if (!isPrivateField(field, { declared: true })) continue
      boxes = true
      if (paintedInClear(field, view)) return { shown: true, boxes }
    }
    for (const host of root.querySelectorAll('*')) {
      const shadow = knownRoot(host)
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
