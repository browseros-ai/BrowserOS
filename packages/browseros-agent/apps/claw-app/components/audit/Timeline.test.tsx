/**
 * Static-markup pins for the Timeline component's expand-all,
 * collapse-all, and HIGH RISK auto-expand behaviour.
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import { Timeline } from './Timeline'

function dispatch(overrides: Partial<ToolDispatchRow> = {}): ToolDispatchRow {
  return {
    id: 1,
    createdAt: 1_000_000,
    agentId: 'a',
    slug: 'a',
    agentLabel: 'A',
    sessionId: 'sess',
    toolName: 'snapshot',
    pageId: 1,
    targetId: null,
    url: null,
    title: null,
    argsJson: '{"page":1}',
    resultMeta: '{"isError":false}',
    durationMs: 5,
    ...overrides,
  }
}

const startedAt = 1_000_000

function render(
  dispatches: ToolDispatchRow[],
  screenshotDispatchIds: readonly number[] = [],
  extra: { showSessionEnd?: boolean; endEvent?: TimelineEndEvent } = {},
): string {
  return renderToStaticMarkup(
    <Timeline
      dispatches={dispatches}
      screenshotDispatchIds={screenshotDispatchIds}
      startedAt={startedAt}
      endEvent={extra.endEvent ?? null}
      showSessionEnd={extra.showSessionEnd}
      onScreenshotClick={() => undefined}
    />,
  )
}

type TimelineEndEvent = {
  createdAt: number
  kind: 'closed' | 'errored'
  reason: string | null
} | null

const CLOSED_END: TimelineEndEvent = {
  createdAt: startedAt + 60_000,
  kind: 'closed',
  reason: null,
}

describe('Timeline', () => {
  it('renders Expand all + Collapse all buttons', () => {
    const html = render([dispatch({ id: 1 }), dispatch({ id: 2 })])
    expect(html).toContain('Expand all')
    expect(html).toContain('Collapse all')
  })

  it('disables Collapse all on first render when nothing is auto-expanded', () => {
    const html = render([dispatch({ id: 1, toolName: 'snapshot' })])
    // Collapse all should be disabled (no HIGH RISK rows auto-expanded).
    expect(html).toMatch(
      /<button[^>]*data-disabled=""[^>]*timeline-collapse-all/,
    )
  })

  it('leaves Collapse all enabled when a HIGH RISK row auto-expands', () => {
    const html = render([
      dispatch({ id: 1, toolName: 'snapshot' }),
      dispatch({ id: 2, toolName: 'act' }),
    ])
    expect(html).not.toMatch(
      /<button[^>]*data-disabled=""[^>]*timeline-collapse-all/,
    )
  })

  it('auto-expands HIGH RISK rows so args + result blocks appear on initial render', () => {
    const html = render([
      dispatch({
        id: 7,
        toolName: 'act',
        argsJson: '{"kind":"click","ref":"btn-submit"}',
        resultMeta: '{"isError":false,"structuredKeys":["clicked"]}',
      }),
    ])
    // JSON in HTML markup is rendered with HTML-entity quotes; check
    // for the inner tokens that survive entity encoding instead.
    expect(html).toContain('btn-submit')
    expect(html).toContain('structuredKeys')
    expect(html).toContain('clicked')
  })

  it('disables both buttons when the dispatch list is empty', () => {
    const html = render([])
    expect(html).toMatch(/<button[^>]*data-disabled=""[^>]*timeline-expand-all/)
    expect(html).toMatch(
      /<button[^>]*data-disabled=""[^>]*timeline-collapse-all/,
    )
  })

  it('renders a copy button per block (args, result, page) on an expanded row', () => {
    const html = render([
      dispatch({
        id: 1,
        toolName: 'act',
        argsJson: '{"kind":"click"}',
        resultMeta: '{"isError":false}',
        url: 'https://example.com',
      }),
    ])
    expect(html).toContain('data-testid="timeline-block-copy-args"')
    expect(html).toContain('data-testid="timeline-block-copy-result"')
    expect(html).toContain('data-testid="timeline-block-copy-page"')
  })

  it('does not render a copy button on the screenshot block (image, not text)', () => {
    const html = render([
      dispatch({
        id: 2,
        toolName: 'screenshot',
        argsJson: '{"page":1}',
        resultMeta: '{"isError":false}',
      }),
    ])
    expect(html).not.toContain('data-testid="timeline-block-copy-screenshot"')
  })

  it('renders the SessionEndRow by default (backward compat)', () => {
    const html = render([dispatch({ id: 1 })], [], { endEvent: CLOSED_END })
    // SessionEndRow markup includes the literal "session closed" (or
    // "session errored") tail; either is enough to pin the presence
    // of the row.
    expect(html).toContain('session closed')
  })

  it('hides the SessionEndRow when showSessionEnd is false (per-tab view)', () => {
    const html = render([dispatch({ id: 1 })], [], {
      endEvent: CLOSED_END,
      showSessionEnd: false,
    })
    expect(html).not.toContain('session closed')
    expect(html).not.toContain('session errored')
    expect(html).not.toContain('Still running')
    // The dispatch row itself is still there.
    expect(html).toContain('snapshot')
  })
})
