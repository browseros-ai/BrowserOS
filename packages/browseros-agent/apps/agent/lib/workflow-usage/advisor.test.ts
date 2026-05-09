import { describe, expect, it } from 'bun:test'
import {
  analyzeWorkflowUsage,
  detectWorkflowAdvisorCommand,
  formatWorkflowAnalysisResponse,
  normalizeToolSequence,
} from './advisor'
import type { WorkflowUsageRecord } from './types'

describe('workflow usage advisor', () => {
  it('detects explicit workflow advisor commands only', () => {
    expect(detectWorkflowAdvisorCommand('analyze my workflow')).toBe('analyze')
    expect(detectWorkflowAdvisorCommand('what patterns do you see?')).toBe(
      'analyze',
    )
    expect(detectWorkflowAdvisorCommand('show workflow usage data')).toBe(
      'view',
    )
    expect(detectWorkflowAdvisorCommand('clear skill suggestion data')).toBe(
      'clear',
    )
    expect(detectWorkflowAdvisorCommand('summarize this page')).toBeNull()
  })

  it('normalizes command sequences without retaining repeated adjacent tools', () => {
    expect(normalizeToolSequence([' new_page ', 'new_page', 'open'])).toEqual([
      'new_page',
      'open',
    ])
  })

  it('suggests repeated local tool-name patterns', () => {
    const analysis = analyzeWorkflowUsage([
      record('1', ['new_page', 'navigate', 'get_page_content'], 100),
      record('2', ['new_page', 'navigate', 'get_page_content'], 200),
      record('3', ['search', 'open'], 300),
    ])

    expect(analysis.totalRuns).toBe(3)
    expect(analysis.suggestions).toHaveLength(1)
    expect(analysis.suggestions[0]).toMatchObject({
      runCount: 2,
      pattern: ['new_page', 'navigate', 'get_page_content'],
    })
  })

  it('formats concrete suggestions with a privacy note', () => {
    const response = formatWorkflowAnalysisResponse(
      analyzeWorkflowUsage([
        record('1', ['new_page', 'navigate', 'get_page_content'], 100),
        record('2', ['new_page', 'navigate', 'get_page_content'], 200),
      ]),
    )

    expect(response).toContain('Pattern: Open page -> Navigate -> Read page')
    expect(response).toContain('Create a "Open Page to Read Page Skill" skill')
    expect(response).toContain('does not include URLs')
    expect(response).toContain('tool inputs')
  })
})

function record(
  id: string,
  toolNames: string[],
  recordedAt: number,
): WorkflowUsageRecord {
  return {
    id,
    source: 'sidepanel-chat',
    recordedAt,
    toolNames,
  }
}
