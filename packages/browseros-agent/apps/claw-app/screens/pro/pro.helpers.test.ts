import { describe, expect, it } from 'bun:test'
import type {
  Plan,
  PlanCheckoutStatus,
  PlanComputer,
  PlanPricing,
} from '@browseros/claw-api'
import { ApiResponseError } from '@browseros/claw-api-client'
import {
  billingPeriods,
  CHECKOUT_GIVE_UP_MS,
  checkoutPhase,
  computerName,
  formatAdded,
  formatPrice,
  keyFullComputers,
  keyHasEnded,
  keyLooksWhole,
  lastUsedLine,
  monthlyEquivalent,
  monthsFree,
  monthsFreeLabel,
  pickPeriod,
  proPrices,
} from './pro.helpers'

const spaces = (text: string) => text.replace(/\s/g, ' ')

const PLAN: Plan = {
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
}

function pricingOf(
  prices: PlanPricing['plans'][number]['prices'],
): PlanPricing {
  return {
    plans: [
      { plan: 'pro', name: 'neo Pro', description: null, features: [], prices },
    ],
  }
}

const MONTH = {
  interval: 'month' as const,
  intervalCount: 1,
  amount: 2000,
  currency: 'usd',
}
const YEAR = {
  interval: 'year' as const,
  intervalCount: 1,
  amount: 20000,
  currency: 'usd',
}

describe('prices', () => {
  it('tests that whole amounts drop their cents and others keep them', () => {
    expect(formatPrice({ amount: 2000, currency: 'usd' })).toBe('$20')
    expect(formatPrice({ amount: 1667, currency: 'usd' })).toBe('$16.67')
    expect(formatPrice({ amount: 900, currency: 'usd' })).toBe('$9')
  })

  it('tests that a currency without minor units is not divided', () => {
    expect(formatPrice({ amount: 3000, currency: 'jpy' })).toBe('¥3,000')
  })

  it('tests that a year shows its monthly cost and the months it saves', () => {
    const prices = proPrices(pricingOf([MONTH, YEAR]))
    if (!prices) throw new Error('no prices')
    expect(monthlyEquivalent(YEAR)).toBe('$16.67')
    expect(monthsFree(prices)).toBe(2)
    expect(monthsFreeLabel(2)).toBe('2 months free')
    expect(monthsFreeLabel(1)).toBe('1 month free')
  })

  it('tests that a year costing as much as twelve months saves nothing', () => {
    const prices = proPrices(
      pricingOf([MONTH, { ...YEAR, amount: MONTH.amount * 12 }]),
    )
    if (!prices) throw new Error('no prices')
    expect(monthsFree(prices)).toBeNull()
  })

  it('tests that only one-month and one-year prices are on offer', () => {
    const prices = proPrices(pricingOf([{ ...MONTH, intervalCount: 3 }, YEAR]))
    expect(prices?.month).toBeUndefined()
    expect(prices && billingPeriods(prices)).toEqual(['year'])
    expect(proPrices(pricingOf([{ ...MONTH, interval: 'week' }]))).toBeNull()
    expect(proPrices({ plans: [] })).toBeNull()
  })

  it('tests that the picked period falls back to one that is on sale', () => {
    const yearOnly = proPrices(pricingOf([YEAR]))
    const both = proPrices(pricingOf([MONTH, YEAR]))
    if (!yearOnly || !both) throw new Error('no prices')
    expect(pickPeriod(yearOnly, 'month')).toBe('year')
    expect(pickPeriod(both, 'year')).toBe('year')
  })
})

describe('the plan', () => {
  it('tests that only a key with no Pro reads as ended', () => {
    expect(keyHasEnded({ ...PLAN, onKey: true })).toBe(true)
    expect(keyHasEnded(PLAN)).toBe(false)
    expect(keyHasEnded({ ...PLAN, onKey: true, status: 'active' })).toBe(false)
  })
})

describe('checkoutPhase', () => {
  const started = { started: true, cancelled: false, startedAt: 1000 }
  const status = (
    phase: PlanCheckoutStatus['status'],
    plan: Plan | null = null,
  ): PlanCheckoutStatus => ({ status: phase, plan })

  it('tests every outcome of a checkout', () => {
    expect(
      checkoutPhase({ ...started, started: false, lastCheckedAt: 0 }),
    ).toBe('idle')
    expect(checkoutPhase({ ...started, lastCheckedAt: 0 })).toBe('waiting')
    expect(
      checkoutPhase({ ...started, status: status('open'), lastCheckedAt: 2 }),
    ).toBe('waiting')
    expect(
      checkoutPhase({
        ...started,
        status: status('complete'),
        lastCheckedAt: 2,
      }),
    ).toBe('confirming')
    expect(
      checkoutPhase({
        ...started,
        status: status('complete', { ...PLAN, status: 'active' }),
        lastCheckedAt: 2,
      }),
    ).toBe('complete')
    expect(
      checkoutPhase({
        ...started,
        status: status('expired'),
        lastCheckedAt: 2,
      }),
    ).toBe('abandoned')
    expect(
      checkoutPhase({ ...started, cancelled: true, lastCheckedAt: 2 }),
    ).toBe('abandoned')
  })

  it('tests that neo stops waiting after 30 minutes', () => {
    const at = (elapsed: number) =>
      checkoutPhase({
        ...started,
        status: status('open'),
        lastCheckedAt: started.startedAt + elapsed,
      })
    expect(at(CHECKOUT_GIVE_UP_MS)).toBe('waiting')
    expect(at(CHECKOUT_GIVE_UP_MS + 1)).toBe('abandoned')
  })
})

describe('the licence key', () => {
  it('tests that a whole key looks whole however it is grouped', () => {
    expect(keyLooksWhole('NEO-7K2QM-R4X9B-T6WZP-3HDVN-8CJYF')).toBe(true)
    expect(keyLooksWhole(' neo-7k2qm-r4x9b-t6wzp-3hdvn-8cjyf ')).toBe(true)
    expect(keyLooksWhole('NEO7K2QMR4X9BT6WZP3HDVN8CJYF')).toBe(true)
  })

  it('tests that part of a key, or something else, does not', () => {
    expect(keyLooksWhole('NEO-7K2QM-R4X9B')).toBe(false)
    expect(keyLooksWhole('ABC-7K2QM-R4X9B-T6WZP-3HDVN-8CJYF')).toBe(false)
    expect(keyLooksWhole('')).toBe(false)
  })

  it('tests that a full key lists its computers and nothing else does', () => {
    const computer: PlanComputer = {
      installId: 'a',
      label: 'Work iMac',
      platform: 'macos',
      lastSeenAt: 1,
      activatedAt: 1,
      thisComputer: false,
    }
    const full = new ApiResponseError(new Response(null, { status: 409 }), {
      code: 'key_full',
      message: 'This key is already on 5 computers.',
      computers: [computer, { installId: 3 }],
    })
    const invalid = new ApiResponseError(new Response(null, { status: 400 }), {
      code: 'key_invalid',
      message: 'That key did not work.',
    })
    expect(keyFullComputers(full)).toEqual([computer])
    expect(keyFullComputers(invalid)).toEqual([])
    expect(keyFullComputers(new Error('offline'))).toEqual([])
  })
})

describe('computers', () => {
  const computer: PlanComputer = {
    installId: 'a',
    label: null,
    platform: 'linux',
    lastSeenAt: 0,
    activatedAt: new Date(2026, 9, 27, 9, 41).getTime(),
    thisComputer: false,
  }

  it('tests that the added time is exact', () => {
    expect(spaces(formatAdded(computer.activatedAt))).toBe(
      'Oct 27, 2026, 9:41 AM',
    )
  })

  it('tests that last use counts calendar days, weeks and months', () => {
    const now = new Date(2026, 9, 30, 8, 0).getTime()
    const daysAgo = (days: number) =>
      new Date(2026, 9, 30 - days, 23, 0).getTime()
    expect(lastUsedLine(new Date(2026, 9, 30, 7, 0).getTime(), now)).toBe(
      'Last used today',
    )
    expect(lastUsedLine(daysAgo(1), now)).toBe('Last used yesterday')
    expect(lastUsedLine(daysAgo(3), now)).toBe('Last used 3 days ago')
    expect(lastUsedLine(daysAgo(14), now)).toBe('Last used 2 weeks ago')
    expect(lastUsedLine(daysAgo(150), now)).toBe('Last used 5 months ago')
  })

  it('tests that a computer with no name is called by its system', () => {
    expect(computerName(computer)).toBe('Linux')
    expect(computerName({ ...computer, label: '  ' })).toBe('Linux')
    expect(computerName({ ...computer, label: 'Studio' })).toBe('Studio')
  })
})
