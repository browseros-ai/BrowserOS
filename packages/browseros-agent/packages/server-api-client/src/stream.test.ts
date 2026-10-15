/** Network segmentation must not change stream framing or detach/cancel semantics. */
import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import manifest from '../../../contracts/server-api/fixtures/manifest.json'
import examples from '../../../contracts/server-api/tests/ui-stream-examples.json'
import { ServerApiError } from './errors'
import { readUiMessageStream } from './stream'

function response(bytes: Uint8Array[], cancel?: () => void): Response {
  let index = 0
  return new Response(
    new ReadableStream<Uint8Array>({
      pull(controller) {
        const part = bytes[index++]
        if (part) controller.enqueue(part)
        else controller.close()
      },
      cancel,
    }),
    {
      headers: {
        'content-type': 'text/event-stream',
        'x-vercel-ai-ui-message-stream': 'v1',
      },
    },
  )
}
const encoder = new TextEncoder()
const { additionalChunks, malformedChunks } = examples
const chunks = manifest
  .filter((item) => item.schema === 'UiMessageChunk')
  .map((item) =>
    JSON.parse(
      readFileSync(
        join(
          import.meta.dir,
          '../../../contracts/server-api/fixtures',
          item.file,
        ),
        'utf8',
      ),
    ),
  )
  .concat(additionalChunks)

test('all chunk variants survive one-byte reads, UTF-8 and all SSE line endings', async () => {
  for (const ending of ['\n', '\r\n', '\r']) {
    const wire =
      ': heartbeat' +
      ending +
      ending +
      chunks
        .map(
          (chunk) =>
            `event: message${ending}id: ignored${ending}data: ${JSON.stringify(chunk)}${ending}${ending}`,
        )
        .join('') +
      `data: [DONE]${ending}${ending}`
    const bytes = encoder.encode(wire)
    const actual = await Array.fromAsync(
      readUiMessageStream(
        response(Array.from(bytes, (byte) => Uint8Array.of(byte))),
      ),
    )
    expect(actual).toEqual(chunks)
  }
})

test('multiline data, BOM and comments follow SSE framing', async () => {
  const wire =
    '\ufeff: keepalive\n\ndata: {"type":"text-delta",\ndata: "id":"t1","delta":"Hi"}\n\ndata: [DONE]\n\n'
  expect(
    await Array.fromAsync(
      readUiMessageStream(response([encoder.encode(wire)])),
    ),
  ).toEqual([{ type: 'text-delta', id: 't1', delta: 'Hi' }])
})

test('consumer break and DONE both cancel the subscriber and release its lock', async () => {
  for (const early of [true, false]) {
    let cancelled = 0
    const raw = new Response(
      new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(
            encoder.encode('data: {"type":"start"}\n\ndata: [DONE]\n\n'),
          )
        },
        cancel() {
          cancelled++
        },
      }),
      { headers: { 'content-type': 'text/event-stream' } },
    )
    for await (const _chunk of readUiMessageStream(raw)) {
      if (early) break
    }
    expect(cancelled).toBe(1)
    expect(raw.body?.locked).toBe(false)
  }
})

test('abort interrupts a pending read and releases its lock', async () => {
  const abort = new AbortController()
  let cancelled = false
  const raw = new Response(
    new ReadableStream<Uint8Array>({
      cancel() {
        cancelled = true
      },
    }),
    { headers: { 'content-type': 'text/event-stream' } },
  )
  const next = readUiMessageStream(raw, { signal: abort.signal }).next()
  abort.abort(new DOMException('Cancelled', 'AbortError'))
  await expect(next).rejects.toHaveProperty('name', 'AbortError')
  expect(cancelled).toBe(true)
  expect(raw.body?.locked).toBe(false)
})

for (const options of [{}, { requireDone: true }]) {
  test(`requires DONE on clean EOF with ${JSON.stringify(options)}`, async () => {
    const raw = response([encoder.encode('data: {"type":"start"}\n\n')])
    await expect(
      Array.fromAsync(readUiMessageStream(raw, options)),
    ).rejects.toMatchObject({
      name: 'ServerApiError',
      code: 'invalid_stream',
      message: 'The chat stream ended before [DONE]',
    })
    expect(raw.body?.locked).toBe(false)
  })
}

test('optional DONE permits clean EOF after a stop without a finish chunk', async () => {
  const raw = response([
    encoder.encode(
      'data: {"type":"start"}\n\ndata: {"type":"abort","reason":"Stopped"}\n\n',
    ),
  ])
  expect(
    await Array.fromAsync(readUiMessageStream(raw, { requireDone: false })),
  ).toEqual([{ type: 'start' }, { type: 'abort', reason: 'Stopped' }])
  expect(raw.body?.locked).toBe(false)
})

test('optional DONE still propagates body read failures', async () => {
  const failure = new Error('Connection dropped')
  const raw = new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        controller.error(failure)
      },
    }),
    { headers: { 'content-type': 'text/event-stream' } },
  )
  await expect(
    Array.fromAsync(readUiMessageStream(raw, { requireDone: false })),
  ).rejects.toBe(failure)
  expect(raw.body?.locked).toBe(false)
})

for (const payload of [
  'nope',
  'null',
  '[]',
  '{}',
  '{"type":null}',
  '{"type":1}',
  '{"type":"text-delta","id":"t"}',
]) {
  test(`optional DONE still rejects invalid chunks: ${payload}`, async () => {
    await expect(
      Array.fromAsync(
        readUiMessageStream(
          response([encoder.encode(`data: ${payload}\n\n`)]),
          {
            requireDone: false,
          },
        ),
      ),
    ).rejects.toMatchObject({ name: 'ServerApiError', code: 'invalid_stream' })
  })
}

for (const wire of [
  'data: nope\n\n',
  'data: {"type":"unknown"}\n\n',
  'data: {"type":"text-delta","id":"x"}\n\n',
  'data: {"type":"start"}\n\n',
  'data: {"type":"start"}',
]) {
  test(`rejects corrupt or truncated stream: ${wire.trim()}`, async () => {
    await expect(
      Array.fromAsync(readUiMessageStream(response([encoder.encode(wire)]))),
    ).rejects.toMatchObject({
      name: 'ServerApiError',
      status: 200,
      code: 'invalid_stream',
    })
  })
}

test('tests that unknown string types pass through unchanged and later chunks still arrive', async () => {
  const expected = [
    { type: 'start' },
    { type: 'future-provider-event', payload: { values: [1, null] } },
    { type: '', extension: true },
    { type: 'text-delta', id: 't1', delta: 'Still streaming' },
    { type: 'finish' },
  ]
  for (const requireDone of [true, false]) {
    const wire =
      expected.map((chunk) => `data: ${JSON.stringify(chunk)}\n\n`).join('') +
      (requireDone ? 'data: [DONE]\n\n' : '')
    const actual: unknown[] = await Array.fromAsync(
      readUiMessageStream(response([encoder.encode(wire)]), {
        requireDone,
      }),
    )
    expect(actual).toEqual(expected)
  }
})

for (const chunk of malformedChunks) {
  test(`tests that known malformed ${JSON.stringify(chunk)} never takes the forward compatibility path`, async () => {
    const raw = response([
      encoder.encode(`data: ${JSON.stringify(chunk)}\n\ndata: [DONE]\n\n`),
    ])
    await expect(
      Array.fromAsync(readUiMessageStream(raw)),
    ).rejects.toMatchObject({
      name: 'ServerApiError',
      code: 'invalid_stream',
    })
    expect(raw.body?.locked).toBe(false)
  })
}
test('HTTP failures use the same parsed error; 204 yields no chunks', async () => {
  await expect(
    Array.fromAsync(
      readUiMessageStream(
        Response.json({ error: 'Forbidden' }, { status: 403 }),
      ),
    ),
  ).rejects.toBeInstanceOf(ServerApiError)
  expect(
    await Array.fromAsync(
      readUiMessageStream(new Response(null, { status: 204 })),
    ),
  ).toEqual([])
})
