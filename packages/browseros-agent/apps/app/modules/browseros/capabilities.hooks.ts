import { useCallback, useEffect, useState } from 'react'
import { Capabilities, Feature } from '@/lib/browseros/capabilities'

interface CapabilitiesState {
  browserOSVersion: string | null
  serverVersion: string | null
  supportedFeatures: Map<Feature, boolean>
}

export interface UseCapabilitiesResult {
  supports: (feature: Feature) => boolean
  isLoading: boolean
  browserOSVersion: string | null
  serverVersion: string | null
}

function getInitialSupportedFeatures(): Map<Feature, boolean> {
  return new Map(
    Object.values(Feature)
      .filter((value) => typeof value === 'string')
      .flatMap((feature) => {
        const supported = Capabilities.getStaticSupport(feature as Feature)
        return supported === null
          ? []
          : ([[feature as Feature, supported]] as const)
      }),
  )
}

/**
 * React hook for version-gated feature checks.
 * Auto-initializes Capabilities and caches feature support results.
 *
 * @example
 * const { supports, isLoading } = useCapabilities()
 *
 * if (isLoading) return <Spinner />
 * if (supports(Feature.NEW_SIDEBAR)) return <NewSidebar />
 *
 * @public
 */
export function useCapabilities(): UseCapabilitiesResult {
  const [isLoading, setIsLoading] = useState(true)
  const [state, setState] = useState<CapabilitiesState>(() => ({
    browserOSVersion: null,
    serverVersion: null,
    supportedFeatures: getInitialSupportedFeatures(),
  }))

  useEffect(() => {
    let cancelled = false

    async function init() {
      const [browserOSVersion, serverVersion] = await Promise.all([
        Capabilities.getBrowserOSVersion(),
        Capabilities.getServerVersion(),
      ])

      // Pre-check all features
      const featureChecks = await Promise.all(
        Object.values(Feature)
          .filter((v) => typeof v === 'string')
          .map(async (feature) => {
            const supported = await Capabilities.supports(feature as Feature)
            return [feature as Feature, supported] as const
          }),
      )

      if (!cancelled) {
        setState({
          browserOSVersion,
          serverVersion,
          supportedFeatures: new Map(featureChecks),
        })
        setIsLoading(false)
      }
    }

    init()

    return () => {
      cancelled = true
    }
  }, [])

  const supports = useCallback(
    (feature: Feature): boolean => {
      return state.supportedFeatures.get(feature) ?? false
    },
    [state.supportedFeatures],
  )

  return {
    supports,
    isLoading,
    browserOSVersion: state.browserOSVersion,
    serverVersion: state.serverVersion,
  }
}
