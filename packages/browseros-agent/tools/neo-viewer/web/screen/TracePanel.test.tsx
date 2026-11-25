/**
 * The sidebar's Trace panel against the fake API: the run's waterfall and the picked span's
 * attributes, and a span picked there naming its call in the run's record, which is how the
 * run block opens that call (`ViewerScreen`).
 */

import { afterEach, describe, expect, it } from 'bun:test'
import type { RunDetail, TraceSpan } from '../../shared/api'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { operation, record } from '../test-support/operations'
import { runDetail, runSummary } from '../test-support/runs'
import { runTrace } from '../test-support/telemetry'
import type { OperationSelection } from './operations'
import { TracePanel } from './TracePanel'

const code = [
  "await page.goto('https://example.com')",
  "try { await page.locator('#missing').click({ timeout: 500 }) } catch {}",
].join('\n')

describe("the sidebar's trace", () => {
  let dom: TestDom
  afterEach(() => dom.restore())

  async function panel(
    spanId: string | null,
    operationId: string | null,
    seq: number,
    onSelectSpan: (
      span: TraceSpan,
      call: OperationSelection | null,
    ) => void = () => {},
  ) {
    const run = {
      ...runSummary(seq, { code }),
      trace: {
        traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
        spanId: 'span-viewer',
        injected: true,
      },
    }
    const detail: RunDetail = {
      ...runDetail(run),
      neo: record([
        operation(1, 'page.goto', { line: 1 }),
        operation(7, 'locator.click', { line: 2, outcome: 'error' }),
      ]),
    }
    dom = await installDom(new Map([[run.id, detail]]), {
      traces: new Map([[run.id, runTrace('neo-run')]]),
    })
    await dom.render(
      <TracePanel
        run={run}
        spanId={spanId}
        operationId={operationId}
        rev={1}
        onSelectSpan={onSelectSpan}
        onShowCall={null}
        onShowReply={() => {}}
        onOpenFull={() => {}}
      />,
    )
  }
  const pressed = () =>
    dom.container
      .querySelector('button[aria-pressed="true"]')
      ?.getAttribute('data-span')

  it("draws the run's spans and says where its traceparent came from", async () => {
    await panel(null, null, 41)
    const text = dom.container.textContent ?? ''
    expect(text).toContain(
      '4bf92f35 · 9 spans, 1 failed, 1.23 s · traceparent from the viewer',
    )
    expect(dom.container.querySelectorAll('button[data-span]')).toHaveLength(9)
    // Nothing picked: the run's own span and its attributes.
    expect(pressed()).toBe('span-run')
    expect(text).toContain('neo.run.status')
    expect(text).toContain(
      'Site detail none: no span carries a URL or host (url.*).',
    )
  })

  it('marks the span of the call selected in the run block', async () => {
    await panel(null, 'neo-run.7', 42)
    expect(pressed()).toBe('span-7')
    expect(dom.container.textContent).toContain('"no_match"')
  })

  it('names the call a picked span is, so the block can open it', async () => {
    const picks: Array<[string, OperationSelection | null]> = []
    await panel(null, null, 43, (span, call) => picks.push([span.spanId, call]))
    const failed = dom.container.querySelector(
      '[data-span="span-7"]',
    ) as Element
    await dom.interact(() => fire(failed, 'click'))
    const run = dom.container.querySelector('[data-span="span-run"]') as Element
    await dom.interact(() => fire(run, 'click'))
    expect(picks).toEqual([
      [
        'span-7',
        {
          runId: 'run-43',
          operationId: 'neo-run.7',
          // The script's second line lays out as three; the click sits on the third.
          place: { line: 3, call: 1, calls: 1 },
        },
      ],
      ['span-run', null],
    ])
  })
})
