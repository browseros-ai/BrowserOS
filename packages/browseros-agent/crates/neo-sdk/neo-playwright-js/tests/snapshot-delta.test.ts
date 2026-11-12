// `ariaSnapshot({ delta: true })` (src/prelude/snapshot-delta.ts): the diff of two v1 texts, and
// the baseline's life as scripts see it. The host's store here is a stand-in Map with the swap
// semantics of neo-playwright's `page.snapshotBaseline`; that op's own rules (a new document, a
// closed tab, the size bound) are tested in Rust and live in tests/prelude_live.rs.
import { describe, expect, it } from 'bun:test'
import {
  NO_BASELINE,
  NO_CHANGE,
  renderDelta,
  TOO_MUCH,
} from '../src/prelude/snapshot-delta'
import type { SnapshotResult } from '../src/prelude/wire'
import {
  type Call,
  loadPrelude,
  refusalCall,
  SEED,
} from './support/prelude-runtime'

const lines = (...text: string[]): string => text.join('\n')

/**
 * Thirty lines of a sidebar before the page proper, so that one change and its context are well
 * under half of the whole output, as on a real page.
 */
const SIDEBAR = [
  '- complementary [ref=e40]:',
  ...Array.from(
    { length: 30 },
    (_, n) => `  - link "Story ${n}" [ref=e${n + 41}]`,
  ),
]

/** A sidebar, a navigation, a main section with a form inside, and two footer links. */
const page = (cart = 'Add to cart', extra: string[] = []): string =>
  lines(
    ...SIDEBAR,
    '- navigation [ref=e2]:',
    '  - link "Home" [ref=e3]',
    '  - link "Deals" [ref=e4]',
    '  - link "Help" [ref=e5]',
    '- main [ref=e6]:',
    '  - heading "Cart" [level=2]',
    '  - text: One item',
    `  - button "${cart}" [ref=e7]`,
    '  - form [ref=e8]:',
    '    - textbox "Coupon" [ref=e9]',
    '    - button "Apply" [ref=e10]',
    '    - text: Codes are case sensitive',
    ...extra,
    '  - link "Checkout" [ref=e11]',
    '  - link "Keep shopping" [ref=e12]',
    '  - text: Free returns',
    '- link "Privacy" [ref=e13]',
    '- link "Terms" [ref=e14]',
  )

describe('the delta text', () => {
  it('tests the four first lines: no baseline, no change, too much differs, changes', () => {
    expect(renderDelta(null, page())).toBe(`${NO_BASELINE}\n${page()}`)
    expect(NO_BASELINE).toBe(
      '# full snapshot: no earlier snapshot of this page',
    )
    expect(renderDelta(page(), page())).toBe(NO_CHANGE)
    expect(NO_CHANGE).toBe('# no change since your last snapshot of this tab')
    expect(TOO_MUCH).toBe(
      '# full snapshot: too much differs from your last snapshot to list as changes',
    )
    const other = lines('- button "Sign in" [ref=e1]', '- link "Help" [ref=e2]')
    // Everything differs: the changes would be longer than the page itself.
    expect(renderDelta(page(), other)).toBe(`${TOO_MUCH}\n${other}`)
    expect(renderDelta(page(), page('Remove from cart')).split('\n')[0]).toBe(
      '# changes since your last snapshot of this tab: 1 added, 1 removed',
    )
  })

  it('tests that a changed line inside a nested block names that block and keeps 3 lines of context on each side', () => {
    expect(renderDelta(page(), page('Remove from cart'))).toBe(
      lines(
        '# changes since your last snapshot of this tab: 1 added, 1 removed',
        '@@ main [ref=e6]',
        ' - main [ref=e6]:',
        '   - heading "Cart" [level=2]',
        '   - text: One item',
        '-  - button "Add to cart" [ref=e7]',
        '+  - button "Remove from cart" [ref=e7]',
        '   - form [ref=e8]:',
        '     - textbox "Coupon" [ref=e9]',
        '     - button "Apply" [ref=e10]',
      ),
    )
  })

  it('tests the hunk of a change two levels deep: the form is its block', () => {
    const coupon = page('Add to cart', ['    - alert: Code expired'])
    expect(renderDelta(page(), coupon)).toBe(
      lines(
        '# changes since your last snapshot of this tab: 1 added, 0 removed',
        '@@ form [ref=e8]',
        '     - textbox "Coupon" [ref=e9]',
        '     - button "Apply" [ref=e10]',
        '     - text: Codes are case sensitive',
        '+    - alert: Code expired',
        '   - link "Checkout" [ref=e11]',
        '   - link "Keep shopping" [ref=e12]',
        '   - text: Free returns',
      ),
    )
  })

  it('tests that a change at the top level has a bare @@ line', () => {
    const before = page()
    const after = `${before}\n- link "Cookies" [ref=e15]`
    expect(renderDelta(before, after)).toBe(
      lines(
        '# changes since your last snapshot of this tab: 1 added, 0 removed',
        '@@',
        '   - text: Free returns',
        ' - link "Privacy" [ref=e13]',
        ' - link "Terms" [ref=e14]',
        '+- link "Cookies" [ref=e15]',
      ),
    )
  })

  it('tests that a hunk which only removes lines finds its block in the old text', () => {
    const before = page('Add to cart', ['    - alert: Code expired'])
    // The old text's form holds the removed line; the new text has no line at that place.
    expect(renderDelta(before, page())).toBe(
      lines(
        '# changes since your last snapshot of this tab: 0 added, 1 removed',
        '@@ form [ref=e8]',
        '     - textbox "Coupon" [ref=e9]',
        '     - button "Apply" [ref=e10]',
        '     - text: Codes are case sensitive',
        '-    - alert: Code expired',
        '   - link "Checkout" [ref=e11]',
        '   - link "Keep shopping" [ref=e12]',
        '   - text: Free returns',
      ),
    )
  })

  it('tests that one line added and one removed far apart make two hunks, each with its own block', () => {
    const before = page()
    const after = before
      .replace('  - link "Deals" [ref=e4]\n', '')
      .replace(
        '- link "Terms" [ref=e14]',
        '- link "Terms" [ref=e14]\n- link "Jobs" [ref=e16]',
      )
    expect(renderDelta(before, after)).toBe(
      lines(
        '# changes since your last snapshot of this tab: 1 added, 1 removed',
        '@@ navigation [ref=e2]',
        '   - link "Story 29" [ref=e70]',
        ' - navigation [ref=e2]:',
        '   - link "Home" [ref=e3]',
        '-  - link "Deals" [ref=e4]',
        '   - link "Help" [ref=e5]',
        ' - main [ref=e6]:',
        '   - heading "Cart" [level=2]',
        '@@',
        '   - text: Free returns',
        ' - link "Privacy" [ref=e13]',
        ' - link "Terms" [ref=e14]',
        '+- link "Jobs" [ref=e16]',
      ),
    )
  })

  it('tests that changes six unchanged lines apart share one hunk, and seven apart do not', () => {
    const base = Array.from(
      { length: 80 },
      (_, n) => `- link "L${n}" [ref=e${n}]`,
    )
    const at = (rows: number[]) =>
      base
        .map((line, n) => (rows.includes(n) ? `${line} [active]` : line))
        .join('\n')
    const near = renderDelta(base.join('\n'), at([40, 47]))
    expect(
      near.split('\n').filter((line) => line.startsWith('@@')),
    ).toHaveLength(1)
    const far = renderDelta(base.join('\n'), at([40, 48]))
    expect(
      far.split('\n').filter((line) => line.startsWith('@@')),
    ).toHaveLength(2)
    expect(far.split('\n').filter((line) => line.startsWith(' '))).toHaveLength(
      12,
    )
  })

  it('tests that a moved block shows as removed in its old place and added in its new one', () => {
    const before = page()
    const nav = lines(
      '- navigation [ref=e2]:',
      '  - link "Home" [ref=e3]',
      '  - link "Deals" [ref=e4]',
      '  - link "Help" [ref=e5]',
    )
    const after = `${before.replace(`${nav}\n`, '')}\n${nav}`
    const delta = renderDelta(before, after)
    expect(delta.split('\n')[0]).toBe(
      '# changes since your last snapshot of this tab: 4 added, 4 removed',
    )
    expect(delta).toContain(
      '-- navigation [ref=e2]:\n-  - link "Home" [ref=e3]',
    )
    expect(delta).toContain(
      '+- navigation [ref=e2]:\n+  - link "Home" [ref=e3]',
    )
  })

  it('tests that changes longer than half of the full output return the full output', () => {
    const before = page()
    // Every link renamed: the changes would be about twice the page.
    const after = before.replace(/link "([^"]+)"/g, 'link "$1 now"')
    expect(renderDelta(before, after)).toBe(`${TOO_MUCH}\n${after}`)
    // Every third sidebar link renamed: about half of the page's characters changed, so the
    // changes and their context pass half of the output.
    const third = before.replace(/"Story (\d*[0369])"/g, '"Story $1 now"')
    expect(renderDelta(before, third)).toBe(`${TOO_MUCH}\n${third}`)
    // One line changed is listed.
    expect(renderDelta(before, page('Buy')).startsWith('# changes')).toBe(true)
  })

  it('tests that the notice and cut lines are not compared and not repeated', () => {
    const notice = (tokens: number) =>
      `# text left out: about ${tokens} tokens. Pass { text: true } to include it.`
    const cut = (more: number) =>
      `# cut: ${more} more lines (3 with refs); the whole snapshot is about 9000 tokens. Pass a larger maxTokens.`
    expect(
      renderDelta(
        `${notice(2900)}\n${page()}\n${cut(40)}`,
        `${notice(3100)}\n${page()}\n${cut(41)}`,
      ),
    ).toBe(NO_CHANGE)
    expect(renderDelta(page(), `${notice(2900)}\n${page()}`)).toBe(NO_CHANGE)
    const delta = renderDelta(
      `${notice(2900)}\n${page()}\n${cut(40)}`,
      `${notice(3100)}\n${page('Buy')}\n${cut(41)}`,
    )
    expect(delta.startsWith('# changes since')).toBe(true)
    expect(delta).not.toContain('# text left out')
    expect(delta).not.toContain('# cut:')
    // A full output is the text as the agent receives it, notice included.
    expect(renderDelta(null, `${notice(2900)}\n${page()}`)).toBe(
      `${NO_BASELINE}\n${notice(2900)}\n${page()}`,
    )
  })

  it('tests on random pages that the listed changes are a shortest edit script', () => {
    // A seeded generator, so a failure repeats.
    let seed = 7
    const next = (bound: number) => {
      // mulberry32, in 32-bit integer arithmetic.
      seed = (seed + 0x6d2b79f5) | 0
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
      return ((t ^ (t >>> 14)) >>> 0) % bound
    }
    const lcs = (a: string[], b: string[]): number => {
      const row = new Array<number>(b.length + 1).fill(0)
      for (const x of a) {
        let diagonal = 0
        for (let j = 1; j <= b.length; j++) {
          const above = row[j]
          row[j] = x === b[j - 1] ? diagonal + 1 : Math.max(row[j], row[j - 1])
          diagonal = above
        }
      }
      return row[b.length]
    }
    const subsequence = (part: string[], whole: string[]) => {
      let at = 0
      for (const line of whole) if (line === part[at]) at += 1
      return at === part.length
    }
    let listed = 0
    for (let round = 0; round < 300; round++) {
      // Long lines from a small alphabet: repeats, so the diff has real choices to make.
      const make = () =>
        Array.from(
          { length: 100 + next(200) },
          () => `- link "${'x'.repeat(next(6) * 40)}${next(5)}"`,
        )
      const before = make()
      // A few lines removed and a few added, some next to each other.
      const after = before.flatMap((line) =>
        next(40) === 0 ? [] : next(40) === 0 ? [line, make()[0]] : [line],
      )
      const delta = renderDelta(before.join('\n'), after.join('\n'))
      const header =
        /^# changes since your last snapshot of this tab: (\d+) added, (\d+) removed$/.exec(
          delta.split('\n')[0],
        )
      if (header === null) continue
      listed += 1
      const added = Number(header[1])
      const removed = Number(header[2])
      expect(added + removed).toBe(
        before.length + after.length - 2 * lcs(before, after),
      )
      const body = delta.split('\n').slice(1)
      const plus = body
        .filter((line) => line.startsWith('+'))
        .map((line) => line.slice(1))
      const minus = body
        .filter((line) => line.startsWith('-'))
        .map((line) => line.slice(1))
      expect([plus.length, minus.length]).toEqual([added, removed])
      expect(subsequence(plus, after)).toBe(true)
      expect(subsequence(minus, before)).toBe(true)
    }
    expect(listed).toBeGreaterThan(100)
  })

  it('tests that a large page with one change diffs quickly and lists only that change', () => {
    const big = Array.from(
      { length: 4000 },
      (_, n) => `  - link "Story ${n}" [ref=e${n + 10}]`,
    )
    const before = ['- main [ref=e2]:', ...big].join('\n')
    const after = before.replace(
      '"Story 2000" [ref=e2010]',
      '"Story 2000" [ref=e2010] [expanded]',
    )
    const started = Date.now()
    const delta = renderDelta(before, after)
    expect(Date.now() - started).toBeLessThan(1000)
    expect(delta.split('\n')).toHaveLength(10)
    expect(delta.split('\n')[1]).toBe('@@ main [ref=e2]')
  })

  it('tests that a page where everything changed stops early and comes back whole', () => {
    const before = Array.from(
      { length: 3000 },
      (_, n) => `- link "A${n}" [ref=e${n}]`,
    ).join('\n')
    const after = Array.from(
      { length: 3000 },
      (_, n) => `- link "B${n}" [ref=f${n}]`,
    ).join('\n')
    const started = Date.now()
    expect(renderDelta(before, after)).toBe(`${TOO_MUCH}\n${after}`)
    expect(Date.now() - started).toBeLessThan(2000)
  })
})

/**
 * A run on its own realm whose host keeps baselines in `store`, by tab, as Rust's op does: it
 * answers the previous text and keeps the new one.
 */
function host(store: Map<number, string>, result: () => SnapshotResult) {
  return loadPrelude({
    seed: SEED,
    answer: (call: Call) => {
      if (call.name.endsWith('.ariaSnapshot')) return result()
      if (call.name === 'page.snapshotBaseline') {
        const input = call.input as { page: { tab: number }; text: string }
        const previous = store.get(input.page.tab) ?? null
        store.set(input.page.tab, input.text)
        return previous
      }
      return null
    },
  })
}

/** A main section with one button that changes, then twenty links that never do. */
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
})
const stories = Array.from(
  { length: 20 },
  (_, n) => `  - link "Story ${n}" [ref=e${n + 5}]`,
)

describe('the baseline as scripts see it', () => {
  it('tests the first snapshot, an unchanged one, and a change, across fresh runs', async () => {
    const store = new Map<number, string>()
    let label = 'Open'
    const run = (code: string) => host(store, () => tree(label)).run(code)
    const full = lines(
      '- main [ref=e2]:',
      '  - button "Open" [ref=e3]',
      '  - link "Help" [ref=e4]',
      ...stories,
    )
    expect(await run('return await page.ariaSnapshot({ delta: true })')).toBe(
      `${NO_BASELINE}\n${full}`,
    )
    // A new run is a fresh realm: the baseline is the host's.
    expect(await run('return await page.ariaSnapshot({ delta: true })')).toBe(
      NO_CHANGE,
    )
    label = 'Close'
    expect(await run('return await page.ariaSnapshot({ delta: true })')).toBe(
      lines(
        '# changes since your last snapshot of this tab: 1 added, 1 removed',
        '@@ main [ref=e2]',
        ' - main [ref=e2]:',
        '-  - button "Open" [ref=e3]',
        '+  - button "Close" [ref=e3]',
        '   - link "Help" [ref=e4]',
        '   - link "Story 0" [ref=e5]',
        '   - link "Story 1" [ref=e6]',
      ),
    )
  })

  it('tests that a full snapshot also moves the baseline, and a locator snapshot does not', async () => {
    const store = new Map<number, string>()
    let label = 'Open'
    const runtime = host(store, () => tree(label))
    await runtime.run('await page.ariaSnapshot()')
    label = 'Close'
    // Only one section: the page's baseline stays the page's last whole text.
    await runtime.run(
      "await page.getByRole('main').ariaSnapshot({ delta: true })",
    )
    expect(store.get(1007)).toContain('"Open"')
    expect(
      await runtime.run('return await page.ariaSnapshot({ delta: true })'),
    ).toContain('+  - button "Close" [ref=e3]')
    // The full snapshot without delta replaced it, so this delta is against "Close".
    await runtime.run('await page.ariaSnapshot()')
    expect(
      await runtime.run('return await page.ariaSnapshot({ delta: true })'),
    ).toBe(NO_CHANGE)
  })

  it('tests that a locator snapshot with delta returns its ordinary text and sends no baseline op', async () => {
    const store = new Map<number, string>()
    const runtime = host(store, () => tree('Open'))
    expect(
      await runtime.run(
        "return await page.getByRole('main').ariaSnapshot({ delta: true })",
      ),
    ).toBe(
      lines(
        '- main [ref=e2]:',
        '  - button "Open" [ref=e3]',
        '  - link "Help" [ref=e4]',
        ...stories,
      ),
    )
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'locator.ariaSnapshot',
    ])
  })

  it("tests that mode 'default' ignores delta and leaves the baseline alone", async () => {
    const store = new Map<number, string>()
    const runtime = host(store, () => tree('Open'))
    const yaml = await runtime.run(
      "return await page.ariaSnapshot({ mode: 'default', delta: true })",
    )
    expect(yaml).not.toContain('# ')
    expect(runtime.calls.map((call) => call.name)).toEqual([
      'page.ariaSnapshot',
    ])
    expect(store.size).toBe(0)
  })

  it('tests that under the conformance switch delta is ignored and no baseline op is sent', async () => {
    const store = new Map<number, string>()
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
    expect(store.size).toBe(0)
  })

  it('tests that the op gets the text the agent receives, notice and cap included', async () => {
    const store = new Map<number, string>()
    const runtime = host(store, () => tree('Open'))
    const text = String(
      await runtime.run('return await page.ariaSnapshot({ maxTokens: 5 })'),
    )
    expect(text).toContain('# cut:')
    expect(store.get(1007)).toBe(text)
    const sent = runtime.calls.find(
      (call) => call.name === 'page.snapshotBaseline',
    )
    expect(sent?.input).toEqual({
      call: { api: 'page.ariaSnapshot', timeoutMs: 0 },
      page: { $neo: 'page', tab: 1007 },
      text,
      delta: false,
    })
  })

  it('tests that delta of the wrong type is refused before any op is sent', async () => {
    const runtime = host(new Map(), () => tree('Open'))
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
