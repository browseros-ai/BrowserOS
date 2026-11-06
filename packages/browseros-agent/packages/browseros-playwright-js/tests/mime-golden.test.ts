// The golden comes from Playwright's own bundle (scripts/capture-mime-golden.mjs): the
// extension-to-type table of the `mime` instance playwright-core 1.64.1 types setInputFiles
// payloads with, and its getType answers for names that exercise the parsing. The vendored
// package must build the same instance, so a payload with no mimeType gets the type Playwright
// would give it.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import mime from '../../../crates/browseros-playwright/third_party/mime/src/index'

type Golden = {
  playwright: string
  mime: string
  getType: Record<string, string | null>
  types: Record<string, string>
}

const golden: Golden = JSON.parse(
  readFileSync(new URL('./data/mime-golden.json', import.meta.url), 'utf8'),
)
const vendoredVersion = readFileSync(
  new URL(
    '../../../crates/browseros-playwright/third_party/mime/VERSION',
    import.meta.url,
  ),
  'utf8',
).split('\n')[0]

it('tests that the vendored mime is the version Playwright 1.64.1 bundles', () => {
  expect(golden.playwright).toBe('1.64.1')
  expect(vendoredVersion).toBe(golden.mime)
})

it("tests that the vendored mime table is the one Playwright's bundle types payloads with", () => {
  const types = Object.fromEntries(mime._getTestState().types)
  expect(Object.keys(types).length).toBeGreaterThan(1000)
  expect(types).toEqual(golden.types)
})

it("tests that getType parses names as Playwright's bundle does", () => {
  const answered = Object.fromEntries(
    Object.keys(golden.getType).map((name) => [name, mime.getType(name)]),
  )
  expect(answered).toEqual(golden.getType)
})
