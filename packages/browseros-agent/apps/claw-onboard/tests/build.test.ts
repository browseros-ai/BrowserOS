import { afterAll, describe, it } from 'bun:test'
import assert from 'node:assert'
import { existsSync, readFileSync, rmSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'
import {
  loadBuildConfig,
  parseAssetBuildArgs,
} from '../../../packages/build-server-tools/src'
import { clawOnboardBuildProduct } from '../../../scripts/build/claw-onboard/descriptor'

const R2_ENV_KEYS = [
  'R2_ACCOUNT_ID',
  'R2_ACCESS_KEY_ID',
  'R2_SECRET_ACCESS_KEY',
  'R2_BUCKET',
  'R2_UPLOAD_PREFIX',
  'R2_DOWNLOAD_PREFIX',
] as const

const EXPECTED_RESOURCE_FILES = [
  'index.html',
  'app.js',
  'app.css',
  'icon/16.png',
  'icon/32.png',
  'icon/48.png',
  'icon/96.png',
  'icon/128.png',
] as const

describe('claw onboard resources build', () => {
  const rootDir = resolve(import.meta.dir, '../../..')
  const versionPkgPath = resolve(rootDir, 'apps/claw-onboard/package.json')
  const buildScript = resolve(rootDir, 'scripts/build/claw-onboard.ts')
  const artifactRoot = resolve(rootDir, 'dist/prod/claw-onboard/universal')
  const resourcesDir = resolve(artifactRoot, 'resources')
  const metadataPath = resolve(artifactRoot, 'artifact-metadata.json')
  const zipPath = resolve(
    rootDir,
    'dist/prod/claw-onboard/browseros-claw-onboard-resources.zip',
  )

  function buildEnv(): NodeJS.ProcessEnv {
    const env = snapshotProcessEnv()
    for (const key of R2_ENV_KEYS) {
      delete env[key]
    }
    return env
  }

  afterAll(() => {
    rmSync(resolve(rootDir, 'dist/prod/claw-onboard'), {
      recursive: true,
      force: true,
    })
  })

  it('resolves the build version from the onboard package', async () => {
    const pkg = await Bun.file(versionPkgPath).json()
    const config = loadBuildConfig(rootDir, clawOnboardBuildProduct, {
      ci: true,
    })
    assert.strictEqual(config.version, pkg.version)
  })

  it('uploads by default while preserving local-only modes', () => {
    assert.strictEqual(
      parseAssetBuildArgs([], clawOnboardBuildProduct).upload,
      true,
    )
    assert.strictEqual(
      parseAssetBuildArgs(['--no-upload'], clawOnboardBuildProduct).upload,
      false,
    )
    assert.strictEqual(
      parseAssetBuildArgs(['--ci'], clawOnboardBuildProduct).upload,
      false,
    )
  })

  it('uses the claw-onboard R2 upload prefix by default and allows env override', () => {
    const originalEnv = snapshotProcessEnv()
    try {
      setProcessEnv('R2_ACCOUNT_ID', 'test')
      setProcessEnv('R2_ACCESS_KEY_ID', 'test')
      setProcessEnv('R2_SECRET_ACCESS_KEY', 'test')
      setProcessEnv('R2_BUCKET', 'test')
      deleteProcessEnv('R2_UPLOAD_PREFIX')

      const defaultConfig = loadBuildConfig(rootDir, clawOnboardBuildProduct, {
        requireR2: true,
      })
      assert.strictEqual(
        defaultConfig.r2?.uploadPrefix,
        'claw-onboard/prod-resources',
      )

      setProcessEnv('R2_UPLOAD_PREFIX', 'custom/onboard')
      const overrideConfig = loadBuildConfig(rootDir, clawOnboardBuildProduct, {
        requireR2: true,
      })
      assert.strictEqual(overrideConfig.r2?.uploadPrefix, 'custom/onboard')
    } finally {
      restoreProcessEnv(originalEnv)
    }
  })

  it('builds the universal resources artifact without R2 credentials', async () => {
    rmSync(zipPath, { force: true })
    const pkg = await Bun.file(versionPkgPath).json()
    const expectedVersion: string = pkg.version

    const build = Bun.spawn(['bun', buildScript, '--no-upload'], {
      cwd: rootDir,
      stdout: 'pipe',
      stderr: 'pipe',
      env: buildEnv(),
    })
    const buildExit = await build.exited
    if (buildExit !== 0) {
      const stderr = await new Response(build.stderr).text()
      assert.fail(`Onboard build failed (exit ${buildExit}):\n${stderr}`)
    }

    for (const file of EXPECTED_RESOURCE_FILES) {
      const filePath = join(resourcesDir, file)
      assert.ok(existsSync(filePath), `Expected staged resource ${filePath}`)
    }
    assert.ok(existsSync(zipPath), `Expected archive at ${zipPath}`)

    const metadata = JSON.parse(readFileSync(metadataPath, 'utf-8'))
    assert.strictEqual(metadata.version, expectedVersion)
    assert.strictEqual(metadata.target, 'universal')
    const metadataPaths: string[] = metadata.files.map(
      (entry: { path: string }) => entry.path,
    )
    for (const file of EXPECTED_RESOURCE_FILES) {
      assert.ok(
        metadataPaths.includes(`resources/${file}`),
        `Expected metadata entry for resources/${file}`,
      )
    }
    for (const entry of metadata.files) {
      assert.match(entry.sha256, /^[a-f0-9]{64}$/)
      assert.strictEqual(
        entry.size,
        statSync(join(artifactRoot, entry.path)).size,
      )
    }

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
    assert.match(zipListing.stdout, /resources\/index\.html/)
    assert.match(zipListing.stdout, /artifact-metadata\.json/)
  }, 300_000)
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
  return { ...process.env }
}

function setProcessEnv(key: string, value: string): void {
  process.env[key] = value
}

function deleteProcessEnv(key: string): void {
  delete process.env[key]
}

function restoreProcessEnv(env: NodeJS.ProcessEnv): void {
  process.env = env
}
