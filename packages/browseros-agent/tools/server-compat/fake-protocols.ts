/** Encodes one deterministic scenario vocabulary into the three real upstream streaming protocols. */
import type { Scenario } from './fake-provider'

type Json = Record<string, unknown>
interface Event {
  data: Json | '[DONE]'
  event?: string
  textDelta?: boolean
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
    textDelta: typeof delta.content === 'string' && delta.content.length > 0,
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
    events.push({
      event: type,
      textDelta:
        type === 'content_block_delta' &&
        (value.delta as Json)?.type === 'text_delta',
      data: { type, ...value },
    })
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
      textDelta: type === 'response.output_text.delta',
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

export function eventsFor(
  path: string,
  scenario: Scenario,
  followup: boolean,
): Event[] {
  return path.endsWith('/messages')
    ? anthropicEvents(scenario, followup)
    : path.endsWith('/responses')
      ? responsesEvents(scenario, followup)
      : chatEvents(scenario, followup)
}
