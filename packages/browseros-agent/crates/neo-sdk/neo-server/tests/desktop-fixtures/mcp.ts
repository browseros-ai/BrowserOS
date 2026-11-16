import { CallToolResultSchema } from '@modelcontextprotocol/sdk/types.js'
import {
  McpRequestError,
  McpSession,
  readRpcResponse,
} from '../../../../../contracts/claw-mcp/tests/mcp-client'
import { jsonObject, jsonValue, stringField } from './json'
import {
  isRunId,
  type JsonObject,
  type RunReply,
  type RunRequest,
} from './model'

const RUN_META_KEY = 'com.browseros.neo/run'

function parameters(request: RunRequest) {
  return {
    name: 'run',
    arguments: { code: request.code, timeout: request.timeoutMs },
    _meta: {
      [RUN_META_KEY]: {
        taskId: request.taskId,
        requestId: request.requestId,
        hostData: { suite: 'sdk-desktop-fixtures' },
      },
    },
  }
}

/**
 * The budget probe keeps its SDK deadline independent of the driver's patience. A single
 * abort controller covers initialization, its notification, response bodies and the run;
 * expiry also seals the send boundary so late initialization can never submit a run.
 */
async function requestWithDriverDeadline(
  baseUrl: string,
  request: RunRequest,
  budget: number,
): Promise<unknown> {
  if (!Number.isSafeInteger(budget) || budget < 1 || budget > 30_000)
    throw new Error('Independent driver deadline must be 1..30000ms')
  const deadline = performance.now() + budget
  const controller = new AbortController()
  const expired = new Error(
    'Independent driver deadline expired; SDK completion is unknown',
  )
  let sessionId: string | undefined
  const ensureLive = () => {
    if (controller.signal.aborted || performance.now() >= deadline)
      throw expired
  }
  const post = async (message: object) => {
    ensureLive()
    return await fetch(`${baseUrl}/mcp`, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        accept: 'application/json, text/event-stream',
        ...(sessionId ? { 'mcp-session-id': sessionId } : {}),
      },
      body: JSON.stringify(message),
      signal: controller.signal,
    })
  }
  const responseValue = async (response: Response, id: number) => {
    if (!response.ok)
      throw new Error(`Driver MCP request failed: ${response.status}`)
    const message = await readRpcResponse(response, id)
    ensureLive()
    if (message.error)
      throw new McpRequestError(
        message.error.code,
        message.error.message,
        message.error.data,
      )
    return message.result
  }
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => {
      controller.abort(expired)
      reject(expired)
    }, budget)
  })
  const attempt = async () => {
    try {
      const initialized = await post({
        jsonrpc: '2.0',
        id: 1,
        method: 'initialize',
        params: {
          protocolVersion: '2025-03-26',
          capabilities: {},
          clientInfo: { name: 'sdk-desktop-fixtures', version: '0.0.1' },
        },
      })
      sessionId = initialized.headers.get('mcp-session-id') ?? undefined
      if (sessionId !== undefined) {
        await initialized.body?.cancel()
        throw new Error('Source SDK unexpectedly opened a stateful MCP session')
      }
      await responseValue(initialized, 1)
      const notified = await post({
        jsonrpc: '2.0',
        method: 'notifications/initialized',
      })
      await notified.body?.cancel()
      if (!notified.ok)
        throw new Error(`Driver MCP notification failed: ${notified.status}`)
      ensureLive()
      return await responseValue(
        await post({
          jsonrpc: '2.0',
          id: 2,
          method: 'tools/call',
          params: parameters(request),
        }),
        2,
      )
    } catch (error) {
      // A late response may reveal a stateful server after driver expiry. Close only that
      // session with a separately bounded cleanup request, preserving both failure causes.
      if (sessionId !== undefined) {
        try {
          const response = await fetch(`${baseUrl}/mcp`, {
            method: 'DELETE',
            headers: {
              'content-type': 'application/json',
              'mcp-session-id': sessionId,
            },
            signal: AbortSignal.timeout(1_000),
          })
          await response.body?.cancel()
          if (!response.ok)
            throw new Error(
              `Unexpected session cleanup failed: ${response.status}`,
            )
        } catch (cleanupError) {
          throw new AggregateError(
            [error, cleanupError],
            `${error instanceof Error ? error.message : String(error)}; ${cleanupError instanceof Error ? cleanupError.message : String(cleanupError)}`,
          )
        }
      }
      throw error
    }
  }
  const pending = attempt()
  try {
    return await Promise.race([pending, timeout])
  } catch (error) {
    // Cancellation seals sends immediately. Join the attempt before returning so any
    // separately bounded session cleanup remains owned and its failure reaches the caller.
    controller.abort(expired)
    try {
      await pending
    } catch (pendingError) {
      if (pendingError !== error && pendingError instanceof AggregateError) {
        throw new AggregateError(
          [error, pendingError],
          `${error instanceof Error ? error.message : String(error)}; ${pendingError.message}`,
        )
      }
    }
    throw error
  } finally {
    if (timer !== undefined) clearTimeout(timer)
    controller.abort()
  }
}

function requireCompletedRun(meta: JsonObject, text: string): void {
  // Script status and write delivery are independent: a script may catch an operation
  // error and return normally even though the page never confirmed the write.
  if (meta.effects === 'unknown')
    throw new Error('SDK reply has uncertain write effects')
  if (
    meta.effects !== undefined &&
    meta.effects !== 'none' &&
    meta.effects !== 'done'
  ) {
    throw new Error('SDK run metadata effects must be none, done, or unknown')
  }
  if (meta.unknown !== undefined) {
    if (!Array.isArray(meta.unknown))
      throw new Error('SDK run metadata unknown must be an array')
    if (meta.unknown.length > 0)
      throw new Error('SDK reply lists uncertain write operations')
  }

  // RunTool keeps unknown writes in _meta, but exposes not_awaited only through these
  // rendered lines. Match its complete operation label and cancellation sentence after
  // the first value/error line; JSON strings and ordinary console markers remain data.
  const unfinished =
    /^\[not awaited\] \S+(?: at run:\d+:\d+)? \(op \d+\) was still running when the script returned; it was cancelled\.$/
  if (
    text
      .split('\n')
      .slice(1)
      .some((line) => unfinished.test(line))
  ) {
    throw new Error('SDK reply reports unfinished operations')
  }
}

/** Decode the standard MCP result first, then the SDK-specific run envelope. */
export function parseRunReply(input: unknown): RunReply {
  const result = CallToolResultSchema.parse(input)
  const meta = jsonObject(result._meta?.[RUN_META_KEY], 'SDK run metadata')
  const runId = meta.id
  if (!isRunId(runId)) throw new Error('SDK reply omitted a valid run ID')
  const status = stringField(meta, 'status')
  const structured = jsonObject(
    result.structuredContent,
    'SDK structured result',
  )
  const text = result.content
    .filter((block) => block.type === 'text')
    .map((block) => block.text)
    .join('\n')
  const images = result.content
    .filter((block) => block.type === 'image')
    .map((block) => ({ data: block.data, mimeType: block.mimeType }))
  requireCompletedRun(meta, text)
  const base = { runId, status, text, images, raw: jsonValue(input) }
  if (result.isError === true) {
    if (status === 'ok') throw new Error('Error MCP reply reported status ok')
    const error = jsonObject(structured.error, 'SDK structured error')
    return {
      ...base,
      kind: 'error',
      isError: true,
      error: {
        owner: stringField(error, 'owner'),
        code: stringField(error, 'code'),
        message: stringField(error, 'message'),
      },
    }
  }
  if (status !== 'ok') {
    throw new Error(`Successful MCP reply reported status ${status}`)
  }
  if (!Object.hasOwn(structured, 'value')) {
    throw new Error('Successful MCP reply omitted structured value')
  }
  return { ...base, kind: 'ok', isError: false, value: structured.value }
}

/**
 * Each call initializes its own client, including the four overlapping captures. The source
 * server is stateless, so response EOF releases this client and there is no shared MCP session.
 */
export function createRunClient(baseUrl: string) {
  return async (request: RunRequest): Promise<RunReply> => {
    if (request.driverTimeoutMs !== undefined) {
      return parseRunReply(
        await requestWithDriverDeadline(
          baseUrl,
          request,
          request.driverTimeoutMs,
        ),
      )
    }
    const client = await McpSession.connect(baseUrl, 'sdk-desktop-fixtures')
    if (client.sessionId !== undefined) {
      await client.close()
      throw new Error('Source SDK unexpectedly opened a stateful MCP session')
    }
    const raw = await client.request(
      'tools/call',
      parameters(request),
      request.timeoutMs + 30_000,
    )
    return parseRunReply(raw)
  }
}
