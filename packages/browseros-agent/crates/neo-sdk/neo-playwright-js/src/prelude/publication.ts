/**
 * Navigation promise publication across independently delivered native replies. A receipt
 * means its requested document reached the load state or its native reply fully settled.
 * Optional metadata is bounded; a genuinely stalled navigation is never joined merely
 * because another browser call returned. Producers may still have JS replies queued.
 */
type Entry = { id: number; ready: boolean; done: Promise<void> }
const producers = new Map<number, Entry>()
const acknowledged = new Set<number>()
let nextPublication = 0

/** Every native envelope carries outstanding receipts until a later navigation acknowledges them. */
export function completedPublications(ids: unknown): void {
  if (!Array.isArray(ids)) return
  for (const id of ids) {
    if (typeof id !== 'number') continue
    const entry = producers.get(id)
    if (entry) entry.ready = true
    acknowledged.add(id)
  }
}

export class Publications {
  private readonly pending = new Set<Entry>()

  track<T>(
    work: (
      ready: () => void,
      publication: { id: number; acknowledged: number[] },
    ) => Promise<T>,
  ): Promise<T> {
    let finish!: () => void
    const id = ++nextPublication
    const entry = {
      id,
      ready: false,
      done: new Promise<void>((resolve) => {
        finish = resolve
      }),
    }
    producers.set(id, entry)
    this.pending.add(entry)
    const published = () => {
      this.pending.delete(entry)
      producers.delete(id)
      finish()
    }
    return Promise.resolve()
      .then(() => {
        const receipts = [...acknowledged]
        acknowledged.clear()
        return work(
          () => {
            entry.ready = true
          },
          { id, acknowledged: receipts },
        )
      })
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
