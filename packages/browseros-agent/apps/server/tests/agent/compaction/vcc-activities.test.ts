/**
 * TDD tests for site-aware VCC extraction.
 *
 * Tests the extractActivities function that extracts site visits
 * and tool usage from BrowserOS browser-oriented tool calls.
 *
 * RED phase — implementation does not exist yet.
 */
import { describe, expect, it } from 'bun:test'
import { extractActivities } from '../../../src/agent/compaction/vcc/extract/activities'
import {
  extractPageId,
  extractUrl,
  isBrowserTool,
  isNavigationTool,
} from '../../../src/agent/compaction/vcc/tool-args'
import type { NormalizedBlock } from '../../../src/agent/compaction/vcc/types'

// ─── Helpers ────────────────────────────────────────────────────────

function toolCall(
  name: string,
  args: Record<string, unknown>,
  sourceIndex: number,
): NormalizedBlock {
  return { kind: 'tool_call', name, args, sourceIndex }
}

function userMsg(text: string, sourceIndex: number): NormalizedBlock {
  return { kind: 'user', text, sourceIndex }
}

function toolResult(
  name: string,
  text: string,
  sourceIndex: number,
): NormalizedBlock {
  return { kind: 'tool_result', name, text, isError: false, sourceIndex }
}

// ─── extractUrl ─────────────────────────────────────────────────────

describe('extractUrl', () => {
  it('returns URL from args.url', () => {
    expect(extractUrl({ url: 'https://example.com' })).toBe(
      'https://example.com',
    )
  })

  it('returns null when no url field', () => {
    expect(extractUrl({ page: 5 })).toBeNull()
  })

  it('returns null for empty string', () => {
    expect(extractUrl({ url: '' })).toBeNull()
  })
})

// ─── extractPageId ──────────────────────────────────────────────────

describe('extractPageId', () => {
  it('returns number from args.page', () => {
    expect(extractPageId({ page: 5 })).toBe(5)
  })

  it('returns null when no page field', () => {
    expect(extractPageId({ url: 'https://example.com' })).toBeNull()
  })
})

// ─── isNavigationTool ───────────────────────────────────────────────

describe('isNavigationTool', () => {
  it('returns true for navigate_page', () => {
    expect(isNavigationTool('navigate_page')).toBe(true)
  })

  it('returns true for new_page', () => {
    expect(isNavigationTool('new_page')).toBe(true)
  })

  it('returns true for new_hidden_page', () => {
    expect(isNavigationTool('new_hidden_page')).toBe(true)
  })

  it('returns false for click', () => {
    expect(isNavigationTool('click')).toBe(false)
  })

  it('returns false for take_snapshot', () => {
    expect(isNavigationTool('take_snapshot')).toBe(false)
  })

  it('returns false for bash', () => {
    expect(isNavigationTool('bash')).toBe(false)
  })
})

// ─── isBrowserTool ──────────────────────────────────────────────────

describe('isBrowserTool', () => {
  it('returns true for click', () => {
    expect(isBrowserTool('click')).toBe(true)
  })

  it('returns true for fill', () => {
    expect(isBrowserTool('fill')).toBe(true)
  })

  it('returns true for take_snapshot', () => {
    expect(isBrowserTool('take_snapshot')).toBe(true)
  })

  it('returns true for navigate_page', () => {
    expect(isBrowserTool('navigate_page')).toBe(true)
  })

  it('returns true for scroll', () => {
    expect(isBrowserTool('scroll')).toBe(true)
  })

  it('returns true for get_page_content', () => {
    expect(isBrowserTool('get_page_content')).toBe(true)
  })

  it('returns true for evaluate_script', () => {
    expect(isBrowserTool('evaluate_script')).toBe(true)
  })

  it('returns false for bash', () => {
    expect(isBrowserTool('bash')).toBe(false)
  })

  it('returns false for Read', () => {
    expect(isBrowserTool('Read')).toBe(false)
  })

  it('returns false for Edit', () => {
    expect(isBrowserTool('Edit')).toBe(false)
  })

  it('returns false for Write', () => {
    expect(isBrowserTool('Write')).toBe(false)
  })
})

// ─── extractActivities ──────────────────────────────────────────────

describe('extractActivities', () => {
  it('single site with multiple tools', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://example.com', page: 1 }, 0),
      toolCall('click', { page: 1, selector: '#btn' }, 1),
      toolCall('click', { page: 1, selector: '#btn2' }, 2),
      toolCall('click', { page: 1, selector: '#btn3' }, 3),
      toolCall('fill', { page: 1, selector: '#input', value: 'hello' }, 4),
      toolCall('fill', { page: 1, selector: '#input2', value: 'world' }, 5),
    ]

    const { visits } = extractActivities(blocks)

    expect(visits).toHaveLength(1)
    expect(visits[0].url).toBe('https://example.com')
    expect(visits[0].domain).toBe('example.com')

    const tools = Object.fromEntries(visits[0].tools)
    expect(tools.navigate_page).toBe(1)
    expect(tools.click).toBe(3)
    expect(tools.fill).toBe(2)
  })

  it('multiple sites sequential visits', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://site1.com', page: 1 }, 0),
      toolCall('click', { page: 1 }, 1),
      toolCall('navigate_page', { url: 'https://site2.com', page: 2 }, 2),
      toolCall('click', { page: 2 }, 3),
    ]

    const { visits } = extractActivities(blocks)

    expect(visits).toHaveLength(2)
    expect(visits[0].domain).toBe('site1.com')
    expect(visits[1].domain).toBe('site2.com')
  })

  it('return visit to same domain merges tool counts', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://example.com/a', page: 1 }, 0),
      toolCall('click', { page: 1 }, 1),
      toolCall('navigate_page', { url: 'https://other.com', page: 2 }, 2),
      toolCall('click', { page: 2 }, 3),
      toolCall('navigate_page', { url: 'https://example.com/b', page: 3 }, 4),
      toolCall('click', { page: 3 }, 5),
    ]

    const { visits } = extractActivities(blocks)

    // Same domain grouped into 1 visit
    const exampleVisits = visits.filter((v) => v.domain === 'example.com')
    expect(exampleVisits).toHaveLength(1)

    const tools = Object.fromEntries(exampleVisits[0].tools)
    expect(tools.navigate_page).toBe(2)
    expect(tools.click).toBe(2)
  })

  it('no navigation tools returns empty array', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('click', { page: 1 }, 0),
      toolCall('fill', { page: 1 }, 1),
      userMsg('hello', 2),
    ]

    const { visits } = extractActivities(blocks)
    expect(visits).toHaveLength(0)
  })

  it('tool count accuracy for repeated same tool', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://example.com', page: 1 }, 0),
      ...Array.from({ length: 10 }, (_, i) =>
        toolCall('click', { page: 1 }, i + 1),
      ),
    ]

    const { visits } = extractActivities(blocks)
    const tools = Object.fromEntries(visits[0].tools)
    expect(tools.click).toBe(10)
  })

  it('max site cap truncates oldest', () => {
    const blocks: NormalizedBlock[] = Array.from({ length: 15 }, (_, i) =>
      toolCall('navigate_page', { url: `https://site${i}.com`, page: i }, i),
    )

    const { visits } = extractActivities(blocks)

    // Max 10 sites
    expect(visits).toHaveLength(10)

    // Should keep the LAST 10 (site5..site14), dropping oldest
    expect(visits[0].domain).toBe('site5.com')
    expect(visits[visits.length - 1].domain).toBe('site14.com')
  })

  it('timeline ordering by sourceIndex', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://z.com', page: 1 }, 10),
      toolCall('navigate_page', { url: 'https://a.com', page: 2 }, 20),
      toolCall('navigate_page', { url: 'https://m.com', page: 3 }, 5),
    ]

    const { visits } = extractActivities(blocks)

    // Ordered by sourceIndex, not alphabetically
    expect(visits[0].domain).toBe('z.com')
    expect(visits[0].sourceIndex).toBe(10)
    expect(visits[1].domain).toBe('a.com')
    expect(visits[1].sourceIndex).toBe(20)
    expect(visits[2].domain).toBe('m.com')
    expect(visits[2].sourceIndex).toBe(5)
  })

  it('domain extraction from full URL', () => {
    const blocks: NormalizedBlock[] = [
      toolCall(
        'navigate_page',
        { url: 'https://docs.google.com/spreadsheets/d/123', page: 1 },
        0,
      ),
    ]

    const { visits } = extractActivities(blocks)
    expect(visits[0].domain).toBe('docs.google.com')
    expect(visits[0].url).toBe('https://docs.google.com/spreadsheets/d/123')
  })

  it('mixed browser and non-browser tools only counts browser tools', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://example.com', page: 1 }, 0),
      toolCall('click', { page: 1 }, 1),
      toolCall('bash', { command: 'ls -la' }, 2),
      toolCall('Read', { file_path: '/src/index.ts' }, 3),
      toolCall('fill', { page: 1 }, 4),
    ]

    const { visits } = extractActivities(blocks)
    const tools = Object.fromEntries(visits[0].tools)

    // bash and Read are not browser tools — should not appear
    expect(tools.bash).toBeUndefined()
    expect(tools.Read).toBeUndefined()
    expect(tools.click).toBe(1)
    expect(tools.fill).toBe(1)
    expect(tools.navigate_page).toBe(1)
  })

  it('new_page creates site visit', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('new_page', { url: 'https://github.com/repo' }, 0),
      toolCall('click', { page: 0 }, 1),
    ]

    const { visits } = extractActivities(blocks)
    expect(visits).toHaveLength(1)
    expect(visits[0].domain).toBe('github.com')
    expect(visits[0].url).toBe('https://github.com/repo')
  })

  it('new_hidden_page creates site visit', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('new_hidden_page', { url: 'https://api.test.com' }, 0),
    ]

    const { visits } = extractActivities(blocks)
    expect(visits).toHaveLength(1)
    expect(visits[0].domain).toBe('api.test.com')
  })

  it('empty blocks returns empty array', () => {
    const { visits } = extractActivities([])
    expect(visits).toHaveLength(0)
  })

  it('only user messages returns empty array', () => {
    const blocks: NormalizedBlock[] = [userMsg('hello', 0), userMsg('world', 1)]
    const { visits } = extractActivities(blocks)
    expect(visits).toHaveLength(0)
  })

  it('visit has sequential order field', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://a.com', page: 1 }, 0),
      toolCall('navigate_page', { url: 'https://b.com', page: 2 }, 1),
      toolCall('navigate_page', { url: 'https://c.com', page: 3 }, 2),
    ]

    const { visits } = extractActivities(blocks)

    expect(visits[0].order).toBe(1)
    expect(visits[1].order).toBe(2)
    expect(visits[2].order).toBe(3)
  })

  it('tool results do not create visits', () => {
    const blocks: NormalizedBlock[] = [toolResult('navigate_page', 'OK', 0)]

    const { visits } = extractActivities(blocks)
    expect(visits).toHaveLength(0)
  })

  // ─── Timeline events ─────────────────────────────────────────────

  it('timeline has sequential navigation events', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://a.com', page: 1 }, 0),
      toolCall('navigate_page', { url: 'https://b.com', page: 2 }, 1),
    ]

    const { timeline: tl } = extractActivities(blocks)
    expect(tl).toHaveLength(2)
    expect(tl[0].domain).toBe('a.com')
    expect(tl[0].isReturnVisit).toBe(false)
    expect(tl[1].domain).toBe('b.com')
    expect(tl[1].isReturnVisit).toBe(false)
  })

  it('timeline marks return visits', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://a.com', page: 1 }, 0),
      toolCall('navigate_page', { url: 'https://b.com', page: 2 }, 1),
      toolCall('navigate_page', { url: 'https://a.com/page2', page: 3 }, 2),
    ]

    const { timeline: tl } = extractActivities(blocks)
    expect(tl).toHaveLength(3)
    expect(tl[0].isReturnVisit).toBe(false)
    expect(tl[1].isReturnVisit).toBe(false)
    expect(tl[2].isReturnVisit).toBe(true)
    expect(tl[2].domain).toBe('a.com')
  })

  it('timeline events include tool snapshot', () => {
    const blocks: NormalizedBlock[] = [
      toolCall('navigate_page', { url: 'https://a.com', page: 1 }, 0),
      toolCall('click', { page: 1 }, 1),
      toolCall('click', { page: 1 }, 2),
      toolCall('navigate_page', { url: 'https://b.com', page: 2 }, 3),
    ]

    const { timeline: tl } = extractActivities(blocks)
    // First timeline event shows tools at that point
    expect(tl[0].toolSnapshot).toContain('navigate_page')
    // Second event
    expect(tl[1].domain).toBe('b.com')
  })

  it('no navigation produces empty timeline', () => {
    const blocks: NormalizedBlock[] = [toolCall('click', { page: 1 }, 0)]

    const { timeline: tl } = extractActivities(blocks)
    expect(tl).toHaveLength(0)
  })
})
