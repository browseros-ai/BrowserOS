import { describe, expect, it } from 'bun:test'
import { shouldAutoOpenDashboard } from '../../src/dashboard/server'

describe('dashboard server', () => {
  it('does not auto-open the dashboard in CI', () => {
    expect(shouldAutoOpenDashboard({ CI: 'true' })).toBe(false)
  })

  it('auto-opens the dashboard outside CI by default', () => {
    expect(shouldAutoOpenDashboard({})).toBe(true)
  })
})
