import { randomUUID } from 'node:crypto'
import {
  AiSdkAgent,
  formatUserMessage,
} from '@browseros/server/agent/tool-loop'
import type { ResolvedAgentConfig } from '@browseros/server/agent/types'
import { Browser } from '@browseros/server/browser'
import { CdpBackend } from '@browseros/server/browser/backends/cdp'
import { CaptchaWaiter } from '../capture/captcha-waiter'
import { DEFAULT_TIMEOUT_MS } from '../constants'
import type { TaskMetadata, TokenUsage, UIMessageStreamEvent } from '../types'
import { extractDatasetMetadata } from '../utils/dataset-metadata'
import { resolveProviderConfig } from '../utils/resolve-provider-config'
import {
  addTokenUsageFromAiSdkStep,
  emptyTokenUsage,
  hasAnyTokenUsage,
} from '../utils/token-usage'
import { withEvalTimeout } from '../utils/with-eval-timeout'
import type { AgentContext, AgentEvaluator, AgentResult } from './types'

export class SingleAgentEvaluator implements AgentEvaluator {
  constructor(private ctx: AgentContext) {}

  async execute(): Promise<AgentResult> {
    const { config, task, capture, workerIndex } = this.ctx
    const startTime = Date.now()
    const timeoutMs = config.timeout_ms ?? DEFAULT_TIMEOUT_MS

    await capture.messageLogger.logUser(task.query)

    if (config.agent.type !== 'single') {
      throw new Error('SingleAgentEvaluator only supports single agent config')
    }
    const providerConfig = await resolveProviderConfig(config.agent)
    const supportsImages = config.agent.supportsImages

    // Build agent config
    const conversationId = randomUUID()
    const agentConfig: ResolvedAgentConfig = {
      ...providerConfig,
      conversationId,
      model: providerConfig.model ?? 'gpt-4o',
      workingDir: `/tmp/browseros-eval-${conversationId}`,
      evalMode: true,
      supportsImages,
    }

    // Connect to Chrome via CDP — same per-worker offset used by app-manager.
    const cdpPort = config.browseros.base_cdp_port + workerIndex
    const cdp = new CdpBackend({
      port: cdpPort,
      exitOnReconnectFailure: false,
    })
    await cdp.connect()

    const browser = new Browser(cdp)
    const browserSession = browser.session
    capture.screenshot.setBrowser(browser)

    // Build browser context so the agent knows the correct starting page ID
    const pages = await browser.listPages()
    const activePage = pages[0]
    const browserContext = activePage
      ? {
          activeTab: {
            id: activePage.tabId,
            pageId: activePage.pageId,
            url: activePage.url,
            title: activePage.title,
          },
        }
      : undefined

    const captchaWaiter = config.captcha
      ? new CaptchaWaiter({
          waitTimeoutMs: config.captcha.wait_timeout_ms,
          pollIntervalMs: config.captcha.poll_interval_ms,
        })
      : null

    let agent: AiSdkAgent | null = null
    const tokenUsage: TokenUsage = emptyTokenUsage()
    // Screenshots are taken in onToolCallFinish (per tool call). Track them by
    // toolCallId so we can stamp the matching tool-output event in messages.jsonl
    // with `screenshot: N` — this is what lets the viewer sync the agent stream
    // to the currently displayed screenshot.
    const screenshotByToolCallId = new Map<string, number>()
    let currentToolCallId: string | null = null

    try {
      agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        browser,
        browserSession,
        browserContext,
        browserUseNewTools: true,
      })

      let finalText: string | null = null
      const { terminationReason } = await withEvalTimeout(
        timeoutMs,
        capture,
        async (signal) => {
          if (!agent) throw new Error('Agent was not initialized')
          // Format prompt with browser context so the agent knows what page it's on
          // (same formatting as chat-service.ts → formatUserMessage)
          const prompt = formatUserMessage(task.query, browserContext)
          const result = await agent.toolLoopAgent.generate({
            prompt,
            abortSignal: signal,

            // biome-ignore lint/suspicious/noExplicitAny: ai-sdk option-type widening under mixed workspace zod versions; see tool-loop-executor-backend.ts for the longer note.
            experimental_onToolCallStart: ({ toolCall }: { toolCall: any }) => {
              currentToolCallId = toolCall.toolCallId
              const input = toolCall.input as
                | Record<string, unknown>
                | undefined
              if (input && typeof input.page === 'number') {
                capture.setActivePageId(input.page)
              }
            },

            experimental_onToolCallFinish: async () => {
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
                if (currentToolCallId) {
                  screenshotByToolCallId.set(currentToolCallId, screenshotNum)
                }
                capture.emitEvent(task.query_id, {
                  type: 'screenshot-captured',
                  screenshot: screenshotNum,
                })
              } catch {
                // Screenshot failures are non-fatal
              }
            },

            // biome-ignore lint/suspicious/noExplicitAny: cascade from the call-site cast; see tool-loop-executor-backend.ts for the longer note.
            onStepFinish: async (step: any) => {
              const { toolCalls, toolResults, text } = step
              addTokenUsageFromAiSdkStep(tokenUsage, step)
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
                  const screenshot = screenshotByToolCallId.get(tr.toolCallId)
                  await capture.messageLogger.logStreamEvent(
                    outputEvent,
                    screenshot,
                  )
                  capture.emitEvent(task.query_id, {
                    ...outputEvent,
                    ...(screenshot !== undefined && { screenshot }),
                  })
                  if (screenshot !== undefined) {
                    screenshotByToolCallId.delete(tr.toolCallId)
                  }
                }
              }

              if (text) {
                const textId = randomUUID()
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
            // biome-ignore lint/suspicious/noExplicitAny: ai-sdk option-type widening under mixed workspace zod versions; see tool-loop-executor-backend.ts.
          } as any)

          finalText = result.text || null
        },
      )

      const endTime = Date.now()
      const datasetMetadata = extractDatasetMetadata(task.metadata)

      const metadata: TaskMetadata = {
        query_id: task.query_id,
        dataset: task.dataset,
        query: task.query,
        started_at: new Date(startTime).toISOString(),
        completed_at: new Date(endTime).toISOString(),
        total_duration_ms: endTime - startTime,
        total_steps: capture.getScreenshotCount(),
        termination_reason: terminationReason,
        final_answer: finalText ?? capture.getLastAssistantText(),
        errors: capture.getErrors(),
        warnings: capture.getWarnings(),
        agent_config: {
          type: 'single',
          model: agentConfig.model,
        },
        grader_results: {},
        ...(hasAnyTokenUsage(tokenUsage) ? { token_usage: tokenUsage } : {}),
        ...(datasetMetadata ? { task_metadata: datasetMetadata } : {}),
      }

      await capture.trajectorySaver.saveMetadata(metadata)

      return {
        metadata,
        messages: capture.getMessages(),
        finalAnswer: finalText ?? capture.getLastAssistantText(),
      }
    } finally {
      if (agent) await agent.dispose().catch(() => {})
      await cdp.disconnect().catch(() => {})
    }
  }
}
