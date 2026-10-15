/** Scripted upstream protocols isolate server translation from model randomness and real credentials. */
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
interface Event {
  data: Json | '[DONE]'
  event?: string
}
const toolInput = {
  query: 'Compatibility fixture',
  suggestedName: 'Fixture',
  scheduleType: 'daily',
  scheduleTime: '09:00',
}

function calls(scenario: Scenario) {
  if (!['one-tool', 'parallel-tools', 'unknown-tool'].includes(scenario))
    return []
  return Array.from(
    { length: scenario === 'parallel-tools' ? 2 : 1 },
    (_, index) => ({
      id: `call_${index + 1}`,
      name:
        scenario === 'unknown-tool'
          ? 'compat_unknown_tool'
          : 'suggest_schedule',
      arguments: JSON.stringify(
        scenario === 'unknown-tool'
          ? {}
          : { ...toolInput, suggestedName: `Fixture ${index + 1}` },
      ),
    }),
  )
}

function chatEvents(scenario: Scenario, followup: boolean): Event[] {
  const chunk = (delta: Json, finish: string | null = null): Event => ({
    data: {
      id: 'chatcmpl_compat',
      object: 'chat.completion.chunk',
      created: 1_700_000_000,
      model: scenario,
      choices: [{ index: 0, delta, finish_reason: finish }],
    },
  })
  const events = [chunk({ role: 'assistant', content: '' })]
  if (scenario === 'reasoning-text' && !followup)
    events.push(chunk({ reasoning_content: 'Thinking.' }))
  const tools = followup ? [] : calls(scenario)
  if (tools.length) {
    // Both calls begin before either finishes; split arguments exercise incremental JSON assembly.
    for (const [index, call] of tools.entries())
      events.push(
        chunk({
          tool_calls: [
            {
              index,
              id: call.id,
              type: 'function',
              function: {
                name: call.name,
                arguments: call.arguments.slice(0, 9),
              },
            },
          ],
        }),
      )
    for (const [index, call] of tools.entries())
      events.push(
        chunk({
          tool_calls: [
            { index, function: { arguments: call.arguments.slice(9) } },
          ],
        }),
      )
  } else {
    const count = scenario === 'long-stream' ? 200 : 1
    for (let i = 0; i < count; i++)
      events.push(
        chunk({
          content: followup
            ? 'Done.'
            : scenario === 'long-stream'
              ? `token-${i} `
              : 'Hello fixture.',
        }),
      )
  }
  if (scenario === 'mid-stream-error')
    return [
      ...events,
      {
        data: {
          error: {
            message: 'scripted stream failure',
            type: 'server_error',
            code: 'compat_error',
          },
        },
      },
      { data: '[DONE]' },
    ]
  return [
    ...events,
    chunk({}, tools.length ? 'tool_calls' : 'stop'),
    { data: '[DONE]' },
  ]
}

function anthropicEvents(scenario: Scenario, followup: boolean): Event[] {
  const events: Event[] = []
  const push = (type: string, value: Json) =>
    events.push({ event: type, data: { type, ...value } })
  push('message_start', {
    message: {
      id: 'msg_compat',
      type: 'message',
      role: 'assistant',
      content: [],
      model: scenario,
      stop_reason: null,
      stop_sequence: null,
      usage: { input_tokens: 10, output_tokens: 0 },
    },
  })
  let index = 0
  if (scenario === 'reasoning-text' && !followup) {
    push('content_block_start', {
      index,
      content_block: { type: 'thinking', thinking: '', signature: '' },
    })
    push('content_block_delta', {
      index,
      delta: { type: 'thinking_delta', thinking: 'Thinking.' },
    })
    push('content_block_delta', {
      index,
      delta: { type: 'signature_delta', signature: 'compat-signature' },
    })
    push('content_block_stop', { index: index++ })
  }
  const tools = followup ? [] : calls(scenario)
  if (tools.length) {
    for (const call of tools) {
      push('content_block_start', {
        index,
        content_block: {
          type: 'tool_use',
          id: call.id,
          name: call.name,
          input: {},
        },
      })
      push('content_block_delta', {
        index,
        delta: { type: 'input_json_delta', partial_json: call.arguments },
      })
      push('content_block_stop', { index: index++ })
    }
  } else {
    push('content_block_start', {
      index,
      content_block: { type: 'text', text: '' },
    })
    for (let i = 0; i < (scenario === 'long-stream' ? 200 : 1); i++)
      push('content_block_delta', {
        index,
        delta: {
          type: 'text_delta',
          text: followup
            ? 'Done.'
            : scenario === 'long-stream'
              ? `token-${i} `
              : 'Hello fixture.',
        },
      })
    if (scenario === 'mid-stream-error') {
      push('error', {
        error: { type: 'api_error', message: 'scripted stream failure' },
      })
      return events
    }
    push('content_block_stop', { index })
  }
  push('message_delta', {
    delta: {
      stop_reason: tools.length ? 'tool_use' : 'end_turn',
      stop_sequence: null,
    },
    usage: { output_tokens: 5 },
  })
  push('message_stop', {})
  return events
}

function responsesEvents(scenario: Scenario, followup: boolean): Event[] {
  const events: Event[] = []
  let sequence = 0
  const push = (type: string, value: Json) =>
    events.push({
      event: type,
      data: { type, sequence_number: sequence++, ...value },
    })
  const response = {
    id: 'resp_compat',
    object: 'response',
    created_at: 1_700_000_000,
    model: scenario,
    status: 'in_progress',
    output: [],
  }
  push('response.created', { response })
  let output_index = 0
  if (scenario === 'reasoning-text' && !followup) {
    const item = { id: 'reason_compat', type: 'reasoning', summary: [] }
    push('response.output_item.added', { output_index, item })
    push('response.reasoning_summary_part.added', {
      item_id: item.id,
      output_index,
      summary_index: 0,
      part: { type: 'summary_text', text: '' },
    })
    push('response.reasoning_summary_text.delta', {
      item_id: item.id,
      output_index,
      summary_index: 0,
      delta: 'Thinking.',
    })
    push('response.reasoning_summary_part.done', {
      item_id: item.id,
      output_index,
      summary_index: 0,
      part: { type: 'summary_text', text: 'Thinking.' },
    })
    push('response.output_item.done', {
      output_index: output_index++,
      item: { ...item, summary: [{ type: 'summary_text', text: 'Thinking.' }] },
    })
  }
  const tools = followup ? [] : calls(scenario)
  if (tools.length) {
    for (const call of tools) {
      const item = {
        id: `fc_${call.id}`,
        type: 'function_call',
        call_id: call.id,
        name: call.name,
        arguments: '',
      }
      push('response.output_item.added', { output_index, item })
      push('response.function_call_arguments.delta', {
        item_id: item.id,
        output_index,
        delta: call.arguments,
      })
      push('response.function_call_arguments.done', {
        item_id: item.id,
        output_index,
        arguments: call.arguments,
      })
      push('response.output_item.done', {
        output_index: output_index++,
        item: { ...item, arguments: call.arguments, status: 'completed' },
      })
    }
  } else {
    const item = {
      id: 'message_compat',
      type: 'message',
      role: 'assistant',
      content: [],
      status: 'in_progress',
    }
    push('response.output_item.added', { output_index, item })
    push('response.content_part.added', {
      item_id: item.id,
      output_index,
      content_index: 0,
      part: { type: 'output_text', text: '', annotations: [] },
    })
    const count = scenario === 'long-stream' ? 200 : 1
    const texts = Array.from({ length: count }, (_, i) =>
      followup
        ? 'Done.'
        : scenario === 'long-stream'
          ? `token-${i} `
          : 'Hello fixture.',
    )
    for (const delta of texts)
      push('response.output_text.delta', {
        item_id: item.id,
        output_index,
        content_index: 0,
        delta,
        logprobs: [],
      })
    if (scenario === 'mid-stream-error') {
      push('error', {
        code: 'server_error',
        message: 'scripted stream failure',
        param: null,
      })
      return events
    }
    push('response.output_text.done', {
      item_id: item.id,
      output_index,
      content_index: 0,
      text: texts.join(''),
      logprobs: [],
    })
    push('response.output_item.done', {
      output_index,
      item: {
        ...item,
        status: 'completed',
        content: [
          {
            type: 'output_text',
            text: texts.join(''),
            annotations: [],
            logprobs: [],
          },
        ],
      },
    })
  }
  push('response.completed', {
    response: {
      ...response,
      status: 'completed',
      usage: {
        input_tokens: 10,
        output_tokens: 5,
        total_tokens: 15,
        input_tokens_details: { cached_tokens: 0 },
        output_tokens_details: { reasoning_tokens: 0 },
      },
      incomplete_details: null,
    },
  })
  return events
}

/** Each session header owns retry state; two compared servers cannot consume each other's 429. */
export function startFakeProvider(
  options: { port?: number; delayMs?: number } = {},
) {
  const attempts = new Map<string, number>()
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
      const events = path.endsWith('/messages')
        ? anthropicEvents(scenario, followup)
        : path.endsWith('/responses')
          ? responsesEvents(scenario, followup)
          : chatEvents(scenario, followup)
      let cursor = 0
      let cancelled = false
      const stream = new ReadableStream<Uint8Array>({
        async pull(controller) {
          // pull honors cancellation and backpressure, so /stop cannot leave an upstream timer running.
          const delay =
            options.delayMs ?? (scenario === 'long-stream' ? 80 : 15)
          if (delay) await Bun.sleep(delay)
          if (cancelled || request.signal.aborted) {
            controller.close()
            return
          }
          const event = events[cursor++]
          if (!event) {
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
