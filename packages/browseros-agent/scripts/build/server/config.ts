import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import { parse } from 'dotenv'

import type { BuildConfig } from './types'

const REQUIRED_PROD_VARS = [
  'BROWSEROS_CONFIG_URL',
  'POSTHOG_API_KEY',
  'SENTRY_DSN',
]
const INLINED_ENV_VARS = [
  ...REQUIRED_PROD_VARS,
  'AGENT_RUNNER_JWT_SECRET',
  'NODE_ENV',
  'LOG_LEVEL',
] as const
const CI_INLINE_ENV_DEFAULTS = {
  BROWSEROS_CONFIG_URL: 'https://browseros.invalid/api/browseros-server/config',
  LOG_LEVEL: 'info',
  NODE_ENV: 'production',
  POSTHOG_API_KEY: 'phc_browseros_ci',
  SENTRY_DSN: 'https://ci@sentry.invalid/1',
} satisfies Partial<Record<(typeof INLINED_ENV_VARS)[number], string>>
const PROD_ENV_PATH = join('apps', 'server', '.env.production')
const PROD_ENV_TEMPLATE_PATH = join('apps', 'server', '.env.production.example')

function readServerVersion(rootDir: string): string {
  const pkgPath = join(rootDir, 'apps/server/package.json')
  const pkg = JSON.parse(readFileSync(pkgPath, 'utf-8'))
  return pkg.version
}

function pickEnv(name: string, fileEnv: Record<string, string>): string {
  const value = process.env[name] ?? fileEnv[name]
  if (!value || value.trim().length === 0) {
    throw new Error(`Missing required environment variable: ${name}`)
  }
  return value
}

function loadProdEnv(
  rootDir: string,
  options: { required?: boolean } = {},
): Record<string, string> {
  const prodEnvPath = join(rootDir, PROD_ENV_PATH)
  if (!existsSync(prodEnvPath)) {
    if (options.required === false) return {}

    const prodEnvTemplatePath = join(rootDir, PROD_ENV_TEMPLATE_PATH)
    if (existsSync(prodEnvTemplatePath)) {
      throw new Error(
        `Missing ${PROD_ENV_PATH}. Create it from ${PROD_ENV_TEMPLATE_PATH} before running build:server.`,
      )
    }
    throw new Error(
      `Missing ${PROD_ENV_PATH}. The template file ${PROD_ENV_TEMPLATE_PATH} was not found.`,
    )
  }
  return parse(readFileSync(prodEnvPath, 'utf-8'))
}

function buildInlineEnv(
  fileEnv: Record<string, string>,
): Record<string, string> {
  const inlineEnv: Record<string, string> = {}
  for (const key of INLINED_ENV_VARS) {
    const value = process.env[key] ?? fileEnv[key]
    if (value !== undefined && value.trim().length > 0) {
      inlineEnv[key] = value
    }
  }
  return inlineEnv
}

function validateProductionEnv(envVars: Record<string, string>): void {
  const missing = REQUIRED_PROD_VARS.filter((name) => {
    const value = envVars[name]
    return !value || value.trim().length === 0
  })
  if (missing.length > 0) {
    throw new Error(
      `Production build requires variables: ${missing.join(', ')} (set them in ${PROD_ENV_PATH} or process env).`,
    )
  }
}

export interface LoadBuildConfigOptions {
  ci?: boolean
}

export function loadBuildConfig(
  rootDir: string,
  options: LoadBuildConfigOptions = {},
): BuildConfig {
  const fileEnv = loadProdEnv(rootDir, { required: !options.ci })
  const envVars = buildInlineEnv(fileEnv)
  if (options.ci) {
    for (const [key, value] of Object.entries(CI_INLINE_ENV_DEFAULTS)) {
      envVars[key] ??= value
    }
  }
  if (!options.ci) {
    validateProductionEnv(envVars)
  }

  const processEnv: NodeJS.ProcessEnv = {
    PATH: process.env.PATH ?? '',
    ...fileEnv,
    ...process.env,
  }

  if (options.ci) {
    return { version: readServerVersion(rootDir), envVars, processEnv }
  }

  return {
    version: readServerVersion(rootDir),
    envVars,
    processEnv,
    r2: {
      accountId: pickEnv('R2_ACCOUNT_ID', fileEnv),
      accessKeyId: pickEnv('R2_ACCESS_KEY_ID', fileEnv),
      secretAccessKey: pickEnv('R2_SECRET_ACCESS_KEY', fileEnv),
      bucket: pickEnv('R2_BUCKET', fileEnv),
      downloadPrefix:
        process.env.R2_DOWNLOAD_PREFIX ?? fileEnv.R2_DOWNLOAD_PREFIX ?? '',
      uploadPrefix:
        process.env.R2_UPLOAD_PREFIX ??
        fileEnv.R2_UPLOAD_PREFIX ??
        'server/prod-resources',
    },
  }
}
