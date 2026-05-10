import { useState, useCallback } from 'react'
import type { ShieldsConfig, SiteShieldsOverride } from '@jarvisos/privacy/shields/shields.types'
import { DEFAULT_SHIELDS_CONFIG } from '@jarvisos/privacy/shields/shields.types'

interface UseJarvisShieldsReturn {
  config: ShieldsConfig
  siteOverrides: SiteShieldsOverride[]
  updateConfig: (patch: Partial<ShieldsConfig>) => void
  resetToDefaults: () => void
  getSiteConfig: (origin: string) => ShieldsConfig
  setSiteOverride: (origin: string, patch: Partial<ShieldsConfig>) => void
  removeSiteOverride: (origin: string) => void
}

export function useJarvisShields(): UseJarvisShieldsReturn {
  const [config, setConfig] = useState<ShieldsConfig>(DEFAULT_SHIELDS_CONFIG)
  const [siteOverrides, setSiteOverrides] = useState<SiteShieldsOverride[]>([])

  const updateConfig = useCallback((patch: Partial<ShieldsConfig>) => {
    setConfig(prev => ({ ...prev, ...patch }))
  }, [])

  const resetToDefaults = useCallback(() => {
    setConfig(DEFAULT_SHIELDS_CONFIG)
  }, [])

  const getSiteConfig = useCallback((origin: string): ShieldsConfig => {
    const override = siteOverrides.find(o => o.origin === origin)
    return override ? { ...config, ...override.overrides } : config
  }, [config, siteOverrides])

  const setSiteOverride = useCallback((origin: string, patch: Partial<ShieldsConfig>) => {
    setSiteOverrides(prev => {
      const existing = prev.findIndex(o => o.origin === origin)
      if (existing >= 0) {
        return prev.map((o, i) => i === existing ? { ...o, overrides: { ...o.overrides, ...patch } } : o)
      }
      return [...prev, { origin, overrides: patch }]
    })
  }, [])

  const removeSiteOverride = useCallback((origin: string) => {
    setSiteOverrides(prev => prev.filter(o => o.origin !== origin))
  }, [])

  return { config, siteOverrides, updateConfig, resetToDefaults, getSiteConfig, setSiteOverride, removeSiteOverride }
}
