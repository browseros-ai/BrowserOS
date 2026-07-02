import { afterAll, describe, it } from 'bun:test'
import assert from 'node:assert'
import { existsSync, readFileSync, rmSync } from 'node:fs'
import { resolve } from 'node:path'
import {
  loadBuildConfig,
  parseBuildArgs,
} from '../../../packages/build-server-tools/src'
import { clawServerBuildProduct } from '../../../scripts/build/claw-server/descriptor'

function getNativeTarget(): { id: string; ext: string; stagedName: string } {
  const os =
    process.platform === 'darwin'
      ? 'darwin'
      : process.platform === 'win32'
        ? 'windows'
        : 'linux'
  const cpu = process.arch === 'arm64' ? 'arm64' : 'x64'
  const ext = process.platform === 'win32' ? '.exe' : ''
  return {
    id: `${os}-${cpu}`,
    ext,
    stagedName: `browseros-claw-server${ext}`,
  }
}

const UNNEEDED_SERVER_AND_R2_ENV_KEYS = [
  'BROWSEROS_CONFIG_URL',
  'POSTHOG_API_KEY',
  'SENTRY_DSN',
  'R2_ACCOUNT_ID',
  'R2_ACCESS_KEY_ID',
  'R2_SECRET_ACCESS_KEY',
  'R2_BUCKET',
] as const

describe('claw server build', () => {
  const rootDir = resolve(import.meta.dir, '../../..')
  const versionPkgPath = resolve(rootDir, 'apps/claw-server/package.json')
  const buildScript = resolve(rootDir, 'scripts/build/claw-server.ts')
  const target = getNativeTarget()
  const binaryPath = resolve(
    rootDir,
    `dist/prod/claw-server/.tmp/binaries/browseros-claw-server-${target.id}${target.ext}`,
  )
  const artifactRoot = resolve(rootDir, `dist/prod/claw-server/${target.id}`)
  const stagedBinaryPath = resolve(
    artifactRoot,
    `resources/bin/${target.stagedName}`,
  )
  const metadataPath = resolve(artifactRoot, 'artifact-metadata.json')
  const migrationJournalPath = resolve(
    artifactRoot,
    'resources/db/migrations/meta/_journal.json',
  )
  const zipPath = resolve(
    rootDir,
    `dist/prod/claw-server/browseros-claw-server-resources-${target.id}.zip`,
  )

  function buildEnv(omitKeys: readonly string[] = []): NodeJS.ProcessEnv {
    const env = snapshotProcessEnv()
    for (const key of omitKeys) {
      delete env[key]
    }
    return env
  }

  afterAll(() => {
    rmSync(resolve(rootDir, 'dist/prod/claw-server'), {
      recursive: true,
      force: true,
    })
  })

  it('resolves the build version from the claw package', async () => {
    const pkg = await Bun.file(versionPkgPath).json()
    const config = loadBuildConfig(rootDir, clawServerBuildProduct, {
      ci: true,
    })
    assert.strictEqual(config.version, pkg.version)
  })

  it('uploads artifacts by default while preserving local-only modes', () => {
    assert.strictEqual(
      parseBuildArgs(['--target=darwin-arm64'], clawServerBuildProduct).upload,
      true,
    )
    assert.strictEqual(
      parseBuildArgs(
        ['--target=darwin-arm64', '--no-upload'],
        clawServerBuildProduct,
      ).upload,
      false,
    )
    assert.strictEqual(
      parseBuildArgs(['--target=darwin-arm64', '--ci'], clawServerBuildProduct)
        .upload,
      false,
    )
  })

  it('builds a local artifact without apps/server env files', async () => {
    rmSync(zipPath, { force: true })
    const pkg = await Bun.file(versionPkgPath).json()
    const expectedVersion: string = pkg.version

    const build = Bun.spawn(
      ['bun', buildScript, `--target=${target.id}`, '--no-upload'],
      {
        cwd: rootDir,
        stdout: 'pipe',
        stderr: 'pipe',
        env: buildEnv(UNNEEDED_SERVER_AND_R2_ENV_KEYS),
      },
    )
    const buildExit = await build.exited
    if (buildExit !== 0) {
      const stderr = await new Response(build.stderr).text()
      assert.fail(`Claw build failed (exit ${buildExit}):\n${stderr}`)
    }

    assert.ok(existsSync(binaryPath), `Expected raw binary at ${binaryPath}`)
    assert.ok(
      existsSync(stagedBinaryPath),
      `Expected staged Claw binary at ${stagedBinaryPath}`,
    )
    assert.ok(existsSync(zipPath), `Expected archive at ${zipPath}`)
    assert.ok(
      existsSync(migrationJournalPath),
      `Expected packaged migrations at ${migrationJournalPath}`,
    )

    const metadata = JSON.parse(readFileSync(metadataPath, 'utf-8'))
    assert.strictEqual(metadata.version, expectedVersion)
    assert.strictEqual(metadata.target, target.id)

    const versionResult = await collectProcess(
      Bun.spawn([binaryPath, '--version'], {
        stdout: 'pipe',
        stderr: 'pipe',
      }),
    )
    assert.strictEqual(
      versionResult.exitCode,
      0,
      `Binary --version exited non-zero:\n${versionResult.stderr}`,
    )
    const actualVersion = versionResult.stdout.trim()
    assert.strictEqual(actualVersion, expectedVersion)
    assert.notStrictEqual(actualVersion, Bun.version)

    const zipListing = await collectProcess(
      Bun.spawn(['unzip', '-l', zipPath], {
        stdout: 'pipe',
        stderr: 'pipe',
      }),
    )
    assert.strictEqual(
      zipListing.exitCode,
      0,
      `Unable to inspect zip:\n${zipListing.stderr}`,
    )
    assert.match(
      zipListing.stdout,
      new RegExp(`resources/bin/${target.stagedName.replace('.', '\\.')}`),
    )
    assert.match(
      zipListing.stdout,
      /resources\/db\/migrations\/meta\/_journal\.json/,
    )
  }, 300_000)

  it('archives CI builds without R2 credentials', async () => {
    rmSync(zipPath, { force: true })

    const build = Bun.spawn(
      ['bun', buildScript, `--target=${target.id}`, '--ci'],
      {
        cwd: rootDir,
        stdout: 'pipe',
        stderr: 'pipe',
        env: buildEnv(UNNEEDED_SERVER_AND_R2_ENV_KEYS),
      },
    )
    const buildExit = await build.exited
    if (buildExit !== 0) {
      const stderr = await new Response(build.stderr).text()
      assert.fail(`Claw CI build failed (exit ${buildExit}):\n${stderr}`)
    }

    assert.ok(existsSync(zipPath), `Expected archive at ${zipPath}`)
    assert.ok(
      existsSync(stagedBinaryPath),
      `Expected staged Claw binary at ${stagedBinaryPath}`,
    )
    assert.ok(
      existsSync(migrationJournalPath),
      `Expected packaged migrations at ${migrationJournalPath}`,
    )
  }, 300_000)

  it('uses the Claw R2 upload prefix by default and allows env override', () => {
    const originalEnv = snapshotProcessEnv()
    try {
      setProcessEnv('R2_ACCOUNT_ID', 'test')
      setProcessEnv('R2_ACCESS_KEY_ID', 'test')
      setProcessEnv('R2_SECRET_ACCESS_KEY', 'test')
      setProcessEnv('R2_BUCKET', 'test')
      deleteProcessEnv('R2_UPLOAD_PREFIX')

      const defaultConfig = loadBuildConfig(rootDir, clawServerBuildProduct, {
        requireR2: true,
      })
      assert.strictEqual(
        defaultConfig.r2?.uploadPrefix,
        'claw-server/prod-resources',
      )

      setProcessEnv('R2_UPLOAD_PREFIX', 'custom/claw')
      const overrideConfig = loadBuildConfig(rootDir, clawServerBuildProduct, {
        requireR2: true,
      })
      assert.strictEqual(overrideConfig.r2?.uploadPrefix, 'custom/claw')
    } finally {
      restoreProcessEnv(originalEnv)
    }
  })
})

interface CollectableProcess {
  stdout: ReadableStream
  stderr: ReadableStream
  exited: Promise<number>
}

async function collectProcess(process: CollectableProcess) {
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(process.stdout).text(),
    new Response(process.stderr).text(),
    process.exited,
  ])

  return { stdout, stderr, exitCode }
}

function snapshotProcessEnv(): NodeJS.ProcessEnv {
  // biome-ignore lint/style/noProcessEnv: build smoke tests construct subprocess env snapshots directly
  return { ...process.env }
}

function setProcessEnv(key: string, value: string): void {
  // biome-ignore lint/style/noProcessEnv: loadBuildConfig intentionally reads process env
  process.env[key] = value
}

function deleteProcessEnv(key: string): void {
  // biome-ignore lint/style/noProcessEnv: loadBuildConfig intentionally reads process env
  delete process.env[key]
}

function restoreProcessEnv(env: NodeJS.ProcessEnv): void {
  process.env = env
}
