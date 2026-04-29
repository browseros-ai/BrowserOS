import { type Dispatch, type SetStateAction, useEffect, useMemo } from 'react'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type {
  HarnessAdapterDescriptor,
  HarnessAgentAdapter,
} from './agent-harness-types'
import type { CreateAgentRuntime } from './agents-page-types'
import { toProviderOptions } from './agents-page-utils'
import {
  buildOpenClawCliProviderOptions,
  findOpenClawCliProviderById,
  useOpenClawCliProviderAuthStatus,
} from './openclaw-cli-providers'

export function useDefaultAgentName(
  createOpen: boolean,
  setNewName: Dispatch<SetStateAction<string>>,
): void {
  useEffect(() => {
    if (!createOpen) return
    setNewName((current) => current || 'agent')
  }, [createOpen, setNewName])
}

export function useHarnessAgentDefaults(input: {
  adapters: HarnessAdapterDescriptor[]
  createOpen: boolean
  harnessAdapterId: HarnessAgentAdapter
  setHarnessAdapterId: Dispatch<SetStateAction<HarnessAgentAdapter>>
  setHarnessModelId: Dispatch<SetStateAction<string>>
  setHarnessReasoningEffort: Dispatch<SetStateAction<string>>
}): void {
  const {
    adapters,
    createOpen,
    harnessAdapterId,
    setHarnessAdapterId,
    setHarnessModelId,
    setHarnessReasoningEffort,
  } = input

  useEffect(() => {
    if (!createOpen) return
    const adapter =
      adapters.find((entry) => entry.id === harnessAdapterId) ?? adapters[0]
    if (!adapter) return
    setHarnessAdapterId(adapter.id)
    setHarnessModelId((current) => current || adapter.defaultModelId)
    setHarnessReasoningEffort(
      (current) => current || adapter.defaultReasoningEffort,
    )
  }, [
    adapters,
    createOpen,
    harnessAdapterId,
    setHarnessAdapterId,
    setHarnessModelId,
    setHarnessReasoningEffort,
  ])
}

export function useOpenClawProviderSelection(input: {
  providers: LlmProviderConfig[]
  defaultProviderId: string
  createOpen: boolean
  createRuntime: CreateAgentRuntime
  createProviderId: string
  setCreateProviderId: Dispatch<SetStateAction<string>>
  setupOpen: boolean
  setupProviderId: string
  setSetupProviderId: Dispatch<SetStateAction<string>>
  cliAuthModalOpen: boolean
  setCliAuthModalOpen: Dispatch<SetStateAction<boolean>>
}) {
  const {
    providers,
    defaultProviderId,
    createOpen,
    createRuntime,
    createProviderId,
    setCreateProviderId,
    setupOpen,
    setupProviderId,
    setSetupProviderId,
    cliAuthModalOpen,
    setCliAuthModalOpen,
  } = input
  const cliProviderOptions = useMemo(
    () => buildOpenClawCliProviderOptions(),
    [],
  )
  const selectableOpenClawProviders = useMemo(
    () => toProviderOptions(providers, cliProviderOptions),
    [providers, cliProviderOptions],
  )

  useEffect(() => {
    if (selectableOpenClawProviders.length === 0) return
    const fallbackId =
      selectableOpenClawProviders.find(
        (provider) => provider.id === defaultProviderId,
      )?.id ?? selectableOpenClawProviders[0].id

    if (createOpen && !createProviderId) {
      setCreateProviderId(fallbackId)
    }
  }, [
    createOpen,
    createProviderId,
    defaultProviderId,
    selectableOpenClawProviders,
    setCreateProviderId,
  ])

  useEffect(() => {
    if (selectableOpenClawProviders.length === 0) return
    const fallbackId =
      selectableOpenClawProviders.find(
        (provider) => provider.id === defaultProviderId,
      )?.id ?? selectableOpenClawProviders[0].id

    if (setupOpen && !setupProviderId) {
      setSetupProviderId(fallbackId)
    }
  }, [
    defaultProviderId,
    selectableOpenClawProviders,
    setSetupProviderId,
    setupOpen,
    setupProviderId,
  ])

  const selectedCreateOption = selectableOpenClawProviders.find(
    (provider) => provider.id === createProviderId,
  )
  const selectedCliProvider = selectedCreateOption
    ? findOpenClawCliProviderById(selectedCreateOption.type)
    : undefined
  const selectedSetupOption = selectableOpenClawProviders.find(
    (provider) => provider.id === setupProviderId,
  )
  const selectedSetupCliProvider = selectedSetupOption
    ? findOpenClawCliProviderById(selectedSetupOption.type)
    : undefined
  const activeCliProvider =
    (setupOpen && selectedSetupCliProvider) ||
    (createOpen && createRuntime === 'openclaw' && selectedCliProvider) ||
    undefined
  const {
    data: cliAuthStatus,
    isLoading: cliAuthLoading,
    error: cliAuthError,
  } = useOpenClawCliProviderAuthStatus(
    activeCliProvider?.id ?? '',
    !!activeCliProvider,
  )

  useEffect(() => {
    if (cliAuthModalOpen && cliAuthStatus?.loggedIn) {
      setCliAuthModalOpen(false)
    }
  }, [cliAuthModalOpen, cliAuthStatus?.loggedIn, setCliAuthModalOpen])

  return {
    selectableOpenClawProviders,
    selectedCliProvider,
    selectedSetupCliProvider,
    authTerminalProvider: selectedSetupCliProvider ?? selectedCliProvider,
    cliAuthStatus,
    cliAuthLoading,
    cliAuthError,
  }
}
