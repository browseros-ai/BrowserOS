import { sessionStorage } from '@/lib/auth/sessionStorage'
import { Capabilities } from '@/lib/browseros/capabilities'
import { getHealthCheckUrl, getMcpServerUrl } from '@/lib/browseros/helpers'
import { openSidePanel, toggleSidePanel } from '@/lib/browseros/toggleSidePanel'
import { checkAndShowChangelog } from '@/lib/changelog/changelog-notifier'
import {
  setupLlmProvidersBackupToBrowserOS,
  setupLlmProvidersSyncToBackend,
  syncLlmProviders,
} from '@/lib/llm-providers/storage'
import { fetchMcpTools } from '@/lib/mcp/client'
import { onServerMessage } from '@/lib/messaging/server/serverMessages'
import { onOpenSidePanelWithSearch } from '@/lib/messaging/sidepanel/openSidepanelWithSearch'
import { authRedirectPathStorage } from '@/lib/onboarding/onboardingStorage'
import { syncOnboardingProfile } from '@/lib/onboarding/syncOnboardingProfile'
import {
  setupScheduledJobsSyncToBackend,
  syncScheduledJobs,
} from '@/lib/schedules/scheduleStorage'
import { searchActionsStorage } from '@/lib/search-actions/searchActionsStorage'
import { selectedTextStorage } from '@/lib/selected-text/selectedTextStorage'
import { stopAgentStorage } from '@/lib/stop-agent/stop-agent-storage'
import { scheduledJobRuns } from './scheduledJobRuns'

export default defineBackground(() => {
  chrome.sidePanel.setOptions({ enabled: false })

  Capabilities.initialize().catch(() => null)
  setupLlmProvidersBackupToBrowserOS()
  setupLlmProvidersSyncToBackend()
  setupScheduledJobsSyncToBackend()

  scheduledJobRuns()

  chrome.action.onClicked.addListener(async (tab) => {
    if (tab.id) {
      await toggleSidePanel(tab.id)
    }
  })

  onOpenSidePanelWithSearch('open', async (messageData) => {
    const currentTabsList = await chrome.tabs.query({
      active: true,
      currentWindow: true,
    })
    const currentTab = currentTabsList?.[0]?.id
    if (currentTab) {
      const { opened } = await openSidePanel(currentTab)

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
      checkAndShowChangelog().catch(() => null)
    }
  })

  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    if (message?.type === 'get-tab-id') {
      sendResponse({ tabId: sender.tab?.id })
      return true
    }

    if (message?.type === 'AUTH_SUCCESS' && sender.tab?.id) {
      const tabId = sender.tab.id
      authRedirectPathStorage
        .getValue()
        .then((redirectPath) => {
          const hash = redirectPath || '/home'
          chrome.tabs.update(tabId, {
            url: chrome.runtime.getURL(`app.html#${hash}`),
          })
          if (redirectPath) authRedirectPathStorage.removeValue()
        })
        .catch(() => {
          chrome.tabs.update(tabId, {
            url: chrome.runtime.getURL('app.html#/home'),
          })
        })
    }

    if (message?.type === 'stop-agent' && message?.conversationId) {
      stopAgentStorage.setValue({
        conversationId: message.conversationId,
        timestamp: Date.now(),
      })
    }
  })

  // Clean up selected text storage when a tab is closed
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
