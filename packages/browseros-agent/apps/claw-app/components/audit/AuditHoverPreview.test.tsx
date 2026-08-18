import { describe, expect, it } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import { AuditHoverPreview } from './AuditHoverPreview'

const task: TaskSummary = {
  sessionId: 'sess-1',
  slug: 'claude-code',
  label: 'Claude Code',
  name: 'Browsed example.com',
  site: 'example.com',
  startedAt: Date.now() - 12_000,
  endedAt: Date.now(),
  durationMs: 12_000,
  dispatchCount: 4,
  toolSequence: ['tabs', 'read'],
  status: 'done',
  errorCount: 0,
  latestScreenshotId: undefined,
  tokenUsage: undefined,
}

function render(t: TaskSummary | null): string {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <AuditHoverPreview task={t} />
    </QueryClientProvider>,
  )
}

describe('AuditHoverPreview', () => {
  // AgentDot paints its per-agent colour through an inline `background`
  // style, so its absence is the thing to assert — a class check would
  // pass even if the dot came back.
  it('renders the agent label without a per-agent colour dot', () => {
    const html = render(task)
    expect(html).toContain('Claude Code')
    expect(html).not.toContain('style="background:')
  })

  it('marks a live session with a static pill and no pulse', () => {
    const html = render({ ...task, status: 'live' })
    expect(html).toContain('Live')
    expect(html).not.toContain('pulse-dot')
  })
})
