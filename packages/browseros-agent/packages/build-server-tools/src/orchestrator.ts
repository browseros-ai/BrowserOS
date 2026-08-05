import { existsSync } from 'node:fs'
import { basename, join, resolve } from 'node:path'

import type { S3Client } from '@aws-sdk/client-s3'

import { archiveAndUploadArtifacts, archiveArtifacts } from './archive'
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
  BuildProductDescriptor,
  BuildTarget,
  R2Config,
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
  product: BuildProductDescriptor,
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

export async function runProdResourceBuild(
  product: BuildProductDescriptor,
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

  log.header(`Building ${product.label} artifacts v${buildConfig.version}`)
  log.info(`Targets: ${args.targets.map((target) => target.id).join(', ')}`)
  log.info(`Mode: ${buildModeLabel(args.ci)}`)

  if (args.ci) {
    const compiled = await compileProductBinaries(
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
      )
      localArtifacts.push(staged)
      log.success(`Packaged ${binary.target.id}`)
    }

    const archiveResults = await archiveArtifacts(
      localArtifacts,
      product.archiveBaseName,
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
  const releaseSha = process.env.RELEASE_SHA?.trim()
  if (args.versionedOnly && !releaseSha) {
    throw new Error('RELEASE_SHA is required for versioned-only uploads')
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
        ? await compileProductBinaries(
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
            )
          : await stageCompiledArtifact(
              product,
              binary.binaryPath,
              binary.target,
              buildConfig.version,
              rules,
              rootDir,
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
            { releaseSha, versionedOnly: args.versionedOnly },
          )
        : await archiveArtifacts(stagedArtifacts, product.archiveBaseName)

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
