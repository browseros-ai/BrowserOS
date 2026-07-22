import { describe, expect, it } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../..')
const prepareClawServerRustRelease = join(
  repoRoot,
  'scripts/release/prepare-claw-server-rust-release.sh',
)
const cargoTomlPath =
  'packages/browseros-agent/apps/claw-server-rust/Cargo.toml'

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

function writeCargoToml(dir: string, version: string): void {
  const path = join(dir, cargoTomlPath)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(
    path,
    [
      '[package]',
      'name = "claw-server-rust"',
      `version = "${version}"`,
      'edition = "2024"',
      '',
      '[[bin]]',
      'name = "browseros-claw-server-rs"',
      'path = "src/main.rs"',
      '',
    ].join('\n'),
  )
}

async function commitCargoToml(dir: string, version: string): Promise<void> {
  writeCargoToml(dir, version)
  await mustRun(dir, ['git', 'add', cargoTomlPath])
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
  const dir = mkdtempSync(join(tmpdir(), 'claw-server-rust-release-'))
  const bareDir = mkdtempSync(
    join(tmpdir(), 'claw-server-rust-release-origin-'),
  )
  await mustRun(dir, ['git', 'init', '--initial-branch=main'])
  await mustRun(dir, ['git', 'config', 'user.name', 'BrowserOS Test'])
  await mustRun(dir, ['git', 'config', 'user.email', 'test@browseros.com'])
  writeCargoToml(dir, version)
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

async function prepare(
  dir: string,
  options: {
    eventName: 'push' | 'workflow_dispatch' | 'workflow_call'
    refName?: string
    requestedVersion?: string
  },
) {
  return run(dir, [
    prepareClawServerRustRelease,
    '--event-name',
    options.eventName,
    '--default-branch',
    'main',
    '--ref-name',
    options.refName ?? 'main',
    '--requested-version',
    options.requestedVersion ?? '',
    '--release-ref',
    'HEAD',
  ])
}

describe('prepare-claw-server-rust-release', () => {
  it('continues the product sequence despite the historical Rust tag', async () => {
    const { dir, bareDir } = await initFixture('0.0.12')
    try {
      await tag(dir, 'claw-server/v0.0.12')
      await tag(dir, 'claw-server-rust/v0.1.0')
      await mustRun(dir, ['git', 'push', 'origin', '--tags'])
      const mainBefore = await revParse(bareDir, 'refs/heads/main')

      const result = await prepare(dir, {
        eventName: 'workflow_dispatch',
        requestedVersion: '0.0.13',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.13',
        tag: 'claw-server/v0.0.13',
        release_sha: mainBefore,
        previous_tag: 'claw-server/v0.0.12',
      })
      expect(await revParse(bareDir, 'claw-server/v0.0.13^{commit}')).toBe(
        mainBefore,
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('derives a workflow_call release version from Cargo.toml', async () => {
    const { dir, bareDir } = await initFixture('0.0.13')
    try {
      const releaseSha = await revParse(dir, 'HEAD')

      const result = await prepare(dir, {
        eventName: 'workflow_call',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.13',
        tag: 'claw-server/v0.0.13',
        release_sha: releaseSha,
      })
      expect(await revParse(bareDir, 'claw-server/v0.0.13^{commit}')).toBe(
        releaseSha,
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('derives a workflow_dispatch release version from Cargo.toml when omitted', async () => {
    const { dir, bareDir } = await initFixture('0.0.13')
    try {
      const releaseSha = await revParse(dir, 'HEAD')

      const result = await prepare(dir, {
        eventName: 'workflow_dispatch',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.13',
        tag: 'claw-server/v0.0.13',
        release_sha: releaseSha,
      })
      expect(await revParse(bareDir, 'claw-server/v0.0.13^{commit}')).toBe(
        releaseSha,
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })

  it('resolves pushed product tags and previous tags', async () => {
    const { dir, bareDir } = await initFixture('0.0.12')
    try {
      await tag(dir, 'claw-server/v0.0.12')
      await tag(dir, 'claw-server-rust/v0.1.0')
      await commitCargoToml(dir, '0.0.13')
      await tag(dir, 'claw-server/v0.0.13')
      await mustRun(dir, ['git', 'push', 'origin', 'main', '--tags'])

      const result = await prepare(dir, {
        eventName: 'push',
        refName: 'claw-server/v0.0.13',
      })

      expect(result.code, result.stderr || result.stdout).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.13',
        tag: 'claw-server/v0.0.13',
        previous_tag: 'claw-server/v0.0.12',
      })
    } finally {
      rmSync(dir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
    }
  })
})
