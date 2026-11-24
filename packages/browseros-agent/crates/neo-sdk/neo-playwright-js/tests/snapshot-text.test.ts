// Who prints a snapshot's text (src/prelude/snapshot-text.ts). On the 'rust' path, a build with
// `SNAPSHOT_TEXT` at 'rust', the prelude keeps its option checks and their messages, sends one op
// with the `format` it chose, and returns the text Rust answers: no baseline op, nothing rendered
// here. On the 'prelude' path with the compare switch on, design 2.3's three properties hold:
// both texts come from one read (one tree op, which answers Rust's text beside the tree, and the
// prelude's own baseline ops as without the switch); a difference names the first differing
// line's number and the two lengths, never a text; every comparison is counted, and a no-time
// answer of either engine is counted apart and not compared.
import { describe, expect, it } from 'bun:test'
import { NO_TIME } from '../src/prelude/snapshot-delta'
import {
  type Call,
  loadPrelude,
  oldPathPrelude,
  SEED,
  withSnapshotText,
} from './support/prelude-runtime'

/** A main section with one button. */
const TREE = {
  json: [
    {
      role: 'main',
      ref: 'e2',
      children: [{ role: 'button', name: 'Go', ref: 'e3' }],
    },
  ],
  partial: [],
  document: 3,
  snapshotCall: 1,
}
/** What the prelude's own renderer prints for `TREE` in Neo's default format. */
const TEXT = '- main [ref=e2]:\n  - button "Go" [ref=e3]'

const thrown = (run: Promise<unknown>): Promise<string | null> =>
  run.then(
    () => null,
    (error: Error) => error.message,
  )

describe("the 'rust' path", () => {
  const rust = (upstreamSnapshots = false) =>
    loadPrelude({
      seed: SEED,
      upstreamSnapshots,
      prelude: withSnapshotText('rust'),
      answer: (call) => {
        if (call.name.endsWith('.ariaSnapshot'))
          return { snapshot: `printed by Rust for ${call.name}` }
        if (call.name.endsWith('.ariaSnapshotJSON'))
          return { snapshot: TREE.json }
        return null
      },
    })
  const sent = (calls: Call[]) =>
    calls.map(({ name, input }) => ({
      name,
      mode: input.mode,
      format: input.format,
      clickable: input.clickable,
      compare: input.compare,
    }))

  it('tests that each call sends its format with every option as the script passed it, and returns the text', async () => {
    const runtime = rust()
    const texts = await runtime.run(`
      const l = page.getByRole('main')
      return [
        await page.ariaSnapshot(),
        await page.ariaSnapshot({ text: false, urls: true, maxTokens: 2.5, inert: true }),
        await page.ariaSnapshot({ delta: true }),
        await page.ariaSnapshot({ mode: 'full' }),
        await page.ariaSnapshot({ mode: 'full', maxTokens: 30, text: false }),
        await page.ariaSnapshot({ mode: 'default', text: false }),
        await l.ariaSnapshot({ delta: true, text: true }),
        await l.ariaSnapshot({ mode: 'full' }),
      ]
    `)
    expect(texts).toEqual([
      ...Array(6).fill('printed by Rust for page.ariaSnapshot'),
      ...Array(2).fill('printed by Rust for locator.ariaSnapshot'),
    ])
    expect(sent(runtime.calls)).toEqual([
      {
        name: 'page.ariaSnapshot',
        mode: 'ai',
        format: { neo: { delta: false } },
      },
      {
        name: 'page.ariaSnapshot',
        mode: 'ai',
        format: {
          neo: {
            text: false,
            urls: true,
            maxTokens: 2.5,
            inert: true,
            delta: false,
          },
        },
      },
      {
        name: 'page.ariaSnapshot',
        mode: 'ai',
        format: { neo: { delta: true } },
      },
      // Left out stays left out: Rust owns the default cap.
      { name: 'page.ariaSnapshot', mode: 'ai', format: { full: {} } },
      {
        name: 'page.ariaSnapshot',
        mode: 'ai',
        format: { full: { maxTokens: 30 } },
      },
      { name: 'page.ariaSnapshot', mode: 'default', format: 'playwright' },
      // A locator's delta is checked and dropped: a section is never a baseline.
      {
        name: 'locator.ariaSnapshot',
        mode: 'ai',
        format: { neo: { text: true } },
      },
      { name: 'locator.ariaSnapshot', mode: 'ai', format: { full: {} } },
    ])
  })

  it("tests that the option checks and their messages are the prelude's, before any op", async () => {
    const runtime = rust()
    for (const [code, message] of [
      [
        "await page.ariaSnapshot({ mode: 'x' })",
        'page.ariaSnapshot: mode: expected one of (ai|default|full)',
      ],
      [
        "await page.ariaSnapshot({ text: 'yes' })",
        'page.ariaSnapshot: text: expected boolean, got string',
      ],
      [
        'await page.ariaSnapshot({ maxTokens: 0 })',
        'page.ariaSnapshot: maxTokens: expected a positive number, got 0',
      ],
      [
        "await page.ariaSnapshot({ mode: 'full', maxTokens: -1 })",
        'page.ariaSnapshot: maxTokens: expected a positive number, got -1',
      ],
      [
        "await page.getByRole('main').ariaSnapshot({ delta: 1 })",
        'locator.ariaSnapshot: delta: expected boolean, got number',
      ],
    ] as const)
      expect(await thrown(runtime.run(code))).toBe(message)
    expect(runtime.calls.some((call) => 'format' in call.input)).toBe(false)
  })

  it("tests that under the conformance switch every mode is Playwright's text of the mode as passed, which the op judges", async () => {
    const runtime = rust(true)
    await runtime.run(`
      await page.ariaSnapshot()
      await page.ariaSnapshot({ mode: 'ai', text: false })
      await page.ariaSnapshot({ mode: 'full' })
    `)
    expect(sent(runtime.calls)).toEqual([
      { name: 'page.ariaSnapshot', mode: 'default', format: 'playwright' },
      { name: 'page.ariaSnapshot', mode: 'ai', format: 'playwright' },
      { name: 'page.ariaSnapshot', mode: 'full', format: 'playwright' },
    ])
  })

  it('tests that no baseline op is sent and the JSON ops answer the tree', async () => {
    const runtime = rust()
    const tree = await runtime.run(`
      await page.ariaSnapshot({ delta: true })
      return await page.ariaSnapshotJSON({ mode: 'ai' })
    `)
    expect(JSON.parse(JSON.stringify(tree))).toEqual(TREE.json)
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'page.ariaSnapshot',
      'page.ariaSnapshotJSON',
    ])
  })
})

describe('the compare switch', () => {
  type Counters = { compared: number; noTime: number; differed: number }
  /** The 'prelude' path with the switch, Rust answering `rust` beside the tree. */
  const comparing = (
    counters: Counters,
    rust: (call: Call) => { snapshot: string; pending?: unknown },
  ) =>
    loadPrelude({
      seed: SEED,
      prelude: oldPathPrelude(),
      compareSnapshots: counters,
      answer: (call) => {
        if (call.name === 'page.ariaSnapshot')
          return { ...TREE, rust: rust(call) }
        if (call.name === 'locator.ariaSnapshot') {
          const { document: _document, snapshotCall: _call, ...section } = TREE
          return { ...section, rust: rust(call) }
        }
        return null
      },
    })
  const KEPT = { text: 'kept', urls: false, maxTokens: 8000 }
  const fresh = (): Counters => ({ compared: 0, noTime: 0, differed: 0 })

  it("tests that both texts come from one read: one tree op asked for Rust's text too, and the prelude's own baseline ops", async () => {
    const counters = fresh()
    const runtime = comparing(counters, () => ({
      snapshot: TEXT,
      pending: { text: TEXT, form: KEPT },
    }))
    expect(await runtime.run('return await page.ariaSnapshot()')).toBe(TEXT)
    expect(
      runtime.calls.map(({ name, input }) => ({
        name,
        format: input.format,
        compare: input.compare,
        clickable: input.clickable,
      })),
    ).toEqual([
      {
        name: 'page.ariaSnapshot',
        format: { neo: { delta: false } },
        compare: true,
        clickable: undefined,
      },
      {
        name: 'page.snapshotPending',
        format: undefined,
        compare: undefined,
        clickable: undefined,
      },
    ])
    expect(counters).toEqual({ compared: 1, noTime: 0, differed: 0 })
  })

  it('tests that a difference fails the call naming the line and both lengths, never a text', async () => {
    const counters = fresh()
    const secret = '- main [ref=e2]:\n  - button "Gone" [ref=e3]'
    const runtime = comparing(counters, () => ({
      snapshot: secret,
      pending: { text: secret, form: KEPT },
    }))
    const message = await thrown(runtime.run('await page.ariaSnapshot()'))
    expect(message).toBe(
      `page.ariaSnapshot: the two snapshot engines differ: text at line 2 (Rust's is ${secret.length} UTF-16 units, the prelude's ${TEXT.length})`,
    )
    expect(message).not.toContain('Gone')
    expect(message).not.toContain('"Go"')
    const form = comparing(counters, () => ({
      snapshot: TEXT,
      pending: { text: TEXT, form: { ...KEPT, urls: true } },
    }))
    expect(await thrown(form.run('await page.ariaSnapshot()'))).toBe(
      "page.ariaSnapshot: the two snapshot engines differ: pending form's urls",
    )
    const kept = comparing(counters, () => ({ snapshot: TEXT }))
    expect(await thrown(kept.run('await page.ariaSnapshot()'))).toBe(
      'page.ariaSnapshot: the two snapshot engines differ: pending baseline recorded by the prelude only',
    )
    expect(counters).toEqual({ compared: 3, noTime: 0, differed: 3 })
  })

  it('tests that every call is counted, each format and a locator, and a no-time answer apart', async () => {
    const script = `return [
      await page.ariaSnapshot({ mode: 'default' }),
      await page.ariaSnapshot({ mode: 'full' }),
      await page.getByRole('main').ariaSnapshot(),
      await page.ariaSnapshot({ delta: true }),
    ]`
    // What the prelude's own renderer prints for each, which Rust's answer then equals.
    const plain = loadPrelude({
      seed: SEED,
      prelude: oldPathPrelude(),
      answer: (call) => (call.name.endsWith('.ariaSnapshot') ? TREE : null),
    })
    const texts = (await plain.run(script)) as string[]
    const counters = fresh()
    let tree = 0
    const runtime = comparing(counters, () => {
      const text = texts[tree] ?? ''
      tree += 1
      // The delta's: Rust's clock found no time to read the stored text.
      if (tree === 4) return { snapshot: `${NO_TIME}\n${TEXT}` }
      return { snapshot: text }
    })
    expect(await runtime.run(script)).toEqual(texts)
    expect(counters).toEqual({ compared: 3, noTime: 1, differed: 0 })
  })

  it('tests that a script setting the switch changes nothing: it is read once, at the start', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      prelude: oldPathPrelude(),
      answer: (call) => (call.name === 'page.ariaSnapshot' ? TREE : null),
    })
    expect(
      await runtime.run(`
        globalThis.__neo_compare_snapshots = { compared: 0, noTime: 0, differed: 0 }
        return await page.ariaSnapshot()
      `),
    ).toBe(TEXT)
    expect(runtime.calls[0]?.input.compare).toBeUndefined()
    expect(runtime.calls[0]?.input.clickable).toBe(true)
  })
})
