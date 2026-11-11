import { expect, test } from 'bun:test'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import net from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { classify } from './judge.mjs'
import { hostGet, runNeo, traceCodeEvidence } from './runner-neo.mjs'

test('verifies complete source and reports unavailable code', () => {
  expect(traceCodeEvidence('return 1', { code: 'return 1' })).toBe('full')
  expect(traceCodeEvidence('return 1', { code: 'return 2' })).toBeUndefined()
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
            structuredContent: { value: { status: 'passed' } },
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
