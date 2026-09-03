import { storage } from '@wxt-dev/storage'

/**
 * A change signal, not data.
 *
 * The provider list and the selected provider live on the server, and every
 * extension surface holds its own query cache of them. Extension storage used
 * to carry the data itself, so `watch` kept the side panel, the new tab, the
 * settings page and the scheduled tasks page in step for free. Moving the data
 * to the server kept the cache per surface and dropped the broadcast, so a
 * provider added in one place stayed invisible to the others until they
 * happened to refetch. This keeps the broadcast half.
 *
 * The value is a timestamp rather than a counter so two surfaces writing at
 * once cannot lose a bump to a read-modify-write race.
 */
export const providerRevisionStorage = storage.defineItem<number>(
  'local:provider-revision',
  { fallback: 0 },
)

export async function bumpProviderRevision(): Promise<void> {
  await providerRevisionStorage.setValue(Date.now())
}

export function watchProviderRevision(onChange: () => void): () => void {
  return providerRevisionStorage.watch(() => onChange())
}
