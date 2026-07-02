import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { loadBuildConfig } from '../src'
import { testProduct } from './helpers'

const REQUIRED_INLINE_ENV = {
  TEST_CONFIG_URL: 'https://stub.test/config',
}

const R2_ENV = {
  R2_ACCOUNT_ID: 'test',
  R2_ACCESS_KEY_ID: 'test',
  R2_SECRET_ACCESS_KEY: 'test',
  R2_BUCKET: 'test',
}

const TEST_ENV_KEYS = [
  ...Object.keys(REQUIRED_INLINE_ENV),
  ...Object.keys(R2_ENV),
  'LOG_LEVEL',
  'R2_DOWNLOAD_PREFIX',
  'R2_UPLOAD_PREFIX',
] as const

describe('build config', () => {
  let tempRoot: string | null = null
  let originalEnv: Partial<Record<(typeof TEST_ENV_KEYS)[number], string>>

  beforeEach(() => {
    originalEnv = {}
    for (const key of TEST_ENV_KEYS) {
      originalEnv[key] = process.env[key]
      delete process.env[key]
    }
  })

  afterEach(async () => {
    for (const key of TEST_ENV_KEYS) {
      const value = originalEnv[key]
      if (value === undefined) {
        delete process.env[key]
      } else {
        process.env[key] = value
      }
    }

    if (tempRoot) {
      await rm(tempRoot, { recursive: true, force: true })
      tempRoot = null
    }
  })

  it('reads package version and inline env from the product env file', async () => {
    const rootDir = await writeProdRoot({
      ...REQUIRED_INLINE_ENV,
      ...R2_ENV,
      LOG_LEVEL: 'debug',
    })

    const config = loadBuildConfig(rootDir, testProduct(), { requireR2: true })

    expect(config.version).toBe('0.0.0-test')
    expect(config.envVars).toMatchObject({
      TEST_CONFIG_URL: 'https://stub.test/config',
      LOG_LEVEL: 'debug',
    })
    expect(config.r2?.uploadPrefix).toBe('test-server/prod-resources')
  })

  it('lets process env override inline env and R2 prefixes from the product env file', async () => {
    const rootDir = await writeProdRoot({
      ...REQUIRED_INLINE_ENV,
      ...R2_ENV,
      LOG_LEVEL: 'debug',
      R2_UPLOAD_PREFIX: 'file-prefix',
    })
    process.env.LOG_LEVEL = 'warn'
    process.env.R2_UPLOAD_PREFIX = 'process-prefix'

    const config = loadBuildConfig(rootDir, testProduct(), { requireR2: true })

    expect(config.envVars.LOG_LEVEL).toBe('warn')
    expect(config.r2?.uploadPrefix).toBe('process-prefix')
  })

  it('does not require a production env file in CI mode', async () => {
    const rootDir = await writeProdRoot({}, { envFile: false })

    const config = loadBuildConfig(rootDir, testProduct(), { ci: true })

    expect(config.envVars).toEqual({
      LOG_LEVEL: 'info',
      TEST_CONFIG_URL: 'https://test.invalid/config',
    })
    expect(config.r2).toBeUndefined()
  })

  it('allows optional-env products to build local artifacts without R2', async () => {
    const rootDir = await writeProdRoot({}, { envFile: false })
    const product = testProduct({
      env: {
        ...testProduct().env,
        requireProdEnvFile: false,
        requiredInlineEnvKeys: [],
        inlineEnvKeys: [],
      },
    })

    const config = loadBuildConfig(rootDir, product)

    expect(config.envVars).toEqual({})
    expect(config.r2).toBeUndefined()
  })

  it('still requires R2 when optional-env products upload artifacts', async () => {
    const rootDir = await writeProdRoot({}, { envFile: false })
    const product = testProduct({
      env: {
        ...testProduct().env,
        requireProdEnvFile: false,
        requiredInlineEnvKeys: [],
        inlineEnvKeys: [],
      },
    })

    expect(() =>
      loadBuildConfig(rootDir, product, { requireR2: true }),
    ).toThrow('R2_ACCOUNT_ID')
  })

  async function writeProdRoot(
    env: Record<string, string>,
    options: { envFile?: boolean } = {},
  ): Promise<string> {
    tempRoot = await mkdtemp(join(tmpdir(), 'build-server-config-'))
    const packageDir = join(tempRoot, 'apps/test-server')
    await mkdir(packageDir, { recursive: true })
    await writeFile(
      join(packageDir, 'package.json'),
      '{"version":"0.0.0-test"}',
    )
    await writeFile(join(packageDir, '.env.production.example'), '')
    if (options.envFile !== false) {
      await writeFile(join(packageDir, '.env.production'), formatEnv(env))
    }
    return tempRoot
  }
})

function formatEnv(env: Record<string, string>): string {
  return `${Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join('\n')}\n`
}
