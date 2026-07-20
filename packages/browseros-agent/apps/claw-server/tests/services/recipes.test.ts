/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Recipes module unit coverage. Locks the host-bucket rule that keeps
 * subdomain-scoped apps from colliding, and the directory shape that
 * layers shared + agent overlays.
 */

import { describe, expect, it } from 'bun:test'
import { sep } from 'node:path'
import {
  agentRecipesDirFor,
  hostBucketFromUrl,
  sharedRecipesDirFor,
} from '../../src/services/recipes'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

describe('hostBucketFromUrl', () => {
  it('strips a leading www. and keeps the rest of the hostname', () => {
    expect(hostBucketFromUrl('https://www.linkedin.com/')).toBe('linkedin.com')
    expect(hostBucketFromUrl('https://linkedin.com/feed')).toBe('linkedin.com')
  })

  it('keeps meaningful subdomains distinct (no collision on apex)', () => {
    // Fix 2: docs / mail / drive on google.com must NOT bucket together.
    expect(hostBucketFromUrl('https://docs.google.com/')).toBe(
      'docs.google.com',
    )
    expect(hostBucketFromUrl('https://mail.google.com/')).toBe(
      'mail.google.com',
    )
    expect(hostBucketFromUrl('https://drive.google.com/')).toBe(
      'drive.google.com',
    )
  })

  it('keeps regional prefixes (AWS console) distinct', () => {
    expect(
      hostBucketFromUrl('https://us-east-1.console.aws.amazon.com/s3'),
    ).toBe('us-east-1.console.aws.amazon.com')
    expect(hostBucketFromUrl('https://s3.console.aws.amazon.com/')).toBe(
      's3.console.aws.amazon.com',
    )
  })

  it('returns null for non-http(s) schemes', () => {
    expect(hostBucketFromUrl('chrome://newtab')).toBeNull()
    expect(hostBucketFromUrl('file:///tmp/index.html')).toBeNull()
    expect(hostBucketFromUrl('about:blank')).toBeNull()
  })

  it('returns null for malformed URLs', () => {
    expect(hostBucketFromUrl('not-a-url')).toBeNull()
    expect(hostBucketFromUrl('')).toBeNull()
  })

  it('lowercases via URL parsing but preserves internal casing', () => {
    // URL parser lowercases hostnames per RFC. Just lock the behaviour.
    expect(hostBucketFromUrl('https://LinkedIn.COM/')).toBe('linkedin.com')
  })
})

describe('recipe directory paths', () => {
  it('shared dir sits under recipes/shared/<host>', async () => {
    await withTempBrowserClawDir(async (root) => {
      const dir = sharedRecipesDirFor('linkedin.com')
      expect(dir.startsWith(root)).toBe(true)
      expect(dir.endsWith(`recipes${sep}shared${sep}linkedin.com`)).toBe(true)
    })
  })

  it('agent overlay dir sits under recipes/agents/<slug>/<host>', async () => {
    await withTempBrowserClawDir(async (root) => {
      const dir = agentRecipesDirFor('claude-code', 'docs.google.com')
      expect(dir.startsWith(root)).toBe(true)
      expect(
        dir.endsWith(
          `recipes${sep}agents${sep}claude-code${sep}docs.google.com`,
        ),
      ).toBe(true)
    })
  })

  it('agent overlay dirs for different slugs are disjoint', async () => {
    await withTempBrowserClawDir(async () => {
      const claude = agentRecipesDirFor('claude-code', 'linkedin.com')
      const codex = agentRecipesDirFor('codex-cli', 'linkedin.com')
      expect(claude).not.toBe(codex)
    })
  })
})
