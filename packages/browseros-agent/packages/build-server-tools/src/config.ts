import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'

import { parse } from 'dotenv'

import type { BuildConfig, BuildProductDescriptor } from './types'

function readPackageVersion(
  rootDir: string,
  product: BuildProductDescriptor,
): string {
  const pkgPath = join(rootDir, product.packageDir, 'package.json')
  const pkg = JSON.parse(readFileSync(pkgPath, 'utf-8'))
  return pkg.version
}

function pickEnv(
  name: string,
  fileEnv: Record<string, string>,
  product: BuildProductDescriptor,
): string {
  const value = process.env[name] ?? fileEnv[name]
  if (!value || value.trim().length === 0) {
    throw new Error(
      `Missing required environment variable for ${product.label}: ${name}`,
    )
  }
  return value
}

function loadProdEnv(
  rootDir: string,
  product: BuildProductDescriptor,
  options: { required?: boolean } = {},
): Record<string, string> {
  const prodEnvPath = join(rootDir, product.env.prodEnvPath)
  if (!existsSync(prodEnvPath)) {
    if (options.required === false) return {}

    const templatePath = product.env.prodEnvTemplatePath
    const absTemplatePath = templatePath ? join(rootDir, templatePath) : ''
    if (templatePath && existsSync(absTemplatePath)) {
      throw new Error(
        `Missing ${product.env.prodEnvPath}. Create it from ${templatePath} before running this build.`,
      )
    }
    throw new Error(`Missing ${product.env.prodEnvPath}.`)
  }
  return parse(readFileSync(prodEnvPath, 'utf-8'))
}

function buildInlineEnv(
  product: BuildProductDescriptor,
  fileEnv: Record<string, string>,
): Record<string, string> {
  const inlineEnv: Record<string, string> = {}
  for (const key of product.env.inlineEnvKeys) {
    const value = process.env[key] ?? fileEnv[key]
    if (value !== undefined && value.trim().length > 0) {
      inlineEnv[key] = value
    }
  }
  return inlineEnv
}

function validateProductionEnv(
  product: BuildProductDescriptor,
  envVars: Record<string, string>,
): void {
  const missing = product.env.requiredInlineEnvKeys.filter((name) => {
    const value = envVars[name]
    return !value || value.trim().length === 0
  })
  if (missing.length > 0) {
    throw new Error(
      `Production ${product.label} build requires variables: ${missing.join(
        ', ',
      )} (set them in ${product.env.prodEnvPath} or process env).`,
    )
  }
}

export interface LoadBuildConfigOptions {
  ci?: boolean
  requireR2?: boolean
}

/** Loads version, inline env, subprocess env, and optional R2 config for one product build. */
export function loadBuildConfig(
  rootDir: string,
  product: BuildProductDescriptor,
  options: LoadBuildConfigOptions = {},
): BuildConfig {
  const requireProdEnv = !options.ci && product.env.requireProdEnvFile !== false
  const fileEnv = loadProdEnv(rootDir, product, { required: requireProdEnv })
  const envVars = buildInlineEnv(product, fileEnv)
  if (options.ci) {
    for (const [key, value] of Object.entries(
      product.env.ciInlineEnvDefaults ?? {},
    )) {
      envVars[key] ??= value
    }
  }
  if (!options.ci) {
    validateProductionEnv(product, envVars)
  }

  const processEnv: NodeJS.ProcessEnv = {
    PATH: process.env.PATH ?? '',
    ...fileEnv,
    ...process.env,
  }

  const config: BuildConfig = {
    version: readPackageVersion(rootDir, product),
    envVars,
    processEnv,
  }

  if (options.requireR2 && !options.ci) {
    config.r2 = {
      accountId: pickEnv('R2_ACCOUNT_ID', fileEnv, product),
      accessKeyId: pickEnv('R2_ACCESS_KEY_ID', fileEnv, product),
      secretAccessKey: pickEnv('R2_SECRET_ACCESS_KEY', fileEnv, product),
      bucket: pickEnv('R2_BUCKET', fileEnv, product),
      downloadPrefix:
        process.env.R2_DOWNLOAD_PREFIX ?? fileEnv.R2_DOWNLOAD_PREFIX ?? '',
      uploadPrefix:
        process.env.R2_UPLOAD_PREFIX ??
        fileEnv.R2_UPLOAD_PREFIX ??
        product.env.defaultR2UploadPrefix,
    }
  }

  return config
}
