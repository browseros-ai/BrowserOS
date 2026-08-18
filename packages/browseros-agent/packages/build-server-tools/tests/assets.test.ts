import { afterEach, describe, expect, it } from 'bun:test'
import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { S3Client } from '@aws-sdk/client-s3'

import {
  type AssetBuildProductDescriptor,
  archiveAssetArtifact,
  parseAssetBuildArgs,
  stageAssetArtifact,
  uploadAssetArchive,
} from '../src'
import { fakeR2Config } from './helpers'

function testAssetProduct(
  overrides: Partial<AssetBuildProductDescriptor> = {},
): AssetBuildProductDescriptor {
  return {
    label: 'Test onboarding',
    packageDir: 'apps/test-onboard',
    buildCommand: ['bun', 'run', 'build:chromium'],
    assetsDir: 'apps/test-onboard/dist/chromium',
    distRoot: 'dist/prod/test-onboard',
    archiveBaseName: 'test-onboard-resources',
    env: {
      requiredInlineEnvKeys: [],
      inlineEnvKeys: [],
      defaultR2UploadPrefix: 'test-onboard/prod-resources',
    },
    ...overrides,
  }
}

describe('asset artifact staging and archiving', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  async function writeAssetSource(root: string): Promise<void> {
    const assetsDir = join(root, 'apps/test-onboard/dist/chromium')
    await mkdir(join(assetsDir, 'icon'), { recursive: true })
    await writeFile(join(assetsDir, 'index.html'), '<html></html>')
    await writeFile(join(assetsDir, 'app.js'), 'console.log(1)')
    await writeFile(join(assetsDir, 'icon/32.png'), 'png-bytes')
  }

  it('stages the built assets under resources with universal metadata', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    await writeAssetSource(tempDir)
    const product = testAssetProduct({ distRoot: join(tempDir, 'dist') })

    const artifact = await stageAssetArtifact(product, '0.1.2-test', tempDir)

    expect(artifact.rootDir).toBe(join(tempDir, 'dist', 'universal'))
    expect(
      await readFile(join(artifact.resourcesDir, 'index.html'), 'utf8'),
    ).toBe('<html></html>')
    expect(
      await readFile(join(artifact.resourcesDir, 'icon/32.png'), 'utf8'),
    ).toBe('png-bytes')

    const metadata = JSON.parse(await readFile(artifact.metadataPath, 'utf8'))
    expect(metadata.version).toBe('0.1.2-test')
    expect(metadata.target).toBe('universal')
    const paths = metadata.files.map((entry: { path: string }) => entry.path)
    expect(paths).toContain('resources/index.html')
    expect(paths).toContain('resources/icon/32.png')
    for (const entry of metadata.files) {
      expect(entry.sha256).toMatch(/^[a-f0-9]{64}$/)
      expect(entry.size).toBeGreaterThanOrEqual(0)
    }
  })

  it('resolves a relative distRoot against the source root, not the cwd', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    await writeAssetSource(tempDir)
    const product = testAssetProduct()

    const artifact = await stageAssetArtifact(product, '0.0.0-test', tempDir)

    expect(artifact.rootDir).toBe(
      join(tempDir, 'dist/prod/test-onboard', 'universal'),
    )
    const zipPath = await archiveAssetArtifact(artifact, product)
    expect(zipPath).toBe(
      join(tempDir, 'dist/prod/test-onboard', 'test-onboard-resources.zip'),
    )
  })

  it('rejects a missing assets directory naming the expected path', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    const product = testAssetProduct({ distRoot: join(tempDir, 'dist') })

    await expect(
      stageAssetArtifact(product, '0.0.0-test', tempDir),
    ).rejects.toThrow('apps/test-onboard/dist/chromium')
  })

  it('rejects an empty assets directory', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    await mkdir(join(tempDir, 'apps/test-onboard/dist/chromium'), {
      recursive: true,
    })
    const product = testAssetProduct({ distRoot: join(tempDir, 'dist') })

    await expect(
      stageAssetArtifact(product, '0.0.0-test', tempDir),
    ).rejects.toThrow('empty')
  })

  it('archives without a target suffix and uploads latest plus version keys', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    await writeAssetSource(tempDir)
    const product = testAssetProduct({ distRoot: join(tempDir, 'dist') })
    const artifact = await stageAssetArtifact(product, '0.0.0-test', tempDir)

    const zipPath = await archiveAssetArtifact(artifact, product)

    expect(zipPath).toBe(join(tempDir, 'dist', 'test-onboard-resources.zip'))
    expect(existsSync(zipPath)).toBe(true)

    const uploadedKeys: string[] = []
    const client = {
      send: async (command: { input?: { Key?: string } }) => {
        if (command.constructor.name === 'GetObjectCommand') {
          throw Object.assign(new Error('missing'), {
            $metadata: { httpStatusCode: 404 },
          })
        }
        uploadedKeys.push(command.input?.Key ?? '')
        return {}
      },
    } as unknown as S3Client

    const result = await uploadAssetArchive(
      zipPath,
      '0.0.0-test',
      client,
      fakeR2Config,
    )

    expect(uploadedKeys).toEqual([
      'server/prod-resources/0.0.0-test/test-onboard-resources.zip',
      'server/prod-resources/latest/test-onboard-resources.zip',
    ])
    expect(result.versionR2Key).toBe(uploadedKeys[0])
    expect(result.latestR2Key).toBe(uploadedKeys[1])
  })

  it('uploads only the immutable versioned asset key when requested', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    await writeAssetSource(tempDir)
    const product = testAssetProduct({ distRoot: join(tempDir, 'dist') })
    const artifact = await stageAssetArtifact(product, '0.0.0-test', tempDir)
    const zipPath = await archiveAssetArtifact(artifact, product)
    const uploadedKeys: string[] = []
    const client = {
      send: async (command: { input?: { Key?: string } }) => {
        if (command.constructor.name === 'GetObjectCommand') {
          throw Object.assign(new Error('missing'), {
            $metadata: { httpStatusCode: 404 },
          })
        }
        uploadedKeys.push(command.input?.Key ?? '')
        return {}
      },
    } as unknown as S3Client

    const result = await uploadAssetArchive(
      zipPath,
      '0.0.0-test',
      client,
      fakeR2Config,
      { versionedOnly: true, releaseSha: 'source-sha' },
    )

    expect(uploadedKeys).toEqual([
      'server/prod-resources/0.0.0-test/test-onboard-resources.zip',
    ])
    expect(result.latestR2Key).toBeUndefined()
    expect(result.versionR2Key).toBe(uploadedKeys[0])
  })

  it('creates byte-identical archives for the same source and version', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    await writeAssetSource(tempDir)
    const product = testAssetProduct({ distRoot: join(tempDir, 'dist') })

    const firstArtifact = await stageAssetArtifact(
      product,
      '0.0.0-test',
      tempDir,
    )
    const zipPath = await archiveAssetArtifact(firstArtifact, product)
    const firstZip = await readFile(zipPath)
    const firstMetadata = await readFile(firstArtifact.metadataPath)

    const secondArtifact = await stageAssetArtifact(
      product,
      '0.0.0-test',
      tempDir,
    )
    await archiveAssetArtifact(secondArtifact, product)

    expect(await readFile(zipPath)).toEqual(firstZip)
    expect(await readFile(secondArtifact.metadataPath)).toEqual(firstMetadata)
  })

  it('recovers a canonical post-upload asset for draft attachment', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    const zipPath = join(tempDir, 'test-onboard-resources.zip')
    const canonical = Buffer.from('canonical uploaded zip')
    await writeFile(zipPath, 'different retry zip')
    const commands: string[] = []
    const client = {
      send: async (command: { constructor: { name: string } }) => {
        commands.push(command.constructor.name)
        return {
          Body: {
            transformToByteArray: async () => canonical,
          },
          Metadata: {
            component: 'server/prod-resources',
            'release-sha': 'source-sha',
            version: '0.0.0-test',
            target: 'universal',
            sha256: createHash('sha256').update(canonical).digest('hex'),
          },
        }
      },
    } as unknown as S3Client

    await uploadAssetArchive(zipPath, '0.0.0-test', client, fakeR2Config, {
      versionedOnly: true,
      releaseSha: 'source-sha',
    })

    expect(await readFile(zipPath)).toEqual(canonical)
    expect(commands).toEqual(['GetObjectCommand'])
  })

  it('rejects canonical recovery without matching source bindings', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    const zipPath = join(tempDir, 'test-onboard-resources.zip')
    await writeFile(zipPath, 'retry zip')
    const client = {
      send: async () => ({
        Body: {
          transformToByteArray: async () =>
            new TextEncoder().encode('existing zip'),
        },
        Metadata: {},
      }),
    } as unknown as S3Client

    await expect(
      uploadAssetArchive(zipPath, '0.0.0-test', client, fakeR2Config, {
        versionedOnly: true,
        releaseSha: 'source-sha',
      }),
    ).rejects.toThrow('immutable R2 object binding mismatch')
  })

  it('rejects a canonical object whose bytes do not match its checksum', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    const zipPath = join(tempDir, 'test-onboard-resources.zip')
    await writeFile(zipPath, 'retry zip')
    const client = {
      send: async () => ({
        Body: {
          transformToByteArray: async () =>
            new TextEncoder().encode('tampered existing zip'),
        },
        Metadata: {
          component: 'server/prod-resources',
          'release-sha': 'source-sha',
          version: '0.0.0-test',
          target: 'universal',
          sha256: '0'.repeat(64),
        },
      }),
    } as unknown as S3Client

    await expect(
      uploadAssetArchive(zipPath, '0.0.0-test', client, fakeR2Config, {
        versionedOnly: true,
        releaseSha: 'source-sha',
      }),
    ).rejects.toThrow('immutable R2 object binding mismatch')
    expect(await readFile(zipPath, 'utf8')).toBe('retry zip')
  })

  it('requires a source binding for versioned-only uploads', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'build-server-tools-assets-'))
    const zipPath = join(tempDir, 'test-onboard-resources.zip')
    await writeFile(zipPath, 'zip')

    await expect(
      uploadAssetArchive(zipPath, '0.0.0-test', {} as S3Client, fakeR2Config, {
        versionedOnly: true,
      }),
    ).rejects.toThrow('releaseSha is required')
  })
})

describe('asset build arg parsing', () => {
  const product = testAssetProduct()

  it('defaults to uploading outside CI', () => {
    expect(parseAssetBuildArgs([], product)).toEqual({
      upload: true,
      ci: false,
      versionedOnly: false,
    })
  })

  it('honors --no-upload', () => {
    expect(parseAssetBuildArgs(['--no-upload'], product)).toEqual({
      upload: false,
      ci: false,
      versionedOnly: false,
    })
  })

  it('forces upload off in CI mode', () => {
    expect(parseAssetBuildArgs(['--ci'], product)).toEqual({
      upload: false,
      ci: true,
      versionedOnly: false,
    })
  })

  it('rejects --ci combined with --upload', () => {
    expect(() => parseAssetBuildArgs(['--ci', '--upload'], product)).toThrow(
      '--ci cannot be combined with --upload',
    )
  })

  it('rejects unknown options such as --target', () => {
    expect(() =>
      parseAssetBuildArgs(['--target=darwin-arm64'], product),
    ).toThrow()
  })

  it('respects a defaultUpload of false', () => {
    const noUploadProduct = testAssetProduct({ defaultUpload: false })
    expect(parseAssetBuildArgs([], noUploadProduct)).toEqual({
      upload: false,
      ci: false,
      versionedOnly: false,
    })
  })

  it('supports versioned-only asset uploads', () => {
    expect(parseAssetBuildArgs(['--versioned-only'], product)).toEqual({
      upload: true,
      ci: false,
      versionedOnly: true,
    })
  })
})
