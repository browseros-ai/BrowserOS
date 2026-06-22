import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { COMPILED_SERVER_EXEC_ARGV } from './compile'

describe('compiled server native addon policy', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('bakes native-addon loading disablement into compiled server binaries', () => {
    expect(COMPILED_SERVER_EXEC_ARGV).toContain('--no-addons')
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
        'try {',
        '  require("./addon.node")',
        '} catch (error) {',
        '  console.error(error?.message ?? String(error))',
        '  setInterval(() => {}, 1000)',
        '}',
      ].join('\n'),
    )

    const build = Bun.spawn(
      [
        'bun',
        'build',
        '--compile',
        `--compile-exec-argv=${COMPILED_SERVER_EXEC_ARGV.join(' ')}`,
        sourcePath,
        '--outfile',
        binaryPath,
      ],
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
      'Cannot load native addon because loading addons is disabled',
    )
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
