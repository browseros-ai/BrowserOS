import { existsSync } from 'node:fs'
import { resolve } from 'node:path'

import { archiveAndUploadArtifacts, archiveArtifacts } from './archive'
import { parseBuildArgs } from './cli'
import { compileProductBinaries } from './compile'
import { loadBuildConfig } from './config'
import { log } from './log'
import { getTargetRules, loadManifest } from './manifest'
import { createR2Client } from './r2'
import { stageCompiledArtifact, stageTargetArtifact } from './stage'
import type { BuildProductDescriptor, ResourceManifest } from './types'

function buildModeLabel(ci: boolean): string {
  return ci ? 'ci' : 'full'
}

function manifestNeedsR2(manifest: ResourceManifest): boolean {
  return manifest.resources.some((rule) => rule.source.type === 'r2')
}

/** Runs the descriptor-driven production artifact build from a wrapper script. */
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

  const compiled = await compileProductBinaries(
    product,
    args.targets,
    buildConfig.envVars,
    buildConfig.processEnv,
    buildConfig.version,
    { ci: args.ci },
  )

  if (args.ci) {
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

  const stagedArtifacts = []
  const r2 = buildConfig.r2
  const client = r2 ? createR2Client(r2) : null

  try {
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
          )
        : await archiveArtifacts(stagedArtifacts, product.archiveBaseName)

    log.done(`Production ${product.label} artifacts completed`)
    for (const result of uploadResults) {
      log.info(`${result.targetId}: ${result.zipPath}`)
      if (result.latestR2Key) {
        log.info(`R2 latest key: ${result.latestR2Key}`)
      }
      if (result.versionR2Key) {
        log.info(`R2 version key: ${result.versionR2Key}`)
      }
    }
  } finally {
    client?.destroy()
  }
}
