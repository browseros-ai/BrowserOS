/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Main orchestrator for OpenClaw integration.
 * Container lifecycle via the VM runtime, agent CRUD via in-container CLI,
 * chat via HTTP /v1/chat/completions proxy.
 */

import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import {
  OPENCLAW_CONTAINER_HOME,
  OPENCLAW_GATEWAY_CONTAINER_PORT,
} from '@browseros/shared/constants/openclaw'
import { DEFAULT_PORTS } from '@browseros/shared/constants/ports'
import { getOpenClawDir } from '../../../lib/browseros-dir'
import { logger } from '../../../lib/logger'
import type { MonitoringChatTurn } from '../../../monitoring/types'
import { buildToolLabel } from '../../../tools/tool-label-registry'
import {
  type AgentLiveStatus,
  type AgentSessionState,
  ClawSession,
} from './claw-session'
import type {
  ContainerRuntime,
  GatewayContainerSpec,
} from './container-runtime'
import {
  buildContainerRuntime,
  type VmCacheRuntimeConfig,
} from './container-runtime-factory'
import {
  OpenClawAgentAlreadyExistsError,
  OpenClawAgentNotFoundError,
  OpenClawInvalidAgentNameError,
  OpenClawProtectedAgentError,
} from './errors'
import {
  type OpenClawAgentRecord,
  OpenClawCliClient,
  type OpenClawConfigBatchEntry,
} from './openclaw-cli-client'
import {
  buildOpenClawCliProviderModelRef,
  getOpenClawCliProvider,
  OPENCLAW_CLI_PROVIDERS,
} from './openclaw-cli-providers/registry'
import type {
  OpenClawCliProvider,
  OpenClawCliProviderAuthStatus,
} from './openclaw-cli-providers/types'
import {
  getHostWorkspaceDir,
  getOpenClawStateConfigPath,
  getOpenClawStateDir,
  getOpenClawStateEnvPath,
  mergeEnvContent,
} from './openclaw-env'
import {
  type OpenClawChatContentPart,
  OpenClawHttpClient,
  type OpenClawSessionHistory,
  type OpenClawSessionHistoryEvent,
} from './openclaw-http-client'
import {
  type ClawEvent,
  OpenClawJsonlReader,
  summarizeToolActivity,
} from './openclaw-jsonl-reader'
import { OpenClawObserver } from './openclaw-observer'
import {
  type ResolvedOpenClawProviderConfig,
  resolveSupportedOpenClawProvider,
} from './openclaw-provider-map'
import type { OpenClawStreamEvent } from './openclaw-types'
import { allocateGatewayPort, readPersistedGatewayPort } from './runtime-state'

const READY_TIMEOUT_MS = 30_000
const AGENT_NAME_PATTERN = /^[a-z][a-z0-9-]*$/
const OPENCLAW_BROWSEROS_USER_SESSION_PATTERN =
  /^agent:[^:]+:openai-user:browseros:[^:]+:(.+)$/

export type OpenClawControlPlaneStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  // Retained for extension compatibility while the UI still branches on it.
  | 'recovering'
  | 'failed'

export type OpenClawGatewayRecoveryReason =
  // Retained for extension compatibility while the UI still renders these reasons.
  | 'transient_disconnect'
  | 'signature_expired'
  | 'pairing_required'
  | 'token_mismatch'
  | 'container_not_ready'
  | 'unknown'

export type OpenClawStatus =
  | 'uninitialized'
  | 'starting'
  | 'running'
  | 'stopped'
  | 'error'

export interface OpenClawStatusResponse {
  status: OpenClawStatus
  podmanAvailable: boolean
  machineReady: boolean
  port: number | null
  agentCount: number
  error: string | null
  controlPlaneStatus: OpenClawControlPlaneStatus
  lastGatewayError: string | null
  lastRecoveryReason: OpenClawGatewayRecoveryReason | null
}

export type OpenClawAgentEntry = OpenClawAgentRecord

export interface SetupInput {
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
  // The agent UI's "Supports Image" flag (LlmProviderConfig.supportsImages).
  // Pass-through to provider-map so custom OpenAI-compat providers can
  // advertise `input: ['text', 'image']` on their model entries when the
  // user asserted vision support.
  supportsImages?: boolean
}

export interface OpenClawProviderUpdateResult {
  restarted: boolean
  modelUpdated: boolean
}

export interface OpenClawServiceConfig {
  browserosServerPort?: number
  resourcesDir?: string
  browserosDir?: string
  vmCache?: VmCacheRuntimeConfig
}

export type OpenClawSessionSource =
  | 'user-chat'
  | 'cron'
  | 'hook'
  | 'channel'
  | 'other'

export interface BrowserOSOpenClawSession {
  key: string
  updatedAt: number
  sessionId: string
  agentId: string
  kind: string
  source: OpenClawSessionSource
  status?: string
  totalTokens?: number
  model?: string
  modelProvider?: string
}

export interface BrowserOSOpenClawAgentSessionResponse {
  agentId: string
  exists: boolean
  sessionKey: string | null
  session: BrowserOSOpenClawSession | null
}

export interface BrowserOSChatHistoryToolCall {
  toolCallId?: string
  toolName: string
  label: string
  subject?: string
  status: 'completed' | 'failed'
  input?: Record<string, unknown>
  output?: string
  error?: string
  durationMs?: number
}

export interface BrowserOSChatHistoryReasoning {
  text: string
  durationMs?: number
}

export interface BrowserOSChatHistoryAttachment {
  kind: 'image' | 'file'
  mediaType: string
  // Images carry the full data: URL so the client can render directly.
  // Files (text / pdf / etc) currently round-trip via inline text in the
  // message body and don't reach this field — kept on the type for v2.
  dataUrl?: string
  name?: string
}

export interface BrowserOSChatHistoryItem {
  id: string
  role: 'user' | 'assistant'
  text: string
  timestamp?: number
  messageSeq: number
  sessionKey: string
  source: OpenClawSessionSource
  costUsd?: number
  tokensIn?: number
  tokensOut?: number
  toolCalls?: BrowserOSChatHistoryToolCall[]
  reasoning?: BrowserOSChatHistoryReasoning
  attachments?: BrowserOSChatHistoryAttachment[]
}

export interface BrowserOSOpenClawHistoryPageResponse {
  agentId: string
  sessionKey: string | null
  session: BrowserOSOpenClawSession | null
  items: BrowserOSChatHistoryItem[]
  page: {
    cursor?: string
    hasMore: boolean
    limit: number
  }
}

interface HistoryPageInput {
  sessionKey?: string
  cursor?: string
  limit?: number
}

export function normalizeBrowserOSChatSessionKey(
  agentId: string,
  sessionKey: string,
): string {
  const trimmed = sessionKey.trim()
  if (!trimmed) return trimmed

  let normalized = trimmed
  const agentSpecificPrefix = getOpenClawBrowserOSSessionPrefix(agentId)

  while (normalized.startsWith(agentSpecificPrefix)) {
    normalized = normalized.slice(agentSpecificPrefix.length)
  }

  while (true) {
    const match = normalized.match(OPENCLAW_BROWSEROS_USER_SESSION_PATTERN)
    if (!match?.[1]) break
    normalized = match[1]
  }

  return normalized.trim() || trimmed
}

function getOpenClawBrowserOSSessionPrefix(agentId: string): string {
  return `agent:${agentId}:openai-user:browseros:${agentId}:`
}

function toOpenClawBrowserOSSessionKey(
  agentId: string,
  sessionKey: string,
): string {
  return `${getOpenClawBrowserOSSessionPrefix(agentId)}${normalizeBrowserOSChatSessionKey(
    agentId,
    sessionKey,
  )}`
}

function normalizeHistoryLimit(limit?: number): number {
  if (limit === undefined || !Number.isFinite(limit)) return 50
  return Math.max(1, Math.min(100, Math.trunc(limit)))
}

function classifySessionSource(key: string): OpenClawSessionSource {
  if (key.includes(':cron:')) return 'cron'
  if (key.includes(':hook:')) return 'hook'
  if (key.includes('openai-user:browseros')) return 'user-chat'
  if (key.includes('qa-channel')) return 'channel'
  return 'other'
}

/**
 * Convert JSONL events to BrowserOS chat history items, applying the same
 * filtering rules as the old HTTP-based pipeline (filterHttpSessionHistoryMessages).
 */
function jsonlEventsToHistoryItems(
  events: ClawEvent[],
  sessionKey: string,
  source: OpenClawSessionSource,
): BrowserOSChatHistoryItem[] {
  const items: BrowserOSChatHistoryItem[] = []
  let seq = 0

  // Accumulate tool calls between text messages. The agent emits
  // tool_use → tool_result pairs interspersed with assistant text. We
  // pair them by toolCallId and attach the resulting list to the next
  // assistant message (the one that follows the tool sequence).
  let pendingToolCalls: BrowserOSChatHistoryToolCall[] = []
  const pendingToolStarts = new Map<string, ClawEvent>()

  // Accumulate thinking blocks across the turn — there can be multiple
  // (e.g., think → tool → think → tool → answer). We collapse them into
  // a single Reasoning block per assistant message so the UI shows one
  // collapsible per turn, with duration = first thinking → final answer.
  let pendingReasoningTexts: string[] = []
  let pendingReasoningFirstAt: number | null = null

  // Accumulate user-side attachments. The reader emits them as separate
  // `user.attachment` events ordered immediately before the user.message
  // they belong to (same JSONL line). We flush them onto the next user
  // history item and reset the buffer alongside the per-turn buffers.
  let pendingAttachments: BrowserOSChatHistoryAttachment[] = []

  for (const event of events) {
    if (event.type === 'user.attachment') {
      if (event.attachment) {
        pendingAttachments.push({
          kind: event.attachment.kind,
          mediaType: event.attachment.mediaType,
          dataUrl: event.attachment.dataUrl,
          name: event.attachment.name,
        })
      }
      continue
    }

    if (event.type === 'agent.thinking') {
      const text = event.content.trim()
      if (text) pendingReasoningTexts.push(text)
      if (pendingReasoningFirstAt == null) {
        pendingReasoningFirstAt = event.createdAt
      }
      continue
    }

    if (event.type === 'agent.tool_use') {
      if (event.toolCallId) {
        pendingToolStarts.set(event.toolCallId, event)
      }
      // Keep order — record the tool call now with status pending; we'll
      // patch the result/error/duration when the matching tool_result arrives.
      const rawName = event.toolName ?? event.content
      const { label, subject } = buildToolLabel(rawName, event.toolArguments)
      pendingToolCalls.push({
        toolCallId: event.toolCallId,
        toolName: rawName,
        label,
        subject,
        status: 'completed', // optimistic; downgraded if a failed result arrives
        input: event.toolArguments,
      })
      continue
    }

    if (event.type === 'agent.tool_result') {
      // Find the matching tool_use entry by toolCallId and patch it
      const match = pendingToolCalls.find(
        (t) => t.toolCallId && t.toolCallId === event.toolCallId,
      )
      if (match) {
        if (event.isError) {
          match.status = 'failed'
          match.error = event.content
        } else {
          match.output = event.content
        }
        const start = event.toolCallId
          ? pendingToolStarts.get(event.toolCallId)
          : undefined
        if (start) {
          match.durationMs = Math.max(0, event.createdAt - start.createdAt)
          if (event.toolCallId) pendingToolStarts.delete(event.toolCallId)
        }
      }
      continue
    }

    if (event.type !== 'user.message' && event.type !== 'agent.message') {
      continue
    }

    let text = event.content.trim()
    // Allow user messages with no text body when attachments are present —
    // the user can attach an image and rely on the model to describe it.
    if (!text) {
      if (event.type === 'user.message' && pendingAttachments.length > 0) {
        // fall through; the empty text is acceptable when paired with media
      } else {
        continue
      }
    }

    // Filter assistant heartbeats
    if (event.type === 'agent.message' && text.startsWith('HEARTBEAT')) continue

    // Filter internal reminders
    if (
      event.type === 'user.message' &&
      text.includes('Handle this reminder internally')
    ) {
      continue
    }

    // Extract actual user text from context-replay wrappers
    if (
      event.type === 'user.message' &&
      text.startsWith('[Chat messages since your last reply')
    ) {
      const marker = '[Current message - respond to this]'
      const index = text.indexOf(marker)
      if (index >= 0) {
        text = text
          .slice(index + marker.length)
          .trim()
          .replace(/^User:\s*/i, '')
      } else {
        // Reset all per-turn buffers — they belong to a discarded turn
        pendingToolCalls = []
        pendingToolStarts.clear()
        pendingReasoningTexts = []
        pendingReasoningFirstAt = null
        pendingAttachments = []
        continue
      }
      if (!text) continue
    }

    const item: BrowserOSChatHistoryItem = {
      id: `${sessionKey}:${seq}`,
      role: event.type === 'user.message' ? 'user' : 'assistant',
      text,
      timestamp: event.createdAt,
      messageSeq: seq,
      sessionKey,
      source,
    }

    if (event.type === 'agent.message') {
      // Pass through per-turn cost and token data
      if (event.costUsd) item.costUsd = event.costUsd
      if (event.tokensIn) item.tokensIn = event.tokensIn
      if (event.tokensOut) item.tokensOut = event.tokensOut

      // Attach any tool calls that happened before this assistant message
      if (pendingToolCalls.length > 0) {
        item.toolCalls = pendingToolCalls
        pendingToolCalls = []
        pendingToolStarts.clear()
      }

      // Attach accumulated thinking. Duration is from the first thinking
      // event to the final answer — the wall-clock time the user waited
      // through the model's reasoning loop.
      if (pendingReasoningTexts.length > 0) {
        const reasoning: BrowserOSChatHistoryReasoning = {
          text: pendingReasoningTexts.join('\n\n'),
        }
        if (pendingReasoningFirstAt != null) {
          reasoning.durationMs = Math.max(
            0,
            event.createdAt - pendingReasoningFirstAt,
          )
        }
        item.reasoning = reasoning
        pendingReasoningTexts = []
        pendingReasoningFirstAt = null
      }
    } else if (event.type === 'user.message') {
      // User messages reset all per-turn buffers — anything pending was
      // part of an earlier turn that had no final assistant message.
      pendingToolCalls = []
      pendingToolStarts.clear()
      pendingReasoningTexts = []
      pendingReasoningFirstAt = null

      // Flush accumulated attachments onto this user message.
      if (pendingAttachments.length > 0) {
        item.attachments = pendingAttachments
        pendingAttachments = []
      }
    }

    items.push(item)
    seq++
  }

  return items
}

function sumCostFromEvents(events: ClawEvent[]): number {
  let cost = 0
  for (const e of events) {
    if (e.type === 'agent.message' && e.costUsd) cost += e.costUsd
  }
  return cost
}

function encodeHistoryCursor(input: {
  sessionKey: string
  end: number
}): string {
  return Buffer.from(JSON.stringify(input), 'utf-8').toString('base64url')
}

function decodeHistoryCursor(
  cursor?: string,
): { sessionKey: string; end: number } | null {
  if (!cursor) return null
  try {
    const parsed = JSON.parse(
      Buffer.from(cursor, 'base64url').toString('utf-8'),
    ) as {
      sessionKey?: unknown
      end?: unknown
    }
    if (typeof parsed.sessionKey !== 'string') return null
    if (typeof parsed.end !== 'number' || !Number.isFinite(parsed.end)) {
      return null
    }
    return {
      sessionKey: parsed.sessionKey,
      end: Math.max(0, Math.trunc(parsed.end)),
    }
  } catch {
    return null
  }
}

export interface AgentOverview {
  agentId: string
  status: AgentLiveStatus
  latestMessage: string | null
  latestMessageAt: number | null
  activitySummary: string | null
  currentTool: string | null
  totalCostUsd: number
  sessionCount: number
}

export interface DashboardResponse {
  agents: AgentOverview[]
  summary: {
    totalAgents: number
    totalCostUsd: number
  }
}

export class OpenClawService {
  private runtime: ContainerRuntime
  private cliClient: OpenClawCliClient
  private bootstrapCliClient: OpenClawCliClient
  private httpClient: OpenClawHttpClient
  private openclawDir: string
  private hostPort = OPENCLAW_GATEWAY_CONTAINER_PORT
  private token: string
  private tokenLoaded = false
  private lastError: string | null = null
  private browserosServerPort: number
  private resourcesDir: string | null
  private browserosDir: string | undefined
  private vmCache: VmCacheRuntimeConfig | undefined
  private controlPlaneStatus: OpenClawControlPlaneStatus = 'disconnected'
  private lastGatewayError: string | null = null
  private lastRecoveryReason: OpenClawGatewayRecoveryReason | null = null
  private stopLogTail: (() => void) | null = null
  private lifecycleLock: Promise<void> = Promise.resolve()
  private clawSession = new ClawSession()
  private observer = new OpenClawObserver(this.clawSession)

  private _jsonlReader: OpenClawJsonlReader | null = null
  private get jsonlReader(): OpenClawJsonlReader {
    if (!this._jsonlReader) {
      this._jsonlReader = new OpenClawJsonlReader(
        getOpenClawStateDir(this.openclawDir),
      )
    }
    return this._jsonlReader
  }

  constructor(config: OpenClawServiceConfig = {}) {
    this.openclawDir = getOpenClawDir()
    this.runtime = buildContainerRuntime({
      resourcesDir: config.resourcesDir,
      projectDir: this.openclawDir,
      browserosRoot: config.browserosDir,
      vmCache: config.vmCache,
    })
    this.token = crypto.randomUUID()
    this.cliClient = new OpenClawCliClient(this.runtime)
    this.bootstrapCliClient = this.buildBootstrapCliClient()
    this.httpClient = new OpenClawHttpClient(
      this.hostPort,
      async () => this.token,
    )
    this.browserosServerPort =
      config.browserosServerPort ?? DEFAULT_PORTS.server
    this.resourcesDir = config.resourcesDir ?? null
    this.browserosDir = config.browserosDir
    this.vmCache = config.vmCache
  }

  configure(config: OpenClawServiceConfig): void {
    if (config.browserosServerPort !== undefined) {
      this.browserosServerPort = config.browserosServerPort
    }

    let runtimeChanged = false
    if (
      config.resourcesDir !== undefined &&
      config.resourcesDir !== this.resourcesDir
    ) {
      this.resourcesDir = config.resourcesDir
      runtimeChanged = true
    }
    if (
      config.browserosDir !== undefined &&
      config.browserosDir !== this.browserosDir
    ) {
      this.browserosDir = config.browserosDir
      runtimeChanged = true
    }
    if (
      config.vmCache !== undefined &&
      !sameVmCacheRuntimeConfig(config.vmCache, this.vmCache)
    ) {
      this.vmCache = config.vmCache
      runtimeChanged = true
    }
    if (runtimeChanged) {
      this.rebuildRuntimeClients()
    }
  }

  getPort(): number {
    return this.hostPort
  }

  /** Subscribe to real-time agent status changes from the ClawSession state machine. */
  onAgentStatusChange(
    listener: (agentId: string, state: AgentSessionState) => void,
  ): () => void {
    return this.clawSession.onStateChange(listener)
  }

  /** Read the current ClawSession state for an agent (read-only snapshot). */
  getAgentState(agentId: string): AgentSessionState {
    return this.clawSession.getState(agentId)
  }

  // ── Lifecycle ────────────────────────────────────────────────────────

  async setup(input: SetupInput, onLog?: (msg: string) => void): Promise<void> {
    return this.withLifecycleLock('setup', async () => {
      const logProgress = this.createProgressLogger(onLog)
      const provider = this.resolveProviderForAgent(input)
      logger.info('Starting OpenClaw setup', {
        hostPort: this.hostPort,
        browserosServerPort: this.browserosServerPort,
        providerType: input.providerType,
        providerName: input.providerName,
        hasBaseUrl: !!input.baseUrl,
        hasModel: !!input.modelId,
        hasApiKey: !!input.apiKey,
      })

      await this.runtime.ensureReady(logProgress)
      logProgress('Container runtime ready')

      await mkdir(this.openclawDir, { recursive: true })
      await mkdir(this.getStateDir(), { recursive: true })
      await mkdir(this.getHostWorkspaceDir('main'), { recursive: true })

      await this.ensureStateEnvFile()
      await this.writeStateEnv(provider.envValues)
      logger.info('Updated OpenClaw state env', {
        providerKeyCount: Object.keys(provider.envValues).length,
      })

      await this.refreshGatewayAuthToken()
      await this.ensureGatewayPortAllocated(logProgress)

      logProgress('Bootstrapping OpenClaw config...')
      await this.bootstrapCliClient.runOnboard({
        acceptRisk: true,
        authChoice: 'skip',
        gatewayAuth: 'token',
        gatewayBind: 'lan',
        gatewayPort: OPENCLAW_GATEWAY_CONTAINER_PORT,
        installDaemon: false,
        mode: 'local',
        nonInteractive: true,
        skipHealth: true,
      })
      await this.applyBrowserosConfig()
      await this.mergeProviderConfigIfChanged(provider)
      if (provider.model) {
        await this.bootstrapCliClient.setDefaultModel(provider.model)
      }

      logProgress('Validating OpenClaw config...')
      await this.assertConfigValid(this.bootstrapCliClient)

      await this.refreshGatewayAuthToken()

      logProgress('Starting OpenClaw gateway...')
      await this.runtime.startGateway(
        this.buildGatewayRuntimeSpec(),
        logProgress,
      )
      this.startGatewayLogTail()
      logProgress('Waiting for gateway readiness...')
      const ready = await this.runtime.waitForReady(
        this.hostPort,
        READY_TIMEOUT_MS,
      )
      if (!ready) {
        this.lastError = 'Gateway did not become ready within 30 seconds'
        const logs = await this.runtime.getGatewayLogs()
        logger.error('Gateway readiness check failed', { logs })
        throw new Error(this.lastError)
      }

      this.controlPlaneStatus = 'connecting'
      logProgress('Probing OpenClaw control plane...')
      await this.runControlPlaneCall(() => this.cliClient.probe())

      await this.ensureAllCliProvidersInstalled(logProgress)

      const existingAgents = await this.listAgents()
      logger.info('Fetched existing OpenClaw agents after setup', {
        count: existingAgents.length,
        names: existingAgents.map((agent) => agent.name),
      })
      if (existingAgents.some((agent) => agent.agentId === 'main')) {
        logProgress('Main agent detected')
      } else {
        logProgress('Creating main agent...')
        await this.runControlPlaneCall(() =>
          this.cliClient.createAgent({
            name: 'main',
            model: provider.model,
          }),
        )
      }

      this.lastError = null
      logProgress(
        `OpenClaw gateway running at http://127.0.0.1:${this.hostPort}`,
      )
      logger.info('OpenClaw setup complete', { hostPort: this.hostPort })
    })
  }

  async start(onLog?: (msg: string) => void): Promise<void> {
    return this.withLifecycleLock('start', async () => {
      const logProgress = this.createProgressLogger(onLog)
      logger.info('Starting OpenClaw service', {
        hostPort: this.hostPort,
      })

      await this.runtime.ensureReady(logProgress)

      logProgress('Refreshing gateway auth token...')
      await this.refreshGatewayAuthToken()
      await this.ensureStateEnvFile()

      await this.ensureGatewayPortAllocated(logProgress)

      if (await this.isGatewayAvailable(this.hostPort)) {
        this.startGatewayLogTail()
        this.controlPlaneStatus = 'connecting'
        logProgress('Probing OpenClaw control plane...')
        try {
          await this.runControlPlaneCall(() => this.cliClient.probe())
          this.lastError = null
          logger.info('OpenClaw gateway already running', {
            hostPort: this.hostPort,
          })
          return
        } catch (error) {
          logger.warn('OpenClaw control plane probe failed during start', {
            hostPort: this.hostPort,
            error: error instanceof Error ? error.message : String(error),
          })
        }
      }

      logProgress('Starting OpenClaw gateway...')
      await this.runtime.startGateway(
        this.buildGatewayRuntimeSpec(),
        logProgress,
      )
      this.startGatewayLogTail()

      logProgress('Waiting for gateway readiness...')
      const ready = await this.runtime.waitForReady(
        this.hostPort,
        READY_TIMEOUT_MS,
      )
      if (!ready) {
        this.lastError = 'Gateway did not become ready after start'
        throw new Error(this.lastError)
      }

      this.controlPlaneStatus = 'connecting'
      logProgress('Probing OpenClaw control plane...')
      await this.runControlPlaneCall(() => this.cliClient.probe())
      await this.ensureAllCliProvidersInstalled(logProgress)
      this.lastError = null
      logger.info('OpenClaw gateway started', { hostPort: this.hostPort })
    })
  }

  async stop(): Promise<void> {
    return this.withLifecycleLock('stop', async () => {
      logger.info('Stopping OpenClaw service', { hostPort: this.hostPort })
      this.controlPlaneStatus = 'disconnected'
      this.observer.disconnect()
      this.stopGatewayLogTail()
      await this.runtime.stopGateway()
      logger.info('OpenClaw container stopped')
    })
  }

  async restart(onLog?: (msg: string) => void): Promise<void> {
    return this.withLifecycleLock('restart', async () => {
      const logProgress = this.createProgressLogger(onLog)
      logger.info('Restarting OpenClaw service', {
        hostPort: this.hostPort,
      })

      this.controlPlaneStatus = 'reconnecting'
      await this.runtime.ensureReady(logProgress)
      this.stopGatewayLogTail()
      logProgress('Refreshing gateway auth token...')
      await this.refreshGatewayAuthToken()
      await this.ensureStateEnvFile()
      await this.ensureGatewayPortAllocated(logProgress)
      logProgress('Restarting OpenClaw gateway...')
      await this.runtime.restartGateway(
        this.buildGatewayRuntimeSpec(),
        logProgress,
      )
      this.startGatewayLogTail()

      logProgress('Waiting for gateway readiness...')
      const ready = await this.runtime.waitForReady(
        this.hostPort,
        READY_TIMEOUT_MS,
      )
      if (!ready) {
        this.lastError = 'Gateway did not become ready after restart'
        throw new Error(this.lastError)
      }

      logProgress('Probing OpenClaw control plane...')
      await this.runControlPlaneCall(() => this.cliClient.probe())
      await this.ensureAllCliProvidersInstalled(logProgress)
      this.lastError = null
      logProgress('Gateway restarted successfully')
      logger.info('OpenClaw gateway restarted', { hostPort: this.hostPort })
    })
  }

  async reconnectControlPlane(onLog?: (msg: string) => void): Promise<void> {
    return this.withLifecycleLock('reconnect', async () => {
      const logProgress = this.createProgressLogger(onLog)
      logger.info('Reconnecting OpenClaw control plane', {
        hostPort: this.hostPort,
      })

      logProgress('Checking gateway readiness...')
      const ready = await this.runtime.isReady(this.hostPort)
      if (!ready) {
        this.controlPlaneStatus = 'failed'
        this.lastGatewayError = 'OpenClaw gateway is not ready'
        this.lastRecoveryReason = 'container_not_ready'
        throw new Error('OpenClaw gateway is not ready')
      }

      logProgress('Reloading gateway auth token...')
      await this.refreshGatewayAuthToken()
      this.controlPlaneStatus = 'reconnecting'
      logProgress('Reconnecting control plane...')
      await this.runControlPlaneCall(() => this.cliClient.probe())
      logProgress('Control plane connected')
    })
  }

  async shutdown(): Promise<void> {
    this.controlPlaneStatus = 'disconnected'
    this.observer.disconnect()
    this.stopGatewayLogTail()
    try {
      await this.runtime.stopGateway()
    } catch {
      // Best effort during shutdown
    }
    await this.runtime.stopVm()
    logger.info('OpenClaw shutdown complete')
  }

  // ── Status ───────────────────────────────────────────────────────────

  async getStatus(): Promise<OpenClawStatusResponse> {
    const isSetUp = existsSync(this.getStateConfigPath())
    if (!isSetUp) {
      const machineStatus = await this.runtime.getMachineStatus()
      return {
        status: 'uninitialized',
        podmanAvailable: true,
        machineReady: machineStatus.running,
        port: null,
        agentCount: 0,
        error: null,
        controlPlaneStatus: 'disconnected',
        lastGatewayError: this.lastGatewayError,
        lastRecoveryReason: this.lastRecoveryReason,
      }
    }

    const machineStatus = await this.runtime.getMachineStatus()
    const ready = machineStatus.running
      ? await this.runtime.isReady(this.hostPort)
      : false

    let agentCount = 0
    if (ready) {
      try {
        const agents = await this.runControlPlaneCall(() =>
          this.cliClient.listAgents(),
        )
        agentCount = agents.length
      } catch {
        // latest control plane error is captured by runControlPlaneCall
      }
    }

    return {
      status: ready ? 'running' : this.lastError ? 'error' : 'stopped',
      podmanAvailable: true,
      machineReady: machineStatus.running,
      port: this.hostPort,
      agentCount,
      error: this.lastError,
      controlPlaneStatus: ready ? this.controlPlaneStatus : 'disconnected',
      lastGatewayError: this.lastGatewayError,
      lastRecoveryReason: this.lastRecoveryReason,
    }
  }

  // ── Agent Management (via CLI) ──────────────────────────────────────

  async createAgent(input: {
    name: string
    providerType?: string
    providerName?: string
    baseUrl?: string
    apiKey?: string
    modelId?: string
    supportsImages?: boolean
  }): Promise<OpenClawAgentEntry> {
    const { name } = input
    if (!AGENT_NAME_PATTERN.test(name)) {
      throw new OpenClawInvalidAgentNameError()
    }

    logger.debug('Creating OpenClaw agent', {
      name,
      providerType: input.providerType,
      providerName: input.providerName,
      hasBaseUrl: !!input.baseUrl,
      hasModel: !!input.modelId,
      hasApiKey: !!input.apiKey,
      supportsImages: !!input.supportsImages,
    })
    await this.assertGatewayReady()

    const provider = this.resolveProviderForAgent(input)
    const configChanged = await this.mergeProviderConfigIfChanged(provider)
    const keysChanged = await this.writeStateEnv(provider.envValues)

    if (configChanged || keysChanged) {
      logger.info('OpenClaw provider config changed while creating agent', {
        name,
        configChanged,
        keysChanged,
      })
      await this.restart()
    }

    const model = provider.model
    let agent: OpenClawAgentRecord
    try {
      agent = await this.runControlPlaneCall(() =>
        this.cliClient.createAgent({
          name,
          model,
        }),
      )
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (message.includes('already exists')) {
        throw new OpenClawAgentAlreadyExistsError(name)
      }
      throw error
    }

    logger.info('Agent created via CLI', {
      agentId: agent.agentId,
      providerType: input.providerType,
    })
    return agent
  }

  async removeAgent(agentId: string): Promise<void> {
    logger.info('Removing OpenClaw agent', { agentId })
    if (agentId === 'main') {
      throw new OpenClawProtectedAgentError('Cannot delete the main agent')
    }

    await this.assertGatewayReady()
    try {
      await this.runControlPlaneCall(() => this.cliClient.deleteAgent(agentId))
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (message.includes('not found')) {
        throw new OpenClawAgentNotFoundError(agentId)
      }
      throw error
    }
    logger.info('Agent removed via CLI', { agentId })
  }

  async listAgents(): Promise<OpenClawAgentEntry[]> {
    await this.assertGatewayReady()
    logger.debug('Listing OpenClaw agents')
    return this.runControlPlaneCall(() => this.cliClient.listAgents())
  }

  listSessions(agentId?: string): BrowserOSOpenClawSession[] {
    logger.debug('Listing OpenClaw sessions', { agentId })

    const agentIds = agentId ? [agentId] : this.jsonlReader.listAgents()

    const sessions: BrowserOSOpenClawSession[] = []
    for (const id of agentIds) {
      for (const entry of this.jsonlReader.listSessions(id)) {
        sessions.push({
          key: entry.key,
          updatedAt: entry.updatedAt,
          sessionId: entry.sessionId,
          agentId: id,
          kind: 'chat',
          source: classifySessionSource(entry.key),
        })
      }
    }
    return sessions.sort((a, b) => b.updatedAt - a.updatedAt)
  }

  resolveAgentSession(agentId: string): BrowserOSOpenClawAgentSessionResponse {
    const sessions = this.listSessions(agentId)
    const session =
      sessions.find((entry) => entry.source === 'user-chat') ??
      sessions.find((entry) => entry.kind.toLowerCase().includes('chat')) ??
      sessions[0] ??
      null

    if (session) {
      return this.resolveSpecificAgentSession(agentId, session.key)
    }

    return {
      agentId,
      exists: false,
      sessionKey: null,
      session: null,
    }
  }

  getAgentHistoryPage(
    agentId: string,
    input: HistoryPageInput = {},
  ): BrowserOSOpenClawHistoryPageResponse {
    const limit = normalizeHistoryLimit(input.limit)
    const cursor = decodeHistoryCursor(input.cursor)
    const resolved = cursor?.sessionKey
      ? this.resolveSpecificAgentSession(agentId, cursor.sessionKey)
      : input.sessionKey
        ? this.resolveSpecificAgentSession(agentId, input.sessionKey)
        : this.resolveAgentSession(agentId)

    const session = resolved.session
    if (!session) {
      return {
        agentId,
        sessionKey: null,
        session: null,
        items: [],
        page: { hasMore: false, limit },
      }
    }

    const sessionKey =
      resolved.sessionKey ??
      normalizeBrowserOSChatSessionKey(agentId, session.key)

    // Read JSONL directly from the host filesystem via Lima virtiofs mount
    const events = this.jsonlReader.listBySession(agentId, session.key)
    const items = jsonlEventsToHistoryItems(events, sessionKey, session.source)

    const end = Math.min(cursor?.end ?? items.length, items.length)
    const start = Math.max(0, end - limit)
    const pageItems = items.slice(start, end)
    const nextCursor =
      start > 0 ? encodeHistoryCursor({ sessionKey, end: start }) : undefined

    return {
      agentId,
      sessionKey,
      session,
      items: pageItems,
      page: {
        cursor: nextCursor,
        hasMore: start > 0,
        limit,
      },
    }
  }

  // ── Dashboard ──────────────────────────────────────────────────────

  getDashboard(): DashboardResponse {
    const agentIds = this.jsonlReader.listAgents()
    const agentOverviews: AgentOverview[] = []
    let totalCostUsd = 0

    for (const agentId of agentIds) {
      const liveStatus = this.clawSession.getState(agentId)
      const sessions = this.jsonlReader.listSessions(agentId)

      if (sessions.length === 0) {
        agentOverviews.push({
          agentId,
          status: liveStatus.status,
          latestMessage: null,
          latestMessageAt: null,
          activitySummary: null,
          currentTool: liveStatus.currentTool,
          totalCostUsd: 0,
          sessionCount: 0,
        })
        continue
      }

      const latestSession = sessions[0]
      // Read the latest session's JSONL once and derive everything from
      // the loaded events array — avoids re-reading the same file for
      // latestAgentMessage() and getSessionStats() individually.
      const events = this.jsonlReader.listBySession(agentId, latestSession.key)

      let latestMsg: ClawEvent | undefined
      for (let i = events.length - 1; i >= 0; i--) {
        if (events[i]?.type === 'agent.message') {
          latestMsg = events[i]
          break
        }
      }

      // Accumulate cost: derive from the already-loaded events for the
      // latest session, read remaining sessions separately.
      let agentCost = sumCostFromEvents(events)
      for (let i = 1; i < sessions.length; i++) {
        const stats = this.jsonlReader.getSessionStats(
          agentId,
          sessions[i]!.key,
        )
        agentCost += stats.totalCostUsd
      }
      totalCostUsd += agentCost

      agentOverviews.push({
        agentId,
        status: liveStatus.status,
        latestMessage: latestMsg?.content?.slice(0, 200) ?? null,
        latestMessageAt: latestMsg?.createdAt ?? latestSession.updatedAt,
        activitySummary: summarizeToolActivity(events),
        currentTool: liveStatus.currentTool,
        totalCostUsd: agentCost,
        sessionCount: sessions.length,
      })
    }

    return {
      agents: agentOverviews,
      summary: { totalAgents: agentIds.length, totalCostUsd },
    }
  }

  // ── Chat Stream (HTTP) ───────────────────────────────────────────────

  async chatStream(
    agentId: string,
    sessionKey: string,
    message: string,
    history: MonitoringChatTurn[] = [],
    options: {
      messageParts?: OpenClawChatContentPart[]
      signal?: AbortSignal
    } = {},
  ): Promise<ReadableStream<OpenClawStreamEvent>> {
    await this.assertGatewayReady()
    const normalizedSessionKey = normalizeBrowserOSChatSessionKey(
      agentId,
      sessionKey,
    )
    logger.info('Starting OpenClaw chat stream', {
      agentId,
      sessionKey: normalizedSessionKey,
      messageLength: message.length,
      historyLength: history.length,
      contentPartCount: options.messageParts?.length ?? 0,
    })
    return this.runControlPlaneCall(() =>
      this.httpClient.streamChat({
        agentId,
        sessionKey: normalizedSessionKey,
        message,
        messageParts: options.messageParts,
        history,
        signal: options.signal,
      }),
    )
  }

  private resolveSpecificAgentSession(
    agentId: string,
    sessionKey: string,
  ): BrowserOSOpenClawAgentSessionResponse {
    const normalizedSessionKey = normalizeBrowserOSChatSessionKey(
      agentId,
      sessionKey,
    )
    const canonicalSessionKey = toOpenClawBrowserOSSessionKey(
      agentId,
      normalizedSessionKey,
    )
    const sessions = this.listSessions(agentId)
    const session =
      sessions.find((entry) => entry.key === canonicalSessionKey) ??
      sessions.find((entry) => entry.key === sessionKey) ??
      sessions.find(
        (entry) =>
          normalizeBrowserOSChatSessionKey(agentId, entry.key) ===
          normalizedSessionKey,
      )

    if (!session) {
      return {
        agentId,
        exists: false,
        sessionKey: normalizedSessionKey,
        session: null,
      }
    }

    return {
      agentId,
      exists: true,
      sessionKey: normalizedSessionKey,
      session,
    }
  }

  // ── Session History (HTTP) ───────────────────────────────────────────

  async getSessionHistory(
    sessionKey: string,
    input: { limit?: number; cursor?: string; signal?: AbortSignal } = {},
  ): Promise<OpenClawSessionHistory> {
    await this.assertGatewayReady()
    return this.runControlPlaneCall(() =>
      this.httpClient.getSessionHistory(sessionKey, input),
    )
  }

  async streamSessionHistory(
    sessionKey: string,
    input: { limit?: number; cursor?: string; signal?: AbortSignal } = {},
  ): Promise<ReadableStream<OpenClawSessionHistoryEvent>> {
    await this.assertGatewayReady()
    return this.runControlPlaneCall(() =>
      this.httpClient.streamSessionHistory(sessionKey, input),
    )
  }

  // ── Provider Keys ────────────────────────────────────────────────────
  async updateProviderKeys(input: {
    providerType: string
    providerName?: string
    baseUrl?: string
    apiKey: string
    modelId?: string
  }): Promise<OpenClawProviderUpdateResult> {
    const provider = this.resolveProviderForAgent(input)
    const configChanged = await this.mergeProviderConfigIfChanged(provider)
    const envChanged = await this.writeStateEnv(provider.envValues)
    const restarted = configChanged || envChanged
    if (restarted) {
      await this.restart()
    }
    if (provider.model) {
      const model = provider.model
      await this.applyCliMutation(() => this.cliClient.setDefaultModel(model))
    }
    logger.info('Provider keys updated', {
      providerType: input.providerType,
      modelUpdated: !!provider.model,
      restarted,
    })
    return {
      restarted,
      modelUpdated: !!provider.model,
    }
  }

  // ── CLI-backed Providers ─────────────────────────────────────────────

  async getCliProviderAuthStatus(
    provider: OpenClawCliProvider,
  ): Promise<OpenClawCliProviderAuthStatus> {
    const { stdout, exitCode } = await this.runtime.runInContainer(
      provider.authStatusCommand,
    )
    return provider.parseAuthStatus(stdout, exitCode)
  }

  // ── Logs ─────────────────────────────────────────────────────────────

  async getLogs(tail = 100): Promise<string[]> {
    logger.debug('Fetching OpenClaw container logs', { tail })
    return this.runtime.getGatewayLogs(tail)
  }

  // ── Auto-start on BrowserOS boot ────────────────────────────────────

  async tryAutoStart(): Promise<void> {
    return this.withLifecycleLock('auto-start', async () => {
      const isSetUp = existsSync(this.getStateConfigPath())
      if (!isSetUp) return

      logger.info('Attempting OpenClaw auto-start', {
        hostPort: this.hostPort,
      })

      try {
        await this.runtime.ensureReady()

        await this.refreshGatewayAuthToken()
        await this.ensureStateEnvFile()

        const persistedPort = await readPersistedGatewayPort(this.openclawDir)
        if (persistedPort !== null) {
          this.setPort(persistedPort)
        }

        if (!(await this.isGatewayAvailable(this.hostPort))) {
          await this.ensureGatewayPortAllocated()
          await this.runtime.startGateway(this.buildGatewayRuntimeSpec())
          const ready = await this.runtime.waitForReady(
            this.hostPort,
            READY_TIMEOUT_MS,
          )
          if (!ready) {
            logger.warn('OpenClaw gateway failed to become ready on auto-start')
            return
          }
        }

        await this.runControlPlaneCall(() => this.cliClient.probe())
        await this.ensureAllCliProvidersInstalled()
        logger.info('OpenClaw gateway auto-started')
      } catch (err) {
        logger.warn('OpenClaw auto-start failed', {
          error: err instanceof Error ? err.message : String(err),
        })
      }
    })
  }

  // ── Internal ─────────────────────────────────────────────────────────

  // CLI-provider short-circuit: skip env writes and custom-provider merges,
  // just build the `<id>/<model>` ref that OpenClaw's own plugin routes to.
  private resolveProviderForAgent(
    input: SetupInput,
  ): ResolvedOpenClawProviderConfig {
    const cliProvider = input.providerType
      ? getOpenClawCliProvider(input.providerType)
      : undefined
    if (cliProvider) {
      return {
        envValues: {},
        model: input.modelId
          ? buildOpenClawCliProviderModelRef(cliProvider.id, input.modelId)
          : undefined,
      }
    }
    return resolveSupportedOpenClawProvider(input)
  }

  private async ensureAllCliProvidersInstalled(
    onLog?: (msg: string) => void,
  ): Promise<void> {
    // Test mocks may swap `this.runtime` for a partial stub without
    // execInContainer. Skip silently — production ContainerRuntime always
    // provides it.
    if (typeof this.runtime.execInContainer !== 'function') return
    for (const provider of OPENCLAW_CLI_PROVIDERS) {
      await this.ensureCliProviderInstalled(provider, onLog)
    }
  }

  private async ensureCliProviderInstalled(
    provider: OpenClawCliProvider,
    onLog?: (msg: string) => void,
  ): Promise<void> {
    // argv probe — no shell, no interpolation: `which` returns 0 if the
    // binary is on PATH in the container, non-zero otherwise.
    const probe = await this.runtime.execInContainer(['which', provider.binary])
    if (probe === 0) {
      logger.info('CLI-backed provider already present', {
        providerId: provider.id,
      })
      return
    }

    // argv install — registry values flow straight through nerdctl exec,
    // never through a shell. Version is pinned in the provider registry.
    const lines: string[] = []
    const exitCode = await this.runtime.execInContainer(
      [
        'npm',
        'install',
        '-g',
        `${provider.npmPackage}@${provider.npmPackageVersion}`,
      ],
      (line) => {
        lines.push(line)
        onLog?.(line)
      },
    )
    if (exitCode !== 0) {
      logger.warn('CLI-backed provider install failed', {
        providerId: provider.id,
        exitCode,
        tail: lines.slice(-5),
      })
      return
    }
    logger.info('CLI-backed provider installed', { providerId: provider.id })
  }

  private buildBootstrapCliClient(): OpenClawCliClient {
    return new OpenClawCliClient({
      execInContainer: (command, onLog) =>
        this.runtime.runGatewaySetupCommand(
          command,
          this.buildGatewayRuntimeSpec(),
          onLog,
        ),
    })
  }

  private rebuildRuntimeClients(): void {
    this.stopGatewayLogTail()
    this.runtime = buildContainerRuntime({
      resourcesDir: this.resourcesDir ?? undefined,
      projectDir: this.openclawDir,
      browserosRoot: this.browserosDir,
      vmCache: this.vmCache,
    })
    this.cliClient = new OpenClawCliClient(this.runtime)
    this.bootstrapCliClient = this.buildBootstrapCliClient()
    this._jsonlReader = null
  }

  private setPort(hostPort: number): void {
    if (hostPort === this.hostPort) return
    this.hostPort = hostPort
    this.httpClient = new OpenClawHttpClient(
      this.hostPort,
      async () => this.token,
    )
  }

  private async ensureGatewayPortAllocated(
    logProgress?: (msg: string) => void,
  ): Promise<void> {
    const persistedPort = await readPersistedGatewayPort(this.openclawDir)
    if (persistedPort !== null) {
      this.setPort(persistedPort)
    }
    if (await this.isGatewayAvailable(this.hostPort)) {
      return
    }
    const hostPort = await allocateGatewayPort(this.openclawDir)
    if (hostPort !== this.hostPort) {
      logProgress?.(`Allocated OpenClaw gateway host port ${hostPort}`)
      logger.info('Allocated OpenClaw gateway host port', { hostPort })
      this.setPort(hostPort)
    }
  }

  private async isGatewayAvailable(hostPort: number): Promise<boolean> {
    if (!(await this.isGatewayPortReady(hostPort))) return false

    if (!this.tokenLoaded) {
      logger.debug(
        'OpenClaw gateway port is ready before auth token is loaded',
        {
          hostPort,
        },
      )
      return false
    }

    const client =
      hostPort === this.hostPort
        ? this.httpClient
        : new OpenClawHttpClient(hostPort, async () => this.token)
    const authenticated = await client.isAuthenticated()
    if (!authenticated) {
      logger.warn('OpenClaw gateway port rejected current auth token', {
        hostPort,
      })
    }
    return authenticated
  }

  private async isGatewayPortReady(hostPort: number): Promise<boolean> {
    if (await this.runtime.isReady(hostPort)) return true

    const runtime = this.runtime as {
      isHealthy?: (port: number) => Promise<boolean>
    }
    if (runtime.isHealthy) {
      return runtime.isHealthy(hostPort)
    }
    return false
  }

  private async assertGatewayReady(): Promise<void> {
    const portReady = await this.runtime.isReady(this.hostPort)
    logger.debug('Checking OpenClaw gateway readiness before use', {
      hostPort: this.hostPort,
      portReady,
      controlPlaneStatus: this.controlPlaneStatus,
    })
    if (portReady) {
      return
    }

    this.controlPlaneStatus = 'failed'
    this.lastGatewayError = 'OpenClaw gateway is not ready'
    this.lastRecoveryReason = 'container_not_ready'
    throw new Error('OpenClaw gateway is not ready')
  }

  private async runControlPlaneCall<T>(fn: () => Promise<T>): Promise<T> {
    try {
      await this.ensureTokenLoaded()
      const result = await fn()
      this.controlPlaneStatus = 'connected'
      this.lastGatewayError = null
      this.lastRecoveryReason = null
      this.ensureObserverConnected()
      return result
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      const reason = this.classifyControlPlaneError(error)
      this.controlPlaneStatus = 'failed'
      this.lastGatewayError = message
      this.lastRecoveryReason = reason
      throw error
    }
  }

  private ensureObserverConnected(): void {
    // Seed the ClawSession state machine from JSONL on first control plane
    // call. This gives every agent a correct initial status (working/idle)
    // before the WS observer has seen any events.
    if (!this.clawSession.isSeeded()) {
      this.clawSession.seedFromJsonl(this.jsonlReader)
    }

    if (this.observer.isConnected()) return
    const url = `http://127.0.0.1:${this.hostPort}`
    this.observer.connect(url, this.token)
  }

  private classifyControlPlaneError(
    error: unknown,
  ): OpenClawGatewayRecoveryReason {
    const message = error instanceof Error ? error.message : String(error)
    if (message.includes('Unauthorized')) return 'token_mismatch'
    if (message.includes('token')) return 'token_mismatch'
    if (message.includes('not ready')) return 'container_not_ready'
    return 'unknown'
  }

  private startGatewayLogTail(): void {
    if (process.env.NODE_ENV !== 'development') return
    if (this.stopLogTail) return
    try {
      this.stopLogTail = this.runtime.tailGatewayLogs((line) => {
        logger.debug(line)
      })
      logger.info('Streaming OpenClaw gateway logs into server log (dev mode)')
    } catch (err) {
      logger.warn('Failed to start OpenClaw gateway log tail', {
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  private stopGatewayLogTail(): void {
    if (!this.stopLogTail) return
    try {
      this.stopLogTail()
    } catch {
      // best effort
    }
    this.stopLogTail = null
  }

  private getHostWorkspaceDir(agentName: string): string {
    return getHostWorkspaceDir(this.openclawDir, agentName)
  }

  private getStateConfigPath(): string {
    return getOpenClawStateConfigPath(this.openclawDir)
  }

  private getStateDir(): string {
    return getOpenClawStateDir(this.openclawDir)
  }

  private getStateEnvPath(): string {
    return getOpenClawStateEnvPath(this.openclawDir)
  }

  private async applyBrowserosConfig(): Promise<void> {
    await this.bootstrapCliClient.setConfigBatch(this.getBrowserosConfigBatch())
  }

  private getBrowserosConfigBatch(): OpenClawConfigBatchEntry[] {
    const entries: OpenClawConfigBatchEntry[] = [
      {
        path: 'agents.defaults.workspace',
        value: `${OPENCLAW_CONTAINER_HOME}/workspace`,
      },
      {
        path: 'agents.defaults.thinkingDefault',
        value: 'off',
      },
      {
        path: 'gateway.controlUi.allowInsecureAuth',
        value: true,
      },
      {
        path: 'gateway.controlUi.dangerouslyDisableDeviceAuth',
        value: true,
      },
      {
        path: 'gateway.controlUi.allowedOrigins',
        value: [
          `http://127.0.0.1:${this.hostPort}`,
          `http://localhost:${this.hostPort}`,
        ],
      },
      {
        path: 'gateway.http.endpoints.chatCompletions.enabled',
        value: true,
      },
      {
        path: 'tools.profile',
        value: 'full',
      },
      {
        path: 'tools.web.search.provider',
        value: 'duckduckgo',
      },
      {
        path: 'tools.web.search.enabled',
        value: true,
      },
      {
        path: 'tools.exec.host',
        value: 'gateway',
      },
      {
        path: 'tools.exec.security',
        value: 'full',
      },
      {
        path: 'tools.exec.ask',
        value: 'off',
      },
      {
        path: 'cron.enabled',
        value: true,
      },
      {
        path: 'hooks.internal.enabled',
        value: true,
      },
      {
        path: 'mcp.servers.browseros.url',
        value: `http://host.containers.internal:${this.browserosServerPort}/mcp`,
      },
      {
        path: 'mcp.servers.browseros.transport',
        value: 'streamable-http',
      },
      {
        path: 'approvals.exec.enabled',
        value: false,
      },
      {
        path: 'skills.install.nodeManager',
        value: 'npm',
      },
      {
        path: 'agents.defaults.memorySearch.enabled',
        value: false,
      },
      {
        // Enable OpenClaw's image-understanding pipeline so models that
        // declare `input: ['text', 'image']` actually receive image bytes
        // instead of having them stripped at the gateway. Without this,
        // image_url content parts are silently dropped even if the model
        // and provider both support vision. Per-model `input` still gates
        // which models see images — this just turns the global pipeline on.
        path: 'tools.media.image.enabled',
        value: true,
      },
    ]

    if (process.env.NODE_ENV === 'development') {
      entries.push(
        {
          path: 'logging.level',
          value: 'debug',
        },
        {
          path: 'logging.consoleLevel',
          value: 'debug',
        },
      )
    }

    return entries
  }

  private async applyCliMutation(action: () => Promise<void>): Promise<void> {
    let retried = false

    while (true) {
      try {
        await action()
        await this.waitForGatewayAfterCliMutation()
        return
      } catch (error) {
        if (!this.isRestartInterruptedCliMutation(error) || retried) {
          throw error
        }

        logger.info(
          'Retrying OpenClaw CLI mutation after gateway reload interrupted the command',
          {
            error: error instanceof Error ? error.message : String(error),
          },
        )
        await this.waitForGatewayAfterCliMutation()
        retried = true
      }
    }
  }

  private isRestartInterruptedCliMutation(error: unknown): boolean {
    const message = error instanceof Error ? error.message : String(error)
    return (
      message.includes('Config overwrite:') && message.includes('openclaw.json')
    )
  }

  private async waitForGatewayAfterCliMutation(): Promise<void> {
    const ready = await this.runtime.waitForReady(
      this.hostPort,
      READY_TIMEOUT_MS,
    )
    if (!ready) {
      this.lastError = 'Gateway did not become ready after applying config'
      throw new Error(this.lastError)
    }
  }

  private async assertConfigValid(
    client: OpenClawCliClient = this.cliClient,
  ): Promise<void> {
    const validation = await client.validateConfig()
    if (
      validation &&
      typeof validation === 'object' &&
      'ok' in validation &&
      validation.ok === false
    ) {
      throw new Error('OpenClaw config validation failed')
    }
  }

  private async ensureStateEnvFile(): Promise<void> {
    const envPath = this.getStateEnvPath()
    if (existsSync(envPath)) return
    await mkdir(this.getStateDir(), { recursive: true })
    await writeFile(envPath, '', { mode: 0o600 })
  }

  // Pin away from latest because newer OpenClaw releases regress OpenRouter chat streams.
  private getGatewayImage(): string {
    return process.env.OPENCLAW_IMAGE || 'ghcr.io/openclaw/openclaw:2026.4.12'
  }

  private buildGatewayRuntimeSpec(): GatewayContainerSpec {
    return {
      image: this.getGatewayImage(),
      hostPort: this.hostPort,
      hostHome: this.openclawDir,
      envFilePath: this.getStateEnvPath(),
      gatewayToken: this.tokenLoaded ? this.token : undefined,
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    }
  }

  private async writeStateEnv(
    values: Record<string, string>,
  ): Promise<boolean> {
    if (Object.keys(values).length === 0) return false

    const envPath = this.getStateEnvPath()
    let content = ''
    try {
      content = await readFile(envPath, 'utf-8')
    } catch {
      // state env may not exist yet
    }

    const next = mergeEnvContent(content, values)
    if (!next.changed) return false

    await mkdir(this.getStateDir(), { recursive: true })
    await writeFile(envPath, next.content, { mode: 0o600 })
    logger.debug('Updated OpenClaw provider credentials', {
      keys: Object.keys(values),
    })
    return true
  }

  private async mergeProviderConfigIfChanged(
    provider: ResolvedOpenClawProviderConfig,
  ): Promise<boolean> {
    if (!provider.customProvider) {
      return false
    }

    const configPath = this.getStateConfigPath()
    const content = await readFile(configPath, 'utf-8')
    const config = JSON.parse(content) as Record<string, unknown>
    const models =
      config.models && typeof config.models === 'object'
        ? (config.models as Record<string, unknown>)
        : {}
    const providers =
      models.providers && typeof models.providers === 'object'
        ? (models.providers as Record<string, Record<string, unknown>>)
        : {}
    const existingProvider = providers[provider.customProvider.providerId] ?? {}
    const existingModels = Array.isArray(existingProvider.models)
      ? (existingProvider.models as Array<Record<string, unknown>>)
      : []
    const desiredModelEntry =
      Array.isArray(provider.customProvider.config.models) &&
      provider.customProvider.config.models.length > 0
        ? (provider.customProvider.config.models[0] as Record<string, unknown>)
        : null
    const hasDesiredModel = desiredModelEntry
      ? existingModels.some(
          (model) =>
            model.id === desiredModelEntry.id ||
            model.name === desiredModelEntry.name,
        )
      : true
    const mergedModels =
      desiredModelEntry && !hasDesiredModel
        ? [...existingModels, desiredModelEntry]
        : existingModels.length > 0
          ? existingModels
          : Array.isArray(provider.customProvider.config.models)
            ? provider.customProvider.config.models
            : undefined

    const nextProvider: Record<string, unknown> = {
      ...existingProvider,
      ...provider.customProvider.config,
      ...(mergedModels ? { models: mergedModels } : {}),
    }
    const nextModels: Record<string, unknown> = {
      ...models,
      mode: 'merge',
      providers: {
        ...providers,
        [provider.customProvider.providerId]: nextProvider,
      },
    }
    const nextConfig: Record<string, unknown> = {
      ...config,
      models: nextModels,
    }

    if (JSON.stringify(config) === JSON.stringify(nextConfig)) {
      return false
    }

    await writeFile(
      configPath,
      `${JSON.stringify(nextConfig, null, 2)}\n`,
      'utf-8',
    )
    logger.debug('Updated OpenClaw custom provider config', {
      providerId: provider.customProvider.providerId,
    })
    return true
  }

  private async ensureTokenLoaded(): Promise<void> {
    if (this.tokenLoaded) {
      return
    }
    if (!existsSync(this.getStateConfigPath())) {
      return
    }

    await this.loadTokenFromConfig()
  }

  private async refreshGatewayAuthToken(): Promise<void> {
    this.tokenLoaded = false
    if (!existsSync(this.getStateConfigPath())) {
      return
    }

    await this.loadTokenFromConfig()
  }

  private async loadTokenFromConfig(): Promise<void> {
    try {
      const config = JSON.parse(
        await readFile(this.getStateConfigPath(), 'utf-8'),
      ) as {
        gateway?: {
          auth?: {
            token?: unknown
          }
        }
      }
      const token = config.gateway?.auth?.token
      if (typeof token === 'string' && token) {
        this.token = token
        this.tokenLoaded = true
        logger.info('Loaded OpenClaw gateway token from mounted config')
      }
    } catch (err) {
      logger.warn('Failed to load OpenClaw gateway token from mounted config', {
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  private createProgressLogger(
    onLog?: (msg: string) => void,
  ): (msg: string) => void {
    return (msg) => {
      logger.debug(`OpenClaw: ${msg}`)
      onLog?.(msg)
    }
  }

  private async withLifecycleLock<T>(
    operation: string,
    fn: () => Promise<T>,
  ): Promise<T> {
    const previous = this.lifecycleLock
    let release!: () => void
    this.lifecycleLock = new Promise<void>((resolve) => {
      release = resolve
    })
    await previous.catch(() => undefined)
    try {
      logger.debug('OpenClaw lifecycle operation started', { operation })
      return await fn()
    } finally {
      release()
    }
  }
}

let service: OpenClawService | null = null

export function configureOpenClawService(
  config: OpenClawServiceConfig,
): OpenClawService {
  if (!service) {
    service = new OpenClawService(config)
    return service
  }

  service.configure(config)
  return service
}

export function configureVmRuntime(config: {
  resourcesDir?: string
  browserosDir?: string
  vmCache?: VmCacheRuntimeConfig
}): OpenClawService {
  return configureOpenClawService(config)
}

export function getOpenClawService(): OpenClawService {
  if (!service) service = new OpenClawService()
  return service
}

function sameVmCacheRuntimeConfig(
  left: VmCacheRuntimeConfig | undefined,
  right: VmCacheRuntimeConfig | undefined,
): boolean {
  return (
    left?.manifestUrl === right?.manifestUrl &&
    left?.ensureAvailable === right?.ensureAvailable &&
    left?.ensureSynced === right?.ensureSynced
  )
}
