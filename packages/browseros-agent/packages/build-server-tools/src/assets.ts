import { existsSync } from 'node:fs'
import { cp, mkdir, readdir, rm } from 'node:fs/promises'
import { basename, dirname, isAbsolute, join, resolve } from 'node:path'

import type { S3Client } from '@aws-sdk/client-s3'

import { zipDirectory } from './archive'
import { parseAssetBuildArgs } from './cli'
import { runCommand } from './command'
import { loadBuildConfig } from './config'
import { log } from './log'
import { writeArtifactMetadata } from './metadata'
import {
  createR2Client,
  joinObjectKey,
  recoverImmutableFileFromObject,
  uploadFileToObject,
  uploadImmutableFileToObject,
} from './r2'
import type {
  AssetBuildProductDescriptor,
  R2Config,
  StagedAssetArtifact,
} from './types'

export const ASSET_TARGET_ID = 'universal'

export interface AssetUploadResult {
  latestR2Key?: string
  versionR2Key: string
}

export async function stageAssetArtifact(
  product: AssetBuildProductDescriptor,
  version: string,
  sourceRoot = process.cwd(),
): Promise<StagedAssetArtifact> {
  const assetsPath = isAbsolute(product.assetsDir)
    ? product.assetsDir
    : resolve(sourceRoot, product.assetsDir)
  if (!existsSync(assetsPath)) {
    throw new Error(`Built assets directory not found: ${product.assetsDir}`)
  }
  const entries = await readdir(assetsPath)
  if (entries.length === 0) {
    throw new Error(`Built assets directory is empty: ${product.assetsDir}`)
  }

  const distRoot = isAbsolute(product.distRoot)
    ? product.distRoot
    : resolve(sourceRoot, product.distRoot)
  const rootDir = join(distRoot, ASSET_TARGET_ID)
  const resourcesDir = join(rootDir, 'resources')
  await rm(rootDir, { recursive: true, force: true })
  await mkdir(rootDir, { recursive: true })
  await cp(assetsPath, resourcesDir, { recursive: true })
  const metadataPath = await writeArtifactMetadata(
    rootDir,
    ASSET_TARGET_ID,
    version,
  )

  return { rootDir, resourcesDir, metadataPath }
}

export async function archiveAssetArtifact(
  artifact: StagedAssetArtifact,
  product: AssetBuildProductDescriptor,
): Promise<string> {
  const zipPath = join(
    dirname(artifact.rootDir),
    `${product.archiveBaseName}.zip`,
  )
  await zipDirectory(artifact.rootDir, zipPath)
  return zipPath
}

export async function uploadAssetArchive(
  zipPath: string,
  version: string,
  client: S3Client,
  r2: R2Config,
  options: { releaseSha?: string; versionedOnly?: boolean } = {},
): Promise<AssetUploadResult> {
  if (options.versionedOnly && !options.releaseSha) {
    throw new Error('releaseSha is required for versioned-only uploads')
  }
  const fileName = basename(zipPath)
  const latestR2Key = joinObjectKey(r2.uploadPrefix, 'latest', fileName)
  const versionR2Key = joinObjectKey(r2.uploadPrefix, version, fileName)
  await uploadImmutableFileToObject(client, r2, versionR2Key, zipPath, {
    identity: options.releaseSha
      ? {
          component: r2.uploadPrefix,
          releaseSha: options.releaseSha,
          target: ASSET_TARGET_ID,
          version,
        }
      : undefined,
  })
  if (!options.versionedOnly) {
    await uploadFileToObject(client, r2, latestR2Key, zipPath)
  }
  return {
    latestR2Key: options.versionedOnly ? undefined : latestR2Key,
    versionR2Key,
  }
}

export async function runProdAssetBuild(
  product: AssetBuildProductDescriptor,
  argv: string[],
  options: { rootDir?: string } = {},
): Promise<void> {
  const rootDir = options.rootDir ?? resolve(import.meta.dir, '../../..')
  process.chdir(rootDir)

  const args = parseAssetBuildArgs(argv, product)
  const requireR2 = !args.ci && args.upload
  const buildConfig = loadBuildConfig(rootDir, product, {
    ci: args.ci,
    requireR2,
  })
  const releaseSha = process.env.RELEASE_SHA?.trim()
  if (args.versionedOnly && !releaseSha) {
    throw new Error('RELEASE_SHA is required for versioned-only uploads')
  }
  const r2 = buildConfig.r2
  if (args.upload && !r2) {
    throw new Error(`R2 configuration is required for ${product.label} uploads`)
  }
  const client = args.upload && r2 ? createR2Client(r2) : null

  log.header(`Building ${product.label} artifacts v${buildConfig.version}`)
  log.info(`Mode: ${args.ci ? 'ci' : 'full'}`)

  try {
    const distRoot = isAbsolute(product.distRoot)
      ? product.distRoot
      : resolve(rootDir, product.distRoot)
    const zipPath = join(distRoot, `${product.archiveBaseName}.zip`)
    if (args.versionedOnly && client && r2 && releaseSha) {
      const versionR2Key = joinObjectKey(
        r2.uploadPrefix,
        buildConfig.version,
        basename(zipPath),
      )
      const recovered = await recoverImmutableFileFromObject(
        client,
        r2,
        versionR2Key,
        zipPath,
        {
          component: r2.uploadPrefix,
          releaseSha,
          target: ASSET_TARGET_ID,
          version: buildConfig.version,
        },
      )
      if (recovered) {
        log.done(`Recovered ${product.label} artifact from immutable R2`)
        log.info(`${ASSET_TARGET_ID}: ${zipPath}`)
        return
      }
    }

    const [command, ...commandArgs] = product.buildCommand
    if (!command) {
      throw new Error(`Missing build command for ${product.label}`)
    }
    log.step(`Running ${product.buildCommand.join(' ')}`)
    // Inline descriptor values must override ambient test/development values.
    await runCommand(
      command,
      commandArgs,
      { ...buildConfig.processEnv, ...buildConfig.envVars },
      join(rootDir, product.packageDir),
    )

    log.step('Staging assets')
    const artifact = await stageAssetArtifact(
      product,
      buildConfig.version,
      rootDir,
    )
    const builtZipPath = await archiveAssetArtifact(artifact, product)

    if (client && r2) {
      log.step('Uploading artifact zip')
      const result = await uploadAssetArchive(
        builtZipPath,
        buildConfig.version,
        client,
        r2,
        { releaseSha, versionedOnly: args.versionedOnly },
      )
      if (result.latestR2Key) {
        log.info(`R2 latest key: ${result.latestR2Key}`)
      }
      log.info(`R2 version key: ${result.versionR2Key}`)
    }

    log.done(`Production ${product.label} artifacts completed`)
    log.info(`${ASSET_TARGET_ID}: ${builtZipPath}`)
  } finally {
    client?.destroy()
  }
}
