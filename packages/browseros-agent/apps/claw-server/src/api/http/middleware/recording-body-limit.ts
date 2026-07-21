/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { RECORDING_INGEST_MAX_BYTES } from '@browseros/shared/constants/limits'
import type { Context, MiddlewareHandler } from 'hono'
import { canonicalApiError } from '../errors'
import type { RequestContextEnv } from '../request-context'

/**
 * Bounds both declared-length and chunked uploads without retaining more than
 * one accepted request. Rejected streams are drained without retention because
 * Bun can leave the client fetch blocked when a partially read upload is cancelled.
 */
export function recordingBodyLimit(): MiddlewareHandler<RequestContextEnv> {
  return async (c, next) => {
    const body = c.req.raw.body
    if (!body) return next()

    const hasTransferEncoding = c.req.raw.headers.has('transfer-encoding')
    const contentLength = c.req.raw.headers.get('content-length')
    if (contentLength !== null && !hasTransferEncoding) {
      if (Number.parseInt(contentLength, 10) > RECORDING_INGEST_MAX_BYTES) {
        await drain(body.getReader())
        return payloadTooLarge(c)
      }
      return next()
    }

    let size = 0
    let oversized = false
    const chunks: Uint8Array[] = []
    const reader = body.getReader()
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      size += value.byteLength
      if (size > RECORDING_INGEST_MAX_BYTES) oversized = true
      if (!oversized) chunks.push(value)
    }
    if (oversized) return payloadTooLarge(c)
    const requestInit: RequestInit & { duplex: 'half' } = {
      body: new ReadableStream({
        start(controller) {
          for (const chunk of chunks) controller.enqueue(chunk)
          controller.close()
        },
      }),
      duplex: 'half',
    }
    c.req.raw = new Request(c.req.raw, requestInit)
    return next()
  }
}

async function drain(reader: ReadableStreamDefaultReader<Uint8Array>) {
  for (;;) {
    const { done } = await reader.read()
    if (done) return
  }
}

function payloadTooLarge(c: Context<RequestContextEnv, string>) {
  return c.json(
    canonicalApiError(
      'recording_payload_too_large',
      `recording payload exceeds ${RECORDING_INGEST_MAX_BYTES.toString()} byte limit`,
      c.get('requestId'),
    ),
    413,
  )
}
