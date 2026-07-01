/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { join } from 'node:path'
import { DEFAULT_PORTS } from '@browseros/shared/constants/ports'
import {
  type AcpRuntimeEvent,
  type AcpRuntimeHandle,
  type AcpRuntimeOptions,
  type AcpRuntimeTurn,
  type AcpRuntimeTurnResult,
  type AcpSessionRecord,
  type AcpRuntime as AcpxCoreRuntime,
  createAcpRuntime,
  createAgentRegistry,
  createRuntimeStore,
} from 'acpx/runtime'
import { getBrowserosDir } from '../../browseros-dir'
import { logger } from '../../logger'
import {
  type AgentDefinition,
  type AgentHistoryEntry,
  type AgentHistoryToolCall,
  type AgentSessionId,
  MAIN_AGENT_SESSION_ID,
} from '../agent-types'
import {
  resolveBundledBun,
  withBundledBunAcpAdapterEnv,
} from '../host-acp/bundled-bun'
import { withBundledNativeBinaryPath } from '../host-acp/bundled-native-binary'
import {
  DANGEROUS_ALLOW_MODE_CANDIDATES,
  HOST_ACP_ADAPTER_CONFIG,
} from '../host-acp/config'
import type {
  AgentHistoryPage,
  AgentPromptInput,
  AgentRowSnapshot,
  AgentRuntime,
  AgentSession,
  AgentStatus,
  AgentStreamEvent,
} from '../types'
import { prepareAcpxAgentContext } from './agent-adapter'
import {
  resolveAgentRuntimePaths,
  shellQuote,
  wrapCommandWithEnv,
} from './runtime-context'
import {
  type LatestRuntimeState,
  loadLatestRuntimeState,
} from './runtime-state'

export type AcpxRuntimeOptions = {
  cwd?: string
  browserosDir?: string
  resourcesDir?: string
  stateDir?: string
  browserosServerPort?: number
  runtimeFactory?: (options: AcpRuntimeOptions) => AcpxCoreRuntime
}

interface PreparedRuntimeContext {
  cwd: string
  runtimeSessionKey: string
  runPrompt: string
  agentCommandEnv: Record<string, string>
  commandIdentity: string
  useBrowserosMcp: boolean
  browserosMcpHost?: string
}

export class AcpxRuntime implements AgentRuntime {
  private readonly defaultCwd: string | null
  private readonly browserosDir: string
  private readonly resourcesDir: string | null
  private readonly stateDir: string
  private readonly browserosServerPort: number
  private readonly runtimeFactory: (
    options: AcpRuntimeOptions,
  ) => AcpxCoreRuntime
  private readonly sessionStore: ReturnType<typeof createRuntimeStore>
  private readonly runtimes = new Map<string, AcpxCoreRuntime>()

  constructor(options: AcpxRuntimeOptions = {}) {
    this.defaultCwd = options.cwd ?? null
    this.browserosDir = options.browserosDir ?? getBrowserosDir()
    this.resourcesDir = options.resourcesDir ?? null
    this.stateDir =
      options.stateDir ??
      process.env.BROWSEROS_ACPX_STATE_DIR ??
      join(this.browserosDir, 'agents', 'acpx')
    this.browserosServerPort =
      options.browserosServerPort ?? DEFAULT_PORTS.server
    this.sessionStore = createRuntimeStore({ stateDir: this.stateDir })
    this.runtimeFactory = options.runtimeFactory ?? createAcpRuntime
  }

  async status(): Promise<AgentStatus> {
    return { state: 'unknown', message: 'acpx status is checked on send' }
  }

  async listSessions(
    input: AgentPromptInput['agent'],
  ): Promise<AgentSession[]> {
    return [
      {
        agentId: input.id,
        id: MAIN_AGENT_SESSION_ID,
        updatedAt: input.updatedAt,
      },
    ]
  }

  async getHistory(input: {
    agent: AgentPromptInput['agent']
    sessionId: AgentSessionId
  }): Promise<AgentHistoryPage> {
    const record = await this.loadSessionRecord(input.agent, input.sessionId)
    if (!record) {
      return { agentId: input.agent.id, sessionId: input.sessionId, items: [] }
    }
    return mapAcpxSessionRecordToHistory(input.agent, input.sessionId, record)
  }

  /**
   * Lightweight read of the session record's row-level fields. Returns
   * `null` for never-used agents so the harness can fill in nulls
   * without throwing. Token bucketing for `last7d` lives outside the
   * session record (no per-message timestamps); a follow-up activity
   * ledger will populate that field — for now we return zeros.
   */
  async getRowSnapshot(input: {
    agent: AgentPromptInput['agent']
    sessionId: AgentSessionId
  }): Promise<AgentRowSnapshot | null> {
    const record = await this.loadSessionRecord(input.agent, input.sessionId)
    if (!record) return null
    return this.createRowSnapshot(record, input.sessionId)
  }

  async getLatestRowSnapshot(
    agent: AgentPromptInput['agent'],
  ): Promise<AgentRowSnapshot | null> {
    const latest = await this.loadLatestAgentSessionRecord(agent)
    if (!latest) return null
    return this.createRowSnapshot(latest.record, latest.sessionId)
  }

  private createRowSnapshot(
    record: AcpSessionRecord,
    sessionId: AgentSessionId,
  ): AgentRowSnapshot {
    return {
      sessionId,
      cwd: record.cwd ?? null,
      lastUsedAt: parseRecordTimestamp(record) || null,
      lastUserMessage: extractLastUserMessage(record),
      tokens: {
        cumulative: {
          input: record.cumulative_token_usage?.input_tokens ?? 0,
          output: record.cumulative_token_usage?.output_tokens ?? 0,
        },
        last7d: { input: 0, output: 0, requestCount: 0 },
      },
    }
  }

  async send(
    input: AgentPromptInput,
  ): Promise<ReadableStream<AgentStreamEvent>> {
    const prepared = await this.prepareRuntimeContext(
      input,
      input.cwd ?? this.defaultCwd,
    )
    const cwd = prepared.cwd
    const imageAttachments = (input.attachments ?? []).filter((a) =>
      a.mediaType.startsWith('image/'),
    )
    logger.info('Agent harness acpx send requested', {
      agentId: input.agent.id,
      adapter: input.agent.adapter,
      sessionId: input.sessionId,
      sessionKey: input.sessionKey,
      cwd,
      stateDir: this.stateDir,
      permissionMode: input.permissionMode,
      modelId: input.agent.modelId,
      reasoningEffort: input.agent.reasoningEffort,
      messageLength: input.message.length,
      imageAttachmentCount: imageAttachments.length,
    })

    const runtime = this.getRuntime({
      cwd,
      permissionMode: input.permissionMode,
      nonInteractivePermissions: 'fail',
      commandEnv: prepared.agentCommandEnv,
      commandIdentity: prepared.commandIdentity,
      useBrowserosMcp: prepared.useBrowserosMcp,
      browserosMcpHost: prepared.browserosMcpHost,
      agentId: input.agent.id,
      sessionId: input.sessionId,
    })

    return createAcpxEventStream(runtime, input, {
      cwd,
      runtimeSessionKey: prepared.runtimeSessionKey,
      runPrompt: prepared.runPrompt,
    })
  }

  private async loadSessionRecord(
    agent: AgentPromptInput['agent'],
    sessionId: AgentSessionId,
    latestForAgentHint?: LatestRuntimeState | null,
  ): Promise<AcpSessionRecord | null> {
    const paths = resolveAgentRuntimePaths({
      browserosDir: this.browserosDir,
      agentId: agent.id,
      sessionId,
    })
    const latestForSession = await loadLatestRuntimeState(
      paths.runtimeSessionStatePath,
    )
    if (latestForSession) {
      const latestRecord = await this.sessionStore.load(
        latestForSession.runtimeSessionKey,
      )
      if (latestRecord) return latestRecord
    }

    if (sessionId !== MAIN_AGENT_SESSION_ID) return null

    const latestForAgent =
      latestForAgentHint === undefined
        ? await loadLatestRuntimeState(paths.runtimeStatePath)
        : latestForAgentHint
    if (latestForAgent?.sessionId === MAIN_AGENT_SESSION_ID) {
      const latestRecord = await this.sessionStore.load(
        latestForAgent.runtimeSessionKey,
      )
      if (latestRecord) return latestRecord
    }
    return (await this.sessionStore.load(agent.sessionKey)) ?? null
  }

  private async loadLatestAgentSessionRecord(
    agent: AgentPromptInput['agent'],
  ): Promise<{ sessionId: AgentSessionId; record: AcpSessionRecord } | null> {
    const paths = resolveAgentRuntimePaths({
      browserosDir: this.browserosDir,
      agentId: agent.id,
    })
    const latestForAgent = await loadLatestRuntimeState(paths.runtimeStatePath)
    if (latestForAgent) {
      const latestRecord = await this.sessionStore.load(
        latestForAgent.runtimeSessionKey,
      )
      if (latestRecord) {
        return {
          sessionId: latestForAgent.sessionId,
          record: latestRecord,
        }
      }
    }

    const mainRecord = await this.loadSessionRecord(
      agent,
      MAIN_AGENT_SESSION_ID,
      latestForAgent,
    )
    return mainRecord
      ? { sessionId: MAIN_AGENT_SESSION_ID, record: mainRecord }
      : null
  }

  private async prepareRuntimeContext(
    input: AgentPromptInput,
    cwdOverride: string | null,
  ): Promise<PreparedRuntimeContext> {
    const prepared = await prepareAcpxAgentContext({
      browserosDir: this.browserosDir,
      agent: input.agent,
      sessionId: input.sessionId,
      sessionKey: input.sessionKey,
      cwdOverride,
      isSelectedCwd: !!input.cwd,
      message: input.message,
    })
    return {
      cwd: prepared.cwd,
      runtimeSessionKey: prepared.runtimeSessionKey,
      runPrompt: prepared.runPrompt,
      agentCommandEnv: prepared.commandEnv,
      commandIdentity: prepared.commandIdentity,
      useBrowserosMcp: prepared.useBrowserosMcp,
      browserosMcpHost: prepared.browserosMcpHost,
    }
  }

  private getRuntime(input: {
    cwd: string
    permissionMode: AcpRuntimeOptions['permissionMode']
    nonInteractivePermissions: AcpRuntimeOptions['nonInteractivePermissions']
    commandEnv: Record<string, string>
    commandIdentity: string
    useBrowserosMcp: boolean
    browserosMcpHost?: string
    // Identifies the active turn so the nudge MCP entry's headers
    // route suggest_app_connection events back to the correct
    // ReadableStream via TurnRegistry.pushEvent.
    agentId: string
    sessionId: AgentSessionId
  }): AcpxCoreRuntime {
    const mcpHost = input.browserosMcpHost ?? '127.0.0.1'
    // agentId + sessionId are part of the key because they're baked
    // into the spawned host's MCP config headers; a different turn
    // identity needs a different runtime even if everything else
    // matches.
    const key = JSON.stringify({
      cwd: input.cwd,
      permissionMode: input.permissionMode,
      nonInteractivePermissions: input.nonInteractivePermissions,
      commandIdentity: input.commandIdentity,
      useBrowserosMcp: input.useBrowserosMcp,
      browserosMcpHost: mcpHost,
      agentId: input.agentId,
      sessionId: input.sessionId,
    })
    const existing = this.runtimes.get(key)
    if (existing) return existing

    const runtime = this.runtimeFactory({
      cwd: input.cwd,
      sessionStore: this.sessionStore,
      agentRegistry: createBrowserosAgentRegistry({
        commandEnv: input.commandEnv,
        resourcesDir: this.resourcesDir,
        browserosDir: this.browserosDir,
      }),
      mcpServers: input.useBrowserosMcp
        ? createBrowserosMcpServers(this.browserosServerPort, mcpHost, {
            agentId: input.agentId,
            sessionId: input.sessionId,
          })
        : [],
      permissionMode: input.permissionMode,
      nonInteractivePermissions: input.nonInteractivePermissions,
    })
    this.runtimes.set(key, runtime)
    logger.debug('Agent harness acpx runtime created', {
      cwd: input.cwd,
      stateDir: this.stateDir,
      permissionMode: input.permissionMode,
      nonInteractivePermissions: input.nonInteractivePermissions,
      browserosServerPort: this.browserosServerPort,
      browserosMcpHost: mcpHost,
      commandIdentity: input.commandIdentity,
      useBrowserosMcp: input.useBrowserosMcp,
      agentId: input.agentId,
      sessionId: input.sessionId,
    })
    return runtime
  }
}

type AcpxSessionMessage = AcpSessionRecord['messages'][number]
type AcpxUserContent = Extract<
  Exclude<AcpxSessionMessage, 'Resume'>,
  { User: unknown }
>['User']['content'][number]
type AcpxAgentMessage = Extract<
  Exclude<AcpxSessionMessage, 'Resume'>,
  { Agent: unknown }
>['Agent']
type AcpxAgentContent = AcpxAgentMessage['content'][number]
type AcpxToolUse = Extract<AcpxAgentContent, { ToolUse: unknown }>['ToolUse']
type AcpxToolResult = AcpxAgentMessage['tool_results'][string]

function mapAcpxSessionRecordToHistory(
  agent: AgentDefinition,
  sessionId: AgentSessionId,
  record: AcpSessionRecord,
): AgentHistoryPage {
  const createdAt = parseRecordTimestamp(record)
  const items = record.messages.flatMap(
    (message, index): AgentHistoryEntry[] => {
      if (message === 'Resume') return []
      const id = `${record.acpxRecordId}:${index}`
      const messageCreatedAt = createdAt + index

      if ('User' in message) {
        const text = message.User.content
          .map(userContentToText)
          .filter(Boolean)
          .join('\n\n')
          .trim()
        if (!text) return []
        return [
          {
            id,
            agentId: agent.id,
            sessionId,
            role: 'user',
            text,
            createdAt: messageCreatedAt,
          },
        ]
      }

      const entry = mapAgentMessageToHistoryEntry({
        id,
        agentId: agent.id,
        sessionId,
        createdAt: messageCreatedAt,
        message: message.Agent,
      })
      return entry ? [entry] : []
    },
  )

  return {
    agentId: agent.id,
    sessionId,
    items,
  }
}

function mapAgentMessageToHistoryEntry(input: {
  id: string
  agentId: string
  sessionId: AgentSessionId
  createdAt: number
  message: AcpxAgentMessage
}): AgentHistoryEntry | null {
  const textParts: string[] = []
  const reasoningParts: string[] = []
  const toolCalls: AgentHistoryToolCall[] = []

  for (const content of input.message.content) {
    if ('Text' in content) {
      textParts.push(content.Text)
    } else if ('Thinking' in content) {
      reasoningParts.push(content.Thinking.text)
    } else if ('RedactedThinking' in content) {
      reasoningParts.push('[redacted_thinking]')
    } else if ('ToolUse' in content) {
      toolCalls.push(
        mapToolUseToHistoryToolCall(
          content.ToolUse,
          input.message.tool_results[content.ToolUse.id],
        ),
      )
    }
  }

  const text = textParts.join('').trim()
  const reasoningText = reasoningParts.join('\n\n').trim()
  if (!text && !reasoningText && toolCalls.length === 0) return null

  return {
    id: input.id,
    agentId: input.agentId,
    sessionId: input.sessionId,
    role: 'assistant',
    text,
    createdAt: input.createdAt,
    ...(reasoningText ? { reasoning: { text: reasoningText } } : {}),
    ...(toolCalls.length > 0 ? { toolCalls } : {}),
  }
}

function mapToolUseToHistoryToolCall(
  tool: AcpxToolUse,
  result: AcpxToolResult | undefined,
): AgentHistoryToolCall {
  const resultValue = result ? toolResultValue(result) : undefined
  const status = result?.is_error
    ? 'failed'
    : result || tool.is_input_complete
      ? 'completed'
      : 'running'

  return {
    toolCallId: tool.id,
    toolName: result?.tool_name ?? tool.name,
    status,
    input: tool.input,
    ...(result?.is_error
      ? { error: stringifyToolError(resultValue) }
      : resultValue !== undefined
        ? { output: resultValue }
        : {}),
  }
}

function userContentToText(content: AcpxUserContent): string {
  if ('Text' in content) return unwrapBrowserosAcpUserMessage(content.Text)
  if ('Mention' in content) return content.Mention.content
  if ('Image' in content) return content.Image.source ? '[image]' : ''
  return ''
}

/**
 * Strip the BrowserOS ACP envelopes from a user-message text so HTTP
 * consumers (history endpoint, listing's `lastUserMessage`) see only
 * the user's actual question. Two layers are added on the wire today:
 *
 *   1. <role>…</role>\n\n<user_request>…</user_request> from
 *      `buildBrowserosAcpPrompt` (outer).
 *   2. ## Browser Context + <selected_text> + <USER_QUERY> from
 *      `apps/server/src/agent/format-message.ts` (inner).
 *
 * Each step is independently defensive — anchors that don't match are
 * skipped — so partially-wrapped text (older persisted records,
 * messages without a selection, future schema drift) gets best-
 * effort cleaning without throwing. The function is idempotent;
 * applying it to already-clean text is a no-op.
 *
 * TODO: drop this once acpx/runtime exposes a real system-prompt
 * surface so we can stop persisting the role block on every user
 * message. Tracked in the server architecture audit.
 */
export function unwrapBrowserosAcpUserMessage(raw: string): string {
  if (!raw) return raw
  let text = raw

  // Order matters: the outer envelope is added AFTER
  // `escapePromptTagText` runs over the inner formatUserMessage
  // payload (see buildBrowserosAcpPrompt). So once the outer
  // <role>…</role>+<user_request>…</user_request> tags are stripped,
  // the inner content is still entity-escaped (`&lt;USER_QUERY&gt;`
  // not `<USER_QUERY>`). We decode entities BEFORE the inner-envelope
  // strips so their anchors actually match.
  text = stripOuterRoleEnvelope(text)
  text = stripOuterRuntimeEnvelope(text)
  text = decodeBasicEntities(text)
  text = stripBrowserContextHeader(text)
  text = stripSelectedTextBlock(text)
  text = unwrapUserQuery(text)

  return text.trim()
}

function stripOuterRoleEnvelope(value: string): string {
  // Any `<role>…</role>\n\n<user_request>\n…\n</user_request>` envelope.
  const match = value.match(
    /^<role\b[^>]*>[\s\S]*?<\/role>\n\n<user_request>\n([\s\S]*?)\n<\/user_request>$/,
  )
  return match ? match[1] : value
}

function stripOuterRuntimeEnvelope(value: string): string {
  const match = value.match(
    /^<browseros_acpx_runtime\b[\s\S]*?<\/browseros_acpx_runtime>\n\n<user_request>\n([\s\S]*?)\n<\/user_request>$/,
  )
  return match ? match[1] : value
}

function stripBrowserContextHeader(value: string): string {
  // The `## Browser Context` block (when present) ends with the
  // `\n\n---\n\n` separator emitted by `formatBrowserContext`.
  // Anchored at the start of the string; non-greedy match through
  // the body; one removal.
  const match = value.match(/^## Browser Context\n[\s\S]*?\n\n---\n\n/)
  return match ? value.slice(match[0].length) : value
}

function stripSelectedTextBlock(value: string): string {
  // Optional `<selected_text [attrs]>…</selected_text>\n\n` block
  // emitted by `formatUserMessage` when the user has a selection.
  return value.replace(
    /<selected_text(?:[^>]*)>\n[\s\S]*?\n<\/selected_text>\n\n/,
    '',
  )
}

function unwrapUserQuery(value: string): string {
  // `formatUserMessage` always wraps the user's typed text in
  // `<USER_QUERY>\n…\n</USER_QUERY>` — even when no browser context
  // or selection is present.
  const match = value.match(/^<USER_QUERY>\n([\s\S]*?)\n<\/USER_QUERY>$/)
  return match ? match[1] : value
}

function decodeBasicEntities(value: string): string {
  // Reverse the three escapes the server applied via
  // `escapePromptTagText` so user-typed XML-like content (e.g.
  // `<USER_QUERY>` typed literally) renders as the user typed it.
  // Decode `&amp;` last to avoid double-decoding sequences like
  // `&amp;lt;` → `&lt;` → `<`.
  return value
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
}

function toolResultValue(result: AcpxToolResult): unknown {
  if (result.output != null) return result.output
  if ('Text' in result.content) return result.content.Text
  if ('Image' in result.content) return result.content.Image.source
  return undefined
}

function stringifyToolError(value: unknown): string {
  if (typeof value === 'string') return value
  if (value === undefined) return 'Tool call failed'
  try {
    return JSON.stringify(value)
  } catch {
    return 'Tool call failed'
  }
}

/**
 * Walk messages newest-to-oldest and return the first user-role text.
 * Returns null when the record has no user messages (rare — a session
 * always starts with one — but possible mid-load).
 */
function extractLastUserMessage(record: AcpSessionRecord): string | null {
  for (let i = record.messages.length - 1; i >= 0; i -= 1) {
    const message = record.messages[i]
    if (message === 'Resume') continue
    if (!('User' in message)) continue
    const text = message.User.content
      .map((block) => userContentToText(block))
      .filter(Boolean)
      .join('\n\n')
      .trim()
    if (text) return text
  }
  return null
}

function parseRecordTimestamp(record: AcpSessionRecord): number {
  const parsed = Date.parse(record.updated_at || record.lastUsedAt)
  return Number.isFinite(parsed) ? parsed : 0
}

function createAcpxEventStream(
  runtime: AcpxCoreRuntime,
  input: AgentPromptInput,
  prepared: {
    cwd: string
    runtimeSessionKey: string
    runPrompt: string
  },
): ReadableStream<AgentStreamEvent> {
  let activeTurn: AcpRuntimeTurn | null = null

  return new ReadableStream<AgentStreamEvent>({
    start(controller) {
      const run = async () => {
        const handle = await runtime.ensureSession({
          sessionKey: prepared.runtimeSessionKey,
          agent: input.agent.adapter,
          mode: 'persistent',
          cwd: prepared.cwd,
        })
        logger.info('Agent harness acpx session ensured', {
          agentId: input.agent.id,
          adapter: input.agent.adapter,
          sessionKey: prepared.runtimeSessionKey,
          browserosSessionKey: input.sessionKey,
          backendSessionId: handle.backendSessionId,
          agentSessionId: handle.agentSessionId,
          acpxRecordId: handle.acpxRecordId,
          cwd: prepared.cwd,
        })

        for (const event of await applyRuntimeControls(
          runtime,
          handle,
          input,
        )) {
          controller.enqueue(event)
        }

        const turn = runtime.startTurn({
          handle,
          text: prepared.runPrompt,
          // Image attachments travel as ACP `image` content blocks
          // alongside the text prompt. acpx's `toPromptInput` builds
          // the multi-part `prompt` array directly from this list.
          attachments:
            input.attachments && input.attachments.length > 0
              ? input.attachments.map((image) => ({
                  mediaType: image.mediaType,
                  data: image.data,
                }))
              : undefined,
          mode: 'prompt',
          requestId: crypto.randomUUID(),
          timeoutMs: input.timeoutMs,
          signal: input.signal,
        })
        activeTurn = turn
        for await (const event of turn.events) {
          controller.enqueue(mapRuntimeEvent(event))
        }
        controller.enqueue(mapTurnResult(await turn.result))
        logger.info('Agent harness acpx turn completed', {
          agentId: input.agent.id,
          adapter: input.agent.adapter,
          sessionKey: prepared.runtimeSessionKey,
          browserosSessionKey: input.sessionKey,
        })
        controller.close()
      }

      void run().catch((err) => {
        logger.error('Agent harness acpx turn failed', {
          agentId: input.agent.id,
          adapter: input.agent.adapter,
          sessionKey: prepared.runtimeSessionKey,
          browserosSessionKey: input.sessionKey,
          error: err instanceof Error ? err.message : String(err),
        })
        controller.enqueue({
          type: 'error',
          message: err instanceof Error ? err.message : String(err),
        })
        controller.close()
      })
    },
    cancel() {
      void activeTurn?.cancel({ reason: 'BrowserOS stream cancelled' })
    },
  })
}

function createBrowserosMcpServers(
  browserosServerPort: number,
  host: string,
  turnIdentity: { agentId: string; sessionId: AgentSessionId },
): NonNullable<AcpRuntimeOptions['mcpServers']> {
  return [
    {
      type: 'http',
      name: 'browseros',
      url: `http://${host}:${browserosServerPort}/mcp`,
      headers: [],
    },
    // Second entry: in-process nudge MCP server. Host LLMs see this as
    // `nudge/suggest_app_connection` and call it whenever a connection
    // is needed. The headers identify the active turn so the handler
    // can push the resulting app_connection_request event onto the
    // right stream via TurnRegistry.pushEvent.
    {
      type: 'http',
      name: 'nudge',
      url: `http://${host}:${browserosServerPort}/mcp/nudge`,
      headers: [
        { name: 'X-BrowserOS-Agent-Id', value: turnIdentity.agentId },
        { name: 'X-BrowserOS-Session-Id', value: turnIdentity.sessionId },
      ],
    },
  ]
}

function createBrowserosAgentRegistry(input: {
  commandEnv: Record<string, string>
  resourcesDir: string | null
  browserosDir: string
}): AcpRuntimeOptions['agentRegistry'] {
  const registry = createAgentRegistry()

  return {
    list() {
      return registry.list()
    },
    resolve(agentName) {
      const lower = agentName.trim().toLowerCase()

      if (lower === 'claude' || lower === 'codex') {
        const launch = resolveBrowserosHostAcpAdapterCommand({
          adapter: lower,
          resourcesDir: input.resourcesDir,
        })
        const commandEnv = withBundledNativeBinaryPath({
          env: input.commandEnv,
          resourcesDir: input.resourcesDir,
        })
        return wrapCommandWithEnv(
          launch.command,
          launch.bundledBunPath
            ? withBundledBunAcpAdapterEnv({
                bunPath: launch.bundledBunPath,
                browserosDir: input.browserosDir,
                env: commandEnv,
              })
            : commandEnv,
        )
      }

      return registry.resolve(agentName)
    },
  }
}

/**
 * Resolve host-spawned Claude/Codex ACP adapters without asking acpx
 * to discover package bins. Packaged macOS builds prefer BrowserOS's
 * bundled Bun so adapter package execution doesn't depend on host
 * `npx` or the app launch environment.
 */
function resolveBrowserosHostAcpAdapterCommand(input: {
  adapter: 'claude' | 'codex'
  resourcesDir: string | null
}): { command: string; bundledBunPath: string | null } {
  const bun = resolveBundledBun({ resourcesDir: input.resourcesDir })
  if (bun) {
    const config = HOST_ACP_ADAPTER_CONFIG[input.adapter]
    return {
      command: `${shellQuote(bun)} x --bun --silent --package ${shellQuote(config.acpPackageSpec)} ${shellQuote(config.acpBin)}`,
      bundledBunPath: bun,
    }
  }

  const config = HOST_ACP_ADAPTER_CONFIG[input.adapter]
  return {
    command: config.acpCommand,
    bundledBunPath: null,
  }
}

async function applyRuntimeControls(
  runtime: AcpxCoreRuntime,
  handle: AcpRuntimeHandle,
  input: AgentPromptInput,
): Promise<AgentStreamEvent[]> {
  const events: AgentStreamEvent[] = []
  events.push(...(await applyPermissionBypass(runtime, handle, input)))

  if (input.agent.modelId && input.agent.modelId !== 'default') {
    events.push({
      type: 'status',
      text: 'Requested model is stored on the BrowserOS agent, but this acpx/runtime version does not expose public model control. Using adapter default.',
    })
  }
  if (!input.agent.reasoningEffort) return events

  const key = input.agent.adapter === 'codex' ? 'reasoning_effort' : 'effort'
  if (!runtime.setConfigOption) {
    events.push({
      type: 'status',
      text: `Requested ${key}=${input.agent.reasoningEffort}, but this acpx/runtime version does not expose config control.`,
    })
    return events
  }

  try {
    await runtime.setConfigOption({
      handle,
      key,
      value: input.agent.reasoningEffort,
    })
    logger.debug('Agent harness acpx config applied', {
      agentId: input.agent.id,
      adapter: input.agent.adapter,
      sessionKey: input.sessionKey,
      key,
      value: input.agent.reasoningEffort,
    })
  } catch (err) {
    logger.warn('Agent harness acpx config unavailable', {
      agentId: input.agent.id,
      adapter: input.agent.adapter,
      sessionKey: input.sessionKey,
      key,
      value: input.agent.reasoningEffort,
      error: err instanceof Error ? err.message : String(err),
    })
    events.push({
      type: 'status',
      text: `Could not apply ${key}=${input.agent.reasoningEffort}; continuing with the adapter default. ${
        err instanceof Error ? err.message : String(err)
      }`,
    })
  }
  return events
}

/**
 * Lifts approve-all sessions into the adapter's full-permission mode via
 * ACP `session/set_mode` — otherwise the adapter inherits the user's own
 * CLI permission defaults. Candidates are tried in order because the two
 * codex-acp packages advertise different ids for the same full-access
 * preset.
 */
async function applyPermissionBypass(
  runtime: AcpxCoreRuntime,
  handle: AcpRuntimeHandle,
  input: AgentPromptInput,
): Promise<AgentStreamEvent[]> {
  if (input.permissionMode !== 'approve-all') return []
  const candidates = DANGEROUS_ALLOW_MODE_CANDIDATES[input.agent.adapter]
  if (!candidates?.length) return []

  const requested = `${HOST_ACP_ADAPTER_CONFIG[input.agent.adapter].displayName} ${candidates.join(' / ')}`

  if (!runtime.setMode) {
    return [
      {
        type: 'status',
        text: `Requested ${requested} mode, but this acpx/runtime version does not expose mode control.`,
      },
    ]
  }

  let lastError: unknown
  for (const mode of candidates) {
    try {
      await runtime.setMode({ handle, mode })
      logger.debug('Agent harness acpx mode applied', {
        agentId: input.agent.id,
        adapter: input.agent.adapter,
        sessionKey: input.sessionKey,
        mode,
      })
      return []
    } catch (err) {
      lastError = err
      // debug, not warn: Codex ACP builds have used both `agent-full-access`
      // and `full-access` for the same bypass mode. Only the all-rejected
      // case below warns.
      logger.debug('Agent harness acpx mode candidate rejected', {
        agentId: input.agent.id,
        adapter: input.agent.adapter,
        sessionKey: input.sessionKey,
        mode,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }
  return [
    {
      type: 'status',
      text: `Could not apply ${requested} mode; continuing with the adapter default. ${
        lastError instanceof Error ? lastError.message : String(lastError)
      }`,
    },
  ]
}

function mapRuntimeEvent(event: AcpRuntimeEvent): AgentStreamEvent {
  switch (event.type) {
    case 'text_delta':
      return {
        type: 'text_delta',
        text: event.text,
        stream: event.stream ?? 'output',
        rawType: event.tag,
      }
    case 'tool_call':
      return {
        type: 'tool_call',
        text: event.text,
        title: event.title ?? 'tool call',
        id: event.toolCallId,
        status: event.status,
        rawType: event.tag,
      }
    case 'status':
      return {
        type: 'status',
        text: event.text,
        rawType: event.tag,
      }
    case 'done':
      return {
        type: 'done',
        stopReason: event.stopReason,
      }
    case 'error':
      return {
        type: 'error',
        message: event.message,
        code: event.code,
      }
    default: {
      const exhaustive: never = event
      return exhaustive
    }
  }
}

function mapTurnResult(result: AcpRuntimeTurnResult): AgentStreamEvent {
  switch (result.status) {
    case 'completed':
      return { type: 'done', stopReason: result.stopReason }
    case 'cancelled':
      return { type: 'done', stopReason: result.stopReason ?? 'cancelled' }
    case 'failed':
      return {
        type: 'error',
        message: result.error.message,
        code: result.error.code,
      }
    default: {
      const exhaustive: never = result
      return exhaustive
    }
  }
}
