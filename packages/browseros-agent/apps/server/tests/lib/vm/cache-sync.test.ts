/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { createHash } from 'node:crypto'
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import {
  ensureVmCacheAvailable,
  ensureVmCacheSynced,
  prefetchVmCache,
} from '../../../src/lib/vm/cache-sync'
import type { VmManifest } from '../../../src/lib/vm/manifest'
import { getCachedManifestPath } from '../../../src/lib/vm/paths'

const CDN_BASE = 'https://cdn.test'
const MANIFEST_URL = `${CDN_BASE}/vm/manifest.json`
const TARBALL_KEY = 'vm/images/openclaw-2026.4.12-arm64.tar.gz'
const TARBALL_BYTES = new TextEncoder().encode('openclaw-tarball')
const TARBALL_SHA = sha256(TARBALL_BYTES)

const manifest: VmManifest = {
  schemaVersion: 2,
  updatedAt: '2026-04-24T00:00:00.000Z',
  agents: {
    openclaw: {
      image: 'ghcr.io/openclaw/openclaw',
      version: '2026.4.12',
      tarballs: {
        arm64: {
          key: TARBALL_KEY,
          sha256: TARBALL_SHA,
          sizeBytes: TARBALL_BYTES.byteLength,
        },
        x64: {
          key: 'vm/images/openclaw-2026.4.12-x64.tar.gz',
          sha256: 'unused',
          sizeBytes: 1,
        },
      },
    },
  },
}

describe('runtime VM cache sync', () => {
  let root: string
  let originalManifestUrl: string | undefined

  beforeEach(async () => {
    root = await mkdtemp('/tmp/browseros-vm-cache-sync-')
    originalManifestUrl = process.env.BROWSEROS_VM_CACHE_MANIFEST_URL
    delete process.env.BROWSEROS_VM_CACHE_MANIFEST_URL
  })

  afterEach(async () => {
    restoreEnv('BROWSEROS_VM_CACHE_MANIFEST_URL', originalManifestUrl)
    await rm(root, { recursive: true, force: true })
  })

  it('downloads the host-arch tarball, verifies it, and writes the manifest last', async () => {
    const calls: string[] = []
    const fetchImpl = fakeVmCacheFetch(calls)

    const result = await ensureVmCacheSynced({
      browserosRoot: root,
      manifestUrl: MANIFEST_URL,
      fetchImpl,
      rawHostArch: 'arm64',
    })

    expect(calls).toEqual([MANIFEST_URL, `${CDN_BASE}/${TARBALL_KEY}`])
    expect(result).toEqual({
      downloaded: [TARBALL_KEY],
      manifestPath: getCachedManifestPath(root),
      skipped: false,
    })
    expect(
      JSON.parse(await readFile(getCachedManifestPath(root), 'utf8')),
    ).toEqual(manifest)
    expect(await readFile(join(root, 'cache', TARBALL_KEY), 'utf8')).toBe(
      'openclaw-tarball',
    )
    await expect(
      stat(join(root, 'cache', `${TARBALL_KEY}.partial`)),
    ).rejects.toThrow()
  })

  it('uses the runtime env manifest URL and resolves artifacts beside it', async () => {
    process.env.BROWSEROS_VM_CACHE_MANIFEST_URL =
      'https://artifacts.test/vm/manifest.json'
    const calls: string[] = []
    const fetchImpl = fakeVmCacheFetch(calls, {
      manifestUrl: 'https://artifacts.test/vm/manifest.json',
      tarballUrl: `https://artifacts.test/${TARBALL_KEY}`,
    })

    await ensureVmCacheSynced({
      browserosRoot: root,
      fetchImpl,
      rawHostArch: 'arm64',
    })

    expect(calls).toEqual([
      'https://artifacts.test/vm/manifest.json',
      `https://artifacts.test/${TARBALL_KEY}`,
    ])
  })

  it('skips downloads when the matching manifest and tarball already exist', async () => {
    await writeLocalManifest(root)
    await writeLocalTarball(root)
    const calls: string[] = []

    const result = await ensureVmCacheSynced({
      browserosRoot: root,
      manifestUrl: MANIFEST_URL,
      fetchImpl: fakeVmCacheFetch(calls),
      rawHostArch: 'arm64',
    })

    expect(calls).toEqual([MANIFEST_URL])
    expect(result.downloaded).toEqual([])
    expect(result.skipped).toBe(true)
  })

  it('downloads a tarball when the manifest matches but the file is missing', async () => {
    await writeLocalManifest(root)
    const calls: string[] = []

    const result = await ensureVmCacheSynced({
      browserosRoot: root,
      manifestUrl: MANIFEST_URL,
      fetchImpl: fakeVmCacheFetch(calls),
      rawHostArch: 'arm64',
    })

    expect(calls).toEqual([MANIFEST_URL, `${CDN_BASE}/${TARBALL_KEY}`])
    expect(result.downloaded).toEqual([TARBALL_KEY])
    expect(await readFile(join(root, 'cache', TARBALL_KEY), 'utf8')).toBe(
      'openclaw-tarball',
    )
  })

  it('uses an existing tarball when the local manifest is missing but the hash matches', async () => {
    await writeLocalTarball(root)
    const calls: string[] = []

    const result = await ensureVmCacheSynced({
      browserosRoot: root,
      manifestUrl: MANIFEST_URL,
      fetchImpl: fakeVmCacheFetch(calls),
      rawHostArch: 'arm64',
    })

    expect(calls).toEqual([MANIFEST_URL])
    expect(result.downloaded).toEqual([])
    expect(result.skipped).toBe(true)
    await expect(readFile(getCachedManifestPath(root), 'utf8')).resolves.toBe(
      `${JSON.stringify(manifest, null, 2)}\n`,
    )
  })

  it('shares concurrent prefetch calls through one in-flight sync', async () => {
    const calls: string[] = []
    let resolveManifest: (response: Response) => void = () => {}
    const manifestResponse = new Promise<Response>((resolve) => {
      resolveManifest = resolve
    })
    const fetchImpl = async (input: RequestInfo | URL): Promise<Response> => {
      const url = String(input)
      calls.push(url)
      if (url === MANIFEST_URL) return manifestResponse
      if (url === `${CDN_BASE}/${TARBALL_KEY}`)
        return new Response(TARBALL_BYTES)
      return new Response('', { status: 404 })
    }

    const first = prefetchVmCache({
      browserosRoot: root,
      manifestUrl: MANIFEST_URL,
      fetchImpl,
      rawHostArch: 'arm64',
    })
    const second = prefetchVmCache({
      browserosRoot: root,
      manifestUrl: MANIFEST_URL,
      fetchImpl,
      rawHostArch: 'arm64',
    })

    expect(second).toBe(first)
    expect(calls).toEqual([MANIFEST_URL])

    resolveManifest(jsonResponse(manifest))

    await expect(first).resolves.toEqual({
      downloaded: [TARBALL_KEY],
      manifestPath: getCachedManifestPath(root),
      skipped: false,
    })
    await expect(second).resolves.toEqual({
      downloaded: [TARBALL_KEY],
      manifestPath: getCachedManifestPath(root),
      skipped: false,
    })
    expect(calls).toEqual([MANIFEST_URL, `${CDN_BASE}/${TARBALL_KEY}`])
  })

  it('syncs different roots independently while another sync is in flight', async () => {
    const otherRoot = await mkdtemp('/tmp/browseros-vm-cache-sync-other-')
    try {
      const calls: string[] = []
      let resolveManifest: (response: Response) => void = () => {}
      const manifestResponse = new Promise<Response>((resolve) => {
        resolveManifest = resolve
      })
      const fetchImpl = async (input: RequestInfo | URL): Promise<Response> => {
        const url = String(input)
        calls.push(url)
        if (calls.length === 1 && url === MANIFEST_URL) return manifestResponse
        if (url === MANIFEST_URL) return jsonResponse(manifest)
        if (url === `${CDN_BASE}/${TARBALL_KEY}`)
          return new Response(TARBALL_BYTES)
        return new Response('', { status: 404 })
      }

      const first = prefetchVmCache({
        browserosRoot: otherRoot,
        manifestUrl: MANIFEST_URL,
        fetchImpl,
        rawHostArch: 'arm64',
      })
      const second = ensureVmCacheSynced({
        browserosRoot: root,
        manifestUrl: MANIFEST_URL,
        fetchImpl,
        rawHostArch: 'arm64',
      })

      expect(second).not.toBe(first)
      await second

      resolveManifest(jsonResponse(manifest))
      await first

      await expect(readFile(getCachedManifestPath(root), 'utf8')).resolves.toBe(
        `${JSON.stringify(manifest, null, 2)}\n`,
      )
      await expect(
        readFile(getCachedManifestPath(otherRoot), 'utf8'),
      ).resolves.toBe(`${JSON.stringify(manifest, null, 2)}\n`)
      expect(calls).toEqual([
        MANIFEST_URL,
        MANIFEST_URL,
        `${CDN_BASE}/${TARBALL_KEY}`,
        `${CDN_BASE}/${TARBALL_KEY}`,
      ])
    } finally {
      await rm(otherRoot, { recursive: true, force: true })
    }
  })

  it('retries on-demand availability after an in-flight prefetch fails', async () => {
    const calls: string[] = []
    let resolveManifest: (response: Response) => void = () => {}
    const manifestResponse = new Promise<Response>((resolve) => {
      resolveManifest = resolve
    })
    const fetchImpl = async (input: RequestInfo | URL): Promise<Response> => {
      const url = String(input)
      calls.push(url)
      if (calls.length === 1 && url === MANIFEST_URL) return manifestResponse
      if (url === MANIFEST_URL) return jsonResponse(manifest)
      if (url === `${CDN_BASE}/${TARBALL_KEY}`)
        return new Response(TARBALL_BYTES)
      return new Response('', { status: 404 })
    }

    const first = prefetchVmCache({
      browserosRoot: root,
      manifestUrl: MANIFEST_URL,
      fetchImpl,
      rawHostArch: 'arm64',
    }).catch((error) => error)
    const available = ensureVmCacheAvailable({
      browserosRoot: root,
      manifestUrl: MANIFEST_URL,
      fetchImpl,
      rawHostArch: 'arm64',
    })

    resolveManifest(new Response('', { status: 503 }))

    await expect(first).resolves.toBeInstanceOf(Error)
    await available
    await expect(readFile(getCachedManifestPath(root), 'utf8')).resolves.toBe(
      `${JSON.stringify(manifest, null, 2)}\n`,
    )
    expect(calls).toEqual([
      MANIFEST_URL,
      MANIFEST_URL,
      `${CDN_BASE}/${TARBALL_KEY}`,
    ])
  })

  it('clears failed in-flight syncs so a later call can retry', async () => {
    const calls: string[] = []
    const fetchImpl = async (input: RequestInfo | URL): Promise<Response> => {
      const url = String(input)
      calls.push(url)
      if (calls.length === 1) return new Response('', { status: 503 })
      if (url === MANIFEST_URL) return jsonResponse(manifest)
      if (url === `${CDN_BASE}/${TARBALL_KEY}`)
        return new Response(TARBALL_BYTES)
      return new Response('', { status: 404 })
    }

    await expect(
      ensureVmCacheSynced({
        browserosRoot: root,
        manifestUrl: MANIFEST_URL,
        fetchImpl,
        rawHostArch: 'arm64',
      }),
    ).rejects.toThrow('manifest fetch failed')

    await expect(
      ensureVmCacheSynced({
        browserosRoot: root,
        manifestUrl: MANIFEST_URL,
        fetchImpl,
        rawHostArch: 'arm64',
      }),
    ).resolves.toEqual({
      downloaded: [TARBALL_KEY],
      manifestPath: getCachedManifestPath(root),
      skipped: false,
    })
    expect(calls).toEqual([
      MANIFEST_URL,
      MANIFEST_URL,
      `${CDN_BASE}/${TARBALL_KEY}`,
    ])
  })

  it('removes the partial file when sha256 verification fails', async () => {
    const badBytes = new TextEncoder().encode('bad-tarball')
    const fetchImpl = (async (input: RequestInfo | URL): Promise<Response> => {
      const url = String(input)
      if (url === MANIFEST_URL) return jsonResponse(manifest)
      if (url === `${CDN_BASE}/${TARBALL_KEY}`) return new Response(badBytes)
      return new Response('', { status: 404 })
    }) as typeof fetch

    await expect(
      ensureVmCacheSynced({
        browserosRoot: root,
        manifestUrl: MANIFEST_URL,
        fetchImpl,
        rawHostArch: 'arm64',
      }),
    ).rejects.toThrow('sha256 mismatch')

    await expect(stat(join(root, 'cache', TARBALL_KEY))).rejects.toThrow()
    await expect(
      stat(join(root, 'cache', `${TARBALL_KEY}.partial`)),
    ).rejects.toThrow()
  })

  it('rejects unsupported host architectures before fetching', async () => {
    const calls: string[] = []

    await expect(
      ensureVmCacheSynced({
        browserosRoot: root,
        manifestUrl: MANIFEST_URL,
        fetchImpl: fakeVmCacheFetch(calls),
        rawHostArch: 'arm',
      }),
    ).rejects.toThrow('unsupported host arch: arm')

    expect(calls).toEqual([])
  })
})

function fakeVmCacheFetch(
  calls: string[],
  opts?: { manifestUrl?: string; tarballUrl?: string },
): typeof fetch {
  const manifestUrl = opts?.manifestUrl ?? MANIFEST_URL
  const tarballUrl = opts?.tarballUrl ?? `${CDN_BASE}/${TARBALL_KEY}`
  return (async (input: RequestInfo | URL): Promise<Response> => {
    const url = String(input)
    calls.push(url)
    if (url === manifestUrl) return jsonResponse(manifest)
    if (url === tarballUrl) return new Response(TARBALL_BYTES)
    return new Response('', { status: 404 })
  }) as typeof fetch
}

function jsonResponse(value: unknown): Response {
  return new Response(JSON.stringify(value), {
    headers: { 'content-type': 'application/json' },
  })
}

async function writeLocalManifest(root: string): Promise<void> {
  const path = getCachedManifestPath(root)
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, `${JSON.stringify(manifest, null, 2)}\n`)
}

async function writeLocalTarball(root: string): Promise<void> {
  const path = join(root, 'cache', TARBALL_KEY)
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, TARBALL_BYTES)
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex')
}

function restoreEnv(key: string, value: string | undefined): void {
  if (value === undefined) {
    delete process.env[key]
  } else {
    process.env[key] = value
  }
}
