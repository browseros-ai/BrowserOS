export interface AgentEntry {
  agentId: string
  name: string
  workspace: string
  model?: unknown
  source?: 'agent-harness'
}

export type HarnessAgentAdapter = 'claude' | 'codex' | 'hermes'

export type AgentHarnessStreamEvent =
  | {
      type: 'text_delta'
      text: string
      stream: 'output' | 'thought'
      rawType?: string
    }
  | {
      type: 'tool_call'
      text: string
      title: string
      id?: string
      status?: string
      rawType?: string
    }
  | {
      type: 'status'
      text: string
      rawType?: string
    }
  | {
      type: 'done'
      text?: string
      stopReason?: string
    }
  | {
      type: 'error'
      message: string
      code?: string
    }

export type HarnessAgentLiveness = 'working' | 'idle' | 'asleep' | 'error'

export interface HarnessAgent {
  id: string
  name: string
  adapter: HarnessAgentAdapter
  modelId?: string
  reasoningEffort?: string
  permissionMode: 'approve-all'
  sessionKey: string
  createdAt: number
  updatedAt: number
  /**
   * Server-derived liveness state. When the listing endpoint hasn't
   * been enriched yet (older deployments) this is undefined and the UI
   * falls back to `unknown`.
   */
  status?: HarnessAgentLiveness
  /**
   * Wall-clock ms of the last persisted turn. `null` for never-used
   * agents. Drives the recency sort and the "Last used X min ago" copy.
   */
  lastUsedAt?: number | null
  /** Pinned agents float to the top of the list. Defaults to `false`. */
  pinned?: boolean
  /** First non-blank line of the most recent user message; null if none. */
  lastUserMessage?: string | null
  /** Working directory the agent runs in; null when no session record yet. */
  cwd?: string | null
  /** Cumulative + 7-day rolling token usage; null when no record. */
  tokens?: {
    last7d: { input: number; output: number; requestCount: number }
    cumulative: { input: number; output: number }
  } | null
  turnsByDay?: number[]
  failedByDay?: number[]
  lastError?: string | null
  lastErrorAt?: number | null
  /** Latest persisted conversation session for this agent. */
  latestSessionId?: string | null
  /** When non-null, an in-flight turn this row can be resumed from. */
  activeTurnId?: string | null
  /** Persistent FIFO queue of messages waiting for this agent. */
  queue?: HarnessQueuedMessage[]
}

export interface HarnessQueuedMessageAttachment {
  mediaType: string
  data: string
}

export interface HarnessQueuedMessage {
  id: string
  createdAt: number
  sessionId?: string
  message: string
  attachments?: ReadonlyArray<HarnessQueuedMessageAttachment>
}

export interface HarnessAdapterHealth {
  healthy: boolean
  reason?: string
  checkedAt: number
  readiness?:
    | 'ready'
    | 'needs-auth'
    | 'needs-install'
    | 'will-fetch-package'
    | 'diagnostic-warning'
    | 'unknown'
  installState?:
    | 'installed'
    | 'npx-available'
    | 'package-runner-available'
    | 'not-installed'
  nativeCliState?: 'present' | 'missing' | 'unknown'
  authState?: 'authenticated' | 'unauthenticated' | 'not-applicable' | 'unknown'
  version?: string
  adapterLaunchSource?:
    | 'bundled-bun'
    | 'host-npx'
    | 'host-cli'
    | 'runtime'
    | 'none'
  packageCacheState?: 'cached' | 'fetch-required' | 'unknown'
}

export interface HarnessAdapterDescriptor {
  id: HarnessAgentAdapter
  name: string
  defaultModelId: string
  defaultReasoningEffort: string
  modelControl: 'runtime-supported' | 'best-effort'
  models: Array<{ id: string; label: string; recommended?: boolean }>
  reasoningEfforts: Array<{ id: string; label: string; recommended?: boolean }>
  health?: HarnessAdapterHealth
}

export interface CreateHarnessAgentInput {
  name: string
  adapter: HarnessAgentAdapter
  modelId?: string
  reasoningEffort?: string
  /**
   * Adapter provider id from the user's BrowserOS AI Settings entry.
   * Provider-backed adapters use this with `apiKey`/`baseUrl` to write
   * or provision their runtime-specific provider config.
   */
  providerType?: string
  /** API key paired with `providerType` when the selected adapter needs one. */
  apiKey?: string
  /** Base URL for OpenAI-compatible/custom provider entries. */
  baseUrl?: string
}

export interface HarnessHistoryReasoning {
  text: string
  durationMs?: number
}

export interface HarnessHistoryToolCall {
  toolCallId?: string
  toolName: string
  status: 'pending' | 'running' | 'completed' | 'failed'
  input?: unknown
  output?: unknown
  error?: string
  durationMs?: number
}

export interface HarnessHistoryEntry {
  id: string
  agentId: string
  sessionId: string
  role: 'user' | 'assistant'
  text: string
  createdAt: number
  reasoning?: HarnessHistoryReasoning
  toolCalls?: HarnessHistoryToolCall[]
}

export interface HarnessAgentHistoryPage {
  agentId: string
  sessionId: string
  items: HarnessHistoryEntry[]
}

export function mapHarnessAgentToEntry(agent: HarnessAgent): AgentEntry {
  return {
    agentId: agent.id,
    name: agent.name,
    workspace: `${agent.adapter}:main`,
    model: agent.modelId,
    source: 'agent-harness',
  }
}
