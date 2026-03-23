import { randomUUID } from 'node:crypto'
import {
  AiSdkAgent,
  formatUserMessage,
} from '@browseros/server/agent/tool-loop'
import type { ResolvedAgentConfig } from '@browseros/server/agent/types'
import { Browser } from '@browseros/server/browser'
import { CdpBackend } from '@browseros/server/browser/backends/cdp'
import { registry } from '@browseros/server/tools/registry'
import { DEFAULT_TIMEOUT_MS } from '../constants'
import type { EvalConfig, TaskMetadata } from '../types'
import { resolveProviderConfig } from '../utils/resolve-provider-config'
import { withEvalTimeout } from '../utils/with-eval-timeout'
import type { AgentContext, AgentEvaluator, AgentResult } from './types'

const CONTROLLER_STUB = {
  start: async () => {},
  stop: async () => {},
  isConnected: () => false,
  send: async () => ({}),
} as any

function extractCdpPort(config: EvalConfig): number {
  const serverUrl = config.browseros.server_url
  const match = serverUrl.match(/:(\d+)$/)
  if (!match) return config.browseros.base_cdp_port
  const serverPort = Number.parseInt(match[1], 10)
  const workerOffset = serverPort - config.browseros.base_server_port
  return config.browseros.base_cdp_port + workerOffset
}

export class SingleAgentEvaluator implements AgentEvaluator {
  constructor(private ctx: AgentContext) {}

  async execute(): Promise<AgentResult> {
    const { config, task, capture } = this.ctx
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

    // Connect to Chrome via CDP
    const cdpPort = extractCdpPort(config)
    const cdp = new CdpBackend({ port: cdpPort })
    await cdp.connect()

    const browser = new Browser(cdp, CONTROLLER_STUB)
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

    let agent: AiSdkAgent | null = null

    try {
      agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        browser,
        registry,
        browserContext,
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

            experimental_onToolCallStart: ({ toolCall }) => {
              const input = toolCall.input as
                | Record<string, unknown>
                | undefined
              if (input && typeof input.page === 'number') {
                capture.setActivePageId(input.page)
              }
            },

            experimental_onToolCallFinish: async () => {
              try {
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
                  const inputEvent = {
                    type: 'tool-input-available',
                    toolCallId: tc.toolCallId,
                    toolName: tc.toolName,
                    input: tc.input,
                  } as any
                  await capture.messageLogger.logStreamEvent(inputEvent)
                  capture.emitEvent(task.query_id, inputEvent)
                }
              }

              if (toolResults) {
                for (const tr of toolResults) {
                  const outputEvent = {
                    type: 'tool-output-available',
                    toolCallId: tr.toolCallId,
                    output: tr.output,
                  } as any
                  await capture.messageLogger.logStreamEvent(outputEvent)
                  capture.emitEvent(task.query_id, outputEvent)
                }
              }

              if (text) {
                const textId = randomUUID()
                const startEvent = { type: 'text-start', id: textId } as any
                const deltaEvent = {
                  type: 'text-delta',
                  id: textId,
                  delta: text,
                } as any
                const endEvent = { type: 'text-end', id: textId } as any
                await capture.messageLogger.logStreamEvent(startEvent)
                await capture.messageLogger.logStreamEvent(deltaEvent)
                await capture.messageLogger.logStreamEvent(endEvent)
                capture.emitEvent(task.query_id, deltaEvent)
              }
            },
          })

          finalText = result.text || null
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
