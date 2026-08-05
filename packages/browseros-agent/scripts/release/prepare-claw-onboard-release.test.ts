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
  releaseRecords: Array<{
    tag_name: string
    draft: boolean
    target_commitish: string
  }> = [],
) {
  const releaseRecordsPath = join(dir, '.release-records.json')
  writeFileSync(releaseRecordsPath, `${JSON.stringify(releaseRecords)}\n`)
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
    '--release-records',
    releaseRecordsPath,
  ])
}

describe('prepare-claw-onboard-release', () => {
  it('resolves a manual onboarding version without creating a tag', async () => {
    const { dir, origin } = await fixture('0.0.2')
    try {
      const result = await prepare(dir, 'workflow_dispatch', '0.0.3')
      expect(result.code, result.stderr).toBe(0)
      expect(outputs(result.stdout)).toMatchObject({
        version: '0.0.3',
        tag: 'claw-onboard/v0.0.3',
        reservation: 'create',
      })
      expect(
        (
          await command(origin, [
            'git',
            'show-ref',
            '--verify',
            'refs/tags/claw-onboard/v0.0.3',
          ])
        ).code,
      ).not.toBe(0)
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
        reservation: 'create',
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

  it('rejects finalizing prepared onboarding older than a public release', async () => {
    const { dir, origin } = await fixture('0.0.4')
    try {
      const releaseSha = (
        await mustRun(dir, ['git', 'rev-parse', 'HEAD'])
      ).trim()
      const result = await prepare(dir, 'workflow_call', '0.0.5', 'main', [
        {
          tag_name: 'claw-onboard/v0.0.5',
          draft: true,
          target_commitish: releaseSha,
        },
        {
          tag_name: 'claw-onboard/v0.0.6',
          draft: false,
          target_commitish: 'newer-source',
        },
      ])

      expect(result.code).toBe(1)
      expect(`${result.stdout}\n${result.stderr}`).toContain(
        'Release version 0.0.5 is older than newest public claw onboarding version 0.0.6 (claw-onboard/v0.0.6)',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(origin, { recursive: true, force: true })
    }
  })
})
