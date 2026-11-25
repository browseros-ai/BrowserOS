import { describe, expect, it } from 'bun:test'
import type { Skill, SkillUltrafast } from '@browseros/claw-api'
import {
  awaitsFirstRun,
  cardStats,
  chosenLine,
  durationLabel,
  firstRunLearning,
  healthLine,
  matchesQuery,
  neoName,
  parseSkillBody,
  runModeCounts,
  runModeLabel,
  runSpeed,
  runsByItself,
  runsChoice,
  skillAgents,
  skillCommand,
  skillSites,
  skillValue,
  stillVerifying,
  stuckWhile,
  turnedUltrafast,
  ultrafastMark,
  upgradeSeconds,
  usageLevel,
  usedLabel,
  VERIFYING_POLL_MS,
  VERIFYING_WINDOW_MS,
  verifyingPoll,
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

describe('skillAgents', () => {
  const linked = (...agents: Skill['linkedAgents']) => ({
    linkedAgents: agents,
  })
  const everyone = ['Claude Code', 'Codex', 'Cursor'] as const

  it('lists the connected agents this skill is linked into, in the usual order', () => {
    expect(skillAgents(linked('Cursor', 'Claude Code'), everyone)).toEqual([
      'Claude Code',
      'Cursor',
    ])
    expect(skillAgents(linked('Codex'), everyone)).toEqual(['Codex'])
  })

  it('leaves out an agent that was disconnected after the skill was saved', () => {
    expect(
      skillAgents(linked('Claude Code', 'Codex'), ['Claude Code']),
    ).toEqual(['Claude Code'])
  })

  it('offers every connected agent for a skill saved with no links', () => {
    expect(skillAgents(linked(), everyone)).toEqual([...everyone])
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

describe('ultrafastMark', () => {
  it('shows the plain wordmark when verified, a dim wordmark and a word otherwise, and nothing for a plain skill', () => {
    expect(ultrafastMark(ultrafast({}))).toEqual({
      dim: false,
      word: null,
      tone: 'muted',
    })
    expect(ultrafastMark(ultrafast({ status: 'degraded' }))).toEqual({
      dim: true,
      word: 'needs a repair',
      tone: 'amber',
    })
    expect(ultrafastMark(ultrafast({ status: 'broken' }))).toMatchObject({
      word: 'paused',
      tone: 'red',
    })
    expect(ultrafastMark(ultrafast({ status: 'none' }))).toBeNull()
    expect(ultrafastMark(undefined)).toBeNull()
  })

  it('says a skill made from a task turns ultrafast after its first run', () => {
    const fromTask: Skill = {
      name: 'neo-flipkart-phones',
      description: 'Find phones under a budget on Flipkart',
      origin: 'history',
      version: 1,
      linkedAgents: [],
      runCount: 0,
      cleanRunCount: 0,
      createdAt: 0,
      updatedAt: 0,
      ultrafast: ultrafast({ status: 'none' }),
    }
    expect(awaitsFirstRun(fromTask)).toBe(true)
    expect(ultrafastMark(fromTask.ultrafast, true)).toEqual({
      dim: true,
      word: 'after its first run',
      tone: 'muted',
    })
    expect(awaitsFirstRun({ ...fromTask, runCount: 1 })).toBe(false)
    expect(awaitsFirstRun({ ...fromTask, origin: 'agent' })).toBe(false)
    expect(
      awaitsFirstRun({
        ...fromTask,
        ultrafast: ultrafast({ status: 'none', locked: true }),
      }),
    ).toBe(false)
    expect(ultrafastMark(ultrafast({ status: 'pending' }), true)?.word).toBe(
      'on the way',
    )
  })
})

describe('firstRunLearning', () => {
  const fromTask: Skill = {
    name: 'neo-phones',
    description: 'Find phones under a budget',
    origin: 'history',
    version: 1,
    linkedAgents: [],
    runCount: 0,
    cleanRunCount: 0,
    createdAt: 0,
    updatedAt: 0,
    ultrafast: ultrafast({ status: 'pending' }),
  }

  it('walks through the first run while neo checks the fast version', () => {
    expect(firstRunLearning(fromTask)?.map((step) => step.state)).toEqual([
      'done',
      'done',
      'now',
      'todo',
    ])
  })

  it('says it finishes on the next run once the page stops checking', () => {
    const steps = firstRunLearning(fromTask, false)
    expect(steps?.map((step) => step.text)).not.toContain(
      'neo is checking the fast version',
    )
    expect(steps?.at(-1)).toEqual({
      text: 'Finishes on its next run',
      state: 'todo',
    })
  })

  it('says nothing once verified, when locked, or for a skill an agent saved', () => {
    expect(
      firstRunLearning({
        ...fromTask,
        ultrafast: ultrafast({ status: 'verified', verifiedAt: 1 }),
      }),
    ).toBeNull()
    expect(
      firstRunLearning({
        ...fromTask,
        ultrafast: ultrafast({ status: 'pending', locked: true }),
      }),
    ).toBeNull()
    expect(firstRunLearning({ ...fromTask, origin: 'agent' })).toBeNull()
    expect(
      firstRunLearning({
        ...fromTask,
        ultrafast: ultrafast({ status: 'none' }),
      }),
    ).toBeNull()
  })
})

describe('turnedUltrafast', () => {
  const fromTask: Skill = {
    name: 'neo-phones',
    description: 'Find phones under a budget',
    origin: 'history',
    version: 1,
    linkedAgents: [],
    runCount: 1,
    cleanRunCount: 1,
    createdAt: 0,
    updatedAt: 0,
    ultrafast: ultrafast({ status: 'verified', verifiedAt: 5, p50Ms: 7300 }),
  }

  it('is the moment a task skill’s fast version is verified', () => {
    expect(turnedUltrafast(fromTask)).toBe(true)
    expect(turnedUltrafast({ ...fromTask, origin: 'agent' })).toBe(false)
    expect(
      turnedUltrafast({
        ...fromTask,
        ultrafast: ultrafast({ status: 'pending' }),
      }),
    ).toBe(false)
  })

  it('rounds the speed to whole seconds, never below one', () => {
    expect(upgradeSeconds(7300)).toBe(7)
    expect(upgradeSeconds(200)).toBe(1)
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

describe('verifyingPoll', () => {
  const pending: Skill = {
    name: 'neo-phones',
    description: 'Find phones under a budget',
    origin: 'history',
    version: 1,
    linkedAgents: [],
    runCount: 0,
    cleanRunCount: 0,
    createdAt: 0,
    updatedAt: 0,
    ultrafast: ultrafast({ status: 'pending' }),
  }
  const openedAt = 1_000_000

  it('checks a pending fast version while the page is fresh', () => {
    expect(verifyingPoll(pending, openedAt, openedAt + 30_000)).toBe(
      VERIFYING_POLL_MS,
    )
  })

  it('runs one check past the window, then stops for good', () => {
    expect(
      verifyingPoll(pending, openedAt, openedAt + VERIFYING_WINDOW_MS + 1),
    ).toBe(VERIFYING_POLL_MS)
    expect(
      verifyingPoll(
        pending,
        openedAt,
        openedAt + VERIFYING_WINDOW_MS + VERIFYING_POLL_MS,
      ),
    ).toBe(false)
    expect(verifyingPoll(pending, openedAt, openedAt + 3 * 86_400_000)).toBe(
      false,
    )
  })

  it('never checks a skill that is not pending', () => {
    expect(
      verifyingPoll(
        { ...pending, ultrafast: ultrafast({ status: 'verified' }) },
        openedAt,
        openedAt,
      ),
    ).toBe(false)
    expect(verifyingPoll(undefined, openedAt, openedAt)).toBe(false)
  })

  it('reads as still checking only for data fetched inside the window', () => {
    expect(stillVerifying(openedAt, openedAt + 500)).toBe(true)
    expect(stillVerifying(openedAt, openedAt - 60_000)).toBe(true)
    expect(stillVerifying(openedAt, openedAt + VERIFYING_WINDOW_MS)).toBe(false)
  })
})

describe('runsByItself', () => {
  const skill = (patch: Partial<SkillUltrafast>): Skill => ({
    name: 'neo-phones',
    description: 'Find phones under a budget',
    origin: 'history',
    version: 1,
    linkedAgents: [],
    runCount: 1,
    cleanRunCount: 1,
    createdAt: 0,
    updatedAt: 0,
    ultrafast: ultrafast({ status: 'verified', verifiedAt: 5, ...patch }),
  })

  it('is true for a read flow and a write chosen to run without the agent', () => {
    expect(runsByItself(skill({}))).toBe(true)
    expect(
      runsByItself(
        skill({
          effects: 'write',
          runs: writeRuns({ chosen: true, mode: 'without_agent' }),
        }),
      ),
    ).toBe(true)
  })

  it('is false when the plan is off or the run still needs the agent', () => {
    expect(runsByItself(skill({ locked: true }))).toBe(false)
    expect(
      runsByItself(
        skill({
          effects: 'write',
          runs: writeRuns({ alwaysWithAgent: 'pay' }),
        }),
      ),
    ).toBe(false)
    expect(
      runsByItself(
        skill({
          effects: 'write',
          runs: writeRuns({}),
        }),
      ),
    ).toBe(false)
  })
})

describe('cardStats', () => {
  const skill = (patch: Partial<Skill>): Skill => ({
    name: 'neo-hn',
    description: 'Hacker News top 10',
    origin: 'agent',
    version: 1,
    linkedAgents: [],
    runCount: 0,
    cleanRunCount: 0,
    createdAt: 0,
    updatedAt: 0,
    ...patch,
  })

  it('shows speed and runs for a skill that runs the fast way', () => {
    expect(
      cardStats(
        skill({
          runCount: 32,
          ultrafast: ultrafast({ status: 'verified', p50Ms: 1830 }),
        }),
      ),
    ).toEqual({ choice: null, speed: '1.8 s', uses: '32 runs' })
  })

  it('shows runs only for a skill that runs with the agent or on a plan without ultrafast', () => {
    const always = ultrafast({
      status: 'verified',
      effects: 'write',
      p50Ms: 900,
      runs: writeRuns({ alwaysWithAgent: 'order' }),
    })
    expect(cardStats(skill({ runCount: 1, ultrafast: always }))).toEqual({
      choice: null,
      speed: null,
      uses: '1 run',
    })
    expect(
      cardStats(
        skill({
          runCount: 3,
          ultrafast: ultrafast({
            status: 'verified',
            p50Ms: 900,
            locked: true,
          }),
        }),
      ),
    ).toEqual({ choice: null, speed: null, uses: '3 runs' })
  })

  it('asks for a choice a write skill still needs', () => {
    const choose = ultrafast({
      status: 'verified',
      effects: 'write',
      runs: writeRuns({}),
    })
    expect(cardStats(skill({ ultrafast: choose })).choice).toBe(
      'Choose how it runs',
    )
    const reask = ultrafast({
      status: 'verified',
      effects: 'write',
      runs: writeRuns({ reask: true }),
    })
    expect(cardStats(skill({ ultrafast: reask })).choice).toBe('Asks again')
  })

  it('says not used yet only when no ultrafast word fills the line', () => {
    expect(cardStats(skill({})).uses).toBe('Not used yet')
    expect(
      cardStats(skill({ ultrafast: ultrafast({ status: 'pending' }) })).uses,
    ).toBeNull()
  })
})

describe('matchesQuery', () => {
  const hn = {
    name: 'neo-hn-front-page',
    description: 'Hacker News top 10',
    site: 'news.ycombinator.com',
  }

  it('matches the title, the name or the site, in any case', () => {
    expect(matchesQuery(hn, 'hacker')).toBe(true)
    expect(matchesQuery(hn, 'FRONT-PAGE')).toBe(true)
    expect(matchesQuery(hn, 'ycombinator')).toBe(true)
    expect(matchesQuery(hn, 'flipkart')).toBe(false)
    expect(matchesQuery(hn, '  ')).toBe(true)
  })
})

describe('ultrafastMark on a plan without ultrafast', () => {
  it('shows no mark once the plan is off', () => {
    expect(
      ultrafastMark(ultrafast({ status: 'verified', locked: true })),
    ).toBeNull()
  })
})

describe('skillValue', () => {
  const now = new Date(2026, 10, 23, 12).getTime()
  const skill = (patch: Partial<Skill>): Skill => ({
    name: 'neo-hn',
    description: 'Hacker News top 10',
    origin: 'agent',
    version: 1,
    linkedAgents: [],
    runCount: 0,
    cleanRunCount: 0,
    createdAt: 0,
    updatedAt: 0,
    ...patch,
  })
  const agentRun = { mode: 'agent' as const, durationMs: 41_000 }

  it('sorts usage into never, rarely, sometimes and a lot', () => {
    expect([0, 1, 2, 3, 9, 10].map(usageLevel)).toEqual([
      'never',
      'rare',
      'rare',
      'sometimes',
      'sometimes',
      'lot',
    ])
  })

  it('invites the first run instead of showing empty figures', () => {
    expect(
      skillValue(
        skill({
          origin: 'history',
          ultrafast: ultrafast({ status: 'none' }),
        }),
        [],
        now,
      ),
    ).toMatchObject({ kind: 'invite', title: 'Run it once and neo learns it' })
    expect(
      skillValue(
        skill({ ultrafast: ultrafast({ status: 'verified', p50Ms: 6000 }) }),
        [agentRun],
        now,
      ),
    ).toMatchObject({
      kind: 'invite',
      title: 'Ready when you are',
      body: 'It takes about 6 sec a run, down from about 41 sec the normal way.',
    })
  })

  it('shows speed beside the runs only when runs go the fast way', () => {
    const fast = skill({
      runCount: 2,
      lastRunAt: now,
      ultrafast: ultrafast({ status: 'verified', p50Ms: 1800 }),
    })
    expect(skillValue(fast, [agentRun], now)).toEqual({
      kind: 'tiles',
      tiles: [
        {
          value: '1.8',
          unit: 's',
          label: 'a run, down from about 41 sec',
          fast: true,
        },
        { value: '2', unit: 'runs', label: 'last used today', fast: false },
      ],
    })
    expect(
      skillValue(
        skill({
          runCount: 5,
          lastRunAt: now,
          ultrafast: ultrafast({ status: 'degraded', p50Ms: 1800 }),
        }),
        [],
        now,
      ),
    ).toEqual({
      kind: 'tiles',
      tiles: [
        { value: '5', unit: 'runs', label: 'last used today', fast: false },
      ],
    })
  })
})
