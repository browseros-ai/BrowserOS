import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { runInNewContext } from 'node:vm'
import { staleOutputs } from '../scripts/build'

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

it("tests that the prelude asset adds only Playwright's four globals to the bootstrap's realm", () => {
  const realm: Record<string, unknown> = {
    __neo: { invoke: () => Promise.resolve(null) },
  }
  runInNewContext(asset('prelude.js'), realm)
  expect(Reflect.ownKeys(realm).sort()).toEqual([
    '__neo',
    'browser',
    'context',
    'expect',
    'page',
  ])
})

it('tests that the prelude asset refuses to start without the bootstrap', () => {
  expect(() => runInNewContext(asset('prelude.js'), {})).toThrow(
    "The Playwright prelude needs claw-sdk's bootstrap",
  )
})
