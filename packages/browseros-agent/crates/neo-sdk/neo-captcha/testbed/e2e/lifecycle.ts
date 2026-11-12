/** Startup and SIGINT can overlap: register each resource before teardown sees it. */
export function createResources() {
  const cleanup: Array<() => Promise<unknown>> = []
  const pending = new Set<Promise<unknown>>()
  let closing: Promise<void> | undefined
  let interrupted = false
  return {
    async own<T>(
      start: () => Promise<T>,
      stop: (resource: T) => Promise<unknown>,
    ): Promise<T> {
      if (interrupted) throw new Error('run interrupted')
      const acquiring = start().then((resource) => {
        cleanup.push(() => stop(resource))
        return resource
      })
      pending.add(acquiring)
      try {
        const resource = await acquiring
        if (interrupted) throw new Error('run interrupted')
        return resource
      } finally {
        pending.delete(acquiring)
      }
    },
    close() {
      interrupted = true
      closing ??= (async () => {
        await Promise.allSettled(pending)
        const errors: unknown[] = []
        for (const stop of cleanup.toReversed())
          await stop().catch((error) => {
            errors.push(error)
          })
        if (errors.length)
          throw new AggregateError(
            errors,
            `cleanup failed: ${errors.map(String).join('; ')}`,
          )
      })()
      return closing
    },
  }
}
