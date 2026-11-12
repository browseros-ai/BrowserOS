// The form of a delta (design section 7): a delta call takes the v1 options it leaves out from
// how its baseline was rendered, renders the new snapshot exactly so, and gives the whole page
// when an option it passes disagrees. Scripts run through the built prelude against a stand-in
// host that keeps each tab's baseline across fresh runs, as neo-playwright's baseline ops and
// neo-server's run end do; the Rust half (the form is stored and dropped with its text) is tested
// in snapshot_baselines.rs and pending_baselines.rs.
import { describe, expect, it } from 'bun:test'
import { NO_BASELINE, NO_CHANGE, TOO_MUCH } from '../src/prelude/snapshot-delta'
import { renderSnapshotV1 } from '../src/prelude/snapshot-v1'
import type {
  SnapshotBaseline,
  SnapshotForm,
  SnapshotResult,
} from '../src/prelude/wire'
import { type Call, loadPrelude, SEED } from './support/prelude-runtime'

const TAB = 1007

type Node = SnapshotResult['json'][number]

/**
 * A page of `stories` stories, each a heading, a sentence and a link with a url: about 126
 * characters of text form and 63 without the text, so 95 stories are just under the
 * 12,000-character text limit and 96 just over. `changed` rewrites those stories' sentences;
 * `renamed` renames their links (a change the interactive form shows too).
 */
type Page = { stories: number; changed?: number[]; renamed?: number[] }

function tree({ stories, changed = [], renamed = [] }: Page): SnapshotResult {
  const story = (n: number): Node[] => [
    { role: 'heading', level: 2, name: `Story ${n}` },
    {
      role: 'paragraph',
      children: [
        changed.includes(n)
          ? `Story ${n} now says something else entirely.`
          : `Words of story ${n}, long enough to read as a sentence.`,
      ],
    },
    {
      role: 'link',
      name: renamed.includes(n) ? `Open ${n}` : `Read ${n}`,
      ref: `e${n + 10}`,
      url: `https://news.example/${n}`,
    },
  ]
  return {
    json: [
      {
        role: 'main',
        ref: 'e2',
        children: [
          { role: 'button', name: 'Menu', ref: 'e3' },
          ...Array.from({ length: stories }, (_, n) => story(n)).flat(),
        ],
      },
    ],
    partial: [],
    document: 1,
  }
}

type PendingInput = {
  page: { tab: number }
  text: string
  form: SnapshotForm
  returned?: string
}

/** Every string inside a returned value, as the run service looks for a snapshot in it. */
function strings(value: unknown): string[] {
  if (typeof value === 'string') return [value]
  if (Array.isArray(value)) return value.flatMap(strings)
  if (value !== null && typeof value === 'object')
    return Object.values(value).flatMap(strings)
  return []
}

/** The host across runs: the page as it is now, each tab's stored baseline, a run's pending. */
class Host {
  page: Page
  readonly stored = new Map<number, SnapshotBaseline>()
  private pending: PendingInput[] = []

  constructor(page: Page) {
    this.page = page
  }

  /** One run in a fresh realm; its end commits the last given text per tab, with its form. */
  async run(code: string): Promise<unknown> {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => this.answer(call),
    })
    const value = await runtime.run(code)
    const given = strings(value)
    const pending = this.pending
    this.pending = []
    for (const entry of [...pending].reverse()) {
      if (
        given.some((string) => string.includes(entry.returned ?? entry.text))
      ) {
        this.stored.set(entry.page.tab, { text: entry.text, form: entry.form })
        break
      }
    }
    return value
  }

  /** A run that returns one whole-page snapshot taken with `options`. */
  async snapshot(options: object = {}): Promise<string> {
    return String(
      await this.run(
        `return await page.ariaSnapshot(${JSON.stringify(options)})`,
      ),
    )
  }

  /** A run that returns a delta taken with `options` besides `delta: true`. */
  async delta(options: object = {}): Promise<string> {
    return this.snapshot({ ...options, delta: true })
  }

  /** The tab's stored form. */
  form(): SnapshotForm | undefined {
    return this.stored.get(TAB)?.form
  }

  private answer(call: Call): unknown {
    if (call.name.endsWith('.ariaSnapshot')) {
      const { document: _document, ...section } = tree(this.page)
      return call.name === 'page.ariaSnapshot' ? tree(this.page) : section
    }
    if (call.name === 'page.snapshotBaseline')
      return this.stored.get((call.input as PendingInput).page.tab) ?? null
    if (call.name === 'page.snapshotPending')
      this.pending.push(call.input as PendingInput)
    return null
  }
}

/** The page's v1 text as a plain call with `options` renders it. */
function whole(
  page: Page,
  options: { text?: boolean; urls?: boolean; maxTokens?: number } = {},
): string {
  return renderSnapshotV1(tree(page), {
    text: options.text,
    urls: options.urls ?? false,
    maxTokens: options.maxTokens ?? 8_000,
  }).text
}

const SMALL: Page = { stories: 40 }
const BIG: Page = { stories: 100 }
const NOTICE =
  /^# text left out: about \d+ tokens\. Pass \{ text: true \} to include it\.$/

/** The first line and the lines after it. */
function split(reply: string): [string, string] {
  const at = reply.indexOf('\n')
  return at === -1 ? [reply, ''] : [reply.slice(0, at), reply.slice(at + 1)]
}

describe('the pages the stand-in serves', () => {
  it('tests that the small page keeps its text, the big one leaves it out, and 95 and 96 stories sit on either side of the limit', () => {
    expect(whole(SMALL)).toBe(whole(SMALL, { text: true }))
    expect(whole(BIG).split('\n')[0]).toMatch(NOTICE)
    expect(whole({ stories: 95 })).toBe(whole({ stories: 95 }, { text: true }))
    expect(whole({ stories: 96 }).split('\n')[0]).toMatch(NOTICE)
  })
})

describe("the ruling's table: every last whole page against every delta call, on an unchanged page", () => {
  // [last whole page given, the page, the delta call's text option, what comes back]
  type Row = [string, Page, object | null, object, 'no change' | object]
  const rows: Row[] = [
    ['default, text kept', SMALL, {}, {}, 'no change'],
    ['default, text kept', SMALL, {}, { text: true }, 'no change'],
    ['default, text kept', SMALL, {}, { text: false }, { text: false }],
    ['default, text left out', BIG, {}, {}, 'no change'],
    ['default, text left out', BIG, {}, { text: true }, { text: true }],
    ['default, text left out', BIG, {}, { text: false }, 'no change'],
    ['text: true', BIG, { text: true }, {}, 'no change'],
    ['text: true', BIG, { text: true }, { text: true }, 'no change'],
    ['text: true', BIG, { text: true }, { text: false }, { text: false }],
    ['text: false', SMALL, { text: false }, {}, 'no change'],
    ['text: false', SMALL, { text: false }, { text: true }, { text: true }],
    ['text: false', SMALL, { text: false }, { text: false }, 'no change'],
  ]
  for (const [last, page, given, call, expected] of rows)
    it(`tests ${last}, then ${JSON.stringify({ delta: true, ...call })}`, async () => {
      const host = new Host(page)
      await host.snapshot(given ?? {})
      const reply = await host.delta(call)
      if (expected === 'no change') expect(reply).toBe(NO_CHANGE)
      // A passed option that disagrees: never compared, the whole page in the asked form.
      else expect(reply).toBe(`${TOO_MUCH}\n${whole(page, expected)}`)
    })

  it('tests that with no baseline each call gives the first line and the page in its own form', async () => {
    for (const call of [{}, { text: true }, { text: false }]) {
      const host = new Host(BIG)
      expect(await host.delta(call)).toBe(`${NO_BASELINE}\n${whole(BIG, call)}`)
    }
  })

  it('tests that a delta never says "no change" about text the caller was not given', async () => {
    // The caller holds the page without its text; asking for the text is never answered from
    // a comparison that could not see it.
    for (const given of [{}, { text: false }]) {
      const host = new Host(given.text === false ? SMALL : BIG)
      await host.snapshot(given)
      const reply = await host.delta({ text: true })
      expect(reply).not.toBe(NO_CHANGE)
      expect(split(reply)[0]).toBe(TOO_MUCH)
      expect(split(reply)[1]).toBe(whole(host.page, { text: true }))
    }
  })
})

describe('the form of a delta', () => {
  it("tests the owner's trap: { text: true } on a page that leaves text out by default, a change, then { delta: true }", async () => {
    const host = new Host(BIG)
    const given = await host.snapshot({ text: true })
    expect(given).toBe(whole(BIG, { text: true }))
    host.page = { ...BIG, changed: [7] }
    const reply = await host.delta()
    expect(reply).toBe(
      [
        '# changes since your last snapshot of this tab: 1 added, 1 removed',
        '@@ main [ref=e2]',
        '   - text: Words of story 6, long enough to read as a sentence.',
        '   - link "Read 6" [ref=e16]',
        '   - heading "Story 7" [level=2]',
        '-  - text: Words of story 7, long enough to read as a sentence.',
        '+  - text: Story 7 now says something else entirely.',
        '   - link "Read 7" [ref=e17]',
        '   - heading "Story 8" [level=2]',
        '   - text: Words of story 8, long enough to read as a sentence.',
      ].join('\n'),
    )
    // The new text in the baseline's form, with that form.
    expect(host.stored.get(TAB)).toEqual({
      text: whole(host.page, { text: true }),
      form: { text: 'always', urls: false, maxTokens: 8_000 },
    })
  })

  it('tests that an unchanged page answers "no change" for every baseline, alone and with the same option repeated', async () => {
    const cases: [string, Page, object][] = [
      ['default, text kept', SMALL, {}],
      ['default, text left out', BIG, {}],
      ['text: true', BIG, { text: true }],
      ['text: false', SMALL, { text: false }],
      ['urls: true', SMALL, { urls: true }],
      ['maxTokens above the default', BIG, { maxTokens: 20_000 }],
      ['maxTokens below the default', BIG, { maxTokens: 900 }],
      ['text left out and cut', BIG, { maxTokens: 900 }],
      ['text: true and cut', BIG, { text: true, maxTokens: 900 }],
    ]
    for (const [name, page, options] of cases) {
      const host = new Host(page)
      await host.snapshot(options)
      expect([name, await host.delta()]).toEqual([name, NO_CHANGE])
      expect([name, await host.delta(options)]).toEqual([name, NO_CHANGE])
    }
  })

  it('tests that a baseline cut under the notice is compared under the notice, whose line takes its share of the cap', async () => {
    const host = new Host(BIG)
    const given = await host.snapshot({ maxTokens: 900 })
    expect(given.split('\n')[0]).toMatch(NOTICE)
    expect(given).toContain('\n# cut: ')
    // Without the notice the cap keeps a line more: the same page as text: false is not the
    // text the caller holds, so a delta rendered that way would list a change that did not
    // happen.
    expect(whole(BIG, { text: false, maxTokens: 900 })).not.toBe(
      given.split('\n').slice(1).join('\n'),
    )
    expect(await host.delta()).toBe(NO_CHANGE)
    expect(await host.delta({ text: false })).toBe(NO_CHANGE)
    host.page = { ...BIG, renamed: [1] }
    expect(split(await host.delta())[0]).toBe(
      '# changes since your last snapshot of this tab: 1 added, 1 removed',
    )
  })

  it("tests that urls or maxTokens other than the baseline's give the whole page at those options, which is then the baseline", async () => {
    for (const call of [
      { urls: true },
      { maxTokens: 20_000 },
      { maxTokens: 900 },
    ]) {
      const host = new Host(BIG)
      await host.snapshot()
      expect(await host.delta(call)).toBe(`${TOO_MUCH}\n${whole(BIG, call)}`)
      expect(await host.delta()).toBe(NO_CHANGE)
    }
    const host = new Host(SMALL)
    await host.snapshot({ urls: true })
    expect(await host.delta({ urls: false })).toBe(
      `${TOO_MUCH}\n${whole(SMALL)}`,
    )
  })

  it('tests the 3,000-token line: a baseline that kept its text keeps comparing with text after the page grows past it', async () => {
    const host = new Host({ stories: 95 })
    await host.snapshot()
    expect(host.form()?.text).toBe('kept')
    host.page = { stories: 97 }
    const reply = await host.delta()
    expect(split(reply)[0]).toBe(
      '# changes since your last snapshot of this tab: 6 added, 0 removed',
    )
    expect(reply).toContain(
      '+  - text: Words of story 95, long enough to read as a sentence.',
    )
    expect(reply).not.toContain('# text left out')
    // Stored in the baseline's form, the length rule not asked again.
    expect(host.stored.get(TAB)).toEqual({
      text: whole({ stories: 97 }, { text: true }),
      form: { text: 'kept', urls: false, maxTokens: 8_000 },
    })
  })

  it('tests the 3,000-token line the other way: a baseline under the notice keeps comparing without text after the page shrinks under it', async () => {
    const host = new Host({ stories: 97 })
    await host.snapshot()
    expect(host.form()?.text).toBe('leftOut')
    host.page = { stories: 94 }
    const reply = await host.delta()
    expect(split(reply)[0]).toBe(
      '# changes since your last snapshot of this tab: 0 added, 6 removed',
    )
    expect(reply).not.toContain('- text:')
    expect(host.stored.get(TAB)?.text.split('\n')[0]).toMatch(NOTICE)
    expect(host.form()?.text).toBe('leftOut')
  })

  it('tests too much: after { text: true } the whole page comes back with its text', async () => {
    const host = new Host(BIG)
    await host.snapshot({ text: true })
    const everyStory = Array.from({ length: 100 }, (_, n) => n)
    host.page = { ...BIG, changed: everyStory }
    expect(await host.delta()).toBe(
      `${TOO_MUCH}\n${whole(host.page, { text: true })}`,
    )
    expect(host.form()?.text).toBe('always')
  })

  it('tests too much: after a default that kept its text, a page now big comes back under the notice, the length rule asked again', async () => {
    const host = new Host(SMALL)
    await host.snapshot()
    host.page = { stories: 200 }
    const reply = await host.delta()
    const [first, rest] = split(reply)
    expect(first).toBe(TOO_MUCH)
    expect(rest.split('\n')[0]).toMatch(NOTICE)
    expect(rest).toBe(whole(host.page))
    expect(host.form()?.text).toBe('leftOut')
  })

  it('tests too much under the notice with text: false passed: the whole page without text and without the notice', async () => {
    const host = new Host(BIG)
    await host.snapshot()
    host.page = { ...BIG, renamed: Array.from({ length: 100 }, (_, n) => n) }
    expect(await host.delta({ text: false })).toBe(
      `${TOO_MUCH}\n${whole(host.page, { text: false })}`,
    )
    expect(host.form()?.text).toBe('never')
  })

  it('tests a chain: each delta lists only the change since the last, in the text form the chain began with', async () => {
    const host = new Host(BIG)
    await host.snapshot({ text: true })
    host.page = { ...BIG, changed: [3] }
    expect(await host.delta()).toContain('+  - text: Story 3 now says')
    host.page = { ...BIG, changed: [3, 60] }
    const second = await host.delta()
    expect(split(second)[0]).toBe(
      '# changes since your last snapshot of this tab: 1 added, 1 removed',
    )
    expect(second).toContain('+  - text: Story 60 now says')
    expect(second).not.toContain('Story 3 now says')
  })

  it('tests that after a whole page in another form the next delta is in that form', async () => {
    const host = new Host(BIG)
    await host.snapshot({ text: true })
    expect(await host.delta({ text: false })).toBe(
      `${TOO_MUCH}\n${whole(BIG, { text: false })}`,
    )
    expect(host.form()).toEqual({
      text: 'never',
      urls: false,
      maxTokens: 8_000,
    })
    host.page = { ...BIG, changed: [5], renamed: [5] }
    const reply = await host.delta()
    expect(split(reply)[0]).toBe(
      '# changes since your last snapshot of this tab: 1 added, 1 removed',
    )
    expect(reply).not.toContain('- text:')
  })

  it("tests that a locator's snapshot in between neither is a baseline nor changes one", async () => {
    const host = new Host(BIG)
    await host.snapshot({ text: true })
    const before = host.stored.get(TAB)
    const section = String(
      await host.run(
        "return await page.getByRole('main').ariaSnapshot({ text: false, delta: true })",
      ),
    )
    expect(section.startsWith('#')).toBe(false)
    expect(host.stored.get(TAB)).toEqual(before)
    expect(await host.delta()).toBe(NO_CHANGE)
  })
})
