/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { existsSync } from 'node:fs'
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'
import { ManifestMissingError } from './errors'
import type { Arch } from './paths'
import { getCachedManifestPath, getInstalledManifestPath } from './paths'

export interface VmArtifact {
  key: string
  sha256: string
  sizeBytes: number
}

export interface VmAgentEntry {
  image: string
  version: string
  tarballs: Record<Arch, VmArtifact>
}

export interface VmManifest {
  schemaVersion: number
  updatedAt: string
  agents: Record<string, VmAgentEntry>
}

export type VersionComparison = 'same' | 'upgrade' | 'downgrade' | 'fresh'

export async function readCachedManifest(
  browserosRoot?: string,
): Promise<VmManifest> {
  const manifestPath = getCachedManifestPath(browserosRoot)
  if (!existsSync(manifestPath)) throw new ManifestMissingError(manifestPath)
  return readManifest(manifestPath)
}

export async function readInstalledManifest(
  browserosRoot?: string,
): Promise<VmManifest | null> {
  const manifestPath = getInstalledManifestPath(browserosRoot)
  if (!existsSync(manifestPath)) return null
  return readManifest(manifestPath)
}

export async function writeInstalledManifest(
  manifest: VmManifest,
  browserosRoot?: string,
): Promise<void> {
  const manifestPath = getInstalledManifestPath(browserosRoot)
  await mkdir(dirname(manifestPath), { recursive: true })
  const tempPath = `${manifestPath}.${process.pid}.${Date.now()}.tmp`
  await writeFile(tempPath, `${JSON.stringify(manifest, null, 2)}\n`)
  await rename(tempPath, manifestPath)
}

export function compareVersions(
  installed: VmManifest | null,
  cached: VmManifest,
): VersionComparison {
  if (!installed) return 'fresh'
  const comparison = compareVersionStrings(
    installed.updatedAt,
    cached.updatedAt,
  )
  if (comparison === 0) return 'same'
  return comparison < 0 ? 'upgrade' : 'downgrade'
}

export function agentForArch(
  manifest: VmManifest,
  name: string,
  arch: Arch,
): {
  image: string
  version: string
  tarball: VmManifest['agents'][string]['tarballs'][Arch]
} {
  const agent = manifest.agents[name]
  if (!agent) throw new Error(`missing agent in VM manifest: ${name}`)
  const tarball = agent.tarballs[arch]
  if (!tarball) throw new Error(`missing ${arch} tarball for agent ${name}`)
  return {
    image: agent.image,
    version: agent.version,
    tarball,
  }
}

async function readManifest(path: string): Promise<VmManifest> {
  return JSON.parse(await readFile(path, 'utf8')) as VmManifest
}

function compareVersionStrings(left: string, right: string): number {
  if (left < right) return -1
  if (left > right) return 1
  return 0
}
