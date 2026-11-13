/**
 * What this world's engine reads behind a shadow host. The browser gates a closed root behind
 * two accessors, `Element.prototype.shadowRoot` (from the host down: selectorEvaluator,
 * selectorUtils, roleSelectorEngine, roleUtils, ariaSnapshot) and `assignedSlot` on elements and
 * text (from a host's own child to the slot that shows it: ariaSnapshot, roleUtils, the hit
 * test), and Neo's probe runs in an isolated world whose DOM prototypes are its own, so the
 * getters installed here decide both for Neo alone: the page and its scripts keep the browser's
 * own answers. The rules, in this order:
 *
 * - A BrowserOS UI host (browseros-ui.ts) has no shadow root, so the engine reads the page as a
 *   browser without BrowserOS's bar shows it.
 * - A host whose closed root this world was handed answers with that root, and a child of that
 *   host answers with the slot of that root it is assigned to, so the engine enters the root as
 *   it enters an open one: in a snapshot, a locator and an action's own checks. Both must move
 *   together: with the root open and the slot still `null`, the engine reads a slotted child as
 *   one no slot took, which is hidden, and prints it nameless before the root's own content.
 *
 * A script in any world gets `null` from a closed host; the root is reachable only from inside
 * it, by `getRootNode()`. Rust finds elements inside closed roots in the DOM capture it already
 * makes for clickable marking (browseros-core `find_closed_shadow_seeds`, never a script in the
 * page) and hands them to a marked snapshot call, which feeds them here before the engine runs
 * (snapshot.ts). The world keeps those roots for the life of its document, so every later read in
 * it sees inside them; a read before the document's first marked snapshot does not. Under the
 * spec harness's conformance switch no snapshot is marked, so nothing is ever fed and a closed
 * root stays closed, as upstream reads it.
 */
import { isBrowserOSUi } from './browseros-ui'
import { ProbeError } from './world'

/**
 * The closed roots this world was handed, by host. A root never leaves its host, so an entry is
 * true for as long as the host lives; a weak map lets a removed host go.
 */
const closedRoots = new WeakMap<Element, ShadowRoot>()

/** This world's own `shadowRoot` getter, once `installShadowRoots` found it. */
let realShadowRoot: ((this: Element) => ShadowRoot | null) | undefined
let installed = false

type Getter<T, V> = (this: T) => V

/**
 * This world's own getter for `name`, wherever on `prototype`'s chain the DOM defines it;
 * `undefined` for a DOM without it.
 */
function getterOf<T, V>(
  prototype: object,
  name: string,
): Getter<T, V> | undefined {
  let owner: object | null = prototype
  while (owner) {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name)
    if (descriptor) {
      if (typeof descriptor.get !== 'function')
        throw new ProbeError(
          'internal',
          `this world's ${name} is not an accessor`,
        )
      return descriptor.get
    }
    owner = Object.getPrototypeOf(owner)
  }
  return undefined
}

/** Replaces `name` on `prototype` itself with `get`, as the DOM defines its accessors. */
function override<T, V>(prototype: object, name: string, get: Getter<T, V>) {
  Object.defineProperty(prototype, name, {
    configurable: true,
    enumerable: true,
    get,
  })
}

/**
 * Installs this world's `shadowRoot` and `assignedSlot` getters with the rules above. Called once
 * per world, before the engine is built; a DOM without shadow roots has nothing to read.
 */
export function installShadowRoots(): void {
  if (installed) return
  installed = true
  const read = getterOf<Element, ShadowRoot | null>(
    Element.prototype,
    'shadowRoot',
  )
  if (!read) return
  realShadowRoot = read
  override(Element.prototype, 'shadowRoot', function (this: Element) {
    if (isBrowserOSUi(this)) return null
    return Reflect.apply(read, this, []) ?? closedRoots.get(this) ?? null
  })
  for (const prototype of [Element.prototype, Text.prototype]) {
    const slot = getterOf<Element | Text, HTMLSlotElement | null>(
      prototype,
      'assignedSlot',
    )
    if (!slot) continue
    override(prototype, 'assignedSlot', function (this: Element | Text) {
      return Reflect.apply(slot, this, []) ?? closedSlotOf(this)
    })
  }
}

/**
 * Each handed-in root's slot for every node assigned to one, built on the first read that needs
 * it and dropped in a microtask: the engine reads `assignedSlot` for every node it visits, so a
 * host with a thousand slotted children costs one pass over its root's slots per snapshot, not
 * one per read, and no read after an await sees an assignment the page has since changed.
 */
const slotMaps = new Map<ShadowRoot, Map<Node, HTMLSlotElement>>()
let dropScheduled = false

/**
 * The slot that shows `node`, a child of a host whose closed root this world was handed: the
 * browser's own answer is `null` for a slot in a closed root. Read from that root's slots, whose
 * `assignedNodes()` answers in every world, so the assignment is the browser's own, named or
 * manual. Every other node keeps the browser's answer at the cost of one map lookup.
 */
function closedSlotOf(node: Element | Text): HTMLSlotElement | null {
  const host = node.parentElement
  const root = host ? closedRoots.get(host) : undefined
  if (!root) return null
  let slots = slotMaps.get(root)
  if (!slots) {
    slots = new Map()
    for (const slot of root.querySelectorAll('slot'))
      for (const assigned of slot.assignedNodes())
        if (!slots.has(assigned)) slots.set(assigned, slot)
    slotMaps.set(root, slots)
    if (!dropScheduled) {
      dropScheduled = true
      queueMicrotask(() => {
        dropScheduled = false
        slotMaps.clear()
      })
    }
  }
  return slots.get(node) ?? null
}

/**
 * Remembers, by its host, the root each of `seeds` sits in, when that is a shadow root the
 * browser does not already show this world (a closed one). Anything else (a seed that is not a
 * node, one in the document itself or in an open root, one in BrowserOS's own UI) is skipped, so
 * a seed can never make the engine read something it would not read through an open root. Rust
 * sends one seed per flat parent, so several seeds often name one root: the map keeps one entry.
 */
export function openClosedRoots(seeds: readonly unknown[]): void {
  const read = realShadowRoot
  if (!read) return
  for (const seed of seeds) {
    if (!(seed instanceof Node)) continue
    const root = seed.getRootNode()
    if (!(root instanceof ShadowRoot)) continue
    const host = root.host
    if (isBrowserOSUi(host)) continue
    if (Reflect.apply(read, host, []) !== root) closedRoots.set(host, root)
  }
}
