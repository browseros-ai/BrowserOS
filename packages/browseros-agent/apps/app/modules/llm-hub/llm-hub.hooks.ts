import { useEffect, useState } from 'react'
import {
  type LlmHubProvider,
  loadProviders,
  saveProviders,
} from '@/lib/llm-hub/storage'

/** @public */
export interface UseLlmHubProvidersReturn {
  providers: LlmHubProvider[]
  isLoading: boolean
  saveProvider: (provider: LlmHubProvider, editIndex?: number) => Promise<void>
  deleteProvider: (index: number) => Promise<void>
}

/** @public */
export function useLlmHubProviders(): UseLlmHubProvidersReturn {
  const [providers, setProviders] = useState<LlmHubProvider[]>([])
  const [isLoading, setIsLoading] = useState(true)

  useEffect(() => {
    const load = async () => {
      setIsLoading(true)
      try {
        const data = await loadProviders()
        setProviders(data)
      } catch {
        setProviders([])
      } finally {
        setIsLoading(false)
      }
    }
    load()
  }, [])

  const saveProvider = async (provider: LlmHubProvider, editIndex?: number) => {
    const currentProviders = await loadProviders()
    const isEdit = editIndex !== undefined && editIndex >= 0
    const updatedProviders = isEdit
      ? currentProviders.map((p, i) => (i === editIndex ? provider : p))
      : [...currentProviders, provider]

    setProviders(updatedProviders)
    const success = await saveProviders(updatedProviders)
    if (!success) {
      const reloaded = await loadProviders()
      setProviders(reloaded)
    }
  }

  const deleteProvider = async (index: number) => {
    const currentProviders = await loadProviders()
    if (currentProviders.length <= 1) return

    const updatedProviders = currentProviders.filter((_, i) => i !== index)

    setProviders(updatedProviders)
    const success = await saveProviders(updatedProviders)
    if (!success) {
      const reloaded = await loadProviders()
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
