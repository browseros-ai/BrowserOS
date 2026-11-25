import { describe, expect, it } from 'bun:test'
import type { Plan } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { usePlan } from '@/modules/api/plan.hooks'
import { usePromoSlot } from './PromoSlot'

const DAY = 86_400_000

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
  it('tests that the last 3 days of a card trial show the charge date first', () => {
    const trial = (overrides: Partial<Plan>) =>
      shown(
        planOf({
          status: 'active',
          plan: 'ultrafast',
          features: ['claw.ultrafast'],
          trial: true,
          trialKind: 'card',
          trialEndsAt: Date.now() + 2 * DAY,
          ...overrides,
        }),
      )
    expect(trial({})).toBe('<output>trial-ending</output>')
    expect(trial({ trialEndsAt: Date.now() + 5 * DAY })).toBe(
      '<output>nothing</output>',
    )
    expect(
      trial({ status: 'ending' }),
      'a cancelled trial won’t be charged',
    ).toBe('<output>nothing</output>')
    expect(
      trial({ trialKind: 'grant' }),
      'a free trial link is a paid plan',
    ).toBe('<output>nothing</output>')
  })
})
