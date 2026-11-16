/** Annotates only browser-declared inert subtrees, in the snapshot's own isolated-world read. */
import type { AriaNodeJSON, AriaSnapshotJSON } from '../prelude/vendored'
import type { World } from './world'
import { lastSnapshotRefs } from './world'

/** Cross shadow hosts as native inertness does; never inspect another document. */
function parent(element: Element): Element | null {
  // A slotted control belongs to the slot's composed ancestry. Its light-DOM parent may
  // be inert while the slot is inside a live modal, or the other way around.
  if (element.assignedSlot) return element.assignedSlot
  if (element.parentElement) return element.parentElement
  const root = element.getRootNode()
  return root instanceof ShadowRoot ? root.host : null
}

export function markInert(world: World, json: AriaSnapshotJSON): void {
  // Selector support or an unfamiliar DOM shape must never turn a read into a refusal or
  // partially hide a live subtree. Commit the marks only after the complete read succeeds.
  try {
    for (const node of inertNodes(world, json)) node.inert = true
  } catch {
    // Listing too much is the conservative answer when native inertness is uncertain.
  }
}

function inertNodes(world: World, json: AriaSnapshotJSON) {
  const document = world.injected.document
  // A closed/open shadow root may hold a modal the document selector cannot see. Do not
  // guess at it: shadow-contained modal content remains listed, including its live controls.
  const modals = [...document.querySelectorAll('dialog:modal')]
  const refs = lastSnapshotRefs(world.injected)
  if (modals.length === 0 && !hasInertAttribute(document, refs)) return []
  const modalAncestors = new Set<Element>()
  for (const modal of modals)
    for (let ancestor = parent(modal); ancestor; ancestor = parent(ancestor))
      modalAncestors.add(ancestor)
  // Cache composed ancestors too: large pages share most of those paths. Keep modal and
  // attribute ancestry separate because a native modal escapes even an inert ancestor.
  const ancestry = new WeakMap<
    Element,
    { modal: boolean; attribute: boolean }
  >()
  const isInert = (element: Element): boolean => {
    const pending: Element[] = []
    let state = { modal: false, attribute: false }
    for (
      let ancestor: Element | null = element;
      ancestor;
      ancestor = parent(ancestor)
    ) {
      const known = ancestry.get(ancestor)
      if (known) {
        state = known
        break
      }
      pending.push(ancestor)
    }
    for (const ancestor of pending.reverse()) {
      state = {
        modal: state.modal || ancestor.matches('dialog:modal'),
        attribute: state.attribute || ancestor.hasAttribute('inert'),
      }
      ancestry.set(ancestor, state)
    }
    return !state.modal && (state.attribute || modals.length > 0)
  }
  const protectedNodes = liveAncestors(json, refs, isInert)
  const marked: AriaNodeJSON[] = []
  const pending = [...json]
  while (pending.length) {
    const node = pending.pop()
    if (!node) continue
    const element = node.ref ? refs.get(node.ref)?.element : undefined
    // A modal escapes its ancestors' inertness. Coalescing that ancestor would hide live
    // descendants; leave the wrapper and mark only its independently inert children.
    if (
      element &&
      !protectedNodes.has(node) &&
      !modalAncestors.has(element) &&
      isInert(element)
    ) {
      marked.push(node)
      continue
    }
    for (const child of node.children ?? [])
      if (typeof child !== 'string') pending.push(child)
  }
  return marked
}

/** Find attribute triggers in roots the engine already read, without another document walk. */
function hasInertAttribute(
  document: Document,
  refs: ReturnType<typeof lastSnapshotRefs>,
): boolean {
  if (document.querySelector('[inert]')) return true
  const seen = new Set<ShadowRoot>()
  for (const { element } of refs.values()) {
    let root = element.getRootNode()
    while (root instanceof ShadowRoot && !seen.has(root)) {
      seen.add(root)
      if (root.querySelector('[inert]')) return true
      root = root.host.getRootNode()
    }
  }
  return false
}

/** Preserve live descendants the engine already exposed, including shadow-contained modals. */
function liveAncestors(
  json: AriaSnapshotJSON,
  refs: ReturnType<typeof lastSnapshotRefs>,
  isInert: (element: Element) => boolean,
): Set<AriaNodeJSON> {
  const protectedNodes = new Set<AriaNodeJSON>()
  const visited: AriaNodeJSON[] = []
  const scan = [...json]
  while (scan.length) {
    const node = scan.pop()
    if (!node) continue
    visited.push(node)
    for (const child of node.children ?? [])
      if (typeof child !== 'string') scan.push(child)
  }
  for (const node of visited.reverse()) {
    const element = node.ref ? refs.get(node.ref)?.element : undefined
    if (
      // A node without a ref has no element to prove inert. In particular a shadow modal
      // can expose only prose or keyboard controls: never hide it with its inert host.
      !element ||
      // The engine may flatten a ref-less modal into its host's text, even with its role
      // overridden. A known shadow host is uncertain; preserve it without searching for modals.
      element.shadowRoot !== null ||
      !isInert(element) ||
      node.children?.some(
        (child) => typeof child !== 'string' && protectedNodes.has(child),
      )
    )
      protectedNodes.add(node)
  }
  return protectedNodes
}
