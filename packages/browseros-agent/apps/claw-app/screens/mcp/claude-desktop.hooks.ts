import { useState } from 'react'
import {
  useClaudeDesktopExtension,
  useInstallClaudeDesktopExtension,
} from '@/modules/api/claude-desktop.hooks'
import { toastApiError } from '@/modules/toast/toast'
import {
  type AddPhase,
  type ClaudeSetupState,
  claudeSetupState,
  INSTALL_WAIT_MS,
} from './claude-desktop.helpers'

const WAITING_POLL_MS = 2000

export interface AddToClaude {
  state: ClaudeSetupState
  /** Opens the extension in Claude; Claude then asks the person to click Install. */
  add: () => void
  isOpening: boolean
}

/**
 * Adds neo to the Claude desktop app in one step. While Claude shows its
 * install sheet this polls Claude's record of installed extensions, and after
 * a couple of minutes without an install it offers the step-by-step guide.
 */
export function useAddToClaude(): AddToClaude {
  const [phase, setPhase] = useState<AddPhase>('idle')
  const extension = useClaudeDesktopExtension({
    refetchInterval: (query) =>
      phase === 'waiting' &&
      claudeSetupState(query.state.data, phase) === 'waiting'
        ? WAITING_POLL_MS
        : false,
  })
  const install = useInstallClaudeDesktopExtension({
    onSuccess: async (result) => {
      await extension.refetch()
      if (result.status !== 'opened') return
      setPhase('waiting')
      setTimeout(
        () =>
          setPhase((current) => (current === 'waiting' ? 'stuck' : current)),
        INSTALL_WAIT_MS,
      )
    },
    onError: (error) => toastApiError('Couldn’t open the Claude app', error),
  })

  return {
    state: claudeSetupState(extension.data, phase),
    add: () => install.mutate(),
    isOpening: install.isPending,
  }
}
