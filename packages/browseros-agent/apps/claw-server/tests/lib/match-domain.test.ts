/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { matchDomain } from '../../src/lib/match-domain'

describe('matchDomain', () => {
  test('exact match is case insensitive', () => {
    expect(matchDomain('stripe.com', 'stripe.com')).toBe(true)
    expect(matchDomain('STRIPE.com', 'stripe.com')).toBe(true)
    expect(matchDomain('stripe.com', 'STRIPE.COM')).toBe(true)
  })

  test('exact match rejects unrelated domains', () => {
    expect(matchDomain('stripe.com', 'evil-stripe.com')).toBe(false)
    expect(matchDomain('stripe.com', 'stripe.com.evil')).toBe(false)
    expect(matchDomain('stripe.com', 'api.stripe.com')).toBe(false)
  })

  test('leading wildcard matches subdomains, not the apex', () => {
    expect(matchDomain('*.example.com', 'foo.example.com')).toBe(true)
    expect(matchDomain('*.example.com', 'a.b.example.com')).toBe(true)
    // The apex (no subdomain label) must NOT match: `.example.com`
    // would normalise to `example.com` which a user wanting to also
    // cover the apex should add as a separate rule.
    expect(matchDomain('*.example.com', 'example.com')).toBe(false)
  })

  test('trailing wildcard matches anything after the dot', () => {
    expect(matchDomain('admin.*', 'admin.example.com')).toBe(true)
    expect(matchDomain('admin.*', 'admin.foo')).toBe(true)
    // Trailing wildcard requires at least one character after the dot.
    expect(matchDomain('admin.*', 'admin.')).toBe(false)
    // The bare label must NOT match the wildcard expansion.
    expect(matchDomain('admin.*', 'admin')).toBe(false)
  })

  test('bare star matches any non-empty domain', () => {
    expect(matchDomain('*', 'example.com')).toBe(true)
    expect(matchDomain('*', 'a')).toBe(true)
    expect(matchDomain('*', '')).toBe(false)
  })

  test('star in the middle works for vendor.product patterns', () => {
    expect(matchDomain('app.*.com', 'app.stripe.com')).toBe(true)
    expect(matchDomain('app.*.com', 'app.foo.bar.com')).toBe(true)
    expect(matchDomain('app.*.com', 'web.stripe.com')).toBe(false)
  })

  test('empty pattern never matches', () => {
    expect(matchDomain('', 'example.com')).toBe(false)
    expect(matchDomain('', '')).toBe(false)
  })

  test('regex metacharacters in patterns are treated as literals', () => {
    // `.` matters: `a.b` must not match `aXb`.
    expect(matchDomain('a.b', 'a.b')).toBe(true)
    expect(matchDomain('a.b', 'aXb')).toBe(false)
    // `+` and `(` should not be interpreted as regex operators.
    expect(matchDomain('foo+bar.com', 'foo+bar.com')).toBe(true)
    expect(matchDomain('foo+bar.com', 'foobar.com')).toBe(false)
    expect(matchDomain('a(b).com', 'a(b).com')).toBe(true)
  })

  test('domain comparison is case insensitive', () => {
    expect(matchDomain('*.Example.com', 'foo.example.com')).toBe(true)
    expect(matchDomain('*.example.com', 'FOO.example.COM')).toBe(true)
  })
})
