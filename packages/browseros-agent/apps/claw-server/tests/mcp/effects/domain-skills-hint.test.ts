/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Unit coverage for the domain-skills-hint dispatch effect. Locks the
 * on-navigate discovery surface: which files bubble into
 * structuredContent.domain_skills, how the shared+agent overlay
 * precedence flows through, and the staleness signal in the synthetic
 * text block.
 */

import { beforeEach, describe, expect, it } from 'bun:test'
import { mkdirSync, writeFileSync } from 'node:fs'
import type { ClientIdentity } from '../../../src/lib/mcp-session'
import { identityService } from '../../../src/lib/mcp-session'
import type { ToolCall } from '../../../src/mcp/dispatch'
import { applyDomainSkillsHint } from '../../../src/mcp/effects/domain-skills-hint'
import type { ToolResult } from '../../../src/mcp/register-fn'
import {
  agentRecipesDirFor,
  sharedRecipesDirFor,
} from '../../../src/services/recipes'
import { withTempBrowserClawDir } from '../../_helpers/temp-browserclaw-dir'

function register(
  sessionId: string,
  clientName = 'Claude Code',
): ClientIdentity {
  return identityService.registerInitialize({
    sessionId,
    clientInfo: { name: clientName, version: '1.0.0' },
  })
}

function makeCall(
  identity: ClientIdentity | null,
  toolName: string,
  args: Record<string, unknown> = {},
): ToolCall {
  return {
    tool: { name: toolName } as never,
    args,
    sessionId: identity?.sessionId ?? '',
    identity,
    key: identity?.key ?? null,
    agent: identity ? { agentId: identity.key, slug: identity.slug } : null,
    agentLabel: identity?.clientName ?? null,
    session: {} as never,
    defaultTabGroupId: null,
    flags: { newPage: false, closePage: false, listTabs: false },
  }
}

function makeResult(overrides: Partial<ToolResult> = {}): ToolResult {
  return {
    content: [{ type: 'text', text: 'navigated to https://linkedin.com/' }],
    isError: false,
    structuredContent: { url: 'https://linkedin.com/' },
    ...overrides,
  }
}

function apply(call: ToolCall, result: ToolResult): ToolResult | undefined {
  return applyDomainSkillsHint({
    call,
    result,
    cancelled: false,
    durationMs: 1,
    startedAtMs: 0,
  })
}

function seedRecipe(
  dir: string,
  name: string,
  body = '# stub\nprose here.\n',
): string {
  mkdirSync(dir, { recursive: true })
  const path = `${dir}/${name}`
  writeFileSync(path, body)
  return path
}

function daysAgo(n: number): string {
  const d = new Date()
  d.setUTCDate(d.getUTCDate() - n)
  return d.toISOString().split('T')[0] ?? ''
}

function structured(result: ToolResult | undefined): Record<string, unknown> {
  return (result?.structuredContent ?? {}) as Record<string, unknown>
}

interface DomainSkillsShape {
  files: Array<{
    name: string
    source: 'shared' | 'agent'
    age_days: number | null
    stale: boolean
  }>
  workspace_dir: string
  shared_dir: string
  agent_dir: string
}

function skills(result: ToolResult | undefined): DomainSkillsShape {
  return structured(result).domain_skills as DomainSkillsShape
}

function tailText(result: ToolResult | undefined): string {
  const last = result?.content[result.content.length - 1]
  return last?.type === 'text' ? last.text : ''
}

describe('applyDomainSkillsHint', () => {
  beforeEach(() => {
    identityService.clear()
  })

  it('is a no-op for non-navigate tools', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      seedRecipe(sharedRecipesDirFor('linkedin.com'), 'a.md')
      expect(
        apply(makeCall(identity, 'snapshot'), makeResult()),
      ).toBeUndefined()
    })
  })

  it('passes through error results untouched', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      seedRecipe(sharedRecipesDirFor('linkedin.com'), 'a.md')
      const errorResult = makeResult({ isError: true })
      expect(apply(makeCall(identity, 'navigate'), errorResult)).toBeUndefined()
    })
  })

  it('is a no-op when the caller has no identity', async () => {
    await withTempBrowserClawDir(async () => {
      expect(apply(makeCall(null, 'navigate'), makeResult())).toBeUndefined()
    })
  })

  it('is a no-op when the URL has no http host bucket', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      const call = makeCall(identity, 'navigate', { url: 'chrome://newtab' })
      const result = makeResult({
        structuredContent: { url: 'chrome://newtab' },
      })
      expect(apply(call, result)).toBeUndefined()
    })
  })

  it('annotates a cold host with an empty files list + workspace nudge', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      const returned = apply(makeCall(identity, 'navigate'), makeResult())
      expect(returned).toBeDefined()
      const s = skills(returned)
      expect(s.files).toEqual([])
      expect(s.workspace_dir).toContain('recipes/shared/linkedin.com')
      expect(s.shared_dir).toContain('recipes/shared/linkedin.com')
      expect(s.agent_dir).toContain(
        `recipes/agents/${identity.slug}/linkedin.com`,
      )
      expect(tailText(returned)).toContain('none yet')
      expect(tailText(returned)).toContain(s.workspace_dir)
    })
  })

  it('surfaces shared recipe filenames with source=shared', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      seedRecipe(sharedRecipesDirFor('linkedin.com'), 'invitation-manager.md')
      seedRecipe(sharedRecipesDirFor('linkedin.com'), 'messages.md')
      const returned = apply(makeCall(identity, 'navigate'), makeResult())
      const s = skills(returned)
      expect(s.files.map((f) => f.name)).toEqual([
        'invitation-manager.md',
        'messages.md',
      ])
      expect(s.files.every((f) => f.source === 'shared')).toBe(true)
      expect(tailText(returned)).toContain('invitation-manager.md, messages.md')
    })
  })

  it('agent overlay wins on filename collision and is labelled source=agent', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      seedRecipe(sharedRecipesDirFor('linkedin.com'), 'invitation.md', 'S')
      seedRecipe(
        agentRecipesDirFor(identity.slug, 'linkedin.com'),
        'invitation.md',
        'A',
      )
      const returned = apply(makeCall(identity, 'navigate'), makeResult())
      const s = skills(returned)
      expect(s.files).toHaveLength(1)
      expect(s.files[0]).toEqual({
        name: 'invitation.md',
        source: 'agent',
        age_days: null,
        stale: false,
      })
    })
  })

  it('flags recipes older than STALE_THRESHOLD_DAYS in the text block', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      seedRecipe(
        sharedRecipesDirFor('linkedin.com'),
        'ancient.md',
        `---\nlast_verified: ${daysAgo(120)}\n---\nbody`,
      )
      seedRecipe(
        sharedRecipesDirFor('linkedin.com'),
        'fresh.md',
        `---\nlast_verified: ${daysAgo(5)}\n---\nbody`,
      )
      const returned = apply(makeCall(identity, 'navigate'), makeResult())
      const s = skills(returned)
      expect(s.files.find((f) => f.name === 'ancient.md')?.stale).toBe(true)
      expect(s.files.find((f) => f.name === 'fresh.md')?.stale).toBe(false)
      expect(tailText(returned)).toContain('Stale')
      expect(tailText(returned)).toContain('ancient.md')
      expect(tailText(returned)).not.toContain('fresh.md (')
    })
  })

  it('preserves other structuredContent fields alongside domain_skills', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      const result = makeResult({
        structuredContent: {
          url: 'https://linkedin.com/',
          page: 42,
          title: 'LinkedIn',
        },
      })
      const returned = apply(makeCall(identity, 'navigate'), result)
      const s = structured(returned)
      expect(s.url).toBe('https://linkedin.com/')
      expect(s.page).toBe(42)
      expect(s.title).toBe('LinkedIn')
      expect(s.domain_skills).toBeDefined()
    })
  })

  it('falls back to args.url when structuredContent has no url', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      seedRecipe(sharedRecipesDirFor('linkedin.com'), 'a.md')
      const call = makeCall(identity, 'navigate', {
        url: 'https://www.linkedin.com/feed',
      })
      const result = makeResult({ structuredContent: undefined })
      const returned = apply(call, result)
      const s = skills(returned)
      expect(s.files.map((f) => f.name)).toEqual(['a.md'])
    })
  })

  it('isolates one agent slug from another (Fix 1 slug-scoped overlay)', async () => {
    await withTempBrowserClawDir(async () => {
      const claude = register('s-claude', 'Claude Code')
      const codex = register('s-codex', 'Codex CLI')
      seedRecipe(agentRecipesDirFor(claude.slug, 'linkedin.com'), 'mine.md')
      seedRecipe(agentRecipesDirFor(codex.slug, 'linkedin.com'), 'theirs.md')

      const claudeReturned = apply(makeCall(claude, 'navigate'), makeResult())
      const codexReturned = apply(makeCall(codex, 'navigate'), makeResult())
      expect(skills(claudeReturned).files.map((f) => f.name)).toEqual([
        'mine.md',
      ])
      expect(skills(codexReturned).files.map((f) => f.name)).toEqual([
        'theirs.md',
      ])
    })
  })

  it('buckets subdomains separately (docs vs mail on google)', async () => {
    await withTempBrowserClawDir(async () => {
      const identity = register('s1')
      seedRecipe(sharedRecipesDirFor('docs.google.com'), 'docs-tips.md')
      seedRecipe(sharedRecipesDirFor('mail.google.com'), 'mail-tips.md')

      const docsCall = makeCall(identity, 'navigate', {
        url: 'https://docs.google.com/document/d/abc',
      })
      const docsResult = makeResult({
        structuredContent: { url: 'https://docs.google.com/document/d/abc' },
      })
      const returned = apply(docsCall, docsResult)
      const s = skills(returned)
      expect(s.files.map((f) => f.name)).toEqual(['docs-tips.md'])
      expect(s.workspace_dir).toContain('recipes/shared/docs.google.com')
    })
  })
})
