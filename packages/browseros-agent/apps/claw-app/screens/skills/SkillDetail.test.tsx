/**
 * Static-markup checks for the task detail stat cards (Tokens saved + Success
 * rate). Stubs the data hook so no backend is needed.
 */

import { describe, expect, it, mock } from 'bun:test'
import type { Skill, SkillDetail as SkillDetailDto } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { SkillDetailScreenData } from './skill-detail.data'

const sampleSkill: Skill = {
  name: 'inbox-sweep',
  description: 'Check the inbox and draft what is owed',
  origin: 'agent',
  version: 2,
  linkedAgents: ['Claude Code'],
  runCount: 5,
  cleanRunCount: 4,
  createdAt: 900_000_000_000,
  updatedAt: 1_000_000_000_000,
}

function detail(over: Partial<SkillDetailDto> = {}): SkillDetailDto {
  return {
    skill: sampleSkill,
    body: '---\nname: inbox-sweep\n---\n\n## Steps\n',
    runs: [],
    tokenSavings: {
      saved: 45_000,
      otherBrowsers: 60_000,
      used: 15_000,
      measuredRunCount: 5,
    },
    ...over,
  }
}

let dataOverride: SkillDetailScreenData = {
  name: 'inbox-sweep',
  detail: detail(),
  isLoading: false,
  isError: false,
}

mock.module('./skill-detail.data', () => ({
  useSkillDetailData: () => dataOverride,
}))

const { SkillDetail } = await import('./SkillDetail')

function renderApp(): string {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <SkillDetail />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('Task detail stat cards', () => {
  it('shows Tokens saved with the saved total and the other-browsers comparison', () => {
    dataOverride = { ...dataOverride, detail: detail() }
    const html = renderApp()
    expect(html).toContain('Tokens saved')
    expect(html).toContain('45.0k')
    expect(html).toContain('tokens in other browsers')
    expect(html).toContain('60.0k')
    expect(html).not.toContain('Getting cheaper')
  })

  it('shows the empty state when no runs are measured', () => {
    dataOverride = {
      ...dataOverride,
      detail: detail({
        tokenSavings: {
          saved: 0,
          otherBrowsers: 0,
          used: 0,
          measuredRunCount: 0,
        },
      }),
    }
    expect(renderApp()).toContain('No measured runs yet')
  })

  it('shows a color-coded Success rate and keeps the without-error line', () => {
    dataOverride = { ...dataOverride, detail: detail() }
    const html = renderApp()
    expect(html).toContain('Success rate')
    // 4 of 5 clean = 80% -> amber band
    expect(html).toContain('80%')
    expect(html).toContain('text-amber')
    expect(html).toContain('4 of 5 runs finished without a tool error')
    expect(html).not.toContain('Safe to leave alone')
  })

  it('greens a perfect success rate and reds a poor one', () => {
    dataOverride = {
      ...dataOverride,
      detail: detail({
        skill: { ...sampleSkill, runCount: 5, cleanRunCount: 5 },
      }),
    }
    expect(renderApp()).toContain('text-green')
    dataOverride = {
      ...dataOverride,
      detail: detail({
        skill: { ...sampleSkill, runCount: 5, cleanRunCount: 2 },
      }),
    }
    expect(renderApp()).toContain('text-red')
  })

  it('shows "not run" for a skill with no runs', () => {
    dataOverride = {
      ...dataOverride,
      detail: detail({
        skill: { ...sampleSkill, runCount: 0, cleanRunCount: 0 },
        tokenSavings: {
          saved: 0,
          otherBrowsers: 0,
          used: 0,
          measuredRunCount: 0,
        },
      }),
    }
    const html = renderApp()
    expect(html).toContain('not run')
    expect(html).toContain('0 of 0 runs finished without a tool error')
  })
})
