import { describe, expect, it } from 'bun:test'
import type { Plan } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { usePlan } from '@/modules/api/plan.hooks'
import { usePromoSlot } from './PromoSlot'

const planOf = (overrides: Partial<Plan> = {}): Plan => ({
  available: true,
  plan: null,
  planName: null,
  features: [],
  status: 'none',
  periodEnd: null,
  endsAt: null,
  graceUntil: null,
  onKey: false,
  checkedAt: null,
  stale: false,
  ...overrides,
})

function Shown() {
  return <output>{usePromoSlot().kind ?? 'nothing'}</output>
}

function shown(plan: Plan): string {
  const client = new QueryClient()
  client.setQueryData(usePlan.getKey(), plan)
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Shown />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('the Home promotion', () => {
  it('tests that Free sees the ultrafast bar while ultrafast is on sale', () => {
    expect(shown(planOf())).toBe('<output>pro</output>')
  })

  it('tests that the ultrafast bar is gone while ultrafast is not on sale', () => {
    expect(shown(planOf({ available: false }))).toBe('<output>nothing</output>')
  })
})
