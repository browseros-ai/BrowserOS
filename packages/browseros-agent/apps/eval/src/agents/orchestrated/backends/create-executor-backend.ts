import type { Browser } from '@browseros/browser-core/browser'
import type { ResolvedAgentConfig } from '@browseros/server/agent/types'
import type {
  ExecutorBackend,
  ExecutorBackendKind,
  ExecutorCallbacks,
} from '../executor-backend'
import { CladoExecutorBackend } from './clado/clado-executor-backend'
import { isCladoActionProvider } from './clado/types'
import { ToolLoopExecutorBackend } from './tool-loop/tool-loop-executor-backend'

export interface CreateExecutorBackendOptions {
  backendKind?: ExecutorBackendKind
  provider?: string
  configTemplate?: ResolvedAgentConfig
  browser?: Browser | null
  serverUrl?: string
  windowId?: number
  tabId?: number
  initialPageId?: number
  callbacks?: ExecutorCallbacks
  executor?: ExecutorBackend
}

export function backendKindForProvider(provider: string): ExecutorBackendKind {
  return isCladoActionProvider(provider) ? 'clado' : 'tool-loop'
}

/** Creates the backend used for one orchestrator delegation. */
export function createExecutorBackend(
  options: CreateExecutorBackendOptions,
): ExecutorBackend {
  if (options.executor) return options.executor

  const kind =
    options.backendKind ??
    backendKindForProvider(
      options.provider ?? options.configTemplate?.provider ?? '',
    )

  if (kind === 'clado') {
    return new CladoExecutorBackend({
      configTemplate: required(options.configTemplate, 'configTemplate'),
      serverUrl: required(options.serverUrl, 'serverUrl'),
      initialPageId: options.initialPageId,
      callbacks: options.callbacks,
    })
  }

  return new ToolLoopExecutorBackend({
    configTemplate: required(options.configTemplate, 'configTemplate'),
    browser: options.browser ?? null,
    callbacks: options.callbacks,
  })
}

function required<T>(value: T | undefined, name: string): T {
  if (value === undefined) throw new Error(`${name} is required`)
  return value
}
