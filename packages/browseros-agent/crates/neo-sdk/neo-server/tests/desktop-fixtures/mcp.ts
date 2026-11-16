import { CallToolResultSchema } from '@modelcontextprotocol/sdk/types.js'
import { McpSession } from '../../../../../contracts/claw-mcp/tests/mcp-client'
import { jsonObject, jsonValue, stringField } from './json'
import {
  isRunId,
  type JsonObject,
  type RunReply,
  type RunRequest,
} from './model'

const RUN_META_KEY = 'com.browseros.neo/run'

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
    const client = await McpSession.connect(baseUrl, 'sdk-desktop-fixtures')
    if (client.sessionId !== undefined) {
      await client.close()
      throw new Error('Source SDK unexpectedly opened a stateful MCP session')
    }
    const raw = await client.request(
      'tools/call',
      {
        name: 'run',
        arguments: { code: request.code, timeout: request.timeoutMs },
        _meta: {
          [RUN_META_KEY]: {
            taskId: request.taskId,
            requestId: request.requestId,
            hostData: { suite: 'sdk-desktop-fixtures' },
          },
        },
      },
      request.timeoutMs + 30_000,
    )
    return parseRunReply(raw)
  }
}
