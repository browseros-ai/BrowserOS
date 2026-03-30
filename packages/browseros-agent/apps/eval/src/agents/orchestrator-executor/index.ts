/**
 * Orchestrator-Executor Evaluator
 *
 * Main entry point for running tasks with orchestrator-executor pattern.
 * Implements the AgentEvaluator interface for integration with eval system.
 *
 * Sets up CDP connection, builds capture callbacks, creates executor factory,
 * and wires everything to OrchestratorAgent.
 */

import type { ResolvedAgentConfig } from '@browseros/server/agent/types'
import { Browser } from '@browseros/server/browser'
import { CdpBackend } from '@browseros/server/browser/backends/cdp'
import { CaptchaWaiter } from '../../capture/captcha-waiter'
import { DEFAULT_TIMEOUT_MS } from '../../constants'
import type {
  EvalConfig,
  OrchestratorExecutorConfig,
  TaskMetadata,
  UIMessageStreamEvent,
} from '../../types'
import { resolveEnvValue } from '../../utils/resolve-env'
import {
  type ResolvedProviderConfig,
  resolveProviderConfig,
} from '../../utils/resolve-provider-config'
import { withEvalTimeout } from '../../utils/with-eval-timeout'
import type { AgentContext, AgentEvaluator, AgentResult } from '../types'
import { Executor, type ExecutorCallbacks } from './executor'
import { OrchestratorAgent } from './orchestrator-agent'
import type { ExecutorFactory, ExecutorResult } from './types'

function extractCdpPort(config: EvalConfig): number {
  const serverUrl = config.browseros.server_url
  const match = serverUrl.match(/:(\d+)$/)
  if (!match) return config.browseros.base_cdp_port
  const serverPort = Number.parseInt(match[1], 10)
  const workerOffset = serverPort - config.browseros.base_server_port
  return config.browseros.base_cdp_port + workerOffset
}

interface ResolvedConfigs {
  orchestratorConfig: ResolvedAgentConfig & { maxTurns?: number }
  executorConfig: ResolvedAgentConfig
  isCladoAction: boolean
}

function toResolvedAgentConfig(
  resolved: ResolvedProviderConfig,
  fallbackModel: string,
  sessionPrefix: string,
): ResolvedAgentConfig {
  return {
    conversationId: crypto.randomUUID(),
    provider: resolved.provider,
    model: resolved.model ?? fallbackModel,
    apiKey: resolved.apiKey,
    baseUrl: resolved.baseUrl,
    upstreamProvider: resolved.upstreamProvider,
    resourceName: resolved.resourceName,
    region: resolved.region,
    accessKeyId: resolved.accessKeyId,
    secretAccessKey: resolved.secretAccessKey,
    sessionToken: resolved.sessionToken,
    workingDir: `/tmp/browseros-eval-${sessionPrefix}-${crypto.randomUUID()}`,
  }
}

async function resolveAgentConfig(
  config: OrchestratorExecutorConfig,
): Promise<ResolvedConfigs> {
  const orchestratorModel = config.orchestrator.model
  const executorModel = config.executor.model
  if (!orchestratorModel) {
    throw new Error('orchestrator.model is required in config')
  }
  if (!executorModel) {
    throw new Error('executor.model is required in config')
  }
  if (config.executor.provider === 'clado-action' && !config.executor.baseUrl) {
    throw new Error(
      'executor.baseUrl is required in config for clado-action provider',
    )
  }

  const resolvedOrchestrator = await resolveProviderConfig(config.orchestrator)

  const isCladoAction = config.executor.provider === 'clado-action'

  let executorConfig: ResolvedAgentConfig
  if (isCladoAction) {
    executorConfig = {
      conversationId: crypto.randomUUID(),
      provider: config.executor.provider as ResolvedAgentConfig['provider'],
      model: executorModel,
      apiKey: resolveEnvValue(config.executor.apiKey),
      baseUrl: config.executor.baseUrl,
      workingDir: `/tmp/browseros-eval-executor-${crypto.randomUUID()}`,
    }
  } else {
    const resolvedExecutor = await resolveProviderConfig(
      config.executor as Parameters<typeof resolveProviderConfig>[0],
    )
    executorConfig = toResolvedAgentConfig(
      resolvedExecutor,
      executorModel,
      'executor',
    )
  }

  const orchestratorConfig = {
    ...toResolvedAgentConfig(
      resolvedOrchestrator,
      orchestratorModel,
      'orchestrator',
    ),
    maxTurns: config.orchestrator.maxTurns,
  }

  return { orchestratorConfig, executorConfig, isCladoAction }
}

export class OrchestratorExecutorEvaluator implements AgentEvaluator {
  constructor(private ctx: AgentContext) {}

  async execute(): Promise<AgentResult> {
    const { config, task, capture } = this.ctx
    const startTime = Date.now()
    const timeoutMs = config.timeout_ms ?? DEFAULT_TIMEOUT_MS

    await capture.messageLogger.logUser(task.query)

    if (config.agent.type !== 'orchestrator-executor') {
      throw new Error(
        'OrchestratorExecutorEvaluator requires orchestrator-executor config',
      )
    }

    const agentConfig = config.agent as OrchestratorExecutorConfig
    const { orchestratorConfig, executorConfig, isCladoAction } =
      await resolveAgentConfig(agentConfig)

    // Connect to Chrome via CDP
    const cdpPort = extractCdpPort(config)
    const cdp = new CdpBackend({ port: cdpPort })
    await cdp.connect()
    const browser = new Browser(cdp)
    capture.screenshot.setBrowser(browser)

    const captchaWaiter = config.captcha
      ? new CaptchaWaiter({
          waitTimeoutMs: config.captcha.wait_timeout_ms,
          pollIntervalMs: config.captcha.poll_interval_ms,
        })
      : null

    try {
      // Build capture callbacks (same pattern as single-agent.ts)
      const callbacks: ExecutorCallbacks = {
        onToolCallStart: ({ input }) => {
          const args = input as Record<string, unknown> | undefined
          if (args && typeof args.page === 'number') {
            capture.setActivePageId(args.page)
          }
        },
        onToolCallFinish: async () => {
          try {
            if (captchaWaiter) {
              await captchaWaiter.waitIfCaptchaPresent(
                browser,
                capture.getActivePageId(),
              )
            }
            const screenshotNum = await capture.screenshot.capture(
              capture.getActivePageId(),
            )
            capture.emitEvent(task.query_id, {
              type: 'screenshot-captured',
              screenshot: screenshotNum,
            })
          } catch {
            // Screenshot failures are non-fatal
          }
        },
        onStepFinish: async ({ toolCalls, toolResults, text }) => {
          if (toolCalls) {
            for (const tc of toolCalls) {
              const inputEvent: UIMessageStreamEvent = {
                type: 'tool-input-available',
                toolCallId: tc.toolCallId,
                toolName: tc.toolName,
                input: tc.input,
              }
              await capture.messageLogger.logStreamEvent(inputEvent)
              capture.emitEvent(task.query_id, inputEvent)
            }
          }
          if (toolResults) {
            for (const tr of toolResults) {
              const outputEvent: UIMessageStreamEvent = {
                type: 'tool-output-available',
                toolCallId: tr.toolCallId,
                output: tr.output,
              }
              await capture.messageLogger.logStreamEvent(outputEvent)
              capture.emitEvent(task.query_id, outputEvent)
            }
          }
          if (text) {
            const textId = crypto.randomUUID()
            const startEvent: UIMessageStreamEvent = {
              type: 'text-start',
              id: textId,
            }
            const deltaEvent: UIMessageStreamEvent = {
              type: 'text-delta',
              id: textId,
              delta: text,
            }
            const endEvent: UIMessageStreamEvent = {
              type: 'text-end',
              id: textId,
            }
            await capture.messageLogger.logStreamEvent(startEvent)
            await capture.messageLogger.logStreamEvent(deltaEvent)
            await capture.messageLogger.logStreamEvent(endEvent)
            capture.emitEvent(task.query_id, deltaEvent)
          }
        },
      }

      // Build executor factory — logs delegation events to capture
      let delegationCount = 0
      const executorFactory: ExecutorFactory = async (instruction, signal) => {
        delegationCount++
        const delegateCallId = `delegate-${delegationCount}`

        // Log delegation start
        const delegateInputEvent: UIMessageStreamEvent = {
          type: 'tool-input-available',
          toolCallId: delegateCallId,
          toolName: 'delegate',
          input: { instruction },
        }
        await capture.messageLogger.logStreamEvent(delegateInputEvent)
        capture.emitEvent(task.query_id, delegateInputEvent)

        const executor = new Executor(
          executorConfig,
          browser,
          config.browseros.server_url,
          { isCladoAction, callbacks },
        )
        let result: ExecutorResult
        try {
          result = await executor.execute(instruction, signal)
        } finally {
          await executor.close().catch(() => {})
        }

        // Log delegation result
        const delegateOutputEvent: UIMessageStreamEvent = {
          type: 'tool-output-available',
          toolCallId: delegateCallId,
          output: {
            status: result.status,
            actionsPerformed: result.actionsPerformed,
            url: result.url,
            observation: result.observation,
          },
        }
        await capture.messageLogger.logStreamEvent(delegateOutputEvent)
        capture.emitEvent(task.query_id, delegateOutputEvent)

        return result
      }

      // Create orchestrator with factory (synchronous — no async init needed)
      const agent = OrchestratorAgent.create(orchestratorConfig, {
        executorFactory,
      })

      let finalAnswer: string | null = null

      const { terminationReason, result: agentResult } = await withEvalTimeout(
        timeoutMs,
        capture,
        async (signal) => {
          const runResult = await agent.run(task.query, signal)
          finalAnswer = runResult.answer

          if (!runResult.success) {
            capture.addError(
              'agent_execution',
              runResult.reason ?? 'Unknown failure',
            )
            if (!finalAnswer) {
              throw new Error(runResult.reason ?? 'Unknown failure')
            }
          }

          return runResult
        },
      )

      const endTime = Date.now()

      const metadata: TaskMetadata = {
        query_id: task.query_id,
        dataset: task.dataset,
        query: task.query,
        started_at: new Date(startTime).toISOString(),
        completed_at: new Date(endTime).toISOString(),
        total_duration_ms: endTime - startTime,
        total_steps:
          agentResult?.totalExecutorSteps ?? capture.getScreenshotCount(),
        termination_reason: terminationReason,
        final_answer: finalAnswer,
        errors: capture.getErrors(),
        warnings: capture.getWarnings(),
        device_pixel_ratio: capture.screenshot.getDevicePixelRatio(),
        agent_config: {
          type: 'orchestrator-executor',
          model: `${orchestratorConfig.model}/${executorConfig.model}`,
        },
        grader_results: {},
      }

      await capture.trajectorySaver.saveMetadata(metadata)

      return {
        metadata,
        messages: capture.getMessages(),
        finalAnswer,
      }
    } finally {
      await cdp.disconnect().catch(() => {})
    }
  }
}

export { Executor } from './executor'
export { OrchestratorAgent } from './orchestrator-agent'
export * from './types'
