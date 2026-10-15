// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Standalone compatibility CLIs are never Turbo-cached.
/** Runs identical model transcripts through actual servers and compares POST, replay and persisted output. */
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import {
  type BootedServer,
  bootServer,
  parseKinds,
  runCli,
  trustedOrigin,
} from './boot'
import { type Scenario, scenarios, startFakeProvider } from './fake-provider'
import {
  diffBytes,
  normalizeBuiltinConversation,
  normalizeSse,
  parseSse,
  validateChunks,
} from './wire'

export const headers = {
  Origin: trustedOrigin,
  'Content-Type': 'application/json',
}
const wireHeaders = [
  'content-type',
  'cache-control',
  'connection',
  'x-vercel-ai-ui-message-stream',
  'x-accel-buffering',
]

export async function jsonRequest(
  server: BootedServer,
  path: string,
  method = 'GET',
  body?: unknown,
) {
  const response = await fetch(`${server.url}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(20_000),
  })
  const value = await response.json()
  if (!response.ok)
    throw new Error(
      `${method} ${path}: ${response.status} ${JSON.stringify(value)}`,
    )
  return value
}

async function capture(
  response: Response,
  onProgress?: (raw: string) => Promise<void>,
): Promise<string> {
  if (response.status !== 200 || !response.body)
    throw new Error(`Stream HTTP ${response.status}: ${await response.text()}`)
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let raw = ''
  try {
    while (true) {
      const part = await reader.read()
      if (part.done) break
      raw += decoder.decode(part.value, { stream: true })
      await onProgress?.(raw)
    }
    raw += decoder.decode()
    return raw
  } finally {
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}

function assertScenario(scenario: Scenario, raw: string) {
  const chunks = parseSse(raw)
  const has = (type: string) => chunks.some((chunk) => chunk.type === type)
  if (!has('start')) throw new Error(`${scenario}: missing start`)
  if (scenario === 'long-stream') {
    // TS stop cancels the registry reader before the SDK can publish abort (recon §11.2 race).
    // Keep those exact bytes as the oracle; lifecycle assertions below must still prove cancellation.
    if (
      chunks.filter((chunk) => chunk.type === 'text-delta').length !== 4 ||
      has('finish') ||
      has('error')
    )
      throw new Error('long-stream: wrong cancellation point')
    return
  }
  if (!has('finish')) throw new Error(`${scenario}: missing finish`)
  if (scenario === 'mid-stream-error') {
    if (
      !has('error') ||
      !chunks.some(
        (chunk) => chunk.type === 'finish' && chunk.finishReason === 'error',
      )
    )
      throw new Error('mid-stream-error: expected error and error finish')
    return
  }
  if (has('error')) throw new Error(`${scenario}: unexpected error ${raw}`)
  if (scenario === 'reasoning-text' && !has('reasoning-delta'))
    throw new Error('Missing reasoning')
  if (['one-tool', 'parallel-tools'].includes(scenario)) {
    const count = chunks.filter(
      (chunk) => chunk.type === 'tool-output-available',
    ).length
    if (count !== (scenario === 'parallel-tools' ? 2 : 1))
      throw new Error(`${scenario}: expected tool outputs; got ${count}`)
  }
  if (scenario === 'unknown-tool' && !has('tool-input-error'))
    throw new Error('Missing unknown-tool error')
  const text = chunks
    .filter((chunk) => chunk.type === 'text-delta')
    .map((chunk) => chunk.delta)
    .join('')
  if (
    text !==
    (['one-tool', 'parallel-tools', 'unknown-tool'].includes(scenario)
      ? 'Done.'
      : 'Hello fixture.')
  )
    throw new Error(`${scenario}: wrong final text ${JSON.stringify(text)}`)
}

async function panelHeartbeat(server: BootedServer): Promise<string[]> {
  const response = await fetch(`${server.url}/chat/panels`, {
    headers,
    signal: AbortSignal.timeout(10_000),
  })
  if (response.status !== 200 || !response.body)
    throw new Error(`Panels HTTP ${response.status}`)
  const reader = response.body.getReader()
  let raw = ''
  const decoder = new TextDecoder()
  try {
    while (raw.split('\n\n').length < 3) {
      const part = await reader.read()
      if (part.done) throw new Error('Panel SSE closed before heartbeat')
      raw += decoder.decode(part.value, { stream: true })
    }
    const frames = raw.split('\n\n').slice(0, 2)
    for (const frame of frames) JSON.parse(frame.replace(/^data: /, ''))
    return frames
  } finally {
    await reader.cancel()
    reader.releaseLock()
  }
}

async function runScenario(
  server: BootedServer,
  providerUrl: string,
  scenario: Scenario,
  session: string,
  output: string,
) {
  const providerId = `compat-${scenario}`
  const conversationId = '22222222-2222-4222-8222-222222222222'
  await jsonRequest(server, `/providers/${providerId}`, 'PUT', {
    type: 'openai-compatible',
    name: 'Compatibility fixture',
    modelId: scenario,
    baseUrl: `${providerUrl}/v1`,
    apiKey: 'fake-key',
    contextWindow: 32768,
    headers: { 'X-Compat-Session': session },
  })
  const response = await fetch(`${server.url}/chat`, {
    method: 'POST',
    headers,
    body: JSON.stringify({
      conversationId,
      providerId,
      message: 'Run the scripted compatibility scenario.',
      persist: true,
    }),
    signal: AbortSignal.timeout(40_000),
  })
  const streamHeaders = Object.fromEntries(
    wireHeaders.map((key) => [key, response.headers.get(key)]),
  )
  if (streamHeaders['x-vercel-ai-ui-message-stream'] !== 'v1')
    throw new Error('Missing AI SDK stream v1 header')
  let replay: Promise<string> | undefined
  let replayRunId: string | undefined
  let stopped = false
  const raw = await capture(response, async (prefix) => {
    if (!replay && prefix.includes('"type":"start"')) {
      const state = await jsonRequest(server, `/chat/${conversationId}/state`)
      if (state.status !== 'running')
        throw new Error(`${scenario}: reconnect missed the active run`)
      replayRunId = state.runId
      // Attach immediately, then consume concurrently: replay is the whole run, never a suffix.
      replay = fetch(
        `${server.url}/chat/${conversationId}/stream?runId=${encodeURIComponent(state.runId)}`,
        { headers, signal: AbortSignal.timeout(40_000) },
      ).then((response) => capture(response))
      // Attach a rejection handler now; a POST failure must not leave an unhandled replay rejection.
      void replay.catch(() => {})
      const status = await jsonRequest(server, '/status')
      if (status.can_update !== false)
        throw new Error('Active run incorrectly permits update')
    }
    if (
      scenario === 'long-stream' &&
      !stopped &&
      (prefix.match(/"type":"text-delta"/g)?.length ?? 0) >= 4
    ) {
      stopped = true
      const result = await jsonRequest(
        server,
        `/chat/${conversationId}/stop`,
        'POST',
      )
      if (result.stopped !== true)
        throw new Error('/stop did not find the active run')
    }
  })
  await writeFile(join(output, `${session}.post.sse`), raw)
  const replayRaw = await replay
  if (!replayRaw) throw new Error('Reconnect was not exercised')
  await writeFile(join(output, `${session}.replay.sse`), replayRaw)
  // Check before normalization. A tolerated F8 delta must still pass the client's own validator.
  await validateChunks(raw)
  await validateChunks(replayRaw)
  assertScenario(scenario, raw)
  const replayDiff = diffBytes(raw, replayRaw)
  if (replayDiff)
    throw new Error(`${scenario}: POST/reconnect differs: ${replayDiff}`)
  const stale = await fetch(
    `${server.url}/chat/${conversationId}/stream?runId=stale`,
    { headers, signal: AbortSignal.timeout(5_000) },
  )
  if (stale.status !== 409)
    throw new Error(`Stale runId expected 409, got ${stale.status}`)
  let state = await jsonRequest(server, `/chat/${conversationId}/state`)
  for (let i = 0; i < 100 && state.status === 'running'; i++) {
    await Bun.sleep(20)
    state = await jsonRequest(server, `/chat/${conversationId}/state`)
  }
  if (scenario === 'long-stream' && (!stopped || state.status !== 'aborted'))
    throw new Error('/stop did not leave the run aborted')
  if (state.status === 'running' || state.runId !== replayRunId)
    throw new Error('Run did not finish with the original identity')
  if ((await jsonRequest(server, '/status')).can_update !== true)
    throw new Error('Finished run still blocks update')
  const persistedResponse = await fetch(
    `${server.url}/conversations/${conversationId}`,
    { headers, signal: AbortSignal.timeout(5_000) },
  )
  const conversation = {
    status: persistedResponse.status,
    body: await persistedResponse.json(),
  }
  if (
    persistedResponse.status !== 200 &&
    !(scenario === 'long-stream' && persistedResponse.status === 404)
  )
    throw new Error(`Persisted conversation HTTP ${persistedResponse.status}`)
  // Current TS cancellation may close before onFinish persists anything. Keep absence in the differential receipt.
  await writeFile(
    join(output, `${session}.conversation.json`),
    JSON.stringify(conversation, null, 2),
  )
  await jsonRequest(server, `/chat/${conversationId}`, 'DELETE')
  if (persistedResponse.status === 200)
    await jsonRequest(server, `/conversations/${conversationId}`, 'DELETE')
  await jsonRequest(server, `/providers/${providerId}`, 'DELETE')
  return {
    post: normalizeSse(raw),
    replay: normalizeSse(replayRaw),
    headers: streamHeaders,
    conversation: {
      status: conversation.status,
      body: normalizeBuiltinConversation(conversation.body),
    },
  }
}

export async function main() {
  const { a, b } = parseKinds()
  const output = process.env.SERVER_COMPAT_OUTPUT
    ? resolve(process.env.SERVER_COMPAT_OUTPUT)
    : await mkdtemp(join(tmpdir(), 'server-compat-streams-'))
  await mkdir(output, { recursive: true })
  console.log(`Artifacts: ${output}`)
  const fake = startFakeProvider({ pauseLongAfterDeltas: 4 })
  const results: Array<Map<string, unknown>> = []
  try {
    for (const [index, kind] of [a, b].entries()) {
      const server = await bootServer(kind)
      try {
        const result = new Map<string, unknown>()
        result.set('panels', await panelHeartbeat(server))
        for (const scenario of scenarios) {
          result.set(
            scenario,
            await runScenario(
              server,
              fake.url,
              scenario,
              `${index}-${scenario}`,
              output,
            ),
          )
          for (let i = 0; i < 100 && fake.activeStreams; i++)
            await Bun.sleep(20)
          if (fake.activeStreams)
            throw new Error(
              `${scenario}: upstream work survived completion/stop`,
            )
          console.log(
            `PASS ${index === 0 ? 'A' : 'B'} ${kind} ${scenario}: POST, mid-run replay, SDK, persistence, idle`,
          )
        }
        results.push(result)
      } finally {
        await server.stop()
      }
    }
    let failures = 0
    for (const [name, value] of results[0]) {
      const other = results[1].get(name)
      const difference = diffBytes(JSON.stringify(value), JSON.stringify(other))
      if (difference) {
        console.error(`FAIL ${name}: ${difference}`)
        failures++
      } else console.log(`PASS diff ${name}`)
    }
    // A nominal retry success is insufficient: prove that each side saw a 429 and made another request.
    for (const scenario of scenarios) {
      const requests = fake.requests.filter(
        (request) => request.scenario === scenario,
      )
      if (
        scenario === 'retry-429' &&
        (requests.filter((r) => r.status === 429).length !== 2 ||
          requests.filter((r) => r.status === 200).length !== 2)
      )
        throw new Error(
          'Retry scenario did not execute one 429 and one successful retry per server',
        )
      if (requests.length < 2)
        throw new Error(
          `${scenario}: fake upstream not reached by both servers`,
        )
    }
    await writeFile(
      join(output, 'summary.json'),
      JSON.stringify(
        {
          a,
          b,
          scenarios: scenarios.length,
          failures,
          requests: fake.requests,
        },
        null,
        2,
      ),
    )
    if (failures)
      throw new Error(`${failures} differential stream failures; see ${output}`)
    console.log(
      `PASS ${a}-vs-${b}: ${scenarios.length} scenarios and panel heartbeat`,
    )
  } finally {
    await fake.stop()
  }
}
if (import.meta.main) await runCli(main)
