/**
 * Hosted Neo's clickable marking, page half. Rust finds the elements of this frame that Chromium
 * reports clickable without a pointer cursor (desktop Neo's rules over `DOMSnapshot`, read over
 * CDP; nothing runs in the page) and hands them to the `snapshot` request. For that one snapshot
 * this world's engine reads their computed cursor as `pointer`, so the engine's own ai rules
 * treat them as the click targets they are: it prints them with `cursor: 'pointer'`, keeps an
 * empty one (an image tile) that it would otherwise drop, and folds the elements inside one into
 * its target, exactly as it does for an element the page styles with a pointer cursor.
 *
 * The vendored engine reads every computed style through the frame window's `getComputedStyle`
 * (domUtils `getElementComputedStyle`, reached from `computeBox`). Like hideBrowserOSUi, this
 * overrides what the engine reads, never the page: the override is this isolated world's own
 * property on its own window, so the page's world, its scripts and its styles never see it. It
 * is installed only around the one snapshot call that has targets and removed in `finally`, so
 * every other read in that world, at any other time, goes to the world's own function. It
 * answers `pointer` only for that call's elements, a set filled just before the engine call and
 * emptied in the same `finally`, and for their descendants as CSS inheritance would: a
 * descendant whose own real cursor equals the real cursor of the target it sits in, so it set
 * none of its own, never one with a cursor of its own (a field's `text`). A target's real cursor
 * is often not `auto`: every `<label>` reads `default` from the browser's own stylesheet, and so
 * does everything on a page that sets `body { cursor: default }`. The engine relies on that
 * inheritance: a target that wraps a single control is folded into it, and the control keeps the
 * mark only because it inherited the cursor.
 *
 * Because the engine folds everything inside a pointer target into it, and the v1 text prints
 * what is inside an actionable node with its text left out, a target that holds a line of its
 * own (a control, a section, an element the page gives a pointer cursor, or another target) is
 * left out (`leafTargets`): marking may add refs and marks, never take a printed one away. Only
 * what the engine draws counts: what it does not draw prints nothing.
 */

/**
 * Desktop Neo's interactive tags and roles (browseros-core `observer/clickable.rs`), the list the
 * v1 renderer's actionable roles also follow; tests/pointer-targets.test.ts pins them to that
 * file.
 */
export const INTERACTIVE_TAGS: ReadonlySet<string> = new Set([
  'a',
  'button',
  'input',
  'select',
  'textarea',
  'details',
  'summary',
])
export const INTERACTIVE_ROLES: ReadonlySet<string> = new Set([
  'button',
  'link',
  'textbox',
  'checkbox',
  'radio',
  'combobox',
  'listbox',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'searchbox',
  'slider',
  'spinbutton',
  'switch',
  'tab',
  'treeitem',
])

/**
 * The roles the v1 text prints as a section line with its ref (prelude/snapshot-v1.ts `SECTION`,
 * without `iframe`, a tag here). Inside an actionable target the v1 walk drops a section whose
 * content is only text, and its ref with it; a frame's content is walked the same way.
 * tests/pointer-leaf-targets.test.ts pins the roles.
 */
export const SECTION_ROLES: ReadonlySet<string> = new Set([
  'banner',
  'navigation',
  'main',
  'complementary',
  'contentinfo',
  'search',
  'form',
  'region',
  'article',
])
/** The engine prints a frame as an `iframe` node whatever role it would compute. */
const FRAME_TAGS: ReadonlySet<string> = new Set(['iframe', 'frame'])

/**
 * What the guard asks the engine (both on `InjectedScript.utils`), so that it judges an element
 * as the snapshot does.
 */
export type EngineReads = {
  /** roleUtils `getAriaRole`: the role the snapshot computes. */
  roleOf: (element: Element) => string | null
  /** domUtils `isElementVisible`: the test an element must pass for the engine to give it a ref. */
  isVisible: (element: Element) => boolean
}

/**
 * The elements that read as `cursor: pointer`, only while one snapshot call runs, each with its
 * own real cursor, read before the override is in place: what an element inside it reads when it
 * set no cursor of its own.
 */
const pointerTargets = new Map<Element, string>()

/** The target an element sits inside, the nearest in the flat tree, learned during the same call. */
const nearestTargets = new Map<Element, Element | null>()

/**
 * The targets that hold no target of their own. Desktop's rules already leave out a listener that
 * only relays its children's clicks; a `tabindex` scroll pane or tab panel full of links, or a
 * `tabindex` list of clickable `div`s, is the same kind of container, and as a pointer target it
 * would fold its controls' marks, and an empty pointer element's ref, into itself. Called before
 * the snapshot, so every style it reads is real.
 */
export function leafTargets(
  targets: readonly Element[],
  engine: EngineReads,
): Element[] {
  const all = new Set(targets)
  return targets.filter((target) => !holdsTarget(target, all, engine))
}

/**
 * Whether a drawn element below `target` in the engine's own tree is a target of its own or
 * another of `targets`. The walk is the engine's (ariaSnapshot.ts `processElement`): a slot's
 * assigned nodes, else the light children no slot took and the shadow root's, then the
 * `aria-owns` elements; each element once, so an ownership cycle ends.
 *
 * Drawn is the engine's own `isElementVisible`, the test its refs depend on. A control or a
 * target with `display: none`, a zero-size box or `visibility: hidden` gets no ref and prints
 * nothing, so nothing of it needs protecting, and the wrapper is the only thing left to click (a
 * drop zone around its hidden file input, a row around a hidden listener `div`). Every element
 * the engine gives a ref passes this test, so the guard still sees every one that prints with a
 * ref, and marking still never removes one. A drawn `aria-hidden` control passes too: in ai mode
 * the engine prints it with its ref, so it counts. The walk goes on below an element that is not
 * drawn, as the engine does: a child can be drawn where its parent is not (`visibility: visible`
 * under `hidden`, content overflowing a zero-size box).
 */
function holdsTarget(
  target: Element,
  targets: ReadonlySet<Element>,
  engine: EngineReads,
): boolean {
  const seen = new Set<Element>([target])
  const pending: Element[] = []
  pushChildren(pending, seen, target)
  for (let element = pending.pop(); element; element = pending.pop()) {
    if (
      engine.isVisible(element) &&
      (targets.has(element) || isOwnTarget(element, engine))
    )
      return true
    pushChildren(pending, seen, element)
  }
  return false
}

function pushChildren(
  pending: Element[],
  seen: Set<Element>,
  element: Element,
): void {
  const push = (child: Element) => {
    if (seen.has(child)) return
    seen.add(child)
    pending.push(child)
  }
  // An HTML slot only: an SVG `<slot>` is an unknown element with no assignment. Assigned text
  // counts as an assignment too, so the fallback content is skipped, as the engine skips it.
  const assigned =
    typeof HTMLSlotElement !== 'undefined' && element instanceof HTMLSlotElement
      ? element.assignedNodes()
      : []
  if (assigned.length) {
    for (const node of assigned)
      if (node.nodeType === Node.ELEMENT_NODE) push(node as Element)
  } else {
    for (const child of element.children) if (!child.assignedSlot) push(child)
    const shadow = element.shadowRoot
    if (shadow) for (const child of shadow.children) push(child)
  }
  for (const id of element.getAttribute('aria-owns')?.split(/\s+/) ?? []) {
    const owned = id ? element.ownerDocument.getElementById(id) : null
    if (owned) push(owned)
  }
}

/**
 * Whether `element` prints a line of its own: a control by desktop's tags or by the engine's
 * role, a section or a frame, or an element with a pointer cursor. The role is the engine's own
 * reading (role tokens, implicit roles in context), so the guard sees what the snapshot sees.
 */
function isOwnTarget(element: Element, engine: EngineReads): boolean {
  const tag = element.localName
  if (INTERACTIVE_TAGS.has(tag) || FRAME_TAGS.has(tag)) return true
  const role = engine.roleOf(element)
  if (role !== null && (INTERACTIVE_ROLES.has(role) || SECTION_ROLES.has(role)))
    return true
  return getComputedStyle(element)?.cursor === 'pointer'
}

/** Runs `snapshot` with `targets` reading as `cursor: pointer` to the engine for its length. */
export function asPointerTargets<T>(
  targets: readonly Element[],
  snapshot: () => T,
): T {
  if (targets.length === 0) return snapshot()
  const read = globalThis.getComputedStyle
  if (typeof read !== 'function') return snapshot()
  const own = Object.getOwnPropertyDescriptor(globalThis, 'getComputedStyle')
  for (const target of targets) {
    const style: CSSStyleDeclaration | null = Reflect.apply(read, globalThis, [
      target,
    ])
    pointerTargets.set(target, style?.cursor ?? '')
  }
  Object.defineProperty(globalThis, 'getComputedStyle', {
    configurable: true,
    enumerable: own?.enumerable ?? true,
    writable: true,
    value: function getComputedStyle(
      this: unknown,
      element: Element,
      pseudo?: string | null,
    ): CSSStyleDeclaration {
      const style: CSSStyleDeclaration = Reflect.apply(read, this, [
        element,
        pseudo,
      ])
      if (pseudo) return style
      if (pointerTargets.has(element)) return pointerStyle(style)
      const target = nearestTarget(element)
      return target && style.cursor === pointerTargets.get(target)
        ? pointerStyle(style)
        : style
    },
  })
  try {
    return snapshot()
  } finally {
    // The world's own getComputedStyle again: outside this call nothing of marking is left.
    if (own) Object.defineProperty(globalThis, 'getComputedStyle', own)
    else Reflect.deleteProperty(globalThis, 'getComputedStyle')
    pointerTargets.clear()
    nearestTargets.clear()
  }
}

/**
 * The nearest of `element`'s ancestors in the flat tree, the tree CSS inheritance follows, that
 * is a target, or `null`. Walked once per element and remembered for the call, since the engine
 * asks for every element of the page. `leafTargets` leaves no drawn target inside another; one
 * the engine does not draw can sit inside a target, and is then the nearest for what it holds,
 * as CSS inheritance would have it.
 */
function nearestTarget(element: Element): Element | null {
  const chain: Element[] = []
  let found: Element | null = null
  for (let node = parentOrHost(element); node; node = parentOrHost(node)) {
    if (pointerTargets.has(node)) {
      found = node
      break
    }
    const known = nearestTargets.get(node)
    if (known !== undefined) {
      found = known
      break
    }
    chain.push(node)
  }
  nearestTargets.set(element, found)
  // Every element walked past sits under the same target: none of them is a target itself.
  for (const node of chain) nearestTargets.set(node, found)
  return found
}

/** `element`'s parent in the flat tree: its slot, its parent, or its shadow root's host. */
function parentOrHost(element: Element): Element | null {
  if (element.assignedSlot) return element.assignedSlot
  if (element.parentElement) return element.parentElement
  const parent = element.parentNode
  return typeof ShadowRoot !== 'undefined' && parent instanceof ShadowRoot
    ? parent.host
    : null
}

/** `style` as the engine reads it, with `cursor` as `pointer`; every other read is the real one. */
function pointerStyle(style: CSSStyleDeclaration): CSSStyleDeclaration {
  return new Proxy(style, {
    get(target, key) {
      if (key === 'cursor') return 'pointer'
      const value: unknown = Reflect.get(target, key)
      return typeof value === 'function' ? value.bind(target) : value
    },
  })
}
