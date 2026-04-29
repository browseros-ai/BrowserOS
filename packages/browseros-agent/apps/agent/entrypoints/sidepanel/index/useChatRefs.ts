import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import useDeepCompareEffect from 'use-deep-compare-effect'
import { useAgentAdapters } from '@/entrypoints/app/agents/useAgents'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { useLlmProviders } from '@/lib/llm-providers/useLlmProviders'
import { type McpServer, useMcpServers } from '@/lib/mcp/mcpServerStorage'
import { usePersonalization } from '@/lib/personalization/personalizationStorage'
import {
  buildSidepanelChatTargets,
  loadSidepanelChatTargetSelection,
  persistSidepanelChatTargetSelection,
  resolveSidepanelChatTarget,
  type SidepanelChatTarget,
  type SidepanelChatTargetSelection,
} from './sidepanel-chat-targets'

const constructMcpServers = (servers: McpServer[]) => {
  return servers
    .filter((eachServer) => eachServer.type === 'managed')
    .map((each) => each.managedServerName)
}

const constructCustomServers = (servers: McpServer[]) => {
  return servers
    .filter((eachServer) => eachServer.type === 'custom')
    .map((each) => ({
      name: each.displayName,
      url: each.config?.url,
    }))
}

export const useChatRefs = () => {
  const { servers: mcpServers } = useMcpServers()
  const {
    providers: llmProviders,
    selectedProvider: selectedLlmProvider,
    setDefaultProvider,
    isLoading: isLoadingProviders,
  } = useLlmProviders()
  const { adapters, loading: isLoadingAdapters } = useAgentAdapters()
  const { personalization } = usePersonalization()
  const [targetSelection, setTargetSelection] =
    useState<SidepanelChatTargetSelection | null>(null)

  useEffect(() => {
    let cancelled = false
    loadSidepanelChatTargetSelection().then((selection) => {
      if (!cancelled) setTargetSelection(selection)
    })
    return () => {
      cancelled = true
    }
  }, [])

  const chatTargets = useMemo(
    () =>
      buildSidepanelChatTargets({
        providers: llmProviders,
        adapters,
      }),
    [llmProviders, adapters],
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

  const selectedLlmProviderRef = useRef<LlmProviderConfig | null>(
    selectedLlmProvider,
  )
  const selectedChatTargetRef = useRef<SidepanelChatTarget | undefined>(
    selectedChatTarget,
  )
  const enabledMcpServersRef = useRef(constructMcpServers(mcpServers))
  const enabledCustomServersRef = useRef(constructCustomServers(mcpServers))
  const personalizationRef = useRef(personalization)

  useDeepCompareEffect(() => {
    selectedLlmProviderRef.current = selectedLlmProvider
    enabledMcpServersRef.current = constructMcpServers(mcpServers)
    enabledCustomServersRef.current = constructCustomServers(mcpServers)
  }, [selectedLlmProvider, mcpServers])

  useEffect(() => {
    selectedChatTargetRef.current = selectedChatTarget
  }, [selectedChatTarget])

  useEffect(() => {
    personalizationRef.current = personalization
  }, [personalization])

  const selectChatTarget = useCallback(
    async (target: SidepanelChatTarget | undefined) => {
      selectedChatTargetRef.current = target
      setTargetSelection(target ? { kind: target.kind, id: target.id } : null)
      await persistSidepanelChatTargetSelection(target)
    },
    [],
  )

  return {
    selectedLlmProviderRef,
    selectedChatTargetRef,
    enabledMcpServersRef,
    enabledCustomServersRef,
    personalizationRef,
    llmProviders,
    setDefaultProvider,
    chatTargets,
    selectedChatTarget,
    selectChatTarget,
    selectedLlmProvider,
    isLoadingProviders: isLoadingProviders || isLoadingAdapters,
  }
}
