/**
 * Probe lifecycle constants and projections of the Rust-generated contract. Request/reply
 * fields live only in Rust; `bun run gen:probe` writes generated.ts and the compare-only test
 * rejects stale output. The examples still guard behavior and recorded protocol bytes.
 */
import type { ProbeReply, ProbeRequest } from './generated'
// Include the implementation's global ABI for type consumers without importing startup code.
import type {} from './index'

export * from './generated'

/** The named ABI view follows the implementation's declared global. */
export type NeoProbe = NonNullable<typeof globalThis.__neoProbe>

/**
 * Bumped on any shape change, and on any change to what Rust relies on the probe doing. `boot`
 * reports it, and an install whose number differs from Rust's PROBE_PROTOCOL fails as
 * `neo / internal` (the engine asset is stale).
 */
export const PROTOCOL = 26

/**
 * The function declaration Rust sends with every request. Rust calls it on the world's global
 * object, with the request and an element handle's node as the call's root (or null), so the
 * probe treats a root that is not a Node as "no root". A world whose probe is gone (or never
 * booted) answers `uninstalled`, which Rust treats like a stale context: install again, once.
 */
export const CALL_FUNCTION =
  'function (request, root) { const probe = globalThis.__neoProbe; return probe ? probe.call(request, root ?? this) : { kind: "uninstalled" }; }'

/** Per-op and per-kind views add names without repeating any generated fields. */
export type Op = ProbeRequest['op']
export type RequestMap = { [R in ProbeRequest as R['op']]: R }
export type RequestOf<O extends Op> = RequestMap[O]
export type ReplyKind = ProbeReply['kind']
export type ValueOp = Exclude<Op, 'pinned'>

export type DescribeRequest = RequestOf<'describe'>
export type ResolveRequest = RequestOf<'resolve'>
export type ActRequest = RequestOf<'act'>
export type VerdictRequest = RequestOf<'verdict'>
export type MovesRequest = RequestOf<'moves'>
export type DisarmRequest = RequestOf<'disarm'>
export type FrameBoxRequest = RequestOf<'frameBox'>
export type EditRequest = RequestOf<'edit'>
export type ReadRequest = RequestOf<'read'>
export type ReadAllRequest = RequestOf<'readAll'>
export type SelectorRequest = RequestOf<'selector'>
export type SnapshotRequest = RequestOf<'snapshot'>
export type ExpectRequest = RequestOf<'expect'>
export type MatchUrlRequest = RequestOf<'matchUrl'>
export type PinnedRequest = RequestOf<'pinned'>
export type DocumentRequest = RequestOf<'document'>
export type SetContentRequest = RequestOf<'setContent'>
export type ArmInputRequest = RequestOf<'armInput'>
export type InputArrivedRequest = RequestOf<'inputArrived'>
export type ScreenshotMaskRequest = RequestOf<'screenshotMask'>
export type ScreenshotMaskCleanupRequest = RequestOf<'screenshotMaskCleanup'>
export type WebStorageRequest = RequestOf<'webStorage'>

export type Described = Extract<ProbeReply, { kind: 'described' }>
export type NoMatch = Extract<ProbeReply, { kind: 'none' }>
export type StrictViolation = Extract<ProbeReply, { kind: 'strict' }>
export type ResolvedElement = Extract<ProbeReply, { kind: 'element' }>
export type ResolvedFrame = Extract<ProbeReply, { kind: 'frame' }>
export type NotFrame = Extract<ProbeReply, { kind: 'notFrame' }>
export type Detached = Extract<ProbeReply, { kind: 'detached' }>
export type MissingState = Extract<ProbeReply, { kind: 'state' }>
export type Measured = Extract<ProbeReply, { kind: 'measured' }>
export type Covered = Extract<ProbeReply, { kind: 'covered' }>
export type Armed = Extract<ProbeReply, { kind: 'armed' }>
export type Verdict = Extract<ProbeReply, { kind: 'verdict' }>
export type Moves = Extract<ProbeReply, { kind: 'moves' }>
export type FrameBox = Extract<ProbeReply, { kind: 'frameBox' }>
export type Value = Extract<ProbeReply, { kind: 'value' }>
export type ReadValue = Extract<ProbeReply, { kind: 'read' }>
export type Values = Extract<ProbeReply, { kind: 'values' }>
export type GeneratedSelector = Extract<ProbeReply, { kind: 'selector' }>
export type Snapshot = Extract<ProbeReply, { kind: 'snapshot' }>
export type ExpectPoll = Extract<ProbeReply, { kind: 'expect' }>
export type UrlMatch = Extract<ProbeReply, { kind: 'urlMatch' }>
export type DocumentText = Extract<ProbeReply, { kind: 'documentText' }>
export type Replaced = Extract<ProbeReply, { kind: 'replaced' }>
export type ErrorReply = Extract<ProbeReply, { kind: 'error' }>
export type Uninstalled = Extract<ProbeReply, { kind: 'uninstalled' }>

/** Allowed handler outcomes are behavior metadata, not a second copy of reply fields. */
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
  webStorage: ['value'],
} as const satisfies Record<ValueOp, readonly ReplyKind[]>

/** Everything a by-value request may answer, including universal failure replies. */
export type ReplyTo<O extends ValueOp> =
  | Extract<ProbeReply, { kind: (typeof REPLY_KINDS)[O][number] }>
  | ErrorReply
  | Uninstalled
