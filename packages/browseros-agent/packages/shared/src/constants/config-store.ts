/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * ConfigStore — runtime configuration overrides with file persistence.
 *
 * Uses lazy Node.js fs imports so the module can be safely imported
 * in browser bundles without pulling in node:fs at build time.
 * File I/O only executes when a filePath is provided (server-side).
 */

import {
  CONFIG_KEY_MAP,
  type ConfigOverrides,
  validateAllOverrides,
} from './config-schema'

function readNumericEnv(envVar: string): number | undefined {
  const raw = process.env[envVar]
  if (raw === undefined) return undefined
  const normalized = raw.trim().replace(/_/g, '')
  if (!/^\d+$/.test(normalized)) return undefined
  const parsed = Number(normalized)
  if (!Number.isSafeInteger(parsed) || parsed < 0) return undefined
  return parsed
}

/** Lazy-loaded Node.js fs functions — only used server-side. */
let fsImpl: typeof import('node:fs') | null = null
let pathImpl: typeof import('node:path') | null = null

function getFs() {
  if (!fsImpl) {
    fsImpl = require('node:fs')
  }
  return fsImpl
}

function getPath() {
  if (!pathImpl) {
    pathImpl = require('node:path')
  }
  return pathImpl
}

export class ConfigStore {
  private activeOverrides: ConfigOverrides = {}
  private pendingOverrides: ConfigOverrides = {}
  private filePath: string | null = null

  init(configFilePath?: string): void {
    this.filePath = configFilePath ?? null
    const diskOverrides = this.readFromDisk()
    this.activeOverrides = { ...diskOverrides }
    this.pendingOverrides = { ...diskOverrides }
  }

  get(key: string): number {
    const meta = CONFIG_KEY_MAP.get(key)
    if (!meta) throw new Error(`Unknown config key: ${key}`)

    const envValue = readNumericEnv(meta.envVar)
    if (envValue !== undefined) return envValue

    const fileValue = this.activeOverrides[key]
    if (fileValue !== undefined) return fileValue

    return meta.default
  }

  getAllActive(): ConfigOverrides {
    const active: ConfigOverrides = {}
    for (const key of CONFIG_KEY_MAP.keys()) {
      active[key] = this.get(key)
    }
    return active
  }

  getFileOverrides(): ConfigOverrides {
    return { ...this.pendingOverrides }
  }

  getDefaults(): ConfigOverrides {
    const defaults: ConfigOverrides = {}
    for (const [key, meta] of CONFIG_KEY_MAP.entries()) {
      defaults[key] = meta.default
    }
    return defaults
  }

  save(overrides: ConfigOverrides): Map<string, string> {
    const errors = validateAllOverrides(overrides)
    if (errors.size > 0) return errors

    this.pendingOverrides = { ...overrides }
    this.writeToDisk()
    return new Map()
  }

  reset(): void {
    this.activeOverrides = {}
    this.pendingOverrides = {}
    if (!this.filePath) return

    try {
      getFs().unlinkSync(this.filePath)
    } catch {
      // ignore missing or already removed file
    }
  }

  hasPendingChanges(): boolean {
    const activeKeys = Object.keys(this.activeOverrides)
    const pendingKeys = Object.keys(this.pendingOverrides)
    if (activeKeys.length !== pendingKeys.length) return true

    for (const key of pendingKeys) {
      if (this.pendingOverrides[key] !== this.activeOverrides[key]) {
        return true
      }
    }

    return false
  }

  private readFromDisk(): ConfigOverrides {
    if (!this.filePath) return {}

    try {
      const fs = getFs()
      if (!fs.existsSync(this.filePath)) return {}

      const content = fs.readFileSync(this.filePath, 'utf8')
      const parsed = JSON.parse(content)
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
        return {}
      }

      const candidate: ConfigOverrides = {}
      for (const [key, value] of Object.entries(parsed)) {
        if (typeof value === 'number') {
          candidate[key] = value
        }
      }

      const errors = validateAllOverrides(candidate)
      if (errors.size > 0) return {}

      return candidate
    } catch {
      return {}
    }
  }

  private writeToDisk(): void {
    if (!this.filePath) return
    const fs = getFs()
    const path = getPath()
    fs.mkdirSync(path.dirname(this.filePath), { recursive: true })
    fs.writeFileSync(
      this.filePath,
      `${JSON.stringify(this.pendingOverrides, null, 2)}\n`,
    )
  }
}

export const configStore = new ConfigStore()
