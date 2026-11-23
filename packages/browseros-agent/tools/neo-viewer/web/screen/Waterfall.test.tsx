import { afterEach, describe, expect, it } from 'bun:test'
import { useState } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { TraceSpan } from '../../shared/api'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { runSummary } from '../test-support/runs'
import { runTrace } from '../test-support/telemetry'
import { SpanAttributes, TraceFacts, TraceFull, Waterfall } from './Waterfall'

describe('the waterfall', () => {
  let dom: TestDom | null = null
  afterEach(async () => {
    await dom?.restore()
    dom = null
  })
  const trace = runTrace('neo-run')

  it('draws neo.run first and its operations under it, nested ones further in', () => {
    const markup = renderToStaticMarkup(
      <Waterfall
        trace={trace}
        selectedSpanId="span-7"
        onSelectSpan={() => {}}
      />,
    )
    const labels = [...markup.matchAll(/aria-label="([^"]*)"/g)].map(
      (match) => match[1],
    )
    expect(labels).toEqual([
      'neo.run, 1.23 s',
      'page.goto, 415 ms',
      'locator.textContent, 25 ms',
      'locator.count, 18 ms',
      'locator.click, 168 ms',
      'frame.waitForLoadState, 120 ms',
      'page.waitForLoadState, 58 ms',
      'locator.click, 508 ms, failed',
      'page.title, 8 ms',
    ])
    expect(markup).toContain(
      'style="padding-left:20px" title="frame.waitForLoadState"',
    )
    expect(markup).toContain('style="padding-left:0px" title="neo.run neo-run"')
  })

  it('puts every bar on the run’s axis: the failed one red, the picked one pressed', () => {
    const markup = renderToStaticMarkup(
      <Waterfall
        trace={trace}
        selectedSpanId="span-7"
        onSelectSpan={() => {}}
      />,
    )
    const row = /<button[^>]*data-span="span-7"[^>]*>.*?<\/button>/.exec(
      markup,
    )?.[0]
    expect(row).toContain('aria-pressed="true"')
    expect(row).toContain(
      `class="absolute inset-y-0 block bg-bad" style="left:min(${(702 / 1230) * 100}%, calc(100% - 2px));width:max(2px, ${(508 / 1230) * 100}%)"`,
    )
    // The axis runs from 0 to the trace's duration.
    expect(markup).toMatch(/<span>0<\/span><span>1\.23 s<\/span>/)
  })

  it('picks a span when its row is clicked', async () => {
    dom = await installDom()
    const picked: TraceSpan[] = []
    await dom.render(
      <Waterfall
        trace={trace}
        selectedSpanId={null}
        onSelectSpan={(span) => picked.push(span)}
      />,
    )
    const row = dom.container.querySelector('[data-span="span-3"]') as Element
    await dom.interact(() => fire(row, 'click'))
    expect(picked.map((span) => span.opId)).toEqual(['neo-run.3'])
  })

  it("lists the picked span's attributes exactly as Neo sent them", () => {
    const failed = trace.spans.find((span) => span.spanId === 'span-7')
    if (!failed) throw new Error('fixture')
    const text = renderToStaticMarkup(<SpanAttributes span={failed} />)
      .replace(/<[^>]+>/g, '')
      .replaceAll('&quot;', '"')
    expect(text).toContain('locator.clickspan span-7')
    expect(text).toMatch(/neo\.error\.code +"no_match"/)
  })
})

describe("a trace's facts", () => {
  it('reads as one line, what failed in the error tone and a partial trace in amber', () => {
    const markup = renderToStaticMarkup(
      <TraceFacts trace={runTrace('neo-run', { complete: false })} />,
    )
    expect(markup.replace(/<[^>]+>/g, '')).toBe(
      '8 spans, 1 failed, 1.23 s, waiting for neo.run',
    )
    expect(markup).toContain('<span class="text-bad">, 1 failed</span>')
    expect(markup).toContain(
      '<span class="text-busy-ink">, waiting for neo.run</span>',
    )
  })
})

describe("a trace's Open full", () => {
  it('opens on the picked span, steps with ↑ ↓ and the buttons, and shows the resource once', async () => {
    const dom = await installDom()
    const trace = runTrace('neo-run')
    const run = runSummary(3)
    function Full() {
      const [spanId, setSpanId] = useState<string | null>('span-7')
      return (
        <TraceFull
          run={run}
          trace={trace}
          selectedSpanId={spanId}
          onSelectSpan={(span) => setSpanId(span.spanId)}
          onClose={() => {}}
        />
      )
    }
    await dom.render(<Full />)
    const dialog = dom.container.querySelector('dialog') as Element
    const picked = () =>
      dialog.querySelector('[aria-pressed="true"]')?.getAttribute('data-span')
    expect(dialog.hasAttribute('open')).toBe(true)
    expect(picked()).toBe('span-7')
    expect(dialog.textContent).toContain('service.name')
    await dom.interact(() => fire(dialog, 'keydown', { key: 'ArrowDown' }))
    expect(picked()).toBe('span-8')
    // The last span has no next.
    await dom.interact(() => fire(dialog, 'keydown', { key: 'ArrowDown' }))
    expect(picked()).toBe('span-8')
    await dom.interact(() => fire(dialog, 'keydown', { key: 'ArrowUp' }))
    await dom.interact(() => fire(dialog, 'keydown', { key: 'ArrowUp' }))
    expect(picked()).toBe('span-6')
    const previous = [...dialog.querySelectorAll('button')].find(
      (button) => button.textContent === 'Previous span',
    ) as Element
    await dom.interact(() => fire(previous, 'click'))
    expect(picked()).toBe('span-5')
    await dom.restore()
  })
})

describe("the waterfall with Neo's own work (e2e F4)", () => {
  const trace = runTrace('neo-run', { internal: true })
  const rows = (markup: string) =>
    [...markup.matchAll(/data-span="([^"]*)"/g)].map((match) => match[1])

  it("folds Neo's own operations under one quiet row after the script's calls", () => {
    const markup = renderToStaticMarkup(
      <Waterfall trace={trace} selectedSpanId={null} onSelectSpan={() => {}} />,
    )
    expect(rows(markup)).toEqual([
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
    expect(markup.replace(/<[^>]+>/g, '')).toContain(
      '▸Neo internal: 6 operations · context.newPage, events.next, page.frames ×2, events.listen ×2',
    )
    expect(markup).toContain('aria-expanded="false" data-internal="6"')
  })

  it('opens the fold on a click, quieter than the script, and a cancelled op is not red', async () => {
    const dom = await installDom()
    await dom.render(
      <Waterfall trace={trace} selectedSpanId={null} onSelectSpan={() => {}} />,
    )
    const fold = dom.container.querySelector('[data-internal]') as Element
    await dom.interact(() => fire(fold, 'click'))
    expect(dom.container.querySelectorAll('button[data-span]')).toHaveLength(15)
    const next = dom.container.querySelector('[data-span="span-10"]') as Element
    expect(next.getAttribute('aria-label')).toBe(
      'events.next, 1.23 s, cancelled',
    )
    expect(next.querySelector('[data-bar="span"]')?.className).toContain(
      'bg-ink-4',
    )
    expect(next.innerHTML).not.toContain('bg-bad')
    // Only the click that did fail is red.
    expect(
      dom.container.querySelectorAll('[data-bar="span"].bg-bad'),
    ).toHaveLength(1)
    await dom.restore()
  })

  it("opens by itself when one of Neo's own spans is picked (from the run block)", () => {
    const markup = renderToStaticMarkup(
      <Waterfall
        trace={trace}
        selectedSpanId="span-10"
        onSelectSpan={() => {}}
      />,
    )
    expect(rows(markup)).toHaveLength(15)
    expect(markup).toContain('aria-expanded="true"')
  })

  it('steps through the rows as drawn in Open full: the script’s, then Neo’s own', async () => {
    const dom = await installDom()
    function Full() {
      const [spanId, setSpanId] = useState<string | null>('span-8')
      return (
        <TraceFull
          run={runSummary(3)}
          trace={trace}
          selectedSpanId={spanId}
          onSelectSpan={(span) => setSpanId(span.spanId)}
          onClose={() => {}}
        />
      )
    }
    await dom.render(<Full />)
    const dialog = dom.container.querySelector('dialog') as Element
    const picked = () =>
      dialog.querySelector('[aria-pressed="true"]')?.getAttribute('data-span')
    await dom.interact(() => fire(dialog, 'keydown', { key: 'ArrowDown' }))
    expect(picked()).toBe('span-9')
    await dom.interact(() => fire(dialog, 'keydown', { key: 'ArrowUp' }))
    expect(picked()).toBe('span-8')
    await dom.restore()
  })
})

describe("the run's own row", () => {
  it('shows how the run ended on its dot, and keeps its bar the light track', () => {
    const trace = runTrace('neo-run')
    const failedRun = {
      ...trace,
      spans: trace.spans.map((span) =>
        span.type === 'run'
          ? {
              ...span,
              attributes: { ...span.attributes, 'neo.run.status': 'error' },
            }
          : span,
      ),
    }
    const markup = renderToStaticMarkup(
      <Waterfall
        trace={failedRun}
        selectedSpanId={null}
        onSelectSpan={() => {}}
      />,
    )
    const row = /<button[^>]*data-span="span-run"[^>]*>.*?<\/button>/.exec(
      markup,
    )?.[0]
    expect(row).toContain('aria-label="neo.run, 1.23 s, failed"')
    expect(row).toContain(
      'title="failed" class="block size-1.5 rounded-full bg-bad"',
    )
    expect(row).toContain('block bg-control')
    expect(row).not.toContain('block bg-bad"')
  })
})
