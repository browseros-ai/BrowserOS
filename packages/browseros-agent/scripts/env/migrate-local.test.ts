import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { existsSync, readFileSync } from 'node:fs'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import { buildLocalEnvMigration, migrateLocalEnv } from './migrate-local'

describe('migrateLocalEnv', () => {
  let tempRoot: string | null = null

  beforeEach(async () => {
    tempRoot = await mkdtemp(join(tmpdir(), 'root-env-migrate-'))
  })

  afterEach(async () => {
    if (tempRoot) {
      await rm(tempRoot, { recursive: true, force: true })
      tempRoot = null
    }
  })

  test('merges known non-empty values and reports dropped keys', async () => {
    const rootDir = requireTempRoot(tempRoot)
    await writeFixture(rootDir, 'apps/server/.env.development', {
      POSTHOG_API_KEY: 'server-key',
      R2_UPLOAD_PREFIX: 'retired-prefix',
    })
    await writeFixture(rootDir, 'apps/app/.env.development', {
      VITE_ALPHA_FEATURES: 'false',
      UNKNOWN_LOCAL_KEY: 'drop-me',
    })

    const result = migrateLocalEnv({ rootDir })
    const development = readFileSync(join(rootDir, '.env.development'), 'utf8')

    expect(result.wrote.map((path) => path.replace(`${rootDir}/`, ''))).toEqual(
      ['.env.development', '.env.production'],
    )
    expect(development).toContain('POSTHOG_API_KEY=server-key')
    expect(development).toContain('VITE_ALPHA_FEATURES=false')
    expect(result.dropped).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          key: 'R2_UPLOAD_PREFIX',
          reason: 'retired to code constants',
        }),
        expect.objectContaining({
          key: 'UNKNOWN_LOCAL_KEY',
          reason: 'not in the development registry',
        }),
      ]),
    )
  })

  test('keeps the first non-empty value and reports conflicts', async () => {
    const rootDir = requireTempRoot(tempRoot)
    await writeFixture(rootDir, 'apps/server/.env.development', {
      LOG_LEVEL: 'debug',
    })
    await writeFixture(rootDir, 'apps/app/.env.development', {
      LOG_LEVEL: 'warn',
    })

    const plan = buildLocalEnvMigration(rootDir)
    const development = plan.files.find((file) => file.mode === 'development')

    expect(development?.content).toContain('LOG_LEVEL=debug')
    expect(development?.content).not.toContain('LOG_LEVEL=warn')
    expect(plan.conflicts).toEqual([
      expect.objectContaining({
        key: 'LOG_LEVEL',
        keptSourceFile: join(rootDir, 'apps/server/.env.development'),
        conflictingSourceFile: join(rootDir, 'apps/app/.env.development'),
      }),
    ])
  })

  test('harvests root .env and .env.local into development at lowest priority', async () => {
    const rootDir = requireTempRoot(tempRoot)
    await writeFixture(rootDir, 'apps/server/.env.development', {
      LOG_LEVEL: 'debug',
    })
    await writeFixture(rootDir, '.env', {
      CDP_PROTOCOL_JSON: '/tmp/protocol.json',
      LOG_LEVEL: 'info',
    })
    await writeFixture(rootDir, '.env.local', {
      BROWSEROS_BINARY: '/tmp/BrowserOS',
    })

    const plan = buildLocalEnvMigration(rootDir)
    const development = plan.files.find((file) => file.mode === 'development')

    expect(development?.content).toContain(
      'CDP_PROTOCOL_JSON=/tmp/protocol.json',
    )
    expect(development?.content).toContain('BROWSEROS_BINARY=/tmp/BrowserOS')
    expect(development?.content).toContain('LOG_LEVEL=debug')
    expect(development?.content).not.toContain('LOG_LEVEL=info')
    expect(plan.oldFiles).toEqual(
      expect.arrayContaining([
        join(rootDir, '.env'),
        join(rootDir, '.env.local'),
      ]),
    )
  })

  test('warns loudly when retired R2 prefixes differ from code defaults', async () => {
    const rootDir = requireTempRoot(tempRoot)
    await writeFixture(rootDir, 'apps/server/.env.production', {
      R2_UPLOAD_PREFIX: 'custom/server',
      R2_DOWNLOAD_PREFIX: 'custom/vendor',
    })
    await writeFixture(rootDir, 'apps/cli/.env.production', {
      R2_UPLOAD_PREFIX: 'cli',
    })

    const plan = buildLocalEnvMigration(rootDir)

    expect(plan.dropped).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          key: 'R2_UPLOAD_PREFIX',
          warning: expect.stringContaining(
            'RETIRED PREFIX WARNING: R2_UPLOAD_PREFIX=custom/server',
          ),
        }),
        expect.objectContaining({
          key: 'R2_DOWNLOAD_PREFIX',
          warning: expect.stringContaining(
            'RETIRED PREFIX WARNING: R2_DOWNLOAD_PREFIX=custom/vendor',
          ),
        }),
        expect.objectContaining({
          key: 'R2_UPLOAD_PREFIX',
          sourceFile: join(rootDir, 'apps/cli/.env.production'),
          warning: undefined,
        }),
      ]),
    )
  })

  test('refuses to overwrite root env files without force', async () => {
    const rootDir = requireTempRoot(tempRoot)
    await writeFixture(rootDir, 'apps/server/.env.development', {
      POSTHOG_API_KEY: 'server-key',
    })
    await writeFile(join(rootDir, '.env.development'), 'EXISTING=1\n')

    const refused = migrateLocalEnv({ rootDir })

    expect(refused.refused).toEqual([join(rootDir, '.env.development')])
    expect(readFileSync(join(rootDir, '.env.development'), 'utf8')).toBe(
      'EXISTING=1\n',
    )

    const forced = migrateLocalEnv({ rootDir, force: true })

    expect(forced.refused).toEqual([])
    expect(readFileSync(join(rootDir, '.env.development'), 'utf8')).toContain(
      'POSTHOG_API_KEY=server-key',
    )
  })

  test('dry-run returns the plan without writing root env files', async () => {
    const rootDir = requireTempRoot(tempRoot)
    await writeFixture(rootDir, 'apps/server/.env.production', {
      R2_BUCKET: 'custom-bucket',
    })

    const result = migrateLocalEnv({ rootDir, dryRun: true })
    const production = result.files.find((file) => file.mode === 'production')

    expect(result.wrote).toEqual([])
    expect(result.refused).toEqual([])
    expect(existsSync(join(rootDir, '.env.production'))).toBe(false)
    expect(production?.content).toContain('R2_BUCKET=custom-bucket')
  })
})

async function writeFixture(
  rootDir: string,
  path: string,
  values: Record<string, string>,
): Promise<void> {
  const fullPath = join(rootDir, path)
  await mkdir(dirname(fullPath), { recursive: true })
  await writeFile(
    fullPath,
    `${Object.entries(values)
      .map(([key, value]) => `${key}=${value}`)
      .join('\n')}\n`,
  )
}

function requireTempRoot(rootDir: string | null): string {
  if (!rootDir) {
    throw new Error('missing temp root')
  }
  return rootDir
}
