/**
 * @license
 * Copyright 2025 BrowserOS
 *
 * Build-time inlined environment variables.
 *
 * IMPORTANT: Values here are replaced at build time by Bun's `--env inline` flag.
 * The `process.env.X` access MUST be direct (not via a variable) for inlining to work.
 *
 * These variables are:
 * - Replaced with literal strings in production builds
 * - Read from actual env vars during development
 *
 * For runtime-only env vars (like BROWSEROS_CDP_PORT), use process.env directly.
 */

export const INLINED_ENV = {
  SENTRY_DSN: process.env.SENTRY_DSN,
  CODEGEN_SERVICE_URL: process.env.CODEGEN_SERVICE_URL,
  POSTHOG_API_KEY: process.env.POSTHOG_API_KEY,
  BROWSEROS_CONFIG_URL: process.env.BROWSEROS_CONFIG_URL,
  BROWSEROS_VM_CACHE_PREFETCH: process.env.BROWSEROS_VM_CACHE_PREFETCH,
  BROWSEROS_VM_CACHE_MANIFEST_URL: process.env.BROWSEROS_VM_CACHE_MANIFEST_URL,
  SKILLS_CATALOG_URL: process.env.SKILLS_CATALOG_URL,
} as const

export const REQUIRED_FOR_PRODUCTION = [
  'SENTRY_DSN',
  'CODEGEN_SERVICE_URL',
  'POSTHOG_API_KEY',
  'BROWSEROS_CONFIG_URL',
  'BROWSEROS_VM_CACHE_PREFETCH',
  'BROWSEROS_VM_CACHE_MANIFEST_URL',
] as const satisfies readonly (keyof typeof INLINED_ENV)[]
