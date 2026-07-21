import { describe, expect, it } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../..')
const resolver = join(
  repoRoot,
  'scripts/release/prepare-claw-onboard-release.sh',
)
const packagePath = 'packages/browseros-agent/apps/claw-onboard/package.json'

async function command(cwd: string, args: string[]) {
  const process = Bun.spawn(args, { cwd, stdout: 'pipe', stderr: 'pipe' })
  const [stdout, stderr, code] = await Promise.all([
    new Response(process.stdout).text(),
    new Response(process.stderr).text(),
    process.exited,
  ])
  return { stdout, stderr, code }
}

async function mustRun(cwd: string, args: string[]): Promise<string> {
  const result = await command(cwd, args)
  expect(result.code, result.stderr || result.stdout).toBe(0)
  return result.stdout
}

async function fixture(version: string) {
  const dir = mkdtempSync(join(tmpdir(), 'claw-onboard-release-'))
  const origin = mkdtempSync(join(tmpdir(), 'claw-onboard-origin-'))
  const path = join(dir, packagePath)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(
    path,
    `${JSON.stringify({ name: '@browseros/claw-onboard', version }, null, 2)}\n`,
  )
  await mustRun(dir, ['git', 'init', '--initial-branch=main'])
  await mustRun(dir, ['git', 'config', 'user.name', 'BrowserOS Test'])
  await mustRun(dir, ['git', 'config', 'user.email', 'test@browseros.com'])
  await mustRun(dir, ['git', 'add', '.'])
  await mustRun(dir, ['git', 'commit', '-m', `version ${version}`])
  await mustRun(origin, ['git', 'init', '--bare', '--initial-branch=main'])
  await mustRun(dir, ['git', 'remote', 'add', 'origin', origin])
  await mustRun(dir, ['git', 'push', '-u', 'origin', 'main'])
  return { dir, origin }
}

function outputs(stdout: string): Record<string, string> {
  return Object.fromEntries(
    stdout
      .trim()
      .split('\n')
      .filter((line) => line && !line.startsWith('::'))
      .map((line) => line.split(/=(.*)/s).slice(0, 2)),
  )
}

async function prepare(
  dir: string,
  eventName: 'push' | 'workflow_dispatch' | 'workflow_call',
  version = '',
  refName = 'main',
) {
  return await command(dir, [
    resolver,
    '--event-name',
    eventName,
    '--default-branch',
    'main',
    '--ref-name',
    refName,
    '--requested-version',
    version,
  ])
}

describe('prepare-claw-onboard-release', () => {
  it('creates a manual onboarding tag', async () => {
    const { dir, origin } = await fixture('0.0.2')
    try {
      const result = await prepare(dir, 'workflow_dispatch', '0.0.3')
      expect(result.code, result.stderr).toBe(0)
      expect(outputs(result.stdout)).toMatchObject({
        version: '0.0.3',
        tag: 'claw-onboard/v0.0.3',
      })
      expect(
        (
          await mustRun(origin, [
            'git',
            'rev-parse',
            'claw-onboard/v0.0.3^{commit}',
          ])
        ).trim(),
      ).toBe((await mustRun(dir, ['git', 'rev-parse', 'HEAD'])).trim())
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(origin, { recursive: true, force: true })
    }
  })

  it('derives workflow-call versions from claw-onboard', async () => {
    const { dir, origin } = await fixture('0.0.4')
    try {
      const result = await prepare(dir, 'workflow_call')
      expect(result.code, result.stderr).toBe(0)
      expect(outputs(result.stdout)).toMatchObject({
        version: '0.0.4',
        tag: 'claw-onboard/v0.0.4',
      })
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(origin, { recursive: true, force: true })
    }
  })

  it('resolves pushed onboarding tags', async () => {
    const { dir, origin } = await fixture('0.0.4')
    try {
      await mustRun(dir, [
        'git',
        'tag',
        '-a',
        'claw-onboard/v0.0.4',
        '-m',
        'release',
      ])
      await mustRun(dir, ['git', 'push', 'origin', '--tags'])
      const result = await prepare(dir, 'push', '', 'claw-onboard/v0.0.4')
      expect(result.code, result.stderr).toBe(0)
      expect(outputs(result.stdout)).toMatchObject({
        version: '0.0.4',
        tag: 'claw-onboard/v0.0.4',
      })
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(origin, { recursive: true, force: true })
    }
  })
})
