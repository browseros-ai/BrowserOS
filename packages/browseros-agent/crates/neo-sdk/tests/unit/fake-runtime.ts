/**
 * A fake `__neo_runtime` that answers the tabs wire contract as neo-runtime does: `pages` and
 * `initializer` getters on Rust-minted guids, `url`/`isClosed` only for guids the run was
 * handed, and `newPage`/`close`/`runBeforeUnload`/`bringToFront`/`click` calls with
 * Playwright's replies. Tests read back every message and getter request the SDK sent.
 */

import type { WireError } from '../../src/connection'
import { createRun } from '../../src/index'
import type { CreatedPage } from '../../src/runtime-bridge'
import { eventless } from './bridge-stub'

export type Message = {
  id: number
  guid: string
  method: string
  params: unknown
  metadata: unknown
}

export type Tab = { id: string; url: string }
export type Reply = {
  result?: unknown
  error?: { error: WireError }
  log?: string[]
}

/** The `metadata` of a call with no timeout, sent from outside an agent script. */
export const NO_TIMEOUT = { location: { file: 'run.js' }, timeout: 0 }

/** Playwright's reply to a call on a closed page or an unknown guid. */
export const TARGET_CLOSED: Reply = {
  error: {
    error: {
      name: 'TargetClosedError',
      message: 'Target page, context or browser has been closed',
    },
  },
  log: [],
}

export function failure(name: string, message: string): Reply {
  return { error: { error: { name, message } }, log: [] }
}

/** Rust's `parseURL` answer, with Bun's WHATWG parser standing in for Rust's. */
function parsed(text: string): string {
  let url: URL
  try {
    url = new URL(text)
  } catch {
    return JSON.stringify(null)
  }
  const { href, origin, protocol, username, password, host } = url
  const { hostname, port, pathname, search, hash } = url
  return JSON.stringify({
    href,
    origin,
    protocol,
    username,
    password,
    host,
    hostname,
    port,
    pathname,
    search,
    hash,
    searchParams: [...url.searchParams],
  })
}

const pageGuid = (tab: Tab) => `page@${tab.id}`
const frameGuid = (tab: Tab) => `frame@${tab.id}`

function created(tab: Tab): CreatedPage {
  return {
    guid: pageGuid(tab),
    initializer: { mainFrame: { guid: frameGuid(tab) }, isClosed: false },
    mainFrame: {
      guid: frameGuid(tab),
      initializer: { url: tab.url, name: '', loadStates: [] },
    },
  }
}

export class FakeRuntime {
  /** The context's open tabs, in tab order. Tests edit it to move or close tabs behind the
   * SDK's back. */
  tabs: Tab[]
  /** Guids handed to the run: the run page, every `pages` record, every `newPage` result. */
  readonly handed = new Set<string>()
  readonly messages: Message[] = []
  readonly gets: { guid: string; field: string }[] = []
  /** The guid of the tab `bringToFront` last activated. */
  active: string | undefined
  /** Answers a call in place of the model when it returns a reply. */
  answer: ((message: Message) => Reply | undefined) | undefined
  /** When set (for every call, or those the predicate picks), a call waits for its abort and
   * then replies as neo-runtime replies an aborted call (`AbortError` with the reason, the log
   * line `progress.rs` writes). */
  hold: boolean | ((message: Message) => boolean) = false
  /** Every `abort(id, reason)` the SDK sent, in order. */
  readonly aborts: { id: number; reason: string }[] = []
  readonly #held = new Map<number, (reply: Reply) => void>()
  #nextTab = 1

  constructor(tabs: Tab[]) {
    this.tabs = [...tabs]
  }

  get(request: string): string {
    const { guid, field, url } = JSON.parse(request)
    this.gets.push({ guid, field })
    if (field === 'parseURL') return parsed(url)
    if (guid === 'context@default' && field === 'pages') {
      for (const tab of this.tabs) this.handed.add(pageGuid(tab))
      return JSON.stringify(this.tabs.map(created))
    }
    if (!this.handed.has(guid))
      throw new Error(`unknown guid ${JSON.stringify(guid)}`)
    const tab = this.#open(guid)
    if (field === 'url') return JSON.stringify(tab ? tab.url : null)
    if (field === 'isClosed') return JSON.stringify(!tab)
    if (field === 'initializer' && tab) return JSON.stringify(created(tab))
    throw new Error(`bad getter request: ${request}`)
  }

  /** Synchronous up to its reply, as Rust starts a call inside the native call: `newPage`'s
   * tab exists before the script's next line runs. */
  async call(text: string): Promise<string> {
    const message: Message = JSON.parse(text)
    this.messages.push(message)
    if (this.hold === true || (this.hold && this.hold(message))) {
      const held = await new Promise<Reply>((resolve) =>
        this.#held.set(message.id, resolve),
      )
      return JSON.stringify({ id: message.id, ...held })
    }
    const reply = this.answer?.(message) ?? this.#serve(message)
    return JSON.stringify({ id: message.id, ...reply })
  }

  /** neo-runtime's `abort`: ends a held call with `AbortError`; any other id is ignored. */
  abort(id: number, reason: string): void {
    this.aborts.push({ id, reason })
    const held = this.#held.get(id)
    this.#held.delete(id)
    held?.({
      error: { error: { name: 'AbortError', message: reason } },
      log: [`  - operation was aborted: ${reason}`],
    })
  }

  #serve({ guid, method }: Message): Reply {
    if (guid === 'context@default' && method === 'newPage') {
      const tab = { id: `NEW${this.#nextTab++}`, url: 'about:blank' }
      this.tabs.push(tab)
      this.handed.add(pageGuid(tab))
      return { result: { page: { guid: pageGuid(tab) } } }
    }
    const page = guid.replace(/^frame@/, 'page@')
    if (!this.handed.has(page)) return TARGET_CLOSED
    const tab = this.#open(page)
    if (method === 'close') {
      if (tab) this.tabs.splice(this.tabs.indexOf(tab), 1)
      return {}
    }
    if (!tab) return TARGET_CLOSED
    if (method === 'runBeforeUnload') {
      this.tabs.splice(this.tabs.indexOf(tab), 1)
      return {}
    }
    if (method === 'bringToFront') this.active = page
    return {}
  }

  #open(guid: string): Tab | undefined {
    return this.tabs.find((tab) => pageGuid(tab) === guid)
  }
}

export const RUN_TAB: Tab = { id: 'RUN', url: 'https://run.test/' }
export const OTHER_TAB: Tab = { id: 'B', url: 'https://b.test/' }

/** A run on `tabs[0]`, with `tabs` open in its context and the fake installed as
 * `__neo_runtime`. */
export function start(tabs: Tab[] = [RUN_TAB]) {
  const [runTab = RUN_TAB] = tabs
  const fake = new FakeRuntime(tabs)
  fake.handed.add(pageGuid(runTab))
  Object.assign(globalThis, {
    __neo_runtime: eventless({
      get: fake.get.bind(fake),
      call: fake.call.bind(fake),
      abort: fake.abort.bind(fake),
    }),
  })
  const run = createRun({
    page: pageGuid(runTab),
    mainFrame: frameGuid(runTab),
    context: 'context@default',
    browser: 'browser@default',
    url: runTab.url,
    testIdAttribute: 'data-testid',
    lineOffset: 2,
  })
  return { fake, ...run }
}

/** Fails unless `actual` holds exactly the `expected` objects, by identity, in order. */
export function sameObjects(
  actual: readonly object[],
  expected: readonly object[],
): boolean {
  return (
    actual.length === expected.length &&
    actual.every((item, index) => item === expected[index])
  )
}
