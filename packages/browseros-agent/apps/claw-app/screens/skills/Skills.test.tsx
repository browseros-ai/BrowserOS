/**
 * Static-markup checks for the Tasks list screen. Stubs the data hook so the
 * test does not need a running backend.
 */

import { describe, expect, it, mock } from 'bun:test'
import type { Skill } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { SkillsScreenData } from './skills.data'

const baseData: SkillsScreenData = {
  skills: [],
  isLoading: false,
  isError: false,
}

let dataOverride: SkillsScreenData = baseData

mock.module('./skills.data', () => ({
  useSkillsScreenData: () => dataOverride,
}))

const { Skills } = await import('./Skills')

function renderApp(): string {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Skills />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

const sampleSkill: Skill = {
  name: 'inbox-sweep',
  description: 'Check the inbox and draft what is owed',
  origin: 'agent',
  version: 3,
  linkedAgents: ['Claude Code', 'Codex'],
  runCount: 5,
  cleanRunCount: 4,
  lastRunAt: 1_000_000_000_000,
  createdAt: 900_000_000_000,
  updatedAt: 1_000_000_000_000,
  tokenSavings: {
    saved: 45000,
    otherBrowsers: 60000,
    used: 15000,
    measuredRunCount: 5,
  },
}

describe('Tasks list screen', () => {
  it('renders the header', () => {
    dataOverride = { ...baseData }
    expect(renderApp()).toContain('Tasks')
  })

  it('shows the empty state when there are no tasks', () => {
    dataOverride = { ...baseData }
    expect(renderApp()).toContain('No tasks yet')
  })

  it('shows skeleton loading rows while the first page is pending', () => {
    dataOverride = { ...baseData, isLoading: true }
    expect(renderApp()).toMatch(/animate-pulse/)
  })

  it('shows the error notice when the query fails', () => {
    dataOverride = { ...baseData, isError: true }
    expect(renderApp()).toContain('Could not load')
  })

  it('renders a row per skill with the command, description, and success rate', () => {
    dataOverride = { ...baseData, skills: [sampleSkill] }
    const html = renderApp()
    expect(html).toContain('/inbox-sweep')
    expect(html).toContain('Check the inbox and draft what is owed')
    // Clean ratio 4/5 and its color-coded 80% success rate.
    expect(html).toContain('4/5')
    expect(html).toContain('80%')
  })

  it('renders tokens saved for a measured skill', () => {
    dataOverride = { ...baseData, skills: [sampleSkill] }
    const html = renderApp()
    expect(html).toContain('45.0k')
    expect(html).toContain('saved')
  })

  it('renders "not run" and "not measured" for a skill with no runs', () => {
    dataOverride = {
      ...baseData,
      skills: [
        {
          ...sampleSkill,
          runCount: 0,
          cleanRunCount: 0,
          lastRunAt: undefined,
          tokenSavings: {
            saved: 0,
            otherBrowsers: 0,
            used: 0,
            measuredRunCount: 0,
          },
        },
      ],
    }
    const html = renderApp()
    expect(html).toContain('not run')
    expect(html).toContain('not measured')
  })
})
