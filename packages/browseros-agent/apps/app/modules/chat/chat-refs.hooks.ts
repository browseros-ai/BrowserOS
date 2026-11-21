import { useEffect, useRef } from 'react'
import { usePersonalization } from '@/lib/personalization/personalizationStorage'
import { useChatTargetSelection } from './use-chat-target-selection'

export const useChatRefs = () => {
  const selection = useChatTargetSelection()
  const { personalization } = usePersonalization()

  const personalizationRef = useRef(personalization)

  useEffect(() => {
    personalizationRef.current = personalization
  }, [personalization])

  return {
    ...selection,
    personalizationRef,
  }
}
