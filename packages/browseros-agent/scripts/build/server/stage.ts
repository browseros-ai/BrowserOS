import { chmod, cp, mkdir, rm } from 'node:fs/promises'
import { dirname, isAbsolute, join, relative, resolve } from 'node:path'

import type { S3Client } from '@aws-sdk/client-s3'

import { writeArtifactMetadata } from './metadata'
import { downloadObjectToFile } from './r2'
import type {
  BuildTarget,
  R2Config,
  ResourceRule,
  StagedArtifact,
} from './types'

function artifactRoot(distRoot: string, target: BuildTarget): string {
  return join(distRoot, target.id)
}

function serverDestinationPath(rootDir: string, target: BuildTarget): string {
  return join(rootDir, 'resources', 'bin', target.serverBinaryName)
}

async function copyServerBinary(
  compiledBinaryPath: string,
  destinationPath: string,
  target: BuildTarget,
): Promise<void> {
  await mkdir(dirname(destinationPath), { recursive: true })
  await cp(compiledBinaryPath, destinationPath)
  if (target.os !== 'windows') {
    await chmod(destinationPath, 0o755)
  }
}

async function createArtifactRoot(
  distRoot: string,
  compiledBinaryPath: string,
  target: BuildTarget,
): Promise<string> {
  const rootDir = artifactRoot(distRoot, target)
  await rm(rootDir, { recursive: true, force: true })
  await mkdir(rootDir, { recursive: true })
  await copyServerBinary(
    compiledBinaryPath,
    serverDestinationPath(rootDir, target),
    target,
  )
  return rootDir
}

async function finalizeArtifact(
  rootDir: string,
  target: BuildTarget,
  version: string,
): Promise<StagedArtifact> {
  const metadataPath = await writeArtifactMetadata(rootDir, target, version)
  return {
    target,
    rootDir,
    resourcesDir: join(rootDir, 'resources'),
    metadataPath,
  }
}

function resolveDestination(rootDir: string, destination: string): string {
  const outputPath = join(rootDir, destination)
  const relativePath = relative(rootDir, outputPath)
  if (relativePath.startsWith('..') || isAbsolute(relativePath)) {
    throw new Error(
      `Invalid destination path outside artifact root: ${destination}`,
    )
  }
  return outputPath
}

async function stageRule(
  rootDir: string,
  sourceRoot: string,
  rule: ResourceRule,
  target: BuildTarget,
  client: S3Client,
  r2: R2Config,
): Promise<void> {
  const destinationPath = resolveDestination(rootDir, rule.destination)
  await mkdir(dirname(destinationPath), { recursive: true })

  if (rule.source.type === 'local') {
    await stageLocalRule(destinationPath, sourceRoot, rule, target)
  } else {
    await downloadObjectToFile(client, r2, rule.source.key, destinationPath)
    if (rule.executable && target.os !== 'windows') {
      await chmod(destinationPath, 0o755)
    }
  }
}

async function stageLocalRule(
  destinationPath: string,
  sourceRoot: string,
  rule: ResourceRule,
  target: BuildTarget,
): Promise<void> {
  if (rule.source.type !== 'local') {
    throw new Error(`Expected local source rule, got ${rule.source.type}`)
  }

  await mkdir(dirname(destinationPath), { recursive: true })
  const sourcePath = isAbsolute(rule.source.path)
    ? rule.source.path
    : resolve(sourceRoot, rule.source.path)
  await cp(sourcePath, destinationPath)

  if (rule.executable && target.os !== 'windows') {
    await chmod(destinationPath, 0o755)
  }
}

export async function stageTargetArtifact(
  distRoot: string,
  compiledBinaryPath: string,
  target: BuildTarget,
  rules: ResourceRule[],
  sourceRoot: string,
  client: S3Client,
  r2: R2Config,
  version: string,
): Promise<StagedArtifact> {
  const rootDir = await createArtifactRoot(distRoot, compiledBinaryPath, target)

  for (const rule of rules) {
    await stageRule(rootDir, sourceRoot, rule, target, client, r2)
  }

  return finalizeArtifact(rootDir, target, version)
}

export async function stageCompiledArtifact(
  distRoot: string,
  compiledBinaryPath: string,
  target: BuildTarget,
  version: string,
  rules: ResourceRule[] = [],
  sourceRoot = process.cwd(),
): Promise<StagedArtifact> {
  const rootDir = await createArtifactRoot(distRoot, compiledBinaryPath, target)

  for (const rule of rules) {
    if (rule.source.type !== 'local') {
      continue
    }
    await stageLocalRule(
      resolveDestination(rootDir, rule.destination),
      sourceRoot,
      rule,
      target,
    )
  }

  return finalizeArtifact(rootDir, target, version)
}
