import type { AgentEntry } from './useOpenClaw'

export type HarnessAgentAdapter = 'claude' | 'codex' | 'openclaw'

/**
 * One file the harness attributed to the assistant turn that just
 * finished. Mirrors the server-side `ProducedFileEventEntry` shape so
 * the inline artifact card can render alongside the streamed text the
 * user just watched complete. Only present for openclaw adapter
 * turns; claude / codex don't produce these events in v1.
 */
export interface HarnessProducedFile {
  id: string
  /** Workspace-relative POSIX path. */
  path: string
  size: number
  mtimeMs: number
}

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
      type: 'produced_files'
      files: HarnessProducedFile[]
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
  message: string
  attachments?: ReadonlyArray<HarnessQueuedMessageAttachment>
}

export interface HarnessAdapterHealth {
  healthy: boolean
  reason?: string
  checkedAt: number
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
  sessionId: 'main'
  role: 'user' | 'assistant'
  text: string
  createdAt: number
  reasoning?: HarnessHistoryReasoning
  toolCalls?: HarnessHistoryToolCall[]
}

export interface HarnessAgentHistoryPage {
  agentId: string
  sessionId: 'main'
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
