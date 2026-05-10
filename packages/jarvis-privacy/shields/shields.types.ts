// JarvisOS Shields — TypeScript types and enums ported from Brave Shields

export enum FingerprintMode {
  Disabled = "disabled",
  Standard = "standard",
  Strict = "strict",
}

export enum CookieMode {
  Allow = "allow",
  CrossSiteBlocked = "cross_site_blocked",
  AllBlocked = "all_blocked",
}

export enum AdblockMode {
  Disabled = "disabled",
  Standard = "standard",
  Aggressive = "aggressive",
}

export enum HttpsUpgradeMode {
  Disabled = "disabled",
  Standard = "standard",
  Strict = "strict",
}

export interface ShieldsConfig {
  enabled: boolean
  adblock: AdblockMode
  trackerBlock: boolean
  fingerprintProtection: FingerprintMode
  cookieProtection: CookieMode
  referrerTrimming: boolean
  bounceTrackingProtection: boolean
  httpsUpgrade: HttpsUpgradeMode
  torAvailable: boolean
}

export interface SiteShieldsOverride {
  origin: string
  overrides: Partial<ShieldsConfig>
}

export const DEFAULT_SHIELDS_CONFIG: ShieldsConfig = {
  enabled: true,
  adblock: AdblockMode.Standard,
  trackerBlock: true,
  fingerprintProtection: FingerprintMode.Standard,
  cookieProtection: CookieMode.CrossSiteBlocked,
  referrerTrimming: true,
  bounceTrackingProtection: true,
  httpsUpgrade: HttpsUpgradeMode.Standard,
  torAvailable: true,
}
