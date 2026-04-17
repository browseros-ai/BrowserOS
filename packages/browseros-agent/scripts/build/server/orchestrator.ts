import { existsSync } from 'node:fs'
import { resolve } from 'node:path'

import { log } from '../log'
import { archiveAndUploadArtifacts, archiveArtifacts } from './archive'
import { parseBuildArgs } from './cli'
import { compileServerBinaries, getDistProdRoot } from './compile'
import { loadBuildConfig } from './config'
import { getTargetRules, loadManifest } from './manifest'
import { createR2Client } from './r2'
import { stageCompiledArtifact, stageTargetArtifact } from './stage'

function buildModeLabel(ci: boolean): string {
  return ci ? 'ci' : 'full'
}

export async function runProdResourceBuild(argv: string[]): Promise<void> {
  const rootDir = resolve(import.meta.dir, '../../..')
  process.chdir(rootDir)

  const args = parseBuildArgs(argv)

  const buildConfig = loadBuildConfig(rootDir, { ci: args.ci })

  log.header(`Building BrowserOS server artifacts v${buildConfig.version}`)
  log.info(`Targets: ${args.targets.map((target) => target.id).join(', ')}`)
  log.info(`Mode: ${buildModeLabel(args.ci)}`)

  const compiled = await compileServerBinaries(
    args.targets,
    buildConfig.envVars,
    buildConfig.processEnv,
    buildConfig.version,
    { ci: args.ci },
  )

  const manifestPath = resolve(rootDir, args.manifestPath)
  if (!existsSync(manifestPath)) {
    throw new Error(`Manifest not found: ${manifestPath}`)
  }
  const manifest = loadManifest(manifestPath)

  if (args.ci) {
    const distRoot = getDistProdRoot()
    const localArtifacts = []

    for (const binary of compiled) {
      log.step(`Packaging ${binary.target.name}`)
      const rules = getTargetRules(manifest, binary.target).filter(
        (rule) => rule.source.type === 'local',
      )
      const staged = await stageCompiledArtifact(
        distRoot,
        binary.binaryPath,
        binary.target,
        buildConfig.version,
        rules,
        rootDir,
      )
      localArtifacts.push(staged)
      log.success(`Packaged ${binary.target.id}`)
    }

    const archiveResults = await archiveArtifacts(localArtifacts)
    log.done('CI build completed')
    for (const result of archiveResults) {
      log.info(`${result.targetId}: ${result.zipPath}`)
    }
    return
  }

  const distRoot = getDistProdRoot()
  const r2 = buildConfig.r2
  if (!r2) {
    throw new Error('R2 configuration is required for full builds')
  }
  const client = createR2Client(r2)
  const stagedArtifacts = []

  try {
    for (const binary of compiled) {
      const rules = getTargetRules(manifest, binary.target)
      log.step(
        `Staging ${binary.target.name} (${rules.length} resource rule(s))`,
      )
      const staged = await stageTargetArtifact(
        distRoot,
        binary.binaryPath,
        binary.target,
        rules,
        rootDir,
        client,
        r2,
        buildConfig.version,
      )
      stagedArtifacts.push(staged)
      log.success(`Staged ${binary.target.id}`)
    }

    const uploadResults = await archiveAndUploadArtifacts(
      stagedArtifacts,
      buildConfig.version,
      client,
      r2,
      args.upload,
    )

    log.done('Production server artifacts completed')
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
    client.destroy()
  }
}
