import { describe, expect, it } from 'bun:test'
import {
  checkFeatureSupport,
  Feature,
  resolveFeatureStaticSupport,
  resolveStaticFeatureSupport,
} from './capabilities'

describe('resolveStaticFeatureSupport', () => {
  it('enables alpha-gated features automatically in development', () => {
    expect(
      resolveStaticFeatureSupport({
        isDevelopment: true,
        alphaFeaturesEnabled: false,
        requiresAlphaFlag: true,
      }),
    ).toBe(true)
  })

  it('enables alpha-gated features only when explicitly opted in', () => {
    expect(
      resolveStaticFeatureSupport({
        isDevelopment: false,
        alphaFeaturesEnabled: true,
        requiresAlphaFlag: true,
      }),
    ).toBe(true)
  })

  it('keeps non-alpha features enabled in development', () => {
    expect(
      resolveStaticFeatureSupport({
        isDevelopment: true,
        alphaFeaturesEnabled: false,
      }),
    ).toBe(true)
  })

  it('leaves non-alpha features unresolved in production', () => {
    expect(
      resolveStaticFeatureSupport({
        isDevelopment: false,
        alphaFeaturesEnabled: false,
      }),
    ).toBeNull()
  })

  it('enables development-only features in development', () => {
    expect(
      resolveStaticFeatureSupport({
        isDevelopment: true,
        alphaFeaturesEnabled: false,
        requiresDevelopmentFlag: true,
      }),
    ).toBe(true)
  })

  it('disables development-only features outside development', () => {
    expect(
      resolveStaticFeatureSupport({
        isDevelopment: false,
        alphaFeaturesEnabled: true,
        requiresDevelopmentFlag: true,
      }),
    ).toBe(false)
  })
})

describe('resolveFeatureStaticSupport', () => {
  it('gates voice input on alpha outside development', () => {
    expect(
      resolveFeatureStaticSupport({
        feature: Feature.VOICE_INPUT_SUPPORT,
        isDevelopment: true,
        alphaFeaturesEnabled: false,
      }),
    ).toBe(true)

    expect(
      resolveFeatureStaticSupport({
        feature: Feature.VOICE_INPUT_SUPPORT,
        isDevelopment: false,
        alphaFeaturesEnabled: false,
      }),
    ).toBe(false)

    expect(
      resolveFeatureStaticSupport({
        feature: Feature.VOICE_INPUT_SUPPORT,
        isDevelopment: false,
        alphaFeaturesEnabled: true,
      }),
    ).toBe(true)
  })
  it('preserves alpha-gated support for alpha features', () => {
    expect(
      resolveFeatureStaticSupport({
        feature: Feature.ALPHA_FEATURES_SUPPORT,
        isDevelopment: false,
        alphaFeaturesEnabled: false,
      }),
    ).toBe(false)

    expect(
      resolveFeatureStaticSupport({
        feature: Feature.ALPHA_FEATURES_SUPPORT,
        isDevelopment: false,
        alphaFeaturesEnabled: true,
      }),
    ).toBe(true)
  })
})

describe('checkFeatureSupport — AGENT_HARNESS_SUPPORT', () => {
  const at = (browserOSVersion: number[] | null) =>
    checkFeatureSupport(
      { browserOSVersion, serverVersion: null },
      Feature.AGENT_HARNESS_SUPPORT,
    )

  it('hides harness agents below BrowserOS 0.46.0.0 or when version is unknown', () => {
    expect(at([0, 45, 9, 9])).toBe(false)
    expect(at([0, 45, 0, 0])).toBe(false)
    expect(at(null)).toBe(false)
  })

  it('shows harness agents at or above BrowserOS 0.46.0.0', () => {
    expect(at([0, 46, 0, 0])).toBe(true)
    expect(at([0, 47, 0, 0])).toBe(true)
  })
})
