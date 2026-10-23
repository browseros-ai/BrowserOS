import { expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { launchConfiguredBrowser } from './browser-launch'
import { parseOptions, selectPages } from './run'

test('tests that CLI defaults and repeated browser flags are preserved', () => {
  expect(parseOptions([])).toMatchObject({
    pages: 'fixtures',
    keys: 'placeholder',
    solveTimeout: 150_000,
    headed: true,
  })
  expect(
    parseOptions([
      '--',
      '--pages',
      'all',
      '--browser-arg',
      '--one',
      '--browser-arg=--host-resolver-rules=MAP fixture.test 127.0.0.1',
    ]).browserArgs,
  ).toEqual(['--one', '--host-resolver-rules=MAP fixture.test 127.0.0.1'])
})

test('tests that invalid CLI options are usage errors', () => {
  for (const args of [
    ['--pages', 'bad'],
    ['--keys', 'bad'],
    ['--solve-timeout', '0'],
    ['--solve-timeout', 'NaN'],
    ['--solve-timeout', '2147483648'],
    ['--only', ''],
    ['--unknown'],
  ]) {
    expect(() => parseOptions(args)).toThrow()
  }
  expect(() => selectPages([], parseOptions(['--only', 'missing']))).toThrow()
})

test('tests that dev watch accepts a startup budget and rejects conflicting launch flags', () => {
  expect(parseOptions(['--dev-watch'])).toMatchObject({
    devWatch: true,
    startupTimeout: 600_000,
  })
  expect(
    parseOptions(['--dev-watch', '--startup-timeout', '120000']),
  ).toMatchObject({ startupTimeout: 120_000 })
  for (const flags of [
    ['--mcp-url', 'http://127.0.0.1:9402/mcp'],
    ['--headless'],
    ['--browser-arg=--anything'],
    ['--startup-timeout', '0'],
    ['--startup-timeout', 'NaN'],
    ['--startup-timeout', '2147483648'],
  ])
    expect(() => parseOptions(['--dev-watch', ...flags])).toThrow()
})

test('tests that browser flags with spaces arrive as one argument and environment is restored', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'captcha-launch-test-'))
  const binary = join(directory, "fake browser's binary")
  await writeFile(binary, '#!/bin/sh\nprintf "%s\\n" "$@"\n', { mode: 0o700 })
  const previous = process.env.BROWSEROS_BINARY
  process.env.BROWSEROS_BINARY = binary
  let wrapper = ''
  try {
    await launchConfiguredBrowser(
      true,
      ['--host-resolver-rules=MAP fixture.test 127.0.0.1'],
      async () => {
        wrapper = process.env.BROWSEROS_BINARY ?? ''
        const result = Bun.spawnSync([wrapper, '--from-harness'])
        expect(result.exitCode).toBe(0)
        expect(result.stdout.toString().trim().split('\n')).toEqual([
          '--from-harness',
          '--host-resolver-rules=MAP fixture.test 127.0.0.1',
        ])
        expect(process.env.BROWSEROS_TEST_HEADLESS).toBe('false')
        return {
          cdpPort: 1,
          userDataDir: directory,
          isRunning: async () => true,
          kill: async () => {},
        }
      },
    )
    expect(parseOptions(['--headless']).headed).toBe(false)
    expect(parseOptions(['--headed']).headed).toBe(true)
    expect(process.env.BROWSEROS_BINARY).toBe(binary)
    expect(await Bun.file(wrapper).exists()).toBe(false)
  } finally {
    if (previous === undefined) delete process.env.BROWSEROS_BINARY
    else process.env.BROWSEROS_BINARY = previous
    await rm(directory, { recursive: true, force: true })
  }
})
