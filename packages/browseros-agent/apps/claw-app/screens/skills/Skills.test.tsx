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
  firstRunTokens: 23000,
  latestRunTokens: 14600,
  lastRunAt: 1_000_000_000_000,
  createdAt: 900_000_000_000,
  updatedAt: 1_000_000_000_000,
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

  it('renders a row per skill with the command, description, and clean ratio', () => {
    dataOverride = { ...baseData, skills: [sampleSkill] }
    const html = renderApp()
    expect(html).toContain('/inbox-sweep')
    expect(html).toContain('Check the inbox and draft what is owed')
    expect(html).toContain('4/5')
    expect(html).toContain('14.6k')
  })

  it('renders the getting-cheaper delta on the cost column', () => {
    dataOverride = { ...baseData, skills: [sampleSkill] }
    // 23000 -> 14600 is roughly a 37% reduction.
    expect(renderApp()).toContain('37%')
  })

  it('renders "not run" for a skill with no measured runs', () => {
    dataOverride = {
      ...baseData,
      skills: [
        {
          ...sampleSkill,
          runCount: 0,
          cleanRunCount: 0,
          firstRunTokens: undefined,
          latestRunTokens: undefined,
          lastRunAt: undefined,
        },
      ],
    }
    expect(renderApp()).toContain('not run')
  })
})
