/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createHash } from 'node:crypto'
import { createReadStream, existsSync } from 'node:fs'
import { mkdir, readFile, rename, rm } from 'node:fs/promises'
import { arch as hostArch } from 'node:os'
import { dirname, join } from 'node:path'
import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import type { VmArtifact, VmManifest } from './manifest'
import type { Arch } from './paths'
import { getCachedManifestPath } from './paths'

const DEFAULT_TIMEOUT_MS = 30_000
const ARCHES: Arch[] = ['arm64', 'x64']
const CANONICAL_MANIFEST_SUFFIX = '/vm/manifest.json'

export interface VmCacheSyncOptions {
  browserosRoot?: string
  manifestUrl?: string
  allArches?: boolean
  fetchImpl?: typeof fetch
  rawHostArch?: NodeJS.Architecture
  timeoutMs?: number
}

export interface VmCacheSyncResult {
  downloaded: string[]
  manifestPath: string
  skipped: boolean
}

const inFlight = new Map<string, Promise<VmCacheSyncResult>>()

export function prefetchVmCache(
  options: VmCacheSyncOptions = {},
): Promise<VmCacheSyncResult> {
  return startOrReuseSync(options)
}

export function ensureVmCacheSynced(
  options: VmCacheSyncOptions = {},
): Promise<VmCacheSyncResult> {
  return startOrReuseSync(options)
}

export async function ensureVmCacheAvailable(
  options: VmCacheSyncOptions = {},
): Promise<void> {
  const cfg = resolveSyncConfig(options)
  const pending = inFlight.get(syncKey(cfg))
  if (pending) {
    await pending.catch(() => {})
  }

  if (existsSync(getCachedManifestPath(cfg.browserosRoot))) return

  await startOrReuseSyncWithConfig(cfg)
}

function startOrReuseSync(
  options: VmCacheSyncOptions,
): Promise<VmCacheSyncResult> {
  try {
    return startOrReuseSyncWithConfig(resolveSyncConfig(options))
  } catch (error) {
    return Promise.reject(error)
  }
}

function startOrReuseSyncWithConfig(
  cfg: SyncConfig,
): Promise<VmCacheSyncResult> {
  const key = syncKey(cfg)
  const existing = inFlight.get(key)
  if (existing) return existing
  const current = syncVmCache(cfg).finally(() => {
    if (inFlight.get(key) === current) inFlight.delete(key)
  })
  inFlight.set(key, current)
  return current
}

async function syncVmCache(cfg: SyncConfig): Promise<VmCacheSyncResult> {
  const remote = await fetchManifest(cfg)
  const manifestPath = getCachedManifestPath(cfg.browserosRoot)
  const local = await readLocalManifest(manifestPath)
  const plan = await planDownloads({
    remote,
    local,
    cacheRoot: cacheRootForManifest(manifestPath),
    arches: cfg.arches,
  })

  for (const item of plan) {
    await downloadArtifact(
      cfg.fetchImpl,
      artifactUrlForKey(cfg.manifestUrl, item.key),
      item.destPath,
      item.sha256,
      cfg.timeoutMs,
    )
  }

  await mkdir(dirname(manifestPath), { recursive: true })
  const tempPath = `${manifestPath}.${process.pid}.${Date.now()}.tmp`
  await Bun.write(tempPath, `${JSON.stringify(remote, null, 2)}\n`)
  await rename(tempPath, manifestPath)

  return {
    downloaded: plan.map((item) => item.key),
    manifestPath,
    skipped: plan.length === 0,
  }
}

interface SyncConfig {
  browserosRoot?: string
  manifestUrl: string
  fetchImpl: typeof fetch
  arches: Arch[]
  timeoutMs: number
}

function resolveSyncConfig(options: VmCacheSyncOptions): SyncConfig {
  return {
    browserosRoot: options.browserosRoot,
    manifestUrl:
      trimNonEmpty(options.manifestUrl) ??
      trimNonEmpty(process.env.BROWSEROS_VM_CACHE_MANIFEST_URL) ??
      EXTERNAL_URLS.VM_CACHE_MANIFEST,
    fetchImpl: options.fetchImpl ?? fetch,
    arches: selectSyncArches(options),
    timeoutMs: options.timeoutMs ?? DEFAULT_TIMEOUT_MS,
  }
}

async function fetchManifest(cfg: SyncConfig): Promise<VmManifest> {
  const response = await fetchWithTimeout(
    cfg.fetchImpl,
    cfg.manifestUrl,
    cfg.timeoutMs,
  )
  if (!response.ok) {
    throw new Error(
      `manifest fetch failed: ${cfg.manifestUrl} (${response.status})`,
    )
  }
  return (await response.json()) as VmManifest
}

interface DownloadPlanItem {
  key: string
  destPath: string
  sha256: string
}

async function planDownloads(opts: {
  remote: VmManifest
  local: VmManifest | null
  cacheRoot: string
  arches: Arch[]
}): Promise<DownloadPlanItem[]> {
  const out: DownloadPlanItem[] = []
  for (const arch of opts.arches) {
    for (const [name, agent] of Object.entries(opts.remote.agents)) {
      const remote = agent.tarballs[arch]
      if (!remote) continue
      const destPath = join(opts.cacheRoot, remote.key)
      if (
        !(await needsDownload(
          remote,
          opts.local?.agents[name]?.tarballs[arch],
          destPath,
        ))
      ) {
        continue
      }
      out.push({ key: remote.key, destPath, sha256: remote.sha256 })
    }
  }
  return out
}

async function needsDownload(
  remote: VmArtifact,
  local: VmArtifact | undefined,
  destPath: string,
): Promise<boolean> {
  if (!existsSync(destPath)) return true
  if (local?.sha256 === remote.sha256) return false
  try {
    return (await sha256File(destPath)) !== remote.sha256
  } catch {
    return true
  }
}

async function downloadArtifact(
  fetchImpl: typeof fetch,
  url: string,
  destPath: string,
  sha256: string,
  timeoutMs: number,
): Promise<void> {
  const partialPath = `${destPath}.partial`
  await mkdir(dirname(destPath), { recursive: true })
  await rm(partialPath, { force: true })

  try {
    const response = await fetchWithTimeout(fetchImpl, url, timeoutMs)
    if (!response.ok || !response.body) {
      throw new Error(`download failed: ${url} (${response.status})`)
    }

    const sink = Bun.file(partialPath).writer()
    const reader = response.body.getReader()
    try {
      for (;;) {
        const { done, value } = await reader.read()
        if (done) break
        sink.write(value)
      }
    } finally {
      await sink.end()
    }

    await verifySha256(partialPath, sha256)
    await rename(partialPath, destPath)
  } catch (error) {
    await rm(partialPath, { force: true })
    throw error
  }
}

async function fetchWithTimeout(
  fetchImpl: typeof fetch,
  url: string,
  timeoutMs: number,
): Promise<Response> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  try {
    return await fetchImpl(url, { signal: controller.signal })
  } catch (error) {
    if ((error as { name?: string }).name === 'AbortError') {
      throw new Error(`fetch timed out after ${timeoutMs}ms: ${url}`)
    }
    throw error
  } finally {
    clearTimeout(timer)
  }
}

async function verifySha256(path: string, expected: string): Promise<void> {
  const actual = await sha256File(path)
  if (actual !== expected) {
    throw new Error(
      `sha256 mismatch for ${path}: expected ${expected}, got ${actual}`,
    )
  }
}

async function sha256File(path: string): Promise<string> {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk)
  }
  return hash.digest('hex')
}

async function readLocalManifest(path: string): Promise<VmManifest | null> {
  try {
    return JSON.parse(await readFile(path, 'utf8')) as VmManifest
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw error
  }
}

function selectSyncArches(options: VmCacheSyncOptions): Arch[] {
  if (options.allArches) return [...ARCHES]
  const rawArch = options.rawHostArch ?? hostArch()
  if (rawArch === 'arm64') return ['arm64']
  if (rawArch === 'x64' || rawArch === 'ia32') return ['x64']
  throw new Error(`unsupported host arch: ${rawArch}`)
}

function cacheRootForManifest(manifestPath: string): string {
  return dirname(dirname(manifestPath))
}

function syncKey(cfg: SyncConfig): string {
  return [
    getCachedManifestPath(cfg.browserosRoot),
    cfg.manifestUrl,
    cfg.arches.join(','),
    String(cfg.timeoutMs),
  ].join('\0')
}

function artifactUrlForKey(manifestUrl: string, key: string): string {
  const artifactKey = key.replace(/^\/+/, '')
  const url = new URL(manifestUrl)
  const normalizedPath = url.pathname.replace(/\/+$/, '')
  const prefix = normalizedPath.endsWith(CANONICAL_MANIFEST_SUFFIX)
    ? normalizedPath.slice(0, -CANONICAL_MANIFEST_SUFFIX.length)
    : normalizedPath.slice(0, Math.max(0, normalizedPath.lastIndexOf('/')))

  url.pathname = `${prefix.replace(/\/+$/, '')}/${artifactKey}`
  url.search = ''
  url.hash = ''
  return url.toString()
}

function trimNonEmpty(value: string | undefined): string | undefined {
  const trimmed = value?.trim()
  return trimmed ? trimmed : undefined
}
