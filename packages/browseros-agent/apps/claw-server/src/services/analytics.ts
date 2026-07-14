/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Anonymous, privacy-first product analytics (PostHog).
 *
 * Principle: measure the product, never the user. We report only
 * metadata about which features get used (a session opened, which
 * agent connected, a harness was linked). We NEVER send urls, page
 * titles/content, prompts, tool arguments or results, screenshots,
 * agent labels, file paths, tokens, or emails. Two defenses keep
 * that true regardless of call sites:
 *
 *   1. `sanitize()` is an ALLOW-LIST: any property key not in
 *      `SAFE_KEYS` is dropped, and any value that looks like a url,
 *      email, or path is dropped, so a mistake at a call site cannot
 *      leak content.
 *   2. Free-text-ish fields (`client_name`) are bucketed to a known
 *      set via `bucketClientName`; anything else becomes `"other"`.
 *
 * Identity is a single anonymous UUID generated once and persisted at
 * `<clawServerDir>/analytics.json` alongside the user's opt-out flag.
 * No `identify`, no PII. The same id is served to the cockpit UI via
 * `/system/telemetry` so both surfaces share one anonymous install.
 *
 * Analytics is OFF unless a project write key is configured
 * (`CLAW_POSTHOG_KEY`), the operator kill-switch is on
 * (`CLAW_ANALYTICS_ENABLED`, default on), and the user has not opted
 * out. When off, no client is constructed and every capture no-ops.
 */

import { randomUUID } from 'node:crypto'
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { PostHog } from 'posthog-node'
import { env } from '../env'
import { getClawServerDir } from '../lib/browserclaw-dir'
import { logger } from '../lib/logger'
import { VERSION } from '../version'

const ANALYTICS_FILE = 'analytics.json'

/** Property keys allowed to leave the machine. Anything else is dropped. */
const SAFE_KEYS: ReadonlySet<string> = new Set([
  'client_name',
  'harness',
  'kind',
  'server_version',
  'os_platform',
  'enabled',
])

/**
 * MCP client names we recognise. Anything else buckets to `"other"`
 * so a custom or self-built client can never leak a user-authored
 * string as a property value.
 */
const KNOWN_CLIENTS: ReadonlySet<string> = new Set([
  'claude-desktop',
  'claude-code',
  'claude-ai',
  'cursor',
  'vscode',
  'vscode-insiders',
  'codex',
  'zed',
  'opencode',
  'antigravity',
  'windsurf',
  'cline',
  'continue',
  'goose',
])

export interface AnalyticsState {
  distinctId: string
  enabled: boolean
}

let state: AnalyticsState | null = null
let client: PostHog | null = null
let initialised = false

function analyticsPath(): string {
  return join(getClawServerDir(), ANALYTICS_FILE)
}

function persistState(next: AnalyticsState): void {
  const dir = getClawServerDir()
  const path = analyticsPath()
  const tmp = `${path}.tmp`
  try {
    mkdirSync(dir, { recursive: true })
    writeFileSync(tmp, `${JSON.stringify(next, null, 2)}\n`, 'utf8')
    renameSync(tmp, path)
  } catch (err) {
    logger.warn('analytics state write failed', {
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

/**
 * Reads the persisted anonymous id + opt-out flag, generating and
 * writing a fresh anonymous UUID on first run. A missing or corrupt
 * file regenerates rather than throwing.
 */
function loadOrCreateState(): AnalyticsState {
  try {
    const parsed = JSON.parse(readFileSync(analyticsPath(), 'utf8')) as Partial<
      Record<keyof AnalyticsState, unknown>
    >
    if (typeof parsed.distinctId === 'string' && parsed.distinctId.length > 0) {
      return {
        distinctId: parsed.distinctId,
        enabled: parsed.enabled !== false,
      }
    }
  } catch {
    // Missing or unreadable: fall through and mint a fresh id.
  }
  const fresh: AnalyticsState = { distinctId: randomUUID(), enabled: true }
  persistState(fresh)
  return fresh
}

function ensureInit(): void {
  if (initialised) return
  initialised = true
  state = loadOrCreateState()

  const disabledReason = !env.posthogKey
    ? 'no-key'
    : !env.analyticsEnabledByEnv
      ? 'env-off'
      : !state.enabled
        ? 'user-opt-out'
        : null
  if (disabledReason) {
    logger.info('analytics disabled', { reason: disabledReason })
    return
  }

  client = new PostHog(env.posthogKey as string, {
    host: env.posthogHost,
    // Local, low-volume process: flush each event promptly so a
    // browser close does not strand the last session event, and
    // shutdownAnalytics() drains anything in flight.
    flushAt: 1,
    flushInterval: 0,
  })
  logger.info('analytics enabled', { host: env.posthogHost })
}

/** Slug-and-bucket an MCP client name to a known token or `"other"`. */
export function bucketClientName(raw: string): string {
  const slug = raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return KNOWN_CLIENTS.has(slug) ? slug : 'other'
}

/**
 * A value is rejected if it looks like it could carry user content:
 * a url, an email, a filesystem path, or an unexpectedly long string.
 * All legitimate event values (short enum tokens, versions, booleans)
 * pass; anything content-shaped is dropped.
 */
function looksSensitive(value: unknown): boolean {
  if (typeof value !== 'string') return false
  return (
    /https?:\/\//i.test(value) ||
    value.includes('@') ||
    value.includes('/') ||
    value.includes('\\') ||
    value.length > 48
  )
}

/** Allow-list keys + drop content-shaped values. Defense in depth. */
export function sanitize(
  props: Record<string, unknown>,
): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(props)) {
    if (!SAFE_KEYS.has(key)) continue
    if (looksSensitive(value)) continue
    out[key] = value
  }
  return out
}

/**
 * Fire-and-forget capture. No-ops when analytics is disabled. Every
 * event carries the anonymous install id plus server version + OS,
 * and its properties are sanitized before send. Never throws.
 */
export function captureEvent(
  event: string,
  props: Record<string, unknown> = {},
): void {
  ensureInit()
  if (!client || !state) return
  try {
    client.capture({
      distinctId: state.distinctId,
      event,
      properties: {
        server_version: VERSION,
        os_platform: process.platform,
        ...sanitize(props),
      },
    })
  } catch (err) {
    logger.warn('analytics capture failed', {
      event,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

/**
 * The anonymous id + enabled flag surfaced to the cockpit UI so it
 * can share the same install identity and reflect the opt-out state.
 */
export function getTelemetryState(): AnalyticsState {
  ensureInit()
  return state ?? { distinctId: '', enabled: false }
}

/** Flush pending events. Called from the boot shutdown path. */
export async function shutdownAnalytics(): Promise<void> {
  if (!client) return
  try {
    await client.shutdown()
  } catch {
    // Best-effort flush on exit; nothing to recover.
  }
}

/** Test-only: reset module state so each test starts clean. */
export function resetAnalyticsForTesting(): void {
  state = null
  client = null
  initialised = false
}
