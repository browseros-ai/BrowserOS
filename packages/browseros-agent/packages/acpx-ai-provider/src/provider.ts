import { EventEmitter } from 'node:events'
import os from 'node:os'
import path from 'node:path'
import type {
  AcpRuntime,
  AcpRuntimeAvailableCommand,
  AcpRuntimeDoctorReport,
  AcpRuntimeEvent,
  AcpRuntimeHandle,
  AcpRuntimeOptions,
  AcpRuntimeSessionModels,
} from 'acpx/runtime'
import {
  createAcpRuntime,
  createAgentRegistry,
  createFileSessionStore,
} from 'acpx/runtime'
import { AcpxLanguageModel } from './language-model'
import { toRuntimeMcpServers } from './mcp-servers'
import type {
  AcpxLanguageModelOptions,
  AcpxProviderSettings,
  AcpxSessionMode,
  AcpxUsageSnapshot,
} from './types'

const DEFAULT_PERMISSION_MODE = 'approve-reads'
const DEFAULT_NON_INTERACTIVE = 'deny'

interface ResolvedHandle {
  handle: AcpRuntimeHandle
  agent: string
}

export interface EnsureHandleResult {
  handle: AcpRuntimeHandle
  sessionKey: string
  mode: AcpxSessionMode
  isFresh: boolean
}

export interface AcpxProviderEvents {
  usage: [AcpxUsageSnapshot]
  availableCommands: [
    { sessionKey: string; commands: AcpRuntimeAvailableCommand[] },
  ]
}

export class AcpxProvider {
  readonly settings: AcpxProviderSettings
  readonly generateId: () => string
  readonly events: EventEmitter<AcpxProviderEvents> =
    new EventEmitter<AcpxProviderEvents>()

  private runtimeInstance: AcpRuntime | null
  private readonly handles = new Map<string, ResolvedHandle>()
  private readonly usedKeys = new Set<string>()
  private readonly lastUsage = new Map<string, AcpxUsageSnapshot>()
  private readonly lastCommands = new Map<
    string,
    AcpRuntimeAvailableCommand[]
  >()

  constructor(settings: AcpxProviderSettings) {
    this.settings = settings
    this.runtimeInstance = settings.runtime ?? null
    this.generateId = settings._internal?.generateId ?? defaultIdGen()
  }

  get runtime(): AcpRuntime {
    if (!this.runtimeInstance) {
      this.runtimeInstance = createAcpRuntime(this.buildRuntimeOptions())
    }
    return this.runtimeInstance
  }

  languageModel(
    _modelId?: string,
    opts: AcpxLanguageModelOptions = {},
  ): AcpxLanguageModel {
    return new AcpxLanguageModel(this, opts)
  }

  async prepare(
    opts: AcpxLanguageModelOptions = {},
  ): Promise<AcpRuntimeHandle> {
    const { handle } = await this.ensureHandle(opts)
    return handle
  }

  async ensureHandle(
    opts: AcpxLanguageModelOptions = {},
  ): Promise<EnsureHandleResult> {
    const sessionKey = this.resolveSessionKey(opts)
    const agent = opts.agent ?? this.settings.agent
    const mode: AcpxSessionMode = this.settings.sessionMode ?? 'persistent'

    let cached = this.handles.get(sessionKey)
    if (!cached || cached.agent !== agent) {
      const handle = await this.runtime.ensureSession({
        sessionKey,
        agent,
        mode,
        cwd: this.settings.cwd,
        resumeSessionId: this.settings.resumeSessionId,
        sessionOptions: this.settings.sessionOptions,
      })
      cached = { handle, agent }
      this.handles.set(sessionKey, cached)
    }

    const isFresh = !this.usedKeys.has(sessionKey)
    return { handle: cached.handle, sessionKey, mode, isFresh }
  }

  markSessionKeyUsed(sessionKey: string): boolean {
    const wasFresh = !this.usedKeys.has(sessionKey)
    this.usedKeys.add(sessionKey)
    return wasFresh
  }

  resolveSessionKey(opts: AcpxLanguageModelOptions): string {
    if (opts.sessionKey) return opts.sessionKey
    if (this.settings.sessionKey) return this.settings.sessionKey
    const cwd = this.settings.cwd ?? process.cwd()
    const agent = opts.agent ?? this.settings.agent
    return `${agent}::${cwd}`
  }

  async cancel(reason = 'cancel'): Promise<void> {
    for (const [, { handle }] of this.handles) {
      await this.runtime.cancel({ handle, reason })
    }
  }

  async close(
    reason = 'close',
    options: { discardPersistentState?: boolean } = {},
  ): Promise<void> {
    for (const [key, { handle }] of this.handles) {
      await this.runtime.close({
        handle,
        reason,
        discardPersistentState: options.discardPersistentState ?? false,
      })
      this.usedKeys.delete(key)
    }
    this.handles.clear()
  }

  async setMode(mode: string): Promise<void> {
    const setModeImpl = this.runtime.setMode
    if (!setModeImpl) return
    for (const [, { handle }] of this.handles) {
      await setModeImpl.call(this.runtime, { handle, mode })
    }
  }

  async setConfigOption(key: string, value: string): Promise<void> {
    const setOptImpl = this.runtime.setConfigOption
    if (!setOptImpl) return
    for (const [, { handle }] of this.handles) {
      await setOptImpl.call(this.runtime, { handle, key, value })
    }
  }

  async getModels(
    opts: AcpxLanguageModelOptions = {},
  ): Promise<AcpRuntimeSessionModels | undefined> {
    const getStatusImpl = this.runtime.getStatus
    if (!getStatusImpl) return undefined
    const { handle } = await this.ensureHandle(opts)
    const status = await getStatusImpl.call(this.runtime, { handle })
    return status.models
  }

  /**
   * Latest `usage_update` snapshot observed for this session, or
   * `undefined` if no usage event has fired yet. Synchronous;
   * does not spawn an agent or query the runtime. Callers wanting
   * push updates should subscribe to `provider.events.on('usage', ...)`.
   */
  getUsage(sessionKey?: string): AcpxUsageSnapshot | undefined {
    const snapshot = this.lastUsage.get(
      sessionKey ?? this.resolveSessionKey({}),
    )
    // Defensive copy so callers cannot mutate the provider's stored state.
    return snapshot ? { ...snapshot } : undefined
  }

  /**
   * Latest list of agent-advertised slash commands for this session,
   * or `[]` if no `available_commands_update` event has fired yet.
   * Synchronous. Subscribe to `provider.events.on('availableCommands', ...)`
   * for push updates.
   */
  getAvailableCommands(sessionKey?: string): AcpRuntimeAvailableCommand[] {
    const commands = this.lastCommands.get(
      sessionKey ?? this.resolveSessionKey({}),
    )
    // Defensive copy so callers cannot mutate the provider's stored state.
    return (commands ?? []).map((c) => ({ ...c }))
  }

  /** Internal: called by the language model when an event arrives. */
  recordUsage(snapshot: AcpxUsageSnapshot): void {
    this.lastUsage.set(snapshot.sessionKey, snapshot)
    this.events.emit('usage', snapshot)
  }

  /** Internal: called by the language model when an event arrives. */
  recordAvailableCommands(
    sessionKey: string,
    commands: AcpRuntimeAvailableCommand[],
  ): void {
    this.lastCommands.set(sessionKey, commands)
    this.events.emit('availableCommands', { sessionKey, commands })
  }

  /**
   * Route a raw runtime event to `provider.events` the same way a
   * language-model turn does. Used by `runSlashCommand`, whose turn does
   * not pass through the language model's event translator.
   */
  private routeLiveEvent(event: AcpRuntimeEvent, sessionKey: string): void {
    if (event.type !== 'status') return
    if (event.tag === 'usage_update') {
      this.recordUsage({
        used: event.used,
        size: event.size,
        cost: event.cost,
        breakdown: event.breakdown,
        at: Date.now(),
        sessionKey,
      })
    } else if (
      event.tag === 'available_commands_update' &&
      event.availableCommands
    ) {
      this.recordAvailableCommands(sessionKey, event.availableCommands)
    }
  }

  /**
   * Send a one-shot prompt whose text is a slash command (e.g.
   * `"/compact"`). The active agent interprets the slash on its side;
   * ACP itself has no compact verb. Drains the event iterator and routes
   * any `usage_update` / `available_commands_update` events the command
   * fires to `provider.events` subscribers, so (for example) the
   * post-compaction usage snapshot reaches the caller.
   */
  async runSlashCommand(input: {
    name: string
    sessionKey?: string
    agent?: string
    timeoutMs?: number
    signal?: AbortSignal
  }): Promise<void> {
    const { handle, sessionKey } = await this.ensureHandle({
      sessionKey: input.sessionKey,
      agent: input.agent,
    })
    const turn = this.runtime.startTurn({
      handle,
      text: input.name,
      mode: 'prompt',
      requestId: this.generateId(),
      timeoutMs: input.timeoutMs,
      signal: input.signal,
    })
    for await (const event of turn.events) {
      // Route the live signals the same way a doStream/doGenerate turn
      // does, so subscribers still see them on a slash-command-only turn.
      this.routeLiveEvent(event, sessionKey)
    }
    const result = await turn.result
    if (result.status === 'failed') {
      throw new Error(
        `slash command "${input.name}" failed: ${result.error.message}`,
      )
    }
  }

  /**
   * Convenience wrapper around `runSlashCommand` that resolves a
   * `/compact`-like command name from the agent's advertised list. The
   * agent must have emitted at least one `available_commands_update`
   * before this is called; that flows through during a `doStream` /
   * `doGenerate` turn, not after `prepare()` alone. Throws if no
   * compact-like command is advertised.
   */
  async compact(
    opts: { sessionKey?: string; agent?: string } = {},
  ): Promise<void> {
    const sessionKey = this.resolveSessionKey({ sessionKey: opts.sessionKey })
    const cmds = this.lastCommands.get(sessionKey) ?? []
    const cmd = cmds.find((c) => {
      const stripped = c.name.replace(/^\//, '').toLowerCase()
      return stripped === 'compact' || stripped === 'condense'
    })
    if (!cmd) {
      throw new Error(
        `active agent does not advertise a compact command on session "${sessionKey}". ` +
          'Wait for an available_commands_update event (it flows during a doStream/doGenerate turn, not after prepare()/ensureHandle alone) before calling compact().',
      )
    }
    const name = cmd.name.startsWith('/') ? cmd.name : `/${cmd.name}`
    await this.runSlashCommand({
      name,
      sessionKey: opts.sessionKey,
      agent: opts.agent,
    })
  }

  async doctor(): Promise<AcpRuntimeDoctorReport> {
    const doctorImpl = this.runtime.doctor
    if (!doctorImpl) {
      return { ok: true, message: 'no doctor implementation in this runtime' }
    }
    return await doctorImpl.call(this.runtime)
  }

  private buildRuntimeOptions(): AcpRuntimeOptions {
    const stateDir = this.settings.stateDir ?? path.join(os.homedir(), '.acpx')
    return {
      cwd: this.settings.cwd ?? process.cwd(),
      sessionStore: createFileSessionStore({ stateDir }),
      agentRegistry: createAgentRegistry({
        overrides: this.settings.agentRegistryOverrides,
      }),
      permissionMode: (this.settings.permissionMode ??
        DEFAULT_PERMISSION_MODE) as AcpRuntimeOptions['permissionMode'],
      nonInteractivePermissions: (this.settings.nonInteractivePermissions ??
        DEFAULT_NON_INTERACTIVE) as AcpRuntimeOptions['nonInteractivePermissions'],
      timeoutMs: this.settings.turnTimeoutMs,
      mcpServers: toRuntimeMcpServers(this.settings.mcpServers),
      onPermissionRequest: this.settings.onPermissionRequest,
    }
  }
}

function defaultIdGen(): () => string {
  let n = 0
  return () => `acpx-${++n}`
}

export function createAcpxProvider(
  settings: AcpxProviderSettings,
): AcpxProvider {
  return new AcpxProvider(settings)
}
