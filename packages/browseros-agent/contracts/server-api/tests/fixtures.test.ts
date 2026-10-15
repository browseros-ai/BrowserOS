/** Shared wire examples are validated, not merely checked for a few expected keys. */
import { expect, test } from 'bun:test'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import manifest from '../fixtures/manifest.json'
import { assertSchema, contractDir } from './support'

test('every JSON fixture is indexed with a schema and honest provenance', () => {
  expect(readdirSync(join(contractDir, 'fixtures')).sort()).toEqual(
    ['manifest.json', ...manifest.map((entry) => entry.file)].sort(),
  )
  expect(new Set(manifest.map((entry) => entry.file)).size).toBe(
    manifest.length,
  )
  for (const entry of manifest) {
    expect(entry.source.length).toBeGreaterThan(0)
    expect(entry.provenance.length).toBeGreaterThan(0)
  }
})
for (const fixture of manifest) {
  test(`${fixture.file} validates as ${fixture.schema}`, () => {
    assertSchema(
      fixture.schema,
      JSON.parse(
        readFileSync(join(contractDir, 'fixtures', fixture.file), 'utf8'),
      ),
    )
  })
}

test('validator rejects missing, mistyped, extra strict fields and bad chunk tags', () => {
  for (const [schema, value] of [
    ['StatusResponse', { status: 'ok' }],
    ['HealthResponse', { status: 'ok', cdpConnected: 'yes' }],
    ['CreateAgentRequest', { name: 'A', type: 'codex', surprise: true }],
    ['CreateAgentRequest', { name: 'A', type: 'custom' }],
    ['AcpxProbeRequest', { type: 'custom' }],
    ['AcpxProbeRequest', { type: 'codex', timeoutMs: 999 }],
    ['ChatRequest', { conversationId: 'x', target: { type: 'custom' } }],
    ['UiMessageChunk', { type: 'text-delta', id: 't1' }],
    ['UiMessageChunk', { type: 'unknown' }],
  ] as const)
    expect(() => assertSchema(schema, value)).toThrow()
})

test('request schemas preserve trimming and Zod UUID limits', () => {
  assertSchema('CreateAgentRequest', {
    name: `${' '.repeat(100)}A${' '.repeat(100)}`,
    type: 'codex',
  })
  expect(() =>
    assertSchema('CreateAgentRequest', { name: 'A'.repeat(81), type: 'codex' }),
  ).toThrow()
  expect(() =>
    assertSchema('CreateAgentRequest', { name: '  ', type: 'codex' }),
  ).toThrow()
  expect(() =>
    assertSchema('ChatRequest', {
      conversationId: '11111111-1111-9111-8111-111111111111',
    }),
  ).toThrow()
  assertSchema('ChatRequest', {
    conversationId: '00000000-0000-0000-0000-000000000000',
  })
})
