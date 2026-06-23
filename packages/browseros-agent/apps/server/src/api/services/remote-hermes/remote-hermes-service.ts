/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * High-level Remote Hermes service. Owns the lifetime of the WS bridge
 * and exposes a small set of methods the HTTP routes orchestrate:
 *
 *   - warm()      → fire-and-forget /v1/laptop/vm/start
 *   - teardown()  → fire-and-forget /v1/laptop/vm/destroy
 *   - status()    → /v1/laptop/vm/status (passthrough)
 *   - streamTurn  → AI SDK UIMessageStreamResponse from a chat turn
 *   - close()     → graceful WS shutdown (called from Application.shutdown)
 *
 * No env / JWT / fetch lives here — those are all in RemoteHermesClient.
 *
 * Wire format end-to-end is the AI SDK UI Message Stream protocol. The
 * VM produces it via `streamText().toUIMessageStream()`; the worker
 * proxies the SSE bytes unchanged; this service parses one `data: …` line
 * at a time and forwards each JSON object straight into the writer. No
 * translation lives anywhere on the path.
 */

import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import {
  createUIMessageStream,
  createUIMessageStreamResponse,
  type UIMessageStreamWriter,
} from 'ai'
import { formatUserMessage } from '../../../agent/format-message'
import {
  COLD_START_BUDGET_MS,
  STATUS_POLL_INTERVAL_MS,
} from '../../../lib/clients/remote-hermes/constants'
import type {
  PostTurnInput,
  RemoteHermesClient,
  VmStatusView,
} from '../../../lib/clients/remote-hermes/remote-hermes-client'
import { WsBridge } from '../../../lib/clients/remote-hermes/ws-bridge'
import { logger } from '../../../lib/logger'

const MODULE = 'remote-hermes'

export interface RemoteHermesServiceDeps {
  client: RemoteHermesClient
  resolveLocalMcpUrl(server: string): string | null
}

export interface StreamTurnInput {
  conversationId: string
  message: string
  modelId?: string | null
  /** Resolved by the route — pageIds already filled in via
   *  resolveBrowserContextPageIds before the service is called. */
  browserContext?: BrowserContext
  selectedText?: string
  selectedTextSource?: { url: string; title: string }
}

export class RemoteHermesService {
  private readonly client: RemoteHermesClient
  private readonly bridge: WsBridge

  constructor(deps: RemoteHermesServiceDeps) {
    this.client = deps.client
    this.bridge = new WsBridge({
      client: deps.client,
      resolveLocalMcpUrl: deps.resolveLocalMcpUrl,
    })
  }

  /**
   * Provider-save side-effect. Best-effort warm-start of the VM. Throws
   * on upstream failure so the route's `.catch` surfaces a real log
   * line — the UI doesn't block on this, so failure is non-fatal.
   */
  async warm(): Promise<void> {
    const res = await this.client.startVm()
    if (!res.ok) {
      throw new Error(
        `Remote Hermes /vm/start failed: ${res.status} ${await safeReadText(res)}`,
      )
    }
    logger.info('Remote Hermes /vm/start dispatched', {
      module: MODULE,
      status: res.status,
    })
  }

  /** Provider-delete side-effect. Best-effort destroy of the VM. */
  async teardown(): Promise<void> {
    const res = await this.client.destroyVm()
    if (!res.ok) {
      throw new Error(
        `Remote Hermes /vm/destroy failed: ${res.status} ${await safeReadText(res)}`,
      )
    }
    logger.info('Remote Hermes /vm/destroy dispatched', {
      module: MODULE,
      status: res.status,
    })
  }

  /** Passthrough to the worker. Used by /remote-hermes/status diagnostics. */
  async status(signal?: AbortSignal): Promise<VmStatusView> {
    return this.client.getVmStatus(signal)
  }

  /**
   * The /chat endpoint forwards `remote-hermes` turns here. Returns an
   * AI SDK UIMessageStreamResponse that the side panel reads identically
   * to any other provider's stream.
   */
  streamTurn(input: StreamTurnInput, abortSignal: AbortSignal): Response {
    const stream = createUIMessageStream({
      execute: async ({ writer }) => {
        await this.bridge.withTurn(async () => {
          const taskId = await this.openTurn(input, writer, abortSignal)
          if (!taskId) return
          await this.pumpEvents(taskId, writer, abortSignal)
        })
      },
      onError: (err) =>
        `Remote Hermes error: ${err instanceof Error ? err.message : String(err)}`,
    })
    return createUIMessageStreamResponse({ stream })
  }

  /** For Application.shutdown(). */
  close(): void {
    this.bridge.close()
  }

  /** Diagnostics passthrough. */
  snapshotBridge() {
    return this.bridge.snapshot()
  }

  private async openTurn(
    input: StreamTurnInput,
    writer: UIMessageStreamWriter,
    signal: AbortSignal,
  ): Promise<string | null> {
    // Phase 1: ensure the VM is up. Branches by current status:
    //   - error     → /vm/destroy then /vm/start (wipe bad state)
    //   - cold      → /vm/start
    //   - stopped   → /vm/start (warmStart in-place restart)
    //   - starting  → just poll, provision already in flight
    //   - running   → skip start, go straight to postTurn
    // Any transition takes the boot poll path so the side-panel pill
    // updates while the VM warms.
    const initial = await this.safeGetStatus(signal)
    if (initial?.status === 'error') {
      logger.warn(
        'Remote Hermes status=error before turn; wiping and restarting',
        {
          module: MODULE,
          lastError: initial.lastError?.message ?? 'unknown',
        },
      )
      await this.client.destroyVm(signal).catch((err) =>
        logger.warn('Remote Hermes destroy during recovery failed', {
          module: MODULE,
          err: err instanceof Error ? err.message : String(err),
        }),
      )
    }
    if (initial?.status !== 'running') {
      const started = await this.ensureStarted(initial?.status, writer, signal)
      if (!started) return null
    }

    // Wrap the user message with the same browser-context block local
    // chat injects, so the remote LLM has windowId/tab/pageId metadata
    // and selected-text framing for tool routing.
    const turnPayload = buildTurnPayload(input)

    // Phase 2: optimistic turn.
    const first = await this.client.postTurn(turnPayload, signal)
    if (first.ok) return readTaskId(first, writer)

    // Attach path: a 409 with `turn_in_progress` means the worker
    // already has a live task for this thread (e.g. the side panel was
    // refreshed mid-stream and resent the same turn). Don't restart,
    // don't retry — subscribe to the existing task's SSE stream so the
    // user picks up the answer in flight.
    if (first.status === 409) {
      const activeTaskId = await readActiveTaskId(first.clone())
      if (activeTaskId) {
        logger.info('Remote Hermes attaching to in-flight task', {
          module: MODULE,
          taskId: activeTaskId,
        })
        return activeTaskId
      }
    }

    // Phase 3: drift recovery. If the worker proxy returned 503/409
    // even though /vm/status said running, the DO record drifted from
    // Fly reality between status read and turn post. /vm/start
    // reconciles in the DO and re-provisions if needed; one retry.
    if (first.status === 503 || first.status === 409) {
      logger.warn(
        'Remote Hermes turn returned mid-stream cold response; forcing reconcile',
        {
          module: MODULE,
          status: first.status,
        },
      )
      const recovered = await this.ensureStarted('cold', writer, signal)
      if (!recovered) return null
      const retry = await this.client.postTurn(turnPayload, signal)
      if (retry.ok) return readTaskId(retry, writer)
      writeUpstreamError(writer, await retry.text(), retry.status)
      writeBootStatus(writer, 'error')
      return null
    }

    writeUpstreamError(writer, await first.text(), first.status)
    return null
  }

  /**
   * Fire /vm/start (idempotent on the DO — no-op if already running)
   * and poll /vm/status until running or budget exceeded. Returns true
   * on ready, false on timeout / abort / error. Emits boot-status
   * updates into the UI message stream so the side panel reflects
   * progress.
   */
  private async ensureStarted(
    knownStatus: string | undefined,
    writer: UIMessageStreamWriter,
    signal: AbortSignal,
  ): Promise<boolean> {
    if (knownStatus !== 'starting') {
      const startRes = await this.client.startVm(signal).catch((err) => {
        logger.warn('Remote Hermes /vm/start failed', {
          module: MODULE,
          err: err instanceof Error ? err.message : String(err),
        })
        return null
      })
      if (startRes && !startRes.ok) {
        writeUpstreamError(writer, await startRes.text(), startRes.status)
        writeBootStatus(writer, 'error')
        return false
      }
    }
    writeBootStatus(writer, 'booting')
    const ready = await this.pollUntilRunning(writer, signal)
    if (!ready) {
      writeBootStatus(writer, 'error')
      writer.write({
        type: 'error',
        errorText: `Remote Hermes VM did not become ready within ${COLD_START_BUDGET_MS / 1000} seconds. Try sending again.`,
      })
      return false
    }
    return true
  }

  /** Wraps getVmStatus in a swallow so phase-1 dispatch never crashes
   *  on a transient worker outage; we fall through to /vm/start and
   *  pollUntilRunning, which both have their own error surfaces. */
  private async safeGetStatus(
    signal: AbortSignal,
  ): Promise<VmStatusView | null> {
    try {
      return await this.client.getVmStatus(signal)
    } catch (err) {
      if (signal.aborted) return null
      logger.debug('Remote Hermes pre-turn status fetch failed', {
        module: MODULE,
        err: err instanceof Error ? err.message : String(err),
      })
      return null
    }
  }

  private async pollUntilRunning(
    writer: UIMessageStreamWriter,
    signal: AbortSignal,
  ): Promise<boolean> {
    const deadline = Date.now() + COLD_START_BUDGET_MS
    let lastProgress: string | undefined
    while (Date.now() < deadline) {
      if (signal.aborted) return false
      let view: VmStatusView | null = null
      try {
        view = await this.client.getVmStatus(signal)
      } catch (err) {
        if (signal.aborted) return false
        logger.debug('Remote Hermes status poll failed', {
          module: MODULE,
          err: err instanceof Error ? err.message : String(err),
        })
      }
      if (view) {
        if (view.status === 'running') return true
        if (view.status === 'error') {
          logger.warn('Remote Hermes VM error during boot poll', {
            module: MODULE,
            lastError: view.lastError?.message ?? 'unknown',
          })
          return false
        }
        if (view.progress && view.progress !== lastProgress) {
          lastProgress = view.progress
          writeBootStatus(writer, 'booting', view.progress)
        }
      }
      await sleep(STATUS_POLL_INTERVAL_MS, signal)
    }
    logger.warn('Remote Hermes cold-start budget exceeded', {
      module: MODULE,
      budgetMs: COLD_START_BUDGET_MS,
    })
    return false
  }

  private async pumpEvents(
    taskId: string,
    writer: UIMessageStreamWriter,
    clientAbort: AbortSignal,
  ): Promise<void> {
    let firstContentSeen = false
    const dismissBoot = () => {
      if (firstContentSeen) return
      firstContentSeen = true
      writeBootStatus(writer, 'running')
    }

    const upstreamAbort = new AbortController()
    const onClientAbort = () => {
      upstreamAbort.abort()
      void this.client.abortTask(taskId).catch((err) =>
        logger.debug('Remote Hermes abort POST failed', {
          module: MODULE,
          err: err instanceof Error ? err.message : String(err),
        }),
      )
    }
    if (clientAbort.aborted) {
      onClientAbort()
      return
    }
    clientAbort.addEventListener('abort', onClientAbort, { once: true })

    let sseRes: Response
    try {
      sseRes = await this.client.openTaskEvents(taskId, upstreamAbort.signal)
    } catch (err) {
      if (!upstreamAbort.signal.aborted) {
        writer.write({
          type: 'error',
          errorText: `Failed to subscribe to remote events: ${
            err instanceof Error ? err.message : String(err)
          }`,
        })
      }
      return
    }

    if (!sseRes.ok || !sseRes.body) {
      writeUpstreamError(writer, await safeReadText(sseRes), sseRes.status)
      return
    }

    try {
      for await (const data of readSseDataLines(sseRes.body)) {
        if (data === '[DONE]') break
        let part: Record<string, unknown>
        try {
          part = JSON.parse(data) as Record<string, unknown>
        } catch {
          logger.debug('Remote Hermes bad UI message stream JSON', {
            module: MODULE,
            preview: data.slice(0, 120),
          })
          continue
        }
        // Any real assistant signal dismisses the boot pill — the `start`
        // part marks the beginning of the assistant message, so anything
        // after counts as "running content arrived".
        if (firstContentSeen || part.type !== 'start') dismissBoot()
        writer.write(part as Parameters<typeof writer.write>[0])
      }
    } catch (err) {
      if (!upstreamAbort.signal.aborted) {
        writer.write({
          type: 'error',
          errorText: `Stream interrupted: ${err instanceof Error ? err.message : String(err)}`,
        })
      }
    } finally {
      // Stream may end (DONE, abort, error) before any non-`start` part
      // arrives — without this the boot pill strands at `booting` and the
      // side panel keeps spinning. dismissBoot is idempotent so the
      // normal first-content path stays a no-op here.
      dismissBoot()
    }
  }
}

/**
 * Emits the payload of each `data: …\n\n` SSE record. Ignores comments,
 * blank lines, and the `event:` field (the worker only uses `data:`).
 */
async function* readSseDataLines(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<string> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) {
        const line = extractDataLine(buffer)
        if (line !== null) yield line
        return
      }
      buffer += decoder.decode(value, { stream: true })
      while (true) {
        const idx = buffer.indexOf('\n\n')
        if (idx === -1) break
        const line = extractDataLine(buffer.slice(0, idx))
        buffer = buffer.slice(idx + 2)
        if (line !== null) yield line
      }
    }
  } finally {
    reader.releaseLock()
  }
}

function extractDataLine(record: string): string | null {
  const dataLines: string[] = []
  for (const line of record.split('\n')) {
    if (!line || line.startsWith(':')) continue
    if (line.startsWith('data:')) {
      dataLines.push(line.slice(5).replace(/^ /, ''))
    }
  }
  if (dataLines.length === 0) return null
  return dataLines.join('\n')
}

function buildTurnPayload(input: StreamTurnInput): PostTurnInput {
  // Only wrap when there's real context to embed. Without this guard
  // every remote-hermes turn would gain `<USER_QUERY>` framing
  // post-upgrade, changing the wire format for conversations that
  // don't attach tabs or selected text.
  const hasContext =
    Boolean(input.browserContext?.activeTab) ||
    Boolean(input.browserContext?.selectedTabs?.length) ||
    Boolean(input.selectedText)
  const message = hasContext
    ? formatUserMessage(
        input.message,
        input.browserContext,
        input.selectedText,
        input.selectedTextSource,
      )
    : input.message
  return {
    conversationId: input.conversationId,
    message,
    modelId: input.modelId,
  }
}

/** Parses a 409 turn-in-progress body. Returns the existing task id if
 *  the worker's body matches `{ error: "turn_in_progress", activeTaskId }`
 *  exactly; otherwise null so the caller can fall through to the
 *  generic drift-recovery path. */
async function readActiveTaskId(res: Response): Promise<string | null> {
  try {
    const body = (await res.json()) as {
      error?: unknown
      activeTaskId?: unknown
    }
    if (
      body.error === 'turn_in_progress' &&
      typeof body.activeTaskId === 'string'
    ) {
      return body.activeTaskId
    }
  } catch {
    // not JSON; fall through
  }
  return null
}

async function readTaskId(
  res: Response,
  writer: UIMessageStreamWriter,
): Promise<string | null> {
  let payload: unknown
  try {
    payload = await res.json()
  } catch {
    writeUpstreamError(writer, 'non-JSON turn response', res.status)
    return null
  }
  if (
    typeof payload === 'object' &&
    payload !== null &&
    typeof (payload as { taskId?: unknown }).taskId === 'string'
  ) {
    return (payload as { taskId: string }).taskId
  }
  writeUpstreamError(writer, 'turn response missing taskId', res.status)
  return null
}

function writeBootStatus(
  writer: UIMessageStreamWriter,
  status: 'booting' | 'running' | 'error',
  progress?: string,
): void {
  writer.write({
    type: 'data-vm-status',
    id: 'remote-hermes-vm-status',
    data: progress ? { status, progress } : { status },
    transient: true,
  })
}

function writeUpstreamError(
  writer: UIMessageStreamWriter,
  text: string,
  status: number,
): void {
  writer.write({
    type: 'error',
    errorText: `Remote Hermes upstream ${status}: ${text.slice(0, 240)}`,
  })
}

async function safeReadText(res: Response): Promise<string> {
  try {
    return await res.text()
  } catch {
    return ''
  }
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const onAbort = () => {
      clearTimeout(t)
      resolve()
    }
    const t = setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    signal.addEventListener('abort', onAbort, { once: true })
  })
}
