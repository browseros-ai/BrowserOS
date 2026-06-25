/**
 * Static markup checks for the task detail page. Stubs the data hook
 * so the test does not need a running backend.
 */

import { describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter, Route, Routes } from 'react-router'
import type { TaskDetail } from '@/modules/api/audit.hooks'
import type { TaskDetailScreenData } from './task-detail.data'

const baseData: TaskDetailScreenData = {
  task: undefined,
  isPending: false,
  isError: false,
  error: null,
}

let dataOverride: TaskDetailScreenData = baseData

mock.module('./task-detail.data', () => ({
  useTaskDetailScreenData: () => dataOverride,
}))

const { TaskDetailPage } = await import('./TaskDetailPage')

function render(): string {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={['/audit/sess-1']}>
        <Routes>
          <Route path="/audit/:sessionId" element={<TaskDetailPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

const sampleTask: TaskDetail = {
  sessionId: 'sess-1',
  agentId: 'claude-code',
  slug: 'claude-code',
  agentLabel: 'Claude Code',
  title: 'Browsed example.com',
  site: 'example.com',
  startedAt: Date.now() - 12000,
  endedAt: Date.now(),
  durationMs: 12000,
  dispatchCount: 2,
  toolSequence: ['tabs', 'screenshot'],
  status: 'done',
  errorCount: 0,
  lastScreenshotDispatchId: 2,
  cursorId: 2,
  dispatches: [
    {
      id: 1,
      createdAt: Date.now() - 12000,
      agentId: 'claude-code',
      slug: 'claude-code',
      agentLabel: 'Claude Code',
      sessionId: 'sess-1',
      toolName: 'tabs',
      pageId: 1,
      targetId: null,
      url: 'https://example.com',
      title: 'Example',
      argsJson: '{"action":"new"}',
      resultMeta: '{"isError":false}',
      durationMs: 12,
    },
    {
      id: 2,
      createdAt: Date.now() - 9000,
      agentId: 'claude-code',
      slug: 'claude-code',
      agentLabel: 'Claude Code',
      sessionId: 'sess-1',
      toolName: 'screenshot',
      pageId: 1,
      targetId: null,
      url: 'https://example.com',
      title: null,
      argsJson: '{"page":1}',
      resultMeta: '{"isError":false}',
      durationMs: 80,
    },
  ],
  screenshotDispatchIds: [2],
  startEvent: null,
  endEvent: { createdAt: Date.now(), kind: 'closed', reason: null },
}

describe('TaskDetailPage', () => {
  it('renders skeleton while pending', () => {
    dataOverride = { ...baseData, isPending: true }
    const html = render()
    expect(html).toMatch(/animate-pulse/)
  })

  it('renders the not-found state when task is null', () => {
    dataOverride = { ...baseData }
    const html = render()
    expect(html).toContain('Task not found')
  })

  it('renders header + timeline + strip for a real task', () => {
    dataOverride = { ...baseData, task: sampleTask }
    const html = render()
    expect(html).toContain('Browsed example.com')
    expect(html).toContain('Claude Code')
    expect(html).toContain('Timeline')
    expect(html).toContain('Screenshots')
    expect(html).toContain('Open final URL')
  })

  it('renders no-screenshots placeholder when there are none', () => {
    const first = sampleTask.dispatches[0]
    if (!first) throw new Error('test fixture missing first dispatch')
    dataOverride = {
      ...baseData,
      task: {
        ...sampleTask,
        screenshotDispatchIds: [],
        dispatches: [first],
      },
    }
    const html = render()
    expect(html).toContain('No screenshots captured for this task.')
  })
})
