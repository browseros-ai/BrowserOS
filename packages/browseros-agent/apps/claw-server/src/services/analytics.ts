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
let stateLoaded = false
let clientInitialised = false

function analyticsPath(): string {
  return join(getClawServerDir(), ANALYTICS_FILE)
}

function persistState(next: AnalyticsState): boolean {
  const dir = getClawServerDir()
  const path = analyticsPath()
  const tmp = `${path}.tmp`
  try {
    mkdirSync(dir, { recursive: true })
    writeFileSync(tmp, `${JSON.stringify(next, null, 2)}\n`, 'utf8')
    renameSync(tmp, path)
    return true
  } catch (err) {
    logger.warn('analytics state write failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    return false
  }
}

/**
 * Reads the persisted anonymous id + opt-out flag. A genuinely MISSING
 * file (first run) mints a fresh id with consent on (the opt-out
 * default). A file that EXISTS but is unreadable or corrupt fails
 * CLOSED (consent off) and is left untouched, so a prior opt-out can
 * never be silently lost.
 */
function loadOrCreateState(): AnalyticsState {
  let raw: string
  try {
    raw = readFileSync(analyticsPath(), 'utf8')
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
      // First run: no prior choice exists, so honour the opt-out
      // default (telemetry on) and persist a fresh anonymous id.
      const fresh: AnalyticsState = { distinctId: randomUUID(), enabled: true }
      persistState(fresh)
      return fresh
    }
    // The file exists but could not be read (permissions, IO). A prior
    // opt-out may be hiding in it, so fail CLOSED and do not overwrite.
    logger.warn('analytics state unreadable; disabling to preserve consent', {
      error: err instanceof Error ? err.message : String(err),
    })
    return { distinctId: randomUUID(), enabled: false }
  }
  try {
    const parsed = JSON.parse(raw) as Partial<
      Record<keyof AnalyticsState, unknown>
    >
    if (typeof parsed.distinctId === 'string' && parsed.distinctId.length > 0) {
      return {
        distinctId: parsed.distinctId,
        enabled: parsed.enabled !== false,
      }
    }
  } catch {
    // Fall through to the fail-closed path below.
  }
  // The file exists but is corrupt or missing an id. We cannot tell
  // whether the user had opted out, so fail CLOSED and leave the file
  // untouched for recovery rather than silently re-enabling.
  logger.warn('analytics state corrupt; disabling to preserve consent')
  return { distinctId: randomUUID(), enabled: false }
}

/**
 * Every gate that must pass before telemetry is actually active on
 * this server: a configured write key, the operator kill-switch on,
 * and the user's consent. This is the single source of truth reported
 * to the UI and used to decide whether to construct the client, so the
 * two can never disagree.
 */
function effectiveEnabled(s: AnalyticsState | null): boolean {
  return Boolean(s?.enabled && env.analyticsEnabledByEnv && env.posthogKey)
}

/** Loads the anonymous id + consent flag. Does NOT construct a client. */
function ensureState(): void {
  if (stateLoaded) return
  stateLoaded = true
  state = loadOrCreateState()
}

/** Constructs the PostHog client iff every gate in `effectiveEnabled` passes. */
function ensureClient(): void {
  ensureState()
  if (clientInitialised) return
  clientInitialised = true
  if (!effectiveEnabled(state)) {
    const reason = !env.posthogKey
      ? 'no-key'
      : !env.analyticsEnabledByEnv
        ? 'env-off'
        : 'user-opt-out'
    logger.info('analytics disabled', { reason })
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
  ensureClient()
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

export interface TelemetryState {
  distinctId: string
  /** Effective: whether the SERVER is actually sending (key + kill-switch + consent). */
  enabled: boolean
  /** The user's consent choice. What the cockpit toggle reflects and both surfaces gate on. */
  consent: boolean
}

/**
 * The anonymous id, the EFFECTIVE enabled flag, and the raw consent
 * flag surfaced to the cockpit UI. `enabled` reflects whether the
 * server is actively sending (so the UI never shows telemetry on while
 * every capture is a no-op); `consent` is the user's choice, which the
 * toggle binds to and the extension gates its own capture on. Loads
 * state but never constructs a network client.
 */
export function getTelemetryState(): TelemetryState {
  ensureState()
  return {
    distinctId: state?.distinctId ?? '',
    enabled: effectiveEnabled(state),
    consent: state?.enabled ?? false,
  }
}

/**
 * Persists the user's consent choice (the cockpit opt-out toggle) and
 * re-evaluates the client so a turn-off stops sending immediately and a
 * turn-on can re-init on the next capture. Returns the updated state.
 */
export function setTelemetryConsent(consent: boolean): TelemetryState {
  ensureState()
  const next: AnalyticsState = {
    distinctId: state?.distinctId ?? randomUUID(),
    enabled: consent,
  }
  // Write to disk BEFORE flipping in-memory state, and surface a failed
  // write at error level: a swallowed failure could stop telemetry for
  // this process yet silently revert to the old on-disk choice on the
  // next restart. We still apply in-memory so the choice holds for this
  // session, but the durability gap is no longer hidden.
  if (!persistState(next)) {
    logger.error(
      'analytics consent write failed; choice may not survive a restart',
      { consent },
    )
  }
  state = next
  // Force re-evaluation on the next capture; drop any live client so a
  // withdrawn consent stops sending right away.
  clientInitialised = false
  if (client) {
    void client.shutdown().catch(() => {})
    client = null
  }
  return {
    distinctId: next.distinctId,
    enabled: effectiveEnabled(next),
    consent: next.enabled,
  }
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
  stateLoaded = false
  clientInitialised = false
}
