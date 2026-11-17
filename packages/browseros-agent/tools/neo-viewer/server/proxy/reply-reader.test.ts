import { expect, test } from 'bun:test'
import { JsonReader, SseReader } from './reply-reader'

function collect() {
  const messages: unknown[] = []
  let comments = 0
  return {
    messages,
    comments: () => comments,
    listener: {
      message: ({ message }: { message: unknown }) => messages.push(message),
      comment: () => {
        comments++
      },
    },
  }
}

test('SseReader reads data events and keep-alives split anywhere across chunks', () => {
  const seen = collect()
  const reader = new SseReader(seen.listener)
  const stream =
    ':\n\n:\r\n\r\ndata: {"jsonrpc":"2.0","method":"notifications/progress"}\r\n\r\ndata: {"id":1,\ndata: "result":{}}\n\n'
  for (const char of stream) reader.push(new TextEncoder().encode(char), 0)
  reader.end(0)
  expect(seen.comments()).toBe(2)
  expect(seen.messages).toEqual([
    { jsonrpc: '2.0', method: 'notifications/progress' },
    { id: 1, result: {} },
  ])
})

test('SseReader drops an event the stream cut before its blank line', () => {
  const seen = collect()
  const reader = new SseReader(seen.listener)
  reader.push(new TextEncoder().encode('data: {"id":1,"result":{}}\n'), 0)
  reader.end(0)
  expect(seen.messages).toEqual([])
})

test('JsonReader reads one message or a batch when the body ends', () => {
  const seen = collect()
  const reader = new JsonReader(seen.listener)
  reader.push(new TextEncoder().encode('[{"id":1,"result":{}},'), 0)
  reader.push(
    new TextEncoder().encode('{"id":2,"error":{"code":-1,"message":"x"}}]'),
    0,
  )
  reader.end(0)
  expect(seen.messages).toHaveLength(2)
})
