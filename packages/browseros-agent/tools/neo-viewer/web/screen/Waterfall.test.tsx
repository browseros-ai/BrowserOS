import { afterEach, describe, expect, it } from 'bun:test'
import { useState } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { TraceSpan } from '../../shared/api'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { runSummary } from '../test-support/runs'
import { runTrace } from '../test-support/telemetry'
import { SpanAttributes, TraceFull, Waterfall } from './Waterfall'

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
      'style="padding-left:20px" title="neo.op frame.waitForLoadState"',
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
    expect(text).toContain('locator.clickneo.op, span span-7')
    expect(text).toContain('neo.error.code   "no_match"')
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
