import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import {
  type ResolvedEnv,
  requireEnv,
  resolveEnv,
} from '@browseros/shared/env/load'

import type { BuildConfig, ProductBuildSpec } from './types'

function readPackageVersion(
  rootDir: string,
  product: ProductBuildSpec,
): string {
  const pkgPath = join(rootDir, product.packageDir, 'package.json')
  const pkg = JSON.parse(readFileSync(pkgPath, 'utf-8'))
  return pkg.version
}

function buildInlineEnv(
  product: ProductBuildSpec,
  resolved: ResolvedEnv,
): Record<string, string> {
  const inlineEnv: Record<string, string> = {}
  for (const key of product.env.inlineEnvKeys) {
    const value = resolved.values[key]
    if (value !== undefined && value.trim().length > 0) {
      inlineEnv[key] = value
    }
  }
  return inlineEnv
}

function requireProductEnv(
  product: ProductBuildSpec,
  resolved: ResolvedEnv,
  keys: readonly string[],
  values: Record<string, string> = resolved.values,
): void {
  try {
    requireEnv({ ...resolved, values }, [...keys])
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`${product.label}: ${message}`)
  }
}

function rootFileEnv(resolved: ResolvedEnv): Record<string, string> {
  const values: Record<string, string> = {}
  for (const [key, source] of Object.entries(resolved.sources)) {
    if (source === 'root-file') {
      values[key] = resolved.values[key]
    }
  }
  return values
}

function ambientEnvWithoutDemotions(resolved: ResolvedEnv): NodeJS.ProcessEnv {
  const ambient: NodeJS.ProcessEnv = { ...process.env }
  for (const key of resolved.demotedKeys) {
    delete ambient[key]
  }
  return ambient
}

function warnDemotedEnv(resolved: ResolvedEnv): void {
  if (resolved.demotedKeys.length === 0) {
    return
  }

  console.warn(
    `env: ignoring ${resolved.demotedKeys.length} process values that match root .env.development (Bun auto-load): ${resolved.demotedKeys
      .toSorted()
      .join(', ')}`,
  )
}

export interface LoadBuildConfigOptions {
  ci?: boolean
  requireR2?: boolean
}

/** Loads version, inline env, subprocess env, and optional R2 config for one product build. */
export function loadBuildConfig(
  rootDir: string,
  product: ProductBuildSpec,
  options: LoadBuildConfigOptions = {},
): BuildConfig {
  const resolved = resolveEnv({ rootDir, mode: 'production' })
  const fileEnv = rootFileEnv(resolved)
  const ambientEnv = ambientEnvWithoutDemotions(resolved)
  const envVars = buildInlineEnv(product, resolved)
  if (!options.ci) {
    warnDemotedEnv(resolved)
  }
  if (options.ci) {
    for (const [key, value] of Object.entries(
      product.env.ciInlineEnvDefaults ?? {},
    )) {
      envVars[key] ??= value
    }
  }
  Object.assign(envVars, product.env.inlineEnvOverrides ?? {})
  if (!options.ci) {
    requireProductEnv(product, resolved, product.env.requiredInlineEnvKeys, {
      ...resolved.values,
      ...envVars,
    })
  }

  const processEnv: NodeJS.ProcessEnv = {
    PATH: process.env.PATH ?? '',
    ...fileEnv,
    ...ambientEnv,
  }

  const config: BuildConfig = {
    version: readPackageVersion(rootDir, product),
    envVars,
    processEnv,
  }

  if (options.requireR2 && !options.ci) {
    const r2 = requireProductR2Env(product, resolved)
    config.r2 = {
      accountId: r2.R2_ACCOUNT_ID,
      accessKeyId: r2.R2_ACCESS_KEY_ID,
      secretAccessKey: r2.R2_SECRET_ACCESS_KEY,
      bucket: r2.R2_BUCKET,
      downloadPrefix:
        process.env.R2_DOWNLOAD_PREFIX ??
        product.env.defaultR2DownloadPrefix ??
        '',
      uploadPrefix:
        process.env.R2_UPLOAD_PREFIX ?? product.env.defaultR2UploadPrefix,
    }
  }

  return config
}

function requireProductR2Env(
  product: ProductBuildSpec,
  resolved: ResolvedEnv,
): Record<
  'R2_ACCOUNT_ID' | 'R2_ACCESS_KEY_ID' | 'R2_SECRET_ACCESS_KEY' | 'R2_BUCKET',
  string
> {
  try {
    return requireEnv(resolved, [
      'R2_ACCOUNT_ID',
      'R2_ACCESS_KEY_ID',
      'R2_SECRET_ACCESS_KEY',
      'R2_BUCKET',
    ]) as Record<
      | 'R2_ACCOUNT_ID'
      | 'R2_ACCESS_KEY_ID'
      | 'R2_SECRET_ACCESS_KEY'
      | 'R2_BUCKET',
      string
    >
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    throw new Error(`${product.label}: ${message}`)
  }
}
