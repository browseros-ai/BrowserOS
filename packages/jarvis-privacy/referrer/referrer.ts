// JarvisOS Referrer Trimming — ported from Brave browser referrer policy

export enum ReferrerPolicy {
  NoReferrer = "no-referrer",
  Origin = "origin",
  StrictOriginWhenCrossOrigin = "strict-origin-when-cross-origin",
  SameOrigin = "same-origin",
}

export interface ReferrerTrimConfig {
  enabled: boolean
  policy: ReferrerPolicy
  allowSameOriginFull: boolean
}

export const DEFAULT_REFERRER_CONFIG: ReferrerTrimConfig = {
  enabled: true,
  policy: ReferrerPolicy.StrictOriginWhenCrossOrigin,
  allowSameOriginFull: true,
}

/**
 * Trims a referrer URL according to the configured policy.
 * Mirrors Brave's referrer trimming logic from brave_shields_util.h.
 */
export function trimReferrer(
  referrer: string,
  destination: string,
  config: ReferrerTrimConfig = DEFAULT_REFERRER_CONFIG,
): string {
  if (!config.enabled) return referrer

  try {
    const referrerUrl = new URL(referrer)
    const destUrl = new URL(destination)
    const isSameOrigin = referrerUrl.origin === destUrl.origin

    if (isSameOrigin && config.allowSameOriginFull) return referrer

    switch (config.policy) {
      case ReferrerPolicy.NoReferrer:
        return ''
      case ReferrerPolicy.Origin:
        return referrerUrl.origin + '/'
      case ReferrerPolicy.SameOrigin:
        return isSameOrigin ? referrer : ''
      case ReferrerPolicy.StrictOriginWhenCrossOrigin:
        if (isSameOrigin) return referrer
        if (referrerUrl.protocol === 'https:' && destUrl.protocol === 'http:') return ''
        return referrerUrl.origin + '/'
      default:
        return referrerUrl.origin + '/'
    }
  } catch {
    return ''
  }
}
