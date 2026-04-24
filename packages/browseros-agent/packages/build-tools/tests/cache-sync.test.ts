import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import {
  type PlanItem,
  planSync,
  readLocalManifest,
  selectSyncArches,
} from '../scripts/cache-sync'
import type { AgentManifest } from '../scripts/common/manifest'
import { sha256File } from '../scripts/common/sha256'
import { buildDevManifest } from '../scripts/seed-dev-agent-tarball'

const openclaw = {
  image: 'ghcr.io/openclaw/openclaw',
  version: '2026.4.12',
}

const claudeCode = {
  image: 'ghcr.io/anthropics/claude-code',
  version: '2026.4.10',
}

function manifest(tarSha: string, includeSecondAgent = false): AgentManifest {
  const agents: AgentManifest['agents'] = {
    openclaw: {
      ...openclaw,
      tarballs: {
        arm64: {
          key: 'vm/images/openclaw-2026.4.12-arm64.tar.gz',
          sha256: `${tarSha}-arm64`,
          sizeBytes: 201,
        },
        x64: {
          key: 'vm/images/openclaw-2026.4.12-x64.tar.gz',
          sha256: `${tarSha}-x64`,
          sizeBytes: 202,
        },
      },
    },
  }

  if (includeSecondAgent) {
    agents['claude-code'] = {
      ...claudeCode,
      tarballs: {
        arm64: {
          key: 'vm/images/claude-code-2026.4.10-arm64.tar.gz',
          sha256: `${tarSha}-claude-arm64`,
          sizeBytes: 301,
        },
        x64: {
          key: 'vm/images/claude-code-2026.4.10-x64.tar.gz',
          sha256: `${tarSha}-claude-x64`,
          sizeBytes: 302,
        },
      },
    }
  }

  return {
    schemaVersion: 2,
    updatedAt: '2026-04-22T00:00:00.000Z',
    agents,
  }
}

function keys(plan: PlanItem[]): string[] {
  return plan.map((item) => item.key)
}

describe('planSync', () => {
  it('downloads every selected-arch agent artifact for a fresh cache', () => {
    const remote = manifest('t1')

    expect(
      keys(planSync({ local: null, remote, cacheRoot: '/c', arches: ['x64'] })),
    ).toEqual(['vm/images/openclaw-2026.4.12-x64.tar.gz'])
  })

  it('does nothing when the local manifest matches the remote manifest', () => {
    const remote = manifest('t1')

    expect(
      planSync({ local: remote, remote, cacheRoot: '/c', arches: ['x64'] }),
    ).toEqual([])
  })

  it('downloads only agent artifacts whose sha256 changed', () => {
    const local = manifest('old-tar')
    const remote = manifest('new-tar')

    expect(
      keys(planSync({ local, remote, cacheRoot: '/c', arches: ['x64'] })),
    ).toEqual(['vm/images/openclaw-2026.4.12-x64.tar.gz'])
  })

  it('supports syncing all release arches', () => {
    const remote = manifest('t1')

    expect(
      planSync({
        local: null,
        remote,
        cacheRoot: '/c',
        arches: ['arm64', 'x64'],
      }),
    ).toHaveLength(2)
  })

  it('selects host arch by default and both arches when requested', () => {
    expect(selectSyncArches(false, 'x64')).toEqual(['x64'])
    expect(selectSyncArches(true, 'x64')).toEqual(['arm64', 'x64'])
  })
})

describe('readLocalManifest', () => {
  let dir: string | null = null

  afterEach(async () => {
    if (!dir) return
    await rm(dir, { recursive: true, force: true })
    dir = null
  })

  it('returns null only when the local manifest is absent', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'browseros-cache-manifest-'))

    await expect(
      readLocalManifest(path.join(dir, 'missing.json')),
    ).resolves.toBeNull()
  })

  it('surfaces corrupt local manifest files', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'browseros-cache-manifest-'))
    const manifestPath = path.join(dir, 'manifest.json')
    await writeFile(manifestPath, '{not json')

    await expect(readLocalManifest(manifestPath)).rejects.toThrow()
  })
})

describe('buildDevManifest', () => {
  it('builds an arm64-only dev manifest from freshly built artifacts', () => {
    const manifest = buildDevManifest(
      [
        {
          agent: {
            name: 'openclaw',
            image: openclaw.image,
            version: openclaw.version,
          },
          key: 'vm/images/openclaw-2026.4.12-arm64.tar.gz',
          path: '/tmp/openclaw.tar.gz',
          sha256: 'fresh-arm64',
          sizeBytes: 404,
        },
      ],
      new Date('2026-04-23T00:00:00.000Z'),
    )

    expect(manifest.schemaVersion).toBe(2)
    expect(manifest.updatedAt).toBe('2026-04-23T00:00:00.000Z')
    expect(manifest.agents.openclaw.image).toBe(openclaw.image)
    expect(manifest.agents.openclaw.version).toBe(openclaw.version)
    expect(manifest.agents.openclaw.tarballs.arm64).toEqual({
      key: 'vm/images/openclaw-2026.4.12-arm64.tar.gz',
      sha256: 'fresh-arm64',
      sizeBytes: 404,
    })
    expect(Object.hasOwn(manifest.agents.openclaw.tarballs, 'x64')).toBe(false)
  })
})

describe('emit-manifest', () => {
  let dir: string | null = null

  afterEach(async () => {
    if (!dir) return
    await rm(dir, { recursive: true, force: true })
    dir = null
  })

  it('rejects the retired vm slice', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'browseros-emit-vm-'))

    const result = await runEmitManifest(
      [
        '--slice',
        'vm',
        '--dist-dir',
        path.join(dir, 'dist'),
        '--out',
        path.join(dir, 'manifest.json'),
      ],
      false,
    )

    expect(result.code).toBe(1)
    expect(result.stderr).toContain('unknown slice: vm')
  })

  it('merges an agent slice while preserving other agents from the baseline', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'browseros-emit-agent-'))
    const distDir = path.join(dir, 'dist')
    await writeAgentFiles(distDir)

    const baseline = manifest('old-tar', true)
    const baselinePath = path.join(dir, 'baseline.json')
    const outPath = path.join(dir, 'manifest.json')
    await writeJson(baselinePath, baseline)

    await runEmitManifest([
      '--slice',
      'agents:openclaw',
      '--dist-dir',
      distDir,
      '--merge-from',
      baselinePath,
      '--out',
      outPath,
    ])

    const merged = JSON.parse(await readFile(outPath, 'utf8')) as AgentManifest
    expect(merged.schemaVersion).toBe(2)
    expect(merged.agents['claude-code']).toEqual(baseline.agents['claude-code'])
    expect(merged.agents.openclaw.tarballs.arm64.sha256).toBe(
      await sha256File(
        path.join(distDir, 'images/openclaw-2026.4.12-arm64.tar.gz'),
      ),
    )
  })

  it('fails slice emission without a merge baseline', async () => {
    dir = await mkdtemp(path.join(tmpdir(), 'browseros-emit-fail-'))

    const result = await runEmitManifest(
      [
        '--slice',
        'agents:openclaw',
        '--dist-dir',
        path.join(dir, 'dist'),
        '--out',
        path.join(dir, 'out.json'),
      ],
      false,
    )

    expect(result.code).toBe(1)
    expect(result.stderr).toContain(
      '--slice agents:openclaw requires --merge-from',
    )
  })
})

async function writeAgentFiles(distDir: string): Promise<void> {
  await mkdir(path.join(distDir, 'images'), { recursive: true })
  await writeFile(
    path.join(distDir, 'images/openclaw-2026.4.12-arm64.tar.gz'),
    'arm tarball',
  )
  await writeFile(
    path.join(distDir, 'images/openclaw-2026.4.12-x64.tar.gz'),
    'x64 tarball',
  )
}

async function writeJson(filePath: string, value: unknown): Promise<void> {
  await writeFile(filePath, `${JSON.stringify(value, null, 2)}\n`)
}

async function runEmitManifest(
  args: string[],
  expectSuccess = true,
): Promise<{ code: number; stdout: string; stderr: string }> {
  const proc = Bun.spawn(
    ['bun', 'run', 'scripts/emit-manifest.ts', '--', ...args],
    {
      cwd: path.join(import.meta.dir, '..'),
      stdout: 'pipe',
      stderr: 'pipe',
    },
  )
  const [stdout, stderr, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ])

  if (expectSuccess && code !== 0) {
    throw new Error(`emit-manifest failed: ${stderr || stdout}`)
  }

  return { code, stdout, stderr }
}
