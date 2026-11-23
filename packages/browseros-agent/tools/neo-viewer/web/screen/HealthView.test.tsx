/**
 * The Health view against the fake API: its sections from `GET /api/metrics`, read again when
 * health moves, a designed note while there is nothing live, and the raw families folded.
 */

import { afterEach, describe, expect, it } from 'bun:test'
import type { MetricsView, TelemetryHealth } from '../../shared/api'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { health, metricsView } from '../test-support/telemetry'
import { HealthView } from './HealthView'

let dom: TestDom
afterEach(() => dom.restore())

async function show(
  metrics: () => MetricsView | null,
  shown: TelemetryHealth = health(),
  onBack = () => {},
) {
  dom = await installDom(new Map(), { metrics })
  await dom.render(<HealthView health={shown} info={null} onBack={onBack} />)
}

const section = (name: string) =>
  dom.container.querySelector(`section[aria-label="${name}"]`) as Element
/** Each row of a section as its cells' text, empty cells left out: `All runs · 15 · 1 · …`. */
const rows = (name: string) =>
  [...section(name).querySelectorAll('div.h-7')].map((row) =>
    [...row.children]
      .map((cell) => (cell.textContent ?? '').replace(/\s+/g, ' ').trim())
      .filter(Boolean)
      .join(' · '),
  )

describe('the Health view', () => {
  it("lists runs, operations, errors, tabs and crashes from Neo's metrics", async () => {
    await show(metricsView)
    expect(section('Runs').textContent).toContain('15 ended')
    expect(rows('Runs')).toEqual([
      'All runs · 15 · 1 · 1.60 s · 2.41 s',
      'ok · 14',
      'error · 1',
    ])
    // Most called first; what failed in the error tone; an op with no durations says so.
    expect(rows('Operations')).toEqual([
      'locator.click · 21 · 11 · 523 ms · 952 ms',
      'page.goto · 10 · 0 · 375 ms · 488 ms',
      'page.title · 10 · 0 · – · –',
    ])
    expect(section('Operations').textContent).toContain('41 calls, 3 methods')
    expect(rows('Errors')).toEqual([
      'script no_match · 11',
      'website navigation_failed · 1',
    ])
    expect(rows('Tabs')).toEqual([
      'agent · 2 · 10',
      'found · 1 · 1',
      'All closed · 8 · > 10 s · > 10 s',
      'script · 8',
      'janitor · 0',
    ])
    expect(section('Crashes').textContent).toContain(
      "No tab's page process has crashed since Neo started.",
    )
  })

  it('lists where navigations landed and what CAPTCHA solves did and cost', async () => {
    await show(metricsView)
    expect(section('Navigations').textContent).toContain('12 navigated')
    expect(section('Navigations').textContent).toContain(
      '1 refused by the network',
    )
    expect(rows('Navigations')).toEqual([
      '200 · 9',
      'no status · 2',
      '404 · 1',
      'name_not_resolved · 1',
    ])
    // A 404 is red though its goto settled ok; a 200 is not.
    const [ok, , notFound] = section('Navigations').querySelectorAll('div.h-7')
    expect(ok?.querySelector('[data-bar="failed"]')).toBeNull()
    expect(notFound?.querySelector('[data-bar="failed"]')).not.toBeNull()
    expect(section('CAPTCHA').textContent).toContain('3 solves')
    expect(section('CAPTCHA').textContent).toContain('$0.0012 charged')
    expect(rows('CAPTCHA')).toEqual([
      'turnstile solved · 2',
      'perimeterx unsupported · 1',
      'checkbox_passed · 2',
      'unsupported_type · 1',
      'capsolver · $0.0012',
    ])
    const [solved, unsupported] = section('CAPTCHA').querySelectorAll('div.h-7')
    expect(solved?.querySelector('[data-bar="failed"]')).toBeNull()
    expect(unsupported?.querySelector('[data-bar="failed"]')).not.toBeNull()
  })

  it('says so when nothing navigated and no CAPTCHA was solved', async () => {
    await show(() => ({
      ...metricsView(),
      navigations: { total: 0, byStatus: {}, errors: {} },
      captchas: { total: 0, solves: [], byReason: {}, spendUsd: {} },
    }))
    expect(section('Navigations').textContent).toContain(
      'No page has navigated since Neo started.',
    )
    expect(section('CAPTCHA').textContent).toContain(
      'No CAPTCHA solve has run since Neo started.',
    )
  })

  it("draws a call count's failed share in red, and each histogram on Neo's own buckets", async () => {
    await show(metricsView)
    const click = section('Operations').querySelector('div.h-7') as Element
    expect(
      click.querySelector('[data-bar="failed"]')?.getAttribute('style'),
    ).toBe(`width:${(11 / 21) * 100}%`)
    const spread = click.querySelector('[data-spread]') as Element
    expect(spread.children).toHaveLength(12)
    expect(spread.getAttribute('title')).toContain('≤ 250 ms 10 · ≤ 500 ms 0')
    // The columns are labelled at their ends with Neo's first and last bounds.
    expect(section('Operations').textContent).toContain('5 ms10 s')
  })

  it('reads the metrics again when health moves, and keeps them while it does', async () => {
    let calls = 21
    await show(() => {
      const view = metricsView()
      const [click] = view.operations
      if (click) click.total = calls++
      return view
    })
    expect(rows('Operations')[0]).toStartWith('locator.click · 21 ·')
    await dom.render(
      <HealthView health={health({ rev: 2 })} info={null} onBack={() => {}} />,
    )
    expect(rows('Operations')[0]).toStartWith('locator.click · 22 ·')
    expect(dom.fetched.filter((path) => path === '/api/metrics')).toHaveLength(
      2,
    )
  })

  it('says what it is waiting for when no source is live', async () => {
    await show(() => null)
    expect(dom.container.textContent).toContain(
      "No live metrics: Neo has sent none in the last 30 s. The viewer scrapes Neo's GET /v1/metrics every 2 s when it has a host API, and takes OTLP pushes at /otlp.",
    )
    expect(dom.container.querySelector('section[aria-label="Runs"]')).toBeNull()
  })

  it('folds the raw families, and goes back to the runs', async () => {
    let back = 0
    await show(metricsView, health(), () => back++)
    expect(dom.container.textContent).not.toContain('# TYPE neo_runs_total')
    const fold = section('Raw').querySelector('button') as Element
    expect(fold.textContent).toContain(
      'neo_runs_total, neo_run_duration_seconds',
    )
    await dom.interact(() => fire(fold, 'click'))
    expect(section('Raw').textContent).toContain(
      '# TYPE neo_runs_total counter',
    )
    const runs = [...dom.container.querySelectorAll('button')].find(
      (button) => button.textContent === '← Runs',
    ) as Element
    await dom.interact(() => fire(runs, 'click'))
    expect(back).toBe(1)
  })
})
