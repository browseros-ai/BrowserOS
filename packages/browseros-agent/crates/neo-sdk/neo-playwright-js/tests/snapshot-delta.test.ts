// `ariaSnapshot({ delta: true })`'s text (src/prelude/snapshot-delta.ts): the diff of two v1
// texts rendered the same way (`listChanges`). How a delta renders the new snapshot from its
// baseline's form is snapshot-delta-form.test.ts; which text it compares against, as scripts see
// it, is snapshot-baseline.test.ts.
import { describe, expect, it } from 'bun:test'
import {
  listChanges,
  NO_BASELINE,
  NO_CHANGE,
  TOO_MUCH,
} from '../src/prelude/snapshot-delta'

const lines = (...text: string[]): string => text.join('\n')

/** The changes from `previous` to `current`, which must be few enough to list. */
const changed = (previous: string, current: string): string => {
  const listed = listChanges(previous, current)
  if (listed === null) throw new Error('too much to list')
  return listed
}

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
    expect(NO_BASELINE).toBe('# whole page: no earlier snapshot of this page')
    expect(listChanges(page(), page())).toBe(NO_CHANGE)
    expect(NO_CHANGE).toBe('# no change since your last snapshot of this tab')
    expect(TOO_MUCH).toBe(
      '# whole page: too much differs from your last snapshot to list as changes',
    )
    const other = lines('- button "Sign in" [ref=e1]', '- link "Help" [ref=e2]')
    // Everything differs: the changes would be longer than the page itself.
    expect(listChanges(page(), other)).toBeNull()
    expect(changed(page(), page('Remove from cart')).split('\n')[0]).toBe(
      '# changes since your last snapshot of this tab: 1 added, 1 removed',
    )
  })

  it('tests that a changed line inside a nested block names that block and keeps 3 lines of context on each side', () => {
    expect(listChanges(page(), page('Remove from cart'))).toBe(
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
    expect(listChanges(page(), coupon)).toBe(
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
    expect(listChanges(before, after)).toBe(
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
    expect(listChanges(before, page())).toBe(
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
    expect(listChanges(before, after)).toBe(
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
    const near = changed(base.join('\n'), at([40, 47]))
    expect(
      near.split('\n').filter((line) => line.startsWith('@@')),
    ).toHaveLength(1)
    const far = changed(base.join('\n'), at([40, 48]))
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
    const delta = changed(before, after)
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
    expect(listChanges(before, after)).toBeNull()
    // Every third sidebar link renamed: about half of the page's characters changed, so the
    // changes and their context pass half of the output.
    const third = before.replace(/"Story (\d*[0369])"/g, '"Story $1 now"')
    expect(listChanges(before, third)).toBeNull()
    // One line changed is listed.
    expect(listChanges(before, page('Buy'))?.startsWith('# changes')).toBe(true)
  })

  it('tests that the notice and cut lines are not compared and not repeated', () => {
    const notice = (tokens: number) =>
      `# text left out: about ${tokens} tokens. Pass { text: true } to include it.`
    const cut = (more: number) =>
      `# cut: ${more} more lines (3 with refs); the whole snapshot is about 9000 tokens. Pass a larger maxTokens.`
    expect(
      listChanges(
        `${notice(2900)}\n${page()}\n${cut(40)}`,
        `${notice(3100)}\n${page()}\n${cut(41)}`,
      ),
    ).toBe(NO_CHANGE)
    expect(listChanges(page(), `${notice(2900)}\n${page()}`)).toBe(NO_CHANGE)
    const delta = String(
      listChanges(
        `${notice(2900)}\n${page()}\n${cut(40)}`,
        `${notice(3100)}\n${page('Buy')}\n${cut(41)}`,
      ),
    )
    expect(delta.startsWith('# changes since')).toBe(true)
    expect(delta).not.toContain('# text left out')
    expect(delta).not.toContain('# cut:')
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
      // Too much to list, or no change: nothing to check this round.
      const delta = listChanges(before.join('\n'), after.join('\n')) ?? ''
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
    const delta = changed(before, after)
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
    expect(listChanges(before, after)).toBeNull()
    expect(Date.now() - started).toBeLessThan(2000)
  })
})
