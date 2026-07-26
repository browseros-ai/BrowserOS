import { rm } from 'node:fs/promises'
import { basename, dirname, isAbsolute, join, resolve } from 'node:path'

import type { S3Client } from '@aws-sdk/client-s3'

import { runCommand } from './command'
import { joinObjectKey, uploadFileToObject } from './r2'
import type { R2Config, StagedArtifact, UploadResult } from './types'

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
  if (process.platform === 'win32') {
    await runCommand(
      'powershell',
      [
        '-NoProfile',
        '-Command',
        `Compress-Archive -Path * -DestinationPath '${absoluteOutputZipPath.replace(/'/g, "''")}' -Force`,
      ],
      process.env,
      artifactRoot,
    )
    return
  }
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
  archiveBaseName: string,
): Promise<UploadResult[]> {
  const results = await archiveArtifacts(artifacts, archiveBaseName)
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
