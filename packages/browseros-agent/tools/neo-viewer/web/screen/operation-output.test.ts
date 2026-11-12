import { describe, expect, it } from 'bun:test'
import type { NeoOperation } from '../../shared/api'
import { operation, record } from '../test-support/operations'
import { operationOutput } from './operation-output'
import { argumentLines } from './operation-values'
import { callPlaces, ordinal, placeOperations, placeText } from './operations'
import { lineText } from './text-lines'

const shown = (op: NeoOperation, all: NeoOperation[] = [op]) => {
  const output = operationOutput(op, all)
  return { ...output, lines: output.formatted.map(lineText) }
}

describe('what a call gave the script', () => {
  it("shows a whole-page snapshot's text from the page.snapshotPending that follows it, each loop pass its own", () => {
    const tree = { document: 4, partial: false, json: [{ role: 'heading' }] }
    const all = [
      operation(3, 'page.ariaSnapshot', { line: 2, result: tree }),
      operation(4, 'page.snapshotPending', {
        line: 2,
        args: {
          document: 4,
          text: '- heading "One" [level=1]',
          form: 'text',
          delta: false,
        },
      }),
      operation(5, 'page.ariaSnapshot', { line: 2, result: tree }),
      operation(6, 'page.snapshotPending', {
        line: 2,
        args: {
          document: 4,
          text: '- heading "Two" [level=1]\n- button "Go" [ref=e2]',
          form: 'text',
          delta: true,
          returned:
            '# changes since your last snapshot of this tab: 1 added, 0 removed\n@@\n+- button "Go" [ref=e2]',
        },
      }),
    ]
    const first = shown(all[0] as NeoOperation, all)
    expect(first.lines).toEqual(['- heading "One" [level=1]'])
    expect(first.source).toContain("The snapshot text the script's await got")
    expect(first.rawText).toContain('"document": 4')
    // A delta call got its delta, not the whole text it stored.
    expect(shown(all[2] as NeoOperation, all).lines).toEqual([
      '# changes since your last snapshot of this tab: 1 added, 0 removed',
      '@@',
      '+- button "Go" [ref=e2]',
    ])
  })

  it("shows a locator's snapshot, or mode 'default', as the recorded tree and says the text was not recorded", () => {
    const tree = { document: 4, json: [{ role: 'region' }] }
    const locator = shown(
      operation(3, 'locator.ariaSnapshot', { line: 1, result: tree }),
    )
    expect(locator.formattedText).toBe(JSON.stringify(tree, null, 2))
    expect(locator.source).toContain('that text is not recorded')
    // A default-mode snapshot does not borrow the next snapshot's text from the same line.
    const all = [
      operation(3, 'page.ariaSnapshot', { line: 1, result: tree }),
      operation(5, 'page.ariaSnapshot', { line: 1, result: tree }),
      operation(6, 'page.snapshotPending', {
        line: 1,
        args: { document: 4, text: 'later', form: 'text', delta: false },
      }),
    ]
    expect(shown(all[0] as NeoOperation, all).source).toContain(
      'that text is not recorded',
    )
    expect(shown(all[1] as NeoOperation, all).lines).toEqual(['later'])
  })

  it("decodes evaluate's serialized value as the run does, not the { value, frames } envelope", () => {
    const evaluated = operation(7, 'page.evaluate', {
      line: 4,
      result: {
        frames: [{ id: 'f1', url: 'https://example.com/' }],
        value: {
          id: 1,
          o: [
            { k: 'title', v: 'Example Domain' },
            {
              k: 'links',
              v: { id: 2, a: [1, { v: 'NaN' }, { v: 'undefined' }] },
            },
            { k: 'when', v: { d: '2026-11-12T04:00:00.000Z' } },
            { k: 'self', v: { ref: 1 } },
          ],
        },
      },
    })
    const out = shown(evaluated)
    expect(out.formattedText).toBe(
      [
        '{',
        '  "title": "Example Domain",',
        '  "links": [',
        '    1,',
        '    NaN,',
        '    undefined',
        '  ],',
        '  "when": Date "2026-11-12T04:00:00.000Z",',
        '  "self": [Circular]',
        '}',
      ].join('\n'),
    )
    expect(out.source).toContain("The value the script's await got")
    expect(out.rawText).toContain('"frames"')
    // A returned string reads as its lines.
    const text = shown(
      operation(8, 'locator.evaluate', {
        result: { value: 'line one\nline two' },
      }),
    )
    expect(text.lines).toEqual(['line one', 'line two'])
  })

  it('shows a string as its lines, a handle in words, no value as such, and an error as thrown', () => {
    expect(
      shown(operation(1, 'page.title', { result: 'Shipping\nform' })).lines,
    ).toEqual(['Shipping', 'form'])
    expect(
      shown(
        operation(2, 'context.newPage', { result: { $neo: 'page', tab: 18 } }),
      ).lines,
    ).toEqual(['page (tab 18)'])
    expect(shown(operation(3, 'locator.click', { result: null })).source).toBe(
      'The call returned no value.',
    )
    const failed = shown(
      operation(4, 'locator.click', {
        outcome: 'error',
        error: {
          owner: 'script',
          code: 'strict_mode_violation',
          message: 'resolved to 2 elements',
          retryable: false,
        },
      }),
    )
    expect(failed.lines).toEqual([
      'script / strict_mode_violation: resolved to 2 elements',
    ])
    expect(shown(operation(5, 'page.goto', { outcome: null })).source).toBe(
      'The call has not finished.',
    )
  })

  it("says the rest is Neo's recorded result, and when Neo kept only its start", () => {
    const goto = shown(
      operation(1, 'page.goto', {
        result: { status: 200, url: 'https://example.com/' },
        resultTruncated: true,
      }),
    )
    expect(goto.source).toContain(
      "Neo's record of the result as it crossed into the run",
    )
    expect(goto.cut).toBe(true)
    // Tagged data stays as recorded.
    expect(
      shown(
        operation(2, 'page.screenshot', {
          result: { $neo: 'bytes', base64: 'iVBORw0K' },
        }),
      ).formattedText,
    ).toBe(JSON.stringify({ $neo: 'bytes', base64: 'iVBORw0K' }, null, 2))
  })
})

describe('where a call sits', () => {
  it('numbers the calls of each method on a line, and words them', () => {
    const operations = [1, 2, 3].flatMap((pass) => [
      operation(pass * 10, 'context.newPage', { line: 2 }),
      operation(pass * 10 + 1, 'page.goto', { line: 3 }),
    ])
    const places = callPlaces(
      placeOperations(
        record([...operations, operation(99, 'events.listen')]),
        4,
      ),
    )
    expect(places.get('neo-run.20')).toEqual({ line: 2, call: 2, calls: 3 })
    expect(places.get('neo-run.31')).toEqual({ line: 3, call: 3, calls: 3 })
    expect(placeText({ line: 2, call: 2, calls: 3 })).toBe(
      'line 2 · call 2 of 3',
    )
    expect(placeText({ line: 2, call: 1, calls: 1 })).toBe('line 2')
    expect(
      placeText(places.get('neo-run.99') ?? { line: 0, call: 0, calls: 0 }),
    ).toBe('not on a line')
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 103].map(ordinal)).toEqual([
      '1st',
      '2nd',
      '3rd',
      '4th',
      '11th',
      '12th',
      '13th',
      '21st',
      '22nd',
      '103rd',
    ])
  })

  it("shows a long argument's start under the row: the panel has the whole value", () => {
    const lines = argumentLines('page.snapshotPending', {
      text: 'x'.repeat(5_000),
    }).map(lineText)
    expect(lines.join('\n')).toContain('… (4800 more chars)')
  })
})
