/**
 * One run's page and context events.
 *
 *   page.on('dialog', fn)   first listener ──▶ events.listen { page, event, on: true, since }
 *   page.off(...)           last listener  ──▶ events.listen { page, event, on: false, since }
 *   pump (one per run)      events.next {}, a background long poll ──▶ { events, lost }
 *                           ──▶ emit each on its page or the context
 *   page.waitForEvent(e)    since = Date.now() at the call ──▶ events.wait { page, event, since }
 *                           predicate said no ──▶ events.wait again with after = that seq,
 *                           with what is left of one deadline (since + timeout)
 *
 * Nothing calls into the runtime from Rust: events reach scripts only because the pump pulls
 * them (sdk-skeleton D7.8). `since` is the reason waits are race-free: `const p =
 * page.waitForEvent('popup'); await page.click(..)` stamps the time before the click is even
 * issued, so the popup the click opens is recorded after it, whichever op the host admits
 * first. QuickJS and the crate read the same clock in one process. The pump holds no cursor:
 * Rust keeps one read position per listener and delivers each event to it once, so a listener
 * attached after the pump read past an event still gets it if it is newer than its own stamp.
 * A reply lost when the host cancels the pump at the run's end loses its events with the run.
 */
import { invoke, invokeObserved, pumpDispatched } from './bridge'
import { trackChooserSetup } from './chooser-setup'
import type { EventEmitter } from './emitter'
import { abortedError, prefixed, unsupportedError } from './errors'
import { EventDeadline } from './event-deadline'
import {
  ConsoleMessage,
  Dialog,
  Download,
  FileChooser,
  pageError,
} from './event-values'
import type { Page } from './page'
import { type SignalLike, signalOf, untilAborted } from './signal'
import type { TimeoutSettings } from './timeouts'
import { timeoutMs } from './timeouts'
import type {
  DialogInfo,
  DownloadInfo,
  EventName,
  EventRecord,
  FileChooserInfo,
  Observation,
} from './wire'

/** Events a page reports, from Rust's log. */
const PAGE_EVENTS: ReadonlySet<string> = new Set([
  'popup',
  'close',
  'dialog',
  'dialogclosed',
  'console',
  'pageerror',
  'download',
  'filechooser',
  'load',
  'domcontentloaded',
  'frameattached',
  'framenavigated',
  'framedetached',
])
/** Events the context reports. */
const CONTEXT_EVENTS: ReadonlySet<string> = new Set([
  'page',
  'pageload',
  'dialogclosed',
  'frameattached',
  'framenavigated',
  'framedetached',
])

/** Playwright events Neo never reports, with what to do instead. */
const UNREPORTED: Readonly<Record<string, string>> = {
  request:
    'Neo does not report network events; wait for what the page shows instead',
  response:
    'Neo does not report network events; wait for what the page shows instead',
  requestfailed:
    'Neo does not report network events; wait for what the page shows instead',
  requestfinished:
    'Neo does not report network events; wait for what the page shows instead',
  websocket: 'Neo does not report WebSockets',
  worker: 'Neo does not report workers',
  serviceworker: 'Neo does not report service workers',
  backgroundpage: 'Neo does not report extension background pages',
  crash: 'Neo does not report crashes; a crashed page fails its next call',
  pageclose: 'listen for close on the page itself: page.on("close")',
  weberror:
    'the context reports no weberror events; pageerror listeners are silent in hosted BrowserOS',
}

/**
 * Throws for a Playwright event Neo never reports on this kind of emitter, so `page.on` fails
 * at once instead of a listener that silently never fires. Events Playwright does not define
 * stay local to the emitter (`emit` works), as in Node.
 */
export function checkReported(
  api: string,
  event: string | symbol,
  reported: 'page' | 'context',
): void {
  if (typeof event !== 'string') return
  const ours = reported === 'page' ? PAGE_EVENTS : CONTEXT_EVENTS
  if (ours.has(event)) return
  const other = reported === 'page' ? CONTEXT_EVENTS : PAGE_EVENTS
  const hint =
    UNREPORTED[event] ??
    (other.has(event)
      ? reported === 'page'
        ? `"${event}" is a browser context event: context.on("${event}")`
        : `Neo reports "${event}" on pages: page.on("${event}")`
      : undefined)
  if (hint !== undefined) throw unsupportedError(`${api}('${event}')`, hint)
}

/** Whether Rust reports `event` for this kind of emitter, so a listener needs its interest. */
export function isReported(
  event: string | symbol,
  on: 'page' | 'context',
): event is EventName {
  return (
    typeof event === 'string' &&
    (on === 'page' ? PAGE_EVENTS : CONTEXT_EVENTS).has(event)
  )
}

type WaitOptions = {
  predicate?: (value: never) => unknown
  timeout?: number
  signal?: SignalLike
}

type ChooserLease = { ready: Promise<void>; release(): Promise<void> }

/** What the hub needs from the context: its Page for a handle, and the context as an emitter. */
export type EventOwner = EventEmitter & {
  _timeoutSettings: TimeoutSettings
  _pageFromResult(value: unknown, url?: string): Page | null
}

export class EventHub {
  private readonly owner: EventOwner
  /** Listened (emitter, event) pairs; the pump runs while there is one. */
  private listening = 0
  private pumping = false
  /** A decoded pump batch owns its close positions until its earlier records run. */
  private batch: {
    closes: Set<number>
    done: Promise<void>
    finish: () => void
  } | null = null
  /** One object per dialog and download, so every listener and wait answers the same one. */
  private readonly dialogs = new Map<number, Dialog>()
  private readonly downloads = new Map<string, Download>()
  /** A record can reach several waits and the pump; they must share the chooser object. */
  private readonly choosers = new Map<number, FileChooser>()
  private nextChooserLease = 0
  private readonly chooserListeners = new Map<Page, ChooserLease>()
  /** Preserve synchronous on/off order even when native interception setup is asynchronous. */
  private readonly chooserChanges = new Map<Page, Promise<void>>()
  /** Native context observation setup must not let a later off overtake its on. */
  private readonly contextChanges = new Map<EventName, Promise<void>>()
  /** A returning old pump cannot deliver into a context listener's replacement period. */
  private readonly contextSince = new Map<EventName, number>()

  constructor(owner: EventOwner) {
    this.owner = owner
  }

  /**
   * A page or the context gained its first listener for `event`, or lost its last one. The
   * stamp is taken now, synchronously, so Rust delivers only what happened after the script
   * attached the listener; the op itself is not awaited, as `page.on` returns at once.
   */
  listen(page: Page | null, event: EventName, on: boolean): void {
    const since = Date.now()
    this.listening += on ? 1 : -1
    if (page === null) {
      if (on) this.contextSince.set(event, since)
      else this.contextSince.delete(event)
      const previous = this.contextChanges.get(event) ?? Promise.resolve()
      const changed = previous.then(() =>
        this.sendListen(null, event, on, since),
      )
      this.contextChanges.set(event, changed)
    } else if (event === 'filechooser') {
      const lease = on ? this.intercept(page) : this.chooserListeners.get(page)
      if (on && lease) this.chooserListeners.set(page, lease)
      else this.chooserListeners.delete(page)
      const previous = this.chooserChanges.get(page) ?? Promise.resolve()
      const changed = previous.then(() =>
        this.sendListen(page, event, on, since, lease),
      )
      this.chooserChanges.set(page, changed)
      trackChooserSetup(changed)
    } else void this.sendListen(page, event, on, since)
    if (on && !this.pumping) {
      this.pumping = true
      void this.pump()
    }
  }

  /**
   * `waitForEvent`: the first `event` recorded at or after `since` that the predicate accepts.
   * Every re-wait sends the same `since`, and the time left of one deadline (see
   * `EventDeadline`), so a rejected event never restarts the clock.
   */
  async wait(
    api: string,
    page: Page | null,
    event: string,
    optionsOrPredicate: unknown,
    since: number,
    settings: TimeoutSettings,
  ): Promise<unknown> {
    const kind = page === null ? 'context' : 'page'
    checkReported(api, event, kind)
    if (!isReported(event, kind))
      throw unsupportedError(
        api,
        `Neo reports only Playwright's events, not "${event}"`,
      )
    const options: WaitOptions =
      typeof optionsOrPredicate === 'function'
        ? { predicate: optionsOrPredicate as WaitOptions['predicate'] }
        : ((optionsOrPredicate ?? {}) as WaitOptions)
    const deadline = new EventDeadline(
      api,
      event,
      since,
      timeoutMs(api, settings.timeout(options)),
    )
    // upstream's Waiter rejects the whole wait when its signal fires, a pending predicate
    // included, with a plain `The operation was aborted`; the `events.wait` in flight takes the
    // signal too, so Rust stops waiting as well.
    const signal = signalOf(api, options)
    const lease = event === 'filechooser' && page ? this.intercept(page) : null
    try {
      return await untilAborted(
        signal,
        () => abortedError(api, signal as SignalLike),
        async () => {
          if (lease) await deadline.race(lease.ready)
          return this.waitFor(
            api,
            page,
            event,
            options,
            since,
            deadline,
            signal,
          )
        },
      )
    } finally {
      // Aborting a wait must not await native setup. The run owns unfinished leases too;
      // this cleanup follows setup and is a barrier for the script's next browser action.
      if (lease) void lease.release().catch(() => {})
    }
  }

  /** `wait`'s loop: one `events.wait` per candidate, until the predicate accepts one. */
  private async waitFor(
    api: string,
    page: Page | null,
    event: EventName,
    options: WaitOptions,
    since: number,
    deadline: EventDeadline,
    signal: SignalLike | undefined,
  ): Promise<unknown> {
    const target = page === null ? null : await page._openedBrand(api)
    let after: number | null = null
    for (;;) {
      const left = deadline.left()
      let record: EventRecord
      let observed: Observation | null = null
      try {
        ;({ value: record, observed } = await invokeObserved(
          'events.wait',
          { api, timeoutMs: left },
          { page: target, event, since, after },
          undefined,
          signal,
        ))
      } catch (error) {
        throw deadline.named(error)
      }
      // The bootstrap registers a batch before resolving either consumer. A duplicate
      // wait must let that batch reach the close before running its own predicate.
      if (record.event === 'close' && this.batch?.closes.has(record.seq))
        await this.batch.done
      let value: unknown
      try {
        value = this.value(record, observed)
      } finally {
        this.settleCloses(observed)
      }
      const predicate = options.predicate
      if (!predicate) return value
      let accepted: unknown
      try {
        accepted = await deadline.race(
          this.duringEvent(record, () =>
            Reflect.apply(predicate, undefined, [value]),
          ),
        )
      } catch (error) {
        throw prefixed(api, error)
      }
      if (accepted) return value
      after = record.seq
    }
  }

  private async sendListen(
    page: Page | null,
    event: EventName,
    on: boolean,
    since: number,
    lease?: ChooserLease,
  ): Promise<void> {
    const api = `${page === null ? 'browserContext' : 'page'}.${on ? 'on' : 'off'}`
    try {
      if (on && lease) await lease.ready
      await invoke(
        'events.listen',
        { api, timeoutMs: 0 },
        {
          page: page === null ? null : await page._openedBrand(api),
          event,
          on,
          since,
        },
      )
    } catch {
      // `page.on` already returned, so there is no caller to tell; a listener on a page that
      // is gone never fires, as in Playwright.
      if (on && lease) void lease.release().catch(() => {})
    } finally {
      if (!on && lease) await lease.release().catch(() => {})
    }
  }

  private intercept(page: Page): ChooserLease {
    const id = ++this.nextChooserLease
    const api = 'page.filechooser'
    const target = page._openedBrand(api)
    const ready = target.then(async (page) => {
      await invoke(
        'events.interceptFileChooser',
        { api, timeoutMs: 0 },
        { page, lease: id, on: true },
      )
    })
    trackChooserSetup(ready)
    let released: Promise<void> | null = null
    return {
      ready,
      release: () => {
        if (!released) {
          released = ready.then(async () => {
            await invoke(
              'events.interceptFileChooser',
              { api, timeoutMs: 0 },
              { page: await target, lease: id, on: false },
            )
          })
          trackChooserSetup(released)
        }
        return released
      },
    }
  }

  private async pump(): Promise<void> {
    try {
      while (this.listening > 0) {
        // Delivered as the reply is decoded. Rust holds an op that ends until its events are in
        // a counted pump reply (`events_before_reply`), and the bridge holds that op's result
        // until this many replies were dispatched (the pump ledger), so the listeners run before
        // the script resumes past the op, as upstream's do.
        try {
          await invokeObserved(
            'events.next',
            { api: 'events.next', timeoutMs: 0 },
            {},
            ({ events }, observed) => this.deliverBatch(events, observed),
          )
        } catch (error) {
          // An error reply is dispatched too: ops held on a reply Rust counted before it failed
          // go on at once instead of waiting out their bound.
          pumpDispatched()
          throw error
        }
      }
    } catch {
      // The host cancels the pump quietly when the run ends; a pump that failed otherwise
      // restarts with the next first listener.
    } finally {
      this.pumping = false
    }
  }

  /**
   * One pump reply's records, in log order, so earlier callbacks observe the page before its
   * close, even within the same reply. Its host-approved closes are staged first, before waits
   * can consume them, and settled after.
   */
  private deliverBatch(
    events: EventRecord[],
    observed: Observation | null,
  ): void {
    this.stageBatch(events)
    try {
      for (const event of events) this.deliver(event, observed)
    } finally {
      try {
        this.settleCloses(observed)
      } finally {
        const batch = this.batch
        this.batch = null
        batch?.finish()
        pumpDispatched()
      }
    }
  }

  /** Runs synchronously as the pump reply is decoded, before waits can consume its closes. */
  private stageBatch(events: EventRecord[]): void {
    const closes = new Set(
      events
        .filter((event) => event.event === 'close')
        .map((event) => event.seq),
    )
    if (closes.size === 0) return
    let finish!: () => void
    const done = new Promise<void>((resolve) => {
      finish = resolve
    })
    this.batch = { closes, done, finish }
  }

  /** The Page an event names; Rust names only pages its run's view admits. */
  private page(value: unknown, url?: string): Page {
    const page = this.owner._pageFromResult(value, url)
    if (!page)
      throw new Error(`Rust named no page in an event: ${String(value)}`)
    return page
  }

  /** Finish this reply's approvals if a record was absent or a callback interrupted the batch. */
  private settleCloses(observed: Observation | null): void {
    for (const close of observed?.closes ?? [])
      this.page({ $neo: 'page', tab: close.tab })._settleClose(observed)
  }

  private deliver(record: EventRecord, observed: Observation | null): void {
    const context =
      record.event === 'page' ||
      record.event === 'pageload' ||
      ('context' in record && record.context)
    if (
      context &&
      typeof record.at === 'number' &&
      record.at < (this.contextSince.get(record.event) ?? Infinity)
    )
      return
    switch (record.event) {
      case 'popup':
        this.page(record.page).emit(
          'popup',
          this.page(record.popup.page, record.popup.url),
        )
        return
      case 'page':
        this.owner.emit('page', this.page(record.page, record.url))
        return
      case 'load':
      case 'domcontentloaded': {
        const page = this.page(record.page)
        page._eventFrame(record.seq, 'framenavigated', record.frame)
        this.duringEvent(record, () => page.emit(record.event, page))
        return
      }
      case 'pageload': {
        const page = this.page(record.page)
        page._eventFrame(record.seq, 'framenavigated', record.frame)
        this.duringEvent(record, () => this.owner.emit('pageload', page))
        return
      }
      case 'frameattached':
      case 'framenavigated':
      case 'framedetached': {
        const page = this.page(record.page)
        const frame = page._eventFrame(record.seq, record.event, record.frame)
        const emitter = record.context ? this.owner : page
        frame._duringEvent(record.frame, record.event, () =>
          emitter.emit(record.event, frame),
        )
        return
      }
      case 'close':
        this.page(record.page)._closeEvent(record.seq, observed)
        return
      case 'dialogclosed': {
        const page = this.page(record.page)
        const emitter = record.context ? this.owner : page
        emitter.emit('dialogclosed', this.dialog(page, record.dialog))
        return
      }
      case 'dialog': {
        const page = this.page(record.page)
        const dialog = this.dialog(page, record.dialog)
        // upstream's client answers a dialog nobody handles, because a listener can be removed
        // after Rust decided to hold the dialog for it.
        if (!page.emit('dialog', dialog))
          void (
            dialog.type() === 'beforeunload'
              ? dialog.accept()
              : dialog.dismiss()
          ).catch(() => {})
        return
      }
      case 'console': {
        const page = this.page(record.page)
        page.emit('console', new ConsoleMessage(page, record.message))
        return
      }
      case 'pageerror':
        this.page(record.page).emit('pageerror', pageError(record.error))
        return
      case 'download': {
        const page = this.page(record.page)
        page.emit('download', this.download(page, record.download))
        return
      }
      case 'filechooser': {
        const page = this.page(record.page)
        page.emit('filechooser', this.chooser(record.seq, page, record.chooser))
        return
      }
    }
  }

  /** Invocation-time metadata is separate from the canonical object's newest cached state. */
  private duringEvent<T>(record: EventRecord, read: () => T): T {
    switch (record.event) {
      case 'frameattached':
      case 'framenavigated':
      case 'framedetached': {
        const frame = this.page(record.page)._eventFrame(
          record.seq,
          record.event,
          record.frame,
        )
        return frame._duringEvent(record.frame, record.event, read)
      }
      case 'load':
      case 'domcontentloaded':
      case 'pageload':
        return this.page(record.page)
          .mainFrame()
          ._duringEvent(record.frame, 'framenavigated', read)
      default:
        return read()
    }
  }

  /** The value `waitForEvent` resolves with for one event. */
  private value(record: EventRecord, observed: Observation | null): unknown {
    switch (record.event) {
      case 'popup':
        return this.page(record.popup.page, record.popup.url)
      case 'page':
        return this.page(record.page, record.url)
      case 'load':
      case 'domcontentloaded':
      case 'pageload': {
        const page = this.page(record.page)
        page._eventFrame(record.seq, 'framenavigated', record.frame)
        return page
      }
      case 'frameattached':
      case 'framenavigated':
      case 'framedetached':
        return this.page(record.page)._eventFrame(
          record.seq,
          record.event,
          record.frame,
        )
      case 'close': {
        const page = this.page(record.page)
        page._closeEvent(record.seq, observed)
        return page
      }
      case 'dialog':
      case 'dialogclosed':
        return this.dialog(this.page(record.page), record.dialog)
      case 'console':
        return new ConsoleMessage(this.page(record.page), record.message)
      case 'pageerror':
        return pageError(record.error)
      case 'download':
        return this.download(this.page(record.page), record.download)
      case 'filechooser':
        return this.chooser(record.seq, this.page(record.page), record.chooser)
    }
  }

  private dialog(page: Page, info: DialogInfo): Dialog {
    const known = this.dialogs.get(info.id)
    if (known) return known
    const dialog = new Dialog(page, info)
    this.dialogs.set(info.id, dialog)
    return dialog
  }

  private download(page: Page, info: DownloadInfo): Download {
    const known = this.downloads.get(info.id)
    if (known) return known
    const download = new Download(page, info)
    this.downloads.set(info.id, download)
    return download
  }

  private chooser(seq: number, page: Page, info: FileChooserInfo): FileChooser {
    const known = this.choosers.get(seq)
    if (known) return known
    const chooser = new FileChooser(page, info)
    this.choosers.set(seq, chooser)
    return chooser
  }
}
