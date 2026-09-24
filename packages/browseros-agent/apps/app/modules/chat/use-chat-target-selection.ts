import { useCallback, useEffect, useMemo, useRef } from 'react'
import type { Provider } from '@/components/chat/chatComponentTypes'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { useAcpAgents } from '@/modules/agents/agents.hooks'
import { useLlmProviders } from '@/modules/llm-providers/llm-providers.hooks'
import { toProviderOption } from './chat-session-request'
import {
  buildSidepanelChatTargets,
  resolveSidepanelChatTarget,
  type SidepanelChatTarget,
} from './sidepanel-chat-targets'

/**
 * Single source of truth for the selected chat target across every surface that
 * picks one (the sidebar, the home composer, and anything added later). Builds
 * the targets from providers plus agents and resolves the chosen one, with a
 * non-destructive fallback when it names nothing.
 *
 * The choice is the server's default pointer and nothing else. It used to be
 * mirrored into extension storage as well, which bought a live broadcast
 * between surfaces at the cost of two records of one fact: the copies drifted
 * whenever a row was written in one surface and read in another, and a repair
 * pass existed to reconcile them. The provider revision signal broadcasts the
 * same way for free, so the mirror only ever added the drift.
 */
export function useChatTargetSelection() {
  const {
    providers: llmProviders,
    selectedProvider: selectedLlmProvider,
    storedDefaultTargetId,
    setDefaultProvider,
    isLoading: isLoadingProviders,
  } = useLlmProviders()
  const {
    agents,
    loading: isLoadingAgents,
    settled: agentsSettled,
  } = useAcpAgents()

  const chatTargets = useMemo(
    () =>
      buildSidepanelChatTargets({
        providers: llmProviders,
        agents,
      }),
    [llmProviders, agents],
  )
  const providerOptions = useMemo(
    () => chatTargets.map(toProviderOption),
    [chatTargets],
  )

  // The stored id verbatim, not the one resolved through the LLM-only list.
  // Resolving first replaced a default naming a coding agent with the first
  // provider, so a profile chatted with something the user never chose. The
  // resolver falls back on its own when this names nothing.
  const selectedChatTarget = useMemo(
    () =>
      resolveSidepanelChatTarget({
        targets: chatTargets,
        defaultTargetId: storedDefaultTargetId,
      }),
    [chatTargets, storedDefaultTargetId],
  )
  const selectedProvider = useMemo(
    () => (selectedChatTarget ? toProviderOption(selectedChatTarget) : null),
    [selectedChatTarget],
  )

  const selectedLlmProviderRef = useRef<LlmProviderConfig | null>(
    selectedLlmProvider,
  )
  const selectedChatTargetRef = useRef<SidepanelChatTarget | undefined>(
    selectedChatTarget,
  )

  // selectedLlmProvider is memoized in useLlmProviders (stable reference until it
  // actually changes), so a plain effect fires exactly when it changes. Not
  // useDeepCompareEffect: its single dep is null before providers load, and that
  // library throws when every dependency is a primitive.
  useEffect(() => {
    selectedLlmProviderRef.current = selectedLlmProvider
  }, [selectedLlmProvider])

  useEffect(() => {
    selectedChatTargetRef.current = selectedChatTarget
  }, [selectedChatTarget])

  const selectChatTarget = useCallback(
    async (target: SidepanelChatTarget | undefined) => {
      selectedChatTargetRef.current = target
      if (target) await setDefaultProvider(target.id)
    },
    [setDefaultProvider],
  )

  const selectProvider = useCallback(
    (provider: Provider) => {
      const target = chatTargets.find(
        (entry) => entry.kind === provider.kind && entry.id === provider.id,
      )
      if (!target) return undefined
      return selectChatTarget(target)
    },
    [chatTargets, selectChatTarget],
  )

  // Both lists have to have settled before absence means anything. Reading it
  // mid-load would tell someone their provider is gone every cold start.
  const isSettled = !isLoadingProviders && agentsSettled

  return {
    llmProviders,
    selectedLlmProvider,
    selectedLlmProviderRef,
    setDefaultProvider,
    isLoadingProviders: isLoadingProviders || isLoadingAgents,
    isSettled,
    /** Whether anything at all is connected: an LLM provider or a coding agent. */
    hasAnyTarget: chatTargets.length > 0,
    agents,
    chatTargets,
    providerOptions,
    selectedChatTarget,
    selectedChatTargetRef,
    selectedProvider,
    selectChatTarget,
    selectProvider,
  }
}
