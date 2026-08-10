import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readdir, rm, utimes } from 'node:fs/promises'
import {
  basename,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from 'node:path'

import type { S3Client } from '@aws-sdk/client-s3'

import { runCommand } from './command'
import {
  joinObjectKey,
  uploadFileToObject,
  uploadImmutableFileToObject,
} from './r2'
import type { R2Config, StagedArtifact, UploadResult } from './types'

const ARCHIVE_TIMESTAMP = new Date('1980-01-01T00:00:00.000Z')
const MAX_ARCHIVE_MEMBER_BYTES = 512 * 1024 * 1024

interface ArtifactArchiveIdentity {
  component: string
  releaseSha: string
  target: string
  version: string
}

interface ArtifactFileMetadata {
  path: string
  sha256: string
  size: number
}

function zipPathForArtifact(
  artifact: StagedArtifact,
  archiveBaseName: string,
): string {
  return join(
    dirname(artifact.rootDir),
    `${archiveBaseName}-${artifact.target.id}.zip`,
  )
}

function sameValues(
  left: readonly string[],
  right: readonly string[],
): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  )
}

function readArchiveMember(zipPath: string, member: string): Buffer {
  return execFileSync('unzip', ['-p', zipPath, member], {
    maxBuffer: MAX_ARCHIVE_MEMBER_BYTES,
  })
}

export function validateArtifactArchive(
  zipPath: string,
  identity: ArtifactArchiveIdentity,
  expectedFiles: readonly string[],
): void {
  const members = execFileSync('unzip', ['-Z1', zipPath], {
    encoding: 'utf8',
  })
    .split(/\r?\n/)
    .filter(Boolean)
  const expectedMembers = ['artifact-metadata.json', ...expectedFiles]
  if (!sameValues(members, expectedMembers)) {
    throw new Error(
      `Recovered artifact ${zipPath} has unexpected archive members`,
    )
  }

  const document: unknown = JSON.parse(
    readArchiveMember(zipPath, 'artifact-metadata.json').toString('utf8'),
  )
  if (
    typeof document !== 'object' ||
    document === null ||
    Array.isArray(document)
  ) {
    throw new Error(`Recovered artifact ${zipPath} has invalid metadata`)
  }
  const metadata = document as Record<string, unknown>
  for (const [field, expected] of Object.entries(identity)) {
    if (metadata[field] !== expected) {
      throw new Error(
        `Recovered artifact ${zipPath} has invalid metadata field ${field}`,
      )
    }
  }
  if (!Array.isArray(metadata.files)) {
    throw new Error(`Recovered artifact ${zipPath} has invalid file metadata`)
  }

  const files: ArtifactFileMetadata[] = []
  for (const value of metadata.files) {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) {
      throw new Error(`Recovered artifact ${zipPath} has invalid file metadata`)
    }
    files.push(value as ArtifactFileMetadata)
  }
  const declaredPaths = files.map((file) => file.path)
  if (!sameValues(declaredPaths, expectedFiles)) {
    throw new Error(
      `Recovered artifact ${zipPath} has unexpected declared files`,
    )
  }
  for (const file of files) {
    if (
      typeof file.size !== 'number' ||
      !Number.isSafeInteger(file.size) ||
      file.size < 0 ||
      typeof file.sha256 !== 'string' ||
      !/^[0-9a-f]{64}$/i.test(file.sha256)
    ) {
      throw new Error(
        `Recovered artifact ${zipPath} has invalid metadata for ${file.path}`,
      )
    }
    const bytes = readArchiveMember(zipPath, file.path)
    const sha256 = createHash('sha256').update(bytes).digest('hex')
    if (bytes.length !== file.size || sha256 !== file.sha256.toLowerCase()) {
      throw new Error(
        `Recovered artifact ${zipPath} failed integrity validation for ${file.path}`,
      )
    }
  }
}

export async function zipDirectory(
  artifactRoot: string,
  outputZipPath: string,
  options: { filesOnly?: boolean } = {},
): Promise<void> {
  const absoluteOutputZipPath = isAbsolute(outputZipPath)
    ? outputZipPath
    : resolve(outputZipPath)
  await rm(absoluteOutputZipPath, { force: true })
  await normalizeArchiveTimestamps(artifactRoot)
  const args = options.filesOnly
    ? [
        '-X',
        '-q',
        absoluteOutputZipPath,
        ...(await collectArchiveFiles(artifactRoot)),
      ]
    : ['-X', '-r', '-q', absoluteOutputZipPath, '.']
  await runCommand('zip', args, process.env, artifactRoot)
}

async function collectArchiveFiles(
  root: string,
  current = root,
): Promise<string[]> {
  const files: string[] = []
  const entries = await readdir(current, { withFileTypes: true })
  for (const entry of entries.sort((left, right) =>
    left.name.localeCompare(right.name),
  )) {
    const entryPath = join(current, entry.name)
    if (entry.isDirectory()) {
      files.push(...(await collectArchiveFiles(root, entryPath)))
    } else {
      files.push(relative(root, entryPath).split(sep).join('/'))
    }
  }
  return files
}

async function normalizeArchiveTimestamps(path: string): Promise<void> {
  const entries = await readdir(path, { withFileTypes: true })
  for (const entry of entries.sort((left, right) =>
    left.name.localeCompare(right.name),
  )) {
    const entryPath = join(path, entry.name)
    if (entry.isDirectory()) {
      await normalizeArchiveTimestamps(entryPath)
    } else {
      await utimes(entryPath, ARCHIVE_TIMESTAMP, ARCHIVE_TIMESTAMP)
    }
  }
  await utimes(path, ARCHIVE_TIMESTAMP, ARCHIVE_TIMESTAMP)
}

export async function archiveAndUploadArtifacts(
  artifacts: StagedArtifact[],
  version: string,
  client: S3Client,
  r2: R2Config,
  upload: boolean,
  archiveBaseName: string,
  options: {
    releaseSha?: string
    versionedOnly?: boolean
    filesOnly?: boolean
  } = {},
): Promise<UploadResult[]> {
  if (options.versionedOnly && !options.releaseSha) {
    throw new Error('releaseSha is required for versioned-only uploads')
  }
  const results = await archiveArtifacts(artifacts, archiveBaseName, {
    filesOnly: options.filesOnly,
  })
  if (!upload) {
    return results
  }

  const uploadedResults: UploadResult[] = []
  for (const result of results) {
    const fileName = basename(result.zipPath)
    const latestR2Key = joinObjectKey(r2.uploadPrefix, 'latest', fileName)
    const versionR2Key = joinObjectKey(r2.uploadPrefix, version, fileName)
    await uploadImmutableFileToObject(
      client,
      r2,
      versionR2Key,
      result.zipPath,
      {
        identity: options.releaseSha
          ? {
              component: r2.uploadPrefix,
              releaseSha: options.releaseSha,
              target: result.targetId,
              version,
            }
          : undefined,
      },
    )
    if (!options.versionedOnly) {
      await uploadFileToObject(client, r2, latestR2Key, result.zipPath)
    }
    uploadedResults.push({
      targetId: result.targetId,
      zipPath: result.zipPath,
      latestR2Key: options.versionedOnly ? undefined : latestR2Key,
      versionR2Key,
    })
  }

  return uploadedResults
}

export async function archiveArtifacts(
  artifacts: StagedArtifact[],
  archiveBaseName: string,
  options: { filesOnly?: boolean } = {},
): Promise<UploadResult[]> {
  const results: UploadResult[] = []

  for (const artifact of artifacts) {
    const zipPath = zipPathForArtifact(artifact, archiveBaseName)
    await zipDirectory(artifact.rootDir, zipPath, options)
    results.push({ targetId: artifact.target.id, zipPath })
  }

  return results
}
