/** Finds the disjoint inert roots the page annotated; no DOM state is inferred in the runtime. */
import type { AriaNodeJSON } from './vendored'

export function inertRoots(json: readonly AriaNodeJSON[]): AriaNodeJSON[] {
  const roots: AriaNodeJSON[] = []
  const pending = [...json].reverse()
  while (pending.length) {
    const node = pending.pop()
    if (!node) continue
    if (node.inert) roots.push(node)
    else {
      const children = node.children ?? []
      for (let index = children.length - 1; index >= 0; index--) {
        const child = children[index]
        if (child && typeof child !== 'string') pending.push(child)
      }
    }
  }
  return roots
}
