/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'

export interface IdentityConfig {
  installId?: string
  statePath?: string
}

interface IdentityStateFile {
  browserosId: string
}

export class IdentityService {
  private browserOSId: string | null = null

  /** Chooses the stable BrowserOS id without coupling it to the product SQLite schema. */
  initialize(config: IdentityConfig): void {
    this.browserOSId =
      normalizeInstallId(config.installId) ??
      this.loadFromState(config.statePath) ??
      this.generateAndSave(config.statePath)
  }

  getBrowserOSId(): string {
    if (!this.browserOSId) {
      throw new Error(
        'IdentityService not initialized. Call initialize() first.',
      )
    }
    return this.browserOSId
  }

  isInitialized(): boolean {
    return this.browserOSId !== null
  }

  private loadFromState(statePath: string | undefined): string | null {
    if (!statePath) return null
    try {
      const parsed = JSON.parse(
        readFileSync(statePath, 'utf8'),
      ) as Partial<IdentityStateFile>
      return typeof parsed.browserosId === 'string' &&
        parsed.browserosId.length > 0
        ? parsed.browserosId
        : null
    } catch (err) {
      if (isNotFoundError(err)) return null
      throw err
    }
  }

  private generateAndSave(statePath: string | undefined): string {
    const browserosId = crypto.randomUUID()
    if (statePath) {
      mkdirSync(dirname(statePath), { recursive: true })
      writeFileSync(statePath, `${JSON.stringify({ browserosId })}\n`, 'utf8')
    }
    return browserosId
  }
}

function normalizeInstallId(installId: string | undefined): string | null {
  return installId && installId.length > 0 ? installId : null
}

function isNotFoundError(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    err.code === 'ENOENT'
  )
}

export const identity = new IdentityService()
