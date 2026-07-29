import { describe, expect, it } from 'bun:test'
import {
  createPostHogConfig,
  reconcileSessionRecording,
  sanitizeProperties,
} from './posthog'

describe('BrowserClaw PostHog privacy', () => {
  it('configures sampled replay with conservative capture boundaries', () => {
    const config = createPostHogConfig('anonymous-install-id')

    expect(config.bootstrap).toEqual({
      distinctID: 'anonymous-install-id',
    })
    expect(config.advanced_disable_decide).toBeUndefined()
    expect(config.advanced_disable_feature_flags_on_first_load).toBe(true)
    expect(config.disable_external_dependency_loading).toBe(true)
    expect(config.disable_session_recording).toBe(true)
    expect(config.session_recording).toEqual({
      blockClass: 'ph-no-capture',
      collectFonts: false,
      maskAllInputs: true,
      recordBody: false,
      recordCrossOriginIframes: false,
      recordHeaders: false,
      sampleRate: 0.2,
    })
  })

  it('strips location and referrer properties without changing safe fields', () => {
    expect(
      sanitizeProperties({
        $current_url: 'chrome-extension://secret/newtab.html#/audit',
        $pathname: '/audit',
        $host: 'secret',
        $referrer: 'https://private.example',
        $referring_domain: 'private.example',
        $initial_current_url: 'chrome-extension://secret/newtab.html',
        $initial_pathname: '/',
        $initial_referrer: 'https://private.example',
        $initial_referring_domain: 'private.example',
        screen: 'cockpit',
      }),
    ).toEqual({ screen: 'cockpit' })
  })

  it('starts sampled recording on opt-in and stops before capture opt-out', () => {
    const calls: string[] = []
    const client = {
      opt_in_capturing: () => calls.push('opt-in'),
      opt_out_capturing: () => calls.push('opt-out'),
      startSessionRecording: (override?: unknown) =>
        calls.push(`start:${String(override)}`),
      stopSessionRecording: () => calls.push('stop'),
    }

    reconcileSessionRecording(client, true, true)
    expect(calls).toEqual(['opt-in', 'start:undefined'])

    calls.length = 0
    reconcileSessionRecording(client, false, true)
    expect(calls).toEqual(['stop', 'opt-out'])
  })
})
