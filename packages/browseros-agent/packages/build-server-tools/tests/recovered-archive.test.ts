import { afterEach, describe, expect, it } from 'bun:test'
import { createHash } from 'node:crypto'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { S3Client } from '@aws-sdk/client-s3'

import {
  archiveArtifacts,
  type ResourceRule,
  recoverVersionedTargets,
  stageCompiledArtifact,
} from '../src'
import { fakeR2Config, testProduct, testTarget } from './helpers'

const VERSION = '1.2.3'
const RELEASE_SHA = 'a'.repeat(40)

describe('recovered artifact validation', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('accepts a recovered archive with exact members, identity, and checksums', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'recovered-artifact-'))
    const { product, bytes } = await createArchive(tempDir, {
      includeSkill: true,
    })

    const recovery = await recoverVersionedTargets(
      product,
      [testTarget()],
      VERSION,
      RELEASE_SHA,
      artifactClient(bytes),
      fakeR2Config,
    )

    expect(recovery.targetsToBuild).toEqual([])
    expect(recovery.recoveredResults.map((result) => result.targetId)).toEqual([
      'darwin-arm64',
    ])
  })

  it('rejects a recovered archive with a missing payload member', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'recovered-artifact-'))
    const { product, bytes } = await createArchive(tempDir)

    await expect(
      recoverVersionedTargets(
        product,
        [testTarget()],
        VERSION,
        RELEASE_SHA,
        artifactClient(bytes),
        fakeR2Config,
      ),
    ).rejects.toThrow('unexpected archive members')
  })

  it('rejects a recovered archive whose payload fails metadata integrity', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'recovered-artifact-'))
    const { product, bytes } = await createArchive(tempDir, {
      includeSkill: true,
      corruptChecksum: true,
    })

    await expect(
      recoverVersionedTargets(
        product,
        [testTarget()],
        VERSION,
        RELEASE_SHA,
        artifactClient(bytes),
        fakeR2Config,
      ),
    ).rejects.toThrow('failed integrity validation')
  })
})

async function createArchive(
  rootDir: string,
  options: { includeSkill?: boolean; corruptChecksum?: boolean } = {},
) {
  const distRoot = join(rootDir, 'dist')
  const sourceRoot = join(rootDir, 'source')
  const binaryPath = join(rootDir, 'compiled')
  const product = testProduct({
    distRoot,
    archiveFilesOnly: true,
    expectedArtifactFiles: () => [
      'resources/bin/test_server',
      'resources/skills/browserclaw/SKILL.md',
    ],
  })
  await writeFile(binaryPath, 'server')
  await mkdir(sourceRoot, { recursive: true })
  const rules: ResourceRule[] = []
  if (options.includeSkill) {
    await writeFile(join(sourceRoot, 'SKILL.md'), 'skill')
    rules.push({
      name: 'BrowserClaw skill',
      source: { type: 'local', path: 'SKILL.md' },
      destination: 'resources/skills/browserclaw/SKILL.md',
      executable: false,
      recursive: false,
    })
  }
  const artifact = await stageCompiledArtifact(
    product,
    binaryPath,
    testTarget(),
    VERSION,
    rules,
    sourceRoot,
    {
      component: fakeR2Config.uploadPrefix,
      releaseSha: RELEASE_SHA,
    },
  )
  if (options.corruptChecksum) {
    const metadata = JSON.parse(
      await readFile(artifact.metadataPath, 'utf8'),
    ) as { files: Array<{ sha256: string }> }
    const file = metadata.files[0]
    if (!file) throw new Error('Missing artifact metadata file')
    file.sha256 = '0'.repeat(64)
    await writeFile(
      artifact.metadataPath,
      `${JSON.stringify(metadata, null, 2)}\n`,
    )
  }
  const [archive] = await archiveArtifacts(
    [artifact],
    product.archiveBaseName,
    { filesOnly: true },
  )
  if (!archive) throw new Error('Missing archive result')
  return { product, bytes: await readFile(archive.zipPath) }
}

function artifactClient(bytes: Buffer): S3Client {
  return {
    send: async (command: { constructor: { name: string } }) => {
      if (command.constructor.name !== 'GetObjectCommand') {
        throw new Error(`Unexpected command: ${command.constructor.name}`)
      }
      return {
        Body: { transformToByteArray: async () => bytes },
        Metadata: {
          component: fakeR2Config.uploadPrefix,
          'release-sha': RELEASE_SHA,
          version: VERSION,
          target: 'darwin-arm64',
          sha256: createHash('sha256').update(bytes).digest('hex'),
        },
      }
    },
  } as unknown as S3Client
}
