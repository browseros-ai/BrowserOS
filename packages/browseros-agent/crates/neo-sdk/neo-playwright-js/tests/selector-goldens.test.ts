// The selector goldens come from real Playwright (scripts/capture-goldens.mjs): for each
// expression over `page`, the selector string playwright-core 1.64.1's client builds and its
// Locator.toString(). The prelude's classes must build the same bytes, so a locator a model
// writes reaches Rust exactly as Playwright would send it to its server.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { loadPrelude, SEED } from './support/prelude-runtime'

type Golden = { code: string; selector: string; toString: string }

const goldens: { playwright: string; cases: Golden[] } = JSON.parse(
  readFileSync(
    new URL('./data/selector-goldens.json', import.meta.url),
    'utf8',
  ),
)

it("tests that every recorded expression builds Playwright's selector and toString()", async () => {
  const runtime = loadPrelude({ seed: SEED })
  await runtime.ready
  expect(goldens.cases.length).toBeGreaterThanOrEqual(60)
  const built = goldens.cases.map((golden) => {
    const locator = runtime.evaluate(golden.code) as {
      _selector?: string
      _frameSelector?: string
    }
    return {
      code: golden.code,
      selector: locator._selector ?? locator._frameSelector,
      toString: String(locator),
    }
  })
  expect(built).toEqual(
    goldens.cases.map((golden) => ({
      code: golden.code,
      selector: golden.selector,
      toString: golden.toString,
    })),
  )
})

it('tests that builders are values: building a locator sends nothing', async () => {
  const runtime = loadPrelude({ seed: SEED })
  await runtime.ready
  for (const golden of goldens.cases) runtime.evaluate(golden.code)
  expect(runtime.calls).toEqual([])
})
