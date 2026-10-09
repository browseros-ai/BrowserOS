/** Tracks worker persistence replies through the recorder's stop boundary. */
export function createRecorderPersistence() {
  const pending = new Set<Promise<void>>()
  // Retire settled promises to bound memory, but keep failures until this
  // recorder is replaced. Later successes cannot make a lost batch durable.
  let failed = false
  return {
    track(post: Promise<boolean>) {
      const tracked = post
        .then(
          (persisted) => {
            if (!persisted) failed = true
          },
          () => {
            failed = true
          },
        )
        .finally(() => pending.delete(tracked))
      pending.add(tracked)
    },
    async confirmed() {
      await Promise.all(pending)
      return !failed
    },
  }
}
