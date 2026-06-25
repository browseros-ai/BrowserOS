import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { AXNode } from '../snapshot/ax-types'
import type { RefEntry } from '../snapshot/refs'
import { ROOT_ROLES } from '../snapshot/roles'
import { fetchAxTree } from './ax-tree'

export interface ResolvedElement {
  session: ProtocolApi
  backendNodeId: number
}

/**
 * Resolves a ref to a live element via two tiers: trust the cached backendNodeId, and on
 * staleness re-query the same accessibility tree by (role, name, nth). Using the same data
 * source that minted the ref keeps matching consistent across DOM churn — far more robust than
 * caching a CSS selector. Mutates `entry.backendNodeId` to refresh the cache on a re-query.
 */
export async function resolveRefEntry(
  session: ProtocolApi,
  entry: RefEntry,
  axParams: { frameId?: string } = {},
): Promise<ResolvedElement> {
  if (await isLive(session, entry.backendNodeId)) {
    return { session, backendNodeId: entry.backendNodeId }
  }

  const fresh = findByRoleNameNth(await fetchAxTree(session, axParams), entry)
  if (fresh === undefined) {
    throw new Error(
      `Stale ref ${entry.ref} (${entry.role} "${entry.name}"); take a new snapshot.`,
    )
  }
  entry.backendNodeId = fresh
  return { session, backendNodeId: fresh }
}

async function isLive(
  session: ProtocolApi,
  backendNodeId: number,
): Promise<boolean> {
  try {
    const resolved = await session.DOM.resolveNode({ backendNodeId })
    const objectId = resolved.object?.objectId
    if (objectId) {
      await session.Runtime.releaseObject({ objectId }).catch(() => {})
    }
    return Boolean(objectId)
  } catch {
    return false
  }
}

/** Pre-order walk matching the renderer, counting (role, name) occurrences to honour `nth`. */
function findByRoleNameNth(
  nodes: AXNode[],
  entry: RefEntry,
): number | undefined {
  const byId = new Map(nodes.map((node) => [node.nodeId, node]))
  const roots = nodes
    .filter((node) => ROOT_ROLES.has(roleOf(node)))
    .map((node) => node.nodeId)
  const start = roots.length > 0 ? roots : nodes[0] ? [nodes[0].nodeId] : []

  let count = 0
  let found: number | undefined
  const visit = (id: string): void => {
    if (found !== undefined) return
    const node = byId.get(id)
    if (!node) return
    if (
      !node.ignored &&
      node.backendDOMNodeId !== undefined &&
      roleOf(node) === entry.role &&
      nameOf(node) === entry.name
    ) {
      if (count === entry.nth) {
        found = node.backendDOMNodeId
        return
      }
      count++
    }
    for (const childId of node.childIds ?? []) visit(childId)
  }
  for (const id of start) visit(id)
  return found
}

function roleOf(node: AXNode): string {
  return typeof node.role?.value === 'string' ? node.role.value : ''
}

function nameOf(node: AXNode): string {
  return typeof node.name?.value === 'string' ? node.name.value : ''
}
