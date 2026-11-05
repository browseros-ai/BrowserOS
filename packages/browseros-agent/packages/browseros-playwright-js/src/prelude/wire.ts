/**
 * The prelude's half of browseros-playwright's op contract: every handle it sends, every op's
 * input after the shared `call` header, and every result it reads back.
 *
 * Rust decodes each input once with `deny_unknown_fields`: the header and the targets in
 * crates/browseros-playwright/src/target.rs, the rest in each family's `*_op.rs`. A field added
 * or renamed here must change there in the same commit, or the op fails `script /
 * invalid_input` before it sends anything. Types the probe consumes unchanged (expect params,
 * select options, URL patterns) are imported from the probe protocol, so the three halves
 * cannot drift apart. `OpInputs` and `OpOutputs` must name every op in ops.generated.ts; the
 * `complete` check at the end fails to compile otherwise.
 */
import type {
  ExpectParams,
  Point,
  Rect,
  SelectOption,
  UrlPattern,
} from '../probe/protocol'
import type { OpName } from './ops.generated'
import type { AriaSnapshotJSON, ParsedSelector } from './vendored'

/**
 * `{"$neo":"page","id":7,"tab":1234}`, the skeleton's page brand: core's process-local page id
 * and the browser's tab id. Rust refuses a handle whose id now names another tab, as after a
 * Neo restart.
 */
export type PageBrand = { $neo: 'page'; id: number; tab: number }

/**
 * A locator, re-resolved by Rust on every attempt. `chunks` is the vendored
 * `splitSelectorByFrame` output, one parsed selector per frame, so Rust never parses selector
 * syntax and nested selectors keep the frame-prefix handling only a whole-selector parse does.
 * `display` is `asLocator('javascript', selector)`, which Rust prints in messages and call logs.
 */
export type LocatorBrand = {
  $neo: 'locator'
  page: number
  /** The frame the locator was built from (`frame.locator()`); null is the main frame. */
  frame: string | null
  chunks: ParsedSelector[]
  display: string
  /**
   * The handle a query is rooted at (`elementHandle.$()`, `elementHandle.$$()`): the first
   * chunk runs from that node in its own document, and `frame` plays no part. Left out
   * otherwise.
   */
  root?: ElementBrand
}

/** One element pinned in one document (`ElementHandle`); detached for good once it goes. */
export type ElementBrand = {
  $neo: 'element'
  page: number
  seq: number
  node: number
  display: string
}

export type Target = LocatorBrand | ElementBrand

/** Locator methods are strict; page and frame shortcuts take the first match. */
export type Strictness = 'strict' | 'firstMatch'

/**
 * Opens every op input. `api` is the method the script called (`page.click` for the
 * `locator.click` op), which every message from the op starts with. `timeoutMs` is the
 * Playwright timeout the prelude resolved; 0 means none, and the run deadline bounds the op.
 */
export type CallHeader = { api: string; timeoutMs: number }

/**
 * What the host knows about a tab: the seed, every envelope's `pages`, events. `tab` is the
 * browser's tab id, which the page's brand carries.
 */
export type PageFact = {
  id: number
  tab?: number
  url?: string
  title?: string
  closed?: boolean
}

/** One frame of a page's tree, main frame first; `parentId` null only for the main frame. */
export type FrameInfo = {
  id: string
  parentId: string | null
  name: string
  url: string
}

/**
 * The response a navigation committed, read from the new document's navigation timing entry
 * without the Network domain. Rust owns all of it: the reason phrase comes from its table, and
 * `headers` is always empty because reading them needs the Network domain.
 */
export type ResponseInfo = {
  url: string
  status: number
  statusText: string
  ok: boolean
  headers: Record<string, string>
}

/**
 * What a navigation or a wait returns. `url` and `frames` refresh `page.url()` and
 * `page.frames()`, which are synchronous in Playwright; `url` is null when no document
 * committed.
 */
export type NavigationResult = {
  response?: ResponseInfo | null
  url?: string | null
  frames?: FrameInfo[]
}

/** `page.frames`: every frame of the tab, the main frame first, each parent before its children. */
export type FrameTree = { frames: FrameInfo[] }

/** One tab, with the URL the prelude caches for its synchronous `url()`. */
export type PageEntry = { page: PageBrand; url: string }

/** What an op that returns nothing answers. */
export type Done = Record<string, never>

/** A frame Rust could not read within its budget; it keeps its iframe line and gets a comment. */
export type PartialFrame = { ref: string; frame: string; reason: string }
/** A stitched, masked aria snapshot. The prelude renders it as YAML. */
export type SnapshotResult = { json: AriaSnapshotJSON; partial: PartialFrame[] }

/** The Playwright events Neo reports. `page` is the context's; the rest are a page's. */
export type EventName =
  | 'popup'
  | 'close'
  | 'dialog'
  | 'console'
  | 'pageerror'
  | 'download'
  | 'page'

export type DialogInfo = {
  id: number
  type: 'alert' | 'beforeunload' | 'confirm' | 'prompt'
  message: string
  defaultValue: string
}
export type ConsoleInfo = {
  type: string
  text: string
  location: { url: string; lineNumber: number; columnNumber: number }
  timestamp: number
}
export type PageErrorInfo = { name: string; message: string; stack: string }
/** `id` is the browser's download guid, which `download.path` and `download.saveAs` name. */
export type DownloadInfo = {
  id: string
  url: string
  suggestedFilename: string
}

/**
 * One event as Rust delivers it to one listener or wait. `event` is Playwright's name and
 * `page` the page it fires on; the context's `page` event names the new tab there. `seq` is the
 * record's place in Rust's log, which a `waitForEvent` whose predicate said no re-waits after.
 *
 * Rust reads its log once per listener, from that listener's own position, so one opened tab
 * reaches the opener's `popup` listener and the context's `page` listener as two events, and a
 * listener attached late still gets events newer than its own stamp. A `page` may arrive
 * revived as a Page by the bootstrap, or as a raw brand.
 */
export type EventRecord = { seq: number } & (
  | { event: 'popup'; page: PageBrand; popup: PageEntry }
  | { event: 'page'; page: PageBrand; url: string; opener: PageBrand | null }
  | { event: 'close'; page: PageBrand }
  | { event: 'dialog'; page: PageBrand; dialog: DialogInfo }
  | { event: 'console'; page: PageBrand; message: ConsoleInfo }
  | { event: 'pageerror'; page: PageBrand; error: PageErrorInfo }
  | { event: 'download'; page: PageBrand; download: DownloadInfo }
)

/**
 * What the pump gets: the events of every listener that had some, in log order, and how many
 * records fell off Rust's ring before this run read them.
 */
export type Pumped = { events: EventRecord[]; lost: number }

export type MouseButton = 'left' | 'right' | 'middle'
export type Modifier = 'Alt' | 'Control' | 'ControlOrMeta' | 'Meta' | 'Shift'

/** Playwright's pointer options, minus the timeout (in the header) and strict (beside them). */
export type PointerOptions = {
  button?: MouseButton
  clickCount?: number
  delay?: number
  force?: boolean
  modifiers?: Modifier[]
  noWaitAfter?: boolean
  position?: Point
  steps?: number
  trial?: boolean
}
export type DragOptions = {
  force?: boolean
  noWaitAfter?: boolean
  sourcePosition?: Point
  targetPosition?: Point
  steps?: number
  trial?: boolean
}
export type FillOptions = { force?: boolean; noWaitAfter?: boolean }
export type KeyOptions = { delay?: number; noWaitAfter?: boolean }
export type LoadState = 'load' | 'domcontentloaded' | 'networkidle' | 'commit'
export type WaitForState = 'attached' | 'detached' | 'visible' | 'hidden'
export type ElementStateName =
  | 'visible'
  | 'hidden'
  | 'stable'
  | 'enabled'
  | 'disabled'
  | 'editable'

/**
 * A main-world function and its JSON argument (v1 passes no handles). An absent `arg` reaches
 * the function as `undefined`, which JSON cannot carry.
 */
export type Evaluation = {
  expression: string
  isFunction: boolean
  arg?: unknown
}

/** The screenshot options Neo captures with; the prelude refuses the others before sending. */
export type ScreenshotOptions = {
  type?: 'png' | 'jpeg'
  quality?: number
  fullPage?: boolean
  clip?: Rect
}

/** `page.pdf`'s options with upstream's names, sizes and margins already in CSS units. */
export type PdfOptions = Record<string, unknown>

export type TargetArgs = { target: Target; strict: Strictness }
export type EveryMatchArgs = { target: Target }
export type PageArgs = { page: PageBrand }
/** A page op on one of the page's frames; `frame` is left out for the main frame. */
export type FrameArgs = PageArgs & { frame?: string }
export type ExpectArgs = {
  /** Playwright's matcher name, `toHaveText`. */
  matcher: string
  /** What the failure header prints in the matcher's parentheses: `expected`, `{ visible: false }`. */
  expectation: string
  /** Playwright's printed expectation line, `Expected: "Hello"` or `Expected pattern: /x/`. */
  expected: string
  params: ExpectParams
}

/** Each op's input after `call`. */
export type OpInputs = {
  'locator.click': TargetArgs & { options: PointerOptions }
  'locator.dblclick': TargetArgs & { options: PointerOptions }
  'locator.hover': TargetArgs & { options: PointerOptions }
  'locator.check': TargetArgs & { options: PointerOptions }
  'locator.uncheck': TargetArgs & { options: PointerOptions }
  /** `to` is resolved in the source's frame, as upstream's `frame.dragAndDrop`. */
  'locator.dragTo': TargetArgs & { to: Target; options: DragOptions }
  'locator.fill': TargetArgs & { value: string; options: FillOptions }
  'locator.press': TargetArgs & { key: string; options: KeyOptions }
  'locator.pressSequentially': TargetArgs & {
    text: string
    options: KeyOptions
  }
  'locator.selectOption': TargetArgs & {
    options: SelectOption[]
    elements: ElementBrand[]
    force?: boolean
  }
  'locator.selectText': TargetArgs & { force?: boolean }
  'locator.focus': TargetArgs
  'locator.blur': TargetArgs
  /** Host paths; Playwright's in-memory file payloads are not supported. */
  'locator.setInputFiles': TargetArgs & { files: string[] }
  'locator.dispatchEvent': TargetArgs & { type: string; eventInit: unknown }
  'locator.scrollIntoViewIfNeeded': TargetArgs
  /**
   * The reads below wait for a locator's element, as upstream's do; an element handle's
   * target needs no waiting, so no flag says it.
   */
  'locator.textContent': TargetArgs
  'locator.innerText': TargetArgs
  'locator.innerHTML': TargetArgs
  'locator.getAttribute': TargetArgs & { name: string }
  'locator.inputValue': TargetArgs
  'locator.isEnabled': TargetArgs
  'locator.isDisabled': TargetArgs
  'locator.isEditable': TargetArgs
  'locator.isChecked': TargetArgs
  'locator.isVisible': TargetArgs
  'locator.isHidden': TargetArgs
  'locator.count': EveryMatchArgs
  'locator.allTextContents': EveryMatchArgs
  'locator.allInnerTexts': EveryMatchArgs
  'locator.boundingBox': TargetArgs
  /**
   * `locator.waitFor`, `page.waitForSelector` and `elementHandle.waitForElementState` (whose
   * states are the element-state names). `omitReturnValue` skips pinning a handle.
   */
  'locator.waitFor': TargetArgs & {
    state: WaitForState | ElementStateName
    omitReturnValue: boolean
  }
  /** `noWait` is `page.$`: the first match now, or null, instead of waiting for one. */
  'locator.elementHandle': TargetArgs & { noWait?: true }
  'locator.elementHandles': EveryMatchArgs
  'page.ariaSnapshot': PageArgs & { mode: 'ai' | 'default'; depth?: number }
  'locator.ariaSnapshot': TargetArgs & {
    mode: 'ai' | 'default'
    depth?: number
  }
  'expect.locator': { target: Target } & ExpectArgs
  'expect.page': PageArgs & ExpectArgs
  'keyboard.down': PageArgs & { key: string }
  'keyboard.up': PageArgs & { key: string }
  'keyboard.press': PageArgs & { key: string; options: { delay?: number } }
  'keyboard.type': PageArgs & { text: string; options: { delay?: number } }
  'keyboard.insertText': PageArgs & { text: string }
  'mouse.move': PageArgs & { x: number; y: number; options: { steps?: number } }
  'mouse.down': PageArgs & {
    options: { button?: MouseButton; clickCount?: number }
  }
  'mouse.up': PageArgs & {
    options: { button?: MouseButton; clickCount?: number }
  }
  'mouse.click': PageArgs & {
    x: number
    y: number
    options: { button?: MouseButton; clickCount?: number; delay?: number }
  }
  'mouse.wheel': PageArgs & { deltaX: number; deltaY: number }
  'page.goto': FrameArgs & {
    url: string
    waitUntil: LoadState
    referer?: string
  }
  'page.reload': PageArgs & { waitUntil: LoadState }
  'page.goBack': PageArgs & { waitUntil: LoadState }
  'page.goForward': PageArgs & { waitUntil: LoadState }
  /** Matched in the page with vendored `urlMatches`, so globs and RegExps keep JS semantics. */
  'page.waitForURL': FrameArgs & { url: UrlPattern; waitUntil: LoadState }
  'page.waitForLoadState': FrameArgs & { state: LoadState }
  'page.title': FrameArgs
  'page.content': FrameArgs
  'page.setContent': FrameArgs & { html: string; waitUntil: LoadState }
  'page.close': PageArgs & { runBeforeUnload?: true }
  'page.opener': PageArgs
  'page.frames': PageArgs
  'page.evaluate': FrameArgs & Evaluation
  /** `noWait` is `$eval`: no waiting for the element, as `locator.evaluate` does. */
  'locator.evaluate': TargetArgs & Evaluation & { noWait?: true }
  'locator.evaluateAll': EveryMatchArgs & Evaluation
  'page.waitForFunction': FrameArgs & Evaluation & { polling?: number }
  /** With `target`, a screenshot of one element (`locator.screenshot`). */
  'page.screenshot': PageArgs & {
    target?: Target
    strict?: Strictness
  } & ScreenshotOptions
  'page.pdf': PageArgs & PdfOptions
  'context.pages': Record<string, never>
  'context.newPage': Record<string, never>
  /** `since` is the script's `Date.now()` when it attached the listener (or removed it). */
  'events.listen': {
    page: PageBrand | null
    event: EventName
    on: boolean
    since: number
  }
  /**
   * The background pump. It names no cursor: Rust keeps one read position per listener in the
   * run's state, so a listener attached after the pump read past an event still gets it.
   */
  'events.next': Record<string, never>
  /**
   * The first matching record at or after `since`, the `Date.now()` taken when the script
   * called `waitForEvent`; with `after`, the first one after that seq (a predicate said no).
   * Rust counts `call.timeoutMs` from the op's own start, so the prelude sends what is left of
   * the `waitForEvent`'s one deadline and names the script's timeout in the message (`Timeout
   * 5000ms exceeded while waiting for event "popup"`). While it waits for `dialog`, the run
   * counts as listening, so Rust holds the dialog for it.
   */
  'events.wait': {
    page: PageBrand | null
    event: EventName
    since: number
    after: number | null
  }
  'dialog.accept': PageArgs & { dialog: number; promptText?: string }
  'dialog.dismiss': PageArgs & { dialog: number }
  'download.path': { download: string }
  'download.saveAs': { download: string; path: string }
}

/**
 * Each op's result. Page and element handles may arrive revived by the bootstrap
 * (`__neo.handles.register`) or as raw brands; the classes read both.
 */
export type OpOutputs = {
  'locator.click': null
  'locator.dblclick': null
  'locator.hover': null
  'locator.check': null
  'locator.uncheck': null
  'locator.dragTo': null
  'locator.fill': null
  'locator.press': null
  'locator.pressSequentially': null
  'locator.selectOption': string[]
  'locator.selectText': null
  'locator.focus': null
  'locator.blur': null
  'locator.setInputFiles': null
  'locator.dispatchEvent': null
  'locator.scrollIntoViewIfNeeded': null
  'locator.textContent': string | null
  'locator.innerText': string
  'locator.innerHTML': string
  'locator.getAttribute': string | null
  'locator.inputValue': string
  'locator.isEnabled': boolean
  'locator.isDisabled': boolean
  'locator.isEditable': boolean
  'locator.isChecked': boolean
  'locator.isVisible': boolean
  'locator.isHidden': boolean
  'locator.count': number
  'locator.allTextContents': string[]
  'locator.allInnerTexts': string[]
  'locator.boundingBox': Rect | null
  'locator.waitFor': unknown
  'locator.elementHandle': unknown
  'locator.elementHandles': unknown[]
  'page.ariaSnapshot': SnapshotResult
  'locator.ariaSnapshot': SnapshotResult
  'expect.locator': null
  'expect.page': null
  'keyboard.down': null
  'keyboard.up': null
  'keyboard.press': null
  'keyboard.type': null
  'keyboard.insertText': null
  'mouse.move': null
  'mouse.down': null
  'mouse.up': null
  'mouse.click': null
  'mouse.wheel': null
  'page.goto': NavigationResult | null
  'page.reload': NavigationResult | null
  'page.goBack': NavigationResult | null
  'page.goForward': NavigationResult | null
  'page.waitForURL': NavigationResult | null
  'page.waitForLoadState': NavigationResult | null
  'page.title': string
  'page.content': string
  'page.setContent': NavigationResult | null
  'page.close': Done
  'page.opener': { opener: PageBrand | null }
  'page.frames': FrameTree
  'page.evaluate': unknown
  'locator.evaluate': unknown
  'locator.evaluateAll': unknown
  'page.waitForFunction': unknown
  /** The bootstrap's revived `{"$neo":"bytes"}` value, handed to the script as it is. */
  'page.screenshot': unknown
  'page.pdf': unknown
  'context.pages': { pages: PageEntry[] }
  'context.newPage': PageEntry
  'events.listen': Done
  'events.next': Pumped
  'events.wait': EventRecord
  'dialog.accept': Done
  'dialog.dismiss': Done
  'download.path': { path: string | null; failure: string | null }
  'download.saveAs': null
}

type Exact<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false
type Assert<T extends true> = T
/** Fails to compile when an op in ops.generated.ts lacks an input or a result type here. */
type _WireCoversEveryOp = [
  Assert<Exact<keyof OpInputs, OpName>>,
  Assert<Exact<keyof OpOutputs, OpName>>,
]
