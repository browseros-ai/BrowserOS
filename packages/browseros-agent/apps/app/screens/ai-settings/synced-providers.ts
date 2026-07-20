import type { IncompleteProvider } from './IncompleteProviderCard'

export interface SyncedProviderPartition {
  incompleteProviders: IncompleteProvider[]
  retiredProviderIds: string[]
}

export function partitionSyncedProviders(
  nodes: readonly (IncompleteProvider | null)[],
  localProviderIds: ReadonlySet<string>,
): SyncedProviderPartition {
  const incompleteProviders: IncompleteProvider[] = []
  const retiredProviderIds: string[] = []

  for (const node of nodes) {
    if (!node) continue
    // The literal identifies synced rows whose provider type no longer exists.
    if (node.type === 'remote-hermes') {
      retiredProviderIds.push(node.rowId)
    } else if (!localProviderIds.has(node.rowId)) {
      incompleteProviders.push(node)
    }
  }

  return { incompleteProviders, retiredProviderIds }
}
