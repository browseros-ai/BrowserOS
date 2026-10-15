// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Standalone compatibility CLIs are never Turbo-cached.
/** Scripted upstream protocols isolate server translation from model randomness and real credentials. */
import { eventsFor } from './fake-protocols'

export const scenarios = [
  'plain-text',
  'reasoning-text',
  'one-tool',
  'parallel-tools',
  'unknown-tool',
  'retry-429',
  'mid-stream-error',
  'long-stream',
] as const
export type Scenario = (typeof scenarios)[number]
type Json = Record<string, unknown>

/** Each session header owns retry state; two compared servers cannot consume each other's 429. */
export function startFakeProvider(
  options: { port?: number; delayMs?: number } = {},
) {
  const attempts = new Map<string, number>()
  const active = new Set<symbol>()
  const requests: Array<{
    path: string
    scenario: Scenario
    followup: boolean
    status: number
  }> = []
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: options.port ?? 0,
    async fetch(request) {
      const path = new URL(request.url).pathname
      if (
        request.method !== 'POST' ||
        !['/v1/chat/completions', '/v1/responses', '/v1/messages'].includes(
          path,
        )
      )
        return Response.json({ error: 'Unknown endpoint' }, { status: 404 })
      let body: Json
      try {
        body = (await request.json()) as Json
      } catch {
        return Response.json({ error: 'Malformed JSON' }, { status: 400 })
      }
      const scenario = body.model as Scenario
      if (!scenarios.includes(scenario))
        return Response.json(
          { error: `Unknown scenario ${String(body.model)}` },
          { status: 400 },
        )
      // Inspect protocol structure instead of searching text: user prompts can contain these words.
      const messages = (body.messages ?? body.input ?? []) as Json[]
      const followup =
        Array.isArray(messages) &&
        messages.some(
          (message) =>
            message.role === 'tool' ||
            message.type === 'function_call_output' ||
            (Array.isArray(message.content) &&
              message.content.some(
                (part: Json) => part.type === 'tool_result',
              )),
        )
      const key = `${request.headers.get('X-Compat-Session') ?? 'default'}:${path}:${scenario}`
      const attempt = (attempts.get(key) ?? 0) + 1
      attempts.set(key, attempt)
      const status = scenario === 'retry-429' && attempt === 1 ? 429 : 200
      requests.push({ path, scenario, followup, status })
      if (status === 429)
        return Response.json(
          {
            error: {
              message: 'scripted retry',
              type: 'rate_limit_error',
              code: 'rate_limit_exceeded',
            },
          },
          { status, headers: { 'Retry-After': '0' } },
        )
      const events = eventsFor(path, scenario, followup)
      const streamId = Symbol('upstream')
      active.add(streamId)
      const finish = () => active.delete(streamId)
      request.signal.addEventListener('abort', finish, { once: true })
      let cursor = 0
      let cancelled = false
      const stream = new ReadableStream<Uint8Array>({
        async pull(controller) {
          // pull honors cancellation and backpressure, so /stop cannot leave an upstream timer running.
          const delay =
            options.delayMs ?? (scenario === 'long-stream' ? 80 : 15)
          if (delay) await Bun.sleep(delay)
          if (cancelled || request.signal.aborted) {
            finish()
            controller.close()
            return
          }
          const event = events[cursor++]
          if (!event) {
            finish()
            request.signal.removeEventListener('abort', finish)
            controller.close()
            return
          }
          const data =
            event.data === '[DONE]' ? event.data : JSON.stringify(event.data)
          controller.enqueue(
            new TextEncoder().encode(
              `${event.event ? `event: ${event.event}\n` : ''}data: ${data}\n\n`,
            ),
          )
        },
        cancel() {
          cancelled = true
          finish()
          request.signal.removeEventListener('abort', finish)
        },
      })
      return new Response(stream, {
        headers: {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          ...(path.endsWith('/messages')
            ? { 'anthropic-version': '2023-06-01' }
            : {}),
        },
      })
    },
  })
  return {
    url: `http://127.0.0.1:${server.port}`,
    requests,
    get activeStreams() {
      return active.size
    },
    stop: async () => {
      await server.stop(true)
    },
  }
}

if (import.meta.main) {
  const fake = startFakeProvider({
    port: Number(process.env.SERVER_COMPAT_PROVIDER_PORT ?? 0),
  })
  console.log(`Fake provider: ${fake.url}`)
  for (const signal of ['SIGINT', 'SIGTERM'] as const)
    process.once(signal, async () => {
      await fake.stop()
      process.exit(0)
    })
}
