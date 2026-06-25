import type { AXNode } from './ax-types'
import type { DocumentId, FrameId, RefMap } from './refs'
import { INTERACTIVE_ROLES, ROOT_ROLES, SKIP_ROLES, VALUE_ROLES } from './roles'

const IFRAME_ROLES: ReadonlySet<string> = new Set(['Iframe', 'iframe'])

export interface IframeStitch {
  /** The iframe element, used to resolve its child frameId for stitching. */
  backendNodeId: number
  /** Index of the `- iframe` line within this render's output, for splicing child content. */
  lineIndex: number
  /** Absolute indent depth of the iframe line. */
  depth: number
}

export interface RenderResult {
  text: string
  iframes: IframeStitch[]
}

export interface RenderOptions {
  /** Shared across all frames of a page so refs form one global namespace. */
  refs: RefMap
  frameId?: FrameId
  documentId?: DocumentId
  /** backendNodeId → reasons, from the DOM cursor-augmentation pass. */
  cursorHits?: Map<number, string[]>
  /** Extra indent levels to prepend (used when splicing a child frame under its iframe line). */
  baseDepth?: number
}

/** Renders a CDP accessibility tree into the canonical agent-facing snapshot. */
export function renderSnapshot(
  nodes: AXNode[],
  opts: RenderOptions,
): RenderResult {
  const byId = new Map<string, AXNode>()
  for (const node of nodes) byId.set(node.nodeId, node)

  const base = opts.baseDepth ?? 0
  const lines: string[] = []
  const iframes: IframeStitch[] = []

  const visit = (nodeId: string, depth: number): void => {
    const node = byId.get(nodeId)
    if (!node) return

    const role = node.ignored ? undefined : strVal(node.role)
    const name = strVal(node.name)
    const isCursorHit =
      node.backendDOMNodeId !== undefined &&
      (opts.cursorHits?.has(node.backendDOMNodeId) ?? false)

    if (isDropped(role, name, isCursorHit)) {
      for (const childId of node.childIds ?? []) visit(childId, depth)
      return
    }

    if (role && IFRAME_ROLES.has(role)) {
      let line = `${'  '.repeat(base + depth)}- iframe`
      if (name) line += ` ${JSON.stringify(name)}`
      lines.push(line)
      if (node.backendDOMNodeId !== undefined) {
        iframes.push({
          backendNodeId: node.backendDOMNodeId,
          lineIndex: lines.length - 1,
          depth: base + depth,
        })
      }
      return
    }

    lines.push(formatLine(node, role as string, name, base + depth, opts))
    for (const childId of node.childIds ?? []) visit(childId, depth + 1)
  }

  for (const rootId of entryNodeIds(nodes)) visit(rootId, 0)

  return { text: lines.join('\n'), iframes }
}

/** Where to begin the walk: document roots if present, else the first node. */
function entryNodeIds(nodes: AXNode[]): string[] {
  const roots = nodes
    .filter((n) => ROOT_ROLES.has(strVal(n.role)))
    .map((n) => n.nodeId)
  if (roots.length > 0) return roots
  return nodes[0] ? [nodes[0].nodeId] : []
}

function isDropped(
  role: string | undefined,
  name: string,
  isCursorHit: boolean,
): boolean {
  if (!role) return true
  if (SKIP_ROLES.has(role) || ROOT_ROLES.has(role)) return true
  // Unnamed generic containers carry no meaning unless they're cursor-interactive.
  if ((role === 'generic' || role === 'group') && !name && !isCursorHit) {
    return true
  }
  return false
}

function formatLine(
  node: AXNode,
  role: string,
  name: string,
  depth: number,
  opts: RenderOptions,
): string {
  let line = `${'  '.repeat(depth)}- ${role}`
  if (name) line += ` ${JSON.stringify(name)}`

  for (const state of formatStates(node)) line += ` [${state}]`

  const backendNodeId = node.backendDOMNodeId
  const cursorReasons =
    backendNodeId !== undefined
      ? opts.cursorHits?.get(backendNodeId)
      : undefined
  const actionable =
    backendNodeId !== undefined &&
    (INTERACTIVE_ROLES.has(role) || cursorReasons !== undefined)

  if (actionable) {
    const ref = opts.refs.mint({
      backendNodeId: backendNodeId as number,
      role,
      name,
      documentId: opts.documentId,
      frameId: opts.frameId,
    })
    line += ` [ref=${ref}]`
  }
  if (cursorReasons !== undefined) line += ' [cursor=pointer]'

  if (VALUE_ROLES.has(role)) {
    const value = strVal(node.value)
    if (value) line += `: ${JSON.stringify(value)}`
  }

  return line
}

function formatStates(node: AXNode): string[] {
  const states: string[] = []
  for (const prop of node.properties ?? []) {
    const v = prop.value.value
    switch (prop.name) {
      case 'checked':
        if (v === true) states.push('checked')
        else if (v === 'mixed') states.push('indeterminate')
        break
      case 'disabled':
        if (v === true) states.push('disabled')
        break
      case 'expanded':
        if (v === true) states.push('expanded')
        else if (v === false) states.push('collapsed')
        break
      case 'required':
        if (v === true) states.push('required')
        break
      case 'selected':
        if (v === true) states.push('selected')
        break
      case 'level':
        states.push(`level=${v}`)
        break
      default:
        break
    }
  }
  return states
}

function strVal(value: AXNode['role']): string {
  return typeof value?.value === 'string' ? value.value : ''
}
