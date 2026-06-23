import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { loadBuildConfig } from './config'

const REQUIRED_INLINE_ENV = {
  BROWSEROS_CONFIG_URL: 'https://stub.test/config',
  POSTHOG_API_KEY: 'phc_test_stub',
  SENTRY_DSN: 'https://stub@sentry.test/0',
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
  'AGENT_RUNNER_JWT_SECRET',
  'NODE_ENV',
  'LOG_LEVEL',
  'R2_DOWNLOAD_PREFIX',
  'R2_UPLOAD_PREFIX',
] as const

describe('server build config', () => {
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

  it('inlines AGENT_RUNNER_JWT_SECRET from the production env file', async () => {
    const rootDir = await writeProdRoot({
      ...REQUIRED_INLINE_ENV,
      ...R2_ENV,
      AGENT_RUNNER_JWT_SECRET: 'file-secret',
    })

    const config = loadBuildConfig(rootDir)

    expect(config.envVars.AGENT_RUNNER_JWT_SECRET).toBe('file-secret')
  })

  it('lets process env override AGENT_RUNNER_JWT_SECRET from the production env file', async () => {
    const rootDir = await writeProdRoot({
      ...REQUIRED_INLINE_ENV,
      ...R2_ENV,
      AGENT_RUNNER_JWT_SECRET: 'file-secret',
    })
    process.env.AGENT_RUNNER_JWT_SECRET = 'process-secret'

    const config = loadBuildConfig(rootDir)

    expect(config.envVars.AGENT_RUNNER_JWT_SECRET).toBe('process-secret')
  })

  it('does not require AGENT_RUNNER_JWT_SECRET for production packaging', async () => {
    const rootDir = await writeProdRoot({
      ...REQUIRED_INLINE_ENV,
      ...R2_ENV,
    })

    const config = loadBuildConfig(rootDir)

    expect(config.envVars.AGENT_RUNNER_JWT_SECRET).toBeUndefined()
  })

  it('treats an empty AGENT_RUNNER_JWT_SECRET as absent', async () => {
    const rootDir = await writeProdRoot({
      ...REQUIRED_INLINE_ENV,
      ...R2_ENV,
      AGENT_RUNNER_JWT_SECRET: '',
    })

    const config = loadBuildConfig(rootDir)

    expect(config.envVars.AGENT_RUNNER_JWT_SECRET).toBeUndefined()
  })

  it('does not require a production env file in CI mode', async () => {
    const rootDir = await writeProdRoot({}, { envFile: false })

    const config = loadBuildConfig(rootDir, { ci: true })

    expect(config.envVars).toEqual({
      BROWSEROS_CONFIG_URL:
        'https://browseros.invalid/api/browseros-server/config',
      LOG_LEVEL: 'info',
      NODE_ENV: 'production',
      POSTHOG_API_KEY: 'phc_browseros_ci',
      SENTRY_DSN: 'https://ci@sentry.invalid/1',
    })
    expect(config.r2).toBeUndefined()
  })

  async function writeProdRoot(
    env: Record<string, string>,
    options: { envFile?: boolean } = {},
  ): Promise<string> {
    tempRoot = await mkdtemp(join(tmpdir(), 'browseros-build-config-test-'))
    const serverDir = join(tempRoot, 'apps/server')
    await mkdir(serverDir, { recursive: true })
    await writeFile(join(serverDir, 'package.json'), '{"version":"0.0.0-test"}')
    await writeFile(join(serverDir, '.env.production.example'), '')
    if (options.envFile !== false) {
      await writeFile(join(serverDir, '.env.production'), formatEnv(env))
    }
    return tempRoot
  }
})

function formatEnv(env: Record<string, string>): string {
  return `${Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join('\n')}\n`
}
