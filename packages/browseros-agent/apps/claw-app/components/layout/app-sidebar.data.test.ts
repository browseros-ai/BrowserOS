import { describe, expect, it } from 'bun:test'
import { isNavItemActive, primaryNavItems } from './app-sidebar.data'
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
