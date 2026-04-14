/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Main orchestrator for OpenClaw integration.
 * Container lifecycle via Podman, agent CRUD via Gateway WS RPC,
 * chat via HTTP /v1/chat/completions proxy.
 */

import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { OPENCLAW_GATEWAY_PORT } from '@browseros/shared/constants/openclaw'
import { DEFAULT_PORTS } from '@browseros/shared/constants/ports'
import { getOpenClawDir } from '../../../lib/browseros-dir'
import { logger } from '../../../lib/logger'
import { ContainerRuntime } from './container-runtime'
import {
  OpenClawAgentAlreadyExistsError,
  OpenClawAgentNotFoundError,
  OpenClawInvalidAgentNameError,
  OpenClawProtectedAgentError,
} from './errors'
import {
  ensureClientIdentity,
  type GatewayAgentEntry,
  GatewayClient,
  type OpenClawStreamEvent,
} from './gateway-client'
import {
  buildBootstrapConfig,
  buildEnvFile,
  deriveOpenClawApiKeyEnvVar,
  deriveOpenClawProviderId,
  PROVIDER_ENV_MAP,
  resolveProviderKeys,
  resolveProviderModel,
} from './openclaw-config'
import { getPodmanRuntime } from './podman-runtime'

const COMPOSE_RESOURCE = resolve(
  import.meta.dir,
  '../../../../resources/openclaw-compose.yml',
)
const OPENCLAW_CONFIG_FILE = 'openclaw.json'
const READY_TIMEOUT_MS = 30_000
const AGENT_NAME_PATTERN = /^[a-z][a-z0-9-]*$/

export type OpenClawControlPlaneStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'recovering'
  | 'failed'

export type OpenClawGatewayRecoveryReason =
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

export interface SetupInput {
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
}

export class OpenClawService {
  private runtime: ContainerRuntime
  private gateway: GatewayClient | null = null
  private openclawDir: string
  private port = OPENCLAW_GATEWAY_PORT
  private token: string
  private lastError: string | null = null
  private browserosServerPort: number
  private controlPlaneStatus: OpenClawControlPlaneStatus = 'disconnected'
  private lastGatewayError: string | null = null
  private lastRecoveryReason: OpenClawGatewayRecoveryReason | null = null
  private gatewayReconnectPromise: Promise<void> | null = null

  constructor(browserosServerPort?: number) {
    this.openclawDir = getOpenClawDir()
    this.runtime = new ContainerRuntime(getPodmanRuntime(), this.openclawDir)
    this.token = crypto.randomUUID()
    this.browserosServerPort = browserosServerPort ?? DEFAULT_PORTS.server
  }

  // ── Lifecycle ────────────────────────────────────────────────────────

  async setup(input: SetupInput, onLog?: (msg: string) => void): Promise<void> {
    const logProgress = this.createProgressLogger(onLog)

    logProgress('Checking container runtime...')
    const available = await this.runtime.isPodmanAvailable()
    if (!available) {
      throw new Error(
        'Podman is not available. Install Podman to use OpenClaw agents.',
      )
    }

    await this.runtime.ensureReady(logProgress)
    logProgress('Container runtime ready')

    await mkdir(this.openclawDir, { recursive: true })
    await mkdir(join(this.openclawDir, 'workspace'), { recursive: true })

    logProgress('Copying compose file...')
    await this.runtime.copyComposeFile(COMPOSE_RESOURCE)

    this.token = crypto.randomUUID()
    const providerKeys = resolveProviderKeys(input)
    const envContent = buildEnvFile({
      token: this.token,
      configDir: this.openclawDir,
      providerKeys,
    })
    await this.runtime.writeEnvFile(envContent)
    logProgress('Generated .env file')

    const config = buildBootstrapConfig({
      gatewayPort: this.port,
      gatewayToken: this.token,
      browserosServerPort: this.browserosServerPort,
      providerType: input.providerType,
      providerName: input.providerName,
      baseUrl: input.baseUrl,
      modelId: input.modelId,
    })
    await this.writeBootstrapConfig(config)
    logProgress('Generated openclaw.json')

    logProgress('Pulling OpenClaw image...')
    await this.runtime.composePull(logProgress)
    logProgress('Image ready')

    logProgress('Starting OpenClaw gateway...')
    await this.runtime.composeUp(logProgress)

    logProgress('Waiting for gateway readiness...')
    const ready = await this.runtime.waitForReady(this.port, READY_TIMEOUT_MS)
    if (!ready) {
      this.lastError = 'Gateway did not become ready within 30 seconds'
      const logs = await this.runtime.composeLogs()
      logger.error('Gateway readiness check failed', { logs })
      throw new Error(this.lastError)
    }

    // Generate client device identity for WS auth
    logProgress('Generating client device identity...')
    ensureClientIdentity(this.openclawDir)

    logProgress('Connecting to gateway...')
    await this.connectGatewayResiliently(logProgress)

    // Ensure main agent exists (gateway may auto-create it)
    // biome-ignore lint/style/noNonNullAssertion: gateway is guaranteed connected after connectGateway()
    const existingAgents = await this.gateway!.listAgents()
    const hasMain = existingAgents.some((a) => a.agentId === 'main')
    if (!hasMain) {
      logProgress('Creating main agent...')
      const model = resolveProviderModel(input)
      // biome-ignore lint/style/noNonNullAssertion: gateway is connected
      await this.gateway!.createAgent({
        name: 'main',
        workspace: GatewayClient.agentWorkspace('main'),
        model,
      })
    } else {
      logProgress('Main agent already exists')
    }

    this.lastError = null
    logProgress(`OpenClaw gateway running at http://127.0.0.1:${this.port}`)
    logger.info('OpenClaw setup complete', { port: this.port })
  }

  async start(onLog?: (msg: string) => void): Promise<void> {
    const logProgress = this.createProgressLogger(onLog)

    logProgress('Loading gateway auth token...')
    await this.loadTokenFromEnv()
    await this.runtime.ensureReady(logProgress)
    logProgress('Starting OpenClaw gateway...')
    await this.runtime.composeUp(logProgress)

    logProgress('Waiting for gateway readiness...')
    const ready = await this.runtime.waitForReady(this.port, READY_TIMEOUT_MS)
    if (!ready) {
      this.lastError = 'Gateway did not become ready after start'
      throw new Error(this.lastError)
    }

    logProgress('Connecting to gateway...')
    await this.connectGatewayResiliently(logProgress)
    this.lastError = null
    logger.info('OpenClaw gateway started', { port: this.port })
  }

  async stop(): Promise<void> {
    this.disconnectGateway()
    await this.runtime.composeStop()
    logger.info('OpenClaw container stopped')
  }

  async restart(onLog?: (msg: string) => void): Promise<void> {
    const logProgress = this.createProgressLogger(onLog)

    this.disconnectGateway()
    logProgress('Loading gateway auth token...')
    await this.loadTokenFromEnv()
    logProgress('Restarting OpenClaw gateway...')
    await this.runtime.composeRestart(logProgress)

    logProgress('Waiting for gateway readiness...')
    const ready = await this.runtime.waitForReady(this.port, READY_TIMEOUT_MS)
    if (!ready) {
      this.lastError = 'Gateway did not become ready after restart'
      throw new Error(this.lastError)
    }

    logProgress('Connecting to gateway...')
    await this.connectGatewayResiliently(logProgress)
    this.lastError = null
    logProgress('Gateway restarted successfully')
    logger.info('OpenClaw gateway restarted', { port: this.port })
  }

  async reconnectControlPlane(onLog?: (msg: string) => void): Promise<void> {
    const logProgress = this.createProgressLogger(onLog)

    logProgress('Checking gateway readiness...')
    const ready = await this.runtime.isReady(this.port)
    if (!ready) {
      this.controlPlaneStatus = 'failed'
      this.lastGatewayError = 'OpenClaw gateway is not ready'
      this.lastRecoveryReason = 'container_not_ready'
      throw new Error('OpenClaw gateway is not ready')
    }

    logProgress('Reloading gateway auth token...')
    await this.loadTokenFromEnv()
    this.disconnectGateway()

    logProgress('Reconnecting control plane...')
    await this.ensureGatewayReady()
    logProgress('Control plane connected')
  }

  async shutdown(): Promise<void> {
    this.disconnectGateway()
    try {
      await this.runtime.composeStop()
    } catch {
      // Best effort during shutdown
    }
    await this.runtime.stopMachineIfSafe()
    logger.info('OpenClaw shutdown complete')
  }

  // ── Status ───────────────────────────────────────────────────────────

  async getStatus(): Promise<OpenClawStatusResponse> {
    const podmanAvailable = await this.runtime.isPodmanAvailable()
    if (!podmanAvailable) {
      return {
        status: 'uninitialized',
        podmanAvailable: false,
        machineReady: false,
        port: null,
        agentCount: 0,
        error: null,
        controlPlaneStatus: 'disconnected',
        lastGatewayError: null,
        lastRecoveryReason: null,
      }
    }

    const isSetUp = existsSync(join(this.openclawDir, OPENCLAW_CONFIG_FILE))
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
      ? await this.runtime.isReady(this.port)
      : false

    let agentCount = 0
    if (ready && this.gateway?.isConnected) {
      try {
        const agents = await this.gateway.listAgents()
        agentCount = agents.length
      } catch {
        // WS may be momentarily unavailable
      }
    }

    return {
      status: ready ? 'running' : this.lastError ? 'error' : 'stopped',
      podmanAvailable: true,
      machineReady: machineStatus.running,
      port: this.port,
      agentCount,
      error: this.lastError,
      controlPlaneStatus: ready
        ? this.gateway?.isConnected
          ? 'connected'
          : this.controlPlaneStatus
        : 'disconnected',
      lastGatewayError: this.lastGatewayError,
      lastRecoveryReason: this.lastRecoveryReason,
    }
  }

  // ── Agent Management (via WS RPC) ───────────────────────────────────

  async createAgent(input: {
    name: string
    providerType?: string
    providerName?: string
    baseUrl?: string
    apiKey?: string
    modelId?: string
  }): Promise<GatewayAgentEntry> {
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
    })
    await this.ensureGatewayReady()

    const configChanged = await this.mergeProviderConfigIfChanged(input)
    const keysChanged =
      input.providerType && input.apiKey
        ? await this.mergeProviderKeyIfChanged(input)
        : false

    if (configChanged || keysChanged) {
      await this.restart()
    }

    const model = resolveProviderModel(input)

    const gateway = this.gateway
    if (!gateway) {
      throw new Error('Gateway WS not connected')
    }

    let agent: GatewayAgentEntry
    try {
      agent = await gateway.createAgent({
        name,
        workspace: GatewayClient.agentWorkspace(name),
        model,
      })
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (message.includes('already exists')) {
        throw new OpenClawAgentAlreadyExistsError(name)
      }
      throw error
    }

    logger.info('Agent created via WS RPC', {
      agentId: agent.agentId,
      providerType: input.providerType,
    })
    return agent
  }

  async removeAgent(agentId: string): Promise<void> {
    if (agentId === 'main') {
      throw new OpenClawProtectedAgentError('Cannot delete the main agent')
    }

    await this.ensureGatewayReady()
    try {
      // biome-ignore lint/style/noNonNullAssertion: ensureGatewayReady() guarantees a connected client
      await this.gateway!.deleteAgent(agentId)
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (message.includes('not found')) {
        throw new OpenClawAgentNotFoundError(agentId)
      }
      throw error
    }
    logger.info('Agent removed via WS RPC', { agentId })
  }

  async listAgents(): Promise<GatewayAgentEntry[]> {
    await this.ensureGatewayReady()
    logger.debug('Listing OpenClaw agents')
    // biome-ignore lint/style/noNonNullAssertion: ensureGatewayReady() guarantees a connected client
    return this.gateway!.listAgents()
  }

  // ── Chat Stream (WS) ─────────────────────────────────────────────────

  async chatStream(
    agentId: string,
    sessionKey: string,
    message: string,
  ): Promise<ReadableStream<OpenClawStreamEvent>> {
    await this.ensureGatewayReady()
    logger.debug('Starting OpenClaw chat stream', { agentId, sessionKey })
    // biome-ignore lint/style/noNonNullAssertion: ensureGatewayReady() guarantees a connected client
    return this.gateway!.chatStream(agentId, sessionKey, message)
  }

  // ── Provider Keys ────────────────────────────────────────────────────

  async updateProviderKeys(input: {
    providerType: string
    providerName?: string
    baseUrl?: string
    apiKey: string
    modelId?: string
  }): Promise<void> {
    await this.mergeProviderConfigIfChanged(input)
    await this.mergeProviderKeyIfChanged(input)
    await this.restart()
    logger.info('Provider keys updated', { providerType: input.providerType })
  }

  // ── Logs ─────────────────────────────────────────────────────────────

  async getLogs(tail = 100): Promise<string[]> {
    logger.debug('Fetching OpenClaw container logs', { tail })
    return this.runtime.composeLogs(tail)
  }

  // ── Auto-start on BrowserOS boot ────────────────────────────────────

  async tryAutoStart(): Promise<void> {
    const isSetUp = existsSync(join(this.openclawDir, OPENCLAW_CONFIG_FILE))
    if (!isSetUp) return

    const available = await this.runtime.isPodmanAvailable()
    if (!available) return

    try {
      await this.loadTokenFromEnv()
      await this.runtime.ensureReady()

      if (!(await this.runtime.isReady(this.port))) {
        await this.runtime.composeUp()
        const ready = await this.runtime.waitForReady(
          this.port,
          READY_TIMEOUT_MS,
        )
        if (!ready) {
          logger.warn('OpenClaw gateway failed to become ready on auto-start')
          return
        }
      }

      await this.connectGatewayResiliently()
      logger.info('OpenClaw gateway auto-started')
    } catch (err) {
      logger.warn('OpenClaw auto-start failed', {
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  private async connectGatewayResiliently(
    onLog?: (msg: string) => void,
  ): Promise<void> {
    const logProgress = this.createProgressLogger(onLog)
    const existingConnection =
      !!this.gateway || this.controlPlaneStatus !== 'disconnected'
    this.controlPlaneStatus = existingConnection ? 'reconnecting' : 'connecting'
    this.lastGatewayError = null
    this.lastRecoveryReason = null

    try {
      await this.connectGateway()
      this.controlPlaneStatus = 'connected'
      this.lastGatewayError = null
      this.lastRecoveryReason = null
      logger.info('OpenClaw gateway control plane connected', {
        port: this.port,
      })
      return
    } catch (error) {
      const reason = this.classifyGatewayError(error)
      const message = error instanceof Error ? error.message : String(error)
      this.lastGatewayError = message
      this.lastRecoveryReason = reason
      logger.warn('OpenClaw gateway connect failed', { reason, error: message })

      if (!this.isRecoverableGatewayError(reason)) {
        this.controlPlaneStatus = 'failed'
        throw error
      }

      this.controlPlaneStatus = 'recovering'
      logProgress(`Recovering gateway connection: ${reason}`)
      await this.performGatewayRecovery(reason, logProgress)

      try {
        await this.connectGateway()
        this.controlPlaneStatus = 'connected'
        this.lastGatewayError = null
        logger.info('OpenClaw gateway control plane recovered', {
          reason,
          port: this.port,
        })
      } catch (retryError) {
        const retryMessage =
          retryError instanceof Error ? retryError.message : String(retryError)
        this.lastGatewayError = retryMessage
        this.lastRecoveryReason = this.classifyGatewayError(retryError)
        this.controlPlaneStatus = 'failed'
        logger.error('OpenClaw gateway recovery failed', {
          reason,
          error: retryMessage,
        })
        throw retryError
      }
    }
  }

  // ── Internal ─────────────────────────────────────────────────────────

  /**
   * Approves the latest pending device pair request via the openclaw CLI
   * running inside the container. This is needed because the gateway requires
   * Ed25519 device identity and approval before granting operator scopes.
   */
  private async approvePendingDevice(
    logProgress: (msg: string) => void,
  ): Promise<void> {
    // List pending devices to get the request ID
    const output: string[] = []
    const listCode = await this.runtime.execInContainer(
      [
        'node',
        'dist/index.js',
        'devices',
        'list',
        '--json',
        '--token',
        this.token,
      ],
      (line) => output.push(line),
    )

    if (listCode !== 0) {
      throw new Error(`Failed to list pending devices (exit ${listCode})`)
    }

    const jsonStr = output.join('\n')
    let data: {
      pending?: Array<{ requestId: string; deviceId?: string }>
    }
    try {
      data = JSON.parse(jsonStr)
    } catch {
      throw new Error(
        `Failed to parse device list output: ${jsonStr.slice(0, 200)}`,
      )
    }

    const pending = data.pending
    if (!pending?.length) {
      logger.warn('No pending device pair requests found')
      throw new Error('No pending device pair requests to approve')
    }

    const clientDeviceId = await this.readClientDeviceId()
    const pendingRequest =
      pending.find((request) => request.deviceId === clientDeviceId) ??
      pending[0]
    const requestId = pendingRequest.requestId

    if (clientDeviceId && pendingRequest.deviceId !== clientDeviceId) {
      logger.warn('Pending device request did not match client identity', {
        clientDeviceId,
        approvedRequestId: requestId,
      })
    }

    logProgress(`Approving device pair request ${requestId.slice(0, 8)}...`)

    const code = await this.runtime.execInContainer([
      'node',
      'dist/index.js',
      'devices',
      'approve',
      requestId,
      '--token',
      this.token,
      '--json',
    ])

    if (code !== 0) {
      logger.warn('Device approval command exited with code', { code })
      throw new Error('Failed to approve client device pairing')
    }

    logProgress('Client device approved')
  }

  private async connectGateway(): Promise<void> {
    this.disconnectGateway()
    logger.debug('Connecting OpenClaw gateway client', { port: this.port })
    const gateway = new GatewayClient(this.port, this.token, this.openclawDir)
    await gateway.connect()
    this.gateway = gateway
  }

  private disconnectGateway(): void {
    if (this.gateway) {
      this.gateway.disconnect()
      this.gateway = null
    }
    this.controlPlaneStatus = 'disconnected'
  }

  private async ensureGatewayReady(): Promise<void> {
    if (this.gateway?.isConnected) {
      this.controlPlaneStatus = 'connected'
      return
    }

    const portReady = await this.runtime.isReady(this.port)
    if (!portReady) {
      this.controlPlaneStatus = 'failed'
      this.lastGatewayError = 'OpenClaw gateway is not ready'
      this.lastRecoveryReason = 'container_not_ready'
      throw new Error('OpenClaw gateway is not ready')
    }

    if (this.gatewayReconnectPromise) {
      await this.gatewayReconnectPromise
      return
    }

    this.gatewayReconnectPromise = this.connectGatewayResiliently()
    try {
      await this.gatewayReconnectPromise
    } finally {
      this.gatewayReconnectPromise = null
    }
  }

  private classifyGatewayError(error: unknown): OpenClawGatewayRecoveryReason {
    const message = error instanceof Error ? error.message : String(error)
    if (message.includes('signature expired')) return 'signature_expired'
    if (message.includes('pairing required')) return 'pairing_required'
    if (message.includes('Gateway WS not connected'))
      return 'transient_disconnect'
    if (message.includes('token')) return 'token_mismatch'
    if (message.includes('not ready')) return 'container_not_ready'
    return 'unknown'
  }

  private isRecoverableGatewayError(
    reason: OpenClawGatewayRecoveryReason,
  ): boolean {
    return (
      reason === 'transient_disconnect' ||
      reason === 'signature_expired' ||
      reason === 'pairing_required' ||
      reason === 'token_mismatch'
    )
  }

  private async performGatewayRecovery(
    reason: OpenClawGatewayRecoveryReason,
    logProgress: (msg: string) => void,
  ): Promise<void> {
    switch (reason) {
      case 'signature_expired': {
        logProgress('Restarting gateway to resync device signature clock...')
        await this.runtime.composeRestart(logProgress)
        const ready = await this.runtime.waitForReady(
          this.port,
          READY_TIMEOUT_MS,
        )
        if (!ready) {
          throw new Error('Gateway not ready after clock resync restart')
        }
        return
      }

      case 'pairing_required':
        logProgress('Approving pending device pairing...')
        await this.approvePendingDevice(logProgress)
        return

      case 'token_mismatch':
        logProgress('Reloading gateway auth token...')
        await this.loadTokenFromEnv()
        return

      case 'transient_disconnect':
        logProgress('Retrying gateway connection...')
        return

      default:
        throw new Error(`Unrecoverable gateway error: ${reason}`)
    }
  }

  private async writeBootstrapConfig(
    config: Record<string, unknown>,
  ): Promise<void> {
    const configPath = join(this.openclawDir, OPENCLAW_CONFIG_FILE)
    await writeFile(configPath, JSON.stringify(config, null, 2))
  }

  /**
   * Merges provider credentials into .env. Returns true when the env file
   * changed, meaning the container should restart to pick up the update.
   */
  private async mergeProviderKeyIfChanged(input: {
    providerType?: string
    providerName?: string
    baseUrl?: string
    apiKey?: string
    modelId?: string
  }): Promise<boolean> {
    const newKeys = resolveProviderKeys(input)
    if (Object.keys(newKeys).length === 0) return false

    const envPath = join(this.openclawDir, '.env')
    let content = ''
    try {
      content = await readFile(envPath, 'utf-8')
    } catch {
      // .env may not exist yet
    }

    let addedNew = false
    let updatedExisting = false
    for (const [key, value] of Object.entries(newKeys)) {
      const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
      const pattern = new RegExp(`^${escapedKey}=.*$`, 'm')
      if (pattern.test(content)) {
        content = content.replace(pattern, `${key}=${value}`)
        updatedExisting = true
      } else {
        content = `${content.trimEnd()}\n${key}=${value}\n`
        addedNew = true
      }
    }

    await writeFile(envPath, content, { mode: 0o600 })
    logger.debug('Updated OpenClaw provider credentials', {
      providerType: input.providerType,
      addedNew,
      updatedExisting,
    })
    return addedNew || updatedExisting
  }

  private async mergeProviderConfigIfChanged(input: {
    providerType?: string
    providerName?: string
    baseUrl?: string
    modelId?: string
  }): Promise<boolean> {
    if (
      !input.providerType ||
      !input.baseUrl ||
      input.providerType in PROVIDER_ENV_MAP
    ) {
      return false
    }

    const configPath = join(this.openclawDir, OPENCLAW_CONFIG_FILE)
    let content = ''
    try {
      content = await readFile(configPath, 'utf-8')
    } catch {
      return false
    }

    const config = JSON.parse(content) as Record<string, unknown>
    const models = (config.models ?? {}) as Record<string, unknown>
    const providers = ((models.providers as
      | Record<string, unknown>
      | undefined) ?? {}) as Record<string, Record<string, unknown>>

    const providerId = deriveOpenClawProviderId(input)
    const existingProvider = providers[providerId] ?? {}
    const nextProvider: Record<string, unknown> = {
      ...existingProvider,
      baseUrl: input.baseUrl,
      apiKey:
        existingProvider.apiKey ??
        `\${${deriveOpenClawApiKeyEnvVar(providerId)}}`,
    }

    if (!existingProvider.api) {
      nextProvider.api = 'openai-completions'
    }

    if (input.modelId) {
      const existingModels = Array.isArray(existingProvider.models)
        ? (existingProvider.models as Array<Record<string, unknown>>)
        : []
      const hasModel = existingModels.some(
        (model) => model.id === input.modelId || model.name === input.modelId,
      )
      if (!hasModel) {
        nextProvider.models = [
          ...existingModels,
          { id: input.modelId, name: input.modelId },
        ]
      }
    }

    if (
      JSON.stringify(existingProvider) === JSON.stringify(nextProvider) &&
      models.mode === 'merge'
    ) {
      return false
    }

    config.models = {
      ...models,
      mode: 'merge',
      providers: {
        ...providers,
        [providerId]: nextProvider,
      },
    }
    await this.writeBootstrapConfig(config)
    logger.debug('Updated OpenClaw provider config', {
      providerId,
      providerType: input.providerType,
      hasModel: !!input.modelId,
    })
    return true
  }

  private async loadTokenFromEnv(): Promise<void> {
    const envPath = join(this.openclawDir, '.env')
    try {
      const content = await readFile(envPath, 'utf-8')
      const match = content.match(/^OPENCLAW_GATEWAY_TOKEN=(.+)$/m)
      if (match) {
        this.token = match[1]
        logger.debug('Loaded OpenClaw gateway token from env')
      }
    } catch {
      logger.debug('OpenClaw env file not available while loading token')
    }
  }

  private async readClientDeviceId(): Promise<string | null> {
    try {
      const identityPath = join(this.openclawDir, 'client-identity.json')
      const identity = JSON.parse(await readFile(identityPath, 'utf-8')) as {
        deviceId?: string
      }
      return identity.deviceId ?? null
    } catch {
      return null
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
}

let service: OpenClawService | null = null

export function getOpenClawService(
  browserosServerPort?: number,
): OpenClawService {
  if (!service) service = new OpenClawService(browserosServerPort)
  return service
}
