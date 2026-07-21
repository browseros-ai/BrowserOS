import type {
  AcpPermissionDecision,
  AcpPermissionRequest,
  AcpRuntime,
  AcpRuntimeDoctorReport,
  AcpRuntimeEvent,
  AcpRuntimeHandle,
  AcpRuntimeSessionModels,
  AcpRuntimeStatus,
  AcpRuntimeTurnResult,
  AcpRuntimeTurnResultError,
  SessionAgentOptions,
  SystemPromptOption,
} from 'acpx/runtime'

export type AcpxPermissionMode = 'approve-all' | 'approve-reads' | 'deny-all'
export type AcpxNonInteractivePermissions = 'deny' | 'fail'
export type AcpxSessionMode = 'persistent' | 'oneshot'

export interface AcpxMcpServerStdio {
  type: 'stdio'
  name: string
  command: string
  args?: string[]
  env?: Record<string, string>
}

export interface AcpxMcpServerHttp {
  type: 'http' | 'sse'
  name: string
  url: string
  headers?: Record<string, string>
}

export type AcpxMcpServerConfig = AcpxMcpServerStdio | AcpxMcpServerHttp

export interface AcpxProviderSettings {
  agent: string
  cwd?: string
  sessionKey?: string
  sessionMode?: AcpxSessionMode
  permissionMode?: AcpxPermissionMode
  nonInteractivePermissions?: AcpxNonInteractivePermissions
  /**
   * Async callback invoked when the agent issues a per-call permission
   * request (e.g. write, shell, delete). Return a decision to gate the
   * call with host UI. Return `undefined` to fall through to the
   * existing `permissionMode` + `nonInteractivePermissions` logic.
   *
   * The callback is invoked while the agent is paused mid-turn waiting
   * for the JSON-RPC response — resolve quickly or honor the abort
   * signal so the agent doesn't hang.
   *
   * Note: this option is *only* honored when `runtime` is left
   * undefined (so the provider builds its own runtime). When the host
   * passes a pre-built `runtime`, the callback must be set on that
   * runtime directly.
   */
  onPermissionRequest?: (
    req: AcpPermissionRequest,
    ctx: { signal: AbortSignal },
  ) => Promise<AcpPermissionDecision | undefined>
  mcpServers?: AcpxMcpServerConfig[]
  agentRegistryOverrides?: Record<string, string>
  stateDir?: string
  resumeSessionId?: string
  turnTimeoutMs?: number
  runtime?: AcpRuntime
  /**
   * Per-session agent options forwarded to `AcpRuntime.ensureSession`.
   * Applied when a fresh ACP session is created; ignored when an existing
   * persistent session is reused (system prompts are fixed at newSession
   * time). To apply a different `systemPrompt` for the same workspace,
   * use a distinct `sessionKey`. Calling `close()` does not help here —
   * it keeps the persistent record, so the next `ensureSession` reloads
   * it and re-applies the original options.
   */
  sessionOptions?: SessionAgentOptions
  _internal?: {
    generateId?: () => string
    now?: () => Date
  }
}

export interface AcpxLanguageModelOptions {
  sessionKey?: string
  agent?: string
  mode?: string
}

export type {
  AcpPermissionDecision,
  AcpPermissionRequest,
  AcpRuntime,
  AcpRuntimeDoctorReport,
  AcpRuntimeEvent,
  AcpRuntimeHandle,
  AcpRuntimeSessionModels,
  AcpRuntimeStatus,
  AcpRuntimeTurnResult,
  AcpRuntimeTurnResultError,
  SessionAgentOptions,
  SystemPromptOption,
}
