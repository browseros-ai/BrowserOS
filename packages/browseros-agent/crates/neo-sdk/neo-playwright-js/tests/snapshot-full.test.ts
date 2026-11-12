// `ariaSnapshot({ mode: 'full' })` (design section 13): Playwright's own text for the engine's
// `'ai'` tree, which the v1 format reads too (marks included), cut at `maxTokens` by the v1
// format's cap. Scripts run through the built prelude against a stand-in host; the blind page's
// tree was recorded from a live engine (neo-playwright's prelude_live test), and its expected
// text is what Playwright 1.64.1's own `ariaSnapshot({ mode: 'ai' })` printed for the same page.
import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { renderFullSnapshot, renderSnapshot } from '../src/prelude/snapshot'
import { NO_CHANGE } from '../src/prelude/snapshot-delta'
import type { SnapshotBaseline, SnapshotResult } from '../src/prelude/wire'
import {
  type Call,
  loadPrelude,
  refusalCall,
  SEED,
} from './support/prelude-runtime'

const blindData = (name: string): string =>
  readFileSync(
    new URL(
      `../../neo-playwright/tests/data/snapshot-v1/${name}`,
      import.meta.url,
    ),
    'utf8',
  )

type Node = SnapshotResult['json'][number]

const buttons = (from: number, count: number): Node[] =>
  Array.from({ length: count }, (_, n) => ({
    role: 'button',
    name: `Button ${from + n}`,
    ref: `e${from + n}`,
  }))

const links = (from: number, count: number): Node[] =>
  Array.from({ length: count }, (_, n) => ({
    role: 'link',
    name: `Link ${from + n}`,
    ref: `e${from + n}`,
    url: `https://x.example/${from + n}`,
  }))

/**
 * A page with a banner of links, a main section, a named footer and a cookie dialog at the end
 * of the document, where sites put it. 1,011 characters of YAML, about 300 tokens.
 */
const PAGE: SnapshotResult = {
  json: [
    {
      role: 'generic',
      ref: 'e1',
      children: [
        { role: 'banner', ref: 'e2', children: links(10, 6) },
        {
          role: 'main',
          ref: 'e3',
          children: [
            { role: 'heading', name: 'Results', level: 1, ref: 'e4' },
            'Some words of text.',
            ...buttons(50, 6),
          ],
        },
        {
          role: 'contentinfo',
          name: 'Footer',
          ref: 'e5',
          children: buttons(70, 4),
        },
        {
          role: 'dialog',
          name: 'Cookies',
          ref: 'e6',
          children: [
            'We use cookies.',
            { role: 'button', name: 'Accept', ref: 'e7' },
          ],
        },
      ],
    },
  ],
  partial: [],
  document: 1,
}
const PAGE_LINES = renderSnapshot(PAGE).split('\n')
const DIALOG = [
  '- dialog "Cookies" [ref=e6]:',
  '  - text: We use cookies.',
  '  - button "Accept" [ref=e7]',
]

/** A run whose snapshot ops answer `result` (a locator's without the page's document). */
function snapshotting(result: SnapshotResult, upstreamSnapshots = false) {
  const { document: _document, ...section } = result
  return loadPrelude({
    seed: SEED,
    upstreamSnapshots,
    answer: (call) => {
      if (call.name === 'page.ariaSnapshot') return result
      if (call.name === 'locator.ariaSnapshot') return section
      return null
    },
  })
}

/** What a run threw, or null. */
async function thrown(
  runtime: ReturnType<typeof snapshotting>,
  code: string,
): Promise<string | null> {
  return runtime.run(code).then(
    () => null,
    (error: Error) => error.message,
  )
}

describe('the text', () => {
  it("tests that the blind page prints Playwright's own ai text, byte for byte", async () => {
    const blind = JSON.parse(blindData('blind.json')) as SnapshotResult
    const runtime = snapshotting(blind)
    const full = await runtime.run(
      "return await page.ariaSnapshot({ mode: 'full' })",
    )
    expect(full).toBe(blindData('blind.playwright-ai.yml'))
    expect(full).toBe(renderSnapshot(blind))
  })

  it("tests that a page and a locator under the cap return renderSnapshot's text, asking the engine for its marked ai tree", async () => {
    const runtime = snapshotting(PAGE)
    const { document: _document, ...section } = PAGE
    expect(
      await runtime.run(`return [
        await page.ariaSnapshot({ mode: 'full' }),
        await page.getByRole('main').ariaSnapshot({ mode: 'full', depth: 3, boxes: true }),
      ]`),
    ).toEqual([renderSnapshot(PAGE), renderSnapshot(section)])
    // Only the snapshot ops: a full-mode text never reads or writes the delta baseline.
    expect(
      runtime.calls.map(({ name, input }) => ({
        name,
        mode: input.mode,
        clickable: input.clickable,
        depth: input.depth,
        boxes: input.boxes,
      })),
    ).toEqual([
      {
        name: 'page.ariaSnapshot',
        mode: 'ai',
        clickable: true,
        depth: undefined,
        boxes: undefined,
      },
      {
        name: 'locator.ariaSnapshot',
        mode: 'ai',
        clickable: true,
        depth: 3,
        boxes: true,
      },
    ])
  })
})

describe('the cap', () => {
  /** The first `count` YAML lines' length: a limit that falls right after them. */
  const tokensFor = (count: number): number =>
    PAGE_LINES.slice(0, count).join('\n').length / 4

  it('tests that a cut inside an open section that fits offers that section in mode full, and the dialog past the cut still prints', () => {
    // Cut after `- button "Button 50"`, inside main, whose own lines fit the limit.
    const kept = 18
    expect(PAGE_LINES[kept - 1]).toBe('    - button "Button 50" [ref=e50]')
    expect(renderFullSnapshot(PAGE, tokensFor(kept)).split('\n')).toEqual([
      ...PAGE_LINES.slice(0, kept),
      "# cut: 10 more lines (10 with refs); the whole snapshot is about 300 tokens. page.getByRef('e3').ariaSnapshot({ mode: 'full' }) shows all of the main section. Or pass a larger maxTokens.",
      ...DIALOG,
    ])
  })

  it('tests that a cut inside a section larger than the cap offers the next section, which is not shown above', () => {
    // Cut inside the banner, whose own 13 lines are longer than the limit.
    const kept = 7
    expect(PAGE_LINES[kept - 1]).toBe('    - link "Link 12" [ref=e12]:')
    expect(renderFullSnapshot(PAGE, tokensFor(kept)).split('\n')).toEqual([
      ...PAGE_LINES.slice(0, kept),
      "# cut: 21 more lines (16 with refs); the whole snapshot is about 300 tokens. page.getByRef('e3').ariaSnapshot({ mode: 'full' }) shows the main section, which is not shown above. Or pass a larger maxTokens.",
      ...DIALOG,
    ])
  })

  it('tests that a cut with no section to offer asks for a larger maxTokens, and a named section is labelled with its name', () => {
    const flat: SnapshotResult = {
      json: [{ role: 'generic', ref: 'e1', children: buttons(10, 40) }],
      partial: [],
    }
    expect(renderFullSnapshot(flat, 100).split('\n').at(-1)).toBe(
      '# cut: 29 more lines (29 with refs); the whole snapshot is about 300 tokens. Pass a larger maxTokens.',
    )
    // Cut inside main, which is larger than the limit: the footer comes next, by its name.
    const footer: SnapshotResult = {
      json: [
        { role: 'main', ref: 'e3', children: buttons(50, 40) },
        {
          role: 'contentinfo',
          name: 'Footer',
          ref: 'e5',
          children: buttons(100, 2),
        },
      ],
      partial: [],
    }
    expect(renderFullSnapshot(footer, 100).split('\n').at(-1)).toBe(
      `# cut: 32 more lines (32 with refs); the whole snapshot is about 400 tokens. page.getByRef('e5').ariaSnapshot({ mode: 'full' }) shows the contentinfo "Footer" section, which is not shown above. Or pass a larger maxTokens.`,
    )
  })

  it('tests that a dialog the limit falls inside is moved past the cut whole, never split', () => {
    const kept = PAGE_LINES.length - 2
    expect(PAGE_LINES[kept - 1]).toBe('  - dialog "Cookies" [ref=e6]:')
    // Every line past the limit is the dialog's, which prints whole anyway: nothing is cut.
    expect(renderFullSnapshot(PAGE, tokensFor(kept))).toBe(renderSnapshot(PAGE))
  })

  it('tests that maxTokens through the prelude cuts a page and a locator, and a larger one returns the text whole', async () => {
    const runtime = snapshotting(PAGE)
    const { document: _document, ...section } = PAGE
    const [cutPage, cutSection, whole] = (await runtime.run(`return [
      await page.ariaSnapshot({ mode: 'full', maxTokens: 60 }),
      await page.getByRole('generic').ariaSnapshot({ mode: 'full', maxTokens: 60 }),
      await page.ariaSnapshot({ mode: 'full', maxTokens: 20000 }),
    ]`)) as string[]
    expect(cutPage).toBe(renderFullSnapshot(PAGE, 60))
    expect(cutPage).toContain('\n# cut: ')
    expect(cutSection).toBe(renderFullSnapshot(section, 60))
    expect(whole).toBe(renderSnapshot(PAGE))
  })

  it('tests that the default cap is 8,000 tokens', async () => {
    const big: SnapshotResult = {
      json: [{ role: 'generic', ref: 'e1', children: buttons(10, 2_000) }],
      partial: [],
      document: 1,
    }
    expect(renderSnapshot(big).length).toBeGreaterThan(8_000 * 4)
    expect(
      await snapshotting(big).run(
        "return await page.ariaSnapshot({ mode: 'full' })",
      ),
    ).toBe(renderFullSnapshot(big, 8_000))
  })

  it("tests that a maxTokens of 0 or 'x' is refused with the v1 format's message before any op is sent", async () => {
    const runtime = snapshotting(PAGE)
    expect(
      await thrown(
        runtime,
        "await page.ariaSnapshot({ mode: 'full', maxTokens: 0 })",
      ),
    ).toBe('page.ariaSnapshot: maxTokens: expected a positive number, got 0')
    expect(
      await thrown(
        runtime,
        "await page.getByRole('main').ariaSnapshot({ mode: 'full', maxTokens: 'x' })",
      ),
    ).toBe('locator.ariaSnapshot: maxTokens: expected a positive number, got x')
    expect(runtime.calls).toEqual([
      refusalCall(
        'page.ariaSnapshot',
        'invalid_input',
        'page.ariaSnapshot: maxTokens: expected a positive number, got 0',
      ),
      refusalCall(
        'locator.ariaSnapshot',
        'invalid_input',
        'locator.ariaSnapshot: maxTokens: expected a positive number, got x',
      ),
    ])
  })
})

describe('the options mode full does not read', () => {
  it('tests that text, urls and delta change nothing and are never refused', async () => {
    const runtime = snapshotting(PAGE)
    const [plain, passed, wrongType] = (await runtime.run(`return [
      await page.ariaSnapshot({ mode: 'full' }),
      await page.ariaSnapshot({ mode: 'full', text: false, urls: false, delta: true }),
      await page.ariaSnapshot({ mode: 'full', text: 'yes' }),
    ]`)) as string[]
    expect(passed).toBe(plain)
    expect(wrongType).toBe(plain)
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'page.ariaSnapshot',
      'page.ariaSnapshot',
      'page.ariaSnapshot',
    ])
  })
})

describe('the delta baseline', () => {
  it('tests that a full-mode snapshot between a v1 snapshot and a delta changes nothing: the delta finds no change', async () => {
    const stored = new Map<number, SnapshotBaseline>()
    const sent: string[] = []
    /** One run; its end stores the last pending text the run returned, as the host does. */
    const run = async (code: string): Promise<unknown> => {
      const pending: {
        tab: number
        baseline: SnapshotBaseline
        returned: string
      }[] = []
      const runtime = loadPrelude({
        seed: SEED,
        answer: (call: Call) => {
          sent.push(call.name)
          if (call.name === 'page.ariaSnapshot') return PAGE
          const input = call.input as {
            page: { tab: number }
            text: string
            form: SnapshotBaseline['form']
            returned?: string
          }
          if (call.name === 'page.snapshotBaseline')
            return stored.get(input.page.tab) ?? null
          if (call.name === 'page.snapshotPending')
            pending.push({
              tab: input.page.tab,
              baseline: { text: input.text, form: input.form },
              returned: input.returned ?? input.text,
            })
          return null
        },
      })
      const value = await runtime.run(code)
      const last = pending.at(-1)
      if (last && String(value).includes(last.returned))
        stored.set(last.tab, last.baseline)
      return value
    }
    await run('return await page.ariaSnapshot()')
    const before = new Map(stored)
    sent.length = 0
    const full = await run("return await page.ariaSnapshot({ mode: 'full' })")
    expect(full).toBe(renderSnapshot(PAGE))
    expect(sent.filter((name) => name.includes('napshot'))).toEqual([
      'page.ariaSnapshot',
    ])
    expect(stored).toEqual(before)
    expect(await run('return await page.ariaSnapshot({ delta: true })')).toBe(
      NO_CHANGE,
    )
  })
})

describe('the modes', () => {
  it("tests that under the conformance switch 'full' goes to the op as the script passed it, with no clickable key", async () => {
    const runtime = snapshotting(PAGE, true)
    await runtime.run(`
      await page.ariaSnapshot({ mode: 'full' })
      await page.getByRole('main').ariaSnapshot({ mode: 'full' })
    `)
    expect(
      runtime.calls.map(({ name, input }) => ({
        name,
        mode: input.mode,
        clickable: Object.hasOwn(input, 'clickable'),
      })),
    ).toEqual([
      { name: 'page.ariaSnapshot', mode: 'full', clickable: false },
      { name: 'locator.ariaSnapshot', mode: 'full', clickable: false },
    ])
  })

  it('tests that any other mode is refused before an op is sent, and ariaSnapshotJSON leaves full to the op', async () => {
    const runtime = snapshotting(PAGE)
    expect(
      await thrown(runtime, "await page.ariaSnapshot({ mode: 'x' })"),
    ).toBe('page.ariaSnapshot: mode: expected one of (ai|default|full)')
    expect(
      await thrown(
        runtime,
        "await page.getByRole('main').ariaSnapshot({ mode: 5 })",
      ),
    ).toBe('locator.ariaSnapshot: mode: expected one of (ai|default|full)')
    await runtime.run("await page.ariaSnapshotJSON({ mode: 'full' })")
    expect(runtime.calls).toEqual([
      refusalCall(
        'page.ariaSnapshot',
        'invalid_input',
        'page.ariaSnapshot: mode: expected one of (ai|default|full)',
      ),
      refusalCall(
        'locator.ariaSnapshot',
        'invalid_input',
        'locator.ariaSnapshot: mode: expected one of (ai|default|full)',
      ),
      expect.objectContaining({
        name: 'page.ariaSnapshot',
        input: expect.objectContaining({ mode: 'full' }),
      }),
    ])
    expect(runtime.calls[2].input).not.toHaveProperty('clickable')
  })
})
