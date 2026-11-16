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
  if (modals.length === 0 && !document.querySelector('[inert]')) return []
  const modalAncestors = new Set<Element>()
  for (const modal of modals)
    for (let ancestor = parent(modal); ancestor; ancestor = parent(ancestor))
      modalAncestors.add(ancestor)
  const inert = new WeakMap<Element, boolean>()
  const isInert = (element: Element): boolean => {
    const cached = inert.get(element)
    if (cached !== undefined) return cached
    let attribute = false
    let liveModal = false
    for (
      let ancestor: Element | null = element;
      ancestor;
      ancestor = parent(ancestor)
    ) {
      if (ancestor.matches('dialog:modal')) liveModal = true
      if (ancestor.hasAttribute('inert')) attribute = true
    }
    const value = !liveModal && (attribute || modals.length > 0)
    inert.set(element, value)
    return value
  }
  const refs = lastSnapshotRefs(world.injected)
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
      (node.ref && (!element || !isInert(element))) ||
      node.children?.some(
        (child) => typeof child !== 'string' && protectedNodes.has(child),
      )
    )
      protectedNodes.add(node)
  }
  return protectedNodes
}
