/**
 * Desktop contracts describe intent. The harness owns case tasks and settings; the suite owns
 * its SDK process, while test:browser owns browser startup and shutdown.
 */
export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | JsonObject
export interface JsonObject {
  [key: string]: JsonValue
}

export type Step =
  | { kind: 'script'; script: string }
  | { kind: 'parallel'; scripts: readonly [string, string, string, string] }

export type FailureSignature =
  | 'hidden-label-ref'
  | 'isolated-child-ref'
  | 'card-child-ref'
  | 'invalid-date-reset'

export type Expectation =
  | { kind: 'pass' }
  | { kind: 'xfail'; reason: string; signature: FailureSignature }

export type SettingsPatch =
  | { tabs: { scope: 'browser' } }
  | { trace: { valueBytes: 262144 } }

export type HostSetup =
  | { kind: 'borrowed-page'; urlTemplate: string }
  | { kind: 'device-scale'; deviceScaleFactor: 2 }

export type HostCheck =
  | {
      kind: 'capture-bytes'
      magicHex?: string
      magicUtf8?: string
      mimeType?: string
      requireImageBlock?: boolean
      requirePersistedExactBytes?: boolean
    }
  | {
      kind: 'download-file'
      pathField: string
      contentIncludes: string
      contentEquals?: string
      requireExists: true
    }
  | {
      kind: 'download-files'
      pathsField: string
      contentIncludes: string
      requireExists: true
    }
  | {
      kind: 'typed-json'
      count: number
      textRepeat: string
      times: number
      requirePersistedResult: true
    }
  | {
      kind: 'nested-capture-exact-bytes'
      fields: readonly string[]
      sizes: Readonly<Record<string, string>>
      requiredValue: Readonly<Record<string, JsonValue>>
      compareReturnedAndPersistedEveryByte: true
      sameRunId: true
    }
  | { kind: 'run-log'; text: string; value: JsonValue }
  | { kind: 'run-error'; messageIncludes: string; requireIsError: true }

/** An expanded desktop alias executes afresh even when another alias uses the same body. */
export interface CaseSpec {
  id: string
  file: string
  name: string
  proves: string
  steps: readonly [Step, ...Step[]]
  timeoutMs: number
  /** An independent caller deadline; omitted requests retain the legacy transport budget. */
  driverTimeoutMs?: number
  expectation: Expectation
  settings?: SettingsPatch
  hostSetup?: HostSetup
  hostCheck?: HostCheck
  postconditions?: readonly string[]
  source?: string
  sourceSha?: string
}

export interface RunError {
  owner: string
  code: string
  message: string
}

export type RunId = string & { readonly __runId: unique symbol }

/** SDK run IDs are UUIDs, unlike caller-chosen case, request and task identifiers. */
export function isRunId(value: unknown): value is RunId {
  return (
    typeof value === 'string' &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
      value,
    )
  )
}

interface ReplyBase {
  runId: RunId
  status: string
  text: string
  images: readonly { data: string; mimeType: string }[]
  raw: JsonValue
}

/** The MCP boundary validates envelopes once, then exposes a typed success or failure. */
export type RunReply =
  | (ReplyBase & { kind: 'ok'; isError: false; value: JsonValue })
  | (ReplyBase & { kind: 'error'; isError: true; error: RunError })

/** Public rulings are pinned to code or documentation that a clean checkout can inspect. */
export interface PublicCitation {
  path: string
  commit: string
  lines: string
  claim: string
}

export interface SdkRuling {
  sdkBehavior: string
  citations: readonly [PublicCitation, ...PublicCitation[]]
}

/** Authored contracts own their assertions; imported body bytes keep their historical owner. */
export type RuledCaseSpec = CaseSpec & {
  expectation: { kind: 'pass' }
  ruling: SdkRuling
} & (
    | { execution: 'ordinary' }
    | {
        execution: 'density'
        hostSetup: { kind: 'device-scale'; deviceScaleFactor: 2 }
      }
    | { execution: 'terminal-select'; id: 'act-017'; file: 'cases-act.ts' }
  )

/** The composed registry has one execution policy, independent of historical/authored provenance. */
export type ExecutableCase =
  | (CaseSpec & { execution: 'ordinary' | 'density' })
  | Extract<RuledCaseSpec, { execution: 'terminal-select' }>

export interface RunRequest {
  code: string
  taskId: string
  requestId: string
  timeoutMs: number
  driverTimeoutMs?: number
}

export interface RunDownload {
  index: number
  path?: string
  size?: number
  state: string
  suggestedFilename: string
}

export interface RunRecord {
  id: RunId
  status: string
  complete: boolean
  result?: JsonValue
  resultTruncated: boolean
  downloads: readonly RunDownload[]
}

export interface HostBytes {
  bytes: Uint8Array
  contentLength?: number
}

/** Authenticated host reads stay on this invocation's private Unix socket. */
export interface HostClient {
  settings(): Promise<JsonObject>
  patchSettings(patch: SettingsPatch): Promise<JsonObject>
  restoreSettings(settings: JsonObject): Promise<JsonObject>
  record(runId: RunId): Promise<RunRecord>
  blob(runId: RunId, blobId: string): Promise<HostBytes>
  download(runId: RunId, index: number): Promise<HostBytes>
}

export interface BrowserRecord {
  profile: string
  home: string
  downloads: string
  args: string[]
  scale: 1 | 2
}

export interface ServerHandle {
  baseUrl: string
  socketPath: string
  token: string
  stop(): Promise<void>
}

export type CaseStatus = 'pass' | 'xfail' | 'fail'
interface CaseResultBase {
  id: string
  file: string
  name: string
  durationMs: number
  cleanupMs: number
  runIds: RunId[]
}

export type CaseResult = CaseResultBase &
  (
    | { status: 'pass' }
    | { status: 'xfail'; reason: string }
    | { status: 'fail'; failure: string }
  )
