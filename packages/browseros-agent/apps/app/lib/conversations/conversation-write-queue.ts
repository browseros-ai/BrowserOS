let tail: Promise<unknown> = Promise.resolve()

const swallow = () => undefined

/**
 * Serializes read-modify-write cycles on the shared conversation store within
 * this context, so a backfill, save, or delete never clobbers another's write
 * from a stale snapshot (#559): a stale save could otherwise drop the backfill's
 * `localOnly` flag (and upload adopted history) or a late backfill write could
 * resurrect a just-deleted conversation. Each mutation reads the latest value
 * inside its own turn, so overlapping mutations apply in order. `chrome.storage`
 * is shared across contexts and every `setValue` is last-write-wins, so this
 * covers same-context overlap; separate windows still resolve last-write-wins.
 */
export function runExclusive<T>(mutate: () => Promise<T>): Promise<T> {
  const result = tail.then(mutate, mutate)
  tail = result.then(swallow, swallow)
  return result
}
