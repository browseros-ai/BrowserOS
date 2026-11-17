import { describe, expect, it } from 'bun:test'
import type { Skill, SkillUltrafast } from '@browseros/claw-api'
import {
  chosenLine,
  durationLabel,
  healthLine,
  neoName,
  parseSkillBody,
  runModeCounts,
  runModeLabel,
  runSpeed,
  runsChoice,
  skillCommand,
  skillSites,
  stuckWhile,
  ultrafastBadge,
  usedLabel,
  worksInAgents,
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
  })

  it('names an older day by its date alone, as in "last used Oct 2"', () => {
    const label = usedLabel(4, new Date(2026, 9, 2).getTime(), now)
    expect(label).toMatch(/^Used 4 times · last used \S/)
    expect(label).not.toContain('last used on ')
  })
})

describe('skillSites', () => {
  it('has no sites when the skill names none', () => {
    expect(skillSites(undefined)).toEqual([])
    expect(skillSites('  ')).toEqual([])
  })

  it('keeps a single site as it was saved', () => {
    expect(skillSites('x.com')).toEqual(['x.com'])
  })

  it('names each site by its host, as a person would read it', () => {
    expect(
      skillSites('https://www.microsoftazuresponsorships.com/usage'),
    ).toEqual(['microsoftazuresponsorships.com'])
    expect(skillSites('https://trmnl.com')).toEqual(['trmnl.com'])
  })

  it('splits a comma list and drops empty entries', () => {
    expect(skillSites('x.com, linkedin.com,reddit.com, ')).toEqual([
      'x.com',
      'linkedin.com',
      'reddit.com',
    ])
  })
})

describe('worksInAgents', () => {
  const linked = (...agents: Skill['linkedAgents']) => ({
    linkedAgents: agents,
  })
  const everyone = ['Claude Code', 'Codex', 'Cursor'] as const

  it('lists every connected agent any skill is linked into, once, in the usual order', () => {
    expect(
      worksInAgents(
        [linked('Cursor', 'Claude Code'), linked('Codex', 'Claude Code')],
        everyone,
      ),
    ).toEqual(['Claude Code', 'Codex', 'Cursor'])
  })

  it('leaves out an agent that was disconnected after the skill was saved', () => {
    expect(
      worksInAgents([linked('Claude Code', 'Codex')], ['Claude Code']),
    ).toEqual(['Claude Code'])
    expect(worksInAgents([linked('Codex')], [])).toEqual([])
  })

  it('is empty when no skill is linked into an agent', () => {
    expect(worksInAgents([linked(), linked()], everyone)).toEqual([])
    expect(worksInAgents([], everyone)).toEqual([])
  })
})

describe('stuckWhile', () => {
  it('names what the run was doing when it got stuck', () => {
    expect(stuckWhile('navigate')).toBe('Got stuck while opening a page.')
    expect(stuckWhile('act')).toBe('Got stuck while clicking on a page.')
    expect(stuckWhile(undefined)).toBe('Got stuck before it finished.')
  })
})

const ultrafast = (patch: Partial<SkillUltrafast>): SkillUltrafast => ({
  status: 'verified',
  flowVersion: 1,
  writeApproved: false,
  ...patch,
})

describe('ultrafastBadge', () => {
  it('names each state and stays quiet for a plain skill', () => {
    expect(ultrafastBadge(ultrafast({}))?.label).toBe('Ultrafast')
    expect(ultrafastBadge(ultrafast({ status: 'degraded' }))?.tone).toBe(
      'warning',
    )
    expect(ultrafastBadge(ultrafast({ status: 'broken' }))?.tone).toBe('danger')
    expect(ultrafastBadge(ultrafast({ status: 'none' }))).toBeNull()
    expect(ultrafastBadge(undefined)).toBeNull()
  })
})

describe('healthLine', () => {
  it('says when it was checked and repaired, and how fast it runs', () => {
    const now = new Date(2026, 9, 29, 12).getTime()
    const yesterday = new Date(2026, 9, 28, 9).getTime()
    expect(
      healthLine(
        ultrafast({ verifiedAt: now, repairedAt: yesterday, p50Ms: 1240 }),
        now,
      ),
    ).toBe('Checked today · repaired yesterday · about 1.2 s a run')
    expect(runSpeed(40)).toBe('about 0.1 s a run')
  })
})

describe('runModeLabel', () => {
  it('reads a handover in plain words and leaves normal runs unlabelled', () => {
    expect(runModeLabel({ mode: 'ultrafast' })).toBe('Ultrafast')
    expect(
      runModeLabel({
        mode: 'handed_back',
        handoverStep: 2,
        handoverCause: 'site_changed',
      }),
    ).toBe('Took over at step 2: the site changed')
    expect(runModeLabel({ mode: 'agent' })).toBeNull()
    expect(runModeLabel({})).toBeNull()
  })

  it('counts runs by how they went', () => {
    expect(
      runModeCounts([
        { mode: 'ultrafast' },
        { mode: 'ultrafast' },
        { mode: 'handed_back' },
        {},
      ]),
    ).toEqual({ ultrafast: 2, handedBack: 1, normal: 1 })
  })
})

const writeRuns = (
  patch: Partial<NonNullable<SkillUltrafast['runs']>>,
): NonNullable<SkillUltrafast['runs']> => ({
  mode: 'with_agent',
  chosen: false,
  reask: false,
  click: 'Add it to the cart.',
  site: 'amazon.in',
  ...patch,
})

describe('runsChoice', () => {
  const agentRuns = [
    { mode: 'agent' as const, durationMs: 62_000 },
    { mode: 'ultrafast' as const, durationMs: 11_000 },
  ]

  it('reads a read-only skill as nothing to choose', () => {
    const choice = runsChoice(ultrafast({ p50Ms: 10_400 }), agentRuns)
    expect(choice?.state).toBe('read')
    expect(choice?.fastLabel).toBe('about 10 sec')
    expect(choice?.slowLabel).toBe('about 1 min')
  })

  it('runs with the agent until the user chooses', () => {
    const choice = runsChoice(
      ultrafast({ effects: 'write', runs: writeRuns({}) }),
      [],
    )
    expect(choice).toMatchObject({
      state: 'choose',
      selected: 'with_agent',
      chip: 'Choose how it runs',
      withoutLocked: false,
    })
    expect(choice?.slowLabel).toBeNull()
  })

  it('shows each chosen, re-asked, locked and always-with state', () => {
    const state = (runs: NonNullable<SkillUltrafast['runs']>, locked = false) =>
      runsChoice(ultrafast({ effects: 'write', runs, locked }), [])
    expect(
      state(writeRuns({ chosen: true, mode: 'without_agent' }))?.state,
    ).toBe('without')
    expect(state(writeRuns({ chosen: true }))?.state).toBe('with')
    expect(state(writeRuns({ reask: true }))?.state).toBe('reask')
    expect(state(writeRuns({ alwaysWithAgent: 'order' }))).toMatchObject({
      state: 'always',
      withoutLocked: true,
      disabled: false,
    })
    expect(
      state(writeRuns({ chosen: true, mode: 'without_agent' }), true),
    ).toMatchObject({ state: 'locked', disabled: true, selected: 'with_agent' })
  })

  it('stays quiet without a working fast version', () => {
    expect(runsChoice(ultrafast({ status: 'pending' }), [])).toBeNull()
    expect(runsChoice(undefined, [])).toBeNull()
  })
})

describe('chosenLine', () => {
  it('says who chose and where', () => {
    const now = new Date(2026, 10, 11, 18).getTime()
    expect(
      chosenLine(
        writeRuns({
          chosen: true,
          via: 'chat',
          client: 'Claude Code',
          chosenAt: new Date(2026, 10, 11, 9).getTime(),
        }),
        now,
      ),
    ).toBe('You chose this in chat with Claude Code today')
    expect(
      chosenLine(writeRuns({ chosen: true, via: 'app', chosenAt: now }), now),
    ).toBe('You chose this in BrowserOS today')
    expect(chosenLine(writeRuns({}), now)).toBeNull()
  })
})

describe('durationLabel', () => {
  it('rounds to the way the page talks about time', () => {
    expect(durationLabel(1_180)).toBe('about 1 sec')
    expect(durationLabel(11_000)).toBe('about 11 sec')
    expect(durationLabel(92_000)).toBe('about 2 min')
    expect(durationLabel(undefined)).toBeNull()
  })
})
