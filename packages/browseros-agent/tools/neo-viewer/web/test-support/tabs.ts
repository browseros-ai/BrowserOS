/**
 * A fake browser origin with several viewer tabs, for the shared event stream's tests. It plays
 * what the browser does across tabs: Web Locks (exclusive and queued per name, held until the
 * callback's promise settles or its tab closes), BroadcastChannel (each message delivered later,
 * in order, to every other open channel of its name, as a copy) and EventSource (the test opens
 * it, sends events and drops it). Everything is delivered in microtasks, so one macrotask settles
 * it all.
 */

import type { StreamEvent } from '../../shared/api'
import type {
  ChannelLike,
  LocksLike,
  SourceLike,
  StreamDeps,
} from '../viewer-events'

/** One event stream a tab opened, as the server end sees it. */
export class FakeStream implements SourceLike {
  onopen: ((event: Event) => void) | null = null
  onmessage: ((message: MessageEvent<string>) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  closed = false

  constructor(
    readonly tab: FakeTab,
    readonly url: string,
  ) {
    tab.origin.streams.push(this)
  }

  /** The server answered: the stream is open. */
  open(): void {
    if (!this.closed) this.onopen?.(new Event('open'))
  }

  send(event: StreamEvent): void {
    if (!this.closed)
      this.onmessage?.(
        new MessageEvent('message', { data: JSON.stringify(event) }),
      )
  }

  /** The connection dropped; an EventSource then retries by itself. */
  drop(): void {
    if (!this.closed) this.onerror?.(new Event('error'))
  }

  close(): void {
    this.closed = true
  }
}

class FakeChannel implements ChannelLike {
  onmessage: ((message: MessageEvent) => void) | null = null
  closed = false

  constructor(
    readonly tab: FakeTab,
    readonly name: string,
  ) {
    tab.origin.channels.add(this)
  }

  postMessage(message: unknown): void {
    if (this.closed) throw new Error('InvalidStateError: the channel is closed')
    this.tab.origin.broadcast(this, structuredClone(message))
  }

  close(): void {
    this.closed = true
    this.tab.origin.channels.delete(this)
  }
}

interface LockRequest {
  tab: FakeTab
  callback: () => Promise<void>
  resolve: () => void
  reject: (cause: unknown) => void
}

/** One page of the origin. */
export class FakeTab {
  /** Messages this tab posted while held, waiting for `release`. */
  private held: Array<() => void> | null = null

  constructor(readonly origin: FakeOrigin) {}

  readonly deps: StreamDeps = {
    locks: {
      request: (name, callback) => this.origin.request(this, name, callback),
    } satisfies LocksLike,
    channel: (name) => new FakeChannel(this, name),
    source: (url) => new FakeStream(this, url),
  }

  /** The browser closes the tab: its locks pass to the next in line, its channels and streams close. */
  close(): void {
    this.origin.closeTab(this)
  }

  /** Holds back the messages this tab posts from now on, so they arrive late; `release` sends them. */
  hold(): void {
    this.held = []
  }

  release(): void {
    const held = this.held ?? []
    this.held = null
    for (const send of held) send()
  }

  /** Sends now, or after `release` while held. */
  post(send: () => void): void {
    if (this.held) this.held.push(send)
    else send()
  }

  /**
   * Puts this tab's fakes where the page's own code finds them (`navigator.locks`,
   * `BroadcastChannel`, `EventSource`), for a test that renders the page. Returns the undo.
   */
  install(): () => void {
    const tab = this
    const values: Record<string, unknown> = {
      BroadcastChannel: class extends FakeChannel {
        constructor(name: string) {
          super(tab, name)
        }
      },
      EventSource: class extends FakeStream {
        constructor(url: string) {
          super(tab, url)
        }
      },
    }
    const saved = Object.keys(values).map(
      (name) =>
        [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const,
    )
    for (const [name, value] of Object.entries(values))
      Object.defineProperty(globalThis, name, {
        configurable: true,
        writable: true,
        value,
      })
    Object.defineProperty(navigator, 'locks', {
      configurable: true,
      value: this.deps.locks,
    })
    return () => {
      for (const [name, descriptor] of saved) {
        if (descriptor) Object.defineProperty(globalThis, name, descriptor)
        else delete (globalThis as Record<string, unknown>)[name]
      }
      delete (navigator as unknown as Record<string, unknown>).locks
    }
  }
}

export class FakeOrigin {
  /** Every event stream any tab opened, in order. */
  readonly streams: FakeStream[] = []
  readonly channels = new Set<FakeChannel>()
  private readonly locks = new Map<
    string,
    { holder: LockRequest | null; queue: LockRequest[] }
  >()

  tab(): FakeTab {
    return new FakeTab(this)
  }

  /** The streams still open: the connections to the viewer they hold. */
  openStreams(): FakeStream[] {
    return this.streams.filter((stream) => !stream.closed)
  }

  request(
    tab: FakeTab,
    name: string,
    callback: () => Promise<void>,
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      this.lockNamed(name).queue.push({ tab, callback, resolve, reject })
      this.grant(name)
    })
  }

  broadcast(sender: FakeChannel, data: unknown): void {
    for (const target of this.channels) {
      if (target === sender || target.name !== sender.name) continue
      sender.tab.post(() =>
        queueMicrotask(() => {
          if (!target.closed)
            target.onmessage?.(new MessageEvent('message', { data }))
        }),
      )
    }
  }

  closeTab(tab: FakeTab): void {
    for (const channel of [...this.channels])
      if (channel.tab === tab) channel.close()
    for (const stream of this.streams) if (stream.tab === tab) stream.close()
    for (const [name, lock] of this.locks) {
      lock.queue = lock.queue.filter((request) => request.tab !== tab)
      if (lock.holder?.tab === tab) this.free(name, lock.holder)
    }
  }

  private lockNamed(name: string) {
    let lock = this.locks.get(name)
    if (!lock) {
      lock = { holder: null, queue: [] }
      this.locks.set(name, lock)
    }
    return lock
  }

  private grant(name: string): void {
    const lock = this.lockNamed(name)
    if (lock.holder) return
    const next = lock.queue.shift()
    if (!next) return
    lock.holder = next
    queueMicrotask(() => {
      // Its tab closed between the grant and the callback.
      if (lock.holder !== next) return
      next
        .callback()
        .then(next.resolve, next.reject)
        .finally(() => this.free(name, next))
    })
  }

  private free(name: string, request: LockRequest): void {
    const lock = this.lockNamed(name)
    if (lock.holder !== request) return
    lock.holder = null
    this.grant(name)
  }
}

/** Lets every delivery, grant and callback the fakes queued run. */
export function settle(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}
