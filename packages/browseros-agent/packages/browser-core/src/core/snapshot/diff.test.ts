import { describe, expect, test } from 'bun:test'
import { diffSnapshotObservations, diffSnapshots } from './diff'

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
