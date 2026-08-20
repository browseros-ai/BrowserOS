import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { Provider } from '@/components/chat/chatComponentTypes'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { useAcpAgents } from '@/modules/agents/agents.hooks'
import { useLlmProviders } from '@/modules/llm-providers/llm-providers.hooks'
import { toProviderOption } from './chat-session-request'
import {
  buildSidepanelChatTargets,
  commitChatTargetSelection,
  loadSidepanelChatTargetSelection,
  persistSidepanelChatTargetSelection,
  resolveRepairedSelection,
  resolveSidepanelChatTarget,
  type SidepanelChatTarget,
  type SidepanelChatTargetSelection,
  watchSidepanelChatTargetSelection,
} from './sidepanel-chat-targets'

/**
 * Single source of truth for the selected chat target across every surface that
 * picks one (the sidebar, the home composer, and anything added later). Owns the
 * whole lifecycle: build targets from providers + agents, load the persisted
 * selection, resolve the selected target (selection-first with a non-destructive
 * fallback), repair genuinely-stale selections once loads settle, and change the
 * selection via the shared `commitChatTargetSelection` side effect. Consolidating
 * this here is what stops the "agent default does not persist" bug from recurring
 * as new surfaces are added.
 */
export function useChatTargetSelection() {
  const {
    providers: llmProviders,
    selectedProvider: selectedLlmProvider,
    setDefaultProvider,
    isLoading: isLoadingProviders,
  } = useLlmProviders()
  const {
    agents,
    loading: isLoadingAgents,
    settled: agentsSettled,
  } = useAcpAgents()

  const [targetSelection, setTargetSelection] =
    useState<SidepanelChatTargetSelection | null>(null)

  useEffect(() => {
    let cancelled = false
    loadSidepanelChatTargetSelection().then((selection) => {
      if (!cancelled) setTargetSelection(selection)
    })
    // Live-sync across surfaces: changing the selection in the sidebar, home, or
    // settings writes storage, and WXT's storage.watch (over browser.storage
    // .onChanged) fires in every extension context, so the other surfaces update
    // without a reload. Re-persisting the same value is a no-op, so this cannot
    // loop with the repair effect.
    const unwatch = watchSidepanelChatTargetSelection((selection) => {
      setTargetSelection(selection)
    })
    return () => {
      cancelled = true
      unwatch()
    }
  }, [])

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

  const selectedChatTarget = useMemo(
    () =>
      resolveSidepanelChatTarget({
        targets: chatTargets,
        defaultProviderId: selectedLlmProvider?.id ?? llmProviders[0]?.id ?? '',
        selection: targetSelection,
      }),
    [chatTargets, llmProviders, selectedLlmProvider, targetSelection],
  )
  const selectedProvider = useMemo(
    () => (selectedChatTarget ? toProviderOption(selectedChatTarget) : null),
    [selectedChatTarget],
  )

  useEffect(() => {
    // Only repair once providers and agents are settled. Otherwise a stored ACP
    // selection is wiped to the LLM fallback during the startup window where the
    // agents fetch has not resolved yet and the agent is absent from the list.
    const ready = !isLoadingProviders && agentsSettled
    const decision = resolveRepairedSelection({
      selection: targetSelection,
      resolvedTarget: selectedChatTarget,
      ready,
    })
    if (!decision.repair) return
    setTargetSelection(decision.selection)
    void persistSidepanelChatTargetSelection(selectedChatTarget)
  }, [agentsSettled, isLoadingProviders, selectedChatTarget, targetSelection])

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
      const selection = target ? { kind: target.kind, id: target.id } : null
      setTargetSelection(selection)
      await commitChatTargetSelection(selection, { setDefaultProvider })
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

  return {
    llmProviders,
    selectedLlmProvider,
    selectedLlmProviderRef,
    setDefaultProvider,
    isLoadingProviders: isLoadingProviders || isLoadingAgents,
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
