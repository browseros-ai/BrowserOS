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
  POSTHOG_API_KEY: process.env.POSTHOG_API_KEY,
  BROWSEROS_CONFIG_URL: process.env.BROWSEROS_CONFIG_URL,
  // Optional. When present, the Remote Hermes provider is wired up at
  // startup (logger.warn if absent). Symmetric secret shared with the
  // Cloudflare agent-control-worker; never sent to the extension.
  AGENT_RUNNER_JWT_SECRET: process.env.AGENT_RUNNER_JWT_SECRET,
} as const

export const REQUIRED_FOR_PRODUCTION = [
  'SENTRY_DSN',
  'POSTHOG_API_KEY',
  'BROWSEROS_CONFIG_URL',
] as const satisfies readonly (keyof typeof INLINED_ENV)[]
