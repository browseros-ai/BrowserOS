/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */
export interface IdentityConfig {
  installId?: string
}

export class IdentityService {
  private browserOSId: string | null = null

  /** Uses the canonical installation ID, with an ephemeral fallback for damaged state. */
  initialize(config: IdentityConfig): void {
    this.browserOSId =
      normalizeInstallId(config.installId) ?? crypto.randomUUID()
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
}

function normalizeInstallId(installId: string | undefined): string | null {
  return installId && installId.length > 0 ? installId : null
}

export const identity = new IdentityService()
