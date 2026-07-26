import { describe, expect, it } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../..')
const prepareServerRelease = join(
  repoRoot,
  'scripts/release/prepare-server-release.sh',
)
const packagePath = 'packages/browseros-agent/apps/server/package.json'
const lockPath = 'packages/browseros-agent/bun.lock'

async function run(
  cwd: string,
  args: string[],
): Promise<{ code: number; stdout: string; stderr: string }> {
  const proc = Bun.spawn(args, {
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [stdout, stderr, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ])
  return { code, stdout, stderr }
}

async function mustRun(cwd: string, args: string[]): Promise<string> {
  const result = await run(cwd, args)
  expect(result.code, result.stderr || result.stdout).toBe(0)
  return result.stdout
}

function writePackage(dir: string, version: string): void {
  const path = join(dir, packagePath)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(
    path,
    `${JSON.stringify({ name: '@browseros/server', version }, null, 2)}\n`,
  )
}

function writeLock(dir: string, version: string): void {
  const path = join(dir, lockPath)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(
    path,
    [
      '{',
      '  "workspaces": {',
      '    "apps/server": {',
      '      "name": "@browseros/server",',
      `      "version": "${version}",`,
      '    },',
      '  },',
      '}',
      '',
    ].join('\n'),
  )
}

async function commitPackage(dir: string, version: string): Promise<void> {
  writePackage(dir, version)
  await mustRun(dir, ['git', 'add', packagePath])
  await mustRun(dir, ['git', 'commit', '-m', `version ${version}`])
}

async function tag(dir: string, name: string): Promise<void> {
  await mustRun(dir, ['git', 'tag', '-a', name, '-m', name])
}

async function revParse(dir: string, ref: string): Promise<string> {
  return (await mustRun(dir, ['git', 'rev-parse', ref])).trim()
}

async function initFixture(version: string): Promise<{
  dir: string
  bareDir: string
}> {
  const dir = mkdtempSync(join(tmpdir(), 'server-release-'))
  const bareDir = mkdtempSync(join(tmpdir(), 'server-release-origin-'))
  await mustRun(dir, ['git', 'init', '--initial-branch=main'])
  await mustRun(dir, ['git', 'config', 'user.name', 'BrowserOS Test'])
  await mustRun(dir, ['git', 'config', 'user.email', 'test@browseros.com'])
  writePackage(dir, version)
  writeLock(dir, version)
  await mustRun(dir, ['git', 'add', '.'])
  await mustRun(dir, ['git', 'commit', '-m', `version ${version}`])
  await mustRun(bareDir, ['git', 'init', '--bare', '--initial-branch=main'])
  await mustRun(dir, ['git', 'remote', 'add', 'origin', bareDir])
  await mustRun(dir, ['git', 'push', '-u', 'origin', 'main'])
  return { dir, bareDir }
}

function parseOutput(stdout: string): Record<string, string> {
  return Object.fromEntries(
    stdout
      .trim()
      .split('\n')
      .filter(Boolean)
      .filter((line) => !line.startsWith('::'))
      .map((line) => line.split(/=(.*)/s).slice(0, 2)),
  )
}

function outputText(result: { stdout: string; stderr: string }): string {
  return `${result.stdout}\n${result.stderr}`
}

async function prepare(
  dir: string,
  options: {
    eventName: 'push' | 'workflow_dispatch' | 'workflow_call'
    refName?: string
    requestedVersion?: string
  },
) {
  return run(dir, [
    prepareServerRelease,
    '--event-name',
    options.eventName,
    '--default-branch',
    'main',
    '--ref-name',
    options.refName ?? 'main',
    '--requested-version',
    options.requestedVersion ?? '',
  ])
}

describe('prepare-server-release', () => {
  it('pushes only the tag on manual dispatch and leaves the default branch untouched', async () => {
    const { dir, bareDir } = await initFixture('0.0.122')
    try {
      const mainBefore = await revParse(bareDir, 'refs/heads/main')

      const result = await prepare(dir, {
        eventName: 'workflow_dispatch',
        requestedVersion: '0.0.124',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.124',
        tag: 'agent-server/v0.0.124',
        release_sha: mainBefore,
        previous_tag: '',
      })

      // The default branch must be identical — no bump commit, no branch push.
      expect(await revParse(bareDir, 'refs/heads/main')).toBe(mainBefore)
      expect(
        await mustRun(bareDir, [
          'git',
          'show',
          `refs/heads/main:${packagePath}`,
        ]),
      ).toContain('"version": "0.0.122"')

      // Only the annotated tag was published, pointing at the default-branch head.
      expect(
        (
          await mustRun(bareDir, [
            'git',
            'cat-file',
            '-t',
            'refs/tags/agent-server/v0.0.124',
          ])
        ).trim(),
      ).toBe('tag')
      expect(await revParse(bareDir, 'agent-server/v0.0.124^{commit}')).toBe(
        mainBefore,
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('creates the release tag without a preconfigured git identity', async () => {
    const { dir, bareDir } = await initFixture('0.0.123')
    try {
      const releaseSha = await revParse(dir, 'HEAD')
      await mustRun(dir, ['git', 'config', '--unset', 'user.name'])
      await mustRun(dir, ['git', 'config', '--unset', 'user.email'])

      const result = await prepare(dir, {
        eventName: 'workflow_dispatch',
        requestedVersion: '0.0.124',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.124',
        tag: 'agent-server/v0.0.124',
        release_sha: releaseSha,
      })
      expect(await revParse(bareDir, 'agent-server/v0.0.124^{commit}')).toBe(
        releaseSha,
      )
      expect(await revParse(bareDir, 'refs/heads/main')).toBe(releaseSha)
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('re-resolves an already published release tag without recreating it', async () => {
    const { dir, bareDir } = await initFixture('0.0.123')
    try {
      const releaseSha = await revParse(dir, 'HEAD')
      await tag(dir, 'agent-server/v0.0.124')
      await mustRun(dir, ['git', 'push', 'origin', 'agent-server/v0.0.124'])

      const result = await prepare(dir, {
        eventName: 'workflow_dispatch',
        requestedVersion: '0.0.124',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.124',
        tag: 'agent-server/v0.0.124',
        release_sha: releaseSha,
        previous_tag: '',
      })
      expect(await revParse(bareDir, 'refs/heads/main')).toBe(releaseSha)
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('derives a manual release version from package.json when no version is requested', async () => {
    const { dir, bareDir } = await initFixture('0.0.124')
    try {
      const releaseSha = await revParse(dir, 'HEAD')

      const result = await prepare(dir, {
        eventName: 'workflow_call',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.124',
        tag: 'agent-server/v0.0.124',
        release_sha: releaseSha,
      })
      expect(await revParse(bareDir, 'agent-server/v0.0.124^{commit}')).toBe(
        releaseSha,
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('resolves a pushed server tag and previous release tag', async () => {
    const { dir, bareDir } = await initFixture('0.0.122')
    try {
      await tag(dir, 'agent-server/v0.0.122')
      await commitPackage(dir, '0.0.123')
      await tag(dir, 'agent-server/v0.0.123')
      await mustRun(dir, ['git', 'push', 'origin', 'main', '--tags'])

      const result = await prepare(dir, {
        eventName: 'push',
        refName: 'agent-server/v0.0.123',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.123',
        tag: 'agent-server/v0.0.123',
        previous_tag: 'agent-server/v0.0.122',
      })
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('resolves a pushed tag even when package.json does not match the version', async () => {
    const { dir, bareDir } = await initFixture('0.0.122')
    try {
      await tag(dir, 'agent-server/v0.0.123')
      await mustRun(dir, ['git', 'push', 'origin', 'main', '--tags'])

      const result = await prepare(dir, {
        eventName: 'push',
        refName: 'agent-server/v0.0.123',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.123',
        tag: 'agent-server/v0.0.123',
      })
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('rejects a pushed tag whose commit is not on the default branch', async () => {
    const { dir, bareDir } = await initFixture('0.0.122')
    try {
      await mustRun(dir, ['git', 'checkout', '-b', 'release-side'])
      await commitPackage(dir, '0.0.123')
      await tag(dir, 'agent-server/v0.0.123')
      await mustRun(dir, ['git', 'push', 'origin', 'agent-server/v0.0.123'])

      const result = await prepare(dir, {
        eventName: 'push',
        refName: 'agent-server/v0.0.123',
      })

      expect(result.code).toBe(1)
      expect(outputText(result)).toContain('is not reachable from origin/main')
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('rejects a release version that already exists as a legacy tag', async () => {
    const { dir, bareDir } = await initFixture('0.0.123')
    try {
      await tag(dir, 'browseros-server-v0.0.123')
      await tag(dir, 'agent-server/v0.0.123')
      await mustRun(dir, ['git', 'push', 'origin', 'main', '--tags'])

      const result = await prepare(dir, {
        eventName: 'push',
        refName: 'agent-server/v0.0.123',
      })

      expect(result.code).toBe(1)
      expect(outputText(result)).toContain(
        'Release version 0.0.123 already exists as tag browseros-server-v0.0.123',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('rejects a manual dispatch that does not increment past the latest tag', async () => {
    const { dir, bareDir } = await initFixture('0.0.123')
    try {
      await tag(dir, 'agent-server/v0.0.124')
      await mustRun(dir, ['git', 'push', 'origin', 'main', '--tags'])

      const result = await prepare(dir, {
        eventName: 'workflow_dispatch',
        requestedVersion: '0.0.123',
      })

      expect(result.code).toBe(1)
      expect(outputText(result)).toContain(
        'Release version 0.0.123 must be greater than latest existing server version 0.0.124 (agent-server/v0.0.124)',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })
})
