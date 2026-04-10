/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Build smoke test — compiles the server binary and verifies --version output.
 * Catches compile failures, broken imports, and version injection bugs.
 */

import { afterAll, describe, it } from 'bun:test'
import assert from 'node:assert'
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

function getNativeTarget(): { id: string; ext: string } {
  const os =
    process.platform === 'darwin'
      ? 'darwin'
      : process.platform === 'win32'
        ? 'windows'
        : 'linux'
  const cpu = process.arch === 'arm64' ? 'arm64' : 'x64'
  return { id: `${os}-${cpu}`, ext: process.platform === 'win32' ? '.exe' : '' }
}

const REQUIRED_INLINE_ENV_KEYS = [
  'BROWSEROS_CONFIG_URL',
  'CODEGEN_SERVICE_URL',
  'POSTHOG_API_KEY',
  'SENTRY_DSN',
] as const

const R2_ENV_KEYS = [
  'R2_ACCOUNT_ID',
  'R2_ACCESS_KEY_ID',
  'R2_SECRET_ACCESS_KEY',
  'R2_BUCKET',
] as const

const PROD_SECRET_KEYS = [...REQUIRED_INLINE_ENV_KEYS, ...R2_ENV_KEYS]

const INLINE_ENV_STUBS: Record<string, string> = {
  BROWSEROS_CONFIG_URL: 'https://stub.test/config',
  CODEGEN_SERVICE_URL: 'https://stub.test/codegen',
  POSTHOG_API_KEY: 'phc_test_stub',
  SENTRY_DSN: 'https://stub@sentry.test/0',
}

const R2_ENV_STUBS: Record<string, string> = {
  R2_ACCOUNT_ID: 'test',
  R2_ACCESS_KEY_ID: 'test',
  R2_SECRET_ACCESS_KEY: 'test',
  R2_BUCKET: 'test',
}

describe('server build', () => {
  const rootDir = resolve(import.meta.dir, '../../..')
  const serverPkgPath = resolve(rootDir, 'apps/server/package.json')
  const prodEnvPath = resolve(rootDir, 'apps/server/.env.production')
  const prodEnvTemplatePath = resolve(
    rootDir,
    'apps/server/.env.production.example',
  )
  const originalProdEnv = existsSync(prodEnvPath)
    ? readFileSync(prodEnvPath, 'utf-8')
    : null
  const prodEnvTemplate = readFileSync(prodEnvTemplatePath, 'utf-8')
  const buildScript = resolve(rootDir, 'scripts/build/server.ts')
  const target = getNativeTarget()
  const binaryPath = resolve(
    rootDir,
    `dist/prod/server/.tmp/binaries/browseros-server-${target.id}${target.ext}`,
  )
  const zipPath = resolve(
    rootDir,
    `dist/prod/server/browseros-server-resources-${target.id}.zip`,
  )
  const tempDir = mkdtempSync(join(tmpdir(), 'browseros-build-test-'))
  const emptyManifestPath = join(tempDir, 'empty-manifest.json')
  writeFileSync(emptyManifestPath, JSON.stringify({ resources: [] }))

  function buildEnv(
    extraEnv: Record<string, string>,
    omitKeys: readonly string[] = [],
  ): NodeJS.ProcessEnv {
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      ...extraEnv,
    }
    for (const key of omitKeys) {
      delete env[key]
    }
    return env
  }

  function resetProdEnvToTemplate(): void {
    writeFileSync(prodEnvPath, prodEnvTemplate)
  }

  afterAll(() => {
    rmSync(tempDir, { recursive: true, force: true })
    if (originalProdEnv === null) {
      rmSync(prodEnvPath, { force: true })
      return
    }
    writeFileSync(prodEnvPath, originalProdEnv)
  })

  it('compiles and --version outputs correct version', async () => {
    resetProdEnvToTemplate()
    const pkg = await Bun.file(serverPkgPath).json()
    const expectedVersion: string = pkg.version

    const build = Bun.spawn(
      [
        'bun',
        buildScript,
        `--target=${target.id}`,
        '--no-upload',
        `--manifest=${emptyManifestPath}`,
      ],
      {
        cwd: rootDir,
        stdout: 'pipe',
        stderr: 'pipe',
        env: buildEnv({ ...INLINE_ENV_STUBS, ...R2_ENV_STUBS }),
      },
    )
    const buildExit = await build.exited
    if (buildExit !== 0) {
      const stderr = await new Response(build.stderr).text()
      assert.fail(`Build failed (exit ${buildExit}):\n${stderr}`)
    }

    const proc = Bun.spawn([binaryPath, '--version'], {
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const [versionOutput, versionStderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ])
    const versionExit = await proc.exited

    assert.strictEqual(
      versionExit,
      0,
      `Binary --version exited non-zero:\n${versionStderr}`,
    )
    assert.strictEqual(versionOutput.trim(), expectedVersion)
  }, 300_000)

  it('archives CI builds without R2 config or production env secrets', async () => {
    resetProdEnvToTemplate()
    rmSync(zipPath, { force: true })

    const build = Bun.spawn(
      ['bun', buildScript, `--target=${target.id}`, '--ci'],
      {
        cwd: rootDir,
        stdout: 'pipe',
        stderr: 'pipe',
        env: buildEnv({}, PROD_SECRET_KEYS),
      },
    )
    const buildExit = await build.exited
    if (buildExit !== 0) {
      const stderr = await new Response(build.stderr).text()
      assert.fail(`CI build failed (exit ${buildExit}):\n${stderr}`)
    }

    assert.ok(existsSync(zipPath), `Expected archive at ${zipPath}`)
  }, 300_000)
})
