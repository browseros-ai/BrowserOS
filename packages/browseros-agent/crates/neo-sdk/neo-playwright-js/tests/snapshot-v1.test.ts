// The v1 snapshot text (src/prelude/snapshot-v1.ts), from the snapshot op's `{ json, partial }`.
// The blind page's JSON was recorded from a live engine by neo-playwright's prelude_live test,
// which also checks a live page prints the same text; the rule tests use small trees.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { renderSnapshotV1 } from '../src/prelude/snapshot-v1'
import type { SnapshotResult } from '../src/prelude/wire'
import { loadPrelude, SEED } from './support/prelude-runtime'

const blindData = (name: string): string =>
  readFileSync(
    new URL(
      `../../neo-playwright/tests/data/snapshot-v1/${name}`,
      import.meta.url,
    ),
    'utf8',
  )
const blind = JSON.parse(blindData('blind.json')) as SnapshotResult

it('tests that the blind page prints its expected text by default and with text: false', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'page.ariaSnapshot' ? blind : null),
  })
  expect(await runtime.run('return await page.ariaSnapshot()')).toBe(
    blindData('blind.v1-default.txt'),
  )
  expect(
    await runtime.run(
      "return await page.ariaSnapshot({ mode: 'ai', text: false })",
    ),
  ).toBe(blindData('blind.v1-interactive.txt'))
  expect(renderSnapshotV1(blind, { urls: false, maxTokens: 8_000 })).toBe(
    blindData('blind.v1-default.txt'),
  )
})
