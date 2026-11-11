/**
 * The probe protocol: the one contract between neo-playwright's Rust half and Neo's
 * probe in each frame's isolated `browseros-engine` world.
 *
 * Rust sends one ProbeRequest per `Runtime.callFunctionOn` (CALL_FUNCTION, `returnByValue`)
 * on the world's execution context, or on an element's object id, and decodes one reply.
 * The Rust mirror lives in crates/neo-sdk/neo-playwright/src/probe/. Neither side generates
 * the other: src/probe/protocol-examples.ts holds typed examples of every request and reply,
 * scripts/build.ts writes them to crates/neo-sdk/neo-playwright/tests/data/probe-protocol.json,
 * and each side decodes that file in its tests, so a shape change on one side fails the
 * other's build.
 */

import type { FrameExpectParams } from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'
import type { AriaSnapshotJSON } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'
import type { ParsedSelector } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'

/**
 * Bumped on any shape change, and on any change to what Rust relies on the probe doing. `boot`
 * reports it, and an install whose number differs from Rust's PROBE_PROTOCOL fails as
 * `neo / internal` (the engine asset is stale).
 */
export const PROTOCOL = 21

/**
 * The function declaration Rust sends with every request. Rust calls it on the world's global
 * object, with the request and an element handle's node as the call's root (or null), so the
 * probe treats a root that is not a Node as "no root". A world whose probe is gone (or never
 * booted) answers `uninstalled`, which Rust treats like a stale context: install again, once.
 */
export const CALL_FUNCTION =
  'function (request, root) { const probe = globalThis.__neoProbe; return probe ? probe.call(request, root ?? this) : { kind: "uninstalled" }; }'

/** An element this world resolved earlier in the same op; see `Pins` in world.ts. */
export type Pin = number
export type Point = { x: number; y: number }
export type Rect = { x: number; y: number; width: number; height: number }

/**
 * What a request runs on. A chunk is one frame's part of a locator, already parsed by the
 * prelude with the vendored selectorParser: re-parsing per-frame strings would lose
 * `parseSelector`'s nested frame-prefix handling, so selectors cross the wire parsed.
 * `root` is the call's root element itself: an ElementHandle's node, which Rust passes as
 * the call's second argument. It reads as detached once it has left its document.
 */
export type ProbeTarget =
  | { chunk: ParsedSelector }
  | { pin: Pin }
  | 'document'
  | 'root'
export type Strictness = 'strict' | 'firstMatch'
/** States the probe checks. `stable` is not one: Rust compares two measurements in time. */
export type ElementState = 'visible' | 'enabled' | 'editable'
/** Upstream's retarget behaviors (InjectedScript.retarget). */
export type Retarget =
  | 'none'
  | 'follow-label'
  | 'no-follow-label'
  | 'button-link'
/** `alreadyProtocol`: Rust scrolled with `DOM.scrollIntoViewIfNeeded`, the probe only measures. */
export type ScrollPlan =
  | 'none'
  | 'ifNeeded'
  | 'end'
  | 'center'
  | 'start'
  | 'alreadyProtocol'
export type HitAction = 'mouse' | 'hover' | 'drag'
/**
 * Arms the hit-target interceptor. `blockAll` is Playwright's `trial`. With `expectRect`
 * the probe arms only while the element still has that rect (the stability baseline).
 * `expiryMs` bounds an interceptor whose verdict never comes; Rust sets it past the last event
 * its op can still send.
 */
export type ArmPlan = {
  action: HitAction
  blockAll: boolean
  token: number
  expiryMs: number
  expectRect?: Rect
  /** Root geometry owns viewport/area checks; actual events alone establish the hit. */
  browserMapped?: boolean
}
/**
 * One option to select, as upstream's client sends it: an option matches when every attribute
 * named here does (injected `selectOptions`), and one that names none matches any option.
 */
export type SelectOption = {
  valueOrLabel?: string
  value?: string
  label?: string
  index?: number
}
/**
 * An edit of a pinned element; each answers `value` or `detached`. A `fill` answers
 * `Filled`, a `selectOptions` answers `Selected`, a `setFiles` answers `FilesSet`, the others
 * answer null.
 */
export type Edit =
  | { kind: 'fill'; value: string }
  | { kind: 'selectOptions'; options: SelectOption[] }
  | { kind: 'selectText' }
  | { kind: 'focus'; resetSelection: boolean }
  | { kind: 'blur' }
  | { kind: 'dispatchEvent'; type: string; init: Record<string, unknown> }
  | { kind: 'setFiles'; payloads: FilePayload[] }
  | {
      kind: 'drop'
      payloads: FilePayload[]
      data: { mimeType: string; value: string }[]
      point: Point
    }
/**
 * One in-memory file as upstream's injected `setInputFiles` takes it. `buffer` is the base64
 * text upstream's server makes of the script's bytes (`buffer.toString('base64')`), which the
 * page decodes with `atob`; `mimeType` is already defaulted from the name.
 */
export type FilePayload = {
  name: string
  mimeType: string
  buffer: string
  lastModifiedMs?: number
}
/**
 * A `setFiles` value: null once upstream's injected `setInputFiles` returned (it set the files,
 * or left a non-file input alone, as upstream ignores why), or what it threw as CDP describes an
 * exception, which upstream's error carries (`DOMException` for a `buffer` that is not base64).
 */
export type FilesSet = null | { threw: string }
/**
 * A fill's `value`: `done` when the input took the value itself (date, color, range and the
 * other input types without typing), `needsinput` when it is focused and selected for Rust's
 * Input.insertText.
 */
export type Filled = 'done' | 'needsinput'
/** A selectOptions' `value`: the selected values, or why upstream retries. */
export type Selected =
  | string[]
  | 'error:optionsnotfound'
  | 'error:optionnotenabled'
export type QueryState =
  | 'visible'
  | 'hidden'
  | 'enabled'
  | 'disabled'
  | 'editable'
  | 'checked'
  | 'unchecked'
/**
 * What `read` reads. `boundingBox` is the border box in this frame's viewport (null with no
 * layout box); `frameOrigin` is an `<iframe>`'s content-box origin there and its scale
 * (`FrameOrigin`), which Rust applies to a child frame's boxes to place them in the main
 * frame's viewport; `located` is what upstream's `waitForSelector` checks on each poll, with
 * the element pinned for a handle; `checkable` is what `check` and `uncheck` read around
 * their click (`Checkable`); `fileInput` is the file input `setInputFiles` sets (`FileInput`).
 */
export type Read =
  | { kind: 'textContent' }
  | { kind: 'innerText' }
  | { kind: 'innerHtml' }
  | { kind: 'inputValue' }
  | { kind: 'boundingBox' }
  | { kind: 'attribute'; name: string }
  | { kind: 'state'; state: QueryState }
  | { kind: 'frameOrigin' }
  | { kind: 'located' }
  | { kind: 'checkable' }
  | { kind: 'fileInput'; multiple: boolean; directory: boolean }
/**
 * A `frameOrigin` read's value: where the iframe's document starts in this frame's viewport,
 * and how much the iframe's own and its ancestors' transforms scale it there (rendered size
 * over layout size). `rotated` when a rotate, skew, flip or 3D transform leaves no
 * axis-aligned box to place a child's box in, which Rust refuses as `neo / unsupported`.
 */
export type FrameOrigin =
  | { x: number; y: number; scaleX: number; scaleY: number }
  | 'rotated'
/**
 * A `checkable` read's value: upstream's `elementState(el, 'checked')` as `_setChecked` reads
 * it, whether the element (or the control its label names) is checked, and whether that
 * control is a radio, which `uncheck` refuses before it clicks.
 */
export type Checkable = { checked: boolean; radio: boolean }
/**
 * A `fileInput` read's value, after upstream's `_setInputFiles` checks for `multiple` files or a
 * `directory`: the `<input>` the element is or its label names, pinned for the files (`pin`);
 * the element itself (`target`, `pin` again when it is the input), on which upstream's directory
 * upload waits for `input`; and whether Chromium's `DOM.setFileInputFiles` takes paths for the
 * input (`type=file`), which Rust checks before it sends any.
 */
export type FileInput = { pin: Pin; target: Pin; takesPaths: boolean }
/** A `located` read's value. `count` is how many matched, for upstream's call-log line. */
export type Located = {
  pin: Pin
  preview: string
  visible: boolean
  count: number
}
export type ReadAll = 'count' | 'textContents' | 'innerTexts' | 'pins'
/** One value of `readAll` `pins`: an element to make an ElementHandle of. */
export type PinnedMatch = { pin: Pin; preview: string }
export type SnapshotMode = 'ai' | 'default'
/** Upstream's per-poll expect parameters, minus the selector (the target carries it). */
export type ExpectParams = Omit<FrameExpectParams, 'selector'>
/** A `waitForURL` pattern as JSON: a glob, or a RegExp's source and flags. */
export type UrlPattern =
  | { glob: string }
  | { regexSource: string; regexFlags: string }

export type DescribeRequest = { op: 'describe' }
/** Query, strict check and pin. `want: 'frame'` also requires an `<iframe>` or `<frame>`. */
export type ResolveRequest = {
  op: 'resolve'
  target: ProbeTarget
  strictness: Strictness
  want: 'element' | 'frame'
}
export type ActRequest = {
  op: 'act'
  target: ProbeTarget
  strictness: Strictness
  retarget: Retarget
  states: ElementState[]
  frameVisible: boolean
  scroll: ScrollPlan
  position?: Point
  arm?: ArmPlan
}
export type VerdictRequest = { op: 'verdict'; token: number }
/**
 * How many trusted pointer moves this frame has received. Rust reads it before an action's
 * approach and again after it, and presses once it grew: the browser now routes the action's
 * point to this frame. It needs no interceptor, so a forced action checks its route too.
 */
export type MovesRequest = { op: 'moves' }
export type DisarmRequest = { op: 'disarm' }
export type FrameBoxRequest = {
  op: 'frameBox'
  iframe: ProbeTarget
  point?: Point
}
export type EditRequest = { op: 'edit'; pin: Pin; edit: Edit }
export type ReadRequest = {
  op: 'read'
  target: ProbeTarget
  strictness: Strictness
  frameVisible: boolean
  read: Read
}
export type ReadAllRequest = {
  op: 'readAll'
  target: ProbeTarget
  read: ReadAll
}
/**
 * `locator.normalize`: upstream's `generateSelectorSimple` for the first element `target`
 * matches now, in this frame. Rust asks once in the element's world and once per parent frame
 * for its `<iframe>`, then joins the parts with `internal:control=enter-frame`.
 */
export type SelectorRequest = { op: 'selector'; target: ProbeTarget }
export type SnapshotRequest = {
  op: 'snapshot'
  root: ProbeTarget
  mode: SnapshotMode
  depth?: number
  /**
   * Each node's box (`{ x, y, width, height }`, rounded) in this frame's own viewport, as
   * upstream's `boxes` option gives it: a child frame's boxes are not offset into the page.
   */
  boxes?: boolean
}
/**
 * One poll of one retrying matcher. Rust's loop owns the timing, `.not` and the failure, so a
 * poll never waits. `strictness` is `firstMatch` for the matchers of every match (count and
 * the array forms), which read all matches.
 */
export type ExpectRequest = {
  op: 'expect'
  target: ProbeTarget
  strictness: Strictness
  frameVisible: boolean
  params: ExpectParams
}
/**
 * Vendored `urlMatches(pattern, url)`. Rust names the URL, the one a navigation reported, so
 * the page serves only as JavaScript's regex engine.
 */
export type MatchUrlRequest = {
  op: 'matchUrl'
  pattern: UrlPattern
  url: string
}
/** Answers with the pinned element itself, by handle (`returnByValue: false`). */
export type PinnedRequest = { op: 'pinned'; pin: Pin }
/**
 * Which text of the document `page.title()` and `page.content()` read, or `expectSnapshot`: the
 * body's aria snapshot upstream's `Frame._expectInternal` attaches when a matcher found nothing
 * (`''` without a body, which upstream leaves out).
 */
export type DocumentPart = 'title' | 'content' | 'expectSnapshot'
/** The title, or the doctype and the root's outerHTML. */
export type DocumentRequest = { op: 'document'; part: DocumentPart }
/**
 * `page.setContent`: document.open, write and close, from this world. The Document, its window
 * and this world outlive document.open, so the world keeps its engine and its seq, as upstream's
 * frame keeps its seq when no new document commits.
 */
export type SetContentRequest = { op: 'setContent'; html: string }
/**
 * A directory upload's wait, first half: a once-listener for `input` on the pinned element, in
 * this world, armed before Rust sends `DOM.setFileInputFiles` so the event cannot come first.
 * The pin may have left its document, as upstream's listener holds its element regardless.
 * Answers `value` null, or `detached` once the pin is evicted.
 */
export type ArmInputRequest = { op: 'armInput'; pin: Pin }
/** Second half: whether that `input` has come, as `value` true or false. Rust polls it. */
export type InputArrivedRequest = { op: 'inputArrived'; pin: Pin }

/** Caller-selected screenshot overlays; cleanup is idempotent in each held world. */
export type ScreenshotMaskRequest = {
  op: 'screenshotMask'
  target: ProbeTarget
  color: string
}
export type ScreenshotMaskCleanupRequest = { op: 'screenshotMaskCleanup' }

export type ProbeRequest =
  | DescribeRequest
  | ResolveRequest
  | ActRequest
  | VerdictRequest
  | MovesRequest
  | DisarmRequest
  | FrameBoxRequest
  | EditRequest
  | ReadRequest
  | ReadAllRequest
  | SelectorRequest
  | SnapshotRequest
  | ExpectRequest
  | MatchUrlRequest
  | PinnedRequest
  | DocumentRequest
  | SetContentRequest
  | ArmInputRequest
  | InputArrivedRequest
  | ScreenshotMaskRequest
  | ScreenshotMaskCleanupRequest
export type Op = ProbeRequest['op']
/** Each request by its op, as a map so a handler table keyed by op type-checks per op. */
export type RequestMap = { [R in ProbeRequest as R['op']]: R }
export type RequestOf<O extends Op> = RequestMap[O]

/** `'done'`, or the element that would have received the pointer events instead. */
export type Hit = 'done' | { hitTargetDescription: string }
/** A rendered `<iframe>` in this frame's snapshot, for Rust to hop into. */
export type SnapshotIframe = { ref: string; depth: number; src: string }
/** How the page classified a thrown error; Rust maps each to an owner and code. */
export type ErrorClass = 'script' | 'selector' | 'unsupported' | 'internal'

export type Described = {
  kind: 'described'
  protocol: number
  seq: number
  url: string
  /**
   * Only on a boot that replaced the world's numbering: the seq it last booted with. Under
   * another epoch, another `Playwright` printed refs under that seq, so Rust never hands it out
   * on this page. Every protocol version keeps this field.
   */
  retired?: number
}
/** Zero matches. `staleRef` names a ref that can never match here again: fail at once. */
export type NoMatch = { kind: 'none'; staleRef?: string }
export type StrictViolation = { kind: 'strict'; message: string }
export type ResolvedElement = {
  kind: 'element'
  pin: Pin
  preview: string
}
/**
 * `key` names the iframe in this world's ref map, for the child frame's parent link. A hop
 * needs no preview of the iframe, so it builds none.
 */
export type ResolvedFrame = {
  kind: 'frame'
  pin: Pin
  key: string
  visible: boolean
  src: string
}
export type NotFrame = { kind: 'notFrame'; preview: string }
/** The element is gone, or its pin was evicted: back to the outer retry loop. */
export type Detached = { kind: 'detached' }
export type MissingState = { kind: 'state'; missing: ElementState }
export type Measured = {
  kind: 'measured'
  pin: Pin
  rect: Rect
  point: Point
  inView: boolean
  transformed: boolean
  /** Original element border widths plus position, independent of the stability target. */
  borderOffset?: Point
}
export type Covered = { kind: 'covered'; by: string }
export type Armed = { kind: 'armed'; rect: Rect; point: Point; token: number }
export type Verdict = { kind: 'verdict'; hit: Hit }
export type Moves = { kind: 'moves'; moves: number }
export type FrameBox = {
  kind: 'frameBox'
  origin: Point | null
  hit: Hit | null
}
/** A read or edit result. */
export type Value = { kind: 'value'; value: unknown }
/** One element's read. */
export type ReadValue = { kind: 'read'; value: unknown }
/** Every match's read. */
export type Values = {
  kind: 'values'
  count: number
  values: unknown[]
}
/** A selector upstream's generator wrote for one element. */
export type GeneratedSelector = { kind: 'selector'; selector: string }
export type Snapshot = {
  kind: 'snapshot'
  json: AriaSnapshotJSON
  iframes: SnapshotIframe[]
}
/**
 * What one poll saw. `matches` is the matcher's own result, before `.not`. `resolved` is
 * upstream's call-log text for what the locator resolved to (an element's preview, or
 * `3 elements`). `received` is the value the matcher compared; left out when it is
 * `undefined`.
 */
export type ExpectPoll = {
  kind: 'expect'
  matches: boolean
  resolved: string
  received?: unknown
  /**
   * On a poll that fails the matcher (its result equals `.not`), the aria snapshot upstream's
   * `InjectedScript.expect` puts in `received.ariaSnapshot` for `error.matcherResult`; absent
   * for count and the array forms, as upstream's.
   */
  ariaSnapshot?: string
}
export type UrlMatch = { kind: 'urlMatch'; matches: boolean; url: string }
/** The document's title or markup. */
export type DocumentText = {
  kind: 'documentText'
  text: string
}
/** The document's content was replaced; the world stays booted under the same seq. */
export type Replaced = { kind: 'replaced' }
export type ErrorReply = { kind: 'error'; message: string; class: ErrorClass }
export type Uninstalled = { kind: 'uninstalled' }

export type ProbeReply =
  | Described
  | NoMatch
  | StrictViolation
  | ResolvedElement
  | ResolvedFrame
  | NotFrame
  | Detached
  | MissingState
  | Measured
  | Covered
  | Armed
  | Verdict
  | Moves
  | FrameBox
  | Value
  | ReadValue
  | Values
  | GeneratedSelector
  | Snapshot
  | ExpectPoll
  | UrlMatch
  | DocumentText
  | Replaced
  | ErrorReply
  | Uninstalled
export type ReplyKind = ProbeReply['kind']

/** The requests answered by value. `pinned` answers by handle (the element, or null). */
export type ValueOp = Exclude<Op, 'pinned'>

/**
 * The replies each request may produce besides `error` and `uninstalled`, which any request
 * may produce. Handlers are typed against it (ReplyTo), and probe-protocol.json
 * carries it so Rust can decode each request's reply as exactly its own.
 */
export const REPLY_KINDS = {
  describe: ['described'],
  resolve: ['element', 'frame', 'notFrame', 'none', 'strict', 'detached'],
  act: ['measured', 'armed', 'covered', 'state', 'detached', 'none', 'strict'],
  verdict: ['verdict'],
  moves: ['moves'],
  disarm: ['verdict'],
  frameBox: ['frameBox', 'detached', 'none', 'strict'],
  edit: ['value', 'detached'],
  read: ['read', 'none', 'strict', 'detached'],
  readAll: ['values'],
  selector: ['selector', 'none', 'detached'],
  snapshot: ['snapshot', 'none', 'strict', 'detached'],
  expect: ['expect', 'none', 'strict', 'detached'],
  matchUrl: ['urlMatch'],
  document: ['documentText'],
  setContent: ['replaced'],
  armInput: ['value', 'detached'],
  inputArrived: ['value', 'detached'],
  screenshotMask: ['value'],
  screenshotMaskCleanup: ['value'],
} as const satisfies Record<ValueOp, readonly ReplyKind[]>

/** Everything a request may answer by value. */
export type ReplyTo<O extends ValueOp> =
  | Extract<ProbeReply, { kind: (typeof REPLY_KINDS)[O][number] }>
  | ErrorReply
  | Uninstalled

/** What `boot` takes: this document's frame seq, and the `Playwright` instance's epoch. */
export type BootOptions = { frameSeq: number; epoch: string }

/** The probe's global, `__neoProbe`, as the install expression and CALL_FUNCTION see it. */
export type NeoProbe = {
  /**
   * Builds the engine for this document and returns `described`. Idempotent for the same
   * epoch (the world keeps its seq); another epoch re-keys the world, and the reply names the
   * seq it held before as `retired`.
   */
  boot(options: BootOptions): Described
  /**
   * One request, with the call's root; see CALL_FUNCTION. Answers `uninstalled` until `boot`
   * has run.
   */
  call(request: unknown, root?: unknown): unknown
}
