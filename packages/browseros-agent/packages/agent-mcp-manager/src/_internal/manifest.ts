import * as fsp from 'node:fs/promises'
import * as path from 'node:path'

import { McpManagerError } from '../errors'
import type { ServerManifest } from '../types'

export function emptyManifest(): ServerManifest {
  return { version: 1, servers: {} }
}

function manifestPath(workspaceDir: string): string {
  return path.join(workspaceDir, 'manifest.json')
}

/**
 * Read the manifest. Returns an empty manifest only when the file is
 * absent or empty — malformed JSON, wrong shape, or wrong version
 * throws so callers can recover instead of silently overwriting their
 * existing state.
 */
export async function readManifest(
  workspaceDir: string,
): Promise<ServerManifest> {
  const file = manifestPath(workspaceDir)
  let raw: string
  try {
    raw = await fsp.readFile(file, 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return emptyManifest()
    throw err
  }
  if (!raw.trim()) return emptyManifest()
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch (err) {
    throw new McpManagerError(
      `Manifest at ${file} is not valid JSON. Inspect and repair or delete to start fresh.`,
      { cause: err },
    )
  }
  if (!parsed || typeof parsed !== 'object') {
    throw new McpManagerError(`Manifest at ${file} is not an object.`)
  }
  const candidate = parsed as Partial<ServerManifest>
  if (candidate.version !== 1) {
    throw new McpManagerError(
      `Manifest at ${file} has unsupported version ${String(candidate.version)}; expected 1.`,
    )
  }
  if (typeof candidate.servers !== 'object' || candidate.servers === null) {
    throw new McpManagerError(
      `Manifest at ${file} is missing a valid \`servers\` object.`,
    )
  }
  return parsed as ServerManifest
}

export async function writeManifest(
  workspaceDir: string,
  manifest: ServerManifest,
): Promise<void> {
  const file = manifestPath(workspaceDir)
  await fsp.mkdir(workspaceDir, { recursive: true })
  const tmp = `${file}.tmp-${process.pid}-${Date.now()}`
  await fsp.writeFile(tmp, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
  await fsp.rename(tmp, file)
}
