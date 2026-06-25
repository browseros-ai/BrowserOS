import { describe, expect, it } from 'bun:test'
import {
  adapterHealthLabel,
  adapterHealthMeta,
  adapterHealthTone,
} from './adapter-health'

describe('adapter health helpers', () => {
  it('labels actionable readiness states', () => {
    expect(
      adapterHealthLabel({
        healthy: false,
        checkedAt: 1,
        readiness: 'needs-auth',
      }),
    ).toBe('Login needed')
    expect(
      adapterHealthLabel({
        healthy: false,
        checkedAt: 1,
        readiness: 'will-fetch-package',
      }),
    ).toBe('Fetch on first run')
  })

  it('keeps install/auth blockers visually stronger than fetch warnings', () => {
    expect(
      adapterHealthTone({
        healthy: false,
        checkedAt: 1,
        readiness: 'needs-install',
      }),
    ).toBe('danger')
    expect(
      adapterHealthTone({
        healthy: false,
        checkedAt: 1,
        readiness: 'will-fetch-package',
      }),
    ).toBe('warning')
    expect(
      adapterHealthTone({
        healthy: true,
        checkedAt: 1,
        readiness: 'diagnostic-warning',
      }),
    ).toBe('warning')
  })

  it('summarizes version and launch source when known', () => {
    expect(
      adapterHealthMeta({
        healthy: true,
        checkedAt: 1,
        version: 'claude 1.2.3',
        adapterLaunchSource: 'host-npx',
      }),
    ).toBe('claude 1.2.3 · npx')
    expect(
      adapterHealthMeta({
        healthy: true,
        checkedAt: 1,
        version: 'custom 0.9.0',
        adapterLaunchSource: 'host-cli',
      }),
    ).toBe('custom 0.9.0 · host CLI')
  })
})
