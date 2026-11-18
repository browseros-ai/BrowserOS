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
  // Only document-level modals make the page inert in this version. Shadow modals are
  // discovered below for protection, so their live content can never be hidden with a host.
  const modals = [...document.querySelectorAll('dialog:modal')]
  const refs = lastSnapshotRefs(world.injected)
  if (modals.length === 0 && !hasInertAttribute(document, refs)) return []
  const modalAncestors = new Set<Element>()
  for (const modal of reachableModals(document, modals))
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
      const modal = ancestor.matches('dialog:modal')
      state = {
        modal: state.modal || modal,
        // A modal escapes inherited inertness, but its own attribute still applies.
        attribute:
          (!modal && state.attribute) || ancestor.hasAttribute('inert'),
      }
      ancestry.set(ancestor, state)
    }
    return state.attribute || (!state.modal && modals.length > 0)
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

/**
 * Modals the engine can read, including closed roots its isolated-world getter already opened.
 * This runs only after an inert trigger. An explicit stack handles deeply nested shadow hosts
 * without spending the JS call stack; no modal found here makes the surrounding page inert.
 */
function reachableModals(
  document: Document,
  modals: readonly Element[],
): Element[] {
  const reachable = [...modals]
  const pending: (Document | ShadowRoot)[] = [document]
  const seen = new Set<ShadowRoot>()
  for (let root = pending.pop(); root; root = pending.pop()) {
    for (const element of root.querySelectorAll('*')) {
      const shadow = element.shadowRoot
      if (!shadow || seen.has(shadow)) continue
      seen.add(shadow)
      reachable.push(...shadow.querySelectorAll('dialog:modal'))
      pending.push(shadow)
    }
  }
  return reachable
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
      // A ref-less image or icon proves nothing about its inert ancestors. Only known-live
      // elements protect this tree path; reachable modal ancestors are protected separately.
      (element !== undefined && !isInert(element)) ||
      node.children?.some(
        (child) => typeof child !== 'string' && protectedNodes.has(child),
      )
    )
      protectedNodes.add(node)
  }
  return protectedNodes
}
