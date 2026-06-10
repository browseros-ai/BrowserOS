import { rm } from 'node:fs/promises'
import { basename, dirname, isAbsolute, join, resolve } from 'node:path'

import type { S3Client } from '@aws-sdk/client-s3'

import { runCommand } from './command'
import { joinObjectKey, uploadFileToObject } from './r2'
import type { R2Config, StagedArtifact, UploadResult } from './types'

function zipPathForArtifact(artifact: StagedArtifact): string {
  return join(
    dirname(artifact.rootDir),
    `browseros-server-resources-${artifact.target.id}.zip`,
  )
}

async function zipArtifactRoot(
  artifactRoot: string,
  outputZipPath: string,
): Promise<void> {
  const absoluteOutputZipPath = isAbsolute(outputZipPath)
    ? outputZipPath
    : resolve(outputZipPath)
  await rm(absoluteOutputZipPath, { force: true })
  await runCommand(
    'zip',
    ['-r', '-q', absoluteOutputZipPath, '.'],
    process.env,
    artifactRoot,
  )
}

export async function archiveAndUploadArtifacts(
  artifacts: StagedArtifact[],
  version: string,
  client: S3Client,
  r2: R2Config,
  upload: boolean,
): Promise<UploadResult[]> {
  const results = await archiveArtifacts(artifacts)
  if (!upload) {
    return results
  }

  const uploadedResults: UploadResult[] = []
  for (const result of results) {
    const fileName = basename(result.zipPath)
    const latestR2Key = joinObjectKey(r2.uploadPrefix, 'latest', fileName)
    const versionR2Key = joinObjectKey(r2.uploadPrefix, version, fileName)
    await uploadFileToObject(client, r2, latestR2Key, result.zipPath)
    await uploadFileToObject(client, r2, versionR2Key, result.zipPath)
    uploadedResults.push({
      targetId: result.targetId,
      zipPath: result.zipPath,
      latestR2Key,
      versionR2Key,
    })
  }

  return uploadedResults
}

export async function archiveArtifacts(
  artifacts: StagedArtifact[],
): Promise<UploadResult[]> {
  const results: UploadResult[] = []

  for (const artifact of artifacts) {
    const zipPath = zipPathForArtifact(artifact)
    await zipArtifactRoot(artifact.rootDir, zipPath)
    results.push({ targetId: artifact.target.id, zipPath })
  }

  return results
}
