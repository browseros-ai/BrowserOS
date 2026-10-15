/** Pins the wire union to the installed SDK rather than another hand-maintained list. */
import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { isUiMessageChunk } from '../../../packages/server-api-client/src/generated/ui-message-chunk'
import { assertSchema, record, schemaFor } from './support'
import examples from './ui-stream-examples.json'

const { additionalChunks, malformedChunks } = examples

test('tests that every installed SDK chunk has the exact contract fields and requiredness', () => {
  const serverRequire = createRequire(
    join(import.meta.dir, '../../../apps/server/package.json'),
  )
  const definitions = readFileSync(
    join(dirname(serverRequire.resolve('ai')), 'index.d.ts'),
    'utf8',
  )
  const declaration = definitions.slice(
    definitions.indexOf('type UIMessageChunk<'),
    definitions.indexOf('type InferUIMessageChunk<'),
  )
  const sdkVariants = [
    ...declaration.matchAll(/type: '([^']+)';([\s\S]*?)(?=\n\} \||\n\};)/g),
  ]
  const contract = schemaFor('UiMessageChunk').oneOf as Array<
    Record<string, unknown>
  >
  const fixedVariants = contract.filter(
    (variant) => record(record(variant.properties).type).enum,
  )
  expect(sdkVariants.length).toBe(28)
  expect(
    fixedVariants
      .flatMap((variant) => record(record(variant.properties).type).enum)
      .sort(),
  ).toEqual(sdkVariants.map((match) => match[1]).sort())
  for (const [, tag, body] of sdkVariants) {
    const variant = fixedVariants.find((candidate) =>
      (record(record(candidate.properties).type).enum as string[]).includes(
        tag as string,
      ),
    )
    expect(variant).toBeDefined()
    if (!variant) throw new Error(`No contract variant for SDK tag ${tag}`)
    const fields = [...(body as string).matchAll(/^ {4}(\w+)(\?)?:/gm)]
    expect(Object.keys(record(variant.properties)).sort()).toEqual(
      ['type', ...fields.map((field) => field[1])].sort(),
    )
    expect((variant.required as string[]).toSorted()).toEqual(
      [
        'type',
        ...fields.filter((field) => !field[2]).map((field) => field[1]),
      ].sort(),
    )
  }
  const data = contract.find(
    (variant) => record(record(variant.properties).type).pattern === '^data-',
  )
  expect(data).toBeDefined()
  expect(Object.keys(record(data?.properties)).sort()).toEqual([
    'data',
    'id',
    'transient',
    'type',
  ])
  expect(data?.required).toEqual(['type', 'data'])
})

for (const chunk of additionalChunks) {
  test(`tests that ${chunk.type} validates in the contract and generated guard`, () => {
    assertSchema('UiMessageChunk', chunk)
    expect(isUiMessageChunk(chunk)).toBe(true)
  })
}

for (const chunk of malformedChunks) {
  test(`tests that malformed ${JSON.stringify(chunk)} fails both validators`, () => {
    expect(() => assertSchema('UiMessageChunk', chunk)).toThrow()
    expect(isUiMessageChunk(chunk)).toBe(false)
  })
}

test('tests that every data namespace accepts opaque JSON and optional fields', () => {
  for (const type of ['data-', 'data-progress', 'data-provider.new']) {
    for (const data of [null, false, 1, 'example', [], {}]) {
      const chunk = { type, data }
      assertSchema('UiMessageChunk', chunk)
      expect(isUiMessageChunk(chunk)).toBe(true)
    }
  }
})
