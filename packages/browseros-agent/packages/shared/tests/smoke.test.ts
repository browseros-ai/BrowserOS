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

import {
  compileAclTerms,
  findMatchingRules,
  matchesElement,
  matchesSitePattern,
} from '@browseros/shared/acl/match'
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
import { UIMessageStreamEventSchema } from '@browseros/shared/schemas/ui-stream'
import { sanitize, sanitizeEvent } from '@browseros/shared/sentry/sanitize'
import type { AclRule, ElementProperties } from '@browseros/shared/types/acl'

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

  it('compileAclTerms includes the normalized textMatch', () => {
    const rule: AclRule = {
      id: 'r1',
      sitePattern: 'example.com',
      textMatch: 'Submit',
      enabled: true,
    }
    assert.ok(compileAclTerms(rule).includes('submit'))
  })

  it('matchesElement matches by selector and by text', () => {
    const button: ElementProperties = {
      tagName: 'button',
      textContent: 'Submit order',
      attributes: {},
    }
    const bySelector: AclRule = {
      id: 'r2',
      sitePattern: '*',
      selector: 'button',
      enabled: true,
    }
    const byText: AclRule = {
      id: 'r3',
      sitePattern: '*',
      textMatch: 'submit',
      enabled: true,
    }
    const noMatch: AclRule = {
      id: 'r4',
      sitePattern: '*',
      textMatch: 'delete account',
      enabled: true,
    }
    assert.strictEqual(matchesElement(button, bySelector), true)
    assert.strictEqual(matchesElement(button, byText), true)
    assert.strictEqual(matchesElement(button, noMatch), false)
  })

  it('findMatchingRules filters by site and enabled flag', () => {
    const props: ElementProperties = {
      tagName: 'button',
      textContent: 'Buy now',
      attributes: {},
    }
    const rules: AclRule[] = [
      {
        id: 'a',
        sitePattern: 'example.com',
        selector: 'button',
        enabled: true,
      },
      { id: 'b', sitePattern: 'other.com', selector: 'button', enabled: true },
      {
        id: 'c',
        sitePattern: 'example.com',
        selector: 'button',
        enabled: false,
      },
    ]
    const matched = findMatchingRules('https://example.com/p', props, rules)
    assert.deepStrictEqual(
      matched.map((r) => r.id),
      ['a'],
    )
  })
})

describe('@browseros/shared schemas/ui-stream', () => {
  it('accepts well-formed stream events', () => {
    for (const event of [
      { type: 'start' },
      { type: 'finish', finishReason: 'stop' },
      { type: 'error', errorText: 'boom' },
      { type: 'text-start', id: 'm1' },
    ]) {
      assert.strictEqual(
        UIMessageStreamEventSchema.safeParse(event).success,
        true,
        `expected ${event.type} to validate`,
      )
    }
  })

  it('rejects an unknown event type', () => {
    assert.strictEqual(
      UIMessageStreamEventSchema.safeParse({ type: 'not-a-real-event' })
        .success,
      false,
    )
  })

  it('rejects an event missing a required field', () => {
    // 'finish' requires finishReason
    assert.strictEqual(
      UIMessageStreamEventSchema.safeParse({ type: 'finish' }).success,
      false,
    )
  })
})

describe('@browseros/shared sentry/sanitizeEvent', () => {
  it('redacts sensitive values inside breadcrumbs, contexts and extra', () => {
    const event = {
      breadcrumbs: [
        { category: 'http', data: { apiKey: 'sk-leak', url: '/x' } },
      ],
      contexts: { auth: { token: 'leak', userId: 'u1' } },
      extra: { password: 'hunter2', note: 'fine' },
    }
    const cleaned = sanitizeEvent(event) as typeof event & {
      contexts: { auth: { token: string; userId: string } }
      extra: { password: string; note: string }
    }
    assert.strictEqual(cleaned.breadcrumbs[0].data.apiKey, '[REDACTED]')
    assert.strictEqual(cleaned.breadcrumbs[0].data.url, '/x')
    assert.strictEqual(cleaned.contexts.auth.token, '[REDACTED]')
    assert.strictEqual(cleaned.contexts.auth.userId, 'u1')
    assert.strictEqual(cleaned.extra.password, '[REDACTED]')
    assert.strictEqual(cleaned.extra.note, 'fine')
  })
})
