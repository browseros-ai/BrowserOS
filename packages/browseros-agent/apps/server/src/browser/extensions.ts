/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Extension management module — wraps CDP Extensions.* domain for CRUD operations.
 * Decoupled from tool layer; consumed by tools/extensions.ts
 */

import type { CdpBackend } from './backends/types'
export type { StorageArea } from '@browseros/cdp-protocol/generated/domains/extensions'
import type { StorageArea } from '@browseros/cdp-protocol/generated/domains/extensions'

// BrowserOS first-party extension IDs (from browseros_constants.h)
const BROWSEROS_EXTENSION_IDS = new Set([
  'bflpfmnmnokmjhmgnolecpppdbdophmk', // Agent
  'adlpneommgkgeanpaekgoaolcpncohkf', // Bug Reporter
  'nlnihljpboknmfagkikhkdblbedophja', // Controller
])

/** Check if an extension ID belongs to BrowserOS first-party */
export function isBrowserOSExtension(id: string): boolean {
  return BROWSEROS_EXTENSION_IDS.has(id)
}

/** Validate that managed storage area is not used for write operations */
function validateNotManaged(area: StorageArea): void {
  if (area === 'managed') {
    throw new Error('Storage area "managed" is read-only and cannot be written to')
  }
}

// ── Lifecycle operations ──

/** Load an unpacked extension from a local directory */
export async function loadUnpackedExtension(
  cdp: CdpBackend,
  path: string,
): Promise<string> {
  if (!path) {
    throw new Error('Extension path is required')
  }
  const result = await cdp.Extensions.loadUnpacked({ path })
  return result.id
}

/** Uninstall an extension by ID. Rejects first-party BrowserOS extensions. */
export async function uninstallExtension(
  cdp: CdpBackend,
  id: string,
): Promise<void> {
  if (isBrowserOSExtension(id)) {
    throw new Error(
      `Cannot uninstall BrowserOS first-party extension: ${id}`,
    )
  }
  await cdp.Extensions.uninstall({ id })
}

// ── Storage operations ──

/** Get storage items from an extension's storage area */
export async function getStorageItems(
  cdp: CdpBackend,
  id: string,
  storageArea: StorageArea,
  keys?: string[],
): Promise<Record<string, unknown>> {
  const params: Record<string, unknown> = { id, storageArea }
  if (keys) {
    params.keys = keys
  }
  const result = await cdp.Extensions.getStorageItems(params as any)
  return result.data
}

/** Set storage items in an extension's storage area */
export async function setStorageItems(
  cdp: CdpBackend,
  id: string,
  storageArea: StorageArea,
  values: Record<string, unknown>,
): Promise<void> {
  validateNotManaged(storageArea)
  await cdp.Extensions.setStorageItems({ id, storageArea, values })
}

/** Remove specific keys from an extension's storage area */
export async function removeStorageItems(
  cdp: CdpBackend,
  id: string,
  storageArea: StorageArea,
  keys: string[],
): Promise<void> {
  validateNotManaged(storageArea)
  await cdp.Extensions.removeStorageItems({ id, storageArea, keys })
}

/** Clear all items from an extension's storage area */
export async function clearStorageItems(
  cdp: CdpBackend,
  id: string,
  storageArea: StorageArea,
): Promise<void> {
  validateNotManaged(storageArea)
  await cdp.Extensions.clearStorageItems({ id, storageArea })
}
