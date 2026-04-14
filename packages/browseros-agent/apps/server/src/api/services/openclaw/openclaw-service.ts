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

    // Attempt WS connect — this triggers a pending pair request
    logProgress('Pairing client device...')
    try {
      await this.connectGateway()
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      if (
        !msg.includes('pairing required') &&
        !msg.includes('signature expired')
      ) {
        throw err
      }
    }

    // Approve the pending device via the openclaw CLI inside the container
    await this.approvePendingDevice(logProgress)

    logProgress('Connecting to gateway...')
    await this.connectGateway()

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
    await this.connectGateway()
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
    await this.connectGateway()
    this.lastError = null
    logProgress('Gateway restarted successfully')
    logger.info('OpenClaw gateway restarted', { port: this.port })
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
    this.ensureGatewayConnected()

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

    this.ensureGatewayConnected()
    try {
      // biome-ignore lint/style/noNonNullAssertion: ensureGatewayConnected() guards above
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
    this.ensureGatewayConnected()
    logger.debug('Listing OpenClaw agents')
    // biome-ignore lint/style/noNonNullAssertion: ensureGatewayConnected() guards above
    return this.gateway!.listAgents()
  }

  // ── Chat Stream (WS) ─────────────────────────────────────────────────

  chatStream(
    agentId: string,
    sessionKey: string,
    message: string,
  ): ReadableStream<OpenClawStreamEvent> {
    this.ensureGatewayConnected()
    logger.debug('Starting OpenClaw chat stream', { agentId, sessionKey })
    // biome-ignore lint/style/noNonNullAssertion: ensureGatewayConnected() guards above
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

      await this.connectGatewayWithRetry()
      logger.info('OpenClaw gateway auto-started')
    } catch (err) {
      logger.warn('OpenClaw auto-start failed', {
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  /**
   * Connects to the gateway, retrying once after a container restart
   * if the signature is expired (clock skew from Podman VM sleep).
   */
  private async connectGatewayWithRetry(): Promise<void> {
    try {
      await this.connectGateway()
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      if (
        msg.includes('signature expired') ||
        msg.includes('pairing required')
      ) {
        logger.info(
          'Gateway WS auth failed, restarting container to resync clock...',
        )
        await this.runtime.composeRestart()
        const ready = await this.runtime.waitForReady(
          this.port,
          READY_TIMEOUT_MS,
        )
        if (!ready)
          throw new Error('Gateway not ready after clock resync restart')

        // Re-approve device if needed (pairing may have been lost)
        try {
          await this.connectGateway()
        } catch (retryErr) {
          const retryMsg =
            retryErr instanceof Error ? retryErr.message : String(retryErr)
          if (retryMsg.includes('pairing required')) {
            await this.approvePendingDevice((m) =>
              logger.debug(`Auto-start: ${m}`),
            )
            await this.connectGateway()
          } else {
            throw retryErr
          }
        }
      } else {
        throw err
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
    this.gateway = new GatewayClient(this.port, this.token, this.openclawDir)
    await this.gateway.connect()
  }

  private disconnectGateway(): void {
    if (this.gateway) {
      this.gateway.disconnect()
      this.gateway = null
    }
  }

  private ensureGatewayConnected(): void {
    if (!this.gateway?.isConnected) {
      logger.debug('OpenClaw gateway client is not connected')
      throw new Error('Gateway WS not connected')
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
