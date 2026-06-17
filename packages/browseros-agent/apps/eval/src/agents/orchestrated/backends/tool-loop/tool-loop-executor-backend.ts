import { randomUUID } from 'node:crypto'
import { AiSdkAgent } from '@browseros/server/agent/tool-loop'
import type { ResolvedAgentConfig } from '@browseros/server/agent/types'
import type { Browser } from '@browseros/server/browser'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import type {
  DelegationResult,
  ExecutorBackend,
  ExecutorCallbacks,
} from '../../executor-backend'
import { TOOL_LOOP_EXECUTOR_SYSTEM_PROMPT } from './tool-loop-executor-prompt'

export interface ToolLoopExecutorBackendOptions {
  configTemplate: ResolvedAgentConfig
  browser: Browser | null
  callbacks?: ExecutorCallbacks
}

/** Executes delegated goals through the BrowserOS ToolLoopAgent. */
export class ToolLoopExecutorBackend implements ExecutorBackend {
  readonly kind = 'tool-loop'
  private stepsUsed = 0
  private currentUrl = ''

  constructor(private readonly options: ToolLoopExecutorBackendOptions) {}

  async execute(
    instruction: string,
    signal?: AbortSignal,
  ): Promise<DelegationResult> {
    const browser = this.options.browser
    if (!browser) {
      throw new Error('Browser instance is required for tool-loop executor')
    }
    const browserSession = browser.session

    const stepsAtStart = this.stepsUsed
    const toolsUsed: string[] = []
    let status: DelegationResult['status'] = 'done'
    let resultText = ''

    const conversationId = randomUUID()
    const agentConfig: ResolvedAgentConfig = {
      ...this.options.configTemplate,
      conversationId,
      userSystemPrompt: TOOL_LOOP_EXECUTOR_SYSTEM_PROMPT,
      evalMode: true,
      workingDir: `/tmp/browseros-eval-executor-${conversationId}`,
    }

    const browserContext = await this.browserContext(browser)
    let agent: AiSdkAgent | null = null

    try {
      agent = await AiSdkAgent.create({
        resolvedConfig: agentConfig,
        browser,
        browserSession,
        browserContext,
        browserUseNewTools: true,
      })

      // biome-ignore lint/suspicious/noExplicitAny: see comment below.
      // `as any` keeps the call compiling when the workspace's zod
      // major versions cross-pollute the ai-sdk's `generate()`
      // option type (cockpit pins zod v4, server pins v3). The
      // `experimental_on*` callbacks ARE valid on the runtime
      // ai-sdk; only TS's view of them gets dropped under the
      // mixed-version resolution. Tracked alongside the cockpit's
      // dynamic-import workaround in apps/server/src/api/server.ts.
      await agent.toolLoopAgent.generate({
        prompt: instruction,
        abortSignal: signal,

        // biome-ignore lint/suspicious/noExplicitAny: see toolCall typing note in the function header above.
        experimental_onToolCallStart: ({ toolCall }: { toolCall: any }) => {
          const input = toolCall.input as Record<string, unknown> | undefined
          if (input && typeof input.url === 'string' && input.url.length > 0) {
            this.currentUrl = input.url
          }
          this.options.callbacks?.onToolCallStart?.({
            toolCallId: toolCall.toolCallId,
            toolName: toolCall.toolName,
            input: toolCall.input,
          })
        },

        experimental_onToolCallFinish: async () => {
          this.stepsUsed++
          await this.options.callbacks?.onToolCallFinish?.()
        },

        // biome-ignore lint/suspicious/noExplicitAny: cascade from the call-site cast; ai-sdk's step type widens to `any` once the literal is cast.
        onStepFinish: async ({
          toolCalls,
          toolResults,
          text,
        }: {
          toolCalls?: any
          toolResults?: any
          text?: string
        }) => {
          if (toolCalls) {
            for (const toolCall of toolCalls) {
              if (!toolsUsed.includes(toolCall.toolName)) {
                toolsUsed.push(toolCall.toolName)
              }
            }
          }

          if (text) resultText = text

          await this.options.callbacks?.onStepFinish?.({
            toolCalls,
            toolResults,
            text,
          })
        },
        // biome-ignore lint/suspicious/noExplicitAny: ai-sdk option-type widening under mixed workspace zod versions; see top-of-call comment.
      } as any)
    } catch {
      status = signal?.aborted ? 'timeout' : 'blocked'
    } finally {
      if (agent) await agent.dispose().catch(() => {})
    }

    if (status === 'done' && signal?.aborted) {
      status = 'timeout'
    }

    return {
      observation: resultText || 'Execution completed with no actions taken.',
      status,
      url: this.currentUrl,
      actionsPerformed: this.stepsUsed - stepsAtStart,
      toolsUsed,
    }
  }

  async close(): Promise<void> {
    // No persistent resources; AiSdkAgent is disposed at the end of each execute() call.
  }

  getTotalSteps(): number {
    return this.stepsUsed
  }

  private async browserContext(
    browser: Browser,
  ): Promise<BrowserContext | undefined> {
    const pages = await browser.listPages()
    const activePage = pages[0]
    if (!activePage) return undefined

    return {
      activeTab: {
        id: activePage.tabId,
        pageId: activePage.pageId,
        url: activePage.url,
        title: activePage.title,
      },
    }
  }
}
