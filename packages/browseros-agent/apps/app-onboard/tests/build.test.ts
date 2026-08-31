import { afterAll, describe, it } from 'bun:test'
import assert from 'node:assert'
import { existsSync, readFileSync, rmSync, statSync } from 'node:fs'
import { join, resolve } from 'node:path'

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
  'icon/keychain-prompt.png',
  'icon/hermes.png',
] as const

/** Exercises the complete Vite → staging → metadata → ZIP handoff used by CI. */
describe('app onboard resources archive', () => {
  const rootDir = resolve(import.meta.dir, '../../..')
  const versionPkgPath = resolve(rootDir, 'apps/app-onboard/package.json')
  const buildScript = resolve(rootDir, 'scripts/build/app-onboard.ts')
  const artifactRoot = resolve(rootDir, 'dist/prod/app-onboard/universal')
  const resourcesDir = resolve(artifactRoot, 'resources')
  const metadataPath = resolve(artifactRoot, 'artifact-metadata.json')
  const zipPath = resolve(
    rootDir,
    'dist/prod/app-onboard/browseros-app-onboard-resources.zip',
  )

  afterAll(() => {
    rmSync(resolve(rootDir, 'dist/prod/app-onboard'), {
      recursive: true,
      force: true,
    })
  })

  it('builds the universal Chromium archive without R2 credentials', async () => {
    rmSync(zipPath, { force: true })
    const pkg = await Bun.file(versionPkgPath).json()
    const build = await collectProcess(
      Bun.spawn(['bun', buildScript, '--no-upload'], {
        cwd: rootDir,
        stdout: 'pipe',
        stderr: 'pipe',
        env: buildEnv(),
      }),
    )

    assert.strictEqual(
      build.exitCode,
      0,
      `Onboard build failed:\n${build.stdout}\n${build.stderr}`,
    )
    assert.ok(existsSync(zipPath), `Expected archive at ${zipPath}`)

    const metadata = JSON.parse(readFileSync(metadataPath, 'utf-8'))
    assert.strictEqual(metadata.version, pkg.version)
    assert.strictEqual(metadata.target, 'universal')
    const metadataPaths: string[] = metadata.files.map(
      (entry: { path: string }) => entry.path,
    )

    for (const file of EXPECTED_RESOURCE_FILES) {
      const filePath = join(resourcesDir, file)
      assert.ok(existsSync(filePath), `Expected staged resource ${filePath}`)
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
    assert.match(zipListing.stdout, /resources\/icon\/hermes\.png/)
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

function buildEnv(): NodeJS.ProcessEnv {
  const env = { ...process.env }
  for (const key of R2_ENV_KEYS) {
    delete env[key]
  }
  return env
}
