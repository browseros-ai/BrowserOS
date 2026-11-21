import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { resolveTargets } from '@browseros/build-server-tools'

import { browserosServerBuildProduct } from './descriptor'
import { prebuiltServerCompiler } from './prebuilt'

describe('prebuilt BrowserOS server binaries', () => {
  let dir: string | null = null

  afterEach(async () => {
    if (dir) {
      await rm(dir, { recursive: true, force: true })
      dir = null
    }
  })

  async function prebuiltDir(targetIds: string[]): Promise<string> {
    dir = await mkdtemp(join(tmpdir(), 'browseros-server-prebuilt-'))
    for (const id of targetIds) {
      const name =
        id === 'windows-x64' ? 'browseros_server.exe' : 'browseros_server'
      await mkdir(join(dir, id), { recursive: true })
      await writeFile(join(dir, id, name), 'binary')
    }
    return dir
  }

  it('hands the packager the release layout for every shipped target', async () => {
    const targets = resolveTargets('all')
    const root = await prebuiltDir(targets.map((target) => target.id))

    const compiled = await prebuiltServerCompiler(root)(
      browserosServerBuildProduct,
      targets,
      {},
      {},
      '0.1.0',
    )

    expect(
      compiled.map(({ target, binaryPath }) => [target.id, binaryPath]),
    ).toEqual([
      ['linux-x64', join(root, 'linux-x64/browseros_server')],
      ['linux-arm64', join(root, 'linux-arm64/browseros_server')],
      ['windows-x64', join(root, 'windows-x64/browseros_server.exe')],
      ['darwin-arm64', join(root, 'darwin-arm64/browseros_server')],
      ['darwin-x64', join(root, 'darwin-x64/browseros_server')],
    ])
  })

  it('names every requested target whose binary is missing', async () => {
    const root = await prebuiltDir(['linux-x64', 'darwin-arm64'])

    const error = await prebuiltServerCompiler(root)(
      browserosServerBuildProduct,
      resolveTargets('all'),
      {},
      {},
      '0.1.0',
    ).catch((caught) => caught as Error)

    expect(error.message).toContain('missing prebuilt binaries')
    expect(error.message).toContain(
      `windows-x64: ${join(root, 'windows-x64/browseros_server.exe')}`,
    )
    expect(error.message).toContain('linux-arm64')
    expect(error.message).toContain('darwin-x64')
    expect(error.message).not.toContain('linux-x64:')
  })

  it('serves a subset when only some targets are requested', async () => {
    const root = await prebuiltDir(['linux-arm64'])

    const compiled = await prebuiltServerCompiler(root)(
      browserosServerBuildProduct,
      resolveTargets('linux-arm64'),
      {},
      {},
      '0.1.0',
    )

    expect(compiled.map(({ target }) => target.id)).toEqual(['linux-arm64'])
  })
})
