/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Unit tests for the `classify` + `annotateTabsListWithOwnership`
 * pair. Deps are injected as plain functions so the tests never
 * touch the process-wide agentTabs or identityService singletons.
 */

import { describe, expect, test } from 'bun:test'
import {
  annotateTabsListWithOwnership,
  classify,
  type OwnershipDeps,
} from '../../src/mcp/tabs-ownership'

function makeDeps(overrides: Partial<OwnershipDeps> = {}): OwnershipDeps {
  return {
    callerAgentId: 'me',
    resolveOwner: () => null,
    labelForAgentId: () => null,
    ...overrides,
  }
}

describe('classify', () => {
  test('no owner returns the "user" bucket', () => {
    const result = classify({ page: 1, url: 'https://u.example/' }, makeDeps())
    expect(result.ownership).toBe('user')
    expect(result.ownerAgentId).toBeNull()
    expect(result.ownerLabel).toBeNull()
  })

  test('caller-owned page returns the "mine" bucket', () => {
    const deps = makeDeps({ resolveOwner: () => 'me' })
    const result = classify({ page: 7 }, deps)
    expect(result.ownership).toBe('mine')
    expect(result.ownerAgentId).toBe('me')
    expect(result.ownerLabel).toBeNull()
  })

  test('foreign owner returns the "other-agent" bucket with a label', () => {
    const deps = makeDeps({
      resolveOwner: () => 'other',
      labelForAgentId: (id) => (id === 'other' ? 'cursor' : null),
    })
    const result = classify({ page: 9 }, deps)
    expect(result.ownership).toBe('other-agent')
    expect(result.ownerAgentId).toBe('other')
    expect(result.ownerLabel).toBe('cursor')
  })

  test('foreign owner with no label (session already closed) leaves ownerLabel null', () => {
    const deps = makeDeps({
      resolveOwner: () => 'other',
      labelForAgentId: () => null,
    })
    const result = classify({ page: 9 }, deps)
    expect(result.ownership).toBe('other-agent')
    expect(result.ownerLabel).toBeNull()
  })
})

describe('annotateTabsListWithOwnership', () => {
  const oneOfEach = {
    isError: false,
    content: [{ type: 'text', text: '(placeholder)' }],
    structuredContent: {
      pages: [
        { page: 3, url: 'https://my.example/', title: 'Mine' },
        { page: 1, url: 'https://u.example/', title: 'User' },
        { page: 7, url: 'https://o.example/', title: 'Other' },
      ],
    },
  }

  test('groups one page per bucket with headers in the fixed order', () => {
    const deps = makeDeps({
      resolveOwner: (id) => (id === 3 ? 'me' : id === 7 ? 'other' : null),
      labelForAgentId: (id) => (id === 'other' ? 'claude-code' : null),
    })
    const result = annotateTabsListWithOwnership(oneOfEach, deps)
    const text = (result.content as Array<{ text: string }>)[0].text
    // Fixed order across the response: Your tabs / User's tabs /
    // Other agents' tabs. Blank line separates sections.
    expect(text).toBe(
      [
        'Your tabs:',
        '[3] https://my.example/ (Mine)',
        '',
        "User's tabs:",
        '[1] https://u.example/ (User)',
        '',
        "Other agents' tabs:",
        '[7] https://o.example/ (Other), owned by claude-code',
      ].join('\n'),
    )
    const sc = result.structuredContent as { pages: unknown[] }
    expect(sc.pages).toHaveLength(3)
  })

  test('omits empty bucket headers when the section is empty', () => {
    const result = annotateTabsListWithOwnership(
      {
        content: [{ type: 'text', text: 'x' }],
        structuredContent: {
          pages: [{ page: 3, url: 'https://my.example/' }],
        },
      },
      makeDeps({ resolveOwner: () => 'me' }),
    )
    const text = (result.content as Array<{ text: string }>)[0].text
    expect(text).toBe(['Your tabs:', '[3] https://my.example/'].join('\n'))
    expect(text).not.toContain("User's tabs:")
    expect(text).not.toContain("Other agents' tabs:")
  })

  test('renders only "User\'s tabs:" when the caller owns nothing and no other agents claim anything', () => {
    const result = annotateTabsListWithOwnership(
      {
        content: [{ type: 'text', text: 'x' }],
        structuredContent: {
          pages: [{ page: 1, url: 'https://u.example/', title: 'Op' }],
        },
      },
      makeDeps(),
    )
    const text = (result.content as Array<{ text: string }>)[0].text
    expect(text).toContain("User's tabs:")
    expect(text).not.toContain('Your tabs:')
    expect(text).not.toContain("Other agents' tabs:")
  })

  test('empty pages list renders the legacy "(no open pages)" cue', () => {
    const result = annotateTabsListWithOwnership(
      {
        content: [{ type: 'text', text: 'x' }],
        structuredContent: { pages: [] },
      },
      makeDeps(),
    )
    const text = (result.content as Array<{ text: string }>)[0].text
    expect(text).toBe('(no open pages)')
  })

  test('missing structuredContent is treated as an empty pages list', () => {
    const result = annotateTabsListWithOwnership(
      { content: [{ type: 'text', text: 'x' }] },
      makeDeps(),
    )
    const text = (result.content as Array<{ text: string }>)[0].text
    expect(text).toBe('(no open pages)')
  })

  test('structuredContent.pages retains ordering from the underlying tool', () => {
    // Pages arrive in a specific order from ctx.session.pages.list();
    // the annotator must not reorder within `structuredContent`
    // (only the text render groups by bucket).
    const result = annotateTabsListWithOwnership(
      {
        content: [{ type: 'text', text: 'x' }],
        structuredContent: {
          pages: [
            { page: 9, url: 'https://o.example/' },
            { page: 3, url: 'https://my.example/' },
            { page: 1, url: 'https://u.example/' },
          ],
        },
      },
      makeDeps({
        resolveOwner: (id) => (id === 3 ? 'me' : id === 9 ? 'other' : null),
        labelForAgentId: (id) => (id === 'other' ? 'cursor' : null),
      }),
    )
    const sc = result.structuredContent as {
      pages: Array<{ page: number; ownership: string }>
    }
    expect(sc.pages.map((p) => p.page)).toEqual([9, 3, 1])
    expect(sc.pages.map((p) => p.ownership)).toEqual([
      'other-agent',
      'mine',
      'user',
    ])
  })

  test('foreign owner without a label omits the ", owned by ..." suffix', () => {
    const result = annotateTabsListWithOwnership(
      {
        content: [{ type: 'text', text: 'x' }],
        structuredContent: {
          pages: [{ page: 9, url: 'https://o.example/' }],
        },
      },
      makeDeps({
        resolveOwner: () => 'ghost',
        labelForAgentId: () => null,
      }),
    )
    const text = (result.content as Array<{ text: string }>)[0].text
    expect(text).toContain("Other agents' tabs:")
    expect(text).toContain('[9] https://o.example/')
    expect(text).not.toContain('owned by')
  })

  test('always clears isError to false on the annotated result', () => {
    const result = annotateTabsListWithOwnership(
      {
        content: [{ type: 'text', text: 'x' }],
        structuredContent: { pages: [] },
        isError: true,
      },
      makeDeps(),
    )
    expect(result.isError).toBe(false)
  })
})
