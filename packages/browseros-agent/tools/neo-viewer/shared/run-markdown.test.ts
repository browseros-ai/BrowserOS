import { describe, expect, test } from 'bun:test'
import {
  detailWith,
  TWO_LINES,
  twoLineRecord,
} from '../server/test-support/neo-records'
import { clock } from '../web/screen/units'
import { fenced, firstStatement, runMarkdown } from './run-markdown'

const FULL = { replyChars: null }
const EXPORT = {
  replyChars: 2_000,
  allOperations: true,
  replyFile: '001.reply.txt',
}

describe('one run as Markdown', () => {
  test('an ok run: the header, the code as sent, each line’s methods, then all the model read', () => {
    const run = detailWith(1, {
      text: '{"title":"Example"}',
      neo: twoLineRecord(),
    })
    const markdown = runMarkdown(run, FULL)
    expect(markdown.split('\n')[0]).toBe(
      `# Run 001 · ${clock(run.sentAt)} · claude-code · ok · round trip 1.00 s · Neo 900 ms · 4 method calls · code ≈15 tok · reply ≈4 tok`,
    )
    expect(markdown).toContain(`## Code\n\n\`\`\`js\n${TWO_LINES}\n\`\`\``)
    expect(markdown).toContain(
      '## Methods by line\n\n- L1 page.goto: +10 ms, 412 ms, ok\n- L2 locator.click: +425 ms, 80 ms, ok\n\n2 operations not on a code line.',
    )
    // Copy lists the rest only as a count: no operations table, no console section.
    expect(markdown).not.toContain('## Operations')
    expect(markdown).not.toContain('## Error')
    expect(
      markdown
        .trimEnd()
        .endsWith(
          '## What the model read\n\n19 chars, ≈4 tok.\n\n```text\n{"title":"Example"}\n```',
        ),
    ).toBe(true)
  })

  test('the export’s form adds every operation, the ones on no line, and the console lines', () => {
    const run = detailWith(1, {
      text: '7\nconsole:\nhi\n[warn] careful',
      neo: twoLineRecord(),
    })
    const markdown = runMarkdown(run, EXPORT)
    expect(markdown).toContain(
      '| 1 | events.listen | read | – | +0 ms | 2 ms | ok |  |\n| 2 | page.goto | write | 1 | +10 ms | 412 ms | ok |  |',
    )
    expect(markdown).toContain('## Operations not on a line')
    expect(markdown).toContain('events.listen, frame.waitForLoadState')
    expect(markdown).toContain('## Console\n\n```text\nhi\n[warn] careful\n```')
  })

  test('a failed run says why, in Neo’s words, and marks the operation that failed', () => {
    const text =
      'script / strict_mode_violation: locator.click: strict mode violation: 2 elements'
    const run = detailWith(2, { text, neo: twoLineRecord(true), isError: true })
    const markdown = runMarkdown(run, EXPORT)
    expect(markdown.split('\n')[0]).toContain(' · failed · ')
    expect(markdown).toContain(
      '- L2 locator.click: +425 ms, 80 ms, error strict_mode_violation',
    )
    // The error the model read, once: Neo's record says the same, so it is not repeated.
    expect(markdown).toContain(
      `## Error\n\n\`\`\`text\n${text}\n\`\`\`\n\n## What the model read`,
    )
    // When Neo's record adds something the model did not read, it is said too.
    const vague = runMarkdown(
      detailWith(2, {
        text: 'neo / internal: run failed',
        neo: twoLineRecord(true),
        isError: true,
      }),
      EXPORT,
    )
    expect(vague).toContain(
      "Neo's record: script / strict_mode_violation: locator.click: strict mode violation: 2 elements",
    )
    expect(markdown).toContain(
      '| 4 | locator.click | write | 2 | +425 ms | 80 ms | error | strict_mode_violation: locator.click: strict mode violation: 2 elements |',
    )
  })

  test('a run Neo has no record of says so, and still shows its code and reply', () => {
    const run = detailWith(3, { text: '1', neo: null })
    const markdown = runMarkdown(run, EXPORT)
    expect(markdown.split('\n')[0]).toContain(' · method calls unknown · ')
    expect(markdown).toContain('No record from Neo for this run')
    expect(markdown).toContain('```js\n')
    expect(markdown).toContain('1 char, ≈0 tok.')
  })

  test('a reply holding triple backticks gets a longer fence, so it cannot end early', () => {
    const text = 'here:\n```js\nconst a = 1\n```\nand ````four````'
    const run = detailWith(4, { text: JSON.stringify(text), neo: null })
    const markdown = runMarkdown(run, FULL)
    const read = markdown.slice(markdown.indexOf('## What the model read'))
    const fence = '`````'
    expect(read).toContain(`${fence}text\n${JSON.stringify(text)}\n${fence}`)
    expect(fenced('```')).toBe('````\n```\n````')
    expect(fenced('plain', 'js')).toBe('```js\nplain\n```')
  })

  test('a long reply is cut at the limit, with its full size and the file that holds all of it', () => {
    const text = 'x'.repeat(10_000)
    const markdown = runMarkdown(detailWith(5, { text, neo: null }), {
      ...EXPORT,
      replyFile: '005.reply.txt',
    })
    expect(markdown).toContain(
      '10,000 chars, ≈2,500 tok. The first 2,000 characters; the full text is in [005.reply.txt](005.reply.txt).',
    )
    expect(markdown).toContain(`\`\`\`text\n${'x'.repeat(2_000)}\n\`\`\``)
    expect(markdown.length).toBeLessThan(4_000)
  })

  test('the first statement skips blank lines and comments, and is cut short', () => {
    expect(
      firstStatement('\n  // open it\n  await page.goto(url)\nreturn 1'),
    ).toBe('await page.goto(url)')
    expect(firstStatement('a'.repeat(80), 10)).toBe('aaaaaaaaa…')
  })
})
