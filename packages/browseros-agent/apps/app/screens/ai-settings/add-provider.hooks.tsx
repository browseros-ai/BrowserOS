import { type FC, useCallback, useState } from 'react'
import {
  CHATGPT_PRO_OAUTH_COMPLETED_EVENT,
  CHATGPT_PRO_OAUTH_DISCONNECTED_EVENT,
  CHATGPT_PRO_OAUTH_STARTED_EVENT,
  GITHUB_COPILOT_OAUTH_COMPLETED_EVENT,
  GITHUB_COPILOT_OAUTH_DISCONNECTED_EVENT,
  GITHUB_COPILOT_OAUTH_STARTED_EVENT,
  QWEN_CODE_OAUTH_COMPLETED_EVENT,
  QWEN_CODE_OAUTH_DISCONNECTED_EVENT,
  QWEN_CODE_OAUTH_STARTED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { CHATGPT_PROVIDER_DISPLAY_NAME } from '@/lib/llm-providers/provider-display-names'
import type { ProviderTemplate } from '@/lib/llm-providers/providerTemplates'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { AcpAgent, AcpAgentType } from '@/modules/agents/acp-agent-types'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import {
  type OAuthProviderFlowConfig,
  useOAuthProviderFlow,
} from '@/modules/llm-providers/oauth-provider-flow.hooks'
import { CustomCodingAgentDialog } from './CustomCodingAgentDialog'
import { DeviceCodeDialog } from './DeviceCodeDialog'
import { NewCodingAgentDialog } from './NewCodingAgentDialog'
import { NewProviderDialog } from './NewProviderDialog'

/** All OAuth providers share the same flow via useOAuthProviderFlow. */
export const OAUTH_PROVIDERS_CONFIG: Record<string, OAuthProviderFlowConfig> = {
  'chatgpt-pro': {
    providerType: 'chatgpt-pro',
    displayName: CHATGPT_PROVIDER_DISPLAY_NAME,
    startedEvent: CHATGPT_PRO_OAUTH_STARTED_EVENT,
    completedEvent: CHATGPT_PRO_OAUTH_COMPLETED_EVENT,
    disconnectedEvent: CHATGPT_PRO_OAUTH_DISCONNECTED_EVENT,
  },
  'github-copilot': {
    providerType: 'github-copilot',
    displayName: 'GitHub Copilot',
    startedEvent: GITHUB_COPILOT_OAUTH_STARTED_EVENT,
    completedEvent: GITHUB_COPILOT_OAUTH_COMPLETED_EVENT,
    disconnectedEvent: GITHUB_COPILOT_OAUTH_DISCONNECTED_EVENT,
    clientAuth: {
      deviceCodeEndpoint: 'https://github.com/login/device/code',
      tokenEndpoint: 'https://github.com/login/oauth/access_token',
      clientId: 'Ov23li8tweQw6odWQebz',
      scopes: 'read:user',
      requiresPKCE: false,
      contentType: 'json',
    },
  },
  'qwen-code': {
    providerType: 'qwen-code',
    displayName: 'Qwen Code',
    startedEvent: QWEN_CODE_OAUTH_STARTED_EVENT,
    completedEvent: QWEN_CODE_OAUTH_COMPLETED_EVENT,
    disconnectedEvent: QWEN_CODE_OAUTH_DISCONNECTED_EVENT,
    clientAuth: {
      deviceCodeEndpoint: 'https://chat.qwen.ai/api/v1/oauth2/device/code',
      tokenEndpoint: 'https://chat.qwen.ai/api/v1/oauth2/token',
      clientId: 'f0304373b74a44d2b584a3fb70ca9e56',
      scopes: 'openid profile email model.completion',
      requiresPKCE: true,
      contentType: 'form',
    },
  },
}

export interface OAuthFlowEntry {
  startOAuthFlow: (url: string | undefined) => Promise<void>
  disconnect: () => Promise<void>
  disconnectedEvent: string
}

export interface AddProviderController {
  /** Wired straight into AddProviderSection. */
  onUseTemplate: (template: ProviderTemplate) => void
  onCreateAgent: (type: AcpAgentType) => void
  onCreateCustomAgent: () => void
  /**
   * Opens the provider form directly. No argument is the "+ Add" case; the
   * settings page passes prefill when completing a synced-but-keyless provider.
   */
  openProviderForm: (values?: Partial<LlmProviderConfig>) => void
  openCustomAgentEditor: (agent: AcpAgent) => void
  /** Consumed by the settings page's delete path to revoke a token. */
  oauthFlows: Record<string, OAuthFlowEntry>
  dialogs: AddProviderDialogState
}

interface AddProviderDialogState {
  isNewDialogOpen: boolean
  setIsNewDialogOpen: (open: boolean) => void
  templateValues: Partial<LlmProviderConfig> | undefined
  newAgentType: AcpAgentType | null
  setNewAgentType: (type: AcpAgentType | null) => void
  customAgentDialogOpen: boolean
  setCustomAgentDialogOpen: (open: boolean) => void
  editingCustomAgent: AcpAgent | null
  activeDeviceCode: ReturnType<typeof useOAuthProviderFlow>['pendingDeviceCode']
  clearActiveDeviceCode: () => void
  onSaveProvider: (provider: LlmProviderConfig) => Promise<void>
  onAgentAdded?: (agentId: string) => void
}

/**
 * Everything needed to turn "user picked a provider" into a saved provider:
 * the three OAuth flows, the dialog state, and the handlers that decide which
 * of the two paths a template takes.
 *
 * Extracted from BrowserOsAiPane so the first-run setup screen can offer the
 * same catalogue without inheriting the settings page's configured list,
 * promos, default-target control and delete flows.
 *
 * `providers` and `saveProvider` are arguments rather than a `useLlmProviders()`
 * call inside: that hook holds its own useState, so a second instance would be
 * a second copy of the provider list that silently diverges from the caller's.
 */
export function useAddProvider(input: {
  providers: LlmProviderConfig[]
  saveProvider: (provider: LlmProviderConfig) => Promise<LlmProviderConfig>
  /** Fires once a provider is successfully added on any path, OAuth included. */
  onProviderAdded?: (provider: LlmProviderConfig) => void | Promise<void>
  /** Fires once a coding agent is successfully created. */
  onAgentAdded?: (agentId: string) => void
}): AddProviderController {
  const {
    providers,
    saveProvider: rawSaveProvider,
    onProviderAdded,
    onAgentAdded,
  } = input
  // Every add path funnels through saveProvider: the dialog form calls it, and
  // so do all three OAuth flows on token success (they poll in this mounted
  // page, they do not navigate away). Wrapping it once is the single definitive
  // "a provider was added" signal, with no list-watching or baselines.
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
  const [newAgentType, setNewAgentType] = useState<AcpAgentType | null>(null)
  const [customAgentDialogOpen, setCustomAgentDialogOpen] = useState(false)
  const [editingCustomAgent, setEditingCustomAgent] = useState<AcpAgent | null>(
    null,
  )
  const [templateValues, setTemplateValues] = useState<
    Partial<LlmProviderConfig> | undefined
  >()

  const chatgptPro = useOAuthProviderFlow(
    OAUTH_PROVIDERS_CONFIG['chatgpt-pro'],
    providers,
    saveProvider,
  )
  const copilot = useOAuthProviderFlow(
    OAUTH_PROVIDERS_CONFIG['github-copilot'],
    providers,
    saveProvider,
  )
  const qwenCode = useOAuthProviderFlow(
    OAUTH_PROVIDERS_CONFIG['qwen-code'],
    providers,
    saveProvider,
  )

  const activeDeviceCode =
    chatgptPro.pendingDeviceCode ??
    copilot.pendingDeviceCode ??
    qwenCode.pendingDeviceCode

  const oauthFlows: Record<string, OAuthFlowEntry> = {
    'chatgpt-pro': {
      startOAuthFlow: chatgptPro.startOAuthFlow,
      disconnect: chatgptPro.disconnect,
      disconnectedEvent: CHATGPT_PRO_OAUTH_DISCONNECTED_EVENT,
    },
    'github-copilot': {
      startOAuthFlow: copilot.startOAuthFlow,
      disconnect: copilot.disconnect,
      disconnectedEvent: GITHUB_COPILOT_OAUTH_DISCONNECTED_EVENT,
    },
    'qwen-code': {
      startOAuthFlow: qwenCode.startOAuthFlow,
      disconnect: qwenCode.disconnect,
      disconnectedEvent: QWEN_CODE_OAUTH_DISCONNECTED_EVENT,
    },
  }

  return {
    onUseTemplate: (template) => {
      // A subscription template signs in rather than collecting a key, so it
      // leaves the page instead of opening the form.
      const oauthFlow = oauthFlows[template.id]
      if (oauthFlow) {
        oauthFlow.startOAuthFlow(agentServerUrl ?? undefined)
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
    onCreateCustomAgent: () => {
      setEditingCustomAgent(null)
      setCustomAgentDialogOpen(true)
    },
    openProviderForm: (values) => {
      setTemplateValues(values)
      setIsNewDialogOpen(true)
    },
    openCustomAgentEditor: (agent) => {
      setEditingCustomAgent(agent)
      setCustomAgentDialogOpen(true)
    },
    oauthFlows,
    dialogs: {
      isNewDialogOpen,
      setIsNewDialogOpen,
      templateValues,
      newAgentType,
      setNewAgentType,
      customAgentDialogOpen,
      setCustomAgentDialogOpen,
      editingCustomAgent,
      activeDeviceCode,
      clearActiveDeviceCode: () => {
        chatgptPro.clearDeviceCode()
        copilot.clearDeviceCode()
        qwenCode.clearDeviceCode()
      },
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
      <CustomCodingAgentDialog
        open={d.customAgentDialogOpen}
        onOpenChange={d.setCustomAgentDialogOpen}
        agent={d.editingCustomAgent}
        onSaved={d.onAgentAdded}
      />
      <DeviceCodeDialog
        deviceCode={d.activeDeviceCode}
        onClose={d.clearActiveDeviceCode}
      />
    </>
  )
}
