/**
 * Neo: author-facing declarations for a hook file's injected `neo` parameter.
 * Reference this file for editor/typecheck support; it is not a runtime import.
 */
import type {
  BrowserContext,
  ElementHandle,
  Frame,
  Locator,
  Page,
} from 'playwright-core'
import type { HookGroup, HookKind } from './hooks-kinds'

export type { HookGroup, HookKind } from './hooks-kinds'

/** JSON crossing the host boundary; metadata is recursively frozen at load time. */
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue }

/** At least one list; kinds OR groups match. Every supplied list must be non-empty; unknown names fail loading. */
export type HookFilter =
  | { kinds: readonly HookKind[]; groups?: readonly HookGroup[] }
  | { kinds?: readonly HookKind[]; groups: readonly HookGroup[] }

export interface HookContext {
  readonly meta: JsonValue
  /** Mutate this shared, per-run object; do not replace it. */
  readonly state: Record<string, unknown>
  readonly signal: AbortSignal
  /** Host-only trace, never the model's logs. */
  trace(...values: unknown[]): void
  /** JSON request/answer, each at most 1 MiB; waiting uses the invocation budget. */
  host(name: string, args?: JsonValue): Promise<JsonValue>
}

export interface HookError {
  name: string
  message: string
}

export interface BeforeRunEvent {
  script: string
  page: Page
  context: BrowserContext
  readonly meta: JsonValue
}

/** No `blocked`: afterRun is skipped when beforeRun blocks or loading fails. */
export interface AfterRunEvent {
  status: 'ok' | 'error' | 'timeout' | 'cancelled'
  value?: unknown
  error?: HookError
  page: Page
  context: BrowserContext
  readonly meta: JsonValue
}

/** A served browser call; element input is checked after its target resolves. */
export interface BeforeActionEvent {
  kind: HookKind | null
  group: HookGroup | null
  method: string
  /** Wire arguments, not positional Playwright arguments. */
  args: { [key: string]: JsonValue }
  /** One-based location in the agent script, when supplied. */
  location?: { line: number; column?: number }
  page?: Page
  frame?: Frame
  context: BrowserContext
  /** Selector-based description; querying it can resolve a different node. */
  locator?: Locator
  /** Exact input target, hook-only; valid only while this call is alive. */
  element?: ElementHandle
  readonly meta: JsonValue
}

/** Wire reply, before the SDK unwraps it. A failed/void call has no replaceable result. */
export interface AfterActionEvent extends BeforeActionEvent {
  result?: unknown
  error?: HookError
}

export type Allow = {
  allow: true
  block?: never
  replaceArgs?: never
  replace?: never
  note?: never
}
/** A non-empty reason; shown to the model. */
export type Block = {
  block: string
  allow?: never
  replaceArgs?: never
  replace?: never
  note?: never
}
export type ReplaceArgs = {
  replaceArgs: { readonly [key: string]: JsonValue }
  allow?: never
  block?: never
  replace?: never
  note?: never
}

export type BeforeRunDecision = undefined | Allow | Block
export type BeforeActionDecision = BeforeRunDecision | ReplaceArgs
/** Key presence matters: `{ replace: undefined }` replaces; `{}` is invalid. */
export type AfterRunDecision =
  | undefined
  | ({ replace: unknown; note?: string } & AfterOnly)
  | ({ note: string; replace?: unknown } & AfterOnly)

/** Successful, non-void wire results only; runtime checks the method-specific shape. */
export type AfterActionDecision = AfterRunDecision
/** Shared after-event syntax, retained for author tooling. */
export type AfterDecision = AfterRunDecision

interface AfterOnly {
  allow?: never
  block?: never
  replaceArgs?: never
}

export type HookHandler<Event, Decision> = (
  event: Event,
  ctx: HookContext,
) => Decision | Promise<Decision>

export interface NeoHooks {
  on(
    event: 'beforeRun',
    handler: HookHandler<BeforeRunEvent, BeforeRunDecision>,
  ): void
  on(
    event: 'afterRun',
    handler: HookHandler<AfterRunEvent, AfterRunDecision>,
  ): void
  /** Null/undefined mean no filter; run events do not accept a filter object. */
  on(
    event: 'beforeRun',
    filter: null | undefined,
    handler: HookHandler<BeforeRunEvent, BeforeRunDecision>,
  ): void
  on(
    event: 'afterRun',
    filter: null | undefined,
    handler: HookHandler<AfterRunEvent, AfterRunDecision>,
  ): void
  on(
    event: 'beforeAction',
    handler: HookHandler<BeforeActionEvent, BeforeActionDecision>,
  ): void
  on(
    event: 'afterAction',
    handler: HookHandler<AfterActionEvent, AfterActionDecision>,
  ): void
  on(
    event: 'beforeAction',
    filter: HookFilter | null | undefined,
    handler: HookHandler<BeforeActionEvent, BeforeActionDecision>,
  ): void
  on(
    event: 'afterAction',
    filter: HookFilter | null | undefined,
    handler: HookHandler<AfterActionEvent, AfterActionDecision>,
  ): void
}

declare global {
  /** Neo: the runtime supplies this parameter to the hook file's async function body. */
  const neo: NeoHooks
}
