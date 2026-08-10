import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'

import type { S3Client } from '@aws-sdk/client-s3'

import {
  archiveAndUploadArtifacts,
  archiveArtifacts,
  validateArtifactArchive,
} from './archive'
import { parseBuildArgs } from './cli'
import { compileProductBinaries } from './compile'
import { loadBuildConfig } from './config'
import { log } from './log'
import { getTargetRules, loadManifest } from './manifest'
import {
  createR2Client,
  joinObjectKey,
  recoverImmutableFileFromObject,
} from './r2'
import { stageCompiledArtifact, stageTargetArtifact } from './stage'
import type {
  ArtifactMetadataIdentity,
  BuildProductDescriptor,
  BuildTarget,
  ProductCompiler,
  R2Config,
  ResourceBuildProductDescriptor,
  ResourceManifest,
  UploadResult,
} from './types'

function buildModeLabel(ci: boolean): string {
  return ci ? 'ci' : 'full'
}

function manifestNeedsR2(manifest: ResourceManifest): boolean {
  return manifest.resources.some((rule) => rule.source.type === 'r2')
}

export async function recoverVersionedTargets(
  product: ResourceBuildProductDescriptor,
  targets: BuildTarget[],
  version: string,
  releaseSha: string,
  client: S3Client,
  r2: R2Config,
): Promise<{
  recoveredResults: UploadResult[]
  targetsToBuild: BuildTarget[]
}> {
  const recoveredResults: UploadResult[] = []
  const targetsToBuild: BuildTarget[] = []

  for (const target of targets) {
    const zipPath = join(
      product.distRoot,
      `${product.archiveBaseName}-${target.id}.zip`,
    )
    const versionR2Key = joinObjectKey(
      r2.uploadPrefix,
      version,
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
        target: target.id,
        version,
      },
    )
    if (recovered) {
      const expectedFiles = product.expectedArtifactFiles?.(target)
      if (expectedFiles) {
        validateArtifactArchive(
          zipPath,
          {
            component: r2.uploadPrefix,
            releaseSha,
            target: target.id,
            version,
          },
          expectedFiles,
        )
      }
      recoveredResults.push({
        targetId: target.id,
        versionR2Key,
        zipPath,
      })
      log.success(`Recovered ${target.id} from immutable R2`)
    } else {
      targetsToBuild.push(target)
    }
  }

  return { recoveredResults, targetsToBuild }
}

function orderArtifactResults(
  targets: BuildTarget[],
  results: UploadResult[],
): UploadResult[] {
  const resultsByTarget = new Map(
    results.map((result) => [result.targetId, result]),
  )
  return targets.map((target) => {
    const result = resultsByTarget.get(target.id)
    if (!result) {
      throw new Error(`Missing artifact result for ${target.id}`)
    }
    return result
  })
}

function logArtifactResults(results: UploadResult[]): void {
  for (const result of results) {
    log.info(`${result.targetId}: ${result.zipPath}`)
    if (result.latestR2Key) {
      log.info(`R2 latest key: ${result.latestR2Key}`)
    }
    if (result.versionR2Key) {
      log.info(`R2 version key: ${result.versionR2Key}`)
    }
  }
}

function requireFullReleaseSha(value: string, source: string): string {
  if (!/^[0-9a-f]{40}$/i.test(value)) {
    throw new Error(`${source} must be a full 40-character git SHA`)
  }
  return value.toLowerCase()
}

function currentReleaseSha(rootDir: string): string {
  try {
    return requireFullReleaseSha(
      execFileSync('git', ['rev-parse', 'HEAD'], {
        cwd: rootDir,
        encoding: 'utf8',
      }).trim(),
      'git rev-parse HEAD',
    )
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`Could not resolve artifact release SHA: ${message}`)
  }
}

function resolveReleaseContext(
  product: ResourceBuildProductDescriptor,
  uploadPrefix: string,
  rootDir: string,
  versionedOnly: boolean,
): {
  releaseSha?: string
  artifactIdentity?: ArtifactMetadataIdentity
} {
  const releaseShaValue = process.env.RELEASE_SHA?.trim()
  const releaseSha = releaseShaValue
    ? requireFullReleaseSha(releaseShaValue, 'RELEASE_SHA')
    : undefined
  const checkoutReleaseSha =
    releaseSha || product.includeArtifactIdentity
      ? currentReleaseSha(rootDir)
      : undefined
  if (releaseSha && releaseSha !== checkoutReleaseSha) {
    throw new Error(
      `RELEASE_SHA ${releaseSha} does not match checkout HEAD ${checkoutReleaseSha}`,
    )
  }
  if (versionedOnly && !releaseSha) {
    throw new Error('RELEASE_SHA is required for versioned-only uploads')
  }
  if (!product.includeArtifactIdentity) {
    return { releaseSha }
  }
  if (!checkoutReleaseSha) {
    throw new Error('Could not resolve the artifact release SHA')
  }
  return {
    releaseSha,
    artifactIdentity: {
      component: uploadPrefix,
      releaseSha: checkoutReleaseSha,
    },
  }
}

export async function runCompiledResourceBuild<
  TProduct extends ResourceBuildProductDescriptor,
>(
  product: TProduct,
  compiler: ProductCompiler<TProduct>,
  argv: string[],
  options: { rootDir?: string } = {},
): Promise<void> {
  const rootDir = options.rootDir ?? resolve(import.meta.dir, '../../..')
  process.chdir(rootDir)

  const args = parseBuildArgs(argv, product)
  const manifestPath = resolve(rootDir, args.manifestPath)
  if (!existsSync(manifestPath)) {
    throw new Error(`Manifest not found: ${manifestPath}`)
  }
  const manifest = loadManifest(manifestPath)
  const requireR2 = !args.ci && (args.upload || manifestNeedsR2(manifest))
  const buildConfig = loadBuildConfig(rootDir, product, {
    ci: args.ci,
    requireR2,
  })
  const { releaseSha, artifactIdentity } = resolveReleaseContext(
    product,
    buildConfig.r2?.uploadPrefix ?? product.env.defaultR2UploadPrefix,
    rootDir,
    args.versionedOnly,
  )

  log.header(`Building ${product.label} artifacts v${buildConfig.version}`)
  log.info(`Targets: ${args.targets.map((target) => target.id).join(', ')}`)
  log.info(`Mode: ${buildModeLabel(args.ci)}`)

  if (args.ci) {
    const compiled = await compiler(
      product,
      args.targets,
      buildConfig.envVars,
      buildConfig.processEnv,
      buildConfig.version,
      { ci: true },
    )
    const localArtifacts = []

    for (const binary of compiled) {
      log.step(`Packaging ${binary.target.name}`)
      const rules = getTargetRules(manifest, binary.target).filter(
        (rule) => rule.source.type === 'local',
      )
      const staged = await stageCompiledArtifact(
        product,
        binary.binaryPath,
        binary.target,
        buildConfig.version,
        rules,
        rootDir,
        artifactIdentity,
      )
      localArtifacts.push(staged)
      log.success(`Packaged ${binary.target.id}`)
    }

    const archiveResults = await archiveArtifacts(
      localArtifacts,
      product.archiveBaseName,
      { filesOnly: product.archiveFilesOnly },
    )
    log.done('CI build completed')
    for (const result of archiveResults) {
      log.info(`${result.targetId}: ${result.zipPath}`)
    }
    return
  }

  if (!buildConfig.r2 && requireR2) {
    throw new Error(`R2 configuration is required for ${product.label} builds`)
  }

  const stagedArtifacts = []
  const r2 = buildConfig.r2
  const client = r2 ? createR2Client(r2) : null

  try {
    const { recoveredResults, targetsToBuild } =
      args.versionedOnly && client && r2 && releaseSha
        ? await recoverVersionedTargets(
            product,
            args.targets,
            buildConfig.version,
            releaseSha,
            client,
            r2,
          )
        : { recoveredResults: [], targetsToBuild: args.targets }

    const compiled =
      targetsToBuild.length > 0
        ? await compiler(
            product,
            targetsToBuild,
            buildConfig.envVars,
            buildConfig.processEnv,
            buildConfig.version,
          )
        : []

    for (const binary of compiled) {
      const rules = getTargetRules(manifest, binary.target)
      log.step(
        `Staging ${binary.target.name} (${rules.length} resource rule(s))`,
      )
      const staged =
        client && r2
          ? await stageTargetArtifact(
              product,
              binary.binaryPath,
              binary.target,
              rules,
              rootDir,
              client,
              r2,
              buildConfig.version,
              artifactIdentity,
            )
          : await stageCompiledArtifact(
              product,
              binary.binaryPath,
              binary.target,
              buildConfig.version,
              rules,
              rootDir,
              artifactIdentity,
            )
      stagedArtifacts.push(staged)
      log.success(`Staged ${binary.target.id}`)
    }

    const uploadResults =
      client && r2
        ? await archiveAndUploadArtifacts(
            stagedArtifacts,
            buildConfig.version,
            client,
            r2,
            args.upload,
            product.archiveBaseName,
            {
              releaseSha: releaseSha ?? artifactIdentity?.releaseSha,
              versionedOnly: args.versionedOnly,
              filesOnly: product.archiveFilesOnly,
            },
          )
        : await archiveArtifacts(stagedArtifacts, product.archiveBaseName, {
            filesOnly: product.archiveFilesOnly,
          })

    const orderedResults = orderArtifactResults(args.targets, [
      ...recoveredResults,
      ...uploadResults,
    ])

    log.done(`Production ${product.label} artifacts completed`)
    logArtifactResults(orderedResults)
  } finally {
    client?.destroy()
  }
}

export async function runProdResourceBuild(
  product: BuildProductDescriptor,
  argv: string[],
  options: { rootDir?: string } = {},
): Promise<void> {
  return runCompiledResourceBuild(
    product,
    compileProductBinaries,
    argv,
    options,
  )
}
