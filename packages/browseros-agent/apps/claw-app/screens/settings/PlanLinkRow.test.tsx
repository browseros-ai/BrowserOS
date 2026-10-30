import { describe, expect, it } from 'bun:test'
import type { Plan } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { usePlan } from '@/modules/api/plan.hooks'
import { PlanLinkRow } from './PlanLinkRow'

const DATE = new Date(2026, 10, 27, 12).getTime()

const planOf = (overrides: Partial<Plan> = {}): Plan => ({
  available: true,
  plan: 'pro',
  planName: 'neo Pro',
  features: [],
  status: 'none',
  periodEnd: DATE,
  endsAt: null,
  graceUntil: null,
  onKey: false,
  checkedAt: null,
  stale: false,
  ...overrides,
})

function render(plan?: Plan): Document {
  const client = new QueryClient()
  if (plan) client.setQueryData(usePlan.getKey(), plan)
  const html = renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <PlanLinkRow />
      </MemoryRouter>
    </QueryClientProvider>,
  )
  return parseHTML(`<!doctype html><html><body>${html}</body></html>`).document
}

const text = (page: Document) =>
  (page.body.textContent ?? '').replace(/\s/g, ' ')

describe('the Your plan row', () => {
  it('tests that Free points to Pro and to the key', () => {
    const page = render(planOf())
    expect(text(page)).toContain('Your plan')
    expect(text(page)).toContain('Free')
    expect(text(page)).toContain(
      'See what Pro adds, or use your licence key if you already paid.',
    )
    expect(page.querySelector('a')?.textContent).toContain('Open neo Pro')
    expect(page.querySelector('a')?.getAttribute('href')).toBe('/pro')
  })

  it('tests that a key whose Pro ended says so', () => {
    expect(text(render(planOf({ onKey: true })))).toContain(
      'Your licence key’s Pro has ended. Get Pro again on the neo Pro page.',
    )
  })

  it.each([
    [
      'active',
      {},
      'Active',
      'Next payment on Nov 27, 2026. Billing, receipts and cancelling are on the neo Pro page.',
    ],
    [
      'ending',
      { endsAt: DATE },
      'Ends Nov 27, 2026',
      'Pro stays on until Nov 27, 2026. You can keep it from the neo Pro page.',
    ],
    [
      'paymentFailed',
      {},
      'Payment failed',
      'Your last payment didn’t go through. Update your card on the neo Pro page.',
    ],
    [
      'unpaid',
      {},
      'Paused',
      'Pro is paused because your payments didn’t go through. Update your card on the neo Pro page.',
    ],
  ] as const)(
    'tests the %s row',
    (status, extra: Partial<Plan>, pill, line) => {
      const page = render(planOf({ status, ...extra }))
      expect(text(page)).toContain('neo Pro')
      expect(text(page)).toContain(pill)
      expect(text(page)).toContain(line)
    },
  )

  it('tests that a build without billing, or a plan not read yet, shows no row', () => {
    expect(render(planOf({ available: false })).body.innerHTML).toBe('')
    expect(render().body.innerHTML).toBe('')
  })
})
