/** Rust words the layer once; the runtime pins those words and moves existing live tree nodes. */
import type { AriaNodeJSON } from './vendored'
import type { SnapshotResult } from './wire'

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
  first: AriaNodeJSON[]
  rest: AriaNodeJSON[]
} {
  const layers = (result.onTop ?? []).filter((layer) => layer.wholeView)
  if (!layers.length) return { first: [], rest: result.json }
  const front = new Map(
    layers.map((layer) => [layer.ref, [] as AriaNodeJSON[]]),
  )
  const nodes: AriaNodeJSON[] = []
  const selected = new Set<AriaNodeJSON>()
  const pending = [...result.json].reverse()
  while (pending.length) {
    const node = pending.pop()
    if (!node) continue
    nodes.push(node)
    if (node.inert) continue
    const block = node.onTop ? front.get(node.onTop) : undefined
    if (block) {
      selected.add(node)
      block.push(node)
    } else
      for (const child of [...(node.children ?? [])].reverse())
        if (typeof child !== 'string') pending.push(child)
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
