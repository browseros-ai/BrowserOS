/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  configFromRuntimeEnv,
  type IngestionSink,
  type ObjectStoreIngestionSinkOptions,
  sinkFromStore,
} from '@agentpond/core'
import { AgentPondSpanExporter } from '@agentpond/otel'
import {
  isOpenInferenceSpan,
  OpenInferenceBatchSpanProcessor,
} from '@arizeai/openinference-vercel'
import { BasicTracerProvider } from '@opentelemetry/sdk-trace-base'
import type { TelemetrySettings } from 'ai'
import { Files } from 'files-sdk'
import { fs } from 'files-sdk/fs'
import { logger } from '../lib/logger'

const AGENTPOND_TRACING_STATE = Symbol.for('browseros.agentpond-tracing')

interface AgentPondTracingState {
  provider: BasicTracerProvider
  telemetry: TelemetrySettings
}

type AgentPondGlobal = typeof globalThis & {
  [AGENTPOND_TRACING_STATE]?: AgentPondTracingState
}

class LocalAgentPondObjectStore {
  private readonly files: Files

  constructor(root: string) {
    this.files = new Files({
      adapter: fs({ root }),
      retries: 3,
      timeout: 10_000,
    })
  }

  toSink(options?: ObjectStoreIngestionSinkOptions): IngestionSink {
    return sinkFromStore(this, options)
  }

  async putJson(key: string, value: unknown): Promise<void> {
    await this.files.upload(key, JSON.stringify(value), {
      contentType: 'application/json',
    })
  }

  async getJson<T>(key: string): Promise<T> {
    return JSON.parse(await (await this.files.download(key)).text()) as T
  }

  async listKeys(prefix: string): Promise<string[]> {
    const keys: string[] = []
    for await (const file of this.files.listAll({ prefix })) {
      keys.push(file.key)
    }
    return keys.sort()
  }
}

function createAgentPondTracingState(): AgentPondTracingState | undefined {
  const providerName = process.env.FILES_SDK_PROVIDER?.trim()
  if (!providerName) return undefined

  if (providerName !== 'fs') {
    logger.warn(
      'BrowserOS AgentPond tracing only supports FILES_SDK_PROVIDER=fs',
    )
    return undefined
  }

  const root = process.env.FILES_SDK_ROOT?.trim()
  if (!root) {
    logger.warn('BrowserOS AgentPond tracing requires FILES_SDK_ROOT')
    return undefined
  }

  try {
    const config = configFromRuntimeEnv()
    const provider = new BasicTracerProvider({
      spanProcessors: [
        new OpenInferenceBatchSpanProcessor({
          exporter: new AgentPondSpanExporter({
            prefix: config.prefix,
            projectId: config.projectId,
            store: new LocalAgentPondObjectStore(root),
          }),
          spanFilter: isOpenInferenceSpan,
          reparentOrphanedSpans: true,
        }),
      ],
    })

    return {
      provider,
      telemetry: {
        functionId: 'browseros-agent',
        isEnabled: true,
        recordInputs: false,
        recordOutputs: false,
        tracer: provider.getTracer('@browseros/server'),
      },
    }
  } catch (error) {
    logger.warn('Failed to initialize AgentPond tracing', {
      error: error instanceof Error ? error.message : String(error),
    })
    return undefined
  }
}

const agentPondGlobal = globalThis as AgentPondGlobal
const tracingState =
  agentPondGlobal[AGENTPOND_TRACING_STATE] ?? createAgentPondTracingState()
if (tracingState) {
  agentPondGlobal[AGENTPOND_TRACING_STATE] = tracingState
}

export const agentPondTelemetry = tracingState?.telemetry

export async function flushAgentPondTracing(): Promise<void> {
  try {
    await tracingState?.provider.forceFlush()
  } catch (error) {
    logger.warn('Failed to flush AgentPond tracing', {
      error: error instanceof Error ? error.message : String(error),
    })
  }
}

let shutdownPromise: Promise<void> | undefined

export function shutdownAgentPondTracing(): Promise<void> {
  if (!shutdownPromise) {
    shutdownPromise = (async () => {
      try {
        await tracingState?.provider.shutdown()
      } catch (error) {
        logger.warn('Failed to shut down AgentPond tracing', {
          error: error instanceof Error ? error.message : String(error),
        })
      } finally {
        delete agentPondGlobal[AGENTPOND_TRACING_STATE]
      }
    })()
  }
  return shutdownPromise
}
