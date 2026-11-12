/**
 * The prelude's half of neo-playwright's op contract: every handle it sends, every op's
 * input after the shared `call` header, and every result it reads back.
 *
 * Rust decodes each input once with `deny_unknown_fields`: the header and the targets in
 * crates/neo-sdk/neo-playwright/src/target.rs, the rest in each family's `*_op.rs`. A field added
 * or renamed here must change there in the same commit, or the op fails `script /
 * invalid_input` before it sends anything. Types the probe consumes unchanged (expect params,
 * select options, URL patterns) are imported from the probe protocol, so the three halves
 * cannot drift apart.
 *
 * The op maps are being moved into Rust one family at a time: a family on route rows has its
 * inputs and results generated from its handlers' types (`./generated`, written by
 * crates/neo-sdk/neo-playwright/tests/wire_generated.rs), and its hand rows leave
 * `HandInputs` and `HandOutputs` here in the same change. `_Partition` at the end fails to
 * compile while an op is described by both, or by neither.
 */

import type { SerializedValue } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'
import type { GeneratedInputs, GeneratedOutputs } from './generated'
// The run envelope's types (Observation, PageFact, WorkerInfo) are neo-server's, in bridge.ts.
// The event records below name the same worker identity.
import type { WorkerInfo } from './generated/envelope'
import type { OpName } from './generated/ops'

// Rust owns these shared shapes now. LocatorBrand is a projection of the tagged Target
// union, so its fields are never repeated in TypeScript.
export type {
  ElementBrand,
  FilesInput,
  FillOptions,
  FrameInfo,
  KeyOptions,
  LoadState,
  MouseButton,
  NavigationCut,
  NavigationPublication,
  NavigationResult,
  PageBrand,
  PageHandle,
  PartialFrame,
  ResponseInfo,
  ScrollOption,
  SnapshotResult,
  Strictness,
  Target,
} from './generated'
/** Playwright's in-page serialized value (`utilityScriptSerializers.ts`), as JSON. */
export type { SerializedValue }

import type {
  ElementBrand,
  FrameInfo,
  PageBrand,
  PageHandle,
  Strictness,
  Target,
} from './generated'

/** Script options are the flat Rust input with its host addressing removed. */
export type PointerOptions = Omit<
  GeneratedInputs['locator.click'],
  'target' | 'strict'
>
export type DragOptions = Omit<
  GeneratedInputs['locator.dragTo'],
  'target' | 'strict' | 'to'
>

export type LocatorBrand = Extract<Target, { $neo: 'locator' }>
export type UnopenedPageBrand = PageHandle & { tab: null }

/**
 * Opens every op input. `api` is the method the script called (`page.click` for the
 * `locator.click` op), which every message from the op starts with. `timeoutMs` is the
 * Playwright timeout the prelude resolved; 0 means none, and the run deadline bounds the op.
 */
export type CallHeader = { api: string; timeoutMs: number }

/** One tab, with the URL the prelude caches for its synchronous `url()`. */
export type PageEntry = { page: PageBrand; url: string }

/**
 * The empty object still returned by host bookkeeping and event acknowledgments. Null results
 * are spelled out per op: changing this alias would misdescribe existing run-record payloads.
 */
export type Done = Record<string, never>

/** The Playwright events Neo reports. `page` is the context's; the rest are a page's. */
export type EventName =
  | 'worker'
  | 'workerclosed'
  | 'workerstate'
  | 'popup'
  | 'close'
  | 'dialog'
  | 'dialogclosed'
  | 'console'
  | 'pageerror'
  | 'download'
  | 'filechooser'
  | 'load'
  | 'domcontentloaded'
  | 'frameattached'
  | 'framenavigated'
  | 'framedetached'
  | 'pageload'
  | 'page'

export type DialogInfo = {
  id: number
  type: 'alert' | 'beforeunload' | 'confirm' | 'prompt'
  message: string
  defaultValue: string
}
export type ConsoleInfo = {
  worker?: WorkerInfo
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

/** The exact native input, rather than a selector that could resolve to a replacement. */
export type FileChooserInfo = { element: ElementBrand; isMultiple: boolean }

/**
 * One event as Rust delivers it to one listener or wait. `event` is Playwright's name and
 * `page` the page it fires on; the context's `page` event names the new tab there. `seq` is the
 * record's place in Rust's log, which a `waitForEvent` whose predicate said no re-waits after.
 * `at` is when Rust recorded it, in ms since the epoch, on the clock that stamps takeover lock
 * times. claw-sdk's door reads it to withhold what a tab did while its owner held it; the prelude
 * ignores it.
 *
 * Rust reads its log once per listener, from that listener's own position, so one opened tab
 * reaches the opener's `popup` listener and the context's `page` listener as two events, and a
 * listener attached late still gets events newer than its own stamp. A `page` may arrive
 * revived as a Page by the bootstrap, or as a raw brand.
 */
export type EventRecord = { seq: number; at: number } & (
  | { event: 'popup'; page: PageBrand; popup: PageEntry }
  | { event: 'page'; page: PageBrand; url: string; opener: PageBrand | null }
  | { event: 'close'; page: PageBrand }
  | {
      event: 'load' | 'domcontentloaded' | 'pageload'
      page: PageBrand
      frame: FrameInfo
    }
  | {
      event: 'frameattached' | 'framenavigated' | 'framedetached'
      page: PageBrand
      frame: FrameInfo
      context: boolean
    }
  | { event: 'dialog'; page: PageBrand; dialog: DialogInfo }
  | {
      event: 'dialogclosed'
      page: PageBrand
      dialog: DialogInfo
      context: boolean
    }
  | { event: 'worker' | 'workerclosed'; page: PageBrand; worker: WorkerInfo }
  | {
      event: 'workerstate'
      page: PageBrand
      worker: WorkerInfo
      closed: boolean
    }
  | { event: 'console'; page: PageBrand; message: ConsoleInfo }
  | { event: 'pageerror'; page: PageBrand; error: PageErrorInfo }
  | { event: 'download'; page: PageBrand; download: DownloadInfo }
  | { event: 'filechooser'; page: PageBrand; chooser: FileChooserInfo }
  | BindingRecord
)

/**
 * A page's call of a binding the run exposed: the stub's JSON `payload` (`{ name, seq,
 * serializedArgs }`), the `executionContext` to answer in (`bindingCall.resolve`), and that
 * context's frame id, `null` for the main frame.
 */
export type BindingRecord = {
  event: 'binding'
  page: PageBrand
  frame: string | null
  executionContext: number
  payload: string
}

/**
 * What the pump gets: the events of every listener that had some, in log order, and how many
 * records fell off Rust's ring before this run read them.
 */
export type Pumped = { events: EventRecord[]; lost: number }

export type Modifier = 'Alt' | 'Control' | 'ControlOrMeta' | 'Meta' | 'Shift'

export type WaitForState = 'attached' | 'detached' | 'visible' | 'hidden'
export type ElementStateName =
  | 'visible'
  | 'hidden'
  | 'stable'
  | 'enabled'
  | 'disabled'
  | 'editable'

/**
 * A main-world script on the wire (evaluation.ts `pageCall`), mirrored by Rust's `ScriptInput`:
 * ready to run, or the message of the server-side check upstream fails first, which Rust raises
 * at its point with nothing sent.
 */
/**
 * An init script's source as upstream's client builds it (`evaluationScript`), or the path of a
 * file on Neo's disk that Rust reads as upstream's client reads it, `//# sourceURL` appended.
 */
export type InitSource = { source: string } | { path: string }

export type PageScript =
  | {
      run: {
        /** `normalizeEvaluationExpression(String(pageFunction), isFunction)`. */
        expression: string
        isFunction: boolean
        /** The argument in Playwright's in-page format, as JSON text. */
        arg: string
        /** The element handles the argument's `{h: i}` names, in order; left out when none. */
        handles?: ElementBrand[]
      }
    }
  | { refused: string }

/** The screenshot options Neo captures with; the prelude refuses the others before sending. */
export type ScreenshotOptions = Omit<
  GeneratedInputs['page.screenshot'],
  'page' | 'target' | 'strict'
>

/**
 * PDF channel fields accepted by pdf.rs. pdf-options.ts keeps the script's numeric lengths
 * separate and normalizes them to px strings before sending, as Playwright's client does.
 */
export type PdfOptions = Omit<GeneratedInputs['page.pdf'], 'page'>

export type TargetArgs = { target: Target; strict: Strictness }
export type EveryMatchArgs = { target: Target }
export type PageArgs = { page: PageHandle }
/** A page op on one of the page's frames; `frame` is left out for the main frame. */
export type FrameArgs = PageArgs & { frame?: string }
/** Each op's input after `call`, for the ops Rust does not describe yet. */
export type HandInputs = {
  'events.interceptFileChooser': PageArgs & { lease: number; on: boolean }

  'worker.evaluate': { page: PageHandle; worker: string; script: PageScript }
  'page.evaluate': FrameArgs & { script: PageScript }
  /** `noWait` is `$eval`: no waiting for the element, as `locator.evaluate` does. */
  'locator.evaluate': TargetArgs & {
    script: PageScript
    /** `$eval`: the first match now instead of waiting for one. */
    noWait?: true
    /** `$eval`'s selector as the script wrote it, which upstream's no-match message quotes. */
    selector?: string
  }
  'locator.evaluateAll': EveryMatchArgs & { script: PageScript }
  /** `polling` in milliseconds; absent is `'raf'`. */
  'page.waitForFunction': FrameArgs & { script: PageScript; polling?: number }
  'locator.waitForFunction': TargetArgs & { script: PageScript }
  'page.addInitScript': PageArgs & { script: InitSource }
  /** `noGlobal`: the controller alone knows the binding (`exposeFunctions` callbacks). */
  'page.exposeBinding': PageArgs & { name: string; noGlobal?: true }
  /** `arg`: `{ name, seq, result }` or `{ name, seq, error }` as in-page JSON text. */
  'bindingCall.resolve': PageArgs & { context: number; arg: string }
  'context.addInitScript': { script: InitSource }
  'context.exposeBinding': { name: string; noGlobal?: true }
  /** The id an `addInitScript` returned. */
  'disposable.dispose': { id: number }
  /**
   * `since` is the script's `Date.now()` when it attached the listener (or removed it). The
   * event ops name no target the door resolves, so their page always has its tab: the
   * prelude opens the run's lazy page first (`Page._openedBrand`).
   */
  'events.listen':
    | { page: PageBrand | null; event: EventName; on: boolean; since: number }
    /** Cleanup addresses only this run's bookkeeping key; the tab may already be gone. */
    | {
        page: null
        retiredTab: number
        event: 'workerstate'
        on: false
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

/** Each op's result, for the ops Rust does not describe yet. */
export type HandOutputs = {
  'events.interceptFileChooser': Done

  'worker.evaluate': { value: SerializedValue }
  'page.evaluate': { value: SerializedValue; frames?: FrameInfo[] }
  'locator.evaluate': SerializedValue
  'locator.evaluateAll': SerializedValue
  /** The predicate's truthy value and upstream's preview of it (`toString()`). */
  'page.waitForFunction': { value: SerializedValue; preview: string }
  'locator.waitForFunction': null
  /** The id `disposable.dispose` takes. */
  'page.addInitScript': { id: number }
  'page.exposeBinding': { id: number }
  'bindingCall.resolve': Done
  'context.addInitScript': { id: number }
  'context.exposeBinding': { id: number }
  'disposable.dispose': Done
  'events.listen': Done
  'events.next': Pumped
  'events.wait': EventRecord
  'dialog.accept': Done
  'dialog.dismiss': Done
  'download.path': { path: string | null; failure: string | null }
  'download.saveAs': null
}

/** Each op's input after `call`: Rust's generated type, or the hand row above. */
export type OpInputs = GeneratedInputs & HandInputs

/**
 * Each op's result. Page and element handles arrive revived by the bootstrap
 * (`__neo.brands.register`); the revivers and the classes read the raw brands.
 */
export type OpOutputs = GeneratedOutputs & HandOutputs

type Exact<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false
type Assert<T extends true> = T
/**
 * Fails to compile when an op is described both by Rust and by hand, or by neither: every op
 * in `OPS` has exactly one input and one result type.
 */
type _Partition = [
  Assert<Exact<keyof GeneratedInputs & keyof HandInputs, never>>,
  Assert<Exact<keyof GeneratedOutputs & keyof HandOutputs, never>>,
  Assert<Exact<keyof OpInputs, OpName>>,
  Assert<Exact<keyof OpOutputs, OpName>>,
]
