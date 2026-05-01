/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { randomUUID } from 'node:crypto'
import { join } from 'node:path'
import { OPENCLAW_GATEWAY_CONTAINER_PORT } from '@browseros/shared/constants/openclaw'
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
import type {
  OpenAIChatMessage,
  OpenAIContentPart,
  OpenClawGatewayChatClient,
} from '../../api/services/openclaw/openclaw-gateway-chat-client'
import { getBrowserosDir } from '../browseros-dir'
import { logger } from '../logger'
import type {
  AgentDefinition,
  AgentHistoryEntry,
  AgentHistoryToolCall,
} from './agent-types'
import type {
  AgentHistoryPage,
  AgentPromptInput,
  AgentRowSnapshot,
  AgentRuntime,
  AgentSession,
  AgentStatus,
  AgentStreamEvent,
} from './types'

/**
 * Live-getter access to the OpenClaw gateway runtime info. Required
 * when spawning the openclaw ACP adapter inside the gateway container.
 *
 * Fields are getters (not snapshot values) so the harness picks up the
 * current token and VM/container paths at spawn time.
 */
export interface OpenclawGatewayAccessor {
  /** Current gateway auth token. Passed to `openclaw acp --token`. */
  getGatewayToken(): string
  /** Container name e.g. browseros-openclaw-openclaw-gateway-1. */
  getContainerName(): string
  /** LIMA_HOME directory containing the browseros-vm instance. */
  getLimaHomeDir(): string
  /** Resolved path to the `limactl` binary (bundled or host). */
  getLimactlPath(): string
  /** VM name registered in LIMA_HOME (e.g. browseros-vm). */
  getVmName(): string
}

type AcpxRuntimeOptions = {
  cwd?: string
  stateDir?: string
  browserosServerPort?: number
  /**
   * Required for adapter='openclaw' agents; harmless when absent for
   * claude/codex (their adapters spawn their own CLI binaries).
   */
  openclawGateway?: OpenclawGatewayAccessor
  /**
   * Optional. When wired, the runtime diverts OpenClaw turns that
   * carry image attachments to the gateway's HTTP `/v1/chat/completions`
   * endpoint (which accepts OpenAI-style `image_url` parts) instead of
   * the ACP bridge — the bridge silently drops image content blocks.
   * Without this client, image turns to OpenClaw agents fall through to
   * the ACP path and the model never sees the image.
   */
  openclawGatewayChat?: OpenClawGatewayChatClient
  runtimeFactory?: (options: AcpRuntimeOptions) => AcpxCoreRuntime
}

const BROWSEROS_ACP_AGENT_INSTRUCTIONS = `<role>
You are BrowserOS - a browser agent with full control of a Chromium browser through the BrowserOS MCP server.

Use the BrowserOS MCP server for all browser tasks, including browsing the web, interacting with pages, inspecting browser state, and managing tabs, windows, bookmarks, and history.
</role>`

export class AcpxRuntime implements AgentRuntime {
  private readonly cwd: string
  private readonly stateDir: string
  private readonly browserosServerPort: number
  private readonly openclawGateway: OpenclawGatewayAccessor | null
  private readonly openclawGatewayChat: OpenClawGatewayChatClient | null
  private readonly runtimeFactory: (
    options: AcpRuntimeOptions,
  ) => AcpxCoreRuntime
  private readonly sessionStore: ReturnType<typeof createRuntimeStore>
  private readonly runtimes = new Map<string, AcpxCoreRuntime>()

  constructor(options: AcpxRuntimeOptions = {}) {
    this.cwd = options.cwd ?? process.cwd()
    this.stateDir =
      options.stateDir ??
      process.env.BROWSEROS_ACPX_STATE_DIR ??
      join(getBrowserosDir(), 'agents', 'acpx')
    this.browserosServerPort =
      options.browserosServerPort ?? DEFAULT_PORTS.server
    this.openclawGateway = options.openclawGateway ?? null
    this.openclawGatewayChat = options.openclawGatewayChat ?? null
    this.sessionStore = createRuntimeStore({ stateDir: this.stateDir })
    this.runtimeFactory = options.runtimeFactory ?? createAcpRuntime
  }

  async status(): Promise<AgentStatus> {
    return { state: 'unknown', message: 'acpx status is checked on send' }
  }

  async listSessions(
    input: AgentPromptInput['agent'],
  ): Promise<AgentSession[]> {
    return [{ agentId: input.id, id: 'main', updatedAt: input.updatedAt }]
  }

  async getHistory(input: {
    agent: AgentPromptInput['agent']
    sessionId: 'main'
  }): Promise<AgentHistoryPage> {
    const record = await this.sessionStore.load(input.agent.sessionKey)
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
    sessionId: 'main'
  }): Promise<AgentRowSnapshot | null> {
    const record = await this.sessionStore.load(input.agent.sessionKey)
    if (!record) return null
    return {
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
    const cwd = input.cwd ?? this.cwd
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

    // Image carve-out for OpenClaw: the openclaw `acp` bridge silently
    // drops ACP `image` content blocks, so the model never sees the
    // attachment. Divert image-bearing turns to the gateway's HTTP
    // /v1/chat/completions endpoint (which accepts OpenAI-style
    // `image_url` parts) and pipe its SSE back through the same
    // AgentStreamEvent shape callers already consume.
    if (
      input.agent.adapter === 'openclaw' &&
      imageAttachments.length > 0 &&
      this.openclawGatewayChat
    ) {
      return this.sendOpenclawViaGateway(input, imageAttachments, cwd)
    }

    const runtime = this.getRuntime({
      cwd,
      permissionMode: input.permissionMode,
      nonInteractivePermissions: 'fail',
      // OpenClaw agents need their gateway sessionKey baked into the
      // spawn command (acpx does not forward sessionKey to newSession);
      // claude/codex don't, and including it would split their cache.
      openclawSessionKey:
        input.agent.adapter === 'openclaw' ? input.sessionKey : null,
    })

    return createAcpxEventStream(runtime, input, cwd)
  }

  private getRuntime(input: {
    cwd: string
    permissionMode: AcpRuntimeOptions['permissionMode']
    nonInteractivePermissions: AcpRuntimeOptions['nonInteractivePermissions']
    openclawSessionKey: string | null
  }): AcpxCoreRuntime {
    const key = JSON.stringify(input)
    const existing = this.runtimes.get(key)
    if (existing) return existing

    // OpenClaw exposes its provider tools through the gateway, not through
    // ACP-side MCP servers. Forwarding the BrowserOS HTTP MCP to its bridge
    // makes newSession fail because openclaw rejects unsupported transports.
    // Claude/codex still need the BrowserOS MCP for browser tooling.
    const isOpenclaw = input.openclawSessionKey !== null
    const runtime = this.runtimeFactory({
      cwd: input.cwd,
      sessionStore: this.sessionStore,
      agentRegistry: createBrowserosAgentRegistry(
        this.openclawGateway,
        input.openclawSessionKey,
      ),
      mcpServers: isOpenclaw
        ? []
        : createBrowserosMcpServers(this.browserosServerPort),
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
      openclawSessionKey: input.openclawSessionKey,
    })
    return runtime
  }

  /**
   * Drives an OpenClaw turn that includes image attachments through the
   * gateway HTTP endpoint, which translates OpenAI-style `image_url`
   * content parts into provider-native multimodal calls. Streams back
   * `AgentStreamEvent` so the chat panel renders identically to ACP
   * turns. On natural completion, appends a synthetic user+assistant
   * pair to the acpx session record so the turn shows up in
   * `getHistory()` after a reload.
   *
   * Persistence is best-effort: when no session record exists yet (e.g.
   * the very first turn for a fresh agent is image-only), the live
   * stream still works but the turn is absent from history on reload.
   * Subsequent text turns through ACP create/update the record normally.
   */
  private async sendOpenclawViaGateway(
    input: AgentPromptInput,
    imageAttachments: ReadonlyArray<{ mediaType: string; data: string }>,
    cwd: string,
  ): Promise<ReadableStream<AgentStreamEvent>> {
    if (!this.openclawGatewayChat) {
      throw new Error(
        'OpenClaw gateway chat client is not wired into AcpxRuntime',
      )
    }

    const existingRecord = await this.sessionStore.load(input.sessionKey)
    const priorMessages = existingRecord
      ? recordToOpenAIMessages(existingRecord)
      : []
    const userContent: OpenAIContentPart[] = [
      { type: 'text', text: buildBrowserosAcpPrompt(input.message) },
      ...imageAttachments.map(
        (a): OpenAIContentPart => ({
          type: 'image_url',
          image_url: { url: `data:${a.mediaType};base64,${a.data}` },
        }),
      ),
    ]
    const messages: OpenAIChatMessage[] = [
      ...priorMessages,
      { role: 'user', content: userContent },
    ]

    logger.info('Agent harness gateway image turn dispatched', {
      agentId: input.agent.id,
      sessionKey: input.sessionKey,
      cwd,
      priorMessageCount: priorMessages.length,
      imageAttachmentCount: imageAttachments.length,
    })

    const upstream = await this.openclawGatewayChat.streamTurn({
      agentId: input.agent.id,
      sessionKey: input.sessionKey,
      messages,
      signal: input.signal,
    })

    const sessionStore = this.sessionStore
    const sessionKey = input.sessionKey
    const userMessageText = input.message
    let accumulated = ''

    return new ReadableStream<AgentStreamEvent>({
      start: (controller) => {
        const reader = upstream.getReader()
        const persist = async () => {
          if (!existingRecord || !accumulated) return
          try {
            await persistGatewayTurn(
              sessionStore,
              sessionKey,
              userMessageText,
              imageAttachments,
              accumulated,
            )
          } catch (err) {
            logger.warn(
              'Failed to persist gateway image turn to acpx session record',
              {
                sessionKey,
                error: err instanceof Error ? err.message : String(err),
              },
            )
          }
        }
        ;(async () => {
          try {
            while (true) {
              const { done, value } = await reader.read()
              if (done) break
              if (value.type === 'text_delta') accumulated += value.text
              controller.enqueue(value)
            }
            await persist()
            controller.close()
          } catch (err) {
            controller.enqueue({
              type: 'error',
              message: err instanceof Error ? err.message : String(err),
            })
            controller.close()
          }
        })().catch(() => {})
      },
      cancel: () => {
        // Best-effort: cancel propagation to the gateway is its own
        // upstream issue (see plan), but at least drop our reader so
        // the OpenAI SSE parse loop exits.
      },
    })
  }
}

async function persistGatewayTurn(
  sessionStore: ReturnType<typeof createRuntimeStore>,
  sessionKey: string,
  userMessageText: string,
  imageAttachments: ReadonlyArray<{ mediaType: string; data: string }>,
  assistantText: string,
): Promise<void> {
  const record = await sessionStore.load(sessionKey)
  if (!record) return
  const userContent: AcpxUserContent[] = [
    { Text: buildBrowserosAcpPrompt(userMessageText) } as AcpxUserContent,
  ]
  for (const _image of imageAttachments) {
    // The history mapper's `userContentToText` reads `Image.source` and
    // emits `[image]` for any non-empty value — we just need a truthy
    // marker so the placeholder renders. We don't store the base64 in
    // the record (it's already in the gateway's transcript and would
    // bloat the JSON file).
    userContent.push({ Image: { source: 'base64' } } as AcpxUserContent)
  }
  // The acpx persistence layer requires User messages to carry an `id`
  // and Agent messages to carry a `tool_results` object — without them
  // the record fails to round-trip through `parseSessionRecord` on next
  // load. See acpx/dist/prompt-turn-... `isUserMessage`/`isAgentMessage`.
  const turnId = randomUUID()
  const updated = {
    ...record,
    messages: [
      ...record.messages,
      { User: { id: `user-${turnId}`, content: userContent } },
      { Agent: { content: [{ Text: assistantText }], tool_results: {} } },
    ],
    lastUsedAt: new Date().toISOString(),
  } as AcpSessionRecord
  await sessionStore.save(updated)
}

function recordToOpenAIMessages(record: AcpSessionRecord): OpenAIChatMessage[] {
  const messages: OpenAIChatMessage[] = []
  for (const message of record.messages) {
    if (message === 'Resume') continue
    if ('User' in message) {
      const text = message.User.content
        .map(userContentToText)
        .filter(Boolean)
        .join('\n\n')
        .trim()
      if (text) messages.push({ role: 'user', content: text })
      continue
    }
    if ('Agent' in message) {
      const text = message.Agent.content
        .map((part) => ('Text' in part ? part.Text : ''))
        .join('')
        .trim()
      if (text) messages.push({ role: 'assistant', content: text })
    }
  }
  return messages
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
  sessionId: 'main',
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
  sessionId: 'main'
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
  text = decodeBasicEntities(text)
  text = stripBrowserContextHeader(text)
  text = stripSelectedTextBlock(text)
  text = unwrapUserQuery(text)

  return text.trim()
}

function stripOuterRoleEnvelope(value: string): string {
  const prefix = `${BROWSEROS_ACP_AGENT_INSTRUCTIONS}

<user_request>
`
  const suffix = `
</user_request>`
  if (!value.startsWith(prefix) || !value.endsWith(suffix)) return value
  return value.slice(prefix.length, -suffix.length)
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
  cwd: string,
): ReadableStream<AgentStreamEvent> {
  let activeTurn: AcpRuntimeTurn | null = null

  return new ReadableStream<AgentStreamEvent>({
    start(controller) {
      const run = async () => {
        const handle = await runtime.ensureSession({
          sessionKey: input.sessionKey,
          agent: input.agent.adapter,
          mode: 'persistent',
          cwd,
        })
        logger.info('Agent harness acpx session ensured', {
          agentId: input.agent.id,
          adapter: input.agent.adapter,
          sessionKey: input.sessionKey,
          backendSessionId: handle.backendSessionId,
          agentSessionId: handle.agentSessionId,
          acpxRecordId: handle.acpxRecordId,
          cwd,
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
          text: buildBrowserosAcpPrompt(input.message),
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
          sessionKey: input.sessionKey,
        })
        controller.close()
      }

      void run().catch((err) => {
        logger.error('Agent harness acpx turn failed', {
          agentId: input.agent.id,
          adapter: input.agent.adapter,
          sessionKey: input.sessionKey,
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
): NonNullable<AcpRuntimeOptions['mcpServers']> {
  return [
    {
      type: 'http',
      name: 'browseros',
      url: `http://127.0.0.1:${browserosServerPort}/mcp`,
      headers: [],
    },
  ]
}

function createBrowserosAgentRegistry(
  openclawGateway: OpenclawGatewayAccessor | null,
  openclawSessionKey: string | null,
): AcpRuntimeOptions['agentRegistry'] {
  const registry = createAgentRegistry()

  return {
    list() {
      return registry.list()
    },
    resolve(agentName) {
      const lower = agentName.trim().toLowerCase()

      if (lower === 'openclaw') {
        if (!openclawGateway) {
          // Fall back to acpx's built-in `openclaw` adapter, which assumes
          // a host-side openclaw binary. BrowserOS doesn't install one on
          // the host, so this branch will fail at spawn time with a
          // descriptive error — the harness should be wired with a
          // gateway accessor.
          return registry.resolve(agentName)
        }
        return resolveOpenclawAcpCommand(openclawGateway, openclawSessionKey)
      }

      return registry.resolve(agentName)
    },
  }
}

/**
 * Builds the command string acpx will spawn for an `openclaw` adapter.
 * Runs `openclaw acp` inside the gateway container via the bundled
 * `limactl shell <vm> -- nerdctl exec -i ...` chain so the binary
 * already installed alongside the gateway is reused; BrowserOS does
 * not require a host-side openclaw install.
 *
 * Auth: `openclaw acp --url ...` deliberately does not reuse implicit
 * env/config credentials, so pass the gateway token explicitly.
 *
 * Banner output: OPENCLAW_HIDE_BANNER and OPENCLAW_SUPPRESS_NOTES
 * suppress non-JSON-RPC chatter on stdout that would otherwise corrupt
 * the ACP message stream.
 */
function resolveOpenclawAcpCommand(
  gateway: OpenclawGatewayAccessor,
  sessionKey: string | null,
): string {
  const token = gateway.getGatewayToken()
  const limactl = gateway.getLimactlPath()
  const vm = gateway.getVmName()
  const container = gateway.getContainerName()
  const limaHome = gateway.getLimaHomeDir()
  const gatewayUrlInsideContainer = `ws://127.0.0.1:${OPENCLAW_GATEWAY_CONTAINER_PORT}`

  // `--session <key>` routes the bridge's newSession requests to the
  // matching gateway agent. acpx does not pass sessionKey through ACP
  // newSession params, so without this CLI flag the bridge falls back
  // to a synthetic acp:<uuid> session that does not resolve to any
  // provisioned gateway agent.
  //
  // Harness keys are `agent:<harness-id>:main`; the harness id matches
  // a dual-created gateway agent name, so the bridge resolves directly.
  // Any legacy non-agent key falls back to the always-provisioned
  // `main` gateway agent with the original key encoded as a channel
  // suffix.
  const bridgeSessionKey = sessionKey
    ? sessionKey.startsWith('agent:')
      ? sessionKey
      : `agent:main:${sessionKey.replace(/[^a-zA-Z0-9-]/g, '-')}`
    : null
  //
  // Prefix `env LIMA_HOME=<path>` so the spawned limactl finds the
  // BrowserOS-owned VM instance. The BrowserOS server doesn't set
  // LIMA_HOME on its own process env (it injects per-spawn elsewhere),
  // so the acpx-spawned subprocess won't inherit it without this hint.
  const argv = [
    'env',
    `LIMA_HOME=${limaHome}`,
    limactl,
    'shell',
    '--workdir',
    '/',
    vm,
    '--',
    'nerdctl',
    'exec',
    '-i',
    '-e',
    'OPENCLAW_HIDE_BANNER=1',
    '-e',
    'OPENCLAW_SUPPRESS_NOTES=1',
    container,
    'openclaw',
    'acp',
    '--url',
    gatewayUrlInsideContainer,
    '--token',
    token,
  ]
  if (bridgeSessionKey) {
    argv.push('--session', bridgeSessionKey)
  }
  return argv.join(' ')
}

function buildBrowserosAcpPrompt(message: string): string {
  return `${BROWSEROS_ACP_AGENT_INSTRUCTIONS}

<user_request>
${escapePromptTagText(message)}
</user_request>`
}

function escapePromptTagText(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
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

async function applyPermissionBypass(
  runtime: AcpxCoreRuntime,
  handle: AcpRuntimeHandle,
  input: AgentPromptInput,
): Promise<AgentStreamEvent[]> {
  if (
    input.permissionMode !== 'approve-all' ||
    input.agent.adapter !== 'claude'
  ) {
    return []
  }

  if (!runtime.setMode) {
    return [
      {
        type: 'status',
        text: 'Requested Claude bypassPermissions mode, but this acpx/runtime version does not expose mode control.',
      },
    ]
  }

  try {
    await runtime.setMode({ handle, mode: 'bypassPermissions' })
    logger.debug('Agent harness acpx mode applied', {
      agentId: input.agent.id,
      adapter: input.agent.adapter,
      sessionKey: input.sessionKey,
      mode: 'bypassPermissions',
    })
  } catch (err) {
    logger.warn('Agent harness acpx mode unavailable', {
      agentId: input.agent.id,
      adapter: input.agent.adapter,
      sessionKey: input.sessionKey,
      mode: 'bypassPermissions',
      error: err instanceof Error ? err.message : String(err),
    })
    return [
      {
        type: 'status',
        text: `Could not apply Claude bypassPermissions mode; continuing with the adapter default. ${
          err instanceof Error ? err.message : String(err)
        }`,
      },
    ]
  }
  return []
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
