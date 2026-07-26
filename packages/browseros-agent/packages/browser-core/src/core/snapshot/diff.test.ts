import { describe, expect, test } from 'bun:test'
import { diffSnapshotObservations, diffSnapshots } from './diff'

function withoutLcsTableAllocation<T>(run: () => T): T {
  const arrayFrom = Array.from
  Array.from = (() => {
    throw new Error('unexpected LCS table allocation')
  }) as typeof Array.from
  try {
    return run()
  } finally {
    Array.from = arrayFrom
  }
}

describe('diffSnapshots', () => {
  test('identical snapshots short-circuit to no change', () => {
    const snap = '- button "Go" [ref=e1]'
    expect(diffSnapshots(snap, snap)).toEqual({
      text: '',
      added: 0,
      removed: 0,
      changed: false,
    })
  })

  test('a state change shows a removed/added pair on the same ref', () => {
    const before = '- button "Save" [ref=e1]'
    const after = '- button "Save" [ref=e1] [disabled]'
    const d = diffSnapshots(before, after)

    expect(d.changed).toBe(true)
    expect(d.added).toBe(1)
    expect(d.removed).toBe(1)
    expect(d.text).toContain('- button "Save" [ref=e1]')
    expect(d.text).toContain('+ button "Save" [ref=e1] [disabled]')
    expect(d.text).toContain('1 added, 1 removed')
  })

  test('preserves output when a change has a common prefix and suffix', () => {
    const before = [
      '- main',
      '  - heading Old',
      '  - button Save [ref=e1]',
      '  - content',
      '- footer',
    ].join('\n')
    const after = [
      '- main',
      '  - heading New',
      '  - button Save [ref=e1]',
      '  - content',
      '- footer',
    ].join('\n')

    expect(diffSnapshots(before, after, { contextRadius: 1 })).toEqual({
      text: [
        '  main',
        '-   heading Old',
        '+   heading New',
        '    button Save [ref=e1]',
        '1 added, 1 removed',
      ].join('\n'),
      added: 1,
      removed: 1,
      changed: true,
    })
  })

  test('uses the trimmed middle when large snapshots share boundaries', () => {
    const prefix = Array.from({ length: 1_000 }, (_, i) => `- prefix ${i}`)
    const suffix = Array.from({ length: 1_000 }, (_, i) => `- suffix ${i}`)
    const before = [...prefix, '- status old', ...suffix].join('\n')
    const after = [...prefix, '- status new', ...suffix].join('\n')

    const d = diffSnapshots(before, after)

    expect(d.added).toBe(1)
    expect(d.removed).toBe(1)
    expect(d.text).toContain('- status old')
    expect(d.text).toContain('+ status new')
    expect(d.text).not.toContain('changed substantially')
  })

  test('bails out before allocating an over-budget LCS table', () => {
    const before = [
      '- shared start',
      ...Array.from({ length: 2_000 }, (_, i) => `- before ${i}`),
      '- shared end',
    ].join('\n')
    const after = [
      '- shared start',
      ...Array.from({ length: 2_000 }, (_, i) => `- after ${i}`),
      '- shared end',
    ].join('\n')

    const d = withoutLcsTableAllocation(() => diffSnapshots(before, after))

    expect(d).toEqual({
      text: [
        'Snapshot changed substantially: 2002 lines before, 2002 lines after.',
        'Line-level diff skipped because the changed region exceeds the 4000000-cell comparison limit.',
      ].join('\n'),
      added: 2_000,
      removed: 2_000,
      changed: true,
      lineDiffSkipped: true,
    })
    expect(d.text.length).toBeLessThan(250)
  })

  test('runs LCS when the padded table is exactly at the cell budget', () => {
    const before = Array.from(
      { length: 1_999 },
      (_, i) => `- before boundary ${i}`,
    ).join('\n')
    const after = Array.from(
      { length: 1_999 },
      (_, i) => `- after boundary ${i}`,
    ).join('\n')

    const d = diffSnapshots(before, after)

    expect(d.added).toBe(1_999)
    expect(d.removed).toBe(1_999)
    expect(d.text).not.toContain('changed substantially')
  })

  test('includes padded cells when budgeting asymmetric LCS tables', () => {
    const before = Array.from(
      { length: 1_000 },
      (_, i) => `- asymmetric before ${i}`,
    ).join('\n')
    const after = Array.from(
      { length: 3_997 },
      (_, i) => `- asymmetric after ${i}`,
    ).join('\n')

    const d = withoutLcsTableAllocation(() => diffSnapshots(before, after))

    expect(d.added).toBe(3_997)
    expect(d.removed).toBe(1_000)
    expect(d.lineDiffSkipped).toBe(true)
    expect(d.text).toContain('changed substantially')
  })

  test('handles empty and one-sided snapshots without an LCS table', () => {
    expect(diffSnapshots('', '')).toEqual({
      text: '',
      added: 0,
      removed: 0,
      changed: false,
    })

    const added = withoutLcsTableAllocation(() =>
      diffSnapshots('', '- main\n  - button Save'),
    )
    expect(added).toEqual({
      text: '+ main\n+   button Save\n2 added, 0 removed',
      added: 2,
      removed: 0,
      changed: true,
    })

    const removed = withoutLcsTableAllocation(() =>
      diffSnapshots('- main\n  - button Save', ''),
    )
    expect(removed).toEqual({
      text: '- main\n-   button Save\n0 added, 2 removed',
      added: 0,
      removed: 2,
      changed: true,
    })
  })

  test('pure additions count only as added and strip the list bullet', () => {
    const before = '- main\n  - link "Home" [ref=e1]'
    const after = '- main\n  - link "Home" [ref=e1]\n  - link "About" [ref=e2]'
    const d = diffSnapshots(before, after)

    expect(d.added).toBe(1)
    expect(d.removed).toBe(0)
    expect(d.text).toContain('+   link "About" [ref=e2]')
  })

  test('stable refs turn top insertions into one added line', () => {
    const before = '- button "A" [ref=e1]\n- link "B" [ref=e2]'
    const after = [
      '- button "X" [ref=e3]',
      '- button "A" [ref=e1]',
      '- link "B" [ref=e2]',
    ].join('\n')

    const d = diffSnapshots(before, after)

    expect(d.added).toBe(1)
    expect(d.removed).toBe(0)
    expect(d.text).toContain('+ button "X" [ref=e3]')
    expect(d.text).toContain('1 added, 0 removed')
  })

  test('collapses far-apart context with an ellipsis', () => {
    const before = Array.from({ length: 30 }, (_, i) => `- item ${i}`).join(
      '\n',
    )
    const after = before
      .replace('- item 0', '- item ZERO')
      .replace('- item 29', '- item LAST')
    const d = diffSnapshots(before, after, { contextRadius: 2 })

    expect(d.text).toContain('…')
    expect(d.text).toContain('- item 0')
    expect(d.text).toContain('+ item ZERO')
    expect(d.text).toContain('+ item LAST')
    expect(d.text).not.toContain('item 15')
  })

  test('url changes return the full current snapshot instead of a line diff', () => {
    const before = {
      text: '- main\n  - button "Old page" [ref=e1]',
      url: 'https://example.com/old',
    }
    const after = {
      text: '- main\n  - heading "New page"',
      url: 'https://example.com/new',
    }

    const d = diffSnapshotObservations(before, after)

    expect(d).toMatchObject({
      text: after.text,
      added: 0,
      removed: 0,
      changed: true,
      urlChanged: true,
      beforeUrl: before.url,
      afterUrl: after.url,
    })
  })

  test('unknown urls keep existing line-diff behavior', () => {
    const d = diffSnapshotObservations(
      { text: '- main\n  - button "Old"', url: 'unknown' },
      { text: '- main\n  - button "New"', url: 'https://example.com/new' },
    )

    expect(d.changed).toBe(true)
    expect(d.urlChanged).toBeUndefined()
    expect(d.added).toBe(1)
    expect(d.removed).toBe(1)
    expect(d.text).toContain('-   button "Old"')
    expect(d.text).toContain('+   button "New"')
  })

  test('same-url diffs preserve the current url for callers', () => {
    const d = diffSnapshotObservations(
      {
        text: '- main\n  - button "Save" [ref=e1]',
        url: 'https://example.com/form',
      },
      {
        text: '- main\n  - button "Saved" [ref=e1]',
        url: 'https://example.com/form',
      },
    )

    expect(d.changed).toBe(true)
    expect(d.urlChanged).toBeUndefined()
    expect(d.afterUrl).toBe('https://example.com/form')
  })
})
