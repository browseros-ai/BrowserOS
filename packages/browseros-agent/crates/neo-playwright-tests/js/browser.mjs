/**
 * Owns one private headless browser and its independent evidence connection. The test runner
 * and Neo worker connect to endpoint themselves. Nothing collected here supplies a test API.
 */
import { execFileSync, spawn } from 'node:child_process'
import { closeSync, openSync, writeSync } from 'node:fs'
import { mkdir, mkdtemp, open, readFile, rm, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { isDeepStrictEqual } from 'node:util'

const START_TIMEOUT_MS = 20_000
const REPLY_TIMEOUT_MS = 10_000
const CLOSE_TIMEOUT_MS = 30_000
const HELPER_GRACE_MS = 5_000
const KILL_TIMEOUT_MS = 2_000
const SNAPSHOT_TIMEOUT_MS = 20_000
const MAX_MESSAGE_BYTES = 32 * 1024 * 1024
const MAX_RAW_BYTES = 64 * 1024 * 1024
const MAX_EVENTS = 50_000
const WORLD = '__neo_conformance_observer__'
const BINDING = '__neo_conformance_evidence__'
const STATE = '__neo_conformance_observer_state__'
const TARGET_FILTER = [{ type: 'page' }, { exclude: true }]
const FRAME_FILTER = [{ type: 'iframe' }, { exclude: true }]

function bounded(promise, timeoutMs, message) {
  let timer
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), timeoutMs)
    }),
  ]).finally(() => clearTimeout(timer))
}

/**
 * Correlates CDP replies on its own socket. A disconnect or deadline rejects every affected
 * caller, so an observer failure cannot leave a cargo case waiting forever.
 */
export class Cdp {
  #socket
  #nextId = 1
  #pending = new Map()
  #listeners = new Set()
  #disconnectListeners = new Set()
  #error
  #timeoutMs
  #closed
  #resolveClosed

  constructor(socket, timeoutMs = REPLY_TIMEOUT_MS) {
    this.#socket = socket
    this.#timeoutMs = timeoutMs
    this.#closed = new Promise((resolveClosed) => {
      this.#resolveClosed = resolveClosed
    })
    socket.addEventListener('message', ({ data }) => this.#receive(data))
    socket.addEventListener('close', () => {
      this.#disconnect(new Error('CDP disconnected'))
      this.#resolveClosed()
    })
    socket.addEventListener('error', () => {
      this.#disconnect(new Error('CDP WebSocket error'))
    })
  }

  static async connect(endpoint, { timeoutMs = REPLY_TIMEOUT_MS } = {}) {
    const socket = new WebSocket(endpoint)
    const cdp = new Cdp(socket, timeoutMs)
    try {
      await bounded(
        new Promise((resolveOpen, reject) => {
          socket.addEventListener('open', resolveOpen, { once: true })
          cdp.onDisconnect(reject)
        }),
        timeoutMs,
        'CDP connection timed out',
      )
      return cdp
    } catch (error) {
      await cdp.close()
      throw error
    }
  }

  send(method, params = {}, sessionId) {
    if (this.#error) return Promise.reject(this.#error)
    return new Promise((resolveReply, reject) => {
      const id = this.#nextId++
      const timer = setTimeout(() => {
        this.#pending.delete(id)
        reject(new Error(`${method}: CDP reply timed out`))
      }, this.#timeoutMs)
      this.#pending.set(id, {
        method,
        sessionId,
        resolve: resolveReply,
        reject,
        timer,
      })
      try {
        this.#socket.send(JSON.stringify({ id, method, params, sessionId }))
      } catch (error) {
        clearTimeout(timer)
        this.#pending.delete(id)
        reject(error)
      }
    })
  }

  /** Listeners receive complete event envelopes, including the flattened sessionId. */
  subscribe(listener) {
    this.#listeners.add(listener)
    return () => this.#listeners.delete(listener)
  }

  onDisconnect(listener) {
    if (this.#error) listener(this.#error)
    else this.#disconnectListeners.add(listener)
    return () => this.#disconnectListeners.delete(listener)
  }

  async close() {
    this.#disconnect(new Error('CDP connection closed'))
    try {
      this.#socket.close()
    } catch {}
    await bounded(this.#closed, 500, 'CDP close timed out').catch(() => {})
  }

  #disconnect(error) {
    if (this.#error) return
    this.#error = error
    for (const { reject, timer } of this.#pending.values()) {
      clearTimeout(timer)
      reject(error)
    }
    this.#pending.clear()
    for (const listener of this.#disconnectListeners) listener(error)
    this.#disconnectListeners.clear()
  }

  #receive(data) {
    try {
      if (
        typeof data !== 'string' ||
        Buffer.byteLength(data) > MAX_MESSAGE_BYTES
      )
        throw new Error('CDP message is binary or exceeds the evidence budget')
      const message = JSON.parse(data)
      if (!message || typeof message !== 'object')
        throw new Error('CDP message is not an object')
      if (message.id !== undefined) {
        const pending = this.#pending.get(message.id)
        if (!pending) return
        this.#pending.delete(message.id)
        clearTimeout(pending.timer)
        if (message.error) {
          const error = new Error(`${pending.method}: ${message.error.message}`)
          error.code = message.error.code
          pending.reject(error)
        } else pending.resolve(message.result)
        return
      }
      if (typeof message.method !== 'string')
        throw new Error('CDP event has no method')
      if (message.method === 'Target.detachedFromTarget') {
        // Chromium may drop a command's reply when its flattened target disappears. The
        // session owns that deadline; unrelated targets keep their connection and requests.
        for (const [id, pending] of this.#pending) {
          if (pending.sessionId !== message.params?.sessionId) continue
          clearTimeout(pending.timer)
          this.#pending.delete(id)
          const error = new Error(`${pending.method}: CDP session closed`)
          error.code = 'CDP_SESSION_CLOSED'
          pending.reject(error)
        }
      }
      for (const listener of this.#listeners) listener(message)
    } catch (error) {
      this.#disconnect(error)
      try {
        this.#socket.close()
      } catch {}
    }
  }
}

export async function connect(endpoint, options) {
  return await Cdp.connect(endpoint, options)
}

// These are the pinned runner's emitted defaults, not a replacement context API. Only the
// already-owned blank page is configured before a test can receive an engine object.
const contextDefaults = {
  acceptDownloads: true,
  bypassCSP: false,
  colorScheme: 'light',
  contrast: 'no-preference',
  forcedColors: 'none',
  hasTouch: false,
  ignoreHTTPSErrors: false,
  isMobile: false,
  javaScriptEnabled: true,
  locale: 'en-US',
  offline: false,
  reducedMotion: 'no-preference',
  serviceWorkers: 'allow',
  viewport: { width: 1280, height: 720 },
}

/** Captures the upstream fixture preconditions and rejects anything this initial-page
 * bootstrap cannot establish. Unsupported configuration is evidence loss, never a Neo API. */
export function contextBootstrapPlan(options = {}, fixtureOptions = {}) {
  const effective = {
    ...structuredClone(contextDefaults),
    ...Object.fromEntries(
      Object.entries(options).filter(([, value]) => value !== undefined),
    ),
  }
  const limitations = []
  for (const [key, value] of Object.entries(options)) {
    if (value === undefined) continue
    if (
      !Object.hasOwn(contextDefaults, key) ||
      !isDeepStrictEqual(value, contextDefaults[key])
    )
      limitations.push(
        `Initial-page bootstrap does not support context option ${key}`,
      )
  }
  const fixtureDefaults = {
    actionTimeout: 0,
    navigationTimeout: 0,
    testIdAttribute: 'data-testid',
  }
  const fixture = { ...fixtureDefaults, ...fixtureOptions }
  for (const [key, value] of Object.entries(fixture))
    if (!isDeepStrictEqual(value, fixtureDefaults[key]))
      limitations.push(
        `Initial-page bootstrap does not support fixture option ${key}`,
      )
  return {
    scope: 'owned-initial-blank-page',
    initialUrl: 'about:blank',
    contextOptions: effective,
    fixtureOptions: fixture,
    limitations,
  }
}

/** This session must stay attached until the engine finishes because CDP can clear emulation
 * on detach. No value read from this infrastructure connection is returned to a test. */
export async function bootstrapInitialPage(
  cdp,
  pageId,
  plan,
  sessionId,
  downloadPath,
) {
  if (plan.limitations.length) throw new Error(plan.limitations.join('; '))
  const { targetInfo } = await cdp.send('Target.getTargetInfo', {
    targetId: pageId,
  })
  if (targetInfo?.type !== 'page' || targetInfo.url !== 'about:blank')
    throw new Error('Context bootstrap requires the owned initial blank page')
  if (!sessionId)
    throw new Error('Context bootstrap requires an owned observer session')
  const { width, height } = plan.contextOptions.viewport
  await cdp.send(
    'Emulation.setDeviceMetricsOverride',
    {
      width,
      height,
      screenWidth: width,
      screenHeight: height,
      deviceScaleFactor: 1,
      mobile: false,
      screenOrientation: { angle: 0, type: 'landscapePrimary' },
    },
    sessionId,
  )
  await cdp.send(
    'Emulation.setEmulatedMedia',
    {
      features: [
        { name: 'prefers-color-scheme', value: 'light' },
        { name: 'prefers-contrast', value: 'no-preference' },
        { name: 'forced-colors', value: 'none' },
        { name: 'prefers-reduced-motion', value: 'no-preference' },
      ],
    },
    sessionId,
  )
  await cdp.send('Emulation.setLocaleOverride', { locale: 'en-US' }, sessionId)
  if (downloadPath)
    await cdp.send('Browser.setDownloadBehavior', {
      behavior: 'allowAndName',
      downloadPath,
      eventsEnabled: true,
    })
}

/** Normalizes BrowserOS's tab-strip identifiers against observer-owned pages. A missing or
 * ambiguous strip cannot be inferred from DOM focus, especially in headless mode. */
export function projectActivation(tabs, pages) {
  if (!Array.isArray(tabs))
    throw new Error('Browser.getTabs did not return a tab strip')
  const live = new Map(
    pages.filter((page) => !page.closed).map((page) => [page.targetId, page]),
  )
  const windows = new Map()
  const seen = new Set()
  const projection = tabs.map((tab) => {
    const page = live.get(tab.targetId)
    if (!page || seen.has(tab.targetId))
      throw new Error('Tab strip contains an unobserved or duplicate page')
    if (
      !Number.isInteger(tab.windowId) ||
      !Number.isInteger(tab.index) ||
      tab.index < 0 ||
      typeof tab.isActive !== 'boolean'
    )
      throw new Error('Tab strip lacks activation, order, or window identity')
    seen.add(tab.targetId)
    if (!windows.has(tab.windowId)) windows.set(tab.windowId, windows.size)
    return {
      page: page.ordinal,
      window: windows.get(tab.windowId),
      index: tab.index,
      active: tab.isActive,
    }
  })
  if (seen.size !== live.size)
    throw new Error('Tab strip does not cover every live observed page')
  for (const window of windows.values()) {
    const strip = projection.filter((tab) => tab.window === window)
    if (
      strip.filter((tab) => tab.active).length !== 1 ||
      new Set(strip.map((tab) => tab.index)).size !== strip.length
    )
      throw new Error('Tab strip has ambiguous active tab or order')
  }
  return { status: 'complete', tabs: projection }
}

/**
 * Runs only in the observer's isolated world. Native DOM wrappers stay independent of the
 * page's JavaScript prototypes. Its only writes install listeners and private bookkeeping.
 */
function installObserver(binding, stateName) {
  if (globalThis[stateName]) return
  let sequence = 0
  let eventCount = 0
  let overflow = false
  let checkpointed = false

  function path(node) {
    const parts = []
    for (let current = node; current; ) {
      if (current.nodeType === Node.DOCUMENT_NODE) {
        parts.unshift('#document')
        break
      }
      if (current.nodeType === Node.DOCUMENT_FRAGMENT_NODE && current.host) {
        parts.unshift(`::shadow(${current.mode})`)
        current = current.host
        continue
      }
      let ordinal = 1
      for (
        let sibling = current.previousSibling;
        sibling;
        sibling = sibling.previousSibling
      )
        if (
          sibling.nodeType === current.nodeType &&
          sibling.nodeName === current.nodeName
        )
          ordinal++
      const id =
        current.nodeType === Node.ELEMENT_NODE
          ? current.getAttribute('id')
          : null
      parts.unshift(
        `${current.nodeName.toLowerCase()}[${ordinal}]${id === null ? '' : `#${JSON.stringify(id)}`}`,
      )
      current = current.parentNode
    }
    return parts.join('/')
  }

  function form(node) {
    if (node instanceof HTMLInputElement)
      return {
        value: node.value,
        checked: node.checked,
        indeterminate: node.indeterminate,
        selectionStart: node.selectionStart,
        selectionEnd: node.selectionEnd,
        selectionDirection: node.selectionDirection,
      }
    if (node instanceof HTMLTextAreaElement)
      return {
        value: node.value,
        selectionStart: node.selectionStart,
        selectionEnd: node.selectionEnd,
        selectionDirection: node.selectionDirection,
      }
    if (node instanceof HTMLSelectElement)
      return { value: node.value, selectedIndex: node.selectedIndex }
    if (node instanceof HTMLOptionElement)
      return { value: node.value, selected: node.selected }
    return null
  }

  function snapshot() {
    let count = 0
    let characters = 0
    const tree = {}
    const pending = [[document, tree]]
    while (pending.length) {
      const [node, result] = pending.pop()
      if (++count > 50_000) return { incomplete: 'dom-node-budget' }
      result.type = node.nodeType
      result.name = node.nodeName
      if (node.nodeValue !== null) {
        characters += node.nodeValue.length
        result.text = node.nodeValue
      }
      if (node.nodeType === Node.ELEMENT_NODE) {
        result.attributes = Array.from(node.attributes, ({ name, value }) => {
          characters += name.length + value.length
          return [name, value]
        })
        const value = form(node)
        if (value) {
          result.form = value
          characters += typeof value.value === 'string' ? value.value.length : 0
        }
      }
      if (node.nodeType === Node.DOCUMENT_FRAGMENT_NODE && node.host)
        result.shadow = node.mode
      const children = Array.from(node.childNodes)
      if (node.nodeType === Node.ELEMENT_NODE && node.shadowRoot)
        children.push(node.shadowRoot)
      result.children = children.map(() => ({}))
      for (let index = children.length - 1; index >= 0; index--)
        pending.push([children[index], result.children[index]])
      if (characters > 8 * 1024 * 1024) return { incomplete: 'dom-text-budget' }
    }
    const selection = document.getSelection()
    const ranges = []
    for (let index = 0; selection && index < selection.rangeCount; index++) {
      const range = selection.getRangeAt(index)
      ranges.push({
        start: path(range.startContainer),
        startOffset: range.startOffset,
        end: path(range.endContainer),
        endOffset: range.endOffset,
      })
    }
    let deepFocus = document.activeElement
    while (deepFocus?.shadowRoot?.activeElement)
      deepFocus = deepFocus.shadowRoot.activeElement
    return {
      url: location.href,
      title: document.title,
      // New tabs and frames are read as they are. Their geometry is never inferred from
      // the bootstrap of the initial page or changed by this independent observer.
      viewport: { width: innerWidth, height: innerHeight, devicePixelRatio },
      tree,
      focus: document.activeElement ? path(document.activeElement) : null,
      deepFocus: deepFocus ? path(deepFocus) : null,
      selection: selection
        ? {
            type: selection.type,
            collapsed: selection.isCollapsed,
            anchor: selection.anchorNode ? path(selection.anchorNode) : null,
            anchorOffset: selection.anchorOffset,
            focus: selection.focusNode ? path(selection.focusNode) : null,
            focusOffset: selection.focusOffset,
            ranges,
          }
        : null,
    }
  }

  function emit(payload) {
    const envelope = { sequence: ++sequence, ...payload }
    const encoded = JSON.stringify(envelope)
    if (encoded.length > 10 * 1024 * 1024) {
      globalThis[binding](
        JSON.stringify({
          sequence,
          kind: 'gap',
          reason: 'binding-message-budget',
        }),
      )
      return
    }
    globalThis[binding](encoded)
  }

  const fields = [
    'isTrusted',
    'bubbles',
    'cancelable',
    'composed',
    'defaultPrevented',
    'altKey',
    'ctrlKey',
    'metaKey',
    'shiftKey',
    'key',
    'code',
    'location',
    'repeat',
    'isComposing',
    'charCode',
    'keyCode',
    'which',
    'data',
    'inputType',
    'button',
    'buttons',
    'detail',
    'clientX',
    'clientY',
    'pageX',
    'pageY',
    'screenX',
    'screenY',
    'movementX',
    'movementY',
    'pointerId',
    'pointerType',
    'isPrimary',
    'width',
    'height',
    'pressure',
    'tangentialPressure',
    'tiltX',
    'tiltY',
    'twist',
  ]
  const eventTypes = [
    'mousedown',
    'mouseup',
    'mousemove',
    'mouseover',
    'mouseout',
    'mouseenter',
    'mouseleave',
    'click',
    'dblclick',
    'auxclick',
    'contextmenu',
    'wheel',
    'pointerdown',
    'pointerup',
    'pointermove',
    'pointerover',
    'pointerout',
    'pointerenter',
    'pointerleave',
    'pointercancel',
    'gotpointercapture',
    'lostpointercapture',
    'keydown',
    'keyup',
    'keypress',
    'beforeinput',
    'input',
    'change',
    'focus',
    'blur',
    'focusin',
    'focusout',
    'compositionstart',
    'compositionupdate',
    'compositionend',
  ]
  const receive = (event) => {
    if (++eventCount > 50_000) {
      if (!overflow) emit({ kind: 'gap', reason: 'document-event-budget' })
      overflow = true
      return
    }
    const received = { type: event.type }
    for (const field of fields)
      if (field in event) received[field] = event[field]
    const target = event.composedPath()[0] ?? event.target
    received.target = target instanceof Node ? path(target) : null
    received.targetState = target instanceof Node ? form(target) : null
    received.relatedTarget =
      event.relatedTarget instanceof Node ? path(event.relatedTarget) : null
    if (event instanceof WheelEvent) {
      received.deltaX = event.deltaX
      received.deltaY = event.deltaY
      received.deltaZ = event.deltaZ
      received.deltaMode = event.deltaMode
    }
    if (typeof event.getTargetRanges === 'function')
      received.targetRanges = Array.from(event.getTargetRanges(), (range) => ({
        start: path(range.startContainer),
        startOffset: range.startOffset,
        end: path(range.endContainer),
        endOffset: range.endOffset,
      }))
    emit({ kind: 'event', event: received })
  }
  for (const type of eventTypes)
    globalThis.addEventListener(type, receive, true)
  // Chromium skips pagehide when it replaces an initial empty document. beforeunload
  // still runs. Its microtask sees changes from later handlers before navigation commits.
  // Only an unchanged empty document can use this checkpoint as its final DOM evidence.
  new MutationObserver(() => {
    if (checkpointed) {
      checkpointed = false
      emit({ kind: 'checkpoint-invalidated' })
    }
  }).observe(document, {
    subtree: true,
    childList: true,
    attributes: true,
    characterData: true,
  })
  globalThis.addEventListener(
    'beforeunload',
    () => {
      queueMicrotask(() => {
        emit({
          kind: 'checkpoint',
          dom: snapshot(),
          empty:
            location.href === 'about:blank' &&
            document.documentElement?.outerHTML ===
              '<html><head></head><body></body></html>',
        })
        checkpointed = true
      })
    },
    true,
  )
  globalThis.addEventListener(
    'pagehide',
    (event) => {
      emit({ kind: 'terminal', persisted: event.persisted, dom: snapshot() })
    },
    true,
  )
  globalThis.addEventListener(
    'pageshow',
    (event) => {
      if (event.persisted) emit({ kind: 'resume', url: location.href })
    },
    true,
  )
  Object.defineProperty(globalThis, stateName, {
    value: { snapshot: () => ({ sequence, dom: snapshot() }) },
  })
  emit({ kind: 'ready', url: location.href })
}

const OBSERVER_SOURCE = `(${installObserver.toString()})(${JSON.stringify(BINDING)}, ${JSON.stringify(STATE)})`
const READ_SOURCE = `globalThis[${JSON.stringify(STATE)}]?.snapshot()`

function processListing() {
  const output = execFileSync('ps', ['-Ao', 'pid=,pgid=,state=,command='], {
    encoding: 'utf8',
    timeout: 2_000,
    maxBuffer: 16 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  return output.split('\n').flatMap((line) => {
    const match = /^\s*(\d+)\s+(\d+)\s+(\S+)\s+(.*)$/.exec(line)
    return match
      ? [
          {
            pid: Number(match[1]),
            group: Number(match[2]),
            state: match[3],
            command: match[4],
          },
        ]
      : []
  })
}

function ownedProcesses(pid, profile) {
  return processListing().filter(
    (entry) =>
      !entry.state.startsWith('Z') &&
      (entry.group === pid || entry.command.includes(profile)),
  )
}

function killOwned(pid, profile, groupVerified) {
  const processes = ownedProcesses(pid, profile)
  if (groupVerified && processes.some((entry) => entry.group === pid)) {
    try {
      process.kill(-pid, 'SIGKILL')
    } catch (error) {
      if (error.code !== 'ESRCH') throw error
    }
  }
  for (const entry of processes) {
    try {
      process.kill(entry.pid, 'SIGKILL')
    } catch (error) {
      if (error.code !== 'ESRCH') throw error
    }
  }
}

/**
 * Tracks pages, flattened sessions, and document epochs separately. Protocol IDs live only
 * in these maps and raw diagnostics. Published evidence uses page/frame/document ordinals.
 */
export class OwnedBrowser {
  #child
  #exited
  #exit
  #profile
  #outputDir
  #groupVerified = false
  #cdp
  #pages = new Map()
  #sessions = new Map()
  #frames = new Map()
  #tasks = new Set()
  #incomplete = []
  #events = []
  #rawFd
  #rawBytes = 0
  #closePromise
  #closing = false
  #lastSnapshot
  #activation = { status: 'incomplete', tabs: [] }
  #signal
  #outputReady = false

  static async start({ executable, outputDir, signal }) {
    if (typeof executable !== 'string' || !executable)
      throw new TypeError('OwnedBrowser.start requires an executable')
    if (typeof outputDir !== 'string' || !outputDir)
      throw new TypeError('OwnedBrowser.start requires an outputDir')
    const owner = new OwnedBrowser()
    owner.#outputDir = resolve(outputDir)
    owner.#signal = signal
    // Startup owns partially acquired files and processes too. Abort is latched immediately,
    // while cleanup waits for startup to stop acquiring resources before closing them.
    const abort = () => {
      owner.#closing = true
      // Closing the known socket and the direct owned child wakes in-flight startup I/O.
      // Full cleanup below still waits until startup can no longer acquire new resources.
      void owner.#cdp?.close().catch(() => {})
      owner.#child?.kill('SIGTERM')
    }
    signal?.addEventListener('abort', abort, { once: true })
    try {
      owner.#checkStarting()
      await owner.#start(executable)
      owner.#checkStarting()
      return owner
    } catch (error) {
      try {
        await owner.close()
      } catch (cleanupError) {
        throw new AggregateError(
          [error, cleanupError],
          'Browser startup and cleanup failed',
        )
      }
      throw error
    } finally {
      signal?.removeEventListener('abort', abort)
    }
  }

  #checkStarting() {
    this.#signal?.throwIfAborted()
    if (this.#closing) throw new Error('Owned browser startup was cancelled')
  }

  async #start(executable) {
    await mkdir(this.#outputDir, { recursive: true, mode: 0o700 })
    this.#outputReady = true
    this.#checkStarting()
    this.#profile = await mkdtemp(join(this.#outputDir, 'profile-'))
    this.#checkStarting()
    this.#rawFd = openSync(
      join(this.#outputDir, 'observer.raw.jsonl'),
      'wx',
      0o600,
    )
    const stdout = await open(
      join(this.#outputDir, 'browser.stdout.log'),
      'wx',
      0o600,
    )
    let stderr
    try {
      stderr = await open(
        join(this.#outputDir, 'browser.stderr.log'),
        'wx',
        0o600,
      )
      this.#checkStarting()
      this.#child = spawn(
        executable,
        [
          '--headless=new',
          // BrowserOS's sidecar has an independent lifecycle and must not escape this owner.
          '--disable-browseros-server',
          '--remote-debugging-port=0',
          `--user-data-dir=${this.#profile}`,
          '--no-first-run',
          '--lang=en-US',
          '--window-size=1280,720',
          '--force-device-scale-factor=1',
          'about:blank',
        ],
        { detached: true, stdio: ['ignore', stdout.fd, stderr.fd] },
      )
      this.#exited = new Promise((resolveExit) => {
        const finish = (exit) => {
          if (this.#exit) return
          this.#exit = exit
          resolveExit(exit)
        }
        this.#child.once('exit', (code, signal) => finish({ code, signal }))
        this.#child.once('error', (error) => finish({ error: error.message }))
      })
    } finally {
      await Promise.allSettled([stdout.close(), stderr?.close()])
    }
    if (!this.#child.pid) {
      await this.#exited
      throw new Error(`Browser failed to spawn: ${this.#exit?.error}`)
    }
    // A negative-PID signal is safe only after proving that this child leads its own group.
    const group = execFileSync(
      'ps',
      ['-o', 'pgid=', '-p', String(this.#child.pid)],
      {
        encoding: 'utf8',
        timeout: 2_000,
        stdio: ['ignore', 'pipe', 'pipe'],
      },
    ).trim()
    this.#groupVerified = group === String(this.#child.pid)
    if (!this.#groupVerified)
      throw new Error('Owned browser did not lead its own process group')
    const deadline = Date.now() + START_TIMEOUT_MS
    while (!this.endpoint) {
      this.#checkStarting()
      if (this.#exit)
        throw new Error('Browser exited before publishing its CDP endpoint')
      try {
        const [port, path] = (
          await readFile(join(this.#profile, 'DevToolsActivePort'), 'utf8')
        ).split('\n')
        if (
          /^\d+$/.test(port) &&
          Number(port) > 0 &&
          Number(port) <= 65535 &&
          path?.startsWith('/devtools/browser/')
        )
          this.endpoint = `ws://127.0.0.1:${port}${path}`
      } catch (error) {
        if (error.code !== 'ENOENT') throw error
      }
      if (!this.endpoint) {
        if (Date.now() >= deadline)
          throw new Error('Browser did not publish its CDP endpoint in time')
        await sleep(25)
      }
    }
    this.#checkStarting()
    this.#cdp = await Cdp.connect(this.endpoint)
    this.#checkStarting()
    this.#cdp.subscribe((message) => this.#receive(message))
    this.#cdp.onDisconnect((error) => {
      if (!this.#closing) this.#gap('observer-disconnected', undefined, error)
    })
    this.version = await this.#cdp.send('Browser.getVersion')
    await this.#cdp.send('Target.setDiscoverTargets', { discover: true })
    await this.#cdp.send('Target.setAutoAttach', {
      autoAttach: true,
      waitForDebuggerOnStart: true,
      flatten: true,
      filter: TARGET_FILTER,
    })
    const { targetInfos } = await this.#cdp.send('Target.getTargets')
    for (const target of targetInfos)
      if (target.type === 'page') this.#page(target)
    await this.#drain()
    this.#checkStarting()
    const initial = [...this.#pages.values()].find(
      (page) => !page.closed && page.url === 'about:blank',
    )
    if (!initial) throw new Error('Owned browser had no initial blank page')
    this.pageId = initial.targetId
    await bootstrapInitialPage(
      this.#cdp,
      this.pageId,
      contextBootstrapPlan(),
      initial.session?.id,
      join(this.#outputDir, 'downloads'),
    )
    this.#checkStarting()
    for (const page of this.#pages.values())
      if (!page.closed && !page.session) this.#gap('page-not-attached', page)
  }

  #raw(record) {
    if (this.#rawFd === undefined) return
    const line = `${JSON.stringify(record)}\n`
    if (this.#rawBytes + Buffer.byteLength(line) > MAX_RAW_BYTES) {
      this.#gap('raw-evidence-budget')
      return
    }
    try {
      writeSync(this.#rawFd, line)
      this.#rawBytes += Buffer.byteLength(line)
    } catch {
      this.#gap('raw-evidence-write')
    }
  }

  #gap(reason, page, error) {
    const issue = { reason, ...(page ? { page: page.ordinal } : {}) }
    if (
      !this.#incomplete.some(
        (item) => item.reason === reason && item.page === issue.page,
      )
    )
      this.#incomplete.push(issue)
    if (error && this.#rawFd !== undefined) {
      try {
        writeSync(
          this.#rawFd,
          `${JSON.stringify({ kind: 'error', ...issue, error: String(error) })}\n`,
        )
      } catch {}
    }
  }

  #event(event) {
    if (this.#events.length >= MAX_EVENTS) this.#gap('event-budget')
    else this.#events.push(event)
  }

  #page(info) {
    let page = this.#pages.get(info.targetId)
    if (!page) {
      page = {
        targetId: info.targetId,
        ordinal: this.#pages.size,
        url: info.url,
        opener: this.#pages.get(info.openerId)?.ordinal ?? null,
        closed: false,
        nextFrame: 0,
        order: 0,
        frames: [],
      }
      this.#pages.set(info.targetId, page)
      this.#event({
        kind: 'page-opened',
        page: page.ordinal,
        opener: page.opener,
        url: page.url,
      })
    } else page.url = info.url
    return page
  }

  #frame(frameId, parentId, page) {
    let frame = this.#frames.get(frameId)
    if (!frame) {
      const parent = this.#frames.get(parentId)
      frame = {
        id: frameId,
        page,
        ordinal: page.nextFrame++,
        parent: parent?.ordinal ?? null,
        documents: [],
      }
      this.#frames.set(frameId, frame)
      page.frames.push(frame)
      if (parentId && !parent) this.#gap('frame-parent-not-observed', page)
    }
    return frame
  }

  #task(promise, page, reason) {
    const task = promise
      .catch((error) => this.#gap(reason, page, error))
      .finally(() => this.#tasks.delete(task))
    this.#tasks.add(task)
  }

  async #drain() {
    await bounded(
      (async () => {
        while (this.#tasks.size) await Promise.all([...this.#tasks])
      })(),
      REPLY_TIMEOUT_MS,
      'Observer attachment did not finish',
    )
  }

  #receive(message) {
    this.#raw({ receivedAt: Date.now(), ...message })
    const { method, params = {}, sessionId } = message
    const session = this.#sessions.get(sessionId)
    if (
      method === 'Target.targetCreated' ||
      method === 'Target.targetInfoChanged'
    ) {
      if (params.targetInfo.type === 'page') this.#page(params.targetInfo)
    } else if (method === 'Target.attachedToTarget') {
      const info = params.targetInfo
      const page = info.type === 'page' ? this.#page(info) : session?.page
      const attached = {
        id: params.sessionId,
        targetId: info.targetId,
        page,
        contexts: new Map(),
        closed: false,
      }
      this.#sessions.set(attached.id, attached)
      if (page) {
        if (info.type === 'page') page.session = attached
        this.#task(this.#observe(attached), page, 'target-observer-install')
      } else {
        this.#gap('unowned-target-attached')
        this.#task(
          this.#cdp.send('Runtime.runIfWaitingForDebugger', {}, attached.id),
          undefined,
          'unowned-target-resume',
        )
      }
    } else if (method === 'Target.detachedFromTarget') {
      const detached = this.#sessions.get(params.sessionId)
      if (detached) {
        detached.closed = true
        for (const context of detached.contexts.values())
          this.#retireContext(detached, context.id)
      }
    } else if (method === 'Target.targetDestroyed') {
      const page = this.#pages.get(params.targetId)
      if (page) {
        page.closed = true
        this.#event({ kind: 'page-closed', page: page.ordinal, url: page.url })
      }
    } else if (method === 'Page.frameAttached' && session?.page) {
      this.#frame(params.frameId, params.parentFrameId, session.page)
    } else if (method === 'Page.frameDetached' && session?.page) {
      const frame = this.#frames.get(params.frameId)
      if (frame) frame.detached = params.reason !== 'swap'
    } else if (method === 'Page.frameNavigated' && session?.page) {
      const frame = this.#frame(
        params.frame.id,
        params.frame.parentId,
        session.page,
      )
      frame.url = params.frame.url
      if (!params.frame.parentId && session === session.page.session)
        session.page.url = frame.url
    } else if (method === 'Page.navigatedWithinDocument' && session?.page) {
      const frame = this.#frames.get(params.frameId)
      if (frame) {
        frame.url = params.url
        if (frame.parent === null) session.page.url = params.url
        this.#event({
          kind: 'same-document-navigation',
          page: session.page.ordinal,
          frame: frame.ordinal,
          url: params.url,
        })
      } else this.#gap('navigation-frame-not-observed', session.page)
    } else if (method === 'Runtime.executionContextCreated' && session) {
      const context = params.context
      if (context.name === WORLD)
        session.contexts.set(context.id, {
          id: context.id,
          frameId: context.auxData?.frameId,
        })
    } else if (method === 'Runtime.executionContextDestroyed' && session) {
      this.#retireContext(session, params.executionContextId)
    } else if (method === 'Runtime.executionContextsCleared' && session) {
      for (const context of session.contexts.values())
        this.#retireContext(session, context.id)
    } else if (method === 'DOM.shadowRootPushed' && session?.page) {
      this.#shadowCoverage(params.root, session.page)
    } else if (method === 'DOM.documentUpdated' && session?.page) {
      session.domCoverage = false
      this.#task(
        this.#watchDom(session),
        session.page,
        'document-shadow-coverage',
      )
    } else if (method === 'DOM.childNodeInserted' && session?.page) {
      this.#shadowCoverage(params.node, session.page)
    } else if (
      method === 'Runtime.bindingCalled' &&
      session &&
      params.name === BINDING
    ) {
      this.#binding(session, params)
    } else if (method === 'Inspector.detached' && session) {
      session.inspectorDetached = true
    } else if (method === 'Inspector.targetCrashed' && session?.page) {
      this.#gap('target-observer-detached', session.page)
    }
  }

  #retireContext(session, contextId) {
    const context = session.contexts.get(contextId)
    if (!context) return
    context.destroyed = true
    if (context.document) context.document.destroyed = true
  }

  #binding(session, params) {
    const context = session.contexts.get(params.executionContextId)
    if (!context) return this.#gap('binding-context-not-observed', session.page)
    let payload
    try {
      payload = JSON.parse(params.payload)
    } catch (error) {
      return this.#gap('binding-invalid-json', session.page, error)
    }
    if (payload.kind === 'ready') {
      if (!context.frameId)
        this.#gap('document-frame-not-observed', session.page)
      let frame = this.#frames.get(context.frameId)
      if (!frame) frame = this.#frame(context.frameId, undefined, session.page)
      const previous = frame.documents.at(-1)
      if (previous) previous.replaced = true
      context.document = {
        frame,
        epoch: frame.documents.length,
        url: payload.url,
        sequence: 0,
        terminal: false,
        dom: null,
      }
      frame.documents.push(context.document)
      this.#event({
        kind: 'document',
        page: session.page.ordinal,
        frame: frame.ordinal,
        epoch: context.document.epoch,
        url: payload.url,
      })
    }
    const document = context.document
    if (!document) return this.#gap('binding-before-document', session.page)
    if (payload.sequence !== document.sequence + 1)
      this.#gap('document-event-sequence-gap', session.page)
    document.sequence = payload.sequence
    if (payload.kind === 'event') {
      this.#event({
        kind: 'input',
        page: session.page.ordinal,
        frame: document.frame.ordinal,
        epoch: document.epoch,
        order: session.page.order++,
        ...payload.event,
      })
    } else if (payload.kind === 'terminal') {
      document.terminal = true
      document.dom = payload.dom
      if (payload.dom?.incomplete)
        this.#gap(payload.dom.incomplete, session.page)
    } else if (payload.kind === 'checkpoint') {
      document.checkpoint = payload
    } else if (payload.kind === 'checkpoint-invalidated') {
      document.checkpoint = undefined
    } else if (payload.kind === 'gap') this.#gap(payload.reason, session.page)
    else if (payload.kind === 'resume') {
      document.terminal = false
      this.#event({
        kind: 'document-resumed',
        page: session.page.ordinal,
        frame: document.frame.ordinal,
        epoch: document.epoch,
        url: payload.url,
      })
    }
  }

  async #observe(session) {
    try {
      // These independent domain commands must all finish before injecting the listener.
      // Pipelining them avoids spending several round trips on a page another client can
      // close immediately. The target pause protects script execution, not target closure.
      const [, , , , { frameTree }] = await Promise.all([
        this.#cdp.send(
          'Target.setAutoAttach',
          {
            autoAttach: true,
            waitForDebuggerOnStart: true,
            flatten: true,
            filter: FRAME_FILTER,
          },
          session.id,
        ),
        this.#cdp.send('Runtime.enable', {}, session.id),
        this.#cdp.send('Page.enable', {}, session.id),
        this.#cdp.send(
          'Runtime.addBinding',
          {
            name: BINDING,
            executionContextName: WORLD,
          },
          session.id,
        ),
        this.#cdp.send('Page.getFrameTree', {}, session.id),
      ])
      const frames = []
      const visit = (tree) => {
        this.#frame(tree.frame.id, tree.frame.parentId, session.page)
        frames.push(tree.frame)
        for (const child of tree.childFrames ?? []) visit(child)
      }
      visit(frameTree)
      await this.#cdp.send(
        'Page.addScriptToEvaluateOnNewDocument',
        {
          source: OBSERVER_SOURCE,
          worldName: WORLD,
          runImmediately: true,
        },
        session.id,
      )
      // Requesting the tree enables DOM's incremental shadow-root notifications. A window
      // capture listener cannot see non-composed events inside author shadow roots, even
      // when DOMSnapshot can read their final nodes. Report that coverage limit explicitly.
      await this.#watchDom(session)
      for (const frame of frames) {
        if (
          [...session.contexts.values()].some(
            (context) =>
              context.frameId === frame.id &&
              !context.destroyed &&
              context.document,
          )
        )
          continue
        const { executionContextId } = await this.#cdp.send(
          'Page.createIsolatedWorld',
          {
            frameId: frame.id,
            worldName: WORLD,
            grantUniveralAccess: false,
          },
          session.id,
        )
        const result = await this.#cdp.send(
          'Runtime.evaluate',
          {
            expression: OBSERVER_SOURCE,
            contextId: executionContextId,
            returnByValue: true,
          },
          session.id,
        )
        if (result.exceptionDetails)
          throw new Error('Observer installation threw in its isolated world')
      }
    } catch (error) {
      // A fully observed document can close while redundant initial-world setup is in
      // flight. Missing terminal evidence still fails below; closure alone is not a gap.
      if (
        !session.closed ||
        !session.contexts.size ||
        [...session.contexts.values()].some(
          (context) => !context.document?.terminal,
        )
      )
        throw error
    } finally {
      // Release even a failed installation. The gap remains evidence, while the test can
      // finish and report it instead of hanging behind our debugger pause.
      if (!session.closed) {
        try {
          await this.#cdp.send(
            'Runtime.runIfWaitingForDebugger',
            {},
            session.id,
          )
        } catch (error) {
          if (!session.closed)
            this.#gap('target-observer-release', session.page, error)
        }
      }
    }
  }

  /** A final read fence. Failure returns incomplete evidence rather than a false equality. */
  async snapshot() {
    if (this.#closing) {
      this.#gap('snapshot-after-close')
      return this.#view()
    }
    try {
      await bounded(
        this.#capture(),
        SNAPSHOT_TIMEOUT_MS,
        'Observer snapshot timed out',
      )
    } catch (error) {
      this.#gap('snapshot-failed', undefined, error)
    }
    this.#lastSnapshot = this.#view()
    await writeFile(
      join(this.#outputDir, 'observer.json'),
      `${JSON.stringify(this.#lastSnapshot, null, 2)}\n`,
      { mode: 0o600 },
    )
    return this.#lastSnapshot
  }

  async #capture() {
    const { targetInfos } = await this.#cdp.send('Target.getTargets')
    for (const info of targetInfos) {
      if (info.type !== 'page') continue
      if (!this.#pages.has(info.targetId))
        this.#gap('page-discovered-after-attach-gap')
      this.#page(info)
    }
    await this.#drain()
    const fences = []
    for (const session of this.#sessions.values()) {
      if (!session.page || session.closed || session.page.closed) continue
      for (const context of session.contexts.values()) {
        if (context.destroyed) continue
        const result = await this.#cdp.send(
          'Runtime.evaluate',
          {
            expression: READ_SOURCE,
            contextId: context.id,
            returnByValue: true,
          },
          session.id,
        )
        const value = result.result?.value
        if (result.exceptionDetails || !value || !context.document) {
          this.#gap('document-fence-missing', session.page)
          continue
        }
        context.document.dom = value.dom
        if (value.dom?.incomplete) this.#gap(value.dom.incomplete, session.page)
        if (value.sequence !== context.document.sequence)
          this.#gap('document-fence-sequence-gap', session.page)
        fences.push([session.page, context.document, value.sequence])
      }
      const snapshot = await this.#cdp.send(
        'DOMSnapshot.captureSnapshot',
        { computedStyles: [] },
        session.id,
      )
      session.dom = this.#projectDom(snapshot, session.page)
    }
    // A reply is ordered after earlier bindings on the observer socket. This last round trip
    // also exposes input arriving after a document's fence while another target was read.
    await this.#cdp.send('Browser.getVersion')
    try {
      const { tabs } = await this.#cdp.send('Browser.getTabs')
      this.#raw({ kind: 'activation', tabs })
      this.#activation = projectActivation(tabs, [...this.#pages.values()])
    } catch (error) {
      this.#activation = { status: 'incomplete', tabs: [] }
      this.#gap('active-tab-evidence-missing', undefined, error)
    }
    for (const [page, document, sequence] of fences)
      if (sequence !== document.sequence)
        this.#gap('input-after-document-fence', page)
    for (const page of this.#pages.values()) {
      if (!page.session) this.#gap('page-not-attached', page)
      if (
        !page.closed &&
        (page.session?.closed || page.session?.inspectorDetached)
      )
        this.#gap('target-observer-detached', page)
      if (!page.frames.length) this.#gap('page-has-no-observed-document', page)
      for (const frame of page.frames) {
        if (!frame.documents.length)
          this.#gap('frame-has-no-observed-document', page)
        for (const document of frame.documents) {
          const retired =
            document.destroyed ||
            document.replaced ||
            page.closed ||
            frame.detached
          const checkpoint = document.checkpoint
          if (
            retired &&
            !document.terminal &&
            document.epoch === 0 &&
            document.url === 'about:blank' &&
            checkpoint?.empty &&
            checkpoint.sequence === document.sequence
          ) {
            document.terminal = true
            document.dom = checkpoint.dom
          }
          // Context-cleared and binding events can cross on navigation. Decide loss only
          // after the read fence, when a late terminal binding has had a chance to arrive.
          if (retired && !document.terminal)
            this.#gap('document-lost-before-terminal', page)
          if (!document.dom) this.#gap('document-dom-missing', page)
          if ((page.closed || frame.detached) && !document.terminal)
            this.#gap('closed-document-without-terminal', page)
        }
      }
    }
  }

  #projectDom(snapshot, page) {
    const { strings, documents } = snapshot
    if (!Array.isArray(strings) || !Array.isArray(documents))
      throw new Error('Invalid DOM snapshot')
    return documents.map((document) => {
      const frame = this.#frames.get(strings[document.frameId])
      if (!frame) this.#gap('snapshot-frame-not-observed', page)
      const nodes = document.nodes
      const count = nodes.nodeType.length
      if (count > 50_000) {
        this.#gap('dom-node-budget', page)
        return { frame: frame?.ordinal ?? null, incomplete: 'dom-node-budget' }
      }
      const textValues = new Map(
        (nodes.textValue?.index ?? []).map((index, entry) => [
          index,
          strings[nodes.textValue.value[entry]],
        ]),
      )
      const inputValues = new Map(
        (nodes.inputValue?.index ?? []).map((index, entry) => [
          index,
          strings[nodes.inputValue.value[entry]],
        ]),
      )
      const shadowTypes = new Map(
        (nodes.shadowRootType?.index ?? []).map((index, entry) => [
          index,
          strings[nodes.shadowRootType.value[entry]],
        ]),
      )
      for (const shadowRootType of shadowTypes.values())
        this.#shadowCoverage({ shadowRootType }, page)
      const checked = new Set(nodes.inputChecked?.index ?? [])
      const selected = new Set(nodes.optionSelected?.index ?? [])
      const contentDocuments = new Map(
        (nodes.contentDocumentIndex?.index ?? []).map((index, entry) => [
          index,
          nodes.contentDocumentIndex.value[entry],
        ]),
      )
      const projected = Array.from({ length: count }, (_, index) => {
        const attrs = nodes.attributes[index] ?? []
        const node = {
          type: nodes.nodeType[index],
          name: strings[nodes.nodeName[index]],
          text: strings[nodes.nodeValue[index]],
          attributes: Array.from({ length: attrs.length / 2 }, (_, attr) => [
            strings[attrs[attr * 2]],
            strings[attrs[attr * 2 + 1]],
          ]),
          children: [],
        }
        if (textValues.has(index)) node.textValue = textValues.get(index)
        if (inputValues.has(index)) node.value = inputValues.get(index)
        if (node.name === 'INPUT') node.checked = checked.has(index)
        if (node.name === 'OPTION') node.selected = selected.has(index)
        if (shadowTypes.has(index)) node.shadow = shadowTypes.get(index)
        if (contentDocuments.has(index)) {
          const content = documents[contentDocuments.get(index)]
          node.contentFrame =
            this.#frames.get(strings[content?.frameId])?.ordinal ?? null
        }
        return node
      })
      const roots = []
      for (let index = 0; index < count; index++) {
        const parent = nodes.parentIndex[index]
        if (parent >= 0) projected[parent].children.push(projected[index])
        else roots.push(projected[index])
      }
      return {
        frame: frame?.ordinal ?? null,
        url: strings[document.documentURL],
        title: strings[document.title],
        tree: roots,
      }
    })
  }

  #shadowCoverage(node, page) {
    const pending = [node]
    while (pending.length) {
      const current = pending.pop()
      if (!current) continue
      if (
        current.shadowRootType === 'closed' ||
        current.shadowRootType === 'open'
      )
        this.#gap(
          `${current.shadowRootType}-shadow-root-input-not-observed`,
          page,
        )
      pending.push(...(current.children ?? []), ...(current.shadowRoots ?? []))
      if (current.contentDocument) pending.push(current.contentDocument)
    }
  }

  async #watchDom(session) {
    try {
      const { root } = await this.#cdp.send(
        'DOM.getDocument',
        { depth: -1, pierce: true },
        session.id,
      )
      this.#shadowCoverage(root, session.page)
      session.domCoverage = true
    } catch (error) {
      if (!session.closed) throw error
      if (!session.domCoverage)
        this.#gap('closed-document-shadow-coverage-missing', session.page)
    }
  }

  #view() {
    const pages = [...this.#pages.values()].map((page) => ({
      ordinal: page.ordinal,
      opener: page.opener,
      url: page.url,
      closed: page.closed,
      dom: [...this.#sessions.values()]
        .filter(
          (session) => session.page === page && session.dom && !session.closed,
        )
        .flatMap((session) => session.dom),
      documents: page.frames.flatMap((frame) =>
        frame.documents.map((document) => ({
          frame: frame.ordinal,
          parentFrame: frame.parent,
          epoch: document.epoch,
          url: document.url,
          retired: document.terminal,
          dom: document.dom,
        })),
      ),
    }))
    // JSON cloning detaches the returned evidence from later protocol events and navigation.
    return JSON.parse(
      JSON.stringify({
        status: this.#incomplete.length ? 'incomplete' : 'complete',
        incomplete: this.#incomplete,
        activation: this.#activation,
        pages,
        events: this.#events,
      }),
    )
  }

  close() {
    this.#closePromise ??= this.#close()
    return this.#closePromise
  }

  async #close() {
    this.#closing = true
    const failures = []
    const cleanup = { forced: false, survivors: [], exit: null }
    const attempt = async (operation) => {
      try {
        return await operation()
      } catch (error) {
        failures.push(error)
      }
    }
    if (this.#child?.pid) {
      const pid = this.#child.pid
      let closing
      if (!this.#exit && this.endpoint) {
        closing = (async () => {
          const cdp = this.#cdp ?? (await Cdp.connect(this.endpoint))
          try {
            await cdp.send('Browser.close')
          } finally {
            if (cdp !== this.#cdp) await cdp.close()
          }
        })().catch(() => {})
      }
      if (!this.#exit) {
        try {
          await bounded(
            this.#exited,
            this.endpoint ? CLOSE_TIMEOUT_MS : 0,
            'Browser graceful close timed out',
          )
        } catch {
          cleanup.forced = true
          await attempt(() =>
            killOwned(pid, this.#profile, this.#groupVerified),
          )
        }
      }
      await attempt(() =>
        bounded(
          this.#exited,
          KILL_TIMEOUT_MS,
          'Browser did not exit after SIGKILL',
        ),
      )
      cleanup.exit = this.#exit ?? null
      await attempt(async () => {
        const deadline = Date.now() + HELPER_GRACE_MS
        let alive = ownedProcesses(pid, this.#profile)
        while (alive.length && Date.now() < deadline) {
          await sleep(25)
          alive = ownedProcesses(pid, this.#profile)
        }
        cleanup.survivors = alive.map(({ pid: survivor, state }) => ({
          pid: survivor,
          state,
        }))
        if (alive.length) {
          cleanup.forced = true
          killOwned(pid, this.#profile, this.#groupVerified)
          const killedBy = Date.now() + KILL_TIMEOUT_MS
          do {
            alive = ownedProcesses(pid, this.#profile)
            if (!alive.length) break
            await sleep(25)
          } while (Date.now() < killedBy)
        }
        if (alive.length)
          throw new Error('Owned browser processes survived SIGKILL')
      })
      await attempt(() => this.#cdp?.close())
      await closing
      // A failed kill must still allow the caller to receive the aggregate cleanup failure.
      if (!this.#exit) this.#child.unref()
    }
    if (this.#profile)
      await attempt(() => rm(this.#profile, { recursive: true, force: true }))
    if (this.#rawFd !== undefined) {
      await attempt(() => closeSync(this.#rawFd))
      this.#rawFd = undefined
    }
    cleanup.status = failures.length ? 'incomplete' : 'complete'
    if (this.#outputReady)
      await attempt(() =>
        writeFile(
          join(this.#outputDir, 'cleanup.json'),
          `${JSON.stringify(cleanup, null, 2)}\n`,
          { mode: 0o600 },
        ),
      )
    if (failures.length)
      throw new AggregateError(failures, 'Owned browser cleanup failed')
    return cleanup
  }
}
