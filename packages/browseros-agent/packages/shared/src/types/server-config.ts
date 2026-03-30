/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shape of ~/.browseros/server.json written by the server on startup.
 * The CLI reads this file for auto-discovery of the server URL.
 */

export interface ServerDiscoveryConfig {
  server_port: number
  url: string
  server_version: string
  browseros_version?: string
  chromium_version?: string
  browseros_id?: string
}
