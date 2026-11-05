/**
 * The probe protocol: the one contract between browseros-playwright's Rust half and Neo's
 * probe in each frame's sealed `browseros-sdk` world.
 *
 * Rust sends one ProbeRequest per `Runtime.callFunctionOn` (CALL_FUNCTION, `returnByValue`)
 * on the world's execution context, or on an element's object id, and decodes one reply.
 * The Rust mirror lives in crates/browseros-playwright/src/probe/. Neither side generates
 * the other: src/probe/protocol-examples.ts holds typed examples of every request and reply,
 * scripts/build.ts writes them to crates/browseros-playwright/tests/data/probe-protocol.json,
 * and each side decodes that file in its tests, so a shape change on one side fails the
 * other's build.
 */

import type { FrameExpectParams } from '../../../../crates/browseros-playwright/third_party/playwright/packages/injected/src/injectedScript'
import type { AriaSnapshotJSON } from '../../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'
import type { ParsedSelector } from '../../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/selectorParser'

/**
 * Bumped on any shape change. `boot` reports it, and an install whose number differs from
 * Rust's PROBE_PROTOCOL fails as `neo / internal` (the engine asset is stale).
 */
export const PROTOCOL = 4

/**
 * The function declaration Rust sends with every request. `this` is the world's global
 * object on a context call and the element on an object-id call, so the probe treats a
 * root that is not a Node as "no root". A world whose probe is gone (or never booted)
 * answers `uninstalled`, which Rust treats like a stale context: install again, once.
 */
export const CALL_FUNCTION =
  'function (request, root) { const probe = globalThis.__neoProbe; return probe ? probe.call(request, root ?? this) : { kind: "uninstalled" }; }'

/** An element this world resolved earlier in the same op; see `Pins` in world.ts. */
export type Pin = number
export type Point = { x: number; y: number }
export type Rect = { x: number; y: number; width: number; height: number }

/** What the seal's `kind(el)` reports for a field that holds a secret. */
export type SensitiveKind = 'password' | 'card' | 'one_time_code'

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
 */
export type ArmPlan = {
  action: HitAction
  blockAll: boolean
  token: number
  expectRect?: Rect
}
export type SelectOption =
  | { valueOrLabel: string }
  | { value: string }
  | { label: string }
  | { index: number }
export type Edit =
  | { kind: 'fill'; value: string }
  | { kind: 'selectOptions'; options: SelectOption[] }
  | { kind: 'selectText' }
  | { kind: 'focus'; resetSelection: boolean }
  | { kind: 'blur' }
  | { kind: 'dispatchEvent'; type: string; init: Record<string, unknown> }
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
 * the element pinned for a handle.
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
/**
 * A `frameOrigin` read's value: where the iframe's document starts in this frame's viewport,
 * and how much the iframe's own and its ancestors' transforms scale it there (rendered size
 * over layout size). `rotated` when a rotate, skew, flip or 3D transform leaves no
 * axis-aligned box to place a child's box in, which Rust refuses as `neo / unsupported`.
 */
export type FrameOrigin =
  | { x: number; y: number; scaleX: number; scaleY: number }
  | 'rotated'
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
export type SnapshotRequest = {
  op: 'snapshot'
  root: ProbeTarget
  mode: SnapshotMode
  depth?: number
}
export type ExpectRequest = {
  op: 'expect'
  target: ProbeTarget
  strictness: Strictness
  frameVisible: boolean
  params: ExpectParams
  finalPoll: boolean
}
export type SweepRequest = { op: 'sweep' }
export type FocusedRequest = { op: 'focused' }
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
/** Which text of the document `page.title()` and `page.content()` read. */
export type DocumentPart = 'title' | 'content'
/** The title, or the doctype and the root's outerHTML, read through the seal. */
export type DocumentRequest = { op: 'document'; part: DocumentPart }
/**
 * `page.setContent`: document.open, write and close, from this world. The window and this
 * world outlive document.open, so the probe unboots itself: Rust boots the new document under
 * a new seq, as for any other new document.
 */
export type SetContentRequest = { op: 'setContent'; html: string }

export type ProbeRequest =
  | DescribeRequest
  | ResolveRequest
  | ActRequest
  | VerdictRequest
  | DisarmRequest
  | FrameBoxRequest
  | EditRequest
  | ReadRequest
  | ReadAllRequest
  | SnapshotRequest
  | ExpectRequest
  | SweepRequest
  | FocusedRequest
  | MatchUrlRequest
  | PinnedRequest
  | DocumentRequest
  | SetContentRequest
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
   * Only on a boot that replaced the world's numbering: the seq it last booted with, kept
   * across an unboot. Under another epoch, another `Playwright` printed refs under that seq,
   * so Rust never hands it out on this page. Every protocol version keeps this field.
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
  sensitive: SensitiveKind | null
}
/** `key` names the iframe in this world's ref map, for the child frame's parent link. */
export type ResolvedFrame = {
  kind: 'frame'
  pin: Pin
  key: string
  preview: string
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
  sensitive: SensitiveKind | null
}
export type Covered = { kind: 'covered'; by: string }
export type Armed = { kind: 'armed'; rect: Rect; point: Point; token: number }
export type Verdict = { kind: 'verdict'; hit: Hit }
export type FrameBox = {
  kind: 'frameBox'
  origin: Point | null
  hit: Hit | null
}
/** A read or edit result. Values come through the seal, so sensitive fields read `••••`. */
export type Value = { kind: 'value'; value: unknown }
/**
 * One element's read, through the seal. `sensitive` is the seal's `kind` of the element the
 * value came from, so Rust can mark the op's target sensitive for the trace. `sweep` holds the
 * raw values of this document's sensitive fields when the value can carry page text (every
 * read but `state`, `boundingBox` and `frameOrigin`, which send none): the seal guards a field,
 * not an ancestor's text, and Rust masks the value with them. Never for the agent.
 */
export type ReadValue = {
  kind: 'read'
  value: unknown
  sensitive: SensitiveKind | null
  sweep: string[]
}
/** Every match's read; `sweep` as in `ReadValue`, none for `count`. */
export type Values = {
  kind: 'values'
  count: number
  values: unknown[]
  sweep: string[]
}
/** `sweep` holds raw sensitive values for Rust's mask, never for the agent. */
export type Snapshot = {
  kind: 'snapshot'
  json: AriaSnapshotJSON
  iframes: SnapshotIframe[]
  sweep: string[]
}
export type ExpectPoll = {
  kind: 'expect'
  matches: boolean
  received?: unknown
  ariaSnapshot?: string
}
/** Raw values of this document's sensitive fields, for Rust's mask; never for the agent. */
export type Sweep = { kind: 'sweep'; values: string[] }
/** The focused element: a sensitive field? An iframe (pinned, so Rust can hop into it)? */
export type Focused = {
  kind: 'focused'
  sensitive: boolean
  iframe: Pin | null
}
export type UrlMatch = { kind: 'urlMatch'; matches: boolean; url: string }
/**
 * Page text, which the seal already masked where it serializes a field. `sweep` holds the raw
 * values of this document's sensitive fields for Rust's mask of echoes, never for the agent.
 */
export type DocumentText = {
  kind: 'documentText'
  text: string
  sweep: string[]
}
/** The document was replaced and this world unbooted: later requests answer `uninstalled`. */
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
  | FrameBox
  | Value
  | ReadValue
  | Values
  | Snapshot
  | ExpectPoll
  | Sweep
  | Focused
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
 * may produce. Handlers are typed against it (ReplyTo), and probe-protocol.json carries it so
 * Rust can decode each request's reply as exactly its own.
 */
export const REPLY_KINDS = {
  describe: ['described'],
  resolve: ['element', 'frame', 'notFrame', 'none', 'strict', 'detached'],
  act: ['measured', 'armed', 'covered', 'state', 'detached', 'none', 'strict'],
  verdict: ['verdict'],
  disarm: ['verdict'],
  frameBox: ['frameBox', 'detached', 'none', 'strict'],
  edit: ['value', 'detached'],
  read: ['read', 'none', 'strict', 'detached'],
  readAll: ['values'],
  snapshot: ['snapshot', 'none', 'strict', 'detached'],
  expect: ['expect', 'none', 'strict', 'detached'],
  sweep: ['sweep'],
  focused: ['focused'],
  matchUrl: ['urlMatch'],
  document: ['documentText'],
  setContent: ['replaced'],
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
   * seq it held before as `retired`. Throws when the world is not sealed, so an install whose
   * seal failed never leaves a usable engine behind.
   */
  boot(options: BootOptions): Described
  /** One request; see CALL_FUNCTION. Answers `uninstalled` until `boot` has run. */
  call(request: unknown, root?: unknown): unknown
}
