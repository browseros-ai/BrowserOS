import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { loadManifest } from './manifest'
import { stageCompiledArtifact } from './stage'
import type { BuildTarget } from './types'

const TARGET: BuildTarget = {
  id: 'darwin-arm64',
  name: 'macOS arm64',
  os: 'macos',
  arch: 'arm64',
  bunTarget: 'bun-darwin-arm64-modern',
  serverBinaryName: 'browseros-server-darwin-arm64',
}

describe('server artifact staging', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('loads local resource rules from the manifest', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
    const manifestPath = join(tempDir, 'manifest.json')
    await writeFile(
      manifestPath,
      JSON.stringify({
        resources: [
          {
            name: 'OpenClaw compose file',
            source: {
              type: 'local',
              path: 'apps/server/resources/openclaw-compose.yml',
            },
            destination: 'resources/openclaw-compose.yml',
          },
        ],
      }),
    )

    expect(loadManifest(manifestPath)).toEqual({
      resources: [
        {
          name: 'OpenClaw compose file',
          source: {
            type: 'local',
            path: 'apps/server/resources/openclaw-compose.yml',
          },
          destination: 'resources/openclaw-compose.yml',
          executable: false,
        },
      ],
    })
  })

  it('copies local resource files into the packaged artifact', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
    const distRoot = join(tempDir, 'dist')
    const compiledBinaryPath = join(tempDir, 'browseros-server')
    const sourceRoot = join(tempDir, 'repo')
    const composeSourcePath = join(
      sourceRoot,
      'apps/server/resources/openclaw-compose.yml',
    )
    await writeFile(compiledBinaryPath, '#!/bin/sh\n')
    await Bun.write(composeSourcePath, 'services:\n')

    const staged = await stageCompiledArtifact(
      distRoot,
      compiledBinaryPath,
      TARGET,
      '1.2.3',
      [
        {
          name: 'OpenClaw compose file',
          source: {
            type: 'local',
            path: 'apps/server/resources/openclaw-compose.yml',
          },
          destination: 'resources/openclaw-compose.yml',
        },
      ],
      sourceRoot,
    )

    expect(
      await readFile(
        join(staged.resourcesDir, 'openclaw-compose.yml'),
        'utf-8',
      ),
    ).toBe('services:\n')
  })
})
