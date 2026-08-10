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
  /** Gates per-call permissions when the provider constructs the runtime. */
  onPermissionRequest?: (
    req: AcpPermissionRequest,
    ctx: { signal: AbortSignal },
  ) => Promise<AcpPermissionDecision | undefined>
  mcpServers?: AcpxMcpServerConfig[]
  agentRegistryOverrides?: Record<string, string | string[]>
  stateDir?: string
  resumeSessionId?: string
  turnTimeoutMs?: number
  runtime?: AcpRuntime
  /** Applied only when creating a fresh ACP session; use a new sessionKey to change them. */
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
