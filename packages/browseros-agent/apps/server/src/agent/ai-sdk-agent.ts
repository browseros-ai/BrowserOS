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
  type UIMessage,
  wrapLanguageModel,
} from 'ai'
import type { Browser } from '../browser/browser'
import type { KlavisClient } from '../lib/clients/klavis/klavis-client'
import { logger } from '../lib/logger'
import { isSoulBootstrap, readSoul } from '../lib/soul'
import { buildSkillsCatalog } from '../skills/catalog'
import { loadSkills } from '../skills/loader'
import { buildFilesystemToolSet } from '../tools/filesystem/build-toolset'
import { buildMemoryToolSet } from '../tools/memory/build-toolset'
import type { ToolRegistry } from '../tools/tool-registry'
import { CHAT_MODE_ALLOWED_TOOLS } from './chat-mode'
import { createCompactionPrepareStep, type StepWithUsage } from './compaction'
import { createContextOverflowMiddleware } from './context-overflow-middleware'
import { buildMcpServerSpecs, createMcpClients } from './mcp-builder'
import {
  getMessageNormalizationOptions,
  normalizeMessagesForModel,
} from './message-normalization'
import { buildSystemPrompt } from './prompt'
import { createLanguageModel } from './provider-factory'
import { buildBrowserToolSet } from './tool-adapter'
import type { ResolvedAgentConfig } from './types'

export interface AiSdkAgentConfig {
  resolvedConfig: ResolvedAgentConfig
  browser: Browser
  registry: ToolRegistry
  browserContext?: BrowserContext
  klavisClient?: KlavisClient
  browserosId?: string
  aiSdkDevtoolsEnabled?: boolean
}

export class AiSdkAgent {
  private constructor(
    private _agent: ToolLoopAgent,
    private _messages: UIMessage[],
    private _mcpClients: Array<{ close(): Promise<void> }>,
    private conversationId: string,
  ) {}

  static async create(config: AiSdkAgentConfig): Promise<AiSdkAgent> {
    const contextWindow =
      config.resolvedConfig.contextWindowSize ??
      AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW

    // Build language model with middleware stack
    const rawModel = createLanguageModel(config.resolvedConfig)
    const isV3Model =
      typeof rawModel === 'object' &&
      rawModel !== null &&
      'specificationVersion' in rawModel &&
      rawModel.specificationVersion === 'v3'

    let model = rawModel
    if (isV3Model) {
      // Always apply context overflow protection
      model = wrapLanguageModel({
        model: rawModel as LanguageModelV3,
        middleware: createContextOverflowMiddleware(contextWindow),
      })

      // Optionally add AI SDK DevTools tracing (dev-only)
      if (config.aiSdkDevtoolsEnabled) {
        model = wrapLanguageModel({
          model: model as LanguageModelV3,
          middleware: devToolsMiddleware() as LanguageModelV3Middleware,
        })
        logger.info('AI SDK DevTools middleware enabled', {
          conversationId: config.resolvedConfig.conversationId,
          provider: config.resolvedConfig.provider,
          model: config.resolvedConfig.model,
        })
      }
    }

    // Build browser tools from the unified tool registry
    const allBrowserTools = buildBrowserToolSet(
      config.registry,
      config.browser,
      config.resolvedConfig.workingDir,
    )
    const browserTools = config.resolvedConfig.chatMode
      ? Object.fromEntries(
          Object.entries(allBrowserTools).filter(([name]) =>
            CHAT_MODE_ALLOWED_TOOLS.has(name),
          ),
        )
      : allBrowserTools
    if (config.resolvedConfig.chatMode) {
      logger.info('Chat mode enabled, restricting to read-only browser tools', {
        allowedTools: Array.from(CHAT_MODE_ALLOWED_TOOLS),
      })
    }

    // Build external MCP server specs (Klavis, custom) and connect clients
    const specs = await buildMcpServerSpecs({
      browserContext: config.browserContext,
      klavisClient: config.klavisClient,
      browserosId: config.browserosId,
    })
    const { clients, tools: externalMcpTools } = await createMcpClients(specs)

    // Add filesystem tools (Pi coding agent) — skip in chat mode (read-only)
    const filesystemTools = config.resolvedConfig.chatMode
      ? {}
      : buildFilesystemToolSet(config.resolvedConfig.workingDir)
    const memoryTools = config.resolvedConfig.chatMode
      ? {}
      : buildMemoryToolSet()
    const tools = {
      ...browserTools,
      ...externalMcpTools,
      ...filesystemTools,
      ...memoryTools,
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
    const soulContent = await readSoul()
    const isBootstrap = await isSoulBootstrap()

    // Load skills catalog for prompt injection
    const skills = await loadSkills()
    const skillsCatalog =
      skills.length > 0 ? buildSkillsCatalog(skills) : undefined

    const instructions = buildSystemPrompt({
      userSystemPrompt: config.resolvedConfig.userSystemPrompt,
      exclude: excludeSections,
      isScheduledTask: config.resolvedConfig.isScheduledTask,
      scheduledTaskWindowId: config.browserContext?.windowId,
      workspaceDir: config.resolvedConfig.workingDir,
      soulContent,
      isSoulBootstrap: isBootstrap,
      chatMode: config.resolvedConfig.chatMode,
      connectedApps: config.browserContext?.enabledMcpServers,
      declinedApps: config.resolvedConfig.declinedApps,
      skillsCatalog,
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
            reasoningEffort: config.resolvedConfig.reasoningEffort || 'high',
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
    logger.info('Agent disposed', { conversationId: this.conversationId })
  }
}

export { formatUserMessage } from './format-message'
