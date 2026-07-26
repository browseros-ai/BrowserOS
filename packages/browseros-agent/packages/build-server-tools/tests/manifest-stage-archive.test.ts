import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { S3Client } from '@aws-sdk/client-s3'

import {
  archiveAndUploadArtifacts,
  archiveArtifacts,
  getTargetRules,
  loadManifest,
  type ResourceRule,
  stageCompiledArtifact,
  stagedBinaryName,
  stageTargetArtifact,
} from '../src'
import { fakeR2Config, testProduct, testTarget } from './helpers'

describe('resource manifest and artifact staging', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('parses local and R2 resource rules and treats missing filters as all-target', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const manifestPath = join(tempDir, 'manifest.json')
    await writeFile(
      manifestPath,
      JSON.stringify({
        resources: [
          {
            name: 'Migrations',
            source: { type: 'local', path: 'apps/test/drizzle' },
            destination: 'resources/db/migrations',
            recursive: true,
          },
          {
            name: 'Linux tool',
            source: { type: 'r2', key: 'tool-linux-x64' },
            destination: 'resources/bin/tool',
            os: ['linux'],
            arch: ['x64'],
            executable: true,
          },
        ],
      }),
    )

    const manifest = loadManifest(manifestPath)

    expect(getTargetRules(manifest, testTarget('darwin-arm64'))).toEqual([
      expect.objectContaining({ name: 'Migrations' }),
    ])
    expect(getTargetRules(manifest, testTarget('linux-x64'))).toHaveLength(2)
  })

  it('rejects incomplete and invalid manifest rules', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const missingDestinationPath = join(tempDir, 'missing-destination.json')
    const invalidOsPath = join(tempDir, 'invalid-os.json')
    await writeFile(
      missingDestinationPath,
      JSON.stringify({
        resources: [
          {
            name: 'Bad rule',
            source: { type: 'local', path: 'apps/test/drizzle' },
          },
        ],
      }),
    )
    await writeFile(
      invalidOsPath,
      JSON.stringify({
        resources: [
          {
            name: 'Bad OS',
            source: { type: 'r2', key: 'tool' },
            destination: 'resources/bin/tool',
            os: ['plan9'],
          },
        ],
      }),
    )

    expect(() => loadManifest(missingDestinationPath)).toThrow(
      'missing source path or destination',
    )
    expect(() => loadManifest(invalidOsPath)).toThrow('invalid os value')
  })

  it('stages the product binary with executable mode for non-Windows targets', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const distRoot = join(tempDir, 'dist')
    const binaryPath = join(tempDir, 'compiled')
    const product = testProduct({
      distRoot,
      stagedBinaryBaseName: 'browseros-claw-server',
    })
    const target = testTarget('darwin-arm64')
    await writeFile(binaryPath, 'server')

    const artifact = await stageCompiledArtifact(
      product,
      binaryPath,
      target,
      '0.0.0-test',
    )

    const stagedPath = join(artifact.resourcesDir, 'bin/browseros-claw-server')
    expect(stagedBinaryName(product, target)).toBe('browseros-claw-server')
    expect(await readFile(stagedPath, 'utf8')).toBe('server')
    expect((await stat(stagedPath)).mode & 0o111).not.toBe(0)
  })

  it('adds .exe to staged Windows binaries', async () => {
    const product = testProduct({
      stagedBinaryBaseName: 'browseros-claw-server',
    })

    expect(stagedBinaryName(product, testTarget('windows-x64'))).toBe(
      'browseros-claw-server.exe',
    )
  })

  it('copies recursive local resources without allowing destination escape', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const sourceRoot = join(tempDir, 'source')
    const distRoot = join(tempDir, 'dist')
    const binaryPath = join(tempDir, 'compiled')
    const migrationsDir = join(sourceRoot, 'apps/test/drizzle')
    const product = testProduct({ distRoot })
    await mkdir(join(migrationsDir, 'meta'), { recursive: true })
    await writeFile(binaryPath, 'server')
    await writeFile(join(migrationsDir, '0000_init.sql'), 'CREATE TABLE x;')
    await writeFile(
      join(migrationsDir, 'meta', '_journal.json'),
      '{"entries":[]}',
    )

    const artifact = await stageCompiledArtifact(
      product,
      binaryPath,
      testTarget(),
      '0.0.0-test',
      [migrationRule],
      sourceRoot,
    )

    expect(
      await readFile(
        join(artifact.resourcesDir, 'db/migrations/0000_init.sql'),
        'utf8',
      ),
    ).toBe('CREATE TABLE x;')
    await expect(
      stageCompiledArtifact(
        product,
        binaryPath,
        testTarget(),
        '0.0.0-test',
        [{ ...migrationRule, destination: '../escape' }],
        sourceRoot,
      ),
    ).rejects.toThrow('outside artifact root')
  })

  it('downloads R2 resources with the configured prefix and executable bit', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const sourceRoot = join(tempDir, 'source')
    const distRoot = join(tempDir, 'dist')
    const binaryPath = join(tempDir, 'compiled')
    const product = testProduct({ distRoot })
    const requests: string[] = []
    await writeFile(binaryPath, 'server')

    const artifact = await stageTargetArtifact(
      product,
      binaryPath,
      testTarget(),
      [bunRule],
      sourceRoot,
      {
        send: async (command: { input?: { Key?: string } }) => {
          requests.push(command.input?.Key ?? '')
          return {
            Body: {
              transformToByteArray: async () =>
                new TextEncoder().encode('#!/bin/sh\n'),
            },
          }
        },
      } as unknown as S3Client,
      fakeR2Config,
      '0.0.0-test',
    )

    const bunPath = join(artifact.resourcesDir, 'bin/third_party/bun')
    expect(requests).toEqual([
      'artifacts/vendor/third_party/bun/bun-darwin-arm64',
    ])
    expect(await readFile(bunPath, 'utf8')).toBe('#!/bin/sh\n')
    expect((await stat(bunPath)).mode & 0o111).not.toBe(0)
  })

  it('archives with the descriptor archive base name and uploads latest plus version keys', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const distRoot = join(tempDir, 'dist')
    const binaryPath = join(tempDir, 'compiled')
    const product = testProduct({
      distRoot,
      archiveBaseName: 'browseros-claw-server-resources',
    })
    const uploadedKeys: string[] = []
    await writeFile(binaryPath, 'server')
    const artifact = await stageCompiledArtifact(
      product,
      binaryPath,
      testTarget(),
      '0.0.0-test',
    )

    const archiveResults = await archiveArtifacts(
      [artifact],
      product.archiveBaseName,
    )
    expect(archiveResults[0]?.zipPath).toBe(
      join(distRoot, 'browseros-claw-server-resources-darwin-arm64.zip'),
    )

    const uploadResults = await archiveAndUploadArtifacts(
      [artifact],
      '0.0.0-test',
      {
        send: async (command: { input?: { Key?: string } }) => {
          uploadedKeys.push(command.input?.Key ?? '')
          return {}
        },
      } as unknown as S3Client,
      fakeR2Config,
      true,
      product.archiveBaseName,
    )

    expect(uploadedKeys).toEqual([
      'server/prod-resources/latest/browseros-claw-server-resources-darwin-arm64.zip',
      'server/prod-resources/0.0.0-test/browseros-claw-server-resources-darwin-arm64.zip',
    ])
    expect(uploadResults[0]?.latestR2Key).toBe(uploadedKeys[0])
    expect(uploadResults[0]?.versionR2Key).toBe(uploadedKeys[1])
  })
})

const migrationRule: ResourceRule = {
  name: 'Drizzle migrations',
  source: {
    type: 'local',
    path: 'apps/test/drizzle',
  },
  destination: 'resources/db/migrations',
  recursive: true,
}

const bunRule: ResourceRule = {
  name: 'Bun - macOS ARM64',
  source: {
    type: 'r2',
    key: 'third_party/bun/bun-darwin-arm64',
  },
  destination: 'resources/bin/third_party/bun',
  os: ['macos'],
  arch: ['arm64'],
  executable: true,
}
