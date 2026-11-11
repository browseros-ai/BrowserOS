/**
 * Client completion after its final browser await. A ready producer may still have promise
 * wrappers publishing its frame state; a later consumer flushes those ready calls only.
 * Pending browser work is never joined, so a stalled navigation cannot block window.stop().
 */
export class Publications {
  private readonly pending = new Set<{ ready: boolean; done: Promise<void> }>()

  track<T>(work: (ready: () => void) => Promise<T>): Promise<T> {
    let finish!: () => void
    const entry = {
      ready: false,
      done: new Promise<void>((resolve) => {
        finish = resolve
      }),
    }
    this.pending.add(entry)
    const published = () => {
      this.pending.delete(entry)
      finish()
    }
    return Promise.resolve()
      .then(() =>
        work(() => {
          entry.ready = true
        }),
      )
      .then(
        (value) => {
          published()
          return value
        },
        (error: unknown) => {
          published()
          throw error
        },
      )
  }

  async publishReady(): Promise<void> {
    const ready = [...this.pending].filter((entry) => entry.ready)
    await Promise.all(ready.map((entry) => entry.done))
  }
}
