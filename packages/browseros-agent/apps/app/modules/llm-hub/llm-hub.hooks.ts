import { useEffect, useState } from 'react'
import {
  type LlmHubProvider,
  loadProviders,
  saveProviders,
} from '@/lib/llm-hub/storage'

let providersCache: LlmHubProvider[] | null = null

/** @public */
export interface UseLlmHubProvidersReturn {
  providers: LlmHubProvider[]
  isLoading: boolean
  saveProvider: (provider: LlmHubProvider, editIndex?: number) => Promise<void>
  deleteProvider: (index: number) => Promise<void>
}

/** @public */
export function useLlmHubProviders(): UseLlmHubProvidersReturn {
  const [providers, setProviders] = useState<LlmHubProvider[]>(
    providersCache ?? [],
  )
  const [isLoading, setIsLoading] = useState(providersCache === null)

  useEffect(() => {
    let isMounted = true
    const load = async () => {
      if (providersCache === null) {
        setIsLoading(true)
      }
      try {
        const data = await loadProviders()
        providersCache = data
        if (isMounted) {
          setProviders(data)
        }
      } catch {
        providersCache = []
        if (isMounted) {
          setProviders([])
        }
      } finally {
        if (isMounted) {
          setIsLoading(false)
        }
      }
    }
    load()
    return () => {
      isMounted = false
    }
  }, [])

  const saveProvider = async (provider: LlmHubProvider, editIndex?: number) => {
    const currentProviders = await loadProviders()
    const isEdit = editIndex !== undefined && editIndex >= 0
    const updatedProviders = isEdit
      ? currentProviders.map((p, i) => (i === editIndex ? provider : p))
      : [...currentProviders, provider]

    providersCache = updatedProviders
    setProviders(updatedProviders)
    const success = await saveProviders(updatedProviders)
    if (!success) {
      const reloaded = await loadProviders()
      providersCache = reloaded
      setProviders(reloaded)
    }
  }

  const deleteProvider = async (index: number) => {
    const currentProviders = await loadProviders()
    if (currentProviders.length <= 1) return

    const updatedProviders = currentProviders.filter((_, i) => i !== index)

    providersCache = updatedProviders
    setProviders(updatedProviders)
    const success = await saveProviders(updatedProviders)
    if (!success) {
      const reloaded = await loadProviders()
      providersCache = reloaded
      setProviders(reloaded)
    }
  }

  return {
    providers,
    isLoading,
    saveProvider,
    deleteProvider,
  }
}
