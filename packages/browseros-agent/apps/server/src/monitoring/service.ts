import { buildJudgeAuditEnvelope } from './envelope'
import { swallowMonitoringError, type ToolExecutionObserver } from './observer'
import { MonitoringSessionRegistry } from './session-registry'
import { MonitoringStorage } from './storage'
import type {
  JudgeAuditEnvelope,
  MonitoringFinalization,
  MonitoringFinalizeInput,
  MonitoringRunSummary,
  MonitoringSessionContext,
  MonitoringSessionStartInput,
  MonitoringToolCallRecord,
  MonitoringToolEndInput,
  MonitoringToolStartInput,
} from './types'

type ActiveToolCallState = Omit<
  MonitoringToolCallRecord,
  'finishedAt' | 'durationMs' | 'error' | 'output'
>

export class MonitoringService {
  private readonly storage = new MonitoringStorage()
  private readonly registry = new MonitoringSessionRegistry()

  async startSession(
    input: MonitoringSessionStartInput,
  ): Promise<MonitoringSessionContext> {
    const context: MonitoringSessionContext = {
      monitoringSessionId: crypto.randomUUID(),
      agentId: input.agentId,
      sessionKey: input.sessionKey,
      originalPrompt: input.originalPrompt,
      chatHistory: input.chatHistory,
      startedAt: new Date().toISOString(),
      source: input.source ?? 'openclaw-agent-chat',
    }

    await this.storage.writeContext(context)
    this.registry.setActive(context.agentId, context.monitoringSessionId)
    return context
  }

  getActiveSessionId(agentId: string): string | undefined {
    return this.registry.getActive(agentId)
  }

  getSingleActiveSession():
    | { agentId: string; monitoringSessionId: string }
    | undefined {
    return this.registry.getSingleActive()
  }
  clearActiveSession(agentId: string, monitoringSessionId: string): void {
    this.registry.clearIfMatches(agentId, monitoringSessionId)
  }

  createObserver(
    monitoringSessionId: string,
    agentId: string,
  ): ToolExecutionObserver {
    const activeToolCalls = new Map<string, ActiveToolCallState>()

    return {
      onToolStart: async (input: MonitoringToolStartInput) => {
        try {
          activeToolCalls.set(input.toolCallId, {
            monitoringSessionId,
            agentId,
            toolCallId: input.toolCallId,
            toolName: input.toolName,
            source: input.source,
            args: input.args,
            startedAt: new Date().toISOString(),
          })
        } catch (error) {
          swallowMonitoringError('tool start recording', error, {
            monitoringSessionId,
            agentId,
            toolCallId: input.toolCallId,
            toolName: input.toolName,
          })
        }
      },

      onToolEnd: async (input: MonitoringToolEndInput) => {
        try {
          const active = activeToolCalls.get(input.toolCallId)
          if (!active) return

          const finishedAt = new Date().toISOString()
          const durationMs = Math.max(
            0,
            new Date(finishedAt).getTime() -
              new Date(active.startedAt).getTime(),
          )

          const record: MonitoringToolCallRecord = {
            ...active,
            finishedAt,
            durationMs,
          }

          if (input.error) {
            record.error = input.error
          }
          if (input.output !== undefined) {
            record.output = input.output
          }

          await this.storage.appendToolCall(record)
          activeToolCalls.delete(input.toolCallId)
        } catch (error) {
          swallowMonitoringError('tool end recording', error, {
            monitoringSessionId,
            agentId,
            toolCallId: input.toolCallId,
          })
        }
      },
    }
  }

  async finalizeSession(
    input: MonitoringFinalizeInput,
  ): Promise<JudgeAuditEnvelope | null> {
    const context = await this.storage.readContext(input.monitoringSessionId)
    if (!context) {
      return null
    }

    const finalization: MonitoringFinalization = {
      monitoringSessionId: input.monitoringSessionId,
      agentId: input.agentId,
      sessionKey: input.sessionKey,
      status: input.status,
      finalizedAt: new Date().toISOString(),
    }

    if (input.finalAssistantMessage) {
      finalization.finalAssistantMessage = input.finalAssistantMessage
    }
    if (input.error) {
      finalization.error = input.error
    }

    await this.storage.writeFinalization(finalization)
    this.registry.clearIfMatches(input.agentId, input.monitoringSessionId)
    return this.buildAndPersistEnvelope(input.monitoringSessionId)
  }

  async getRunEnvelope(runId: string): Promise<JudgeAuditEnvelope | null> {
    const context = await this.storage.readContext(runId)
    if (!context) return null

    const toolCalls = await this.storage.readToolCalls(runId)
    const finalization = await this.storage.readFinalization(runId)

    return buildJudgeAuditEnvelope({
      context,
      toolCalls,
      finalization,
    })
  }

  async listRuns(limit = 50): Promise<MonitoringRunSummary[]> {
    const runIds = (await this.storage.listRunIds()).slice(0, limit)
    const summaries = await Promise.all(
      runIds.map(async (runId) => {
        const context = await this.storage.readContext(runId)
        if (!context) return null

        const [toolCalls, finalization] = await Promise.all([
          this.storage.readToolCalls(runId),
          this.storage.readFinalization(runId),
        ])

        const summary: MonitoringRunSummary = {
          monitoringSessionId: context.monitoringSessionId,
          agentId: context.agentId,
          sessionKey: context.sessionKey,
          originalPrompt: context.originalPrompt,
          startedAt: context.startedAt,
          source: context.source,
          toolCallCount: toolCalls.length,
        }

        if (finalization) {
          summary.finalization = {
            status: finalization.status,
            finalizedAt: finalization.finalizedAt,
            error: finalization.error,
          }
        }

        return summary
      }),
    )

    return summaries.filter((summary): summary is MonitoringRunSummary =>
      Boolean(summary),
    )
  }

  private async buildAndPersistEnvelope(
    runId: string,
  ): Promise<JudgeAuditEnvelope | null> {
    const envelope = await this.getRunEnvelope(runId)
    if (!envelope) return null

    await this.storage.writeAuditEnvelope(runId, envelope)
    return envelope
  }
}

let monitoringService: MonitoringService | null = null

export function getMonitoringService(): MonitoringService {
  if (!monitoringService) {
    monitoringService = new MonitoringService()
  }
  return monitoringService
}
