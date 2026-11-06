import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { runInNewContext } from 'node:vm'
import { staleOutputs } from '../scripts/build'
import { loadPrelude } from './support/prelude-runtime'

const asset = (name: string) =>
  readFileSync(
    new URL(
      `../../../crates/browseros-playwright/assets/${name}`,
      import.meta.url,
    ),
    'utf8',
  )

it('tests that the committed assets and protocol examples match a fresh build', async () => {
  expect(await staleOutputs()).toEqual([])
})

it('tests that builds are independent of the caller directory', async () => {
  const originalCwd = process.cwd()
  try {
    process.chdir(tmpdir())
    expect(await staleOutputs()).toEqual([])
  } finally {
    process.chdir(originalCwd)
  }
})

it('tests that the page asset defines only __neoProbe, and a second evaluation keeps the first', () => {
  const context: Record<string, unknown> = {}
  runInNewContext(asset('injected.js'), context)
  expect(Reflect.ownKeys(context)).toEqual(['__neoProbe'])
  const probe = context.__neoProbe
  expect(typeof Reflect.get(Object(probe), 'boot')).toBe('function')
  expect(typeof Reflect.get(Object(probe), 'call')).toBe('function')
  runInNewContext(asset('injected.js'), context)
  expect(context.__neoProbe).toBe(probe)
})

// The JS twin of claw-sdk's runtime/tests.rs pin: a run's realm gains only `__neo` and the
// natives, and the agent's API arrives as its function's parameters.
it("tests that the prelude asset adds no global and gives the agent's function Playwright's four parameters and Buffer", () => {
  const runtime = loadPrelude()
  const fresh = new Set(
    runInNewContext('Object.getOwnPropertyNames(globalThis)') as string[],
  )
  const names = runtime.evaluate(
    'Object.getOwnPropertyNames(globalThis)',
  ) as string[]
  expect(names.filter((name) => !fresh.has(name)).sort()).toEqual([
    '__neo',
    '__neo_invoke',
    '__neo_log',
    '__neo_sleep',
  ])
  expect(runtime.evaluate('__neo.parameters()')).toBe(
    'console, setTimeout, clearTimeout, NeoError, TimeoutError, page, context, browser, expect, Buffer',
  )
})

it('tests that the prelude asset refuses to start without the bootstrap', () => {
  expect(() => runInNewContext(asset('prelude.js'), {})).toThrow(
    "The Playwright prelude needs claw-sdk's bootstrap",
  )
})
