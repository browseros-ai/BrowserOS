import { describe, expect, test } from 'bun:test'
import {
  detailWith,
  TWO_LINES,
  twoLineRecord,
} from '../server/test-support/neo-records'
import { clock, count } from '../web/screen/units'
import {
  SNAPSHOT,
  SNAPSHOT_WITH_TIP,
  TIP_LINE,
} from '../web/test-support/replies'
import { cell, fenced, runMarkdown, statementPreview } from './run-markdown'

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
    expect(markdown).toContain(
      `one statement per line, as the viewer shows it (whitespace only); \`L<n>\` below is a line of it.\n\n\`\`\`js\n${TWO_LINES}\n\`\`\``,
    )
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
    // Neo answered (it named the run), but its record was not read in time.
    expect(markdown.split('\n')[0]).toContain(
      " · Neo's record had not arrived · ",
    )
    expect(markdown.split('\n')[0]).not.toContain('unknown')
    expect(markdown).toContain('No record from Neo for this run')
    // No Neo run at all (an HTTP error, a lost run): nothing to wait for, so unknown.
    const none = runMarkdown({ ...run, neoRunId: null }, EXPORT)
    expect(none.split('\n')[0]).toContain(' · method calls unknown · ')
    expect(markdown).toContain('```js\n')
    expect(markdown).toContain('1 char, ≈0 tok.')
  })

  test('a reply holding triple backticks gets a longer fence, so it cannot end early', () => {
    const text = 'here:\n```js\nconst a = 1\n```\nand ````four````'
    const run = detailWith(4, { text: JSON.stringify(text), neo: null })
    const markdown = runMarkdown(run, FULL)
    const read = markdown.slice(markdown.indexOf('## What the model read'))
    const fence = '`````'
    // A returned string is shown decoded, in a fence longer than its longest run of backticks.
    expect(read).toContain(`${fence}text\n${text}\n${fence}`)
    // A reply that is not a JSON string keeps its exact text, in a longer fence too.
    const object = runMarkdown(
      detailWith(4, { text: `{"note":"${'`'.repeat(4)}"}`, neo: null }),
      FULL,
    )
    expect(object).toContain(
      `${fence}text\n{"note":"${'`'.repeat(4)}"}\n${fence}`,
    )
    expect(fenced('```')).toBe('````\n```\n````')
    expect(fenced('plain', 'js')).toBe('```js\nplain\n```')
  })

  test('a returned string is shown decoded, its size still what the model received', () => {
    // A snapshot as run() returns it: newlines, quotes, and a run of backticks in the page text.
    const snapshot =
      '- link "Hacker News" [ref=e16]\n- text: "|"\n- text: use ```code``` here'
    const json = JSON.stringify(snapshot)
    const markdown = runMarkdown(
      detailWith(11, { text: json, neo: null }),
      FULL,
    )
    const read = markdown.slice(markdown.indexOf('## What the model read'))
    expect(read).toBe(
      [
        '## What the model read',
        '',
        `${json.length} chars, ≈${Math.floor(json.length / 4)} tok.`,
        '',
        `Shown decoded; the model received it as a JSON string of ${json.length} characters.`,
        '',
        '````text',
        '- link "Hacker News" [ref=e16]',
        '- text: "|"',
        '- text: use ```code``` here',
        '````',
        '',
      ].join('\n'),
    )
    expect(read).not.toContain('\\n')
  })

  test('a decoded string keeps the console lines after it, and the export cuts it with the exact file named', () => {
    const value = Array.from(
      { length: 400 },
      (_, i) => `- link "Story ${i}" [ref=e${i}]`,
    ).join('\n')
    const text = `${JSON.stringify(value)}\nconsole:\nread 400 links`
    const run = detailWith(12, { text, neo: null })
    const copy = runMarkdown(run, FULL)
    expect(copy).toContain(
      '```text\n- link "Story 0" [ref=e0]\n- link "Story 1" [ref=e1]',
    )
    expect(copy).toContain(
      'Then, as the model read them:\n\n```text\nconsole:\nread 400 links\n```',
    )
    const exported = runMarkdown(run, { ...EXPORT, replyFile: '012.reply.txt' })
    expect(exported).toContain(
      'The first 2,000 characters, decoded; the exact text the model received is in [012.reply.txt](012.reply.txt):',
    )
    expect(exported).toContain('```text\n- link "Story 0" [ref=e0]\n')
    // The decoded value used the whole allowance: the console lines are named, not shown again.
    expect(exported).toContain(
      'Console lines or notes follow it; they are in [012.reply.txt](012.reply.txt).',
    )
    const read = exported.slice(exported.indexOf('## What the model read'))
    expect(read).not.toContain('read 400 links')
  })

  test("the text after a decoded string is the model's own, and the export cuts it too", () => {
    // A console line that looks like one of Neo's notes stays a console line, as the model read it.
    const text = `${JSON.stringify('ok')}\nconsole:\n[not awaited] this is a console log`
    const copy = runMarkdown(detailWith(15, { text, neo: null }), FULL)
    expect(copy).toContain(
      'Then, as the model read them:\n\n```text\nconsole:\n[not awaited] this is a console log\n```',
    )
    // Long console output after a short string: the two share the export's one allowance.
    const long = `${JSON.stringify('ok')}\nconsole:\n${'c'.repeat(50_000)}`
    const exported = runMarkdown(detailWith(16, { text: long, neo: null }), {
      ...EXPORT,
      replyFile: '016.reply.txt',
    })
    expect(exported.length).toBeLessThan(8_000)
    expect(exported).toContain(
      'Then, as the model read them:\n\nThe first 1,998 characters; the full text is in [016.reply.txt](016.reply.txt):',
    )
  })

  test("run 8, a returned snapshot then Neo's tip: Copy shows the snapshot decoded and the tip as read", () => {
    const copy = runMarkdown(
      detailWith(8, { text: SNAPSHOT_WITH_TIP, neo: null }),
      FULL,
    )
    const read = copy.slice(copy.indexOf('## What the model read'))
    const json = SNAPSHOT_WITH_TIP.split('\n')[0] ?? ''
    expect(read).toContain(
      `Shown decoded; the model received it as a JSON string of ${count(json.length)} characters.`,
    )
    expect(read).toContain(`\`\`\`text\n${SNAPSHOT}\n\`\`\``)
    expect(read).toContain(
      `Then, as the model read them:\n\n\`\`\`text\n${TIP_LINE}\n\`\`\``,
    )
    expect(read).not.toContain('\\n- ')
  })

  test('a value that is not a string, or an error, is shown as the model read it', () => {
    const object = runMarkdown(
      detailWith(13, { text: '{"title":"Example","n":2}', neo: null }),
      FULL,
    )
    expect(object).toContain('```text\n{"title":"Example","n":2}\n```')
    expect(object).not.toContain('Shown decoded')
    const error = runMarkdown(
      detailWith(14, {
        text: 'script / no_match: "a\\nb"',
        neo: null,
        isError: true,
      }),
      FULL,
    )
    expect(error).not.toContain('Shown decoded')
  })

  test('a long reply is cut at the limit, with its full size and the file that holds all of it', () => {
    const text = 'x'.repeat(10_000)
    const markdown = runMarkdown(detailWith(5, { text, neo: null }), {
      ...EXPORT,
      replyFile: '005.reply.txt',
    })
    expect(markdown).toContain(
      '10,000 chars, ≈2,500 tok.\n\nThe first 2,000 characters; the full text is in [005.reply.txt](005.reply.txt):',
    )
    expect(markdown).toContain(`\`\`\`text\n${'x'.repeat(2_000)}\n\`\`\``)
    expect(markdown.length).toBeLessThan(4_000)
  })

  test('the header and the facts name the tasks: the script’s, and the host’s', () => {
    const run = {
      ...detailWith(9, { text: '1', neo: null }),
      tasks: [
        { id: 'shop-4b80', name: 'shop' },
        { id: 't-91c2', name: null },
      ],
      agentTaskId: 'host-1',
    }
    const markdown = runMarkdown(run, FULL)
    expect(markdown.split('\n')[0]).toContain(
      ' · claude-code · tasks shop-4b80, t-91c2 · host task host-1 · ok · ',
    )
    expect(markdown).toContain(
      "- Tasks the script created or picked: `shop-4b80` (shop), `t-91c2`\n- Host task (the request's `_meta` taskId): `host-1`",
    )
    // A name is Neo's free text: it stays on its line and cannot start Markdown of its own.
    const odd = runMarkdown(
      { ...run, tasks: [{ id: 'shop-4b80', name: 'shop\n\n## Review *now*' }] },
      FULL,
    )
    expect(odd).toContain('`shop-4b80` (shop \\#\\# Review \\*now\\*)')
    expect(odd).not.toContain('\n## Review')
    const one = runMarkdown(
      { ...run, tasks: [{ id: 'shop-4b80', name: 'shop' }], agentTaskId: null },
      FULL,
    )
    expect(one.split('\n')[0]).toContain(
      ' · claude-code · task shop-4b80 · ok · ',
    )
    expect(
      runMarkdown(detailWith(10, { text: '1', neo: null }), FULL).split(
        '\n',
      )[0],
    ).toContain(' · claude-code · ok · ')
  })

  test('the first statement is the page’s, on one line, and cut short', () => {
    expect(statementPreview('await page.goto(url);await page.click("a")')).toBe(
      'await page.goto(url);',
    )
    expect(statementPreview(`return ${'a'.repeat(80)}`, 10)).toBe('return aa…')
  })

  test('minified code is laid out as the page lays it out, and methods land on those lines', () => {
    const code = "await page.goto('https://example.com');await page.click('a')"
    const record = twoLineRecord()
    // Neo records sites against the code as sent: both on line 1.
    for (const operation of record.operations) {
      if (operation.name === 'page.goto')
        operation.site = { line: 1, column: 12 }
      if (operation.name === 'locator.click')
        operation.site = { line: 1, column: 51 }
    }
    const markdown = runMarkdown(
      detailWith(6, { code, text: '1', neo: record }),
      FULL,
    )
    expect(markdown).toContain(
      "```js\nawait page.goto('https://example.com');\nawait page.click('a')\n```",
    )
    expect(markdown).toContain(
      '- L1 page.goto: +10 ms, 412 ms, ok\n- L2 locator.click: +425 ms, 80 ms, ok',
    )
  })

  test('a failed run’s error is cut at the limit too, pointing to the full text', () => {
    const text = `script / huge: ${'e'.repeat(300_000)}`
    const markdown = runMarkdown(
      detailWith(7, { text, neo: null, isError: true }),
      { ...EXPORT, replyFile: '007.reply.txt' },
    )
    expect(markdown.length).toBeLessThan(6_000)
    expect(markdown).toContain(
      '## Error\n\nThe first 2,000 characters; the full text is in [007.reply.txt](007.reply.txt):',
    )
  })

  test('the export cuts long console output at the limit too', () => {
    const text = `1\nconsole:\n${'c'.repeat(50_000)}`
    const markdown = runMarkdown(detailWith(8, { text, neo: null }), {
      ...EXPORT,
      replyFile: '008.reply.txt',
    })
    expect(markdown.length).toBeLessThan(8_000)
    expect(markdown).toContain(
      '## Console\n\nThe first 2,000 characters; the full text is in [008.reply.txt](008.reply.txt):',
    )
  })

  test('a table cell keeps a pipe in its cell, even behind a backslash', () => {
    expect(cell('a | b')).toBe('a \\| b')
    expect(cell('return /\\|/.test(x)')).toBe('return /\\\\\\|/.test(x)')
    expect(cell('one\ntwo')).toBe('one two')
  })
})
