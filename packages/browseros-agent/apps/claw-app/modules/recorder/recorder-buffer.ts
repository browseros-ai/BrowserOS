/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { RECORDING_INGEST_FALLBACK_MAX_BYTES } from '@browseros/shared/constants/limits'

export interface RecorderEventTarget {
  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject,
  ): void
}

export interface RecorderBufferOptions {
  send: (ndjson: string, hasGap: boolean) => void
  warnDropped?: (count: number) => void
  now?: () => number
  queueMicrotask?: (callback: () => void) => void
  setTimeout?: (callback: () => void, delay: number) => unknown
  clearTimeout?: (handle: unknown) => void
  bufferCap?: number
  flushAtSize?: number
  maxBatchBytes?: number
  flushIntervalMs?: number
}

export interface RecorderBuffer {
  emit: (event: unknown) => void
  flushNow: () => void
  close: () => void
}

const DEFAULT_BUFFER_CAP = 500
const DEFAULT_FLUSH_AT_SIZE = 50
const DEFAULT_FLUSH_INTERVAL_MS = 2_500

/** Buffers rrweb events and emits bounded NDJSON batches off the record callback. */
export function createRecorderBuffer(
  options: RecorderBufferOptions,
): RecorderBuffer {
  const bufferCap = options.bufferCap ?? DEFAULT_BUFFER_CAP
  const flushAtSize = options.flushAtSize ?? DEFAULT_FLUSH_AT_SIZE
  const maxBatchBytes =
    options.maxBatchBytes ?? RECORDING_INGEST_FALLBACK_MAX_BYTES
  const flushIntervalMs = options.flushIntervalMs ?? DEFAULT_FLUSH_INTERVAL_MS
  const now = options.now ?? Date.now
  const enqueueMicrotask =
    options.queueMicrotask ??
    (typeof queueMicrotask === 'function'
      ? queueMicrotask
      : (callback: () => void) => setTimeout(callback, 0))
  const schedule =
    options.setTimeout ??
    ((callback: () => void, delay: number) => setTimeout(callback, delay))
  const cancel =
    options.clearTimeout ??
    ((handle: unknown) => clearTimeout(handle as number))
  const encoder = new TextEncoder()
  const lines: Array<{ value: string; bytes: number }> = []
  const rawQueue: unknown[] = []
  let bufferedBytes = 0
  let dropped = 0
  let pendingSerialization = false
  let timer: unknown | null = null
  let closed = false

  function send(ndjson: string): void {
    const hasGap = dropped > 0
    if (dropped > 0) {
      options.warnDropped?.(dropped)
      dropped = 0
    }
    options.send(ndjson, hasGap)
  }

  function flush(): void {
    if (lines.length === 0) return
    const ndjson = lines.map(({ value }) => value).join('\n')
    lines.length = 0
    bufferedBytes = 0
    send(ndjson)
  }

  function appendLine(value: string): void {
    const bytes = encoder.encode(value).byteLength
    if (bytes > maxBatchBytes) {
      flush()
      send(value)
      return
    }
    const separatorBytes = lines.length > 0 ? 1 : 0
    if (
      lines.length > 0 &&
      bufferedBytes + separatorBytes + bytes > maxBatchBytes
    ) {
      flush()
    }
    if (lines.length >= bufferCap) {
      const removed = lines.shift()
      if (removed) {
        bufferedBytes -= removed.bytes + (lines.length > 0 ? 1 : 0)
      }
      dropped++
    }
    if (lines.length > 0) bufferedBytes++
    lines.push({ value, bytes })
    bufferedBytes += bytes
    if (lines.length >= flushAtSize) flush()
  }

  function armFlushTimer(): void {
    if (timer !== null || closed) return
    timer = schedule(() => {
      timer = null
      flush()
    }, flushIntervalMs)
  }

  function drainRawQueue(): void {
    pendingSerialization = false
    for (const event of rawQueue) {
      let line: string
      try {
        const rrwebEvent = event as {
          timestamp?: number
          type?: number
          data?: unknown
        }
        line = JSON.stringify({
          ts:
            typeof rrwebEvent.timestamp === 'number'
              ? rrwebEvent.timestamp
              : now(),
          type: rrwebEvent.type,
          data: rrwebEvent.data,
        })
      } catch {
        continue
      }
      appendLine(line)
    }
    rawQueue.length = 0
    armFlushTimer()
  }

  function flushNow(): void {
    if (rawQueue.length > 0) drainRawQueue()
    flush()
  }

  return {
    emit(event): void {
      if (closed) return
      if (rawQueue.length >= bufferCap) {
        rawQueue.shift()
        dropped++
      }
      rawQueue.push(event)
      if (pendingSerialization) return
      pendingSerialization = true
      enqueueMicrotask(drainRawQueue)
    },
    flushNow,
    close(): void {
      if (closed) return
      closed = true
      flushNow()
      if (timer !== null) {
        cancel(timer)
        timer = null
      }
    },
  }
}

/** Flushes buffered events before a document is hidden or discarded. */
export function installRecorderFlushListeners(options: {
  page: RecorderEventTarget
  document: RecorderEventTarget & { visibilityState: string }
  flush: () => void
}): void {
  options.page.addEventListener('pagehide', options.flush)
  options.document.addEventListener('visibilitychange', () => {
    if (options.document.visibilityState === 'hidden') options.flush()
  })
}
