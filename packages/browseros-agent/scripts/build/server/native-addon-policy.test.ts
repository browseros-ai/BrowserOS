import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { SERVER_BUNDLE_ENTRYPOINT } from './compile'

const nativeAddonGuardPath = join(
  process.cwd(),
  'apps/server/src/lib/native-addon-guard.ts',
)

describe('compiled server native addon policy', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('bundles the compiled bootstrap entrypoint', () => {
    expect(SERVER_BUNDLE_ENTRYPOINT).toBe(
      'apps/server/src/compiled-bootstrap.ts',
    )
  })

  it('installs the native-addon guard idempotently', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-native-addon-policy-'))
    const sourcePath = join(tempDir, 'idempotent.ts')
    await writeFile(
      sourcePath,
      [
        `import { installNativeAddonGuard } from ${JSON.stringify(nativeAddonGuardPath)}`,
        'installNativeAddonGuard()',
        'const guarded = process.dlopen',
        'installNativeAddonGuard()',
        'console.log(String(process.dlopen === guarded))',
      ].join('\n'),
    )

    const result = await collectProcess(
      Bun.spawn(['bun', sourcePath], {
        stdout: 'pipe',
        stderr: 'pipe',
      }),
    )

    expect(result).toMatchObject({ exitCode: 0, stdout: 'true\n' })
  })

  it('prevents Bun from opening hidden temp native addons', async () => {
    if (process.platform !== 'darwin') return

    tempDir = await mkdtemp(join(tmpdir(), 'browseros-native-addon-policy-'))
    const sourcePath = join(tempDir, 'app.js')
    const addonPath = join(tempDir, 'addon.node')
    const binaryPath = join(tempDir, 'app')
    const runTmpDir = join(tempDir, 'tmp')

    await writeFile(addonPath, 'not a native addon')
    await writeFile(
      sourcePath,
      [
        `import { installNativeAddonGuard } from ${JSON.stringify(nativeAddonGuardPath)}`,
        'installNativeAddonGuard()',
        'try {',
        '  require("./addon.node")',
        '} catch (error) {',
        '  console.error(error?.message ?? String(error))',
        '  setInterval(() => {}, 1000)',
        '}',
      ].join('\n'),
    )

    const build = Bun.spawn(
      ['bun', 'build', '--compile', sourcePath, '--outfile', binaryPath],
      {
        stdout: 'pipe',
        stderr: 'pipe',
      },
    )
    const buildResult = await collectProcess(build)
    expect(buildResult).toMatchObject({ exitCode: 0 })

    await rm(sourcePath)
    await rm(addonPath)
    await mkdir(runTmpDir)

    const app = Bun.spawn([binaryPath], {
      env: {
        ...process.env,
        BUN_TMPDIR: runTmpDir,
        TMPDIR: runTmpDir,
      },
      stdout: 'pipe',
      stderr: 'pipe',
    })
    await Bun.sleep(500)

    const openFiles = await collectProcess(
      Bun.spawn(['lsof', '-p', String(app.pid)], {
        stdout: 'pipe',
        stderr: 'pipe',
      }),
    )

    app.kill()
    const appResult = await collectProcess(app)

    expect(appResult.stderr).toContain(
      'BrowserOS server disables native addon loading in compiled production builds',
    )
    expect(await listFiles(runTmpDir)).toEqual([])
    expect(openFiles.stdout).not.toContain('.node')
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

async function listFiles(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { recursive: true })
  return entries.map(String).sort()
}
