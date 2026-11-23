import { type FC, useCallback, useState } from 'react'
import {
  CHATGPT_PRO_OAUTH_COMPLETED_EVENT,
  CHATGPT_PRO_OAUTH_DISCONNECTED_EVENT,
  CHATGPT_PRO_OAUTH_STARTED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { CHATGPT_PROVIDER_DISPLAY_NAME } from '@/lib/llm-providers/provider-display-names'
import type { ProviderTemplate } from '@/lib/llm-providers/providerTemplates'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent } from '@/modules/agents/acp-agent-types'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import {
  type OAuthProviderFlowConfig,
  useOAuthProviderFlow,
} from '@/modules/llm-providers/oauth-provider-flow.hooks'
import { NewCodingAgentDialog } from './NewCodingAgentDialog'
import { NewProviderDialog } from './NewProviderDialog'

const CHATGPT_OAUTH_CONFIG: OAuthProviderFlowConfig = {
  providerType: 'chatgpt-pro',
  displayName: CHATGPT_PROVIDER_DISPLAY_NAME,
  startedEvent: CHATGPT_PRO_OAUTH_STARTED_EVENT,
  completedEvent: CHATGPT_PRO_OAUTH_COMPLETED_EVENT,
  disconnectedEvent: CHATGPT_PRO_OAUTH_DISCONNECTED_EVENT,
}

export interface OAuthFlowEntry {
  disconnect: () => Promise<void>
  disconnectedEvent: string
}

export interface AddProviderController {
  /** Wired straight into AddProviderSection. */
  onUseTemplate: (template: ProviderTemplate) => void
  onCreateAgent: (type: AcpAgent['type']) => void
  /**
   * Opens the provider form from the configured list.
   */
  openProviderForm: (values?: Partial<LlmProviderConfig>) => void
  /** Consumed by the settings page's delete path to revoke a token. */
  chatgptOAuth: OAuthFlowEntry
  dialogs: AddProviderDialogState
}

interface AddProviderDialogState {
  isNewDialogOpen: boolean
  setIsNewDialogOpen: (open: boolean) => void
  templateValues: Partial<LlmProviderConfig> | undefined
  newAgentType: AcpAgent['type'] | null
  setNewAgentType: (type: AcpAgent['type'] | null) => void
  onSaveProvider: (provider: LlmProviderConfig) => Promise<void>
  onAgentAdded?: (agentId: string) => void
}

/** Shares provider setup and its completion callbacks between onboarding and settings. */
export function useAddProvider(input: {
  saveProvider: (provider: LlmProviderConfig) => Promise<LlmProviderConfig>
  /** Fires once a provider is successfully added on any path, OAuth included. */
  onProviderAdded?: (provider: LlmProviderConfig) => void | Promise<void>
  /** Fires once a coding agent is successfully created. */
  onAgentAdded?: (agentId: string) => void
}): AddProviderController {
  const { saveProvider: rawSaveProvider, onProviderAdded, onAgentAdded } = input
  const saveProvider = useCallback(
    async (provider: LlmProviderConfig) => {
      // Report the row that was actually persisted: a single-instance reconnect
      // keeps the existing id, which is the id the chat-target selection needs.
      const saved = await rawSaveProvider(provider)
      await onProviderAdded?.(saved)
    },
    [rawSaveProvider, onProviderAdded],
  )
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  const [isNewDialogOpen, setIsNewDialogOpen] = useState(false)
  const [newAgentType, setNewAgentType] = useState<AcpAgent['type'] | null>(
    null,
  )
  const [templateValues, setTemplateValues] = useState<
    Partial<LlmProviderConfig> | undefined
  >()

  const chatgptPro = useOAuthProviderFlow(CHATGPT_OAUTH_CONFIG, saveProvider)

  return {
    onUseTemplate: (template) => {
      // Sign-in opens a tab while this page polls for completion.
      if (template.id === 'chatgpt-pro') {
        chatgptPro.startOAuthFlow(agentServerUrl ?? undefined)
        return
      }

      setTemplateValues({
        type: template.id,
        name: template.name,
        baseUrl: template.defaultBaseUrl,
        modelId: template.defaultModelId,
        supportsImages: template.supportsImages,
        contextWindow: template.contextWindow,
        temperature: 0.2,
      })
      setIsNewDialogOpen(true)
    },
    onCreateAgent: setNewAgentType,
    openProviderForm: (values) => {
      setTemplateValues(values)
      setIsNewDialogOpen(true)
    },
    chatgptOAuth: {
      disconnect: chatgptPro.disconnect,
      disconnectedEvent: CHATGPT_OAUTH_CONFIG.disconnectedEvent,
    },
    dialogs: {
      isNewDialogOpen,
      setIsNewDialogOpen,
      templateValues,
      newAgentType,
      setNewAgentType,
      onSaveProvider: saveProvider,
      onAgentAdded,
    },
  }
}

/** The dialogs the add path can open. Rendered by every screen that adds. */
export const AddProviderDialogs: FC<{ controller: AddProviderController }> = ({
  controller,
}) => {
  const d = controller.dialogs
  return (
    <>
      <NewProviderDialog
        open={d.isNewDialogOpen}
        onOpenChange={d.setIsNewDialogOpen}
        initialValues={d.templateValues}
        onSave={d.onSaveProvider}
      />
      <NewCodingAgentDialog
        type={d.newAgentType}
        open={d.newAgentType !== null}
        onOpenChange={(open) => {
          if (!open) d.setNewAgentType(null)
        }}
        onSaved={d.onAgentAdded}
      />
    </>
  )
}
