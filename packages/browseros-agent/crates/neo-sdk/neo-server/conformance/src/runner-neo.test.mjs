import { expect, test } from 'bun:test'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import net from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { classify } from './judge.mjs'
import {
  hostGet,
  parseRunText,
  runNeo,
  traceCodeEvidence,
} from './runner-neo.mjs'

test('reads JSON only from Result, including null and multiline strings', () => {
  for (const value of [
    null,
    42,
    'line one\n### Console\nline two',
    { status: 'passed' },
  ]) {
    expect(
      parseRunText(
        `### Result\n${JSON.stringify(value)}\n### Console\nhello\n### Tips\nadvice`,
      ),
    ).toEqual({ value })
  }
})

test('reads the whole Error section and unescapes header lines', () => {
  expect(
    parseRunText(
      '### Error\nscript / script_error: failed\n\\### Result\n\\### Error\n\\### Console\n\\### Notes\n\\### Tips\ncall log\n### Console\nhello\n### Notes\nnote',
    ),
  ).toEqual({
    error:
      'script / script_error: failed\n### Result\n### Error\n### Console\n### Notes\n### Tips\ncall log',
  })
})

test('rejects missing headers and malformed JSON', () => {
  for (const text of [
    undefined,
    '',
    '{}',
    '### Console\nhello',
    '### Result\nbroken',
  ]) {
    expect(() => parseRunText(text)).toThrow()
  }
})

test('text-only values and errors survive the complete conformance run path', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'conformance-reply-'))
  const hostSocket = join(dir, 'host.sock')
  const file = join(dir, 'program.js')
  const code = 'return { status: "passed" }'
  writeFileSync(file, code)
  const server = http.createServer((_request, response) => {
    response.end(
      JSON.stringify({
        id: 'run',
        requestId: 'request',
        taskId: 'task',
        code,
        complete: true,
        operations: [],
      }),
    )
  })
  try {
    server.listen(hostSocket)
    await once(server, 'listening')
    for (const isError of [false, true]) {
      const record = await runNeo({
        env: { hostSocket },
        row: { id: 'test' },
        file,
        output: join(dir, 'record.json'),
        taskId: 'task',
        requestId: 'request',
        client: {
          callTool: async () => ({
            content: [
              {
                type: 'text',
                text: isError
                  ? '### Error\nscript / script_error: failed\n\\### Notes\n### Console\nhello'
                  : '### Result\n{"status":"passed"}\n### Console\nhello',
              },
            ],
            isError,
            _meta: {
              'com.browseros.neo/run': {
                id: 'run',
                status: isError ? 'error' : 'ok',
              },
            },
          }),
        },
      })
      expect(record.provenance.neo.traceVerified).toBe(true)
      if (isError)
        expect(record.transport.error).toBe(
          'script / script_error: failed\n### Notes',
        )
      else expect(record.status).toBe('passed')
    }
  } finally {
    await new Promise((resolve) => server.close(resolve))
    rmSync(dir, { recursive: true, force: true })
  }
})

test('verifies complete source and rejects missing source evidence', () => {
  expect(traceCodeEvidence('return 1', { code: 'return 1' })).toBe('full')
  expect(traceCodeEvidence('return 1', { code: 'return 2' })).toBeUndefined()
  expect(
    traceCodeEvidence('return 1', { codeWithheld: 'sensitive_input' }),
  ).toBeUndefined()
  expect(
    traceCodeEvidence('return 1', { codeWithheld: 'unavailable' }),
  ).toBeUndefined()
})

test('verifies the exact Neo trace cutoff and omitted UTF-8 byte count', () => {
  // Neo reserves 24 bytes for its suffix. A multibyte character crossing that boundary
  // belongs entirely to the omitted byte count.
  const prefix = 'x'.repeat(64 * 1024 - 25)
  const program = `${prefix}🌐${'z'.repeat(40)}`
  const preview = `${prefix}…[+44 bytes]`
  expect(traceCodeEvidence(program, { code: preview })).toBe('truncated')
  expect(
    traceCodeEvidence(program, { code: `${prefix}…[+43 bytes]` }),
  ).toBeUndefined()
  expect(
    traceCodeEvidence(program, { code: `y${preview.slice(1)}` }),
  ).toBeUndefined()
  expect(traceCodeEvidence(program, { code: prefix })).toBeUndefined()
})

// Send raw HTTP over a real Unix socket so the client sees disconnects after headers.
async function withTracePeer(send, use) {
  const dir = mkdtempSync(join(tmpdir(), 'conformance-trace-'))
  const hostSocket = join(dir, 'host.sock')
  const sockets = new Set()
  const server = net.createServer((socket) => {
    sockets.add(socket)
    // Deadline and truncation cases deliberately close the client while this peer writes.
    // A write can fail before close clears its timer; that disconnect belongs to the fixture.
    socket.on('error', (error) => {
      if (error.code !== 'EPIPE' && error.code !== 'ECONNRESET') throw error
      socket.destroy()
    })
    socket.once('close', () => sockets.delete(socket))
    socket.once('data', () => {
      socket.write('HTTP/1.1 200 OK\r\nContent-Length: 500\r\n\r\n{')
      send(socket)
    })
  })
  try {
    server.listen(hostSocket)
    await once(server, 'listening')
    await use({ hostSocket }, dir)
  } finally {
    for (const socket of sockets) socket.destroy()
    await new Promise((resolve) => server.close(resolve))
    rmSync(dir, { recursive: true, force: true })
  }
}
test('a truncated trace becomes a harness error and the peer closes', async () => {
  await withTracePeer(
    (socket) => setTimeout(() => socket.destroy(), 10),
    async (env, dir) => {
      const row = { id: 'test', selected: true, class: 'runnable' }
      const file = join(dir, 'program.js')
      writeFileSync(file, 'return {}')
      const record = await runNeo({
        env,
        row,
        file,
        output: join(dir, 'record.json'),
        taskId: 'task',
        requestId: 'request',
        client: {
          callTool: async () => ({
            _meta: { 'com.browseros.neo/run': { id: 'run' } },
            content: [
              { type: 'text', text: '### Result\n{"status":"passed"}' },
            ],
          }),
        },
      })
      expect(record.transport.error).toBe(
        'Error: incomplete host trace response',
      )
      expect(classify(row, {}, record).classification).toBe('harness-error')
    },
  )
})
test('trace reads have an absolute deadline even with continuing data', async () => {
  await withTracePeer(
    (socket) => {
      const timer = setInterval(() => socket.write(' '), 10)
      socket.once('close', () => clearInterval(timer))
    },
    async (env) => {
      await expect(hostGet(env, '/', { timeout: 80 })).rejects.toThrow(
        'host trace deadline exceeded',
      )
    },
  )
})
