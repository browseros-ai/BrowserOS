// Which text `ariaSnapshot({ delta: true })` compares against, as scripts see it: the last
// whole-page snapshot text the caller was given, decided when the run ends. The host here is a
// stand-in for neo-playwright's two baseline ops and neo-server's run end (RunReply::shows,
// RunState::settle_snapshot_baselines); their own rules (a new document, a closed tab, the size
// bound) are tested in Rust, and live in tests/prelude_live.rs and playwright_browseros.rs.
import { describe, expect, it } from 'bun:test'
import { NO_BASELINE, NO_CHANGE } from '../src/prelude/snapshot-delta'
import type { SnapshotForm, SnapshotResult } from '../src/prelude/wire'
import {
  type Call,
  loadPrelude,
  refusalCall,
  SEED,
} from './support/prelude-runtime'

const lines = (...text: string[]): string => text.join('\n')

/** A main section whose first button reads `label`, then twenty links that never change. */
const tree = (label: string): SnapshotResult => ({
  json: [
    {
      role: 'main',
      ref: 'e2',
      children: [
        { role: 'button', name: label, ref: 'e3' },
        { role: 'link', name: 'Help', ref: 'e4' },
        ...Array.from({ length: 20 }, (_, n) => ({
          role: 'link' as const,
          name: `Story ${n}`,
          ref: `e${n + 5}`,
        })),
      ],
    },
  ],
  partial: [],
  document: 1,
})
const stories = Array.from(
  { length: 20 },
  (_, n) => `  - link "Story ${n}" [ref=e${n + 5}]`,
)
const page = (label: string) =>
  lines(
    '- main [ref=e2]:',
    `  - button "${label}" [ref=e3]`,
    '  - link "Help" [ref=e4]',
    ...stories,
  )

type PendingInput = {
  page: { tab: number }
  text: string
  form: SnapshotForm
  returned?: string
}

/** How a default snapshot that kept its text was rendered. */
const KEPT: SnapshotForm = { text: 'kept', urls: false, maxTokens: 8000 }

/** What the host keeps per tab, as Rust's `Baseline` answers it. */
type Stored = { text: string; form: SnapshotForm }

/** Every string inside a returned value, as the run service looks for a snapshot in it. */
function strings(value: unknown): string[] {
  if (typeof value === 'string') return [value]
  if (Array.isArray(value)) return value.flatMap(strings)
  if (value !== null && typeof value === 'object')
    return Object.values(value).flatMap(strings)
  return []
}

/**
 * The host across runs: the stored baselines by tab, the page's button label (a click on any
 * button turns it to `Close`), and each run's pending texts, settled when the run ends.
 */
class Host {
  readonly stored = new Map<number, Stored>()
  label = 'Open'
  private pending: PendingInput[] = []

  /** One run in a fresh realm; its end commits or drops its pending texts. */
  async run(code: string): Promise<unknown> {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => this.answer(call),
    })
    let value: unknown
    let ok = true
    try {
      value = await runtime.run(code)
    } catch {
      ok = false
    }
    const pending = this.pending
    this.pending = []
    if (ok) {
      const given = strings(value)
      const committed = new Set<number>()
      for (const entry of [...pending].reverse()) {
        if (committed.has(entry.page.tab)) continue
        const returned = entry.returned ?? entry.text
        if (given.some((string) => string.includes(returned))) {
          this.stored.set(entry.page.tab, {
            text: entry.text,
            form: entry.form,
          })
          committed.add(entry.page.tab)
        }
      }
    }
    return value
  }

  private answer(call: Call): unknown {
    if (call.name.endsWith('.ariaSnapshot')) return tree(this.label)
    if (call.name === 'locator.click') this.label = 'Close'
    if (call.name === 'page.snapshotBaseline')
      return this.stored.get((call.input as PendingInput).page.tab) ?? null
    if (call.name === 'page.snapshotPending')
      this.pending.push(call.input as PendingInput)
    return null
  }
}

const changed = lines(
  '# changes since your last snapshot of this tab: 1 added, 1 removed',
  '@@ main [ref=e2]',
  ' - main [ref=e2]:',
  '-  - button "Open" [ref=e3]',
  '+  - button "Close" [ref=e3]',
  '   - link "Help" [ref=e4]',
  '   - link "Story 0" [ref=e5]',
  '   - link "Story 1" [ref=e6]',
)

describe('the baseline is the last text the caller was given', () => {
  it('tests the first snapshot, an unchanged one, and a change, across fresh runs', async () => {
    const host = new Host()
    const delta = 'return await page.ariaSnapshot({ delta: true })'
    expect(await host.run(delta)).toBe(`${NO_BASELINE}\n${page('Open')}`)
    expect(await host.run(delta)).toBe(NO_CHANGE)
    host.label = 'Close'
    expect(await host.run(delta)).toBe(changed)
    expect(await host.run(delta)).toBe(NO_CHANGE)
  })

  it('tests that a snapshot taken and then thrown away is not compared against', async () => {
    const host = new Host()
    await host.run('return await page.ariaSnapshot()')
    host.label = 'Close'
    // Each of these took the new text but never gave it to the caller.
    await host.run(
      "await page.ariaSnapshot(); throw new Error('after the snapshot')",
    )
    await host.run('const snap = await page.ariaSnapshot(); return snap.length')
    await host.run('return (await page.ariaSnapshot()).slice(0, 40)')
    await host.run(
      "const snap = await page.ariaSnapshot(); return snap.includes('Close')",
    )
    expect(host.stored.get(1007)?.text).toBe(page('Open'))
    expect(
      await host.run('return await page.ariaSnapshot({ delta: true })'),
    ).toBe(changed)
  })

  it('tests that a snapshot inside a returned object counts as given', async () => {
    const host = new Host()
    // Console lines count too: that half is RunReply::shows, tested in neo-server.
    await host.run('return { snap: [await page.ariaSnapshot()] }')
    expect(host.stored.get(1007)).toEqual({ text: page('Open'), form: KEPT })
  })

  it("tests that a delta in one run compares against the baseline from the run's start", async () => {
    const host = new Host()
    await host.run('return await page.ariaSnapshot()')
    host.label = 'Middle'
    const [full, delta] = (await host.run(`
      const full = await page.ariaSnapshot()
      await page.locator('button').click()
      return [full, await page.ariaSnapshot({ delta: true })]
    `)) as [string, string]
    expect(full).toBe(page('Middle'))
    // Against the text stored when the run started, not the run's own pending one.
    expect(delta).toBe(changed)
    expect(host.stored.get(1007)?.text).toBe(page('Close'))
  })

  it('tests that a locator snapshot with delta returns its ordinary text and sends no baseline op', async () => {
    const host = new Host()
    const calls: string[] = []
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => {
        calls.push(call.name)
        return call.name === 'locator.ariaSnapshot' ? tree('Open') : null
      },
    })
    expect(
      await runtime.run(
        "return await page.getByRole('main').ariaSnapshot({ delta: true })",
      ),
    ).toBe(page('Open'))
    expect(calls).toContain('locator.ariaSnapshot')
    expect(calls.filter((name) => name.startsWith('page.snapshot'))).toEqual([])
    expect(host.stored.size).toBe(0)
  })

  it("tests that mode 'default' ignores delta and sends no baseline op", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'page.ariaSnapshot' ? tree('Open') : null,
    })
    const yaml = await runtime.run(
      "return await page.ariaSnapshot({ mode: 'default', delta: true })",
    )
    expect(yaml).not.toContain('# ')
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'page.ariaSnapshot',
    ])
  })

  it('tests that under the conformance switch delta is ignored and no baseline op is sent', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      upstreamSnapshots: true,
      answer: (call) =>
        call.name === 'page.ariaSnapshot' ? tree('Open') : null,
    })
    await runtime.run("await page.ariaSnapshot({ mode: 'ai', delta: true })")
    await runtime.run('await page.ariaSnapshot({ delta: true })')
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'page.ariaSnapshot',
      'page.ariaSnapshot',
    ])
  })
})

describe('what the baseline ops are sent', () => {
  it("tests that a full call records its text once, with the snapshot's document and timeout", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'page.ariaSnapshot' ? tree('Open') : null,
    })
    const text = String(
      await runtime.run(
        'return await page.ariaSnapshot({ maxTokens: 5, timeout: 900 })',
      ),
    )
    expect(text).toContain('# cut:')
    const sent = runtime.calls.filter((call) =>
      call.name.startsWith('page.snapshot'),
    )
    expect(sent.map((call) => call.name)).toEqual(['page.snapshotPending'])
    const { call, ...input } = (sent[0]?.input ?? {}) as Record<
      string,
      unknown
    > & { call: { timeoutMs: number } }
    expect(input).toEqual({
      page: { $neo: 'page', tab: 1007 },
      document: 1,
      // The text after the notice and the cap: what the caller receives, and how it was
      // rendered, at the cap the call passed.
      text,
      form: { ...KEPT, maxTokens: 5 },
      delta: false,
    })
    // What is left of the script's 900 ms, never 0, which would mean no deadline.
    expect(call.timeoutMs).toBeGreaterThan(0)
    expect(call.timeoutMs).toBeLessThanOrEqual(900)
  })

  it('tests that a delta call reads the stored baseline, then records its text and its delta', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => {
        if (call.name === 'page.ariaSnapshot') return tree('Close')
        if (call.name === 'page.snapshotBaseline')
          return { text: page('Open'), form: KEPT }
        return null
      },
    })
    expect(
      await runtime.run('return await page.ariaSnapshot({ delta: true })'),
    ).toBe(changed)
    const sent = runtime.calls
      .filter((call) => call.name.startsWith('page.snapshot'))
      .map(
        ({
          name,
          input: { call: _call, ...input },
        }): Record<string, unknown> => ({
          name,
          ...input,
        }),
      )
    expect(sent).toEqual([
      {
        name: 'page.snapshotBaseline',
        page: { $neo: 'page', tab: 1007 },
        document: 1,
      },
      {
        name: 'page.snapshotPending',
        page: { $neo: 'page', tab: 1007 },
        document: 1,
        text: page('Close'),
        form: KEPT,
        delta: true,
        returned: changed,
      },
    ])
  })

  it('tests that the script aborting while the baseline is read rejects the snapshot', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: async (call) => {
        if (call.name === 'page.ariaSnapshot') return tree('Open')
        if (call.name !== 'page.snapshotBaseline' || call.abort === undefined)
          return null
        const reason = await call.abort.aborted
        return {
          err: {
            code: 'aborted',
            owner: 'script',
            delivery: 'not_sent',
            retryable: true,
            operationId: 'op.2',
            timeout: false,
            message: `page.ariaSnapshot: ${reason}\nCall log:\n  - operation was aborted: ${reason}\n`,
          },
        }
      },
    })
    const outcome = await runtime.run(`
      const listeners = []
      const signal = {
        aborted: false,
        reason: undefined,
        addEventListener(type, listener) { listeners.push(listener) },
        removeEventListener() {},
      }
      const pending = page.ariaSnapshot({ delta: true, signal }).catch((e) => e)
      await new Promise((resolve) => setTimeout(resolve, 10))
      signal.aborted = true
      signal.reason = new Error('stop')
      for (const listener of listeners) listener()
      const error = await pending
      return [error instanceof Error, error.name, error.code]
    `)
    expect(outcome).toEqual([true, 'AbortError', 'aborted'])
    expect(runtime.aborts.length).toBeGreaterThan(0)
    expect(runtime.sent('page.snapshotPending')).toEqual([])
  })

  it('tests that delta of the wrong type is refused before any op is sent', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const thrown = await runtime
      .run("await page.ariaSnapshot({ delta: 'yes' })")
      .then(
        () => null,
        (error: Error) => error.message,
      )
    expect(thrown).toBe(
      'page.ariaSnapshot: delta: expected boolean, got string',
    )
    expect(runtime.calls).toEqual([
      refusalCall('page.ariaSnapshot', 'invalid_input', thrown ?? ''),
    ])
  })
})
