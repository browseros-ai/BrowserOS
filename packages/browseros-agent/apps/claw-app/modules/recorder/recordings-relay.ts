/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  Configuration,
  DefaultApi,
  RECORDING_INGEST_FALLBACK_MAX_BYTES,
  RECORDING_INGEST_MAX_BYTES,
  ResponseError,
  type Tab,
} from '@browseros/claw-api'

export type Fetcher = (
  input: Parameters<typeof globalThis.fetch>[0],
  init?: Parameters<typeof globalThis.fetch>[1],
) => ReturnType<typeof globalThis.fetch>

type TimerHandle = ReturnType<typeof globalThis.setTimeout>

export interface RecordingsRelayOptions {
  resolveServerBaseUrl: () => Promise<string>
  fetch?: Fetcher
  now?: () => number
  warn?: (...args: unknown[]) => void
  setTimeout?: (callback: () => void, delayMs: number) => TimerHandle
  clearTimeout?: (handle: TimerHandle) => void
}

export interface RecordingsRelay {
  post: (tabId: number, ndjson: string) => Promise<void>
  onTabRecoveredAfterLoss: (listener: (tabId: number) => void) => () => void
}

interface QueuedBatch {
  batchId: string
  ndjson: string
  bytes: number
  /**
   * The session/page/target that produced these events, stamped from the
   * tab's last known association at enqueue time (or on first successful
   * tab lookup). A pinned batch whose association no longer matches the
   * tab's live one is dropped rather than written into the session that
   * now owns the tab.
   */
  association?: TabAssociation
}

type SendOutcome =
  | { kind: 'success' }
  | { kind: 'legacy' }
  | { kind: 'unknown-tab' }
  | { kind: 'oversize'; error: unknown }
  | { kind: 'transient'; error: unknown }
type TerminalSendOutcome = Exclude<
  SendOutcome,
  { kind: 'legacy' } | { kind: 'transient'; error: unknown }
>

export const RECORDINGS_QUEUE_MAX_BYTES = 2 * RECORDING_INGEST_MAX_BYTES
const LEGACY_TTL_MS = 10 * 60_000
const RETRY_INTERVAL_MS = 5_000
const WARNING_INTERVAL_MS = 60_000
const CAPABILITY_CACHE_TTL_MS = 60_000
const OVERSIZE_RECOVERY_INTERVAL_MS = 60_000

/**
 * Identity of the recording stream a tab's events belong to, as reported
 * by the canonical tab listing. Chrome tab ids are reused when a tab is
 * reattached to a new session, so this trio — not the tab id — decides
 * where a batch may land: `sessionId` scopes the ingest URL, and the
 * page/target ids travel as headers for the server to re-validate
 * against its live registry (mismatch comes back as 409).
 */
interface TabAssociation {
  sessionId: string
  pageId: number
  targetId: string
}

/**
 * Session-lived delivery boundary between recorder content scripts and the
 * local recordings ingest. It preserves each tab's rrweb order in memory and
 * reports recovered gaps so the background can request a fresh checkpoint.
 */
export function createRecordingsRelay(
  options: RecordingsRelayOptions,
): RecordingsRelay {
  const fetch = options.fetch ?? globalThis.fetch
  const now = options.now ?? Date.now
  const warn = options.warn ?? console.warn
  const setTimer = options.setTimeout ?? globalThis.setTimeout
  const clearTimer = options.clearTimeout ?? globalThis.clearTimeout
  const encoder = new TextEncoder()
  const queues = new Map<number, QueuedBatch[]>()
  const queuedBytesByTab = new Map<number, number>()
  const sendingTabs = new Set<number>()
  const sendingQueuedBatchIds = new Set<string>()
  const gappedTabs = new Set<number>()
  const oversizeGappedTabs = new Set<number>()
  const recoveredListeners = new Set<(tabId: number) => void>()
  const lastWarningAt = new Map<string, number>()
  const lastOversizeRecoveryAt = new Map<number, number>()
  const ingestLimits = new Map<
    string,
    { maxBytes: number; expiresAt: number }
  >()
  let legacyUntil = 0
  let totalBytes = 0
  let queuedBatchCount = 0
  let retryTimer: TimerHandle | null = null
  let drainPromise: Promise<void> | null = null
  let deliveryInterrupted = false
  // Last known association per tab id, kept so batches queued while the
  // server is unreachable pin to the session that produced them, not to
  // whichever session owns the tab once delivery resumes.
  const associations = new Map<number, TabAssociation>()

  function safeWarn(...args: unknown[]): void {
    try {
      warn(...args)
    } catch {
      // Logging must not change delivery behavior.
    }
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

  function cancelRetry(): void {
    if (retryTimer === null) return
    clearTimer(retryTimer)
    retryTimer = null
  }

  function markDeliveryInterrupted(): void {
    if (!deliveryInterrupted) {
      deliveryInterrupted = true
      safeWarn('[browseros-claw replay] delivery interrupted; events queued')
    }
  }

  function reportDeliveryRecovered(): void {
    if (deliveryInterrupted && queuedBatchCount === 0) {
      deliveryInterrupted = false
      safeWarn('[browseros-claw replay] queued event delivery recovered')
    }
  }

  function addBatch(tabId: number, batch: QueuedBatch, atFront = false): void {
    batch.association ??= associations.get(tabId)
    const queue = queues.get(tabId)
    if (queue) {
      if (atFront) queue.unshift(batch)
      else queue.push(batch)
    } else {
      queues.set(tabId, [batch])
    }
    queuedBytesByTab.set(
      tabId,
      (queuedBytesByTab.get(tabId) ?? 0) + batch.bytes,
    )
    totalBytes += batch.bytes
    queuedBatchCount++
    enforceQueueBudget()
  }

  function removeBatchAt(tabId: number, index: number): QueuedBatch | null {
    const queue = queues.get(tabId)
    const batch = queue?.[index]
    if (!queue || !batch) return null
    queue.splice(index, 1)
    if (queue.length === 0) queues.delete(tabId)
    const remainingBytes = (queuedBytesByTab.get(tabId) ?? 0) - batch.bytes
    if (remainingBytes > 0) queuedBytesByTab.set(tabId, remainingBytes)
    else queuedBytesByTab.delete(tabId)
    totalBytes -= batch.bytes
    queuedBatchCount--
    reportDeliveryRecovered()
    if (queuedBatchCount === 0) cancelRetry()
    return batch
  }

  function removeBatch(tabId: number, batchId: string): QueuedBatch | null {
    const index =
      queues.get(tabId)?.findIndex((batch) => batch.batchId === batchId) ?? -1
    return index === -1 ? null : removeBatchAt(tabId, index)
  }

  function clearQueues(): void {
    if (queuedBatchCount === 0) return
    queues.clear()
    queuedBytesByTab.clear()
    totalBytes = 0
    queuedBatchCount = 0
    deliveryInterrupted = false
    cancelRetry()
  }

  function enforceQueueBudget(): void {
    while (totalBytes > RECORDINGS_QUEUE_MAX_BYTES) {
      let eviction:
        | { tabId: number; batchIndex: number; queuedBytes: number }
        | undefined
      for (const [tabId, queue] of queues) {
        const batchIndex = queue.findIndex(
          (batch) => !sendingQueuedBatchIds.has(batch.batchId),
        )
        if (batchIndex === -1) continue
        const queuedBytes = queuedBytesByTab.get(tabId) ?? 0
        if (!eviction || queuedBytes > eviction.queuedBytes) {
          eviction = { tabId, batchIndex, queuedBytes }
        }
      }
      if (!eviction) return

      // Evict from the largest producer so one hot tab cannot starve all others.
      removeBatchAt(eviction.tabId, eviction.batchIndex)
      markGap(eviction.tabId)
      warnRateLimited(
        'queue-eviction',
        '[browseros-claw replay] recording batch evicted under queue pressure',
        `tab ${eviction.tabId}`,
      )
    }
  }

  function makeBatch(ndjson: string): QueuedBatch {
    return {
      batchId: crypto.randomUUID(),
      ndjson,
      bytes: encoder.encode(ndjson).byteLength,
    }
  }

  function makeBatches(ndjson: string): QueuedBatch[] {
    const sourceLines = ndjson.split('\n')
    if (sourceLines.length > 1 && sourceLines.at(-1) === '') sourceLines.pop()
    if (sourceLines.length === 0) return [makeBatch('')]

    const batches: QueuedBatch[] = []
    let batchLines: string[] = []
    let batchBytes = 0
    const flush = () => {
      if (batchLines.length === 0) return
      batches.push(makeBatch(batchLines.join('\n')))
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
        batches.push(makeBatch(line))
        continue
      }
      if (batchLines.length > 0) batchBytes++
      batchLines.push(line)
      batchBytes += lineBytes
    }
    flush()
    return batches
  }

  function markGap(tabId: number): void {
    gappedTabs.add(tabId)
  }

  function markOversizeGap(
    tabId: number,
    batch: QueuedBatch,
    error: unknown,
  ): void {
    gappedTabs.add(tabId)
    oversizeGappedTabs.add(tabId)
    warnRateLimited(
      'oversize-send',
      '[browseros-claw replay] recording batch exceeds ingest limit; replay gap recorded',
      new Error(
        `${batch.bytes.toString()} byte batch: ${error instanceof Error ? error.message : String(error)}`,
      ),
    )
  }

  function notifyRecovered(tabId: number): void {
    if (!gappedTabs.has(tabId)) return
    if (oversizeGappedTabs.has(tabId)) {
      const timestamp = now()
      const lastAt = lastOversizeRecoveryAt.get(tabId)
      if (
        lastAt !== undefined &&
        timestamp - lastAt < OVERSIZE_RECOVERY_INTERVAL_MS
      ) {
        return
      }
      lastOversizeRecoveryAt.set(tabId, timestamp)
      oversizeGappedTabs.delete(tabId)
    }
    gappedTabs.delete(tabId)
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

  function markDeliverySuccess(tabId: number): void {
    lastWarningAt.delete('transient-send')
    notifyRecovered(tabId)
  }

  function handleTerminalOutcome(
    tabId: number,
    batch: QueuedBatch,
    outcome: TerminalSendOutcome,
  ): void {
    if (outcome.kind === 'unknown-tab') {
      markGap(tabId)
    } else if (outcome.kind === 'oversize') {
      markOversizeGap(tabId, batch, outcome.error)
    } else {
      markDeliverySuccess(tabId)
    }
  }

  async function sendBatch(
    tabId: number,
    batch: QueuedBatch,
  ): Promise<SendOutcome> {
    try {
      const baseUrl = await options.resolveServerBaseUrl()
      const client = new DefaultApi(
        new Configuration({ basePath: baseUrl, fetchApi: fetch }),
      )
      if (batch.bytes > RECORDING_INGEST_FALLBACK_MAX_BYTES) {
        const maxBytes = await discoverIngestLimit(baseUrl, client)
        if (maxBytes !== undefined && batch.bytes > maxBytes) {
          return {
            kind: 'oversize',
            error: new Error(
              `server accepts at most ${maxBytes.toString()} bytes`,
            ),
          }
        }
      }
      const tab = (await client.listTabs()).items.find(
        (candidate) =>
          candidate.tabId === tabId && typeof candidate.sessionId === 'string',
      )
      if (!tab?.sessionId) {
        associations.delete(tabId)
        return { kind: 'unknown-tab' }
      }
      const association = rememberAssociation(tabId, tab)
      if (
        batch.association &&
        !associationsMatch(batch.association, association)
      ) {
        // The tab has moved on (new session/page/target) since these
        // events were recorded. Dropping beats leaking one session's
        // events into another's replay; the drain loop marks the gap
        // when it sees the unknown-tab outcome.
        return { kind: 'unknown-tab' }
      }
      batch.association = association
      // Batches enqueued before the tab was first resolved carry no pin;
      // they were recorded under this association, so stamp it now.
      for (const queuedBatch of queues.get(tabId) ?? []) {
        queuedBatch.association ??= association
      }
      const response = await fetch(
        `${baseUrl}/api/v1/sessions/${encodeURIComponent(batch.association.sessionId)}/recording/events`,
        {
          method: 'POST',
          headers: {
            'content-type': 'application/x-ndjson',
            'X-Recording-Batch-Id': batch.batchId,
            'X-Recording-Tab-Id': tabId.toString(),
            'X-Recording-Page-Id': association.pageId.toString(),
            'X-Recording-Target-Id': association.targetId,
          },
          body: batch.ndjson,
          credentials: 'omit',
        },
      )
      if ([404, 409, 410].includes(response.status)) {
        // The pinned session cannot take these events any more: gone
        // (404), association drifted server-side (409), or ended (410).
        // Forget the association so the next batch re-resolves the tab.
        associations.delete(tabId)
        return { kind: 'unknown-tab' }
      }
      if (response.status === 413) {
        return {
          kind: 'oversize',
          error: new Error('recordings ingest returned 413'),
        }
      }
      if (!response.ok) {
        return {
          kind: 'transient',
          error: new Error(`recordings ingest returned ${response.status}`),
        }
      }
      return { kind: 'success' }
    } catch (error) {
      // Only the generated client throws ResponseError, so a 404 here is
      // `listTabs` itself missing — a pre-canonical server. Back off via
      // legacy mode instead of treating every tab as unknown.
      if (error instanceof ResponseError && error.response.status === 404) {
        return { kind: 'legacy' }
      }
      return { kind: 'transient', error }
    }
  }

  async function discoverIngestLimit(
    baseUrl: string,
    client: DefaultApi,
  ): Promise<number | undefined> {
    const cached = ingestLimits.get(baseUrl)
    if (cached && now() < cached.expiresAt) return cached.maxBytes
    try {
      const system = await client.getSystemInfo()
      const advertised = system.capabilities?.recordingIngestMaxBytes
      const maxBytes =
        typeof advertised === 'number' &&
        Number.isSafeInteger(advertised) &&
        advertised > 0
          ? Math.min(advertised, RECORDINGS_QUEUE_MAX_BYTES)
          : RECORDING_INGEST_FALLBACK_MAX_BYTES
      ingestLimits.set(baseUrl, {
        maxBytes,
        expiresAt: now() + CAPABILITY_CACHE_TTL_MS,
      })
      return maxBytes
    } catch {
      // An unreachable capability endpoint is not evidence that the ingest
      // endpoint is old; let the actual POST decide instead of dropping early.
      return undefined
    }
  }

  function rememberAssociation(tabId: number, tab: Tab): TabAssociation {
    const association = {
      sessionId: tab.sessionId as string,
      pageId: tab.pageId,
      targetId: tab.targetId,
    }
    const previous = associations.get(tabId)
    if (previous && !associationsMatch(previous, association)) {
      // The tab was reattached mid-recording; mark the gap so the next
      // successful delivery fires the recovered listeners and the
      // background re-checkpoints the new stream.
      markGap(tabId)
    }
    associations.set(tabId, association)
    return association
  }

  function associationsMatch(
    left: TabAssociation,
    right: TabAssociation,
  ): boolean {
    return (
      left.sessionId === right.sessionId &&
      left.pageId === right.pageId &&
      left.targetId === right.targetId
    )
  }

  function markLegacy(triggeringTabId: number): void {
    // A legacy verdict can outlive the server process that produced it. Keep
    // dropped tabs gapped so a later endpoint can heal them after the TTL.
    markGap(triggeringTabId)
    for (const queuedTabId of queues.keys()) markGap(queuedTabId)
    legacyUntil = now() + LEGACY_TTL_MS
    clearQueues()
  }

  function armRetry(): void {
    if (queuedBatchCount === 0 || retryTimer !== null) return
    retryTimer = setTimer(() => {
      retryTimer = null
      return drainQueues()
    }, RETRY_INTERVAL_MS)
  }

  async function drainQueues(): Promise<void> {
    if (drainPromise) return drainPromise
    cancelRetry()
    const drain = async () => {
      let progressed = true
      while (progressed && queuedBatchCount > 0 && now() >= legacyUntil) {
        progressed = false
        for (const [tabId, queue] of [...queues]) {
          const batch = queue[0]
          if (!batch || sendingTabs.has(tabId)) continue
          sendingTabs.add(tabId)
          sendingQueuedBatchIds.add(batch.batchId)
          const outcome = await sendBatch(tabId, batch)

          if (outcome.kind === 'transient') {
            sendingTabs.delete(tabId)
            sendingQueuedBatchIds.delete(batch.batchId)
            enforceQueueBudget()
            markDeliveryInterrupted()
            warnRateLimited(
              'transient-send',
              '[browseros-claw replay] events POST failed',
              outcome.error,
            )
            return
          }

          removeBatch(tabId, batch.batchId)
          sendingTabs.delete(tabId)
          sendingQueuedBatchIds.delete(batch.batchId)
          enforceQueueBudget()
          progressed = true

          if (outcome.kind === 'legacy') {
            markLegacy(tabId)
            return
          }
          handleTerminalOutcome(tabId, batch, outcome)
        }
      }
    }

    drainPromise = drain().finally(() => {
      drainPromise = null
      armRetry()
    })
    return drainPromise
  }

  async function post(tabId: number, ndjson: string): Promise<void> {
    try {
      if (now() < legacyUntil) {
        markGap(tabId)
        return
      }
      const batches = makeBatches(ndjson)
      if ((queues.get(tabId)?.length ?? 0) > 0 || sendingTabs.has(tabId)) {
        for (const batch of batches) addBatch(tabId, batch)
        await drainQueues()
        return
      }

      const [batch, ...remainingBatches] = batches
      if (!batch) return
      sendingTabs.add(tabId)
      for (const remaining of remainingBatches) addBatch(tabId, remaining)
      const outcome = await sendBatch(tabId, batch)
      sendingTabs.delete(tabId)

      if (outcome.kind === 'legacy') {
        markLegacy(tabId)
        return
      }
      if (outcome.kind === 'transient') {
        if (now() >= legacyUntil) addBatch(tabId, batch, true)
        else markGap(tabId)
        markDeliveryInterrupted()
        warnRateLimited(
          'transient-send',
          '[browseros-claw replay] events POST failed',
          outcome.error,
        )
        armRetry()
        return
      }
      handleTerminalOutcome(tabId, batch, outcome)

      if ((queues.get(tabId)?.length ?? 0) > 0) await drainQueues()
    } catch (error) {
      sendingTabs.delete(tabId)
      warnRateLimited(
        'relay-internal',
        '[browseros-claw replay] relay failed unexpectedly',
        error,
      )
    }
  }

  return {
    post,
    onTabRecoveredAfterLoss(listener) {
      recoveredListeners.add(listener)
      return () => recoveredListeners.delete(listener)
    },
  }
}
