import { describe, expect, it } from 'bun:test'
import {
  neoName,
  parseSkillBody,
  skillCommand,
  stuckWhile,
  usedLabel,
} from './skills.helpers'

describe('skillCommand', () => {
  it('asks for the skill by name, without any one agent’s prefix', () => {
    expect(skillCommand('inbox-sweep')).toBe('use the inbox-sweep skill')
  })
})

describe('neoName', () => {
  it('namespaces a bare suffix under neo-', () => {
    expect(neoName('weather')).toBe('neo-weather')
  })

  it('does not double an over-typed neo- prefix', () => {
    expect(neoName('neo-weather')).toBe('neo-weather')
  })

  it('leaves an interior neo- untouched', () => {
    expect(neoName('weather-neo-check')).toBe('neo-weather-neo-check')
  })
})

describe('parseSkillBody', () => {
  const body = [
    '---',
    'name: inbox-sweep',
    'description: "Check the inbox"',
    'tools: browseros-neo',
    '---',
    '',
    '## Steps',
    '1. Call the mark_skill_run tool with name: inbox-sweep so this run is recorded.',
    '2. Open the inbox',
    '3. Draft replies',
    '',
    '## Learned from past runs',
    '- Read the DOM snapshot, not screenshots',
    '- Leave drafts unsent',
  ].join('\n')

  it('extracts the user steps and drops the auto run-marking step', () => {
    expect(parseSkillBody(body).steps).toEqual([
      'Open the inbox',
      'Draft replies',
    ])
  })

  it('extracts the learned notes', () => {
    expect(parseSkillBody(body).learnedNotes).toEqual([
      'Read the DOM snapshot, not screenshots',
      'Leave drafts unsent',
    ])
  })

  it('returns empty arrays for a body with no sections', () => {
    expect(parseSkillBody('just prose')).toEqual({
      steps: [],
      learnedNotes: [],
    })
  })
})

describe('usedLabel', () => {
  const now = new Date(2026, 9, 23, 9).getTime()

  it('says when a skill has never run', () => {
    expect(usedLabel(0, undefined, now)).toBe('Not used yet')
  })

  it('counts runs and names the last day in plain words', () => {
    expect(usedLabel(1, new Date(2026, 9, 23, 8).getTime(), now)).toBe(
      'Used once · last used today',
    )
    expect(usedLabel(4, new Date(2026, 9, 22, 23).getTime(), now)).toBe(
      'Used 4 times · last used yesterday',
    )
    expect(usedLabel(4, new Date(2026, 9, 2).getTime(), now)).toMatch(
      /^Used 4 times · last used on /,
    )
  })
})

describe('stuckWhile', () => {
  it('names what the run was doing when it got stuck', () => {
    expect(stuckWhile('navigate')).toBe('Got stuck while opening a page.')
    expect(stuckWhile('act')).toBe('Got stuck while clicking on a page.')
    expect(stuckWhile(undefined)).toBe('Got stuck before it finished.')
  })
})
