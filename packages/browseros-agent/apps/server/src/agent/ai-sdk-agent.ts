import { devToolsMiddleware } from '@ai-sdk/devtools'
import type {
  LanguageModelV3,
  LanguageModelV3Middleware,
} from '@ai-sdk/provider'
import { AGENT_LIMITS } from '@browseros/shared/constants/limits'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import {
  type LanguageModel,
  type ModelMessage,
  stepCountIs,
  ToolLoopAgent,
  type ToolSet,
  type UIMessage,
  wrapLanguageModel,
} from 'ai'
import type { KlavisService } from '../api/services/klavis'
import type { BrowserSession } from '../browser/core/session'
import { logger } from '../lib/logger'
import { metrics } from '../lib/metrics'
import {
  type BrowserOutputFileAccess,
  createBrowserOutputFileAccess,
} from '../tools/browser/output-file'
import { buildFilesystemToolSet } from '../tools/filesystem/build-toolset'
import { createReadTool } from '../tools/filesystem/read'
import { isAcpProvider } from './acp-providers'
import { CHAT_MODE_ALLOWED_TOOLS } from './chat-mode'
import { createCompactionPrepareStep, type StepWithUsage } from './compaction'
import { buildMcpServerSpecs, createMcpClients } from './mcp-builder'
import {
  getMessageNormalizationOptions,
  normalizeMessagesForModel,
} from './message-normalization'
import { buildNudgeToolSet } from './nudge-tools'
import { buildSystemPrompt } from './prompt'
import { createLanguageModel } from './provider-factory'
import { buildBrowserToolSet } from './tool-adapter'
import type { ResolvedAgentConfig } from './types'

export interface AiSdkAgentConfig {
  resolvedConfig: ResolvedAgentConfig
  browserSession: BrowserSession
  browserContext?: BrowserContext
  klavis?: KlavisService
  browserosId?: string
  aiSdkDevtoolsEnabled?: boolean
  outputFileAccess?: BrowserOutputFileAccess
}

/** Builds filesystem tools for model-backed sessions, with scoped readback outside full workspace mode. */
export function buildAgentFilesystemToolSet(
  resolvedConfig: ResolvedAgentConfig,
  options: { outputFileAccess?: BrowserOutputFileAccess } = {},
): ToolSet {
  if (isAcpProvider(resolvedConfig.provider)) {
    return {}
  }
  if (resolvedConfig.chatMode || !resolvedConfig.workingDir) {
    return {
      filesystem_read: createReadTool(undefined, {
        allowedOutputPaths: options.outputFileAccess?.paths,
      }),
    }
  }
  return buildFilesystemToolSet(resolvedConfig.workingDir)
}

export class AiSdkAgent {
  private constructor(
    private _agent: ToolLoopAgent,
    private _messages: UIMessage[],
    private _mcpClients: Array<{ close(): Promise<void> }>,
    private conversationId: string,
    private _toolNames: Set<string>,
    /**
     * ACP-provider teardown. Closes the spawned agent process and its
     * persistent session record. Undefined for model-backed providers,
     * where the LanguageModel owns no host-side state.
     */
    private _modelClose?: () => Promise<void>,
  ) {}

  /** Tool names registered on this agent — used to sanitize messages during session rebuilds. */
  get toolNames(): Set<string> {
    return this._toolNames
  }

  static async create(config: AiSdkAgentConfig): Promise<AiSdkAgent> {
    const contextWindow =
      config.resolvedConfig.contextWindowSize ??
      AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW

    const { model: rawModel, close: modelClose } = await createLanguageModel(
      config.resolvedConfig,
    )
    const isV3Model =
      typeof rawModel === 'object' &&
      rawModel !== null &&
      'specificationVersion' in rawModel &&
      rawModel.specificationVersion === 'v3'

    let model = rawModel
    if (isV3Model && config.aiSdkDevtoolsEnabled) {
      model = wrapLanguageModel({
        model: rawModel as LanguageModelV3,
        middleware: devToolsMiddleware() as LanguageModelV3Middleware,
      })
      logger.info('AI SDK DevTools middleware enabled', {
        conversationId: config.resolvedConfig.conversationId,
        provider: config.resolvedConfig.provider,
        model: config.resolvedConfig.model,
      })
    }

    // ACP-backed providers (Claude Code, Codex, custom ACP) reach tools
    // exclusively through the MCP boundary acpx-ai-provider sets up; the
    // ai-sdk `tools` argument never crosses the ACP wire. Skip every
    // tool-set builder and every server-side MCP client connection on
    // this branch. The spawned agent dials BrowserOS's own /mcp route
    // (and any user-configured MCP servers) directly via the
    // mcpServers config on ResolvedAgentConfig.
    const useMcpBoundaryOnly = isAcpProvider(config.resolvedConfig.provider)
    const outputFileAccess =
      config.outputFileAccess ?? createBrowserOutputFileAccess()

    const allBrowserTools = useMcpBoundaryOnly
      ? {}
      : buildBrowserToolSet(config.browserSession, {
          readOnly: config.resolvedConfig.chatMode,
          outputFileAccess,
        })
    const reservedBrowserToolNames = new Set(Object.keys(allBrowserTools))
    const chatModeAllowedTools = CHAT_MODE_ALLOWED_TOOLS
    const browserTools = config.resolvedConfig.chatMode
      ? Object.fromEntries(
          Object.entries(allBrowserTools).filter(([name]) =>
            chatModeAllowedTools.has(name),
          ),
        )
      : allBrowserTools
    if (config.resolvedConfig.chatMode && !useMcpBoundaryOnly) {
      logger.info('Chat mode enabled, restricting to read-only browser tools', {
        allowedTools: Array.from(chatModeAllowedTools),
      })
    }

    const klavisTools =
      !useMcpBoundaryOnly && config.klavis
        ? config.klavis.buildAiSdkToolSet({
            selectedServerNames: config.browserContext?.enabledMcpServers,
          })
        : {}

    // Connect custom (non-Klavis) MCP servers per-session
    const specs = useMcpBoundaryOnly
      ? []
      : await buildMcpServerSpecs({
          browserContext: config.browserContext,
        })
    const { clients, tools: customMcpTools } = await createMcpClients(specs)
    const klavisCollidingToolNames = Object.keys(customMcpTools).filter(
      (name) => name in klavisTools,
    )
    if (klavisCollidingToolNames.length > 0) {
      logger.warn('Custom MCP tools override Klavis tools', {
        toolNames: klavisCollidingToolNames,
      })
    }
    const rawExternalMcpTools = withoutReservedBrowserToolNames(
      { ...klavisTools, ...customMcpTools },
      reservedBrowserToolNames,
    )

    // Wrap external MCP tools (Klavis, custom) with metrics
    const externalMcpTools: ToolSet = {}
    for (const [name, t] of Object.entries(rawExternalMcpTools)) {
      const originalExecute = t.execute
      externalMcpTools[name] = {
        ...t,
        execute: originalExecute
          ? async (
              ...args: Parameters<NonNullable<typeof originalExecute>>
            ) => {
              const startTime = performance.now()
              try {
                const result = await originalExecute(...args)
                metrics.log('tool_executed', {
                  tool_name: name,
                  duration_ms: Math.round(performance.now() - startTime),
                  success: true,
                  source: 'chat',
                })
                return result
              } catch (error) {
                metrics.log('tool_executed', {
                  tool_name: name,
                  duration_ms: Math.round(performance.now() - startTime),
                  success: false,
                  error_message:
                    error instanceof Error ? error.message : String(error),
                  source: 'chat',
                })
                throw error
              }
            }
          : undefined,
      }
    }

    // ACP providers skip AI SDK filesystem tools. Chat and no-workspace sessions
    // get only output-file reads for browser-generated files.
    const filesystemTools = buildAgentFilesystemToolSet(config.resolvedConfig, {
      outputFileAccess,
    })
    const workspaceDirForPrompt =
      !config.resolvedConfig.chatMode && 'filesystem_write' in filesystemTools
        ? config.resolvedConfig.workingDir
        : undefined
    const tools = {
      ...browserTools,
      ...externalMcpTools,
      ...filesystemTools,
      ...buildNudgeToolSet(),
    }

    if (
      config.resolvedConfig.isScheduledTask ||
      config.resolvedConfig.chatMode
    ) {
      delete tools.suggest_schedule
      delete tools.suggest_app_connection
    }

    // Build system prompt with optional section exclusions
    const excludeSections: string[] = []
    if (
      config.resolvedConfig.isScheduledTask ||
      config.resolvedConfig.chatMode
    ) {
      excludeSections.push('nudges')
    }
    const instructions = buildSystemPrompt({
      userSystemPrompt: config.resolvedConfig.userSystemPrompt,
      exclude: excludeSections,
      isScheduledTask: config.resolvedConfig.isScheduledTask,
      scheduledTaskPageId: config.browserContext?.activeTab?.pageId,
      workspaceDir: workspaceDirForPrompt,
      chatMode: config.resolvedConfig.chatMode,
      connectedApps: config.browserContext?.enabledMcpServers,
      declinedApps: config.resolvedConfig.declinedApps,
      origin: config.resolvedConfig.origin,
      generatedOutputReadAvailable: 'filesystem_read' in filesystemTools,
    })

    // Configure compaction for context window management
    const compactionPrepareStep = createCompactionPrepareStep({
      contextWindow,
    })
    const normalizationOptions = getMessageNormalizationOptions(
      config.resolvedConfig,
    )
    const prepareStep = async (options: {
      messages: ModelMessage[]
      steps: ReadonlyArray<StepWithUsage>
      model: LanguageModel
      experimental_context: unknown
    }) =>
      compactionPrepareStep({
        ...options,
        messages: normalizeMessagesForModel(
          options.messages,
          normalizationOptions,
        ),
      })

    // Codex requires store=false — tell the SDK to inline content
    // instead of using item_reference (which fails with store=false)
    const isChatGPTPro =
      config.resolvedConfig.provider === LLM_PROVIDERS.CHATGPT_PRO

    const agent = new ToolLoopAgent({
      model,
      instructions,
      tools,
      stopWhen: [stepCountIs(AGENT_LIMITS.MAX_TURNS)],
      prepareStep,
      ...(isChatGPTPro && {
        providerOptions: {
          openai: {
            store: false,
            reasoningEffort: config.resolvedConfig.reasoningEffort || 'medium',
            reasoningSummary: config.resolvedConfig.reasoningSummary || 'auto',
            include: ['reasoning.encrypted_content'],
          },
        },
      }),
    })

    logger.info('Agent session created (v2)', {
      conversationId: config.resolvedConfig.conversationId,
      provider: config.resolvedConfig.provider,
      model: config.resolvedConfig.model,
      toolCount: Object.keys(tools).length,
    })

    return new AiSdkAgent(
      agent,
      [],
      clients,
      config.resolvedConfig.conversationId,
      new Set(Object.keys(tools)),
      modelClose,
    )
  }

  get toolLoopAgent(): ToolLoopAgent {
    return this._agent
  }

  get messages(): UIMessage[] {
    return this._messages
  }

  set messages(msgs: UIMessage[]) {
    this._messages = msgs
  }

  appendUserMessage(content: string): void {
    this._messages.push({
      id: crypto.randomUUID(),
      role: 'user',
      parts: [{ type: 'text', text: content }],
    })
  }

  async dispose(): Promise<void> {
    for (const client of this._mcpClients) {
      await client.close().catch(() => {})
    }
    if (this._modelClose) {
      await this._modelClose().catch((error: unknown) => {
        logger.warn('LanguageModel close hook failed', {
          conversationId: this.conversationId,
          error: error instanceof Error ? error.message : String(error),
        })
      })
    }
    logger.info('Agent disposed', { conversationId: this.conversationId })
  }
}

function withoutReservedBrowserToolNames(
  tools: ToolSet,
  reservedNames: Set<string>,
): ToolSet {
  const result: ToolSet = {}
  const skipped: string[] = []
  for (const [name, value] of Object.entries(tools)) {
    if (reservedNames.has(name)) {
      skipped.push(name)
      continue
    }
    result[name] = value
  }
  if (skipped.length > 0) {
    logger.warn(
      'External MCP tools skipped due to BrowserOS tool name collision',
      {
        toolNames: skipped,
      },
    )
  }
  return result
}

export { formatUserMessage } from './format-message'
