import { afterEach, describe, expect, it } from 'bun:test'
import { PostHog } from 'posthog-js'
import {
  createPostHogConfig,
  maskCapturedReplayRequest,
  reconcileSessionRecording,
  reconcileTelemetryIdentity,
  sanitizeProperties,
} from './posthog'

const originalChrome = globalThis.chrome

afterEach(() => {
  Object.defineProperty(globalThis, 'chrome', {
    configurable: true,
    value: originalChrome,
  })
})

describe('BrowserClaw PostHog privacy', () => {
  it('keeps the installed app version when switching anonymous identity', async () => {
    Object.defineProperty(globalThis, 'chrome', {
      configurable: true,
      value: {
        runtime: { getManifest: () => ({ version: '0.2.25.7' }) },
      },
    })
    const events: Array<{
      event: string
      properties: Record<string, unknown>
    }> = []
    const client = new PostHog()
    client.init('test-project-key', {
      ...createPostHogConfig('installation-B'),
      persistence: 'memory',
      advanced_disable_decide: true,
      before_send: (event) => {
        if (event) events.push(event)
        return null // Inspect the actual SDK payload without any network delivery.
      },
    })
    // A prior build's persisted properties must not override this package.
    client.register({ app_version: '0.2.24.0' })
    client.capture('before-migration', { app_version: 'caller-override' })
    reconcileTelemetryIdentity(client, 'analytics-A')
    client.opt_in_capturing({ captureEventName: false })
    client.capture('after-migration')

    expect(client.get_distinct_id()).toBe('analytics-A')
    expect(events.map(({ event }) => event)).toEqual([
      'before-migration',
      'after-migration',
    ])
    expect(events.map(({ properties }) => properties.distinct_id)).toEqual([
      'installation-B',
      'analytics-A',
    ])
    expect(events.map(({ properties }) => properties.app_version)).toEqual([
      '0.2.25.7',
      '0.2.25.7',
    ])
    expect(
      events.every(
        ({ properties }) => properties.$process_person_profile === false,
      ),
    ).toBe(true)
    const sessionId = client.get_session_id()
    reconcileTelemetryIdentity(client, 'analytics-A')
    expect(client.get_session_id()).toBe(sessionId)
    await client.shutdown()
  })

  it('configures sampled replay with conservative capture boundaries', () => {
    const config = createPostHogConfig('anonymous-install-id')

    expect(config.bootstrap).toEqual({
      distinctID: 'anonymous-install-id',
    })
    expect(config.advanced_disable_decide).toBeUndefined()
    expect(config.advanced_disable_feature_flags_on_first_load).toBe(true)
    expect(config.remote_config_refresh_interval_ms).toBe(0)
    expect(config.save_campaign_params).toBe(false)
    expect(config.save_referrer).toBe(false)
    expect(config.disable_capture_url_hashes).toBe(true)
    expect(config.disable_external_dependency_loading).toBe(true)
    expect(config.enable_recording_console_log).toBe(false)
    expect(config.disable_session_recording).toBe(true)
    expect(config.session_recording).toEqual({
      blockClass: 'ph-no-capture',
      collectFonts: false,
      maskAllInputs: true,
      maskCapturedNetworkRequestFn: maskCapturedReplayRequest,
      recordBody: false,
      recordCrossOriginIframes: false,
      recordHeaders: false,
      sampleRate: 0.2,
    })
    expect(
      maskCapturedReplayRequest({
        name: 'chrome-extension://private/newtab.html#/audit/session-secret',
        initiatorType: 'fetch',
      }),
    ).toEqual({
      name: 'browserclaw://redacted',
      initiatorType: 'fetch',
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
        app_version: 'stale-without-extension-runtime',
        screen: 'cockpit',
      }),
    ).toEqual({ screen: 'cockpit' })
  })

  it('starts sampled recording on opt-in and stops before capture opt-out', () => {
    const calls: string[] = []
    const client = {
      opt_in_capturing: (options?: { captureEventName?: false }) =>
        calls.push(`opt-in:${String(options?.captureEventName)}`),
      opt_out_capturing: () => calls.push('opt-out'),
      startSessionRecording: (override?: unknown) =>
        calls.push(`start:${String(override)}`),
      stopSessionRecording: () => calls.push('stop'),
    }

    reconcileSessionRecording(client, true, false)
    expect(calls).toEqual(['opt-in:false', 'start:undefined'])

    calls.length = 0
    reconcileSessionRecording(client, true, true)
    expect(calls).toEqual(['opt-in:false', 'start:undefined'])

    calls.length = 0
    reconcileSessionRecording(client, false, true)
    expect(calls).toEqual(['stop', 'opt-out'])
  })
})
