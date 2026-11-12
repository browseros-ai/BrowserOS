import { describe, expect, it } from 'bun:test'
import { SNAPSHOT, SNAPSHOT_WITH_TIP, TIP_LINE } from '../test-support/replies'
import { runDetail, runSummary } from '../test-support/runs'
import {
  errorCode,
  formattedLines,
  rawLines,
  replyParts,
  returnedWhat,
  valueChip,
} from './reply'
import { lineText } from './text-lines'

const NOTE =
  '[unknown outcome] locator.click at run:3:7 (op 7) was sent but not confirmed. Take a snapshot before repeating it.'
const NOT_AWAITED =
  '[not awaited] page.goto at run:4:1 (op 9) was still running when the script returned; it was cancelled.'
const HEADERS = [
  '### Result',
  '### Error',
  '### Console',
  '### Notes',
  '### Tips',
]

describe("reading a run's reply", () => {
  it.each(['{"a":1}', 'null', '7', 'false', '[]', '""'])(
    'reads a Result section containing %s',
    (value) => {
      expect(replyParts(`### Result\n${value}`)).toEqual({
        value,
        console: [],
        notes: [],
        tips: [],
      })
    },
  )

  it('splits the result, console, notes and tips without guessing from their content', () => {
    const text = [
      '### Result',
      '{"ok":true}',
      '### Console',
      'hello',
      '[warn] slow',
      '[3 more console lines were dropped: a run keeps 100 lines and 32 KiB]',
      'Tip: logged by the script',
      '[not awaited] also logged by the script',
      '### Notes',
      NOTE,
      NOT_AWAITED,
      '### Tips',
      TIP_LINE,
    ].join('\n')
    const console = [
      'hello',
      '[warn] slow',
      '[3 more console lines were dropped: a run keeps 100 lines and 32 KiB]',
      'Tip: logged by the script',
      '[not awaited] also logged by the script',
    ]
    expect(replyParts(text)).toEqual({
      value: '{"ok":true}',
      console,
      notes: [NOTE, NOT_AWAITED],
      tips: [TIP_LINE],
    })
    const detail = runDetail(runSummary(1, { reply: text }), text)
    expect(formattedLines(detail).map(lineText)).toEqual([
      'ok true',
      'console',
      ...console,
      NOTE,
      NOT_AWAITED,
      TIP_LINE,
    ])
    expect(rawLines(detail).map(lineText).join('\n')).toBe(text)
  })

  it('shows a returned snapshot decoded, with its tip below it', () => {
    expect(replyParts(SNAPSHOT_WITH_TIP)).toEqual({
      value: JSON.stringify(SNAPSHOT),
      console: [],
      notes: [],
      tips: [TIP_LINE],
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

  it('never splits a JSON string at a section header inside its value', () => {
    const snapshot = ['- text: before', ...HEADERS, '- text: after'].join('\n')
    const text = `### Result\n${JSON.stringify(snapshot)}\n### Tips\n${TIP_LINE}`
    expect(replyParts(text).value).toBe(JSON.stringify(snapshot))
    expect(
      formattedLines(runDetail(runSummary(1, { reply: text }), text)).map(
        lineText,
      ),
    ).toEqual([...snapshot.split('\n'), TIP_LINE])
  })

  it('keeps a multi-line error whole, followed by console output and notes', () => {
    const error = [
      'website / strict_mode_violation: getByRole resolved to 2 elements:',
      '  1) <button>Place order</button>',
      '  2) <button>Confirm later</button>',
      '',
      'console:',
      'Tip: refresh the session',
      '[not awaited] this belongs to the error',
    ].join('\n')
    const text = `### Error\n${error}\n### Console\nclicked\n### Notes\n${NOTE}`
    expect(replyParts(text)).toEqual({
      value: error,
      console: ['clicked'],
      notes: [NOTE],
      tips: [],
    })
    const detail = runDetail(
      runSummary(4, { reply: text, status: 'error' }),
      text,
    )
    expect(formattedLines(detail).map(lineText)).toEqual([
      ...error.split('\n'),
      'console',
      'clicked',
      NOTE,
    ])
    expect(errorCode(text)).toBe('strict_mode_violation')
    expect(returnedWhat(detail)).toBe('failed: strict_mode_violation')
  })

  it.each(HEADERS)('unescapes %s inside error and console text', (header) => {
    const error = 'script / script_error: failed (op 9, delivery unknown)'
    const text = `### Error\n${error}\n\\${header}\n### Console\n\\${header}\n### Notes\n${NOTE}`
    expect(replyParts(text)).toEqual({
      value: `${error}\n${header}`,
      console: [header],
      notes: [NOTE],
      tips: [],
    })
    const detail = runDetail(
      runSummary(1, { reply: text, status: 'error' }),
      text,
    )
    expect(formattedLines(detail).map(lineText)).toEqual([
      error,
      header,
      'console',
      header,
      NOTE,
    ])
    expect(rawLines(detail).map(lineText).join('\n')).toBe(text)
  })

  it('requires exact headers and only removes the header escape', () => {
    const console = [
      '',
      ' ### Notes',
      '### Tips ',
      '### Result: more',
      '#### Error',
      '### Unknown',
      '\\ordinary',
      '\\### Notes ',
      '\\\\### Notes',
      '',
    ]
    expect(
      replyParts(`### Result\nnull\n### Console\n${console.join('\n')}`),
    ).toEqual({
      value: 'null',
      console,
      notes: [],
      tips: [],
    })
  })

  it.each([
    '',
    '7',
    '7\nconsole:\nhello\n[not awaited] page.goto\nTip: snapshot',
    'script / script_error: failed\nconsole:\nhello\n[unknown outcome] click',
    '\\### Result\n### Unknown\n ### Error\n### Tips ',
  ])('leaves a reply with no Neo headers whole: %j', (text) => {
    expect(replyParts(text)).toEqual({
      value: text,
      console: [],
      notes: [],
      tips: [],
    })
  })

  it('shows old multi-line replies whole, without labelling console or notes', () => {
    const text = '7\nconsole:\nhello\n[not awaited] page.goto\nTip: snapshot'
    expect(
      formattedLines(runDetail(runSummary(1, { reply: text }), text))
        .map(lineText)
        .join('\n'),
    ).toBe(text)
  })

  it('formats an object with its keys aligned and a snapshot under its key', () => {
    const text = `### Result\n${JSON.stringify({
      url: 'http://127.0.0.1:8800/form.html',
      title: 'Form',
      snapshot: '- generic [ref=e1]:\n  - heading "Shipping" [ref=e2]',
    })}`
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
    const text = `### Result\n${JSON.stringify({ snapshot: '\n'.repeat(130_000) })}`
    expect(
      formattedLines(runDetail(runSummary(1, { reply: text }), text)),
    ).toHaveLength(130_002)
  })

  it('shows a short value as it is, and a failure by its code', () => {
    expect(
      valueChip('### Result\n{"name":"Grace Hopper","notes":"Leave at door"}'),
    ).toEqual({
      text: '{"name":"Grace Hopper","notes":"Leave at door"}',
      failed: false,
    })
    const failure = 'website / strict_mode_violation: 2 elements'
    const text = `### Error\n${failure}`
    expect(valueChip(text)).toEqual({ text: failure, failed: true })
    expect(
      returnedWhat(
        runDetail(runSummary(4, { reply: text, status: 'error' }), text),
      ),
    ).toBe('failed: strict_mode_violation')
    expect(errorCode('### Result\n{"ok":true}')).toBeNull()
    expect(errorCode(null)).toBeNull()
  })
})
