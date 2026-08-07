/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Privacy guard for analytics. The whole point of the feature is that
 * it can never carry user content, so these tests pin the two defenses
 * (allow-list sanitizer + client-name bucketing) that enforce it.
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { env } from '../../src/env'
import {
  bucketClientName,
  captureEvent,
  getTelemetryState,
  resetAnalyticsForTesting,
  sanitize,
  setTelemetryConsent,
} from '../../src/services/analytics'

describe('sanitize (allow-list + value guard)', () => {
  it('drops any property key not on the allow-list', () => {
    const out = sanitize({
      client_name: 'cursor',
      // none of these may ever survive
      url: 'http://127.0.0.1:9200/mcp',
      title: 'Inbox',
      agent_label: 'my agent',
      session_id: 'abc',
      prompt: 'do the thing',
    })
    expect(out).toEqual({ client_name: 'cursor' })
  })

  it('drops values that look like user content even on allowed keys', () => {
    // A bug at a call site puts content into an allowed key; the value
    // guard still refuses it.
    expect(sanitize({ client_name: 'https://example.com' })).toEqual({})
    expect(sanitize({ client_name: 'user@example.com' })).toEqual({})
    expect(sanitize({ client_name: '/home/user/secret' })).toEqual({})
    expect(sanitize({ client_name: 'C:\\Users\\someone' })).toEqual({})
    expect(sanitize({ client_name: 'x'.repeat(64) })).toEqual({})
  })

  it('keeps legitimate short metadata values', () => {
    expect(
      sanitize({
        client_name: 'claude-code',
        harness: 'VS Code',
        kind: 'closed',
        enabled: true,
      }),
    ).toEqual({
      client_name: 'claude-code',
      harness: 'VS Code',
      kind: 'closed',
      enabled: true,
    })
  })
})

describe('bucketClientName', () => {
  it('maps known clients to their slug', () => {
    expect(bucketClientName('Claude Code')).toBe('claude-code')
    expect(bucketClientName('cursor')).toBe('cursor')
    expect(bucketClientName('VSCode')).toBe('vscode')
  })

  it('buckets anything unknown to "other" so custom names cannot leak', () => {
    expect(bucketClientName('my-secret-internal-tool')).toBe('other')
    expect(bucketClientName('user@example.com')).toBe('other')
    expect(bucketClientName('')).toBe('other')
  })
})

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

describe('telemetry state + gating', () => {
  let dir: string
  let priorDir: string | undefined
  let priorKey: string | undefined
  let priorEnvEnabled: boolean

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'claw-analytics-'))
    priorDir = env.browserClawDirOverride
    priorKey = env.posthogKey
    priorEnvEnabled = env.analyticsEnabledByEnv
    env.browserClawDirOverride = dir
    env.posthogKey = undefined
    env.analyticsEnabledByEnv = true
    resetAnalyticsForTesting()
  })

  afterEach(() => {
    env.browserClawDirOverride = priorDir
    env.posthogKey = priorKey
    env.analyticsEnabledByEnv = priorEnvEnabled
    resetAnalyticsForTesting()
    rmSync(dir, { recursive: true, force: true })
  })

  it('mints and persists an anonymous uuid with consent on by default', () => {
    const state = getTelemetryState()
    expect(state.distinctId).toMatch(UUID_RE)
    const onDisk = JSON.parse(readFileSync(join(dir, 'analytics.json'), 'utf8'))
    expect(onDisk.distinctId).toBe(state.distinctId)
    expect(onDisk.enabled).toBe(true)
  })

  it('reports enabled=false without a key, even with consent on', () => {
    // The reported flag is the EFFECTIVE state, so the UI never shows
    // telemetry as on while every capture is a no-op.
    expect(getTelemetryState().enabled).toBe(false)
  })

  it('reports enabled=true only when key + kill-switch + consent all pass', () => {
    env.posthogKey = 'phc_test'
    resetAnalyticsForTesting()
    expect(getTelemetryState().enabled).toBe(true)
    // Operator kill-switch overrides consent.
    env.analyticsEnabledByEnv = false
    resetAnalyticsForTesting()
    expect(getTelemetryState().enabled).toBe(false)
  })

  it('fails closed on a corrupt state file and leaves it untouched', () => {
    const path = join(dir, 'analytics.json')
    writeFileSync(path, '{ not valid json', 'utf8')
    env.posthogKey = 'phc_test'
    resetAnalyticsForTesting()
    // A prior opt-out might be hiding in the corrupt file, so we must
    // not silently re-enable.
    expect(getTelemetryState().enabled).toBe(false)
    expect(readFileSync(path, 'utf8')).toBe('{ not valid json')
  })

  it('honours a persisted opt-out even with a key present', () => {
    writeFileSync(
      join(dir, 'analytics.json'),
      JSON.stringify({ distinctId: 'abc', enabled: false }),
      'utf8',
    )
    env.posthogKey = 'phc_test'
    resetAnalyticsForTesting()
    expect(getTelemetryState().enabled).toBe(false)
  })

  it('is a no-op (no throw) when no PostHog key is configured', () => {
    expect(() => captureEvent('server_started')).not.toThrow()
    expect(() =>
      captureEvent('harness_connected', { harness: 'Zed' }),
    ).not.toThrow()
  })

  it('exposes the raw consent flag alongside the effective enabled flag', () => {
    // no key → effective off, but consent (user choice) is on by default
    const state = getTelemetryState()
    expect(state.enabled).toBe(false)
    expect(state.consent).toBe(true)
  })

  it('setTelemetryConsent persists the choice and keeps the same id', () => {
    const before = getTelemetryState()
    const off = setTelemetryConsent(false)
    expect(off.consent).toBe(false)
    expect(off.enabled).toBe(false)
    expect(off.distinctId).toBe(before.distinctId)
    // persisted to disk
    const onDisk = JSON.parse(readFileSync(join(dir, 'analytics.json'), 'utf8'))
    expect(onDisk.enabled).toBe(false)
    expect(onDisk.distinctId).toBe(before.distinctId)
    // re-reading reflects the opt-out
    resetAnalyticsForTesting()
    expect(getTelemetryState().consent).toBe(false)
    // turning back on
    expect(setTelemetryConsent(true).consent).toBe(true)
  })
})
