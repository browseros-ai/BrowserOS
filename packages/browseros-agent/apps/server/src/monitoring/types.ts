export type MonitoringChatTurnRole = 'user' | 'assistant'

export interface MonitoringChatTurn {
  role: MonitoringChatTurnRole
  content: string
}

export interface MonitoringSessionContext {
  monitoringSessionId: string
  agentId: string
  sessionKey: string
  originalPrompt: string
  chatHistory: MonitoringChatTurn[]
  startedAt: string
  source: 'openclaw-agent-chat' | 'debug'
}

export type MonitoringToolCallSource = 'browser-tool' | 'klavis-tool'

export interface MonitoringToolCallRecord {
  monitoringSessionId: string
  agentId: string
  toolCallId: string
  toolName: string
  source: MonitoringToolCallSource
  args: unknown
  output?: unknown
  error?: string
  startedAt: string
  finishedAt?: string
  durationMs?: number
}

export interface MonitoringFinalization {
  monitoringSessionId: string
  agentId: string
  sessionKey: string
  status: 'completed' | 'failed' | 'aborted' | 'incomplete'
  finalAssistantMessage?: string
  error?: string
  finalizedAt: string
}

export interface JudgeAuditEnvelope {
  run: MonitoringSessionContext
  toolCalls: MonitoringToolCallRecord[]
  finalization?: MonitoringFinalization
}

export interface MonitoringRunSummary {
  monitoringSessionId: string
  agentId: string
  sessionKey: string
  originalPrompt: string
  startedAt: string
  source: MonitoringSessionContext['source']
  toolCallCount: number
  finalization?: Pick<
    MonitoringFinalization,
    'status' | 'finalizedAt' | 'error'
  >
}

export interface MonitoringSessionStartInput {
  agentId: string
  sessionKey: string
  originalPrompt: string
  chatHistory: MonitoringChatTurn[]
  source?: MonitoringSessionContext['source']
}

export interface MonitoringToolStartInput {
  toolCallId: string
  toolName: string
  source: MonitoringToolCallSource
  args: unknown
}

export interface MonitoringToolEndInput {
  toolCallId: string
  output?: unknown
  error?: string
}

export interface MonitoringFinalizeInput {
  monitoringSessionId: string
  agentId: string
  sessionKey: string
  status: MonitoringFinalization['status']
  finalAssistantMessage?: string
  error?: string
}
