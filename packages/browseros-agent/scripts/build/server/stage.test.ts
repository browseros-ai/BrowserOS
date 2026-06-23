import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { S3Client } from '@aws-sdk/client-s3'
import { getTargetRules, loadManifest } from './manifest'
import { stageCompiledArtifact, stageTargetArtifact } from './stage'
import { resolveTargets } from './targets'
import type { BuildTarget, R2Config, ResourceRule } from './types'

describe('server artifact staging', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('loads empty local-resource rules from the manifest', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
    const manifestPath = join(tempDir, 'manifest.json')
    await writeFile(manifestPath, JSON.stringify({ resources: [] }))

    expect(loadManifest(manifestPath)).toEqual({
      resources: [],
    })
  })

  it('parses recursive local-resource rules from the manifest', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
    const manifestPath = join(tempDir, 'manifest.json')
    await writeFile(
      manifestPath,
      JSON.stringify({
        resources: [
          {
            name: 'Drizzle migrations',
            source: {
              type: 'local',
              path: 'apps/server/src/lib/db/migrations',
            },
            destination: 'resources/db/migrations',
            recursive: true,
          },
        ],
      }),
    )

    expect(loadManifest(manifestPath).resources[0]).toMatchObject({
      name: 'Drizzle migrations',
      recursive: true,
    })
  })

  it('copies recursive local resource directories', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
    const sourceRoot = join(tempDir, 'source')
    const distRoot = join(tempDir, 'dist')
    const binaryPath = join(tempDir, 'browseros-server')
    const migrationsDir = join(sourceRoot, 'apps/server/src/lib/db/migrations')
    await mkdir(join(migrationsDir, 'meta'), { recursive: true })
    await writeFile(binaryPath, 'server')
    await writeFile(join(migrationsDir, '0000_init.sql'), 'CREATE TABLE x;')
    await writeFile(
      join(migrationsDir, 'meta', '_journal.json'),
      '{"entries":[]}',
    )

    const artifact = await stageCompiledArtifact(
      distRoot,
      binaryPath,
      testTarget,
      '0.0.0-test',
      [migrationRule],
      sourceRoot,
    )

    expect(
      await readFile(
        join(artifact.resourcesDir, 'db/migrations/0000_init.sql'),
        'utf8',
      ),
    ).toBe('CREATE TABLE x;')
    expect(
      await readFile(
        join(artifact.resourcesDir, 'db/migrations/meta/_journal.json'),
        'utf8',
      ),
    ).toBe('{"entries":[]}')
  })

  it('downloads R2 executable resources and marks them executable', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
    const sourceRoot = join(tempDir, 'source')
    const distRoot = join(tempDir, 'dist')
    const binaryPath = join(tempDir, 'browseros-server')
    const payload = new TextEncoder().encode('#!/bin/sh\n')
    await writeFile(binaryPath, 'server')

    const artifact = await stageTargetArtifact(
      distRoot,
      binaryPath,
      testTarget,
      [bunRule],
      sourceRoot,
      {
        send: async () => ({
          Body: {
            transformToByteArray: async () => payload,
          },
        }),
      } as unknown as S3Client,
      fakeR2Config,
      '0.0.0-test',
    )

    const bunPath = join(artifact.resourcesDir, 'bin/third_party/bun')
    expect(await readFile(bunPath, 'utf8')).toBe('#!/bin/sh\n')
    expect((await stat(bunPath)).mode & 0o111).not.toBe(0)
  })

  for (const { target, expectedKey, expectedRelativePath } of [
    {
      target: linuxArm64Target,
      expectedKey: 'third_party/bun/bun-linux-arm64',
      expectedRelativePath: 'bin/third_party/bun',
    },
    {
      target: linuxX64Target,
      expectedKey: 'third_party/bun/bun-linux-x64-baseline',
      expectedRelativePath: 'bin/third_party/bun',
    },
    {
      target: windowsX64Target,
      expectedKey: 'third_party/bun/bun-windows-x64-baseline.exe',
      expectedRelativePath: 'bin/third_party/bun.exe',
    },
  ]) {
    it(`stages the platform Bun resource for ${target.id}`, async () => {
      tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
      const sourceRoot = join(tempDir, 'source')
      const distRoot = join(tempDir, 'dist')
      const binaryPath = join(tempDir, target.serverBinaryName)
      const requests: string[] = []
      const payload = new TextEncoder().encode(`${target.id}-bun`)
      await writeFile(binaryPath, 'server')

      const rules = bunRulesForTarget(target)
      expect(rules).toHaveLength(1)
      expect(rules[0]?.source).toEqual({
        type: 'r2',
        key: expectedKey,
      })

      const artifact = await stageTargetArtifact(
        distRoot,
        binaryPath,
        target,
        rules,
        sourceRoot,
        {
          send: async (command: unknown) => {
            requests.push((command as { input: { Key: string } }).input.Key)
            return {
              Body: {
                transformToByteArray: async () => payload,
              },
            }
          },
        } as unknown as S3Client,
        fakeR2Config,
        '0.0.0-test',
      )

      expect(requests).toEqual([`artifacts/vendor/${expectedKey}`])
      expect(
        await readFile(
          join(artifact.resourcesDir, expectedRelativePath),
          'utf8',
        ),
      ).toBe(`${target.id}-bun`)
    })
  }

  it('does not package VM-only resources in the production manifest', () => {
    const manifest = loadManifest(
      'scripts/build/config/server-prod-resources.json',
    )
    const destinations = manifest.resources.map((rule) => rule.destination)

    expect(
      destinations.filter(
        (destination) =>
          destination.includes('third_party/lima') ||
          destination.startsWith('resources/vm/'),
      ),
    ).toEqual([])
    expect(destinations).toContain('resources/bin/third_party/bun')
    expect(destinations).toContain('resources/db/migrations')
  })

  it('selects Drizzle migrations for every production server target', () => {
    const manifest = loadManifest(
      'scripts/build/config/server-prod-resources.json',
    )

    for (const target of resolveTargets('all')) {
      const migrationRules = getTargetRules(manifest, target).filter(
        (rule) => rule.name === 'Drizzle migrations',
      )

      expect(migrationRules).toEqual([
        expect.objectContaining({
          destination: 'resources/db/migrations',
          recursive: true,
        }),
      ])
    }
  })

  it('downloads bundled Codex CLI for Linux targets and marks it executable', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
    const sourceRoot = join(tempDir, 'source')
    const distRoot = join(tempDir, 'dist')
    const binaryPath = join(tempDir, 'browseros-server')
    await writeFile(binaryPath, 'server')

    const artifact = await stageTargetArtifact(
      distRoot,
      binaryPath,
      linuxArm64Target,
      [codexLinuxArm64Rule],
      sourceRoot,
      fakeObjectClient({
        'artifacts/vendor/third_party/codex/codex-linux-arm64':
          '#!/bin/sh\ncodex\n',
      }),
      fakeR2Config,
      '0.0.0-test',
    )

    const codexPath = join(artifact.resourcesDir, 'bin/third_party/codex')
    expect(await readFile(codexPath, 'utf8')).toBe('#!/bin/sh\ncodex\n')
    expect((await stat(codexPath)).mode & 0o111).not.toBe(0)
  })

  it('downloads bundled Codex CLI for Windows targets without chmod', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'browseros-stage-test-'))
    const sourceRoot = join(tempDir, 'source')
    const distRoot = join(tempDir, 'dist')
    const binaryPath = join(tempDir, 'browseros-server.exe')
    await writeFile(binaryPath, 'server')

    const artifact = await stageTargetArtifact(
      distRoot,
      binaryPath,
      windowsX64Target,
      [codexWindowsX64Rule],
      sourceRoot,
      fakeObjectClient({
        'artifacts/vendor/third_party/codex/codex-windows-x64.exe': 'codex.exe',
      }),
      fakeR2Config,
      '0.0.0-test',
    )

    const codexPath = join(artifact.resourcesDir, 'bin/third_party/codex.exe')
    expect(await readFile(codexPath, 'utf8')).toBe('codex.exe')
    expect((await stat(codexPath)).mode & 0o111).toBe(0)
  })

  it('loads target-filtered bundled CLI rules from the production manifest', () => {
    const manifest = loadManifest(
      'scripts/build/config/server-prod-resources.json',
    )
    const linuxRules = getTargetRules(manifest, linuxArm64Target)
    const windowsRules = getTargetRules(manifest, windowsX64Target)
    const claudeRules = manifest.resources.filter(
      (rule) =>
        rule.name.includes('Claude Code') ||
        rule.destination.includes('third_party/claude') ||
        (rule.source.type === 'r2' && rule.source.key.includes('claude-code')),
    )

    expect(linuxRules).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          name: 'Codex - Linux ARM64',
          source: {
            type: 'r2',
            key: 'third_party/codex/codex-linux-arm64',
          },
          destination: 'resources/bin/third_party/codex',
          executable: true,
        }),
      ]),
    )
    expect(windowsRules).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          name: 'Codex - Windows x64',
          source: {
            type: 'r2',
            key: 'third_party/codex/codex-windows-x64.exe',
          },
          destination: 'resources/bin/third_party/codex.exe',
          executable: true,
        }),
      ]),
    )
    expect(claudeRules).toEqual([])
    expect(linuxRules.find((rule) => rule.name === 'Codex - Windows x64')).toBe(
      undefined,
    )
  })
})

const testTarget: BuildTarget = {
  id: 'darwin-arm64',
  name: 'macOS ARM64',
  os: 'macos',
  arch: 'arm64',
  bunTarget: 'bun-darwin-arm64',
  serverBinaryName: 'browseros-server',
}

const linuxArm64Target: BuildTarget = {
  id: 'linux-arm64',
  name: 'Linux ARM64',
  os: 'linux',
  arch: 'arm64',
  bunTarget: 'bun-linux-arm64',
  serverBinaryName: 'browseros_server',
}

const linuxX64Target: BuildTarget = {
  id: 'linux-x64',
  name: 'Linux x64',
  os: 'linux',
  arch: 'x64',
  bunTarget: 'bun-linux-x64-baseline',
  serverBinaryName: 'browseros_server',
}

const windowsX64Target: BuildTarget = {
  id: 'windows-x64',
  name: 'Windows x64',
  os: 'windows',
  arch: 'x64',
  bunTarget: 'bun-windows-x64-baseline',
  serverBinaryName: 'browseros_server.exe',
}

function bunRulesForTarget(target: BuildTarget): ResourceRule[] {
  return getTargetRules(
    loadManifest(join(import.meta.dir, '../config/server-prod-resources.json')),
    target,
  ).filter(
    (rule) => rule.source.type === 'r2' && rule.name.startsWith('Bun - '),
  )
}

const migrationRule: ResourceRule = {
  name: 'Drizzle migrations',
  source: {
    type: 'local',
    path: 'apps/server/src/lib/db/migrations',
  },
  destination: 'resources/db/migrations',
  recursive: true,
}

const bunRule: ResourceRule = {
  name: 'Bun - macOS ARM64',
  source: {
    type: 'r2',
    key: 'third_party/bun/bun-darwin-arm64',
  },
  destination: 'resources/bin/third_party/bun',
  os: ['macos'],
  arch: ['arm64'],
  executable: true,
}

const codexLinuxArm64Rule: ResourceRule = {
  name: 'Codex - Linux ARM64',
  source: {
    type: 'r2',
    key: 'third_party/codex/codex-linux-arm64',
  },
  destination: 'resources/bin/third_party/codex',
  os: ['linux'],
  arch: ['arm64'],
  executable: true,
}

const codexWindowsX64Rule: ResourceRule = {
  name: 'Codex - Windows x64',
  source: {
    type: 'r2',
    key: 'third_party/codex/codex-windows-x64.exe',
  },
  destination: 'resources/bin/third_party/codex.exe',
  os: ['windows'],
  arch: ['x64'],
  executable: true,
}

const fakeR2Config: R2Config = {
  accountId: 'test',
  accessKeyId: 'test',
  secretAccessKey: 'test',
  bucket: 'browseros-test',
  downloadPrefix: 'artifacts/vendor',
  uploadPrefix: 'server/prod-resources',
}

function fakeObjectClient(objects: Record<string, string>): S3Client {
  return {
    send: async (command: { input?: { Key?: string } }) => {
      const key = command.input?.Key
      const payload = key ? objects[key] : undefined
      if (payload === undefined) {
        throw new Error(`Unexpected R2 object: ${String(key)}`)
      }
      return {
        Body: {
          transformToByteArray: async () => new TextEncoder().encode(payload),
        },
      }
    },
  } as unknown as S3Client
}
