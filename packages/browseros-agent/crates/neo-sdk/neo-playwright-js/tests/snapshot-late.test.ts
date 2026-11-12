// A whole-page v1 snapshot whose call's timeout passes after its tree arrived: the owner's run 2
// (export-20261112-115525), where a tree Rust took in time reached the realm 640 ms past the
// 5,000 ms default and the call threw a TimeoutError with no op behind it, which the run reported
// as the script's own error. Rust now decides whether the call's time is gone, at its baseline
// ops (neo-playwright snapshot_calls.rs); `LATE_SNAPSHOT` (snapshot-late.ts) chooses between
// A, the snapshot returned with no baseline recorded, and B, the call failed with Rust's
// timeout. The committed prelude is A; B is checked on a build with that one line flipped. The
// host stands in for Rust: it answers a baseline op sent at or after the tree op's deadline with
// the timeout Rust gives, and the realm's clock moves to each op's arrival, as the diagnosis's
// replay did. The attribution audit's cases (repro/timeout-attribution-audit.json) close it.

import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { NO_BASELINE, NO_CHANGE, NO_TIME } from '../src/prelude/snapshot-delta'
import type { SnapshotForm, SnapshotResult } from '../src/prelude/wire'
import { type Call, loadPrelude, SEED } from './support/prelude-runtime'

/** The realm's `Date.now()` when each call starts. */
const START = 100_000
/** Neo's default timeout, which the tree op and its deadline take. */
const TIMEOUT = 5_000

/** The committed prelude (A), and the same build with `LATE_SNAPSHOT` set to B. */
const committed = readFileSync(
  new URL('../../neo-playwright/assets/prelude.js', import.meta.url),
  'utf8',
)
const SWITCH = 'var LATE_SNAPSHOT = "return";'
const PRELUDE_B = committed.replace(SWITCH, 'var LATE_SNAPSHOT = "fail";')

/** A main section with one button reading `label`, then twenty links that never change. */
const tree = (label: string): SnapshotResult => ({
  json: [
    {
      role: 'main',
      ref: 'e2',
      children: [
        { role: 'button', name: label, ref: 'e3' },
        ...Array.from({ length: 20 }, (_, n) => ({
          role: 'link' as const,
          name: `Story ${n}`,
          ref: `e${n + 4}`,
        })),
      ],
    },
  ],
  partial: [],
  document: 1,
  snapshotCall: 7,
})
const page = (label: string): string =>
  [
    '- main [ref=e2]:',
    `  - button "${label}" [ref=e3]`,
    ...Array.from(
      { length: 20 },
      (_, n) => `  - link "Story ${n}" [ref=e${n + 4}]`,
    ),
  ].join('\n')

type PendingInput = {
  page: { tab: number }
  snapshotCall?: number
  text: string
  form: SnapshotForm
  returned?: string
}

/** When (ms after the call started) each op of a run lands; the realm's clock reads it then. */
type Arrivals = {
  snapshot: number
  /** When Rust checks a delta's baseline read; the tree's arrival when absent. */
  rust?: number
  /** When that read's reply reaches the realm; Rust's check when absent. */
  baseline?: number
  /** The call's timeout, as the tree op took it; 0 has none. */
  timeout?: number
}

/** What Rust answers a baseline op sent after its snapshot call's timeout passed. */
const rustTimeout = (phase: string, timeout: number) => ({
  err: {
    code: 'wait_timeout',
    owner: 'website',
    delivery: 'not_sent',
    retryable: true,
    operationId: 'op.rust',
    timeout: true,
    message: `page.ariaSnapshot: Timeout ${timeout}ms exceeded after the snapshot was taken, while ${phase}.`,
  },
})

/**
 * The host across runs: stored baselines by tab, each run's pending texts stored at its end when
 * the run ended ok with the text in its value (neo-server's RunReply::shows and
 * RunState::settle_snapshot_baselines), and Rust's decision on each baseline op.
 */
class Host {
  readonly stored = new Map<number, { text: string; form: SnapshotForm }>()
  label = 'Open'

  async run(
    code: string,
    arrivals: Arrivals = { snapshot: 0 },
    prelude?: string,
  ): Promise<{ settled: unknown; sent: Call[] }> {
    const pending: PendingInput[] = []
    const timeout = arrivals.timeout ?? TIMEOUT
    let now = 0
    // Rust's own clock: the tree op started with the call, so its deadline is the call's.
    const passed = () => timeout !== 0 && now >= timeout
    const runtime = loadPrelude({
      seed: SEED,
      now: START,
      prelude,
      answer: (call) => {
        const at = (ms: number) => {
          now = ms
          runtime.evaluate(`Date.now = () => ${START + ms}`)
        }
        if (call.name === 'page.ariaSnapshot') {
          at(arrivals.snapshot)
          return tree(this.label)
        }
        // Rust holds a baseline op to the deadline of the call it names.
        if (call.name.startsWith('page.snapshot'))
          expect((call.input as PendingInput).snapshotCall).toBe(7)
        if (call.name === 'page.snapshotBaseline') {
          now = arrivals.rust ?? now
          const late = passed()
          at(arrivals.baseline ?? now)
          if (late) return rustTimeout('reading its delta baseline', timeout)
          return this.stored.get((call.input as PendingInput).page.tab) ?? null
        }
        if (call.name === 'page.snapshotPending') {
          if (passed())
            return rustTimeout('recording its delta baseline', timeout)
          pending.push(call.input as PendingInput)
        }
        return null
      },
    })
    await runtime.ready
    const settled = await runtime.settle(code)
    if ('ok' in settled && typeof settled.ok === 'string') {
      const last = pending.at(-1)
      if (last && settled.ok.includes(last.returned ?? last.text))
        this.stored.set(last.page.tab, { text: last.text, form: last.form })
    }
    return { settled, sent: runtime.calls }
  }
}

const names = (sent: Call[]) => sent.map((call) => call.name)
const timeoutsSent = (sent: Call[]) =>
  sent.map((call) => (call.input.call as { timeoutMs: number }).timeoutMs)
const SNAPSHOT = 'return await page.ariaSnapshot()'
const DELTA = 'return await page.ariaSnapshot({ delta: true })'
/** The run's error under B: Rust's timeout of the call, with its op and the phase named. */
const failedB = (phase: string) => ({
  operation: rustTimeout(phase, TIMEOUT).err,
})

it('tests that the switch is one line of the committed prelude, set to A', () => {
  expect(committed.split(SWITCH)).toHaveLength(2)
  expect(PRELUDE_B).not.toBe(committed)
})

describe('A: a snapshot whose tree arrived is returned, with no baseline once the time is gone', () => {
  it('tests the replay of run 2: a tree at 4,999, 5,001 and 5,640 ms is returned every time, and only the one in time becomes a baseline', async () => {
    for (const [arrives, recorded] of [
      [4_999, true],
      [5_001, false],
      [5_640, false],
    ] as const) {
      const host = new Host()
      const { settled, sent } = await host.run(SNAPSHOT, { snapshot: arrives })
      expect(settled).toEqual({ ok: page('Open') })
      expect(names(sent)).toEqual(
        recorded
          ? ['page.ariaSnapshot', 'page.snapshotPending']
          : ['page.ariaSnapshot'],
      )
      if (recorded) expect(timeoutsSent(sent)).toEqual([TIMEOUT, 1])
      // The caller holds the late text, but no baseline was recorded for it: the next delta
      // compares against nothing.
      const next = await host.run(DELTA)
      expect(next.settled).toEqual({
        ok: recorded ? NO_CHANGE : `${NO_BASELINE}\n${page('Open')}`,
      })
    }
  })

  it('tests that a tree on the deadline itself sends nothing more, since 0 ms left means none', async () => {
    const { settled, sent } = await new Host().run(SNAPSHOT, {
      snapshot: TIMEOUT,
    })
    expect(settled).toEqual({ ok: page('Open') })
    expect(names(sent)).toEqual(['page.ariaSnapshot'])
  })

  it('tests that a delta too late to read its baseline is the whole page under a line that says why, and keeps the older baseline', async () => {
    const host = new Host()
    await host.run(SNAPSHOT)
    const before = host.stored.get(1007)
    expect(before?.text).toBe(page('Open'))
    host.label = 'Close'
    const late = await host.run(DELTA, { snapshot: 5_640 })
    expect(late.settled).toEqual({ ok: `${NO_TIME}\n${page('Close')}` })
    expect(names(late.sent)).toEqual(['page.ariaSnapshot'])
    // The caller holds both texts; the next delta compares against the older one it was given.
    expect(host.stored.get(1007)).toEqual(before)
    const next = await host.run(DELTA)
    expect(String((next.settled as { ok: string }).ok)).toStartWith(
      '# changes since your last snapshot of this tab: 1 added, 1 removed',
    )
  })

  it('tests that Rust timing out the baseline read, though the realm saw time left, still returns the whole page', async () => {
    const host = new Host()
    await host.run(SNAPSHOT)
    // The realm's clock says 4,990 ms; Rust's says the call's 5,000 ms have passed.
    const { settled, sent } = await host.run(DELTA, {
      snapshot: 4_990,
      rust: 5_010,
    })
    expect(settled).toEqual({ ok: `${NO_TIME}\n${page('Open')}` })
    expect(names(sent)).toEqual(['page.ariaSnapshot', 'page.snapshotBaseline'])
  })

  it('tests that a delta whose baseline read Rust answered in time, but which reached the realm late, is returned and records nothing', async () => {
    const host = new Host()
    await host.run(SNAPSHOT)
    host.label = 'Close'
    const late = await host.run(DELTA, { snapshot: 10, baseline: 5_200 })
    expect(String((late.settled as { ok: string }).ok)).toStartWith(
      '# changes since your last snapshot of this tab: 1 added, 1 removed',
    )
    expect(names(late.sent)).toEqual([
      'page.ariaSnapshot',
      'page.snapshotBaseline',
    ])
    expect(host.stored.get(1007)?.text).toBe(page('Open'))
  })
})

describe('B: the call fails with the timeout Rust decided, naming the phase', () => {
  it('tests the replay of run 2 under B: in time it is returned; late, every baseline op is sent and Rust fails the call', async () => {
    const inTime = await new Host().run(
      SNAPSHOT,
      { snapshot: 4_999 },
      PRELUDE_B,
    )
    expect(inTime.settled).toEqual({ ok: page('Open') })
    for (const arrives of [5_001, 5_640]) {
      const { settled, sent } = await new Host().run(
        SNAPSHOT,
        { snapshot: arrives },
        PRELUDE_B,
      )
      expect(settled).toEqual(failedB('recording its delta baseline'))
      expect(names(sent)).toEqual(['page.ariaSnapshot', 'page.snapshotPending'])
      expect(timeoutsSent(sent)).toEqual([TIMEOUT, 0])
    }
  })

  it('tests that a caught timeout under B is the TimeoutError Rust decided, with its op', async () => {
    const host = new Host()
    await host.run(SNAPSHOT)
    const { settled } = await host.run(
      `try { await page.ariaSnapshot({ delta: true }) } catch (error) {
        return [error.name, error.code, error.owner, error.operationId, error.message]
      }`,
      { snapshot: 5_640 },
      PRELUDE_B,
    )
    expect(settled).toEqual({
      ok: [
        'TimeoutError',
        'wait_timeout',
        'website',
        'op.rust',
        'page.ariaSnapshot: Timeout 5000ms exceeded after the snapshot was taken, while reading its delta baseline.',
      ],
    })
  })
})

describe("the attribution audit: only Rust's decision is a snapshot timeout", () => {
  for (const [variant, prelude] of [
    ['A', undefined],
    ['B', PRELUDE_B],
  ] as const) {
    it(`tests that a TimeoutError a script builds stays its own error (${variant})`, async () => {
      const { settled } = await new Host().run(
        'throw new TimeoutError("agent-created timeout")',
        undefined,
        prelude,
      )
      expect(settled).toEqual({
        error: { name: 'TimeoutError', message: 'agent-created timeout' },
      })
    })

    it(`tests that an error copying a snapshot timeout's name, fields and message stays the script's own (${variant})`, async () => {
      const message =
        'page.ariaSnapshot: Timeout 5000ms exceeded after the snapshot was taken, while recording its delta baseline.'
      const { settled } = await new Host().run(
        `const error = new Error(${JSON.stringify(message)})
        error.name = 'TimeoutError'; error.code = 'wait_timeout'; error.owner = 'website'
        error.delivery = 'not_sent'; error.timeout = true; error.operationId = 'op.rust'
        throw error`,
        undefined,
        prelude,
      )
      expect(settled).toEqual({ error: { name: 'TimeoutError', message } })
    })

    it(`tests that a snapshot's own late tree is never script_error (${variant})`, async () => {
      const { settled } = await new Host().run(
        SNAPSHOT,
        { snapshot: 5_001 },
        prelude,
      )
      expect(settled).toEqual(
        variant === 'A'
          ? { ok: page('Open') }
          : failedB('recording its delta baseline'),
      )
    })

    it(`tests that timeout 0 stays unlimited (${variant})`, async () => {
      const { settled, sent } = await new Host().run(
        'return await page.ariaSnapshot({ timeout: 0 })',
        { snapshot: 3_600_000, timeout: 0 },
        prelude,
      )
      expect(settled).toEqual({ ok: page('Open') })
      expect(names(sent)).toEqual(['page.ariaSnapshot', 'page.snapshotPending'])
      expect(timeoutsSent(sent)).toEqual([0, 0])
    })
  }
})
