import os from 'node:os'
import path from 'node:path'
import type {
  AcpRuntime,
  AcpRuntimeDoctorReport,
  AcpRuntimeHandle,
  AcpRuntimeOptions,
  AcpRuntimeSessionModels,
} from 'acpx/runtime'
import {
  createAcpRuntime,
  createAgentRegistry,
  createFileSessionStore,
} from 'acpx/runtime'
import { AcpxLanguageModel } from './language-model.ts'
import { toRuntimeMcpServers } from './mcp-servers.ts'
import type {
  AcpxLanguageModelOptions,
  AcpxProviderSettings,
  AcpxSessionMode,
} from './types.ts'

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

export class AcpxProvider {
  readonly settings: AcpxProviderSettings
  readonly generateId: () => string

  private runtimeInstance: AcpRuntime | null
  private readonly handles = new Map<string, ResolvedHandle>()
  private readonly usedKeys = new Set<string>()

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

  async close(reason = 'close'): Promise<void> {
    for (const [key, { handle }] of this.handles) {
      await this.runtime.close({
        handle,
        reason,
        discardPersistentState: false,
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
