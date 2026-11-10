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
import { invoke } from './bridge'
import type { EventEmitter } from './emitter'
import { prefixed, unsupportedError } from './errors'
import { EventDeadline } from './event-deadline'
import { ConsoleMessage, Dialog, Download, pageError } from './event-values'
import type { Page } from './page'
import type { TimeoutSettings } from './timeouts'
import { timeoutMs } from './timeouts'
import type { DialogInfo, DownloadInfo, EventName, EventRecord } from './wire'

/** Events a page reports, from Rust's log. */
const PAGE_EVENTS: ReadonlySet<string> = new Set([
  'popup',
  'close',
  'dialog',
  'console',
  'pageerror',
  'download',
])
/** Events the context reports. */
const CONTEXT_EVENTS: ReadonlySet<string> = new Set(['page'])

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
  filechooser:
    'Neo does not report file choosers; call locator.setInputFiles() on the input',
  frameattached:
    'Neo does not report frame events; read page.frames(), which Neo refreshes after navigations and waits',
  framedetached:
    'Neo does not report frame events; read page.frames(), which Neo refreshes after navigations and waits',
  framenavigated: 'Neo does not report frame events; use page.waitForURL()',
  load: 'Neo does not report load events; use page.waitForLoadState()',
  domcontentloaded:
    'Neo does not report load events; use page.waitForLoadState("domcontentloaded")',
  crash: 'Neo does not report crashes; a crashed page fails its next call',
  pageload: 'Neo does not report load events; use page.waitForLoadState()',
  pageclose: 'listen for close on the page itself: page.on("close")',
  weberror: 'listen for pageerror on the page itself: page.on("pageerror")',
  dialogclosed: 'Neo does not report closed dialogs',
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

type WaitOptions = { predicate?: (value: never) => unknown; timeout?: number }

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
  /** One object per dialog and download, so every listener and wait answers the same one. */
  private readonly dialogs = new Map<number, Dialog>()
  private readonly downloads = new Map<string, Download>()

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
    void this.sendListen(page, event, on, since)
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
    const target = page === null ? null : await page._openedBrand(api)
    let after: number | null = null
    for (;;) {
      const left = deadline.left()
      let record: EventRecord
      try {
        record = await invoke(
          'events.wait',
          { api, timeoutMs: left },
          { page: target, event, since, after },
        )
      } catch (error) {
        throw deadline.named(error)
      }
      const value = this.value(record)
      if (!options.predicate) return value
      let accepted: unknown
      try {
        accepted = await deadline.race(
          Reflect.apply(options.predicate, undefined, [value]),
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
  ): Promise<void> {
    const api = `${page === null ? 'browserContext' : 'page'}.${on ? 'on' : 'off'}`
    try {
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
    }
  }

  private async pump(): Promise<void> {
    try {
      while (this.listening > 0) {
        const { events } = await invoke(
          'events.next',
          { api: 'events.next', timeoutMs: 0 },
          {},
        )
        for (const event of events) this.deliver(event)
      }
    } catch {
      // The host cancels the pump quietly when the run ends; a pump that failed otherwise
      // restarts with the next first listener.
    } finally {
      this.pumping = false
    }
  }

  /** The Page an event names; Rust names only pages its run's view admits. */
  private page(value: unknown, url?: string): Page {
    const page = this.owner._pageFromResult(value, url)
    if (!page)
      throw new Error(`Rust named no page in an event: ${String(value)}`)
    return page
  }

  private deliver(record: EventRecord): void {
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
      case 'close':
        this.page(record.page)._didClose()
        return
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
    }
  }

  /** The value `waitForEvent` resolves with for one event. */
  private value(record: EventRecord): unknown {
    switch (record.event) {
      case 'popup':
        return this.page(record.popup.page, record.popup.url)
      case 'page':
        return this.page(record.page, record.url)
      case 'close': {
        const page = this.page(record.page)
        page._didClose()
        return page
      }
      case 'dialog':
        return this.dialog(this.page(record.page), record.dialog)
      case 'console':
        return new ConsoleMessage(this.page(record.page), record.message)
      case 'pageerror':
        return pageError(record.error)
      case 'download':
        return this.download(this.page(record.page), record.download)
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
}
