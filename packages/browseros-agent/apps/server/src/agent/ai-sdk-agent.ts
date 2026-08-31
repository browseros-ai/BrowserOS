import { devToolsMiddleware } from '@ai-sdk/devtools'
import type {
  LanguageModelV4,
  LanguageModelV4Middleware,
} from '@ai-sdk/provider'
import {
  type BrowserOutputFileAccess,
  createBrowserOutputFileAccess,
} from '@browseros/browser-mcp/output-file'
import { AGENT_LIMITS } from '@browseros/shared/constants/limits'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import {
  type LanguageModel,
  type ModelMessage,
  stepCountIs,
  ToolLoopAgent,
  type ToolSet,
  type UIMessage,
  wrapLanguageModel,
} from 'ai'
import { logger } from '../lib/logger'
import { metrics } from '../lib/metrics'
import { buildFilesystemToolSet } from '../tools/filesystem/build-toolset'
import { createReadTool } from '../tools/filesystem/read'
import { createCompactionPrepareStep, type StepWithUsage } from './compaction'
import {
  buildInternalBrowserMcpSpec,
  buildMcpServerSpecs,
  createMcpClients,
} from './mcp-builder'
import {
  getMessageNormalizationOptions,
  normalizeMessagesForModel,
} from './message-normalization'
import { buildNudgeToolSet } from './nudge-tools'
import { buildSystemPrompt } from './prompt'
import { createLanguageModel } from './provider-factory'
import { buildAgentReasoningConfig } from './reasoning-config'
import type { ResolvedAgentConfig } from './types'

export interface AiSdkAgentConfig {
  resolvedConfig: ResolvedAgentConfig
  serverPort: number
  browserToolLeaseToken: string
  browserContext?: BrowserContext
  browserosId?: string
  aiSdkDevtoolsEnabled?: boolean
  outputFileAccess?: BrowserOutputFileAccess
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function summarizeToolInput(input: unknown): Record<string, unknown> {
  if (!isRecord(input)) {
    return { inputType: typeof input }
  }
  const summary: Record<string, unknown> = {
    argKeys: Object.keys(input).sort(),
  }
  if (typeof input.server_name === 'string') {
    summary.serverName = input.server_name
  }
  if (typeof input.path === 'string') {
    summary.path = input.path
  }
  if (typeof input.page === 'number') {
    summary.page = input.page
  }
  if (typeof input.action === 'string') {
    summary.action = input.action
  }
  return summary
}

function toolResultIsError(result: unknown): boolean {
  return isRecord(result) && result.isError === true
}

function summarizeToolResultError(
  result: unknown,
): Record<string, unknown> | undefined {
  if (!isRecord(result) || !Array.isArray(result.content)) {
    return undefined
  }
  const textBlocks = result.content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        typeof item === 'object' &&
        item !== null &&
        'type' in item &&
        item.type === 'text' &&
        'text' in item &&
        typeof item.text === 'string',
    )
    .map((item) => item.text)
  const text = textBlocks.join('\n')
  return {
    contentCount: result.content.length,
    textBlockCount: textBlocks.length,
    textLength: text.length,
    lineCount: text.length ? text.split('\n').length : 0,
  }
}

/** Builds filesystem tools for model-backed sessions, with scoped readback outside full workspace mode. */
export function buildAgentFilesystemToolSet(
  resolvedConfig: ResolvedAgentConfig,
  options: { outputFileAccess?: BrowserOutputFileAccess } = {},
): ToolSet {
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
  ) {}

  /** Tool names registered on this agent — used to sanitize messages during session rebuilds. */
  get toolNames(): Set<string> {
    return this._toolNames
  }

  static async create(config: AiSdkAgentConfig): Promise<AiSdkAgent> {
    const contextWindow =
      config.resolvedConfig.contextWindowSize ??
      AGENT_LIMITS.DEFAULT_CONTEXT_WINDOW

    const { model: rawModel } = await createLanguageModel(config.resolvedConfig)
    const isV4Model =
      typeof rawModel === 'object' &&
      rawModel !== null &&
      'specificationVersion' in rawModel &&
      rawModel.specificationVersion === 'v4'

    let model = rawModel
    if (isV4Model && config.aiSdkDevtoolsEnabled) {
      model = wrapLanguageModel({
        model: rawModel as LanguageModelV4,
        middleware: devToolsMiddleware() as LanguageModelV4Middleware,
      })
      logger.info('AI SDK DevTools middleware enabled', {
        conversationId: config.resolvedConfig.conversationId,
        provider: config.resolvedConfig.provider,
        model: config.resolvedConfig.model,
      })
    }

    const outputFileAccess =
      config.outputFileAccess ?? createBrowserOutputFileAccess()

    const connectedMcpClients: Array<{ close(): Promise<void> }> = []
    let clientsTransferred = false
    try {
      // BrowserOS tools and managed connectors use the same required loopback
      // MCP path as ACP/external clients. This keeps guards, effects, metrics,
      // and generated-output grants behind one server-owned seam.
      const internalSpec = buildInternalBrowserMcpSpec({
        serverPort: config.serverPort,
        leaseToken: config.browserToolLeaseToken,
        readOnly: config.resolvedConfig.chatMode ?? false,
      })
      const internalMcp = await createMcpClients([internalSpec])
      connectedMcpClients.push(...internalMcp.clients)
      const reservedInternalToolNames = new Set(Object.keys(internalMcp.tools))

      const customSpecs = await buildMcpServerSpecs({
        browserContext: config.browserContext,
      })
      const customMcp = await createMcpClients(customSpecs)
      connectedMcpClients.push(...customMcp.clients)
      const rawExternalMcpTools = withoutReservedInternalToolNames(
        customMcp.tools,
        reservedInternalToolNames,
      )

      // Custom MCP servers remain outside BrowserMcpModule, so their metrics
      // stay at this adapter boundary. Managed connectors arrive through the
      // internal MCP client and are already instrumented server-side.
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
                const logBase = {
                  toolName: name,
                  source: 'chat',
                  conversationId: config.resolvedConfig.conversationId,
                  provider: config.resolvedConfig.provider,
                }
                logger.debug('External MCP chat tool started', {
                  ...logBase,
                  args: summarizeToolInput(args[0]),
                })
                try {
                  const result = await originalExecute(...args)
                  const durationMs = Math.round(performance.now() - startTime)
                  const isError = toolResultIsError(result)
                  metrics.log('tool_executed', {
                    tool_name: name,
                    duration_ms: durationMs,
                    success: !isError,
                    source: 'chat',
                  })
                  logger.debug('External MCP chat tool completed', {
                    ...logBase,
                    durationMs,
                    isError,
                  })
                  if (isError) {
                    logger.info('External MCP chat tool returned error', {
                      ...logBase,
                      durationMs,
                      errorSummary: summarizeToolResultError(result),
                    })
                  }
                  return result
                } catch (error) {
                  const errorText =
                    error instanceof Error ? error.message : String(error)
                  const durationMs = Math.round(performance.now() - startTime)
                  metrics.log('tool_executed', {
                    tool_name: name,
                    duration_ms: durationMs,
                    success: false,
                    error_message: errorText,
                    source: 'chat',
                  })
                  logger.info('External MCP chat tool threw', {
                    ...logBase,
                    durationMs,
                    error: errorText,
                  })
                  throw error
                }
              }
            : undefined,
        }
      }

      const filesystemTools = buildAgentFilesystemToolSet(
        config.resolvedConfig,
        {
          outputFileAccess,
        },
      )
      const workspaceDirForPrompt =
        !config.resolvedConfig.chatMode && 'filesystem_write' in filesystemTools
          ? config.resolvedConfig.workingDir
          : undefined
      const tools = {
        ...internalMcp.tools,
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
        runtimeContext: unknown
      }) =>
        compactionPrepareStep({
          ...options,
          messages: normalizeMessagesForModel(
            options.messages,
            normalizationOptions,
          ),
        })

      const agent = new ToolLoopAgent({
        model,
        instructions,
        tools,
        stopWhen: [stepCountIs(AGENT_LIMITS.MAX_TURNS)],
        prepareStep,
        ...buildAgentReasoningConfig(config.resolvedConfig),
      })

      logger.info('Agent session created (v2)', {
        conversationId: config.resolvedConfig.conversationId,
        provider: config.resolvedConfig.provider,
        model: config.resolvedConfig.model,
        toolCount: Object.keys(tools).length,
      })

      const result = new AiSdkAgent(
        agent,
        [],
        connectedMcpClients,
        config.resolvedConfig.conversationId,
        new Set(Object.keys(tools)),
      )
      clientsTransferred = true
      return result
    } finally {
      // Before the AiSdkAgent is returned, this factory owns every connected
      // transport. Close them if prompt/tool construction fails midway.
      if (!clientsTransferred) {
        await Promise.allSettled(
          connectedMcpClients.map((client) => client.close()),
        )
      }
    }
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

function withoutReservedInternalToolNames(
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
