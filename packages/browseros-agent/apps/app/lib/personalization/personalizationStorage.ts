import { storage } from '@wxt-dev/storage'
import { useCallback, useEffect, useRef, useState } from 'react'

export const personalizationStorage = storage.defineItem<string>(
  'local:personalization',
  {
    fallback: '',
  },
)

export function usePersonalization() {
  const [personalization, setPersonalizationState] = useState('')
  const isLocalUpdate = useRef(false)
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    personalizationStorage.getValue().then(setPersonalizationState)
    const unwatch = personalizationStorage.watch((newValue) => {
      if (!isLocalUpdate.current) {
        setPersonalizationState(newValue ?? '')
      }
      isLocalUpdate.current = false
    })
    return unwatch
  }, [])

  const setPersonalization = useCallback((value: string) => {
    setPersonalizationState(value)
    isLocalUpdate.current = true

    if (debounceTimer.current) {
      clearTimeout(debounceTimer.current)
    }
    debounceTimer.current = setTimeout(() => {
      personalizationStorage.setValue(value)
    }, 300)
  }, [])

  const clearPersonalization = useCallback(async () => {
    setPersonalizationState('')
    isLocalUpdate.current = true
    await personalizationStorage.setValue('')
  }, [])

  useEffect(() => {
    return () => {
      if (debounceTimer.current) {
        clearTimeout(debounceTimer.current)
      }
    }
  }, [])

  return { personalization, setPersonalization, clearPersonalization }
}
