import { test } from 'bun:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { generateServerApiTypes } from '../../../scripts/codegen/server-api'

const root = join(import.meta.dir, '../../..')

test('the server API TypeScript matches the committed OpenAPI document', () => {
  // The Rust OpenAPI test checks the previous link, from Rust to the committed spec.
  const generated = generateServerApiTypes(
    join(root, 'contracts/server-api/openapi.json'),
  )
  const committed = readFileSync(
    join(root, 'packages/server-api/src/generated/openapi.ts'),
  )

  assert.ok(
    committed.equals(Buffer.from(generated)),
    'Generated server API TypeScript is stale. Run `bun run codegen:server-api` from packages/browseros-agent.',
  )
})
