import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { runInNewContext } from 'node:vm'
import { buildClassic } from '../scripts/build-classic'

const assetPath = new URL(
  '../../../crates/browseros-core/src/assets/agent-cursor.js',
  import.meta.url,
)

it('tests that the committed classic asset matches a fresh build', async () => {
  const fresh = await buildClassic()
  const committed = readFileSync(assetPath, 'utf8')
  if (fresh !== committed) {
    throw new Error(
      'Run bun run build:classic with Bun 1.4.2 in packages/agent-cursor',
    )
  }
})

it('tests that classic builds are independent of the caller directory', async () => {
  const originalCwd = process.cwd()
  try {
    process.chdir(tmpdir())
    const buildCwd = process.cwd()
    const fresh = await buildClassic()
    expect(process.cwd()).toBe(buildCwd)
    expect(fresh).toBe(readFileSync(assetPath, 'utf8'))
  } finally {
    process.chdir(originalCwd)
  }
})

it('tests that the classic asset exposes MotionPlayer inside a function without globals', () => {
  const asset = readFileSync(assetPath, 'utf8')
  const context = {}
  const cursor = runInNewContext(
    `(function () {\n${asset}\nreturn BrowserOSAgentCursor;\n})()`,
    context,
  )
  expect(typeof cursor.MotionPlayer).toBe('function')
  expect(Object.keys(cursor)).toEqual(['MotionPlayer'])
  expect(Reflect.ownKeys(context)).toEqual([])
  expect(asset).not.toMatch(/^\s*(?:import|export)\b/m)
  expect(asset).toContain('Copyright (c) 2025 Cua AI, Inc.')
  const license = readFileSync(new URL('../LICENSE', import.meta.url), 'utf8')
  expect(asset.startsWith(`/*\n${license}`)).toBe(true)
  expect(asset).toContain('4a44f4d39677c2d906d588b5b205a85aa8353354')
  expect(asset.endsWith('\n')).toBe(true)
})

it('tests that pointer consumers can wrap MotionPlayer for tracing', () => {
  const asset = readFileSync(assetPath, 'utf8')
  const cursor = runInNewContext(
    `(function () {\n${asset}\nreturn BrowserOSAgentCursor;\n})()`,
  )
  class TracedPlayer extends cursor.MotionPlayer {}
  cursor.MotionPlayer = TracedPlayer
  expect(cursor.MotionPlayer).toBe(TracedPlayer)
})
