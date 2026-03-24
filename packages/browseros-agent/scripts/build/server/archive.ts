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

export async function zipArtifactRoot(
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
): Promise<UploadResult[]> {
  const results: UploadResult[] = []

  for (const artifact of artifacts) {
    const zipPath = zipPathForArtifact(artifact)
    await zipArtifactRoot(artifact.rootDir, zipPath)

    if (!upload) {
      results.push({ targetId: artifact.target.id, zipPath })
      continue
    }

    const fileName = basename(zipPath)
    const latestR2Key = joinObjectKey(r2.uploadPrefix, 'latest', fileName)
    const versionR2Key = joinObjectKey(r2.uploadPrefix, version, fileName)
    await uploadFileToObject(client, r2, latestR2Key, zipPath)
    await uploadFileToObject(client, r2, versionR2Key, zipPath)
    results.push({
      targetId: artifact.target.id,
      zipPath,
      latestR2Key,
      versionR2Key,
    })
  }

  return results
}
