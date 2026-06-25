import { storage } from '@wxt-dev/storage'
import { sessionStorage } from '@/lib/auth/sessionStorage'
import { Capabilities } from '@/lib/browseros/capabilities'
import { getHealthCheckUrl, getMcpServerUrl } from '@/lib/browseros/helpers'
import {
  ensureSidePanelRuntimeStateLoaded,
  openSidePanel,
  registerSidePanelOpenStateListeners,
  setSidePanelPerWindowPreference,
  toggleSidePanel,
} from '@/lib/browseros/toggleSidePanel'
import { checkAndShowChangelog } from '@/lib/changelog/changelog-notifier'
import {
  setupLlmProvidersBackupToBrowserOS,
  setupLlmProvidersSyncToBackend,
  syncLlmProviders,
} from '@/lib/llm-providers/storage'
import { fetchMcpTools } from '@/lib/mcp/client'
import {
  onRuntimeMessage,
  RuntimeMessageType,
} from '@/lib/messaging/runtime/runtimeMessages'
import { onServerMessage } from '@/lib/messaging/server/serverMessages'
import { onOpenSidePanelWithSearch } from '@/lib/messaging/sidepanel/openSidepanelWithSearch'
import { authRedirectPathStorage } from '@/lib/onboarding/onboardingStorage'
import { syncOnboardingProfile } from '@/lib/onboarding/syncOnboardingProfile'
import {
  setupScheduledJobsSyncToBackend,
  syncScheduledJobs,
} from '@/lib/schedules/syncSchedulesToBackend'
import { searchActionsStorage } from '@/lib/search-actions/searchActionsStorage'
import { selectedTextStorage } from '@/lib/selected-text/selectedTextStorage'
import { stopAgentStorage } from '@/lib/stop-agent/stop-agent-storage'
import { scheduledJobRuns } from './scheduledJobRuns'

const LEGACY_TOOL_APPROVAL_STORAGE_KEYS = [
  'local:tool-approval-config',
  'local:pending-tool-approvals',
  'local:approval-responses',
  'local:tool-execution-log',
] as const

/**
 * Removes persisted state for the unshipped Tool Approvals feature during extension updates.
 */
const cleanupLegacyToolApprovalStorage = async () => {
  await storage.removeItems([...LEGACY_TOOL_APPROVAL_STORAGE_KEYS])
}

export default defineBackground(() => {
  registerSidePanelOpenStateListeners()
  ensureSidePanelRuntimeStateLoaded().catch(() => null)

  Capabilities.initialize().catch(() => null)
  setupLlmProvidersBackupToBrowserOS()
  setupLlmProvidersSyncToBackend()
  setupScheduledJobsSyncToBackend()

  scheduledJobRuns()

  chrome.action.onClicked.addListener(async (tab) => {
    if (typeof tab.id === 'number' && typeof tab.windowId === 'number') {
      await toggleSidePanel({ tabId: tab.id, windowId: tab.windowId })
    }
  })

  onOpenSidePanelWithSearch('open', async (messageData) => {
    const currentTabsList = await chrome.tabs.query({
      active: true,
      currentWindow: true,
    })
    const currentTab = currentTabsList?.[0]
    if (
      typeof currentTab?.id === 'number' &&
      typeof currentTab.windowId === 'number'
    ) {
      const { opened } = await openSidePanel({
        tabId: currentTab.id,
        windowId: currentTab.windowId,
      })

      if (opened) {
        setTimeout(() => {
          searchActionsStorage.setValue(messageData.data)
        }, 500)
      }
    }
  })

  chrome.runtime.onInstalled.addListener((details) => {
    if (details.reason === chrome.runtime.OnInstalledReason.INSTALL) {
      chrome.tabs.create({
        url: chrome.runtime.getURL('app.html#/onboarding'),
      })
    }

    if (details.reason === chrome.runtime.OnInstalledReason.UPDATE) {
      cleanupLegacyToolApprovalStorage().catch(() => null)
      checkAndShowChangelog().catch(() => null)
    }
  })

  onRuntimeMessage(RuntimeMessageType.getTabId, ({ sender }) => {
    return { tabId: sender.tab?.id }
  })

  onRuntimeMessage(RuntimeMessageType.authSuccess, async ({ sender }) => {
    if (!sender.tab?.id) return

    const tabId = sender.tab.id

    try {
      const redirectPath = await authRedirectPathStorage.getValue()
      const hash = redirectPath || '/home'
      await chrome.tabs.update(tabId, {
        url: chrome.runtime.getURL(`app.html#${hash}`),
      })
      if (redirectPath) await authRedirectPathStorage.removeValue()
    } catch {
      await chrome.tabs.update(tabId, {
        url: chrome.runtime.getURL('app.html#/home'),
      })
    }
  })

  onRuntimeMessage(RuntimeMessageType.stopAgent, async ({ data }) => {
    await stopAgentStorage.setValue({
      conversationId: data.conversationId,
      timestamp: Date.now(),
    })
  })

  onRuntimeMessage(
    RuntimeMessageType.sidePanelScopeChanged,
    async ({ data }) => {
      await setSidePanelPerWindowPreference(data.perWindow)
    },
  )

  chrome.tabs.onRemoved.addListener((tabId) => {
    const key = String(tabId)
    selectedTextStorage.getValue().then((map) => {
      if (map[key]) {
        const { [key]: _, ...rest } = map
        selectedTextStorage.setValue(rest)
      }
    })
  })

  sessionStorage.watch(async (newSession) => {
    if (newSession?.user?.id) {
      try {
        await syncLlmProviders()
      } catch {}
      try {
        await syncScheduledJobs()
      } catch {}
      try {
        await syncOnboardingProfile(newSession.user.id)
      } catch {}
    }
  })

  onServerMessage('checkHealth', async () => {
    try {
      const url = await getHealthCheckUrl()
      const response = await fetch(url)
      return { healthy: response.ok }
    } catch {
      return { healthy: false }
    }
  })

  onServerMessage('fetchMcpTools', async () => {
    try {
      const url = await getMcpServerUrl()
      const tools = await fetchMcpTools(url)
      return { tools }
    } catch (err) {
      return {
        tools: [],
        error: err instanceof Error ? err.message : 'Failed to fetch tools',
      }
    }
  })
})
