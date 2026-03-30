import { existsSync } from 'node:fs'
import { resolve } from 'node:path'

import { log } from '../log'
import { archiveAndUploadArtifacts } from './archive'
import { parseBuildArgs } from './cli'
import { compileServerBinaries, getDistProdRoot } from './compile'
import { loadBuildConfig } from './config'
import { getTargetRules, loadManifest } from './manifest'
import { createR2Client } from './r2'
import { stageTargetArtifact } from './stage'

export async function runProdResourceBuild(argv: string[]): Promise<void> {
  const rootDir = resolve(import.meta.dir, '../../..')
  process.chdir(rootDir)

  const args = parseBuildArgs(argv)

  const buildConfig = loadBuildConfig(rootDir, {
    compileOnly: args.compileOnly,
  })

  log.header(`Building BrowserOS server artifacts v${buildConfig.version}`)
  log.info(`Targets: ${args.targets.map((target) => target.id).join(', ')}`)
  log.info(`Mode: ${args.compileOnly ? 'compile-only' : 'full'}`)

  const compiled = await compileServerBinaries(
    args.targets,
    buildConfig.envVars,
    buildConfig.processEnv,
    buildConfig.version,
  )

  if (args.compileOnly) {
    log.done('Compile-only build completed')
    for (const binary of compiled) {
      log.info(`${binary.target.id}: ${binary.binaryPath}`)
    }
    return
  }

  const manifestPath = resolve(rootDir, args.manifestPath)
  if (!existsSync(manifestPath)) {
    throw new Error(`Manifest not found: ${manifestPath}`)
  }

  const manifest = loadManifest(manifestPath)
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
        `Staging ${binary.target.name} (${rules.length} download rule(s))`,
      )
      const staged = await stageTargetArtifact(
        distRoot,
        binary.binaryPath,
        binary.target,
        rules,
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
