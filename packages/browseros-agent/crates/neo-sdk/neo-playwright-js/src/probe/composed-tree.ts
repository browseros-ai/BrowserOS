/**
 * Every element under a root in composed-tree order: the order the engine's aria walk visits
 * (ariaSnapshot.ts `visit` at v1.64.1), so a list built here lines up with the refs a snapshot
 * would have minted. An element's light children that no slot shows come first, then its shadow
 * root's children; a slot's assigned nodes take its place, or its own children when nothing is
 * assigned. The shadow roots are the ones this world's `shadowRoot` getter answers: open roots,
 * and closed roots a marked snapshot handed it (shadow-roots.ts).
 */
export function* composedElements(root: Element): Generator<Element> {
  const visited = new Set<Element>()
  const pending: Element[] = [root]
  while (pending.length) {
    const element = pending.pop()
    if (!element || visited.has(element)) continue
    visited.add(element)
    yield element
    // A stack, so no nesting depth can exhaust the call stack: children go on in reverse to come
    // off in order, one push each, since a spread of a huge child list overflows its arguments.
    const children = childrenOf(element)
    for (let index = children.length - 1; index >= 0; index--) {
      const child = children[index]
      if (child) pending.push(child)
    }
  }
}

/** The element children the walk visits next, text and comments left out. */
function childrenOf(element: Element): Element[] {
  const assigned =
    element.nodeName === 'SLOT'
      ? (element as HTMLSlotElement).assignedNodes()
      : []
  if (assigned.length) return assigned.filter(isElement)
  const children: Element[] = []
  for (
    let child = element.firstElementChild;
    child;
    child = child.nextElementSibling
  )
    if (!child.assignedSlot) children.push(child)
  const shadow = element.shadowRoot
  for (
    let child = shadow?.firstElementChild;
    child;
    child = child.nextElementSibling
  )
    children.push(child)
  return children
}

function isElement(node: Node): node is Element {
  return node.nodeType === 1
}
