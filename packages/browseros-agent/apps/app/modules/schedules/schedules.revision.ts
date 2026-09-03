import { storage } from '@wxt-dev/storage'

/**
 * A change signal, not data.
 *
 * Scheduled runs are written by the background while the side panel and new
 * tab display them, and the two are separate contexts. Extension storage used
 * to carry both the data and the notification, so `watch` kept every surface
 * current for free. The data now lives on the server, which nothing can watch,
 * so this keeps the notification half: the background bumps it after a write
 * and the query cache is invalidated wherever a view is mounted.
 *
 * The value is a timestamp rather than a counter so two contexts writing at
 * once cannot lose a bump to a read-modify-write race.
 */
export const scheduleRevisionStorage = storage.defineItem<number>(
  'local:schedule-revision',
  { fallback: 0 },
)

export async function bumpScheduleRevision(): Promise<void> {
  await scheduleRevisionStorage.setValue(Date.now())
}

export function watchScheduleRevision(onChange: () => void): () => void {
  return scheduleRevisionStorage.watch(() => onChange())
}
