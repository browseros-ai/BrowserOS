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
import { createR2Client, joinObjectKey, uploadFileToObject } from './r2'
import type {
  AssetBuildProductDescriptor,
  R2Config,
  StagedAssetArtifact,
} from './types'

// Static asset bundles are platform-independent, so a single artifact stands
// in for the per-target artifacts of the binary flow.
export const ASSET_TARGET_ID = 'universal'

export interface AssetUploadResult {
  latestR2Key: string
  versionR2Key: string
}

/** Stages the built assets directory as a `resources/` artifact with metadata. */
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

/** Zips the staged asset artifact without a target suffix. */
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

/** Uploads the asset zip under the latest and versioned keys. */
export async function uploadAssetArchive(
  zipPath: string,
  version: string,
  client: S3Client,
  r2: R2Config,
): Promise<AssetUploadResult> {
  const fileName = basename(zipPath)
  const latestR2Key = joinObjectKey(r2.uploadPrefix, 'latest', fileName)
  const versionR2Key = joinObjectKey(r2.uploadPrefix, version, fileName)
  await uploadFileToObject(client, r2, latestR2Key, zipPath)
  await uploadFileToObject(client, r2, versionR2Key, zipPath)
  return { latestR2Key, versionR2Key }
}

/** Runs the descriptor-driven production asset build from a wrapper script. */
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

  log.header(`Building ${product.label} artifacts v${buildConfig.version}`)
  log.info(`Mode: ${args.ci ? 'ci' : 'full'}`)

  const [command, ...commandArgs] = product.buildCommand
  if (!command) {
    throw new Error(`Missing build command for ${product.label}`)
  }
  log.step(`Running ${product.buildCommand.join(' ')}`)
  // Unlike the binary flow, inline env reaches the asset build as process
  // env — it must win over ambient values (e.g. NODE_ENV=test under bun
  // test) so the artifact is always a production build.
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
  const zipPath = await archiveAssetArtifact(artifact, product)

  if (args.upload) {
    if (!buildConfig.r2) {
      throw new Error(
        `R2 configuration is required for ${product.label} uploads`,
      )
    }
    const client = createR2Client(buildConfig.r2)
    try {
      log.step('Uploading artifact zip')
      const result = await uploadAssetArchive(
        zipPath,
        buildConfig.version,
        client,
        buildConfig.r2,
      )
      log.info(`R2 latest key: ${result.latestR2Key}`)
      log.info(`R2 version key: ${result.versionR2Key}`)
    } finally {
      client.destroy()
    }
  }

  log.done(`Production ${product.label} artifacts completed`)
  log.info(`${ASSET_TARGET_ID}: ${zipPath}`)
}
