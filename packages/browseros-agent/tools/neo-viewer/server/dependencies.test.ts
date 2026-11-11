import { expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  missingDependencies,
  pinnedBun,
  planDependencies,
} from './dependencies'

test("a folder without this tool's packages has every one missing; this checkout has none", async () => {
  const bare = await mkdtemp(join(tmpdir(), 'neo-viewer-deps-'))
  try {
    expect(missingDependencies(bare)).toEqual([
      'react',
      'react-dom',
      'tailwindcss',
      '@tailwindcss/node',
      '@tailwindcss/oxide',
    ])
  } finally {
    await rm(bare, { recursive: true, force: true })
  }
  expect(missingDependencies(join(import.meta.dir, '..'))).toEqual([])
})

test('missing packages: the pinned Bun installs them, another Bun stops with the exact command', () => {
  const base = {
    missing: ['@tailwindcss/node'],
    pinned: '1.4.2',
    agentDir: '/repo/packages/browseros-agent',
  }
  expect(planDependencies({ ...base, bunVersion: '1.4.2' })).toEqual({
    kind: 'install',
    command: ['bun', 'install', '--frozen-lockfile'],
    cwd: '/repo/packages/browseros-agent',
  })
  const stop = planDependencies({ ...base, bunVersion: '1.3.6' })
  expect(stop.kind).toBe('stop')
  expect(stop.kind === 'stop' && stop.message).toContain(
    'Run: cd /repo/packages/browseros-agent && npx -y bun@1.4.2 install --frozen-lockfile',
  )
  expect(stop.kind === 'stop' && stop.message).toContain('this is Bun 1.3.6')
  expect(
    planDependencies({ ...base, missing: [], bunVersion: '1.3.6' }),
  ).toEqual({ kind: 'ready' })
})

test("the pinned Bun is the agent package's packageManager", async () => {
  expect(await pinnedBun(join(import.meta.dir, '..', '..', '..'))).toMatch(
    /^\d+\.\d+\.\d+$/,
  )
})
