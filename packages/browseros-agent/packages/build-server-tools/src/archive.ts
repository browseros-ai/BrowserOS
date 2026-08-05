import { readdir, rm, utimes } from 'node:fs/promises'
import { basename, dirname, isAbsolute, join, resolve } from 'node:path'

import type { S3Client } from '@aws-sdk/client-s3'

import { runCommand } from './command'
import {
  joinObjectKey,
  uploadFileToObject,
  uploadImmutableFileToObject,
} from './r2'
import type { R2Config, StagedArtifact, UploadResult } from './types'

const ARCHIVE_TIMESTAMP = new Date('1980-01-01T00:00:00.000Z')

function zipPathForArtifact(
  artifact: StagedArtifact,
  archiveBaseName: string,
): string {
  return join(
    dirname(artifact.rootDir),
    `${archiveBaseName}-${artifact.target.id}.zip`,
  )
}

export async function zipDirectory(
  artifactRoot: string,
  outputZipPath: string,
): Promise<void> {
  const absoluteOutputZipPath = isAbsolute(outputZipPath)
    ? outputZipPath
    : resolve(outputZipPath)
  await rm(absoluteOutputZipPath, { force: true })
  await normalizeArchiveTimestamps(artifactRoot)
  await runCommand(
    'zip',
    ['-X', '-r', '-q', absoluteOutputZipPath, '.'],
    process.env,
    artifactRoot,
  )
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
  options: { releaseSha?: string; versionedOnly?: boolean } = {},
): Promise<UploadResult[]> {
  if (options.versionedOnly && !options.releaseSha) {
    throw new Error('releaseSha is required for versioned-only uploads')
  }
  const results = await archiveArtifacts(artifacts, archiveBaseName)
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
): Promise<UploadResult[]> {
  const results: UploadResult[] = []

  for (const artifact of artifacts) {
    const zipPath = zipPathForArtifact(artifact, archiveBaseName)
    await zipDirectory(artifact.rootDir, zipPath)
    results.push({ targetId: artifact.target.id, zipPath })
  }

  return results
}
