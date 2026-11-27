/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { spawnSync } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  buildTestCommand,
  getAtomicGroupTargets,
  listAllGroups,
  withTestEnv,
} from './__helpers__/run-test-group'

describe('withTestEnv', () => {
  it('defaults NODE_ENV to test when absent', () => {
    expect(withTestEnv({ PATH: '/usr/bin' }).NODE_ENV).toBe('test')
  })

  it('preserves an explicit NODE_ENV', () => {
    expect(withTestEnv({ NODE_ENV: 'production' }).NODE_ENV).toBe('production')
  })
})

describe('buildTestCommand', () => {
  it('preloads the test env bootstrap before running targets', () => {
    expect(buildTestCommand(['./tests/api'])).toEqual([
      process.execPath,
      '--env-file=../../.env.development',
      'test',
      '--preload=./tests/__helpers__/test-env.ts',
      './tests/api',
    ])
  })
})

describe('test groups', () => {
  it('includes the lib tests in the group list', () => {
    expect(listAllGroups()).toContain('lib')
  })

  it('excludes helper-only directories from the group list', () => {
    expect(listAllGroups()).not.toContain('_helpers')
    expect(listAllGroups()).not.toContain('__helpers__')
    expect(listAllGroups()).not.toContain('__fixtures__')
  })

  it('runs available integration tests in the integration group', () => {
    expect(getAtomicGroupTargets('integration')).toEqual([
      './tests/server.integration.test.ts',
    ])
  })

  it('does not duplicate group names', () => {
    const groups = listAllGroups()

    expect(new Set(groups).size).toBe(groups.length)
  })
})

/** Exercise the real CLI without launching a browser or the broad cleanup script. */
async function withRunnerFixture(
  run: (root: string, server: string) => Promise<void>,
) {
  const root = await mkdtemp(join(tmpdir(), 'test-group-routing-'))
  const server = join(root, 'apps/server')
  const helpers = join(server, 'tests/__helpers__')
  try {
    await mkdir(helpers, { recursive: true })
    await mkdir(join(server, 'tests/lib'))
    await mkdir(join(root, 'home'))
    await writeFile(
      join(helpers, 'run-test-group.ts'),
      await readFile(
        join(import.meta.dir, '__helpers__/run-test-group.ts'),
        'utf8',
      ),
    )
    await writeFile(join(helpers, 'test-env.ts'), '')
    await writeFile(join(root, '.env.development'), '')
    // A harmless positive-control marker stands in for the real cleanup script.
    await writeFile(
      join(helpers, 'cleanup.sh'),
      "#!/bin/sh\nprintf 'cleanup\\n' >> cleanup-receipt\n",
    )
    for (const name of ['index', 'other', 'lib/fixture']) {
      await writeFile(
        join(server, `tests/${name}.test.ts`),
        `import { test } from 'bun:test'
import { writeFileSync } from 'node:fs'
test('routing fixture', () => writeFileSync(${JSON.stringify(join(root, `${name.replaceAll('/', '-')}-receipt`))}, 'ran'))
`,
      )
    }
    await writeFile(
      join(server, 'tests/server.integration.test.ts'),
      "throw new Error('integration must not run in root or lib')\n",
    )
    await run(root, server)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}

function runFixture(root: string, server: string, group: string) {
  const result = spawnSync(
    process.execPath,
    ['--no-env-file', './tests/__helpers__/run-test-group.ts', group],
    {
      cwd: server,
      env: {
        PATH: process.env.PATH,
        HOME: join(root, 'home'),
        NODE_ENV: 'test',
      },
      encoding: 'utf8',
      timeout: 5_000,
    },
  )
  expect(result.error).toBeUndefined()
  expect(result.signal).toBeNull()
  if (result.status !== 0)
    throw new Error(
      `Fixture runner failed:\n${result.stdout}\n${result.stderr}`,
    )
}

describe('test group CLI routing', () => {
  it('runs the root E2E entrypoint without broad cleanup or integration tests', async () => {
    await withRunnerFixture(async (root, server) => {
      runFixture(root, server, 'root')
      expect(await readFile(join(root, 'index-receipt'), 'utf8')).toBe('ran')
      expect(await readFile(join(root, 'other-receipt'), 'utf8')).toBe('ran')
      await expect(readFile(join(server, 'cleanup-receipt'))).rejects.toThrow()
      await expect(
        readFile(join(root, 'lib-fixture-receipt')),
      ).rejects.toThrow()
    })
  })

  it('still invokes cleanup for a non-root group and only runs that group', async () => {
    await withRunnerFixture(async (root, server) => {
      runFixture(root, server, 'lib')
      expect(await readFile(join(server, 'cleanup-receipt'), 'utf8')).toBe(
        'cleanup\n',
      )
      expect(await readFile(join(root, 'lib-fixture-receipt'), 'utf8')).toBe(
        'ran',
      )
      await expect(readFile(join(root, 'index-receipt'))).rejects.toThrow()
      await expect(readFile(join(root, 'other-receipt'))).rejects.toThrow()
    })
  })
})
