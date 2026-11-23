import { describe, expect, it } from 'bun:test'
import { operation, record } from '../test-support/operations'
import { runSummary } from '../test-support/runs'
import { runTrace } from '../test-support/telemetry'
import { lineText } from './text-lines'
import {
  attributeLines,
  isInternalSpan,
  namesWithCounts,
  operationFor,
  selectedSpan,
  shortTraceId,
  siteDetail,
  spanBar,
  spanEnd,
  spanName,
  splitSpans,
  traceFacts,
} from './trace'

describe("a run's trace", () => {
  const trace = runTrace('neo-run')
  const span = (seq: number) =>
    trace.spans.find((one) => one.spanId === `span-${seq}`) ?? trace.spans[0]

  it('says what it holds in one line, and while neo.run has not arrived, that it is partial', () => {
    const facts = (shape: typeof trace) =>
      traceFacts(shape).map(
        (fact) =>
          `${fact.text}${fact.tone === 'plain' ? '' : ` [${fact.tone}]`}`,
      )
    expect(facts(trace)).toEqual(['9 spans', '1 failed [bad]', '1.23 s'])
    expect(facts(runTrace('neo-run', { complete: false }))).toEqual([
      '8 spans',
      '1 failed [bad]',
      '1.23 s',
      'waiting for neo.run [busy]',
    ])
    // Nothing failed: no failure fact at all.
    const allOk = {
      ...trace,
      spans: trace.spans.map((span) => ({
        ...span,
        attributes: { ...span.attributes, 'neo.op.outcome': 'ok' },
      })),
    }
    expect(facts(allOk)).toEqual(['9 spans', '1.23 s'])
    expect(shortTraceId(trace.traceId)).toBe('4bf92f35')
  })

  it("draws each span inside the run's time", () => {
    const goto = span(1)
    expect(goto && spanName(goto)).toBe('page.goto')
    expect(trace.spans[0] && spanName(trace.spans[0])).toBe('neo.run')
    const click = span(7)
    expect(click && spanBar(click, trace.durationMs)).toEqual({
      left: 702 / 1230,
      width: 508 / 1230,
    })
    // A span reaching past the axis is cut at its end.
    expect(
      click && spanBar({ ...click, durationMs: 5000 }, trace.durationMs),
    ).toEqual({ left: 702 / 1230, width: 1 - 702 / 1230 })
  })

  it('shows the span picked in the waterfall, else the selected call’s, else the run’s own', () => {
    expect(selectedSpan(trace, 'span-3', 'neo-run.7')?.spanId).toBe('span-3')
    expect(selectedSpan(trace, null, 'neo-run.7')?.spanId).toBe('span-7')
    expect(selectedSpan(trace, null, null)?.spanId).toBe('span-run')
    expect(selectedSpan(trace, 'gone', 'neo-run.99')?.spanId).toBe('span-run')
  })

  it('names the call a span is, as the run block selects it', () => {
    const run = runSummary(3, {
      code: "await page.goto('https://example.com')\nawait page.title()",
    })
    const neo = record([
      operation(1, 'page.goto', { line: 1 }),
      operation(8, 'page.title', { line: 2 }),
    ])
    const title = span(8)
    expect(title && operationFor(run, neo, title)).toEqual({
      runId: 'run-3',
      operationId: 'neo-run.8',
      place: { line: 2, call: 1, calls: 1 },
    })
    // The run's own span, and an op the record does not hold, name no call.
    expect(trace.spans[0] && operationFor(run, neo, trace.spans[0])).toBeNull()
    const click = span(7)
    expect(click && operationFor(run, neo, click)).toBeNull()
    expect(title && operationFor(run, null, title)).toBeNull()
  })

  it('says whether any span carries site detail, which is what redaction leaves', () => {
    expect(siteDetail(trace).level).toBe('none')
    expect(
      siteDetail(
        runTrace('neo-run', { attributes: { 'url.domain': 'example.com' } }),
      ).level,
    ).toBe('host')
    expect(
      siteDetail(
        runTrace('neo-run', {
          attributes: {
            'url.domain': 'example.com',
            'url.full': 'https://example.com/a',
          },
        }),
      ).level,
    ).toBe('url')
  })

  it('lists attributes as sent, keys aligned, errors and site detail marked', () => {
    const lines = attributeLines({
      'neo.op.name': 'locator.click',
      'code.line.number': 6,
      'neo.error.code': 'no_match',
      'url.domain': 'example.com',
    })
    expect(lines.map(lineText)).toEqual([
      'neo.op.name       "locator.click"',
      'code.line.number  6',
      'neo.error.code    "no_match"',
      'url.domain        "example.com"',
    ])
    expect(lines.map((line) => line[1]?.tone)).toEqual([
      'string',
      'plain',
      'error',
      'note',
    ])
  })
})

describe("Neo's own work in a trace", () => {
  const withInternal = runTrace('neo-run', { internal: true })
  const byId = (id: string) => {
    const found = withInternal.spans.find((span) => span.spanId === id)
    if (!found) throw new Error(id)
    return found
  }

  it('takes how a span ended from what Neo says on it, never its status', () => {
    // events.next ends cancelled in every run, and an earlier Neo marked that ERROR.
    expect(byId('span-10').error).toBe(true)
    expect(spanEnd(byId('span-10'))).toBe('cancelled')
    expect(spanEnd(byId('span-7'))).toBe('failed')
    expect(spanEnd(byId('span-1'))).toBe('ok')
    const run = byId('span-run')
    const status = (value: string) =>
      spanEnd({
        ...run,
        attributes: { ...run.attributes, 'neo.run.status': value },
      })
    expect(status('ok')).toBe('ok')
    expect(status('error')).toBe('failed')
    expect(status('timeout')).toBe('failed')
    expect(status('cancelled')).toBe('cancelled')
  })

  it("puts the script's calls first and Neo's own work after, each with what ran inside it", () => {
    const { script, internal } = splitSpans(withInternal.spans)
    expect(script.map((span) => span.spanId)).toEqual([
      'span-run',
      'span-1',
      'span-2',
      'span-3',
      'span-4',
      'span-5',
      'span-6',
      'span-7',
      'span-8',
    ])
    expect(internal.every(isInternalSpan)).toBe(true)
    expect(namesWithCounts(internal)).toBe(
      'context.newPage, events.next, page.frames ×2, events.listen ×2',
    )
    // An operation that ran inside another is not Neo's own: it stays under its call.
    expect(isInternalSpan(byId('span-5'))).toBe(false)
  })

  it('counts as failed only the operations Neo says failed: not a cancelled op, not the run', () => {
    const facts = (shape: typeof withInternal) =>
      traceFacts(shape).map((fact) => fact.text)
    expect(facts(withInternal)).toEqual(['15 spans', '1 failed', '1.23 s'])
    const runStatus = (status: string, outcome7: string) => ({
      ...withInternal,
      spans: withInternal.spans.map((span) =>
        span.spanId === 'span-run'
          ? {
              ...span,
              attributes: { ...span.attributes, 'neo.run.status': status },
            }
          : span.spanId === 'span-7'
            ? {
                ...span,
                attributes: { ...span.attributes, 'neo.op.outcome': outcome7 },
              }
            : span,
      ),
    })
    // The run failed because its click did: the click is the failure.
    expect(facts(runStatus('error', 'error'))).toEqual([
      '15 spans',
      '1 failed',
      '1.23 s',
    ])
    // A run that timed out with its ops cancelled has no failed operation; its row says how it ended.
    expect(facts(runStatus('timeout', 'cancelled'))).toEqual([
      '15 spans',
      '1.23 s',
    ])
  })
})
