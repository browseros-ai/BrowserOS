import { afterEach, describe, expect, it } from 'bun:test'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
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
  recoverVersionedTargets,
  stageCompiledArtifact,
  stagedBinaryName,
  stageTargetArtifact,
  uploadImmutableFileToObject,
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

  it('writes opt-in release identity into artifact metadata', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const binaryPath = join(tempDir, 'compiled')
    const product = testProduct({ distRoot: join(tempDir, 'dist') })
    const releaseSha = 'a'.repeat(40)
    await writeFile(binaryPath, 'server')

    const artifact = await stageCompiledArtifact(
      product,
      binaryPath,
      testTarget(),
      '1.2.3',
      [],
      tempDir,
      {
        component: 'claw-server-rust/prod-resources',
        releaseSha,
      },
    )

    const metadata = JSON.parse(await readFile(artifact.metadataPath, 'utf8'))
    expect(metadata).toMatchObject({
      component: 'claw-server-rust/prod-resources',
      version: '1.2.3',
      target: 'darwin-arm64',
      releaseSha,
    })
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
          if (command.constructor.name === 'GetObjectCommand') {
            throw Object.assign(new Error('missing'), {
              $metadata: { httpStatusCode: 404 },
            })
          }
          uploadedKeys.push(command.input?.Key ?? '')
          return {}
        },
      } as unknown as S3Client,
      fakeR2Config,
      true,
      product.archiveBaseName,
    )

    expect(uploadedKeys).toEqual([
      'server/prod-resources/0.0.0-test/browseros-claw-server-resources-darwin-arm64.zip',
      'server/prod-resources/latest/browseros-claw-server-resources-darwin-arm64.zip',
    ])
    expect(uploadResults[0]?.versionR2Key).toBe(uploadedKeys[0])
    expect(uploadResults[0]?.latestR2Key).toBe(uploadedKeys[1])
  })

  it('can archive only files for workflow-compatible member lists', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const binaryPath = join(tempDir, 'compiled')
    const product = testProduct({ distRoot: join(tempDir, 'dist') })
    await writeFile(binaryPath, 'server')
    const artifact = await stageCompiledArtifact(
      product,
      binaryPath,
      testTarget(),
      '0.0.0-test',
    )

    const [result] = await archiveArtifacts(
      [artifact],
      product.archiveBaseName,
      { filesOnly: true },
    )
    if (!result) throw new Error('Missing archive result')
    const members = execFileSync('unzip', ['-Z1', result.zipPath], {
      encoding: 'utf8',
    })
      .trim()
      .split('\n')

    expect(members).toEqual([
      'artifact-metadata.json',
      'resources/bin/test_server',
    ])
  })

  it('does not overwrite a versioned object with different bytes', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const filePath = join(tempDir, 'artifact.zip')
    await writeFile(filePath, 'new bytes')
    const client = {
      send: async (command: { input?: { Key?: string } }) => {
        if (command.constructor.name === 'GetObjectCommand') {
          return {
            Body: {
              transformToByteArray: async () =>
                new TextEncoder().encode('different bytes'),
            },
          }
        }
        throw new Error(`Unexpected write to ${command.input?.Key ?? ''}`)
      },
    } as unknown as S3Client

    await expect(
      uploadImmutableFileToObject(
        client,
        fakeR2Config,
        'server/prod-resources/0.0.0-test/artifact.zip',
        filePath,
      ),
    ).rejects.toThrow('immutable R2 object already exists with different bytes')
  })

  it('reuses an immutable versioned object with identical bytes', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const filePath = join(tempDir, 'artifact.zip')
    await writeFile(filePath, 'same bytes')
    const commands: string[] = []
    const client = {
      send: async (command: { constructor: { name: string } }) => {
        commands.push(command.constructor.name)
        return {
          Body: {
            transformToByteArray: async () =>
              new TextEncoder().encode('same bytes'),
          },
        }
      },
    } as unknown as S3Client

    const result = await uploadImmutableFileToObject(
      client,
      fakeR2Config,
      'server/prod-resources/0.0.0-test/artifact.zip',
      filePath,
    )

    expect(result).toBe('reused')
    expect(commands).toEqual(['GetObjectCommand'])
  })

  it('recovers the canonical object after conditional upload races', async () => {
    for (const status of [409, 412]) {
      tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
      const filePath = join(tempDir, `artifact-${status}.zip`)
      await writeFile(filePath, 'our build')
      const canonical = Buffer.from(`canonical ${status}`)
      let gets = 0
      const client = {
        send: async (command: { constructor: { name: string } }) => {
          if (command.constructor.name === 'GetObjectCommand') {
            gets += 1
            if (gets === 1) {
              throw Object.assign(new Error('missing'), {
                $metadata: { httpStatusCode: 404 },
              })
            }
            return {
              Body: { transformToByteArray: async () => canonical },
              Metadata: {
                component: 'server/prod-resources',
                'release-sha': 'source-sha',
                version: '0.0.0-test',
                target: 'linux-x64',
                sha256: createHash('sha256').update(canonical).digest('hex'),
              },
            }
          }
          throw Object.assign(new Error('conditional race'), {
            $metadata: { httpStatusCode: status },
          })
        },
      } as unknown as S3Client

      const result = await uploadImmutableFileToObject(
        client,
        fakeR2Config,
        'server/prod-resources/0.0.0-test/artifact.zip',
        filePath,
        {
          identity: {
            component: 'server/prod-resources',
            releaseSha: 'source-sha',
            target: 'linux-x64',
            version: '0.0.0-test',
          },
        },
      )

      expect(result).toBe('recovered')
      expect(await readFile(filePath)).toEqual(canonical)
    }
  })

  it('recovers uploaded targets and uploads only missing targets', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const firstRoot = join(tempDir, 'first')
    const secondRoot = join(tempDir, 'second')
    await mkdir(firstRoot, { recursive: true })
    await mkdir(secondRoot, { recursive: true })
    await writeFile(join(firstRoot, 'server'), 'first retry build')
    await writeFile(join(secondRoot, 'server'), 'second build')
    const canonical = Buffer.from('first canonical upload')
    const putKeys: string[] = []
    const client = {
      send: async (command: {
        constructor: { name: string }
        input?: { Key?: string; Metadata?: Record<string, string> }
      }) => {
        const key = command.input?.Key ?? ''
        if (command.constructor.name === 'GetObjectCommand') {
          if (key.includes('darwin-arm64')) {
            return {
              Body: { transformToByteArray: async () => canonical },
              Metadata: {
                component: 'server/prod-resources',
                'release-sha': 'source-sha',
                version: '0.0.0-test',
                target: 'darwin-arm64',
                sha256: createHash('sha256').update(canonical).digest('hex'),
              },
            }
          }
          throw Object.assign(new Error('missing'), {
            $metadata: { httpStatusCode: 404 },
          })
        }
        putKeys.push(key)
        expect(command.input?.Metadata).toMatchObject({
          component: 'server/prod-resources',
          'release-sha': 'source-sha',
          version: '0.0.0-test',
          target: 'linux-x64',
        })
        expect(command.input?.Metadata?.sha256).toMatch(/^[0-9a-f]{64}$/)
        return {}
      },
    } as unknown as S3Client

    const results = await archiveAndUploadArtifacts(
      [
        {
          target: testTarget('darwin-arm64'),
          rootDir: firstRoot,
          resourcesDir: firstRoot,
          metadataPath: join(firstRoot, 'metadata.json'),
        },
        {
          target: testTarget('linux-x64'),
          rootDir: secondRoot,
          resourcesDir: secondRoot,
          metadataPath: join(secondRoot, 'metadata.json'),
        },
      ],
      '0.0.0-test',
      client,
      fakeR2Config,
      true,
      'artifact',
      { versionedOnly: true, releaseSha: 'source-sha' },
    )

    expect(putKeys).toEqual([
      'server/prod-resources/0.0.0-test/artifact-linux-x64.zip',
    ])
    const recoveredResult = results[0]
    if (!recoveredResult) throw new Error('Missing recovered target result')
    expect(await readFile(recoveredResult.zipPath)).toEqual(canonical)
  })

  it('skips builds for targets already recovered from immutable R2', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-'))
    const product = testProduct({ distRoot: join(tempDir, 'dist') })
    const canonical = Buffer.from('canonical darwin artifact')
    const client = {
      send: async (command: {
        constructor: { name: string }
        input?: { Key?: string }
      }) => {
        const key = command.input?.Key ?? ''
        if (command.constructor.name !== 'GetObjectCommand') {
          throw new Error(`Unexpected command: ${command.constructor.name}`)
        }
        if (!key.includes('darwin-arm64')) {
          throw Object.assign(new Error('missing'), {
            $metadata: { httpStatusCode: 404 },
          })
        }
        return {
          Body: { transformToByteArray: async () => canonical },
          Metadata: {
            component: 'server/prod-resources',
            'release-sha': 'source-sha',
            version: '0.0.0-test',
            target: 'darwin-arm64',
            sha256: createHash('sha256').update(canonical).digest('hex'),
          },
        }
      },
    } as unknown as S3Client

    const recovery = await recoverVersionedTargets(
      product,
      [testTarget('darwin-arm64'), testTarget('linux-x64')],
      '0.0.0-test',
      'source-sha',
      client,
      fakeR2Config,
    )

    expect(recovery.targetsToBuild.map((target) => target.id)).toEqual([
      'linux-x64',
    ])
    expect(recovery.recoveredResults.map((result) => result.targetId)).toEqual([
      'darwin-arm64',
    ])
    const recovered = recovery.recoveredResults[0]
    if (!recovered) throw new Error('Missing recovered target')
    expect(await readFile(recovered.zipPath)).toEqual(canonical)
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
