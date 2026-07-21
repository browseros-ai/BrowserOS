/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Server-side MCP URL helpers. Browser/UI imports the shared URL
 * constants directly so env/config parsing stays out of WXT bundles.
 */

export {
  BROWSEROS_MCP_SERVER_NAME,
  canonicalMcpUrlForPort,
  MCP_PATH,
} from '@browseros/shared/constants/urls'

import { canonicalMcpUrlForPort } from '@browseros/shared/constants/urls'
import { env } from '../env'

/** Resolves the public server-side MCP URL from runtime ports. */
export function publicMcpUrl(): string {
  return canonicalMcpUrlForPort(env.proxyPort ?? env.serverPort)
}
