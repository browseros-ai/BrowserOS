/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { cors } from 'hono/cors'

type CorsOptions = Parameters<typeof cors>[0]

const STATIC_ALLOWED_ORIGINS = new Set<string>([
  'chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk',
])

let cachedAllowedOrigins: Set<string> | null = null

function buildAllowedOrigins(): Set<string> {
  const fromEnv = (process.env.BROWSEROS_TRUSTED_ORIGINS ?? '')
    .split(',')
    .map((value) => value.trim())
    .filter((value) => value.length > 0)
  return new Set([...STATIC_ALLOWED_ORIGINS, ...fromEnv])
}

function getAllowedOrigins(): Set<string> {
  if (!cachedAllowedOrigins) {
    cachedAllowedOrigins = buildAllowedOrigins()
  }
  return cachedAllowedOrigins
}

export function resetAllowedOriginsForTesting(): void {
  cachedAllowedOrigins = null
}

export function isAllowedOrigin(origin: string): boolean {
  return getAllowedOrigins().has(origin)
}

export const defaultCorsConfig: CorsOptions = {
  origin: (origin: string | undefined) => {
    if (origin && isAllowedOrigin(origin)) return origin
    return null
  },
  allowMethods: ['GET', 'POST', 'DELETE', 'OPTIONS'],
  allowHeaders: ['Content-Type', 'Authorization', 'Accept'],
  credentials: true,
}
