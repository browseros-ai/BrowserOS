/** Rust words the layer once; the runtime pins those words and moves existing live tree nodes. */
import { beside, type Hidden } from './snapshot-hidden'
import type { AriaNodeJSON } from './vendored'
import type { SnapshotResult } from './wire'

/**
 * A node a whole-view layer holds, printed first and so outside every line around it in the
 * page: `hidden` is `'carried'` when an `aria-hidden` node is above it in its document, whose
 * line no longer encloses it.
 */
export type Held = { node: AriaNodeJSON; hidden: Hidden }

export function onTopHead(result: SnapshotResult): string[] {
  return (result.onTop ?? []).flatMap((layer) => layer.line.split('\n'))
}

export function isOnTopLine(line: string): boolean {
  return (
    line.startsWith('# on top: ') ||
    line.startsWith('#   its controls: ') ||
    line.startsWith('# no longer on top: ')
  )
}

/** The baseline is the text actually returned, so no page-side read can replace its identities. */
export function onTopDelta(previous: string, result: SnapshotResult): string[] {
  const current = new Set((result.onTop ?? []).map((layer) => layer.ref))
  const removed: string[] = []
  for (const line of previous.split('\n')) {
    // Names are JSON quoted. Text that contains a ref-shaped word must not become an identity.
    const match =
      /^# on top: (\S+ (?:"(?:[^"\\]|\\.)*" )?\[ref=([^\]]+)\])/.exec(line)
    if (match && !current.has(match[2]))
      removed.push(`# no longer on top: ${match[1]}`)
  }
  return [...onTopHead(result), ...removed]
}

/** Only the default form uses this projection; full mode keeps the engine's document order. */
export function onTopContent(result: SnapshotResult): {
  first: Held[]
  rest: AriaNodeJSON[]
} {
  const layers = (result.onTop ?? []).filter((layer) => layer.wholeView)
  if (!layers.length) return { first: [], rest: result.json }
  const front = new Map(layers.map((layer) => [layer.ref, [] as Held[]]))
  const nodes: AriaNodeJSON[] = []
  const selected = new Set<AriaNodeJSON>()
  // Each node with what it inherits of an `aria-hidden` node above it once no line encloses it.
  const pending: Held[] = [...result.json]
    .reverse()
    .map((node) => ({ node, hidden: 'no' }))
  while (pending.length) {
    const candidate = pending.pop()
    if (!candidate) continue
    const { node, hidden } = candidate
    nodes.push(node)
    if (node.inert) continue
    const block = node.onTop ? front.get(node.onTop) : undefined
    if (block) {
      selected.add(node)
      block.push(candidate)
    } else {
      const below = beside(hidden, node)
      for (const child of [...(node.children ?? [])].reverse())
        if (typeof child !== 'string')
          pending.push({ node: child, hidden: below })
    }
  }
  // One iterative projection preserves deep trees, source order and the untouched input.
  const remaining = new Map<AriaNodeJSON, AriaNodeJSON | undefined>()
  for (const node of nodes.reverse()) {
    if (selected.has(node)) {
      remaining.set(node, undefined)
      continue
    }
    const children = node.children?.flatMap((child) => {
      if (typeof child === 'string' || !remaining.has(child)) return [child]
      const kept = remaining.get(child)
      return kept ? [kept] : []
    })
    const changed =
      children &&
      (children.length !== node.children?.length ||
        children.some((child, index) => child !== node.children?.[index]))
    remaining.set(node, changed ? { ...node, children } : node)
  }
  return {
    first: layers.flatMap((layer) => front.get(layer.ref) ?? []),
    rest: result.json.flatMap((node) => {
      const kept = remaining.get(node)
      return kept ? [kept] : []
    }),
  }
}
