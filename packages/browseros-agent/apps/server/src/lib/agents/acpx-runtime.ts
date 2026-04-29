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
 * current port/token at spawn time — port can change after a gateway
 * restart, token can rotate.
 */
export interface OpenclawGatewayAccessor {
  /** Host port the gateway is currently bound to (e.g. 18789). */
  getPort(): number
  /** Current gateway auth token. Passed via OPENCLAW_GATEWAY_TOKEN env. */
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

  async send(
    input: AgentPromptInput,
  ): Promise<ReadableStream<AgentStreamEvent>> {
    const cwd = input.cwd ?? this.cwd
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
    })
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
        input.permissionMode,
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
  if ('Text' in content) return unwrapBrowserosAcpPrompt(content.Text)
  if ('Mention' in content) return content.Mention.content
  if ('Image' in content) return content.Image.source ? '[image]' : ''
  return ''
}

function unwrapBrowserosAcpPrompt(value: string): string {
  const prefix = `${BROWSEROS_ACP_AGENT_INSTRUCTIONS}

<user_request>
`
  const suffix = `
</user_request>`
  if (!value.startsWith(prefix) || !value.endsWith(suffix)) return value

  // TODO: nikhil: remove this once acpx/runtime exposes system prompt support.
  return unescapePromptTagText(value.slice(prefix.length, -suffix.length))
}

function unescapePromptTagText(value: string): string {
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
  permissionMode: AcpRuntimeOptions['permissionMode'],
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

      const command = registry.resolve(agentName)
      if (permissionMode !== 'approve-all') return command

      switch (lower) {
        case 'claude':
          return appendCommandArg(command, '--dangerously-skip-permissions')
        case 'codex':
          return appendCommandArg(
            command,
            '--dangerously-bypass-approvals-and-sandbox',
          )
        default:
          return command
      }
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
 * Auth: the gateway token is injected as the OPENCLAW_GATEWAY_TOKEN
 * env var on the container exec, which the openclaw CLI honors per
 * its documented env-var precedence. This avoids the `--token` flag
 * showing the secret in `ps aux`.
 *
 * Banner output: OPENCLAW_HIDE_BANNER and OPENCLAW_SUPPRESS_NOTES
 * suppress non-JSON-RPC chatter on stdout that would otherwise corrupt
 * the ACP message stream.
 */
function resolveOpenclawAcpCommand(
  gateway: OpenclawGatewayAccessor,
  sessionKey: string | null,
): string {
  const port = gateway.getPort()
  const token = gateway.getGatewayToken()
  const limactl = gateway.getLimactlPath()
  const vm = gateway.getVmName()
  const container = gateway.getContainerName()
  const limaHome = gateway.getLimaHomeDir()

  // `--session <key>` routes the bridge's newSession requests to the
  // matching gateway agent. acpx does not pass sessionKey through ACP
  // newSession params, so without this CLI flag the bridge falls back
  // to a synthetic acp:<uuid> session that does not resolve to any
  // provisioned gateway agent.
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
    vm,
    '--',
    'nerdctl',
    'exec',
    '-i',
    '-e',
    'OPENCLAW_HIDE_BANNER=1',
    '-e',
    'OPENCLAW_SUPPRESS_NOTES=1',
    '-e',
    `OPENCLAW_GATEWAY_TOKEN=${token}`,
    container,
    'openclaw',
    'acp',
    '--url',
    `ws://127.0.0.1:${port}`,
  ]
  if (sessionKey) {
    argv.push('--session', sessionKey)
  }
  return argv.join(' ')
}

function appendCommandArg(command: string, arg: string): string {
  return command.split(/\s+/).includes(arg) ? command : `${command} ${arg}`
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
