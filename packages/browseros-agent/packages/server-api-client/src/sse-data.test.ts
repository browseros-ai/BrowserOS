/** Panel streams share chat framing, without inheriting chat's DONE protocol. */
import { expect, test } from 'bun:test'
import { readSseData } from './stream'

test('panel payloads survive one-byte UTF-8, CRLF and multiline data, ending at EOF', async () => {
  const bytes = new TextEncoder().encode(
    ': heartbeat\r\n\r\ndata: {"assignments":\r\ndata: [],"label":"🌍"}\r\n\r\n',
  )
  const response = new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        for (const byte of bytes) controller.enqueue(Uint8Array.of(byte))
        controller.close()
      },
    }),
    { headers: { 'content-type': 'text/event-stream' } },
  )
  expect(await Array.fromAsync(readSseData(response))).toEqual([
    '{"assignments":\n[],"label":"🌍"}',
  ])
  expect(response.body?.locked).toBe(false)
})

test('breaking a panel subscription cancels its body and releases the lock', async () => {
  let cancelled = false
  const response = new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          new TextEncoder().encode('data: {"assignments":[]}\n\n'),
        )
      },
      cancel() {
        cancelled = true
      },
    }),
    { headers: { 'content-type': 'text/event-stream' } },
  )
  for await (const _data of readSseData(response)) break
  expect(cancelled).toBe(true)
  expect(response.body?.locked).toBe(false)
})
