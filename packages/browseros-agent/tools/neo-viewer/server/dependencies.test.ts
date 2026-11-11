import { afterEach, expect, test } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  bunNote,
  dependencyProblems,
  pinnedBun,
  planDependencies,
} from './dependencies'

const TOOL_DIR = join(import.meta.dir, '..')
const AGENT_DIR = join(TOOL_DIR, '..', '..')
const dirs: string[] = []
afterEach(async () => {
  for (const dir of dirs.splice(0))
    await rm(dir, { recursive: true, force: true })
})

async function writeJson(path: string, value: unknown) {
  await mkdir(join(path, '..'), { recursive: true })
  await writeFile(path, JSON.stringify(value))
}

test('a stale or missing package is found against the tool’s package.json and bun.lock', async () => {
  const root = await mkdtemp(join(tmpdir(), 'neo-viewer-deps-'))
  dirs.push(root)
  const tool = join(root, 'tool')
  await writeJson(join(tool, 'package.json'), {
    dependencies: { alpha: '^1.0.0', beta: '^1.0.0' },
    devDependencies: { gamma: '^2.0.0', '@types/alpha': '^1.0.0' },
  })
  await writeJson(join(tool, 'node_modules', 'alpha', 'package.json'), {
    version: '1.0.0',
  })
  await writeJson(join(tool, 'node_modules', 'gamma', 'package.json'), {
    version: '2.0.0',
  })
  // bun.lock: JSON with trailing commas; the workspace's own key wins over the top-level one.
  await writeFile(
    join(root, 'bun.lock'),
    `{
  "lockfileVersion": 1,
  "packages": {
    "alpha": ["alpha@1.1.0", "", {}, "sha512-a"],
    "beta": ["beta@1.0.0", "", {}, "sha512-b"],
    "gamma": ["gamma@1.0.0", "", {}, "sha512-c"],
    "@browseros/neo-viewer/gamma": ["gamma@2.0.0", "", {}, "sha512-d"],
  },
}`,
  )
  expect(await dependencyProblems(tool, root)).toEqual([
    { name: 'alpha', installed: '1.0.0', locked: '1.1.0' },
    { name: 'beta', installed: null, locked: '1.0.0' },
  ])
})

test('this checkout’s packages match its bun.lock', async () => {
  expect(await dependencyProblems(TOOL_DIR, AGENT_DIR)).toEqual([])
})

test('missing or stale packages: the pinned Bun installs, another Bun stops with the exact command', () => {
  const base = {
    problems: [
      { name: '@fontsource/ibm-plex-sans', installed: null, locked: '5.3.0' },
      { name: 'react', installed: '19.2.0', locked: '19.3.0' },
    ],
    pinned: '1.4.2',
    agentDir: '/repo/packages/browseros-agent',
  }
  expect(planDependencies({ ...base, bunVersion: '1.4.2' })).toEqual({
    kind: 'install',
    command: ['bun', 'install', '--frozen-lockfile'],
    cwd: '/repo/packages/browseros-agent',
  })
  const stop = planDependencies({ ...base, bunVersion: '1.3.6' })
  const message = stop.kind === 'stop' ? stop.message : ''
  expect(message).toContain(
    '@fontsource/ibm-plex-sans missing, react 19.2.0 (lockfile 19.3.0)',
  )
  expect(message).toContain('this is Bun 1.3.6')
  expect(message).toContain(
    'Run: cd /repo/packages/browseros-agent && npx -y bun@1.4.2 install --frozen-lockfile',
  )
  expect(
    planDependencies({ ...base, problems: [], bunVersion: '1.3.6' }),
  ).toEqual({ kind: 'ready' })
})

test('the printed command survives a checkout path with spaces or quotes', () => {
  const stop = planDependencies({
    problems: [{ name: 'react', installed: null, locked: '19.3.0' }],
    bunVersion: '1.3.6',
    pinned: '1.4.2',
    agentDir: "/Users/me/my repo's/agent",
  })
  expect(stop.kind === 'stop' && stop.message).toContain(
    `cd '/Users/me/my repo'\\''s/agent' && npx`,
  )
})

test('a Bun other than the pinned one is named in one line', async () => {
  const pinned = await pinnedBun(AGENT_DIR)
  expect(pinned).toMatch(/^\d+\.\d+\.\d+$/)
  expect(bunNote(pinned ?? '', pinned)).toBeNull()
  expect(bunNote('1.3.6', '1.4.2')).toBe(
    "Note: this is Bun 1.3.6; the repo pins Bun 1.4.2, and only its install links this tool's packages.",
  )
})
