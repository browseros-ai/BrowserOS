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
  it('explains omitted values without treating the missing result as null', () => {
    const omitted = operation(1, 'page.evaluate', {
      values: 'off',
      args: null,
      result: null,
    })
    const output = shown(omitted)
    expect(output.formattedText).toBe('(values not recorded)')
    expect(output.source).toBe(
      'Neo did not record values for this operation (trace.values is off)',
    )
    expect(output.rawText).toBe('')
    expect(output.cut).toBe(false)
    const failed = shown({
      ...omitted,
      outcome: 'error',
      error: {
        owner: 'script',
        code: 'no_match',
        message: '',
        retryable: true,
      },
    })
    expect(failed.formattedText).toContain('script / no_match')
    expect(JSON.parse(failed.rawText)).toMatchObject({
      owner: 'script',
      code: 'no_match',
      message: '',
    })
  })

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

  it("renders mode 'default' with the run's own renderer, and shows a locator's v1 snapshot as the tree", () => {
    const tree = {
      document: 4,
      partial: [],
      json: [{ role: 'heading', name: 'Hello', level: 1, children: [] }],
    }
    const yaml = shown(
      operation(3, 'page.ariaSnapshot', {
        line: 1,
        args: { mode: 'default' },
        result: tree,
      }),
    )
    expect(yaml.lines).toEqual(['- heading "Hello" [level=1]'])
    expect(yaml.source).toContain("rendered from Neo's recorded tree")
    // Frames Neo could not read: the prelude's own text for this tree (renderSnapshot, run once
    // in neo-playwright-js), each comment under its iframe, or at the end when it is not printed.
    const checkout = {
      document: 3,
      json: [
        {
          role: 'main',
          children: [
            { role: 'heading', name: 'Checkout', level: 1, children: [] },
            { role: 'iframe', ref: 'e7', children: [] },
            { role: 'button', name: 'Pay', ref: 'e9', children: [] },
          ],
        },
      ],
      partial: [
        {
          ref: 'e7',
          frame: 'https://pay.example/form',
          reason: 'timeout after 2000 ms',
        },
        { ref: 'e99', frame: 'https://gone.example/', reason: 'detached' },
      ],
    }
    const partial = shown(
      operation(3, 'locator.ariaSnapshot', {
        args: { mode: 'default' },
        result: checkout,
      }),
    )
    expect(partial.formattedText).toBe(
      '- main:\n  - heading "Checkout" [level=1]\n  - iframe [ref=e7]\n    # partial: https://pay.example/form not read (timeout after 2000 ms)\n  - button "Pay" [ref=e9]\n# partial: https://gone.example/ not read (detached)',
    )
    const locator = shown(
      operation(3, 'locator.ariaSnapshot', {
        line: 1,
        args: { mode: 'ai' },
        result: tree,
      }),
    )
    expect(locator.formattedText).toBe(JSON.stringify(tree, null, 2))
    expect(locator.source).toContain('that text is not recorded')
  })

  it("never shows another call's text: overlapping snapshots of one tab say so, other tabs keep theirs", () => {
    const tree = { document: 4, json: [] }
    const pendingFor = (seq: number, tabId: number, text: string) =>
      operation(seq, 'page.snapshotPending', {
        line: 1,
        tabId,
        args: { document: 4, text, form: 'text', delta: false },
      })
    // Promise.all of two snapshots of one tab: either pending could be either's.
    const sameTab = [
      operation(3, 'page.ariaSnapshot', { line: 1, tabId: 1, result: tree }),
      operation(4, 'page.ariaSnapshot', { line: 1, tabId: 1, result: tree }),
      pendingFor(5, 1, 'first'),
      pendingFor(6, 1, 'second'),
    ]
    expect(shown(sameTab[0] as NeoOperation, sameTab).source).toContain(
      'overlapped',
    )
    expect(shown(sameTab[1] as NeoOperation, sameTab).source).toContain(
      'overlapped',
    )
    // The same, on two tabs: each snapshot's own text.
    const twoTabs = [
      operation(3, 'page.ariaSnapshot', { line: 1, tabId: 1, result: tree }),
      operation(4, 'page.ariaSnapshot', { line: 1, tabId: 2, result: tree }),
      pendingFor(5, 1, 'tab one'),
      pendingFor(6, 2, 'tab two'),
    ]
    expect(shown(twoTabs[0] as NeoOperation, twoTabs).lines).toEqual([
      'tab one',
    ])
    expect(shown(twoTabs[1] as NeoOperation, twoTabs).lines).toEqual([
      'tab two',
    ])
  })

  it('pairs a later snapshot again once overlapping ones are done, and skips calls that record no text', () => {
    const tree = { document: 4, json: [] }
    const snap = (seq: number, extra: Partial<NeoOperation> = {}) =>
      operation(seq, 'page.ariaSnapshot', {
        line: 1,
        tabId: 1,
        result: tree,
        ...extra,
      })
    const pend = (seq: number, text: string) =>
      operation(seq, 'page.snapshotPending', {
        line: 1,
        tabId: 1,
        args: { document: 4, text, form: 'text', delta: false },
      })
    // Two overlapped, then a third ran alone.
    const all = [
      snap(1),
      snap(2),
      pend(3, 'a or b'),
      pend(4, 'b or a'),
      snap(5),
      pend(6, 'third'),
    ]
    expect(shown(all[0] as NeoOperation, all).source).toContain('overlapped')
    expect(shown(all[1] as NeoOperation, all).source).toContain('overlapped')
    expect(shown(all[4] as NeoOperation, all).lines).toEqual(['third'])
    // ariaSnapshotJSON({ mode: 'ai' }) and a failed snapshot never record text, so neither
    // makes the next one look overlapped.
    const json = snap(1, {
      args: {
        mode: 'ai',
        call: { api: 'page.ariaSnapshotJSON', timeoutMs: 0 },
      },
    })
    const failedSnap = snap(2, {
      outcome: 'error',
      error: {
        owner: 'website',
        code: 'timeout',
        message: 'snapshot timed out',
        retryable: true,
      },
    })
    const after = [json, failedSnap, snap(3), pend(4, 'mine')]
    expect(shown(after[2] as NeoOperation, after).lines).toEqual(['mine'])
  })

  it('closes a delta call whose stored baseline could not be read, so the next one keeps its text', () => {
    const tree = { document: 4, json: [] }
    const all = [
      operation(1, 'page.ariaSnapshot', { line: 1, result: tree }),
      operation(2, 'page.snapshotBaseline', {
        line: 1,
        args: { document: 4 },
        outcome: 'error',
        error: {
          owner: 'neo',
          code: 'timeout',
          message: 'baseline read timed out',
          retryable: true,
        },
      }),
      operation(3, 'page.ariaSnapshot', { line: 1, result: tree }),
      operation(4, 'page.snapshotPending', {
        line: 1,
        args: { document: 4, text: 'second', form: 'text', delta: false },
      }),
    ]
    const first = shown(all[0] as NeoOperation, all)
    expect(first.lines).toEqual(['neo / timeout: baseline read timed out'])
    expect(first.source).toContain(
      "reading this tab's stored baseline for the delta failed",
    )
    expect(shown(all[2] as NeoOperation, all).lines).toEqual(['second'])
  })

  it("says who got a failure of Neo's own bookkeeping", () => {
    const listen = shown(
      operation(1, 'events.listen', {
        outcome: 'error',
        error: {
          owner: 'neo',
          code: 'closed',
          message: 'page closed',
          retryable: false,
        },
      }),
    )
    expect(listen.source).toContain('its event bookkeeping failed')
    expect(listen.source).toContain('the run handles that failure itself')
    // The snapshot call awaits its own bookkeeping, so its failure reached the script.
    const pendingFailed = shown(
      operation(2, 'page.snapshotPending', {
        outcome: 'error',
        error: {
          owner: 'neo',
          code: 'timeout',
          message: 'timed out',
          retryable: true,
        },
      }),
    )
    expect(pendingFailed.source).toContain(
      "the script's ariaSnapshot() await threw this",
    )
  })

  it('shows a pending record on its own as the baseline text it records, not the delta', () => {
    const recorded = shown(
      operation(1, 'page.snapshotPending', {
        args: {
          document: 4,
          text: 'the whole page',
          form: 'text',
          delta: true,
          returned: '# changes since…',
        },
      }),
    )
    expect(recorded.lines).toEqual(['the whole page'])
    expect(recorded.source).toContain('pending delta baseline')
  })

  it('flags only the text it shows as cut: a whole delta beside a cut baseline is whole', () => {
    const all = [
      operation(1, 'page.ariaSnapshot', {
        line: 1,
        result: { document: 4, json: [] },
      }),
      operation(2, 'page.snapshotPending', {
        line: 1,
        argsTruncated: true,
        args: {
          document: 4,
          text: '- link "One" [ref=e1]\n…[+90000 bytes]',
          form: 'text',
          delta: true,
          returned: '# no change since your last snapshot of this tab',
        },
      }),
    ]
    const delta = shown(all[0] as NeoOperation, all)
    expect(delta.lines).toEqual([
      '# no change since your last snapshot of this tab',
    ])
    expect(delta.cut).toBe(false)
    // The pending record on its own shows that baseline, which Neo did cut.
    expect(shown(all[1] as NeoOperation, all).cut).toBe(true)
  })

  it('does not claim the script got a value nested deeper than the run can decode', () => {
    let value: unknown = { id: 0, o: [] }
    for (let id = 1; id <= 60; id++)
      value = { id, o: [{ k: 'next', v: value }] }
    const deep = shown(operation(1, 'page.evaluate', { result: { value } }))
    expect(deep.source).toContain(
      "so the script's await may have thrown instead",
    )
    const shallow = shown(
      operation(2, 'page.evaluate', {
        result: { value: { id: 1, o: [{ k: 'a', v: 1 }] } },
      }),
    )
    expect(shallow.source).not.toContain('may have thrown')
  })

  it('says a cancelled call returned nothing, not null', () => {
    const pump = shown(
      operation(1, 'events.next', { outcome: 'cancelled', result: null }),
    )
    expect(pump.lines).toEqual(['(cancelled)'])
    expect(pump.source).toContain('Neo internal: stopped when the run ended')
    expect(
      shown(operation(2, 'page.goto', { outcome: 'cancelled', result: null }))
        .source,
    ).toBe(
      'The call was cancelled before it answered, so it returned no value.',
    )
  })

  it("pairs no text across records Neo dropped, and keeps Neo's own null its own", () => {
    const tree = { document: 4, json: [] }
    const all = [
      operation(1, 'page.ariaSnapshot', { line: 1, tabId: 1, result: tree }),
      operation(3, 'page.snapshotPending', {
        line: 1,
        tabId: 1,
        args: {
          document: 4,
          text: 'maybe not mine',
          form: 'text',
          delta: false,
        },
      }),
    ]
    const gapped = operationOutput(all[0] as NeoOperation, all, 1)
    expect(gapped.source).toContain("Neo dropped some of this run's records")
    expect(gapped.formattedText).toBe(JSON.stringify(tree, null, 2))
    // page.snapshotBaseline found no stored text: Neo's null, not the script's.
    const baseline = shown(
      operation(2, 'page.snapshotBaseline', { result: null }),
    )
    expect(baseline.source).toContain(
      'Neo internal: its own record of this step',
    )
    expect(baseline.source).not.toContain('the script got null')
  })

  it('says which view Neo cut: a whole text from a cut tree', () => {
    const all = [
      operation(3, 'page.ariaSnapshot', {
        line: 1,
        result: { document: 4, json: [{ $more: 900 }] },
        resultTruncated: true,
      }),
      operation(4, 'page.snapshotPending', {
        line: 1,
        args: { document: 4, text: 'whole text', form: 'text', delta: false },
      }),
    ]
    const out = operationOutput(all[0] as NeoOperation, all)
    expect(out.formattedText).toBe('whole text')
    expect(out.cut).toBe(false)
    expect(out.rawCut).toBe(true)
  })

  it('says the await threw when recording the snapshot text failed', () => {
    const all = [
      operation(3, 'page.ariaSnapshot', {
        line: 1,
        result: { document: 4, json: [] },
      }),
      operation(4, 'page.snapshotPending', {
        line: 1,
        args: {
          document: 4,
          text: 'never returned',
          form: 'text',
          delta: false,
        },
        outcome: 'error',
        error: {
          owner: 'neo',
          code: 'timeout',
          message: 'snapshotPending timed out',
          retryable: true,
        },
      }),
    ]
    const out = shown(all[0] as NeoOperation, all)
    expect(out.lines).toEqual(['neo / timeout: snapshotPending timed out'])
    expect(out.source).toContain("so the script's await threw this")
    const cancelled = [
      all[0] as NeoOperation,
      { ...(all[1] as NeoOperation), outcome: 'cancelled', error: null },
    ]
    expect(shown(cancelled[0] as NeoOperation, cancelled).source).toContain(
      'got no text',
    )
  })

  it('decodes each evaluate method by its own envelope, as the run does', () => {
    const object = { id: 1, o: [{ k: 'title', v: 'Example Domain' }] }
    const decoded = '{\n  "title": "Example Domain"\n}'
    // page.evaluate: { value, frames }.
    const page = shown(
      operation(1, 'page.evaluate', { result: { value: object, frames: [] } }),
    )
    expect(page.formattedText).toBe(decoded)
    expect(page.rawText).toContain('"frames"')
    // locator.evaluate and its kin: the serialized value itself.
    for (const name of [
      'locator.evaluate',
      'worker.evaluate',
      'page.evalOnSelector',
      'locator.evaluateAll',
    ]) {
      expect(shown(operation(2, name, { result: object })).formattedText).toBe(
        decoded,
      )
    }
    expect(
      shown(operation(3, 'locator.evaluate', { result: 'line one\nline two' }))
        .lines,
    ).toEqual(['line one', 'line two'])
    // page.waitForFunction resolves to a JSHandle holding the value.
    const wait = shown(
      operation(4, 'page.waitForFunction', {
        result: { value: { n: 3 }, preview: '3' },
      }),
    )
    expect(wait.formattedText).toBe('{\n  "n": 3\n}')
    expect(wait.source).toContain('JSHandle')
    // locator.waitForFunction resolves to nothing.
    expect(
      shown(operation(5, 'locator.waitForFunction', { result: null })).source,
    ).toContain('nothing at all from a method that returns no value')
  })

  it("decodes with the run's own decoder: a shared object once, a cycle as such, typed arrays as numbers", () => {
    const shared = { id: 2, o: [{ k: 'n', v: 1 }] }
    const value = {
      id: 1,
      o: [
        { k: 'first', v: shared },
        { k: 'again', v: { ref: 2 } },
        { k: 'self', v: { ref: 1 } },
        { k: 'bytes', v: { ta: { k: 'ui8', b: 'AQID' } } },
        { k: 'big', v: { ta: { k: 'bi64', b: 'AQAAAAAAAAA=' } } },
        {
          k: 'odd',
          v: { id: 3, a: [{ v: 'NaN' }, { v: '-0' }, { bi: '12' }, { h: 0 }] },
        },
      ],
    }
    expect(
      shown(operation(1, 'page.evaluate', { result: { value } })).formattedText,
    ).toBe(
      [
        '{',
        '  "first": {',
        '    "n": 1',
        '  },',
        '  "again": [the same object as above],',
        '  "self": [Circular],',
        '  "bytes": Uint8Array [1, 2, 3],',
        '  "big": BigInt64Array [1n],',
        '  "odd": [',
        '    NaN,',
        '    -0,',
        '    12n,',
        '    undefined',
        '  ]',
        '}',
      ].join('\n'),
    )
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
    const nothing = shown(
      operation(3, 'locator.getAttribute', { result: null }),
    )
    expect(nothing.lines).toEqual(['null'])
    expect(nothing.source).toContain('the script got null')
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

describe('a value no reading expected', () => {
  it('skips a __proto__ entry as the run does, and stays small for a value full of shared objects', () => {
    const proto = {
      id: 1,
      o: [
        { k: '__proto__', v: { id: 2, o: [{ k: 'x', v: 1 }] } },
        { k: 'y', v: 2 },
      ],
    }
    expect(
      shown(operation(1, 'page.evaluate', { result: { value: proto } }))
        .formattedText,
    ).toBe('{\n  "y": 2\n}')
    // x = { a: x, b: x }, fourteen times over: each object once, then by name.
    let value: unknown = { id: 1, o: [] }
    for (let id = 2; id <= 15; id++) {
      value = {
        id,
        o: [
          { k: 'a', v: value },
          { k: 'b', v: { ref: id - 1 } },
        ],
      }
    }
    const big = shown(operation(2, 'page.evaluate', { result: { value } }))
    expect(big.formattedText.length).toBeLessThan(2_000)
    expect(big.formattedText).toContain('[the same object as above]')
  })

  it('never decodes a value Neo cut, and says so when the decoder refuses one', () => {
    const cut = shown(
      operation(1, 'locator.evaluate', {
        result: { ta: { k: 'ui8', b: 'AQIDBAUG…[+60 bytes]' } },
        resultTruncated: true,
      }),
    )
    expect(cut.source).toContain('a cut value cannot be decoded')
    expect(cut.rawText).toContain('…[+60 bytes]')
    const refused = shown(
      operation(2, 'locator.evaluate', {
        result: { ta: { k: 'ui8', b: '%%%' } },
      }),
    )
    expect(refused.source).toContain(
      "The run's decoder refuses this recorded value",
    )
  })

  it("shows ariaSnapshotJSON's tree as the JSON the script got, not as YAML", () => {
    const json = [{ role: 'heading', name: 'Hello', level: 1, children: [] }]
    const out = shown(
      operation(1, 'page.ariaSnapshot', {
        args: {
          mode: 'default',
          call: { api: 'page.ariaSnapshotJSON', timeoutMs: 5000 },
        },
        result: { document: 2, partial: [], json },
      }),
    )
    expect(out.formattedText).toBe(JSON.stringify(json, null, 2))
    expect(out.source).toContain('ariaSnapshotJSON() returns')
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

  it('counts two calls written side by side once each: the line ran once', () => {
    const side = [
      operation(1, 'page.title', { site: { line: 1, column: 9 } }),
      operation(2, 'page.title', { site: { line: 1, column: 31 } }),
    ]
    const places = callPlaces(placeOperations(record(side), 1))
    expect(places.get('neo-run.1')).toEqual({ line: 1, call: 1, calls: 1 })
    expect(places.get('neo-run.2')).toEqual({ line: 1, call: 1, calls: 1 })
  })

  it("counts one place by its calls whatever method each pass called, and Neo's bookkeeping on its own", () => {
    const at = { line: 2, column: 30 }
    const passes = [
      operation(1, 'locator.textContent', { site: at }),
      operation(2, 'locator.innerText', { site: at }),
      operation(3, 'locator.textContent', { site: at }),
    ]
    const places = callPlaces(placeOperations(record(passes), 3))
    expect(passes.map((op) => places.get(op.id))).toEqual([
      { line: 2, call: 1, calls: 3 },
      { line: 2, call: 2, calls: 3 },
      { line: 2, call: 3, calls: 3 },
    ])
    // One snapshot and the pending record it made share a site: one call each.
    const snapshot = [
      operation(4, 'page.ariaSnapshot', { site: at }),
      operation(5, 'page.snapshotPending', { site: at }),
    ]
    const own = callPlaces(placeOperations(record(snapshot), 3))
    expect(own.get('neo-run.4')).toEqual({ line: 2, call: 1, calls: 1 })
    expect(own.get('neo-run.5')).toEqual({ line: 2, call: 1, calls: 1 })
  })

  it("shows a long argument's start under the row: the panel has the whole value", () => {
    const lines = argumentLines('page.snapshotPending', {
      text: 'x'.repeat(5_000),
    }).map(lineText)
    expect(lines.join('\n')).toContain('… (4800 more chars)')
  })
})
