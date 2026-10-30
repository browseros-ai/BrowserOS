import { describe, expect, it } from 'bun:test'
import type { Plan } from '@browseros/claw-api'
import {
  isNavItemActive,
  planTagFor,
  primaryNavItems,
  secondaryNavItemsFor,
} from './app-sidebar.data'
import { pageKeyFor } from './PageTransition'

describe('primaryNavItems', () => {
  it('lists Skills right after Home', () => {
    expect(primaryNavItems.map((item) => item.name)).toEqual([
      'Home',
      'Skills',
      'Connected agents',
      'History',
    ])
  })
})

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

describe('the neo Pro item', () => {
  const names = (plan?: Plan) =>
    secondaryNavItemsFor(plan).map((item) => item.name)

  it('tests that it sits before Settings, named for Free or Pro', () => {
    expect(names(planOf())).toEqual(['Get neo Pro', 'Settings'])
    expect(names(planOf({ status: 'active' }))).toEqual(['neo Pro', 'Settings'])
    expect(secondaryNavItemsFor(planOf())[0]?.to).toBe('/pro')
  })

  it('tests that it stays away without billing, or before the plan is read', () => {
    expect(names(planOf({ available: false }))).toEqual(['Settings'])
    expect(names(undefined)).toEqual(['Settings'])
  })

  it('tests the tag for every status', () => {
    const tag = (status: Plan['status']) =>
      planTagFor(planOf({ status }))?.label ?? null
    expect(tag('none')).toBeNull()
    expect(tag('active')).toBe('On')
    expect(tag('ending')).toBe('Ending')
    expect(tag('paymentFailed')).toBe('Check card')
    expect(tag('unpaid')).toBe('Paused')
    expect(
      planTagFor(planOf({ available: false, status: 'active' })),
    ).toBeNull()
  })
})

describe('isNavItemActive', () => {
  it('matches Home only on the root', () => {
    expect(isNavItemActive('/', '/')).toBe(true)
    expect(isNavItemActive('/', '/audit')).toBe(false)
  })

  it('matches a section and its nested pages, not look-alike paths', () => {
    expect(isNavItemActive('/audit', '/audit')).toBe(true)
    expect(isNavItemActive('/audit', '/audit/session-1/replay')).toBe(true)
    expect(isNavItemActive('/skills', '/skillset')).toBe(false)
  })
})

describe('pageKeyFor', () => {
  it('folds the replay deep link into its task page', () => {
    expect(pageKeyFor('/audit/session-1/replay')).toBe('/audit/session-1')
    expect(pageKeyFor('/audit/session-1')).toBe('/audit/session-1')
    expect(pageKeyFor('/settings')).toBe('/settings')
  })
})
