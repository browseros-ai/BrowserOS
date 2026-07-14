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
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { env } from '../../src/env'
import {
  bucketClientName,
  captureEvent,
  getTelemetryState,
  resetAnalyticsForTesting,
  sanitize,
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

describe('telemetry state + disabled default', () => {
  let dir: string
  let prior: string | undefined

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'claw-analytics-'))
    prior = env.browserClawDirOverride
    env.browserClawDirOverride = dir
    resetAnalyticsForTesting()
  })

  afterEach(() => {
    env.browserClawDirOverride = prior
    resetAnalyticsForTesting()
    rmSync(dir, { recursive: true, force: true })
  })

  it('mints and persists an anonymous uuid, enabled by default', () => {
    const state = getTelemetryState()
    expect(state.enabled).toBe(true)
    expect(state.distinctId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    )
    const onDisk = JSON.parse(readFileSync(join(dir, 'analytics.json'), 'utf8'))
    expect(onDisk.distinctId).toBe(state.distinctId)
  })

  it('is a no-op (no throw) when no PostHog key is configured', () => {
    // Default dev/test env has no CLAW_POSTHOG_KEY, so no client is
    // constructed and capture must be a safe no-op.
    expect(() => captureEvent('server_started')).not.toThrow()
    expect(() =>
      captureEvent('harness_connected', { harness: 'Zed' }),
    ).not.toThrow()
  })
})
