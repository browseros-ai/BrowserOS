/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Finds the CDP endpoint of a running BrowserOS neo. The browser binds its
 * managed CDP server on startup (browseros_server_manager.cc) and persists the
 * resolved port in Local State as `browseros.server.cdp_port`; the default is
 * 9110 for neo and 9100 for BrowserOS when the port was free. Reading Local
 * State first finds a port the browser had to move.
 */

import { existsSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

/** `kProductPathName` per product (chrome/install_static patches). */
const PRODUCTS = [
  { label: 'BrowserOS neo', pathName: 'BrowserClaw', defaultPort: 9110 },
  { label: 'BrowserOS', pathName: 'BrowserOS', defaultPort: 9100 },
] as const

export interface BrowserEndpoint {
  port: number
  wsUrl: string
  /** `Browser` from /json/version, e.g. `Chrome/155.0.8059.26`. */
  product: string
}

export function userDataDir(
  pathName: string,
  env: NodeJS.ProcessEnv = process.env,
  platform: NodeJS.Platform = process.platform,
): string {
  if (platform === 'win32') {
    const local = env.LOCALAPPDATA ?? join(homedir(), 'AppData', 'Local')
    return join(local, pathName, 'User Data')
  }
  if (platform === 'darwin') {
    return join(homedir(), 'Library', 'Application Support', pathName)
  }
  return join(homedir(), '.config', pathName.toLowerCase())
}

export function cdpPortFromLocalState(json: string): number | undefined {
  const port = (
    JSON.parse(json) as { browseros?: { server?: { cdp_port?: unknown } } }
  ).browseros?.server?.cdp_port
  return typeof port === 'number' && port > 0 ? port : undefined
}

export function candidatePorts(explicit?: number): number[] {
  if (explicit) return [explicit]
  const ports: number[] = []
  for (const product of PRODUCTS) {
    const localState = join(userDataDir(product.pathName), 'Local State')
    if (!existsSync(localState)) continue
    try {
      const port = cdpPortFromLocalState(readFileSync(localState, 'utf8'))
      if (port) ports.push(port)
    } catch {
      // Local State is rewritten in place by the browser; a torn read just
      // falls back to the defaults below.
    }
  }
  for (const product of PRODUCTS) ports.push(product.defaultPort)
  return [...new Set(ports)]
}

export async function findBrowser(
  explicitPort?: number,
): Promise<BrowserEndpoint | undefined> {
  for (const port of candidatePorts(explicitPort)) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`, {
        signal: AbortSignal.timeout(1500),
      })
      if (!response.ok) continue
      const info = (await response.json()) as {
        webSocketDebuggerUrl?: string
        Browser?: string
      }
      if (info.webSocketDebuggerUrl) {
        return {
          port,
          wsUrl: info.webSocketDebuggerUrl,
          product: info.Browser ?? 'unknown',
        }
      }
    } catch {
      // Nothing listening on this port; try the next candidate.
    }
  }
  return undefined
}
