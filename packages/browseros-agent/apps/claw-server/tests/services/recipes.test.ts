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
import { mkdirSync, writeFileSync } from 'node:fs'
import { sep } from 'node:path'
import {
  agentRecipesDirFor,
  describeRecipesForHost,
  hostBucketFromInput,
  hostBucketFromUrl,
  listRecipeFiles,
  readRecipeMetadata,
  STALE_THRESHOLD_DAYS,
  sharedRecipesDirFor,
} from '../../src/services/recipes'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

function seed(dir: string, name: string, body = '# stub'): string {
  mkdirSync(dir, { recursive: true })
  const path = `${dir}/${name}`
  writeFileSync(path, body)
  return path
}

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

describe('listRecipeFiles', () => {
  it('returns an empty list when neither layer exists', async () => {
    await withTempBrowserClawDir(async () => {
      expect(listRecipeFiles('claude-code', 'linkedin.com')).toEqual([])
    })
  })

  it('returns shared files with source=shared', async () => {
    await withTempBrowserClawDir(async () => {
      seed(sharedRecipesDirFor('linkedin.com'), 'invitation-manager.md')
      seed(sharedRecipesDirFor('linkedin.com'), 'messages.md')
      const entries = listRecipeFiles('claude-code', 'linkedin.com')
      expect(entries.map((e) => e.name)).toEqual([
        'invitation-manager.md',
        'messages.md',
      ])
      expect(entries.every((e) => e.source === 'shared')).toBe(true)
    })
  })

  it('returns agent-overlay files with source=agent', async () => {
    await withTempBrowserClawDir(async () => {
      seed(agentRecipesDirFor('codex-cli', 'linkedin.com'), 'codex-only.md')
      const entries = listRecipeFiles('codex-cli', 'linkedin.com')
      expect(entries).toEqual([
        {
          name: 'codex-only.md',
          absolutePath: expect.stringContaining(
            'recipes/agents/codex-cli/linkedin.com/codex-only.md',
          ),
          source: 'agent',
        },
      ])
    })
  })

  it('agent overlay wins on filename collision', async () => {
    await withTempBrowserClawDir(async () => {
      seed(sharedRecipesDirFor('linkedin.com'), 'invitation-manager.md', 'S')
      seed(
        agentRecipesDirFor('claude-code', 'linkedin.com'),
        'invitation-manager.md',
        'A',
      )
      const entries = listRecipeFiles('claude-code', 'linkedin.com')
      expect(entries).toHaveLength(1)
      expect(entries[0]?.name).toBe('invitation-manager.md')
      expect(entries[0]?.source).toBe('agent')
      expect(entries[0]?.absolutePath).toContain('agents/claude-code')
    })
  })

  it('agent slug is isolated: one agent does not see another overlay', async () => {
    await withTempBrowserClawDir(async () => {
      seed(agentRecipesDirFor('claude-code', 'linkedin.com'), 'mine.md')
      seed(agentRecipesDirFor('codex-cli', 'linkedin.com'), 'theirs.md')
      const claude = listRecipeFiles('claude-code', 'linkedin.com')
      const codex = listRecipeFiles('codex-cli', 'linkedin.com')
      expect(claude.map((e) => e.name)).toEqual(['mine.md'])
      expect(codex.map((e) => e.name)).toEqual(['theirs.md'])
    })
  })

  it('merges shared + agent, sorted by name', async () => {
    await withTempBrowserClawDir(async () => {
      seed(sharedRecipesDirFor('linkedin.com'), 'zulu.md')
      seed(sharedRecipesDirFor('linkedin.com'), 'alpha.md')
      seed(agentRecipesDirFor('claude-code', 'linkedin.com'), 'mike.md')
      const names = listRecipeFiles('claude-code', 'linkedin.com').map(
        (e) => e.name,
      )
      expect(names).toEqual(['alpha.md', 'mike.md', 'zulu.md'])
    })
  })

  it('skips non-.md files and subdirectories', async () => {
    await withTempBrowserClawDir(async () => {
      const dir = sharedRecipesDirFor('linkedin.com')
      seed(dir, 'ok.md')
      seed(dir, 'notes.txt')
      mkdirSync(`${dir}/subdir`, { recursive: true })
      writeFileSync(`${dir}/subdir/nested.md`, 'nested')
      const entries = listRecipeFiles('claude-code', 'linkedin.com')
      expect(entries.map((e) => e.name)).toEqual(['ok.md'])
    })
  })

  it('caps output at MAX_RECIPES_SURFACED (10) even when more exist', async () => {
    await withTempBrowserClawDir(async () => {
      const dir = sharedRecipesDirFor('linkedin.com')
      for (let i = 0; i < 15; i++) {
        seed(dir, `recipe-${String(i).padStart(2, '0')}.md`)
      }
      const entries = listRecipeFiles('claude-code', 'linkedin.com')
      expect(entries).toHaveLength(10)
      expect(entries[0]?.name).toBe('recipe-00.md')
      expect(entries[9]?.name).toBe('recipe-09.md')
    })
  })
})

describe('readRecipeMetadata', () => {
  const NOW = new Date('2026-07-20T12:00:00Z')

  function seedFrontmatter(body: string): string {
    const dir = sharedRecipesDirFor('linkedin.com')
    return seed(dir, 'invitation-manager.md', body)
  }

  it('returns empty metadata when the file is missing', async () => {
    await withTempBrowserClawDir(async () => {
      const path = `${sharedRecipesDirFor('linkedin.com')}/does-not-exist.md`
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.frontmatter.lastVerified).toBeNull()
      expect(meta.ageDays).toBeNull()
      expect(meta.isStale).toBe(false)
    })
  })

  it('returns empty metadata when the file has no frontmatter', async () => {
    await withTempBrowserClawDir(async () => {
      const path = seedFrontmatter('# Just prose, no yaml here\n')
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.frontmatter).toEqual({
        lastVerified: null,
        verifiedBy: null,
        usesSelectors: [],
      })
      expect(meta.ageDays).toBeNull()
      expect(meta.isStale).toBe(false)
    })
  })

  it('parses last_verified as an unquoted YAML date and computes age', async () => {
    await withTempBrowserClawDir(async () => {
      const path = seedFrontmatter(
        `---
last_verified: 2026-07-10
verified_by: claude-code
uses_selectors:
  - listitem
  - aria-label=Accept invitation from
---

# body`,
      )
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.frontmatter.lastVerified).toBe('2026-07-10')
      expect(meta.frontmatter.verifiedBy).toBe('claude-code')
      expect(meta.frontmatter.usesSelectors).toEqual([
        'listitem',
        'aria-label=Accept invitation from',
      ])
      expect(meta.ageDays).toBe(10)
      expect(meta.isStale).toBe(false)
    })
  })

  it('parses last_verified as a quoted string too', async () => {
    await withTempBrowserClawDir(async () => {
      const path = seedFrontmatter(
        `---
last_verified: "2026-05-01"
---

body`,
      )
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.frontmatter.lastVerified).toBe('2026-05-01')
      expect(meta.ageDays).toBeGreaterThan(60)
    })
  })

  it('flags recipes older than STALE_THRESHOLD_DAYS as stale', async () => {
    await withTempBrowserClawDir(async () => {
      const stalePath = seedFrontmatter(
        `---\nlast_verified: 2026-04-01\n---\nbody`,
      )
      const meta = readRecipeMetadata(stalePath, NOW)
      expect(meta.ageDays).toBeGreaterThanOrEqual(STALE_THRESHOLD_DAYS)
      expect(meta.isStale).toBe(true)
    })
  })

  it('exact boundary at STALE_THRESHOLD_DAYS counts as stale', async () => {
    await withTempBrowserClawDir(async () => {
      const then = new Date(NOW)
      then.setUTCDate(then.getUTCDate() - STALE_THRESHOLD_DAYS)
      const iso = then.toISOString().split('T')[0]
      const path = seedFrontmatter(`---\nlast_verified: ${iso}\n---\nbody`)
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.ageDays).toBe(STALE_THRESHOLD_DAYS)
      expect(meta.isStale).toBe(true)
    })
  })

  it('treats a future last_verified as fresh (ageDays=0)', async () => {
    await withTempBrowserClawDir(async () => {
      const path = seedFrontmatter(`---\nlast_verified: 2027-01-01\n---\n`)
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.ageDays).toBe(0)
      expect(meta.isStale).toBe(false)
    })
  })

  it('returns null age when last_verified is unparseable', async () => {
    await withTempBrowserClawDir(async () => {
      const path = seedFrontmatter(
        `---\nlast_verified: "not-a-date"\n---\nbody`,
      )
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.frontmatter.lastVerified).toBe('not-a-date')
      expect(meta.ageDays).toBeNull()
      expect(meta.isStale).toBe(false)
    })
  })

  it('recovers to empty metadata on malformed YAML rather than throwing', async () => {
    await withTempBrowserClawDir(async () => {
      const path = seedFrontmatter(
        `---\nlast_verified: {broken: [unclosed\n---\nbody`,
      )
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.frontmatter.lastVerified).toBeNull()
      expect(meta.ageDays).toBeNull()
    })
  })

  it('filters non-string entries out of uses_selectors', async () => {
    await withTempBrowserClawDir(async () => {
      const path = seedFrontmatter(
        `---\nuses_selectors:\n  - listitem\n  - 42\n  - true\n  - aria-label=X\n---\n`,
      )
      const meta = readRecipeMetadata(path, NOW)
      expect(meta.frontmatter.usesSelectors).toEqual([
        'listitem',
        'aria-label=X',
      ])
    })
  })
})

describe('hostBucketFromInput', () => {
  it('accepts a full http URL and reduces to the bucket', () => {
    expect(hostBucketFromInput('https://www.linkedin.com/feed')).toBe(
      'linkedin.com',
    )
    expect(hostBucketFromInput('http://docs.google.com/')).toBe(
      'docs.google.com',
    )
  })

  it('accepts a bare hostname and strips www.', () => {
    expect(hostBucketFromInput('linkedin.com')).toBe('linkedin.com')
    expect(hostBucketFromInput('www.linkedin.com')).toBe('linkedin.com')
    expect(hostBucketFromInput('docs.google.com')).toBe('docs.google.com')
    expect(hostBucketFromInput('  LINKEDIN.COM  ')).toBe('linkedin.com')
  })

  it('rejects garbage that is neither a URL nor a plausible hostname', () => {
    expect(hostBucketFromInput('')).toBeNull()
    expect(hostBucketFromInput('   ')).toBeNull()
    expect(hostBucketFromInput('not a host')).toBeNull()
    expect(hostBucketFromInput('has/slash.com')).toBeNull()
    expect(hostBucketFromInput('has:port.com')).toBeNull()
  })

  it('rejects non-http(s) URLs', () => {
    expect(hostBucketFromInput('file:///tmp/index.html')).toBeNull()
    expect(hostBucketFromInput('chrome://newtab')).toBeNull()
  })
})

describe('describeRecipesForHost', () => {
  it('returns an empty listing and a cold-nudge summary when no recipes exist', async () => {
    await withTempBrowserClawDir(async () => {
      const d = describeRecipesForHost('claude-code', 'linkedin.com')
      expect(d.files).toEqual([])
      expect(d.workspace_dir).toContain('recipes/shared/linkedin.com')
      expect(d.shared_dir).toBe(d.workspace_dir)
      expect(d.agent_dir).toContain('recipes/agents/claude-code/linkedin.com')
      expect(d.summary).toContain('none yet')
      expect(d.summary).toContain(d.workspace_dir)
    })
  })

  it('lists shared and agent overlays with source labels + fresh summary', async () => {
    await withTempBrowserClawDir(async () => {
      seed(sharedRecipesDirFor('linkedin.com'), 'invitation-manager.md')
      seed(agentRecipesDirFor('claude-code', 'linkedin.com'), 'my-only.md')
      const d = describeRecipesForHost('claude-code', 'linkedin.com')
      expect(d.files.map((f) => `${f.name}:${f.source}`)).toEqual([
        'invitation-manager.md:shared',
        'my-only.md:agent',
      ])
      expect(d.summary).toContain('invitation-manager.md, my-only.md')
      expect(d.summary).not.toContain('Stale')
    })
  })

  it('flags stale entries in the summary', async () => {
    await withTempBrowserClawDir(async () => {
      const now = new Date('2026-07-20T12:00:00Z')
      seed(
        sharedRecipesDirFor('linkedin.com'),
        'ancient.md',
        '---\nlast_verified: 2026-01-01\n---\nbody',
      )
      const d = describeRecipesForHost('claude-code', 'linkedin.com', now)
      expect(d.files[0]?.stale).toBe(true)
      expect(d.summary).toContain('Stale')
      expect(d.summary).toContain('ancient.md')
      expect(d.summary).toContain(String(STALE_THRESHOLD_DAYS))
    })
  })
})
