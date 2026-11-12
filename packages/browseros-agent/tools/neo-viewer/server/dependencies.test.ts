import { afterEach, describe, expect, test } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  dependencyProblems,
  ensureDependencies,
  MIN_INSTALL_BUN,
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

const PROBLEMS = [
  { name: '@fontsource/ibm-plex-sans', installed: null, locked: '5.3.0' },
  { name: 'react', installed: '19.2.0', locked: '19.3.0' },
]

test('missing or stale packages: any Bun at or above the minimum installs them itself', () => {
  for (const bunVersion of [MIN_INSTALL_BUN, '1.3.6', '1.4.2', '1.4.4']) {
    const plan = planDependencies({
      problems: PROBLEMS,
      bunVersion,
      pinned: '1.4.2',
      agentDir: '/repo/packages/browseros-agent',
    })
    expect(plan).toEqual({
      kind: 'install',
      args: ['install', '--frozen-lockfile'],
      cwd: '/repo/packages/browseros-agent',
      note: `Installing the viewer's packages with Bun ${bunVersion} (@fontsource/ibm-plex-sans missing, react 19.2.0 (lockfile 19.3.0)): bun install --frozen-lockfile in /repo/packages/browseros-agent`,
    })
  }
})

test('below the minimum: one line naming the Bun needed and the exact command', () => {
  for (const bunVersion of ['1.2.18', '1.1.0']) {
    const plan = planDependencies({
      problems: PROBLEMS,
      bunVersion,
      pinned: '1.4.2',
      agentDir: '/repo/packages/browseros-agent',
    })
    const message = plan.kind === 'stop' ? plan.message : ''
    expect(message).not.toContain('\n')
    expect(message).toContain(`Bun ${bunVersion} cannot install them`)
    expect(message).toContain(`it needs Bun ${MIN_INSTALL_BUN} or newer`)
    expect(message).toEndWith(
      'Run: cd /repo/packages/browseros-agent && npx -y bun@1.4.2 install --frozen-lockfile',
    )
  }
  // Nothing missing: nothing to say, whatever the Bun.
  expect(
    planDependencies({
      problems: [],
      bunVersion: '1.1.0',
      pinned: '1.4.2',
      agentDir: '/repo',
    }),
  ).toEqual({ kind: 'ready' })
})

test('the printed command survives a checkout path with spaces or quotes', () => {
  const stop = planDependencies({
    problems: [{ name: 'react', installed: null, locked: '19.3.0' }],
    bunVersion: '1.2.0',
    pinned: '1.4.2',
    agentDir: "/Users/me/my repo's/agent",
  })
  expect(stop.kind === 'stop' && stop.message).toContain(
    `cd '/Users/me/my repo'\\''s/agent' && npx`,
  )
})

test('the repo still names a Bun to suggest, and the running Bun is at or above the minimum', async () => {
  expect(await pinnedBun(AGENT_DIR)).toMatch(/^\d+\.\d+\.\d+$/)
  expect(Bun.semver.order(Bun.version, MIN_INSTALL_BUN)).toBeGreaterThanOrEqual(
    0,
  )
})

describe('a start with packages missing', () => {
  /** A workspace whose tool needs alpha 1.1.0 and has 1.0.0 installed: one stale package. */
  async function staleWorkspace() {
    const root = await mkdtemp(join(tmpdir(), 'neo-viewer-ensure-'))
    dirs.push(root)
    const tool = join(root, 'tool')
    await writeJson(join(root, 'package.json'), { packageManager: 'bun@1.4.2' })
    await writeFile(
      join(root, 'bun.lock'),
      '{ "lockfileVersion": 1, "packages": { "alpha": ["alpha@1.1.0", "", {}, "sha512-a"], }, }',
    )
    await writeJson(join(tool, 'package.json'), {
      dependencies: { alpha: '^1.1.0' },
    })
    const installed = join(tool, 'node_modules', 'alpha', 'package.json')
    await writeJson(installed, { version: '1.0.0' })
    return { root, tool, installed }
  }

  function run(
    workspace: { root: string; tool: string },
    bunVersion: string,
    install: (
      args: string[],
      cwd: string,
    ) => Promise<{ exitCode: number; output: string }>,
  ) {
    const said: string[] = []
    const calls: Array<{ args: string[]; cwd: string }> = []
    const outcome = ensureDependencies({
      toolDir: workspace.tool,
      agentDir: workspace.root,
      bunVersion,
      install: (args, cwd) => {
        calls.push({ args, cwd })
        return install(args, cwd)
      },
      say: (line) => said.push(line),
    })
    return { outcome, said, calls }
  }

  test('a Bun at or above the minimum installs with --frozen-lockfile, says so in one line and carries on', async () => {
    const workspace = await staleWorkspace()
    const { outcome, said, calls } = run(workspace, '1.4.4', async () => {
      await writeJson(workspace.installed, { version: '1.1.0' })
      return { exitCode: 0, output: 'Checked 1 install (no changes)' }
    })
    expect(await outcome).toEqual({ kind: 'installed' })
    expect(calls).toEqual([
      { args: ['install', '--frozen-lockfile'], cwd: workspace.root },
    ])
    expect(said).toHaveLength(1)
    expect(said[0]).toContain('with Bun 1.4.4 (alpha 1.0.0 (lockfile 1.1.0))')
  })

  test('an install that links nothing is caught by looking again, and stops with the command', async () => {
    const workspace = await staleWorkspace()
    const { outcome } = run(workspace, '1.4.4', async () => ({
      exitCode: 0,
      output: 'Checked 1 install (no changes)',
    }))
    const result = await outcome
    expect(result.kind).toBe('stop')
    const message = result.kind === 'stop' ? result.message : ''
    expect(message).toContain(
      'with Bun 1.4.4 did not link alpha 1.0.0 (lockfile 1.1.0)',
    )
    expect(message).toEndWith(`npx -y bun@1.4.2 install --frozen-lockfile`)
  })

  test('a failed install stops with its output kept for the owner', async () => {
    const workspace = await staleWorkspace()
    const { outcome } = run(workspace, '1.4.4', async () => ({
      exitCode: 1,
      output: 'error: lockfile had changes, but lockfile is frozen',
    }))
    expect(await outcome).toMatchObject({
      kind: 'stop',
      output: 'error: lockfile had changes, but lockfile is frozen',
    })
  })

  test('below the minimum it stops without trying to install', async () => {
    const workspace = await staleWorkspace()
    const { outcome, calls, said } = run(workspace, '1.2.18', async () => ({
      exitCode: 0,
      output: '',
    }))
    const result = await outcome
    expect(result.kind === 'stop' && result.message).toContain(
      `it needs Bun ${MIN_INSTALL_BUN} or newer`,
    )
    expect(calls).toHaveLength(0)
    expect(said).toHaveLength(0)
  })

  test('nothing missing: no install and no line at all, whatever the Bun', async () => {
    const workspace = await staleWorkspace()
    await writeJson(workspace.installed, { version: '1.1.0' })
    const { outcome, calls, said } = run(workspace, '1.4.4', async () => ({
      exitCode: 0,
      output: '',
    }))
    expect(await outcome).toEqual({ kind: 'ready' })
    expect(calls).toHaveLength(0)
    expect(said).toHaveLength(0)
  })
})
