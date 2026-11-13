import { describe, expect, it } from 'bun:test'
import { SNAPSHOT, SNAPSHOT_WITH_TIP, TIP_LINE } from '../test-support/replies'
import { runDetail, runSummary } from '../test-support/runs'
import {
  errorCode,
  formattedLines,
  replyParts,
  returnedWhat,
  valueChip,
} from './reply'
import { lineText } from './text-lines'

describe("reading a run's reply", () => {
  it("splits Neo's reply into the value, the console lines and the notes", () => {
    const text = [
      '{"name":"Grace Hopper"}',
      'console:',
      'Form',
      '[warn] slow',
      '[unknown outcome] locator.click at run:3:7 (op 7) was sent but not confirmed. Take a snapshot before repeating it.',
    ].join('\n')
    expect(replyParts(text)).toEqual({
      value: '{"name":"Grace Hopper"}',
      console: ['Form', '[warn] slow'],
      notes: [
        '[unknown outcome] locator.click at run:3:7 (op 7) was sent but not confirmed. Take a snapshot before repeating it.',
      ],
    })
  })

  it("reads run 8, a returned snapshot then Neo's tip, as the snapshot's lines with the tip under them", () => {
    const value = SNAPSHOT_WITH_TIP.split('\n')[0]
    expect(replyParts(SNAPSHOT_WITH_TIP)).toEqual({
      value,
      console: [],
      notes: [TIP_LINE],
    })
    const detail = runDetail(
      runSummary(8, { reply: SNAPSHOT_WITH_TIP }),
      SNAPSHOT_WITH_TIP,
    )
    const lines = formattedLines(detail)
    expect(lines.map(lineText)).toEqual([...SNAPSHOT.split('\n'), TIP_LINE])
    expect(lines.at(-1)?.[0]?.tone).toBe('note')
    expect(returnedWhat(detail)).toBe(
      `returned a ${SNAPSHOT.split('\n').length}-line string`,
    )
    expect(valueChip(SNAPSHOT_WITH_TIP).failed).toBe(false)
  })

  it('takes any line after the value for a note, so a line Neo adds later cannot break the value', () => {
    const text = `${JSON.stringify('- button "Go" [ref=e2]\n- link "Home" [ref=e3]')}\nNotice: a line Neo does not write yet`
    expect(replyParts(text).notes).toEqual([
      'Notice: a line Neo does not write yet',
    ])
    const detail = runDetail(runSummary(1, { reply: text }), text)
    expect(formattedLines(detail).map(lineText)).toEqual([
      '- button "Go" [ref=e2]',
      '- link "Home" [ref=e3]',
      'Notice: a line Neo does not write yet',
    ])
  })

  it('keeps the console lines and the notes apart when a tip follows console output', () => {
    const text = [
      JSON.stringify('- button "Go" [ref=e2]'),
      'console:',
      'clicked',
      '[warn] slow',
      '[not awaited] page.waitForTimeout at run:2:1 (op 4) was still running when the script returned; it was cancelled.',
      TIP_LINE,
    ].join('\n')
    const parts = replyParts(text)
    expect(parts.console).toEqual(['clicked', '[warn] slow'])
    expect(parts.notes).toHaveLength(2)
    expect(parts.notes.at(-1)).toBe(TIP_LINE)
    // A line Neo adds after its tip one day is a note too, and keeps the tip a note.
    expect(
      replyParts(`${text}\nNotice: a line Neo does not write yet`),
    ).toEqual({
      value: parts.value,
      console: ['clicked', '[warn] slow'],
      notes: [...parts.notes, 'Notice: a line Neo does not write yet'],
    })
    const detail = runDetail(runSummary(1, { reply: text }), text)
    expect(formattedLines(detail).map(lineText)[0]).toBe(
      '- button "Go" [ref=e2]',
    )
  })

  it("never cuts a returned string at a line of its own that reads like Neo's", () => {
    const snapshot =
      '- text: before\nTip: the page says this\n[not awaited] so does this\n- text: after'
    const text = `${JSON.stringify(snapshot)}\n${TIP_LINE}`
    expect(replyParts(text).value).toBe(JSON.stringify(snapshot))
    const detail = runDetail(runSummary(1, { reply: text }), text)
    expect(formattedLines(detail).map(lineText)).toEqual([
      ...snapshot.split('\n'),
      TIP_LINE,
    ])
  })

  it('keeps a multi-line error message whole', () => {
    const text =
      'website / strict_mode_violation: getByRole resolved to 2 elements:\n  1) <button>Place order</button>\n  2) <button>Confirm later</button>'
    expect(replyParts(text)).toEqual({ value: text, console: [], notes: [] })
    // Errors get no tips, so a line of the message that reads like one stays in it.
    const tipLike =
      'script / script_error: login failed\nTip: refresh the session'
    expect(replyParts(tipLike)).toEqual({
      value: tipLike,
      console: [],
      notes: [],
    })
    expect(errorCode(text)).toBe('strict_mode_violation')
    expect(errorCode('{"ok":true}')).toBeNull()
  })

  it('formats an object with its keys aligned and a snapshot under its key', () => {
    const text = JSON.stringify({
      url: 'http://127.0.0.1:8800/form.html',
      title: 'Form',
      snapshot: '- generic [ref=e1]:\n  - heading "Shipping" [ref=e2]',
    })
    const detail = runDetail(runSummary(2, { reply: text }), text)
    expect(formattedLines(detail).map(lineText)).toEqual([
      'url      "http://127.0.0.1:8800/form.html"',
      'title    "Form"',
      'snapshot',
      '- generic [ref=e1]:',
      '  - heading "Shipping" [ref=e2]',
    ])
    const refs = formattedLines(detail)[4]?.filter(
      (piece) => piece.tone === 'ref',
    )
    expect(refs?.map((piece) => piece.text)).toEqual(['[ref=e2]'])
    expect(valueChip(text)).toEqual({
      text: '{ url, title, snapshot: 2 lines }',
      failed: false,
    })
    expect(returnedWhat(detail)).toBe('returned 3 values')
  })

  it('formats a snapshot with more lines than a call takes arguments', () => {
    // Within Neo's 256 KiB reply limit.
    const text = JSON.stringify({ snapshot: '\n'.repeat(130_000) })
    const detail = runDetail(runSummary(1, { reply: text }), text)
    expect(formattedLines(detail)).toHaveLength(130_002)
  })

  it('shows a short value as it is, and a failure by its code', () => {
    expect(
      valueChip('{"name":"Grace Hopper","notes":"Leave at door"}'),
    ).toEqual({
      text: '{"name":"Grace Hopper","notes":"Leave at door"}',
      failed: false,
    })
    const failure = 'website / strict_mode_violation: 2 elements'
    expect(valueChip(failure)).toEqual({ text: failure, failed: true })
    const run = runSummary(4, { reply: failure, status: 'error' })
    expect(returnedWhat(runDetail(run))).toBe('failed: strict_mode_violation')
  })
})
