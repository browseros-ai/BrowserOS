import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { loadBuildConfig } from '@browseros/build-server-tools'

import { appOnboardBuildProduct } from './descriptor'

const R2_ENV_KEYS = [
  'R2_ACCOUNT_ID',
  'R2_ACCESS_KEY_ID',
  'R2_SECRET_ACCESS_KEY',
  'R2_BUCKET',
  'R2_UPLOAD_PREFIX',
  'R2_DOWNLOAD_PREFIX',
] as const

describe('app onboard build descriptor', () => {
  let tempRoot: string | null = null
  let originalEnv: Partial<Record<(typeof R2_ENV_KEYS)[number], string>> = {}

  beforeEach(() => {
    originalEnv = {}
    for (const key of R2_ENV_KEYS) {
      originalEnv[key] = process.env[key]
      delete process.env[key]
    }
  })

  afterEach(async () => {
    for (const key of R2_ENV_KEYS) {
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

  it('loads without a production env file and reads the app version', async () => {
    const rootDir = await writeOnboardPackageRoot()

    const config = loadBuildConfig(rootDir, appOnboardBuildProduct)

    expect(config.version).toBe('0.0.0-test')
  })

  it('defaults the upload prefix to app-onboard/prod-resources', async () => {
    const rootDir = await writeOnboardPackageRoot()
    setFakeR2Env()

    const config = loadBuildConfig(rootDir, appOnboardBuildProduct, {
      requireR2: true,
    })

    expect(config.r2?.uploadPrefix).toBe('app-onboard/prod-resources')
  })

  it('honors an R2_UPLOAD_PREFIX override', async () => {
    const rootDir = await writeOnboardPackageRoot()
    setFakeR2Env()
    process.env.R2_UPLOAD_PREFIX = 'custom/onboard'

    const config = loadBuildConfig(rootDir, appOnboardBuildProduct, {
      requireR2: true,
    })

    expect(config.r2?.uploadPrefix).toBe('custom/onboard')
  })

  it('forces production NODE_ENV over ambient env', async () => {
    const originalNodeEnv = process.env.NODE_ENV
    process.env.NODE_ENV = 'test'
    try {
      const rootDir = await writeOnboardPackageRoot()

      const config = loadBuildConfig(rootDir, appOnboardBuildProduct)

      expect(config.envVars.NODE_ENV).toBe('production')
    } finally {
      if (originalNodeEnv === undefined) {
        delete process.env.NODE_ENV
      } else {
        process.env.NODE_ENV = originalNodeEnv
      }
    }
  })

  it('builds the chromium dist of the app-onboard app', () => {
    expect(appOnboardBuildProduct.packageDir).toBe('apps/app-onboard')
    expect(appOnboardBuildProduct.buildCommand).toEqual([
      'bun',
      'run',
      'build:chromium',
    ])
    expect(appOnboardBuildProduct.assetsDir).toBe(
      'apps/app-onboard/dist/chromium',
    )
    expect(appOnboardBuildProduct.archiveBaseName).toBe(
      'browseros-app-onboard-resources',
    )
  })

  function setFakeR2Env(): void {
    process.env.R2_ACCOUNT_ID = 'test'
    process.env.R2_ACCESS_KEY_ID = 'test'
    process.env.R2_SECRET_ACCESS_KEY = 'test'
    process.env.R2_BUCKET = 'test'
  }

  async function writeOnboardPackageRoot(): Promise<string> {
    tempRoot = await mkdtemp(join(tmpdir(), 'app-onboard-build-descriptor-'))
    const packageDir = join(tempRoot, 'apps/app-onboard')
    await mkdir(packageDir, { recursive: true })
    await writeFile(
      join(packageDir, 'package.json'),
      '{"version":"0.0.0-test"}',
    )
    return tempRoot
  }
})
