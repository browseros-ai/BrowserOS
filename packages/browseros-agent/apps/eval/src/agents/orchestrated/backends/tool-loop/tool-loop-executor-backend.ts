import { randomUUID } from 'node:crypto'
import type { Browser } from '@browseros/browser-core/browser'
import { AiSdkAgent } from '@browseros/server/agent/tool-loop'
import type { ResolvedAgentConfig } from '@browseros/server/agent/types'
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
        browserSession,
        browserContext,
      })

      await agent.toolLoopAgent.generate({
        prompt: instruction,
        abortSignal: signal,

        onStepFinish: async ({
          toolCalls,
          toolResults,
          text,
        }: {
          // ai-sdk option-type widening under this branch's mixed-zod
          // workspace (cockpit pins zod v4, server pins v3) drops the
          // destructure to implicit-any. The explicit `any` keeps the
          // call compiling; the runtime contract is unchanged.
          // biome-ignore lint/suspicious/noExplicitAny: see comment above
          toolCalls?: any
          // biome-ignore lint/suspicious/noExplicitAny: see comment above
          toolResults?: any
          text?: string
        }) => {
          // Pre-v6.0.208 split this into experimental_onToolCallStart and
          // experimental_onToolCallFinish; ToolLoopAgent no longer exposes
          // those per-call hooks. Replay both lifecycle callbacks here so
          // outer observers still see per-tool-call events, and update
          // step-level state once per tool call within the step.
          if (toolCalls) {
            for (const toolCall of toolCalls) {
              const input = toolCall.input as
                | Record<string, unknown>
                | undefined
              if (
                input &&
                typeof input.url === 'string' &&
                input.url.length > 0
              ) {
                this.currentUrl = input.url
              }
              this.options.callbacks?.onToolCallStart?.({
                toolCallId: toolCall.toolCallId,
                toolName: toolCall.toolName,
                input: toolCall.input,
              })

              this.stepsUsed++
              await this.options.callbacks?.onToolCallFinish?.()

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
