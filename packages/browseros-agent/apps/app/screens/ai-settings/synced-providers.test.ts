import { describe, expect, it } from 'bun:test'
import type { IncompleteProvider } from './IncompleteProviderCard'
import { partitionSyncedProviders } from './synced-providers'

function syncedProvider(
  overrides: Partial<IncompleteProvider> & Pick<IncompleteProvider, 'rowId'>,
): IncompleteProvider {
  return {
    type: 'openai',
    name: 'OpenAI',
    modelId: 'gpt-5',
    supportsImages: true,
    ...overrides,
  }
}

describe('partitionSyncedProviders', () => {
  it('reconciles Remote Hermes rows instead of offering them for restoration', () => {
    const local = syncedProvider({ rowId: 'local' })
    const missing = syncedProvider({ rowId: 'missing' })
    const remoteHermes = syncedProvider({
      rowId: 'remote-hermes',
      type: 'remote-hermes',
      name: 'Remote Hermes',
    })

    expect(
      partitionSyncedProviders(
        [null, local, missing, remoteHermes],
        new Set([local.rowId]),
      ),
    ).toEqual({
      incompleteProviders: [missing],
      retiredProviderIds: [remoteHermes.rowId],
    })
  })
})
