import { env } from '../env'
import { BrowserOSAdapter } from './adapter'

const SERVER_VERSION_PREF = 'browseros.server.version'

type FeatureConfig = {
  minBrowserOSVersion?: string
  maxBrowserOSVersion?: string
  minServerVersion?: string
  maxServerVersion?: string
  requiresAlphaFlag?: boolean
  requiresDevelopmentFlag?: boolean
}

/**
 * Features gated by BrowserOS version or explicit environment flags.
 * Add new features here with corresponding config in FEATURE_CONFIG.
 *
 * Note: In development mode, all features are enabled regardless of version
 * or alpha flag. Development-only gates resolve false outside development.
 * @public
 */
export enum Feature {
  // Unfinished UI surfaces behind an explicit alpha opt-in
  ALPHA_FEATURES_SUPPORT = 'ALPHA_FEATURES_SUPPORT',
  // Inline chat in the new tab page
  NEWTAB_CHAT_SUPPORT = 'NEWTAB_CHAT_SUPPORT',
  // Vertical tabs preference and customization
  VERTICAL_TABS_SUPPORT = 'VERTICAL_TABS_SUPPORT',
  // ChatGPT Pro OAuth LLM provider
  CHATGPT_PRO_SUPPORT = 'CHATGPT_PRO_SUPPORT',
  // GitHub Copilot OAuth LLM provider
  GITHUB_COPILOT_SUPPORT = 'GITHUB_COPILOT_SUPPORT',
  // Qwen Code OAuth LLM provider
  QWEN_CODE_SUPPORT = 'QWEN_CODE_SUPPORT',
  // Credit-based usage tracking
  CREDITS_SUPPORT = 'CREDITS_SUPPORT',
  // Claude Code / Codex agent-harness adapters in the unified picker + settings
  AGENT_HARNESS_SUPPORT = 'AGENT_HARNESS_SUPPORT',
  // Remote Hermes provider
  HERMES_AGENT_SUPPORT = 'HERMES_AGENT_SUPPORT',
}

/**
 * Version requirements for each feature.
 * - minBrowserOSVersion: feature enabled when BrowserOS >= this version
 * - maxBrowserOSVersion: feature enabled when BrowserOS < this version (for deprecation)
 * - minServerVersion: feature enabled when server >= this version
 * - maxServerVersion: feature enabled when server < this version (for deprecation)
 *
 * TypeScript enforces that every Feature has a config entry.
 * In development mode, all features are enabled regardless of version or
 * alpha flag.
 */
const FEATURE_CONFIG: { [K in Feature]: FeatureConfig } = {
  [Feature.ALPHA_FEATURES_SUPPORT]: { requiresAlphaFlag: true },
  [Feature.NEWTAB_CHAT_SUPPORT]: { minBrowserOSVersion: '0.40.0.0' },
  [Feature.VERTICAL_TABS_SUPPORT]: { minBrowserOSVersion: '0.42.0.0' },
  [Feature.CHATGPT_PRO_SUPPORT]: { minServerVersion: '0.0.77' },
  [Feature.GITHUB_COPILOT_SUPPORT]: { minServerVersion: '0.0.77' },
  [Feature.QWEN_CODE_SUPPORT]: { minServerVersion: '0.0.77' },
  [Feature.CREDITS_SUPPORT]: { minServerVersion: '0.0.78' },
  [Feature.AGENT_HARNESS_SUPPORT]: { minBrowserOSVersion: '0.46.0.0' },
  [Feature.HERMES_AGENT_SUPPORT]: {
    requiresAlphaFlag: true,
    minServerVersion: '0.0.116',
  },
}

function hasVersionConstraints(config: FeatureConfig): boolean {
  return Boolean(
    config.minBrowserOSVersion ||
      config.maxBrowserOSVersion ||
      config.minServerVersion ||
      config.maxServerVersion,
  )
}

function parseVersion(version: string): number[] {
  const parts = version.split('.').map(Number)
  if (parts.length < 2 || parts.some(Number.isNaN)) {
    throw new Error(`Invalid version format: ${version}`)
  }
  return parts
}

function compareVersions(a: number[], b: number[]): number {
  const maxLen = Math.max(a.length, b.length)
  for (let i = 0; i < maxLen; i++) {
    const aVal = a[i] ?? 0
    const bVal = b[i] ?? 0
    if (aVal < bVal) return -1
    if (aVal > bVal) return 1
  }
  return 0
}

function checkVersionConstraints(
  version: number[] | null,
  minVersionStr?: string,
  maxVersionStr?: string,
): boolean {
  if (!version) return false
  if (
    minVersionStr &&
    compareVersions(version, parseVersion(minVersionStr)) < 0
  )
    return false
  if (
    maxVersionStr &&
    compareVersions(version, parseVersion(maxVersionStr)) >= 0
  )
    return false
  return true
}

/** Resolves static environment gates before version checks. */
export function resolveStaticFeatureSupport({
  isDevelopment,
  alphaFeaturesEnabled,
  requiresDevelopmentFlag = false,
  requiresAlphaFlag = false,
}: {
  isDevelopment: boolean
  alphaFeaturesEnabled: boolean
  requiresDevelopmentFlag?: boolean
  requiresAlphaFlag?: boolean
}): boolean | null {
  if (requiresDevelopmentFlag) {
    return isDevelopment
  }
  if (isDevelopment) {
    return true
  }
  if (requiresAlphaFlag) {
    return alphaFeaturesEnabled
  }
  return null
}

/** Applies static gates, falling through when version gates still need evaluation. */
export function resolveFeatureStaticSupport({
  feature,
  isDevelopment,
  alphaFeaturesEnabled,
}: {
  feature: Feature
  isDevelopment: boolean
  alphaFeaturesEnabled: boolean
}): boolean | null {
  const config = FEATURE_CONFIG[feature]
  if (!config) return false
  const staticSupport = resolveStaticFeatureSupport({
    isDevelopment,
    alphaFeaturesEnabled,
    requiresDevelopmentFlag: config.requiresDevelopmentFlag,
    requiresAlphaFlag: config.requiresAlphaFlag,
  })
  if (staticSupport !== true) return staticSupport
  if (hasVersionConstraints(config) && !isDevelopment) return null
  return true
}

export type CapabilitiesState = {
  browserOSVersion: number[] | null
  serverVersion: number[] | null
}

let initPromise: Promise<CapabilitiesState> | null = null

function getStaticFeatureSupport(feature: Feature): boolean | null {
  return resolveFeatureStaticSupport({
    feature,
    isDevelopment: import.meta.env.DEV,
    alphaFeaturesEnabled: env.VITE_ALPHA_FEATURES,
  })
}

async function doInitialize(): Promise<CapabilitiesState> {
  const adapter = BrowserOSAdapter.getInstance()
  const state: CapabilitiesState = {
    browserOSVersion: null,
    serverVersion: null,
  }

  try {
    const versionStr = await adapter.getBrowserosVersion()
    if (versionStr) {
      state.browserOSVersion = parseVersion(versionStr)
    }
  } catch {
    // BrowserOS version unknown - features requiring it will be disabled
  }

  try {
    const pref = await adapter.getPref(SERVER_VERSION_PREF)
    if (pref?.value) {
      state.serverVersion = parseVersion(pref.value)
    }
  } catch {
    // Server version unknown - features requiring it will be disabled
  }

  return state
}

function ensureInitialized(): Promise<CapabilitiesState> {
  if (!initPromise) {
    initPromise = doInitialize()
  }
  return initPromise
}

// Exported for unit tests: resolves a feature's version gate directly,
// bypassing the dev-mode/static short-circuit in `supports`.
export function checkFeatureSupport(
  state: CapabilitiesState,
  feature: Feature,
): boolean {
  const config = FEATURE_CONFIG[feature]
  if (!config) return false

  const hasBrowserOSConstraints =
    config.minBrowserOSVersion || config.maxBrowserOSVersion
  if (
    hasBrowserOSConstraints &&
    !checkVersionConstraints(
      state.browserOSVersion,
      config.minBrowserOSVersion,
      config.maxBrowserOSVersion,
    )
  ) {
    return false
  }

  const hasServerConstraints =
    config.minServerVersion || config.maxServerVersion
  if (
    hasServerConstraints &&
    !checkVersionConstraints(
      state.serverVersion,
      config.minServerVersion,
      config.maxServerVersion,
    )
  ) {
    return false
  }

  return true
}

/**
 * Version-gated feature capabilities.
 * All methods auto-initialize and are safe to call at any time.
 * @public
 */
export const Capabilities = {
  getStaticSupport(feature: Feature): boolean | null {
    return getStaticFeatureSupport(feature)
  },

  /**
   * Check if a feature is supported.
   * In development mode, all features are enabled.
   */
  async supports(feature: Feature): Promise<boolean> {
    const staticSupport = getStaticFeatureSupport(feature)
    if (staticSupport !== null) return staticSupport
    const state = await ensureInitialized()
    return checkFeatureSupport(state, feature)
  },

  async getBrowserOSVersion(): Promise<string | null> {
    const state = await ensureInitialized()
    if (!state.browserOSVersion) return null
    return state.browserOSVersion.join('.')
  },

  async getServerVersion(): Promise<string | null> {
    const state = await ensureInitialized()
    if (!state.serverVersion) return null
    return state.serverVersion.join('.')
  },

  /**
   * Pre-initialize capabilities. Optional - methods auto-initialize if needed.
   * Useful for warming up before first use.
   */
  async initialize(): Promise<void> {
    await ensureInitialized()
  },

  /**
   * Reset state for testing purposes.
   */
  reset(): void {
    initPromise = null
  },
}

// Pre-initialize when module is imported
ensureInitialized()
