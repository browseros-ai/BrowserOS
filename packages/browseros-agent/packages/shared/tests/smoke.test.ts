/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Smoke tests for the foundational @browseros/shared package. These guard the
 * public export surface other packages depend on: constant values stay defined,
 * Zod schemas validate/reject correctly, sensitive data is redacted, and the
 * ACL matcher behaves on basic cases.
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'

import { matchesSitePattern } from '@browseros/shared/acl/match'
import {
  AGENT_LIMITS,
  CONTENT_LIMITS,
} from '@browseros/shared/constants/limits'
import {
  DEFAULT_PORTS,
  DEV_PORTS,
  TEST_PORTS,
} from '@browseros/shared/constants/ports'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import { BrowserContextSchema } from '@browseros/shared/schemas/browser-context'
import { sanitize } from '@browseros/shared/sentry/sanitize'

describe('@browseros/shared constants', () => {
  it('exposes distinct, defined port sets', () => {
    for (const ports of [DEFAULT_PORTS, TEST_PORTS, DEV_PORTS]) {
      assert.ok(Number.isInteger(ports.cdp))
      assert.ok(Number.isInteger(ports.server))
      assert.ok(Number.isInteger(ports.extension))
    }
    // The three environments must not collide on the server port.
    const serverPorts = new Set([
      DEFAULT_PORTS.server,
      TEST_PORTS.server,
      DEV_PORTS.server,
    ])
    assert.strictEqual(serverPorts.size, 3)
  })

  it('exposes defined timeouts and limits', () => {
    assert.ok(TIMEOUTS && typeof TIMEOUTS === 'object')
    assert.ok(CONTENT_LIMITS && typeof CONTENT_LIMITS === 'object')
    assert.ok(AGENT_LIMITS && typeof AGENT_LIMITS === 'object')
  })

  it('exposes the z.ai endpoint among external URLs', () => {
    assert.ok(EXTERNAL_URLS.ZAI_API.startsWith('https://'))
  })
})

describe('@browseros/shared schemas', () => {
  it('BrowserContextSchema accepts a valid context', () => {
    const result = BrowserContextSchema.safeParse({
      windowId: 1,
      activeTab: { id: 10, url: 'https://example.com', title: 'Example' },
      tabs: [{ id: 10 }, { id: 11 }],
    })
    assert.strictEqual(result.success, true)
  })

  it('BrowserContextSchema rejects malformed tabs', () => {
    const result = BrowserContextSchema.safeParse({
      tabs: [{ id: 'not-a-number' }],
    })
    assert.strictEqual(result.success, false)
  })
})

describe('@browseros/shared sentry/sanitize', () => {
  it('redacts sensitive keys and preserves the rest', () => {
    const cleaned = sanitize({
      apiKey: 'sk-secret',
      nested: { authorization: 'Bearer xyz', model: 'glm-4.6' },
      safe: 'visible',
    }) as Record<string, unknown>
    assert.strictEqual(cleaned.apiKey, '[REDACTED]')
    assert.strictEqual(
      (cleaned.nested as Record<string, unknown>).authorization,
      '[REDACTED]',
    )
    assert.strictEqual(
      (cleaned.nested as Record<string, unknown>).model,
      'glm-4.6',
    )
    assert.strictEqual(cleaned.safe, 'visible')
  })
})

describe('@browseros/shared acl/match', () => {
  it('matches simple domains and the wildcard, rejects others', () => {
    assert.strictEqual(matchesSitePattern('https://example.com/x', '*'), true)
    assert.strictEqual(
      matchesSitePattern('https://sub.example.com/x', 'example.com'),
      true,
    )
    assert.strictEqual(
      matchesSitePattern('https://evil.com', 'example.com'),
      false,
    )
    assert.strictEqual(matchesSitePattern('not a url', 'example.com'), false)
  })
})
