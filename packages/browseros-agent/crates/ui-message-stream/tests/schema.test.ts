/** Validate the Rust byte-fixture inventory against C1's shared AJV schema and real SDK. */
import { expect, test } from 'bun:test'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { assertSchema } from '../../../contracts/server-api/tests/support'

const dir = join(import.meta.dir, 'fixtures')
const fixtures = readdirSync(dir)
  .filter((name) => name.endsWith('.sse'))
  .sort()

test('fixture inventory covers every SDK v7 chunk variant', () => {
  const types = new Set<string>()
  expect(fixtures.length).toBeGreaterThanOrEqual(8)
  for (const file of fixtures) {
    for (const frame of readFileSync(join(dir, file), 'utf8').split('\n\n')) {
      if (!frame || frame === 'data: [DONE]') continue
      types.add(JSON.parse(frame.slice(6)).type)
    }
  }
  expect([...types].sort()).toEqual(
    [
      'abort',
      'custom',
      'data-progress',
      'error',
      'file',
      'finish',
      'finish-step',
      'message-metadata',
      'reasoning-delta',
      'reasoning-end',
      'reasoning-file',
      'reasoning-start',
      'reset-step',
      'source-document',
      'source-url',
      'start',
      'start-step',
      'text-delta',
      'text-end',
      'text-start',
      'tool-approval-request',
      'tool-approval-response',
      'tool-input-available',
      'tool-input-delta',
      'tool-input-error',
      'tool-input-start',
      'tool-output-available',
      'tool-output-denied',
      'tool-output-error',
    ].sort(),
  )
})

for (const file of fixtures) {
  test(`${file} matches C1 UiMessageChunk and SSE framing`, () => {
    const body = readFileSync(join(dir, file), 'utf8')
    expect(body.endsWith('data: [DONE]\n\n')).toBe(true)
    expect(body.match(/data: \[DONE\]/g)?.length).toBe(1)
    for (const frame of body.split('\n\n')) {
      if (!frame || frame === 'data: [DONE]') continue
      expect(frame.startsWith('data: ')).toBe(true)
      const chunk = JSON.parse(frame.slice(6))
      assertSchema('UiMessageChunk', chunk)
      expect(frame).toBe(`data: ${JSON.stringify(chunk)}`)
    }
  })
}
