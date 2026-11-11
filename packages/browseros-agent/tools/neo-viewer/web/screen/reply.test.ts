import { describe, expect, it } from 'bun:test'
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

  it('keeps a multi-line error message whole', () => {
    const text =
      'website / strict_mode_violation: getByRole resolved to 2 elements:\n  1) <button>Place order</button>\n  2) <button>Confirm later</button>'
    expect(replyParts(text)).toEqual({ value: text, console: [], notes: [] })
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
