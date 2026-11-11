import { describe, expect, it } from 'bun:test'
import { argumentLines, handleText, resultLines } from './operation-values'
import { lineText } from './text-lines'

const text = (lines: ReturnType<typeof resultLines>) => lines.map(lineText)

describe("a method's arguments and result", () => {
  it('shows a locator by its display, the timeout as timeout, the rest as recorded', () => {
    // locator.selectOption's args as Neo recorded them.
    const args = {
      call: { api: 'locator.selectOption', timeoutMs: 5000 },
      options: [{ valueOrLabel: 'United States' }],
      strict: 'strict',
      target: {
        $neo: 'locator',
        display: "locator('aria-ref=e7')",
        page: { $neo: 'page', tab: 18 },
      },
    }
    expect(text(argumentLines('locator.selectOption', args))).toEqual([
      'options [{"valueOrLabel":"United States"}]',
      'strict  "strict"',
      "target  locator('aria-ref=e7')",
      'timeout 5000',
    ])
  })

  it('says when the script called the method by another name', () => {
    const args = {
      call: { api: 'page.waitForEvent', timeoutMs: 5000 },
      event: 'download',
    }
    expect(text(argumentLines('events.wait', args))).toEqual([
      'called as page.waitForEvent',
      'event     "download"',
      'timeout   5000',
    ])
    expect(argumentLines('context.newPage', {})).toEqual([])
  })

  it('shows a result as JSON, text line by line, and a handle in words', () => {
    expect(text(resultLines(['us']))).toEqual(['[', '  "us"', ']'])
    expect(
      text(resultLines('- heading "Shipping"\n- button "Place order"')),
    ).toEqual(['- heading "Shipping"', '- button "Place order"'])
    expect(text(resultLines(null))).toEqual(['null'])
    expect(handleText({ $neo: 'page', tab: 18 })).toBe('page (tab 18)')
    expect(handleText({ name: 'x' })).toBeNull()
    // Tagged data is not a handle: a screenshot's bytes stay as recorded.
    expect(text(resultLines({ $neo: 'bytes', base64: 'iVBORw0K' }))).toEqual([
      '{',
      '  "$neo": "bytes",',
      '  "base64": "iVBORw0K"',
      '}',
    ])
  })

  it('shows handles nested in arrays and objects by their display', () => {
    // page.screenshot({ mask: [...] }) as Neo records it.
    const args = {
      call: { api: 'page.screenshot', timeoutMs: 5000 },
      mask: [
        {
          $neo: 'locator',
          display: "getByRole('textbox')",
          chunks: [{ parts: [{ body: 'textbox', name: 'internal:role' }] }],
        },
      ],
    }
    expect(text(argumentLines('page.screenshot', args))).toEqual([
      'mask    ["getByRole(\'textbox\')"]',
      'timeout 5000',
    ])
  })
})
