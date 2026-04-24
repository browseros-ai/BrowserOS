#!/usr/bin/env bun
import { copyFile, mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import type { Arch } from './common/arch'
import {
  type AgentEntry,
  type AgentManifest,
  type Artifact,
  type Bundle,
  type BundleAgent,
  tarballKey,
} from './common/manifest'
import { sha256File, verifySha256 } from './common/sha256'

export const DEV_ARCH: Arch = 'arm64'

export interface BuiltAgentArtifact {
  agent: BundleAgent
  key: string
  path: string
  sha256: string
  sizeBytes: number
}

export interface DevAgentEntry extends Omit<AgentEntry, 'tarballs'> {
  tarballs: Partial<Record<Arch, Artifact>>
}

export interface DevAgentManifest extends Omit<AgentManifest, 'agents'> {
  agents: Record<string, DevAgentEntry>
}

if (import.meta.main) {
  await seedDevAgentTarballs()
}

export async function seedDevAgentTarballs(): Promise<void> {
  assertDevelopment()

  const pkgRoot = path.resolve(import.meta.dir, '..')
  const bundle = await readBundle(pkgRoot)
  const distImagesDir = path.join(pkgRoot, 'dist', 'images')
  const cacheRoot = devCacheRoot()
  const artifacts: BuiltAgentArtifact[] = []

  for (const agent of bundle.agents) {
    await buildTarball(pkgRoot, agent, distImagesDir)
    const artifact = await readBuiltArtifact(agent, distImagesDir)
    await seedArtifact(cacheRoot, artifact)
    artifacts.push(artifact)
  }

  const manifestPath = path.join(cacheRoot, 'vm', 'manifest.json')
  await mkdir(path.dirname(manifestPath), { recursive: true })
  await writeFile(
    manifestPath,
    `${JSON.stringify(buildDevManifest(artifacts), null, 2)}\n`,
  )
  console.log(`manifest written to ${manifestPath}`)
}

export function buildDevManifest(
  artifacts: BuiltAgentArtifact[],
  now: Date = new Date(),
): DevAgentManifest {
  const agents: Record<string, DevAgentEntry> = {}
  for (const artifact of artifacts) {
    agents[artifact.agent.name] = {
      image: artifact.agent.image,
      version: artifact.agent.version,
      tarballs: {
        [DEV_ARCH]: {
          key: artifact.key,
          sha256: artifact.sha256,
          sizeBytes: artifact.sizeBytes,
        },
      },
    }
  }

  return {
    schemaVersion: 2,
    updatedAt: now.toISOString(),
    agents,
  }
}

async function readBundle(pkgRoot: string): Promise<Bundle> {
  return JSON.parse(
    await readFile(path.join(pkgRoot, 'bundle.json'), 'utf8'),
  ) as Bundle
}

async function buildTarball(
  pkgRoot: string,
  agent: BundleAgent,
  outputDir: string,
): Promise<void> {
  console.log(`building ${agent.name} ${DEV_ARCH} tarball`)
  await spawnChecked(
    [
      'bun',
      'run',
      'scripts/build-tarball.ts',
      '--',
      '--agent',
      agent.name,
      '--arch',
      DEV_ARCH,
      '--output-dir',
      outputDir,
    ],
    pkgRoot,
  )
}

async function readBuiltArtifact(
  agent: BundleAgent,
  distImagesDir: string,
): Promise<BuiltAgentArtifact> {
  const key = tarballKey(agent.name, agent.version, DEV_ARCH)
  const filePath = path.join(distImagesDir, path.basename(key))
  await assertExists(filePath, agent.name)
  return {
    agent,
    key,
    path: filePath,
    sha256: await sha256File(filePath),
    sizeBytes: (await stat(filePath)).size,
  }
}

async function seedArtifact(
  cacheRoot: string,
  artifact: BuiltAgentArtifact,
): Promise<void> {
  const dest = path.join(cacheRoot, artifact.key)
  if (await matchesExisting(dest, artifact.sha256)) {
    console.log(`cache hit: ${artifact.key}`)
    return
  }

  await mkdir(path.dirname(dest), { recursive: true })
  await copyFile(artifact.path, dest)
  await verifySha256(dest, artifact.sha256)
  console.log(`seeded ${artifact.key}`)
}

function assertDevelopment(): void {
  if (process.env.NODE_ENV === 'development') {
    return
  }
  throw new Error(
    'dev:seed:tarball refuses to run without NODE_ENV=development; it writes to ~/.browseros-dev/cache/vm/',
  )
}

function devCacheRoot(): string {
  return path.join(
    homedir(),
    PATHS.DEV_BROWSEROS_DIR_NAME,
    PATHS.CACHE_DIR_NAME,
  )
}

async function assertExists(
  filePath: string,
  agentName: string,
): Promise<void> {
  try {
    await stat(filePath)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw error
    }
    throw new Error(`build did not produce ${agentName} tarball at ${filePath}`)
  }
}

async function matchesExisting(
  filePath: string,
  expectedSha: string,
): Promise<boolean> {
  try {
    return (await sha256File(filePath)) === expectedSha
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return false
    }
    throw error
  }
}

async function spawnChecked(argv: string[], cwd: string): Promise<void> {
  const proc = Bun.spawn(argv, {
    cwd,
    stdout: 'inherit',
    stderr: 'inherit',
  })
  const code = await proc.exited
  if (code !== 0) {
    throw new Error(`${argv.join(' ')} exited with code ${code}`)
  }
}
