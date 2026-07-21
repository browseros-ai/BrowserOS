/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  RECORDING_INGEST_FALLBACK_MAX_BYTES,
  RECORDING_INGEST_MAX_BYTES,
} from '@browseros/shared/constants/limits'
import { ApiResponseError, ClawApiClient } from '../api/client'
import {
  createIndexedDbRecordingOutbox,
  type NewRecordingBatch,
  type RecordingOutbox,
  type StoredRecordingBatch,
} from './recordings-outbox'

export type Fetcher = (
  input: Parameters<typeof globalThis.fetch>[0],
  init?: Parameters<typeof globalThis.fetch>[1],
) => ReturnType<typeof globalThis.fetch>

type TimerHandle = ReturnType<typeof globalThis.setTimeout>

export interface RecordingsRelayOptions {
  resolveServerBaseUrl: () => Promise<string>
  outbox?: RecordingOutbox
  fetch?: Fetcher
  now?: () => number
  warn?: (...args: unknown[]) => void
  setTimeout?: (callback: () => void, delayMs: number) => TimerHandle
  clearTimeout?: (handle: TimerHandle) => void
}

export interface RecordingsRelay {
  start: () => Promise<void>
  post: (
    tabId: number,
    documentId: string,
    ndjson: string,
    hasGap?: boolean,
  ) => Promise<void>
  onTabRecoveredAfterLoss: (listener: (tabId: number) => void) => () => void
}

type SendOutcome =
  | { kind: 'success'; gapToken?: string }
  | { kind: 'unsupported' }
  | { kind: 'invalid'; error: unknown }
  | { kind: 'oversize'; error: unknown }
  | { kind: 'transient'; error: unknown }

interface IngestCapability {
  maxBytes: number
  expiresAt: number
  supported: boolean
}

export const RECORDINGS_QUEUE_MAX_BYTES = 2 * RECORDING_INGEST_MAX_BYTES
const RETRY_INTERVAL_MS = 5_000
const WARNING_INTERVAL_MS = 60_000
const CAPABILITY_CACHE_TTL_MS = 60_000

/**
 * Durable, session-neutral delivery from recorder documents to ingest v2.
 * Every batch is committed to IndexedDB before delivery, so background
 * suspension can only cause a retry and the server's batch catalog dedupes it.
 */
export function createRecordingsRelay(
  options: RecordingsRelayOptions,
): RecordingsRelay {
  const fetch = options.fetch ?? globalThis.fetch
  const outbox = options.outbox ?? createIndexedDbRecordingOutbox()
  const now = options.now ?? Date.now
  const warn = options.warn ?? console.warn
  const setTimer = options.setTimeout ?? globalThis.setTimeout
  const clearTimer = options.clearTimeout ?? globalThis.clearTimeout
  const encoder = new TextEncoder()
  const recoveredListeners = new Set<(tabId: number) => void>()
  const lastWarningAt = new Map<string, number>()
  const capabilities = new Map<string, IngestCapability>()
  let retryTimer: TimerHandle | null = null
  let drainPromise: Promise<void> | null = null
  let mutationChain = Promise.resolve()
  let sendingBatchId: string | null = null

  function safeWarn(...args: unknown[]): void {
    try {
      warn(...args)
    } catch {}
  }

  function warnRateLimited(
    kind: string,
    message: string,
    error: unknown,
  ): void {
    const timestamp = now()
    const lastAt = lastWarningAt.get(kind)
    if (lastAt !== undefined && timestamp - lastAt < WARNING_INTERVAL_MS) return
    lastWarningAt.set(kind, timestamp)
    safeWarn(message, {
      error: error instanceof Error ? error.message : String(error),
    })
  }

  function mutate<T>(operation: () => Promise<T>): Promise<T> {
    const result = mutationChain.then(operation, operation)
    mutationChain = result.then(
      () => undefined,
      () => undefined,
    )
    return result
  }

  function makeBatch(
    tabId: number,
    documentId: string,
    ndjson: string,
  ): NewRecordingBatch {
    return {
      batchId: crypto.randomUUID(),
      tabId,
      documentId,
      ndjson,
      bytes: encoder.encode(ndjson).byteLength,
      createdAt: now(),
    }
  }

  function makeBatches(
    tabId: number,
    documentId: string,
    ndjson: string,
  ): NewRecordingBatch[] {
    const sourceLines = ndjson.split('\n')
    if (sourceLines.length > 1 && sourceLines.at(-1) === '') sourceLines.pop()
    if (sourceLines.length === 0) return []

    const batches: NewRecordingBatch[] = []
    let batchLines: string[] = []
    let batchBytes = 0
    const flush = () => {
      if (batchLines.length === 0) return
      batches.push(makeBatch(tabId, documentId, batchLines.join('\n')))
      batchLines = []
      batchBytes = 0
    }

    for (const line of sourceLines) {
      const lineBytes = encoder.encode(line).byteLength
      const separatorBytes = batchLines.length > 0 ? 1 : 0
      if (
        batchLines.length > 0 &&
        batchBytes + separatorBytes + lineBytes >
          RECORDING_INGEST_FALLBACK_MAX_BYTES
      ) {
        flush()
      }
      if (lineBytes > RECORDING_INGEST_FALLBACK_MAX_BYTES) {
        flush()
        batches.push(makeBatch(tabId, documentId, line))
        continue
      }
      if (batchLines.length > 0) batchBytes++
      batchLines.push(line)
      batchBytes += lineBytes
    }
    flush()
    return batches
  }

  async function enforceQueueBudget(): Promise<void> {
    let batches = await outbox.list()
    let totalBytes = batches.reduce((sum, batch) => sum + batch.bytes, 0)
    while (totalBytes > RECORDINGS_QUEUE_MAX_BYTES) {
      const bytesByDocument = new Map<string, number>()
      for (const batch of batches) {
        if (batch.batchId === sendingBatchId) continue
        bytesByDocument.set(
          batch.documentId,
          (bytesByDocument.get(batch.documentId) ?? 0) + batch.bytes,
        )
      }
      const largestDocument = [...bytesByDocument].sort(
        (left, right) => right[1] - left[1],
      )[0]?.[0]
      const eviction = batches.find(
        (batch) =>
          batch.documentId === largestDocument &&
          batch.batchId !== sendingBatchId,
      )
      if (!eviction) return
      await outbox.remove(eviction.batchId)
      await outbox.markGap(eviction.documentId, eviction.tabId)
      totalBytes -= eviction.bytes
      batches = batches.filter((batch) => batch.batchId !== eviction.batchId)
      warnRateLimited(
        'queue-eviction',
        '[browseros-claw replay] recording batch evicted under outbox pressure',
        `document ${eviction.documentId}`,
      )
    }
  }

  async function discoverCapability(
    baseUrl: string,
    client: ClawApiClient,
  ): Promise<IngestCapability | undefined> {
    const cached = capabilities.get(baseUrl)
    if (cached && now() < cached.expiresAt) return cached
    try {
      const system = await client.getSystemInfo()
      const advertisedMax = system.capabilities?.recordingIngestMaxBytes
      const capability = {
        supported: system.capabilities?.recordingIngestVersion === 2,
        maxBytes:
          typeof advertisedMax === 'number' &&
          Number.isSafeInteger(advertisedMax) &&
          advertisedMax > 0
            ? Math.min(advertisedMax, RECORDINGS_QUEUE_MAX_BYTES)
            : RECORDING_INGEST_FALLBACK_MAX_BYTES,
        expiresAt: now() + CAPABILITY_CACHE_TTL_MS,
      }
      capabilities.set(baseUrl, capability)
      return capability
    } catch (error) {
      if (error instanceof ApiResponseError && error.response.status === 404) {
        const capability = {
          supported: false,
          maxBytes: RECORDING_INGEST_FALLBACK_MAX_BYTES,
          expiresAt: now() + CAPABILITY_CACHE_TTL_MS,
        }
        capabilities.set(baseUrl, capability)
        return capability
      }
      return undefined
    }
  }

  async function sendBatch(batch: StoredRecordingBatch): Promise<SendOutcome> {
    try {
      const baseUrl = await options.resolveServerBaseUrl()
      const client = new ClawApiClient(baseUrl, { fetch })
      const capability = await discoverCapability(baseUrl, client)
      if (!capability) {
        return {
          kind: 'transient',
          error: new Error('recording ingest capability is unreachable'),
        }
      }
      if (!capability.supported) return { kind: 'unsupported' }
      if (batch.bytes > capability.maxBytes) {
        return {
          kind: 'oversize',
          error: new Error(
            `server accepts at most ${capability.maxBytes.toString()} bytes`,
          ),
        }
      }

      const gap = await outbox.getGap(batch.documentId)
      await client.appendRecordingEvents({
        xRecordingTabId: batch.tabId,
        xRecordingDocumentId: batch.documentId,
        xRecordingBatchId: batch.batchId,
        xRecordingHasGap: gap ? true : undefined,
        body: batch.ndjson,
      })
      return { kind: 'success', gapToken: gap?.token }
    } catch (error) {
      if (error instanceof ApiResponseError) {
        if (error.response.status === 413) return { kind: 'oversize', error }
        if (error.response.status === 400) return { kind: 'invalid', error }
      }
      return { kind: 'transient', error }
    }
  }

  function cancelRetry(): void {
    if (retryTimer === null) return
    clearTimer(retryTimer)
    retryTimer = null
  }

  function armRetry(): void {
    if (retryTimer !== null) return
    retryTimer = setTimer(() => {
      retryTimer = null
      return drain()
    }, RETRY_INTERVAL_MS)
  }

  function notifyRecovered(tabId: number): void {
    for (const listener of recoveredListeners) {
      try {
        listener(tabId)
      } catch (error) {
        warnRateLimited(
          'recovery-listener',
          '[browseros-claw replay] recovery listener failed',
          error,
        )
      }
    }
  }

  async function drain(): Promise<void> {
    if (drainPromise) return drainPromise
    cancelRetry()
    const run = async () => {
      while (true) {
        await mutationChain
        const batch = (await outbox.list())[0]
        if (!batch) return
        sendingBatchId = batch.batchId
        const outcome = await sendBatch(batch)
        sendingBatchId = null

        if (outcome.kind === 'unsupported') {
          warnRateLimited(
            'unsupported-server',
            '[browseros-claw replay] sidecar does not support document recording ingest; events remain queued',
            'recordingIngestVersion is not 2',
          )
          armRetry()
          return
        }
        if (outcome.kind === 'transient') {
          warnRateLimited(
            'transient-send',
            '[browseros-claw replay] events POST failed; events remain queued',
            outcome.error,
          )
          armRetry()
          return
        }

        await mutate(async () => {
          await outbox.remove(batch.batchId)
          if (outcome.kind === 'success' && outcome.gapToken) {
            await outbox.clearGap(batch.documentId, outcome.gapToken)
          } else if (outcome.kind !== 'success') {
            await outbox.markGap(batch.documentId, batch.tabId)
          }
        })

        if (outcome.kind === 'success') {
          lastWarningAt.delete('transient-send')
          if (outcome.gapToken) notifyRecovered(batch.tabId)
        } else {
          warnRateLimited(
            outcome.kind,
            '[browseros-claw replay] invalid recording batch dropped; replay gap recorded',
            outcome.error,
          )
        }
      }
    }

    drainPromise = run().finally(() => {
      sendingBatchId = null
      drainPromise = null
    })
    return drainPromise
  }

  async function post(
    tabId: number,
    documentId: string,
    ndjson: string,
    hasGap = false,
  ): Promise<void> {
    try {
      await mutate(async () => {
        if (hasGap) await outbox.markGap(documentId, tabId)
        for (const batch of makeBatches(tabId, documentId, ndjson)) {
          await outbox.add(batch)
        }
        await enforceQueueBudget()
      })
      await drain()
    } catch (error) {
      warnRateLimited(
        'relay-internal',
        '[browseros-claw replay] relay failed unexpectedly',
        error,
      )
      armRetry()
      throw error
    }
  }

  return {
    start: drain,
    post,
    onTabRecoveredAfterLoss(listener) {
      recoveredListeners.add(listener)
      return () => recoveredListeners.delete(listener)
    },
  }
}
