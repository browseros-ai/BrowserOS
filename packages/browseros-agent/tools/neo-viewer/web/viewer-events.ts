/**
 * One `/api/events` stream per origin, heard by every viewer tab of that origin.
 *
 * A browser opens at most six HTTP/1.1 connections to one origin, and an event stream holds one
 * of them for as long as its page is open. With a stream per tab, six viewer tabs take all six,
 * and every other request those tabs make (the live screen's status poll, a run's detail) waits
 * for a connection that never frees. So the tabs share one stream:
 *
 *   leader tab     holds the `neo-viewer:events` Web Lock and the one EventSource; relays what
 *       │          it hears on the BroadcastChannel of the same name
 *       ▼  open {epoch}, then event {epoch, event}, …
 *   other tabs     queued for the lock, in the order they opened; hear the relay
 *
 * The lock is exclusive and queued, and the browser releases it when its tab closes, reloads or
 * crashes. The next tab in line then opens its own stream and announces it, and every tab takes
 * a fresh snapshot, as a page does when its own stream opens. Each opening of the stream has its
 * own epoch, and a tab hears only the events of the opening it last adopted, so a late message
 * from a leader that has gone is dropped.
 *
 * No event falls between a tab's snapshot and the events it hears: a tab takes its snapshot only
 * after it adopts an `open`. Every event the leader heard before that `open` was already in the
 * server's store when the snapshot was read; every event after it is relayed under the adopted
 * epoch, and a BroadcastChannel keeps one sender's messages in order.
 *
 * Web Locks and a BroadcastChannel rather than a SharedWorker that owns the stream: the page is
 * one bundle with no worker entry, Bun's tests cannot run a worker, and the browser hands the lock
 * on when a tab crashes, with no heartbeat to keep. Where either is missing (`navigator.locks`
 * needs a secure context, which `http://127.0.0.1` is), each tab opens its own stream.
 */

import { apiRoutes, type ViewerEvent } from '../shared/api'

/**
 * The lock's and the channel's name. A tab still running an older page shares the channel, so a
 * change to `Relayed` changes the name.
 */
const NAME = 'neo-viewer:events'

export interface ViewerEventListener {
  /** The stream this tab hears (re)opened: take a fresh snapshot; every later event follows it. */
  opened(): void
  event(event: ViewerEvent): void
}

/** What a tab tells the others on the channel. */
type Relayed =
  /** A tab that just started asks whether a stream is open. */
  | { kind: 'hello'; tab: string }
  /** The leader's stream opened; with `tab`, it answers that tab's hello and no other tab adopts it. */
  | { kind: 'open'; epoch: string; tab?: string }
  | { kind: 'event'; epoch: string; event: ViewerEvent }

/** One subscription's part in the origin's stream. */
type Role =
  /** Queued for the lock; hears the leader's relay of the opening it adopted (none yet: null). */
  | { kind: 'following'; epoch: string | null }
  /** Holds the lock and the stream; `epoch` is null while the stream is not open. */
  | { kind: 'leading'; source: SourceLike; epoch: string | null }
  | { kind: 'closed' }

/** The parts of `navigator.locks` the stream uses. */
export interface LocksLike {
  /** Runs `callback` once this tab holds the exclusive lock `name`; it is held until the callback's promise settles. */
  request(name: string, callback: () => Promise<void>): Promise<unknown>
}

/** The parts of a BroadcastChannel the stream uses. */
export interface ChannelLike {
  onmessage: ((message: MessageEvent<Relayed>) => void) | null
  postMessage(message: Relayed): void
  close(): void
}

/** The parts of an EventSource the stream uses. */
export interface SourceLike {
  onopen: ((event: Event) => void) | null
  onmessage: ((message: MessageEvent<string>) => void) | null
  onerror: ((event: Event) => void) | null
  close(): void
}

/** The browser pieces the stream needs; tests pass their own to play several tabs. */
export interface StreamDeps {
  locks: LocksLike | undefined
  channel: ((name: string) => ChannelLike) | undefined
  source(url: string): SourceLike
}

function browserStreams(): StreamDeps {
  return {
    // Absent outside a secure context, whatever the DOM types say.
    locks: navigator.locks as LockManager | undefined,
    channel:
      typeof BroadcastChannel === 'function'
        ? (name) => new BroadcastChannel(name)
        : undefined,
    source: (url) => new EventSource(url),
  }
}

/** Starts hearing the origin's one stream; the returned function stops (and hands leadership on). */
export function listenToViewerEvents(
  listener: ViewerEventListener,
  deps: StreamDeps = browserStreams(),
): () => void {
  const { locks, channel: openChannel } = deps
  if (!locks || !openChannel) return listenAlone(listener, deps.source)

  const tab = crypto.randomUUID()
  let role: Role = { kind: 'following', epoch: null }
  let release = () => {}
  const held = new Promise<void>((resolve) => {
    release = resolve
  })

  const channel = openChannel(NAME)
  channel.onmessage = ({ data: message }) => {
    if (role.kind === 'leading') {
      // `open` and `event` can only be late ones from the previous leader.
      if (message.kind === 'hello' && role.epoch !== null)
        channel.postMessage({
          kind: 'open',
          epoch: role.epoch,
          tab: message.tab,
        })
      return
    }
    if (role.kind !== 'following') return
    // A hello that crosses the leader's broadcast `open` is answered with that same opening.
    if (
      message.kind === 'open' &&
      message.epoch !== role.epoch &&
      (message.tab === undefined || message.tab === tab)
    ) {
      role = { kind: 'following', epoch: message.epoch }
      listener.opened()
    } else if (message.kind === 'event' && message.epoch === role.epoch) {
      listener.event(message.event)
    }
  }

  const lead = () => {
    const source = deps.source(apiRoutes.events)
    role = { kind: 'leading', source, epoch: null }
    source.onopen = () => {
      if (role.kind !== 'leading') return
      const epoch = crypto.randomUUID()
      role = { ...role, epoch }
      // No `tab`: every tab re-snapshots, as each did when its own stream opened.
      channel.postMessage({ kind: 'open', epoch })
      listener.opened()
    }
    source.onmessage = (message) => {
      if (role.kind !== 'leading' || role.epoch === null) return
      const event = JSON.parse(message.data) as ViewerEvent
      channel.postMessage({ kind: 'event', epoch: role.epoch, event })
      listener.event(event)
    }
    // The EventSource retries by itself; its next `onopen` announces a new epoch.
    source.onerror = () => {
      if (role.kind === 'leading') role = { ...role, epoch: null }
    }
  }

  channel.postMessage({ kind: 'hello', tab })
  // A subscription stopped while queued is still granted the lock in its turn (React's StrictMode
  // mounts, unmounts and mounts again before any grant); it lets the lock go at once, so the next
  // tab in line leads.
  void locks.request(NAME, async () => {
    if (role.kind === 'closed') return
    lead()
    await held
  })

  return () => {
    if (role.kind === 'leading') role.source.close()
    role = { kind: 'closed' }
    release()
    channel.close()
  }
}

/** This tab's own stream, heard by this tab only: the viewer before tabs shared one. */
function listenAlone(
  listener: ViewerEventListener,
  openSource: (url: string) => SourceLike,
): () => void {
  const source = openSource(apiRoutes.events)
  source.onopen = () => listener.opened()
  source.onmessage = (message) =>
    listener.event(JSON.parse(message.data) as ViewerEvent)
  return () => source.close()
}
