import { buildJudgeAuditEnvelope } from './envelope'
import { LazyMonitoringJudgeError } from './judge/llm-judge'
import type { LazyMonitoringJudgeService } from './judge/service'
import { createLazyMonitoringJudgeService } from './judge/service'
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

interface MonitoringServiceDeps {
  storage?: MonitoringStorage
  registry?: MonitoringSessionRegistry
  judge?: LazyMonitoringJudgeService
}

export class MonitoringService {
  private readonly storage: MonitoringStorage
  private readonly registry: MonitoringSessionRegistry
  private readonly judge: LazyMonitoringJudgeService
  private readonly completedToolCallsBySession = new Map<
    string,
    MonitoringToolCallRecord[]
  >()

  constructor(deps: MonitoringServiceDeps = {}) {
    this.storage = deps.storage ?? new MonitoringStorage()
    this.registry = deps.registry ?? new MonitoringSessionRegistry()
    this.judge = deps.judge ?? createLazyMonitoringJudgeService()
  }

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
    this.registry.setActive(
      context.agentId,
      context.monitoringSessionId,
      context.source,
    )
    this.completedToolCallsBySession.set(context.monitoringSessionId, [])
    return context
  }

  getActiveSessionId(agentId: string): string | undefined {
    return this.registry.getActive(agentId)
  }

  /**
   * Resolve when no monitoring session is active for `agentId`. Used by the
   * chat route to gate user-chat sends behind any in-flight cron / hook turn
   * without rejecting the client outright.
   *
   * Resolves immediately if the agent is already free. Otherwise registers
   * a one-shot listener on the session-end event and resolves when it
   * fires. Rejects with a TimeoutError-shaped Error after `timeoutMs`.
   */
  async waitForSessionFree(
    agentId: string,
    options: { timeoutMs?: number } = {},
  ): Promise<void> {
    if (!this.registry.getActive(agentId)) return

    const timeoutMs = options.timeoutMs ?? 30_000

    return new Promise<void>((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | null = null
      let unsubscribe: (() => void) | null = null

      const cleanup = () => {
        if (timer) clearTimeout(timer)
        unsubscribe?.()
      }

      timer = setTimeout(() => {
        cleanup()
        reject(
          new Error(
            `Timed out waiting for agent "${agentId}" to become free after ${timeoutMs}ms`,
          ),
        )
      }, timeoutMs)

      unsubscribe = this.registry.onSessionEnd(agentId, () => {
        if (this.registry.getActive(agentId)) return
        cleanup()
        resolve()
      })

      // Re-check after listener registration to close a race where the
      // session ended between the initial getActive() and the subscribe.
      if (!this.registry.getActive(agentId)) {
        cleanup()
        resolve()
      }
    })
  }

  resolveSessionForMcpRequest(
    explicitAgentId?: string,
  ): { agentId: string; monitoringSessionId: string } | undefined {
    if (explicitAgentId) {
      const monitoringSessionId = this.registry.getActive(explicitAgentId)
      return monitoringSessionId
        ? { agentId: explicitAgentId, monitoringSessionId }
        : undefined
    }

    return this.registry.resolveForUnattributedToolCalls()
  }

  clearActiveSession(agentId: string, monitoringSessionId: string): void {
    this.registry.clearIfMatches(agentId, monitoringSessionId)
  }

  createObserver(
    monitoringSessionId: string,
    agentId: string,
  ): ToolExecutionObserver {
    const activeToolCalls = new Map<string, ActiveToolCallState>()
    const completedToolCalls =
      this.completedToolCallsBySession.get(monitoringSessionId) ?? []
    this.completedToolCallsBySession.set(
      monitoringSessionId,
      completedToolCalls,
    )
    const contextPromise = this.storage.readContext(monitoringSessionId)
    let judgeQueue = Promise.resolve()

    const enqueueJudgeReview = (toolCall: ActiveToolCallState): void => {
      const priorToolCalls = [...completedToolCalls]

      judgeQueue = judgeQueue
        .catch(() => undefined)
        .then(async () => {
          const context = await contextPromise
          if (!context) {
            return
          }

          const judgment = await this.judge.evaluate({
            run: context,
            priorToolCalls,
            currentToolCall: toolCall,
          })

          console.log(
            JSON.stringify({
              type: 'lazy-monitoring-judge',
              monitoringSessionId,
              agentId,
              originalPrompt: context.originalPrompt,
              toolCallId: judgment.toolCallId,
              toolName: judgment.toolName,
              verdict: judgment.verdict,
              summary: judgment.summary,
              mode: judgment.mode,
              destructive: judgment.destructive,
              categories: judgment.categories,
              matchedIntentCategories: judgment.matchedIntentCategories,
              policyDimensions: judgment.policyDimensions,
              policyVersion: judgment.policyVersion,
              model: judgment.model,
              shouldInterrupt: judgment.shouldInterrupt,
            }),
          )
        })
        .catch((error) => {
          if (error instanceof LazyMonitoringJudgeError) {
            const errorPayload: Record<string, unknown> = {
              type: 'lazy-monitoring-judge-error',
              monitoringSessionId,
              agentId,
              toolCallId: toolCall.toolCallId,
              toolName: toolCall.toolName,
              error: error.message,
              stack: error.stack,
            }
            if (error.cause) {
              const cause = error.cause
              errorPayload.cause =
                cause instanceof Error
                  ? {
                      message: cause.message,
                      name: cause.name,
                      stack: cause.stack,
                    }
                  : String(cause)
            }
            console.error(JSON.stringify(errorPayload))
            this.storage
              .appendErrorLog(monitoringSessionId, errorPayload)
              .catch(() => {})
            return
          }

          swallowMonitoringError('judge review', error, {
            monitoringSessionId,
            agentId,
            toolCallId: toolCall.toolCallId,
            toolName: toolCall.toolName,
          })
        })
    }

    return {
      onToolStart: async (input: MonitoringToolStartInput) => {
        try {
          const toolCall: ActiveToolCallState = {
            monitoringSessionId,
            agentId,
            toolCallId: input.toolCallId,
            toolName: input.toolName,
            toolDescription: input.toolDescription,
            source: input.source,
            args: input.args,
            startedAt: new Date().toISOString(),
          }
          activeToolCalls.set(input.toolCallId, toolCall)
          enqueueJudgeReview(toolCall)
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
          completedToolCalls.push(record)
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
    const envelope = await this.buildAndPersistEnvelope(
      input.monitoringSessionId,
    )
    this.completedToolCallsBySession.delete(input.monitoringSessionId)
    return envelope
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
