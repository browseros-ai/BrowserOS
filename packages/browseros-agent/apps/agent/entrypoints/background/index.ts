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
import {
  authRedirectPathStorage,
  onboardingShownOnceStorage,
} from '@/lib/onboarding/onboardingStorage'
import { syncOnboardingProfile } from '@/lib/onboarding/syncOnboardingProfile'
import {
  setupScheduledJobsSyncToBackend,
  syncScheduledJobs,
} from '@/lib/schedules/scheduleStorage'
import { searchActionsStorage } from '@/lib/search-actions/searchActionsStorage'
import { selectedTextStorage } from '@/lib/selected-text/selectedTextStorage'
import { openBrowserOSHomeOnStartupStorage } from '@/lib/startup/startup-storage'
import { stopAgentStorage } from '@/lib/stop-agent/stop-agent-storage'
import { scheduledJobRuns } from './scheduledJobRuns'

const BROWSEROS_HOME_URL = chrome.runtime.getURL('app.html#/home')
const BROWSEROS_ONBOARDING_URL = chrome.runtime.getURL('app.html#/onboarding')
const BROWSEROS_APP_URL_PREFIX = chrome.runtime.getURL('app.html')
const BROWSEROS_INTERNAL_URL_PREFIX = 'chrome://browseros/'

function shouldReplaceWithBrowserOSHome(url?: string): boolean {
  if (!url) return true
  return (
    url.startsWith(BROWSEROS_INTERNAL_URL_PREFIX) ||
    url === 'about:blank' ||
    url === 'chrome://newtab/' ||
    url.startsWith('chrome://new-tab-page') ||
    url.startsWith('https://www.google.com/_/chrome/newtab') ||
    url.startsWith('file:///C:/Users/') ||
    url.startsWith('http://data/')
  )
}

function isBrowserOSAppUrl(url?: string): boolean {
  return url?.startsWith(BROWSEROS_APP_URL_PREFIX) ?? false
}

function isBrowserOSHomeUrl(url?: string): boolean {
  return url?.startsWith(BROWSEROS_HOME_URL) ?? false
}

function isBrowserOSOnboardingUrl(url?: string): boolean {
  return (
    url?.startsWith(BROWSEROS_ONBOARDING_URL) ||
    url?.startsWith('chrome://browseros/onboarding') ||
    false
  )
}

function isBrowserOSInternalUrl(url?: string): boolean {
  return url?.startsWith(BROWSEROS_INTERNAL_URL_PREFIX) ?? false
}

async function enforceSingleBrowserOSHomeTab() {
  const shouldOpenHome = await openBrowserOSHomeOnStartupStorage.getValue()
  if (!shouldOpenHome) return

  const allTabs = await chrome.tabs.query({})
  const browserOSTabs = allTabs.filter((tab) => isBrowserOSAppUrl(tab.url))

  if (browserOSTabs.length === 0) {
    const replaceableTab = allTabs.find((tab) =>
      shouldReplaceWithBrowserOSHome(tab.url),
    )
    if (replaceableTab?.id) {
      await chrome.tabs.update(replaceableTab.id, {
        url: BROWSEROS_HOME_URL,
        active: true,
      })
      if (replaceableTab.windowId !== undefined) {
        await chrome.windows
          .update(replaceableTab.windowId, { focused: true })
          .catch(() => null)
      }
      return
    }
    await chrome.tabs.create({ url: BROWSEROS_HOME_URL, active: true })
    return
  }

  let preferredTab =
    browserOSTabs.find((tab) => isBrowserOSHomeUrl(tab.url)) ?? browserOSTabs[0]

  // If only onboarding is open, convert it to home so startup always lands on main UI.
  if (isBrowserOSOnboardingUrl(preferredTab.url) && preferredTab.id) {
    await chrome.tabs
      .update(preferredTab.id, { url: BROWSEROS_HOME_URL, active: true })
      .catch(() => null)
    preferredTab = { ...preferredTab, url: BROWSEROS_HOME_URL }
  }

  if (preferredTab.id) {
    await chrome.tabs
      .update(preferredTab.id, { active: true })
      .catch(() => null)
  }
  if (preferredTab.windowId !== undefined) {
    await chrome.windows
      .update(preferredTab.windowId, { focused: true })
      .catch(() => null)
  }

  const redundantTabs = allTabs
    .filter((tab) => {
      if (shouldReplaceWithBrowserOSHome(tab.url)) return true
      return (
        isBrowserOSOnboardingUrl(tab.url) &&
        tab.id !== undefined &&
        tab.id !== preferredTab.id
      )
    })
    .map((tab) => tab.id)
    .filter((tabId): tabId is number => typeof tabId === 'number')

  if (redundantTabs.length > 0) {
    await chrome.tabs.remove(redundantTabs).catch(() => null)
  }
}

async function normalizeStaleOnboardingTabs() {
  const shownOnce = await onboardingShownOnceStorage.getValue()
  if (!shownOnce) return

  const allTabs = await chrome.tabs.query({})
  const onboardingTabs = allTabs.filter((tab) =>
    isBrowserOSOnboardingUrl(tab.url),
  )
  if (onboardingTabs.length === 0) return

  const [first, ...rest] = onboardingTabs
  if (first?.id) {
    await chrome.tabs
      .update(first.id, { url: BROWSEROS_HOME_URL, active: true })
      .catch(() => null)
  }

  const redundantIds = rest
    .map((tab) => tab.id)
    .filter((tabId): tabId is number => typeof tabId === 'number')

  if (redundantIds.length > 0) {
    await chrome.tabs.remove(redundantIds).catch(() => null)
  }
}

async function redirectOnboardingToHomeIfCompleted(
  tabId: number,
  url?: string,
) {
  if (!isBrowserOSOnboardingUrl(url) && !isBrowserOSInternalUrl(url)) return
  const shownOnce = await onboardingShownOnceStorage.getValue()
  if (!shownOnce) return
  await chrome.tabs.update(tabId, { url: BROWSEROS_HOME_URL }).catch(() => null)
}

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
      onboardingShownOnceStorage
        .getValue()
        .then((shownOnce) => {
          if (shownOnce) return
          chrome.tabs.create({ url: BROWSEROS_ONBOARDING_URL })
          onboardingShownOnceStorage.setValue(true)
        })
        .catch(() => null)
    }

    if (details.reason === chrome.runtime.OnInstalledReason.UPDATE) {
      checkAndShowChangelog().catch(() => null)
    }
  })

  chrome.runtime.onStartup.addListener(async () => {
    await normalizeStaleOnboardingTabs().catch(() => null)
    await enforceSingleBrowserOSHomeTab().catch(() => null)
    setTimeout(() => {
      normalizeStaleOnboardingTabs().catch(() => null)
      enforceSingleBrowserOSHomeTab().catch(() => null)
    }, 800)
    setTimeout(() => {
      normalizeStaleOnboardingTabs().catch(() => null)
      enforceSingleBrowserOSHomeTab().catch(() => null)
    }, 1800)
  })

  const closeRedundantNewTabIfNeeded = async (tabId: number) => {
    const shouldOpenHome = await openBrowserOSHomeOnStartupStorage.getValue()
    if (!shouldOpenHome) return

    const currentTab = await chrome.tabs.get(tabId).catch(() => null)
    if (!currentTab || !shouldReplaceWithBrowserOSHome(currentTab.url)) return

    const allTabs = await chrome.tabs.query({})
    const hasBrowserOSHome = allTabs.some((tab) => isBrowserOSAppUrl(tab.url))
    if (hasBrowserOSHome) {
      await chrome.tabs.remove(tabId).catch(() => null)
    }
  }

  // If BrowserOS home is already open, auto-close extra blank/new-tab pages.
  chrome.tabs.onCreated.addListener((tab) => {
    if (!tab.id) return
    setTimeout(() => {
      redirectOnboardingToHomeIfCompleted(tab.id as number, tab.url).catch(
        () => null,
      )
      closeRedundantNewTabIfNeeded(tab.id as number).catch(() => null)
      enforceSingleBrowserOSHomeTab().catch(() => null)
    }, 300)
  })

  chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
    if (!changeInfo.url && changeInfo.status !== 'complete') return
    redirectOnboardingToHomeIfCompleted(tabId, changeInfo.url).catch(() => null)
    normalizeStaleOnboardingTabs().catch(() => null)
    closeRedundantNewTabIfNeeded(tabId).catch(() => null)
    enforceSingleBrowserOSHomeTab().catch(() => null)
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
