import { storage } from '@wxt-dev/storage'
import { sessionStorage } from '@/lib/auth/sessionStorage'
import { markRestorePending } from '@/lib/browseros/activeSessionStorage'
import { Capabilities } from '@/lib/browseros/capabilities'
import { getHealthCheckUrl, getMcpServerUrl } from '@/lib/browseros/helpers'
import {
  migrateSidePanelIfOpenBetweenTabs,
  openSidePanel,
  SHIMMY_AGENT_SIDEPANEL_BUSY_KEY,
  SHIMMY_SIDEPANEL_LAST_TAB_KEY,
  toggleSidePanel,
  ensureSidePanelRuntimeStateLoaded,
  initializeSidePanelOptions,
  registerSidePanelOpenStateListeners,
  setSidePanelPerWindowPreference,
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
import {
  authRedirectPathStorage,
  onboardingShownOnceStorage,
} from '@/lib/onboarding/onboardingStorage'
import { syncOnboardingProfile } from '@/lib/onboarding/syncOnboardingProfile'
import {
  setupScheduledJobsSyncToBackend,
  syncScheduledJobs,
} from '@/lib/schedules/syncSchedulesToBackend'
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
  if (url.startsWith('chrome-extension://') && !isBrowserOSAppUrl(url)) {
    // Stale/foreign extension pages can show ERR_BLOCKED_BY_CLIENT at startup.
    // Treat them like replaceable new-tab placeholders.
    return true
  }
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

/** Cold start: session restore may reopen only a deep link (e.g. workflows). Land on home instead. */
function shouldResetAppTabToHomeOnStartup(url?: string): boolean {
  if (!isBrowserOSAppUrl(url)) return false
  if (isBrowserOSHomeUrl(url)) return false
  if (isBrowserOSOnboardingUrl(url)) return false
  return true
}

/** Tabs opened via + / Ctrl+T (child of another tab) or initial about:blank — must not be auto-closed. */
function isLikelyUserInitiatedBlankOrChildTab(tab: chrome.tabs.Tab): boolean {
  if (tab.openerTabId != null) return true
  const url = tab.url ?? tab.pendingUrl ?? ''
  if (url === 'about:blank' || url === '') return true
  return false
}

async function enforceSingleBrowserOSHomeTab(options?: {
  /** When false, do not steal focus (e.g. user opened a new tab with +). */
  activatePreferred?: boolean
}) {
  const activatePreferred = options?.activatePreferred ?? true
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
        active: activatePreferred,
      })
      if (activatePreferred && replaceableTab.windowId !== undefined) {
        await chrome.windows
          .update(replaceableTab.windowId, { focused: true })
          .catch(() => null)
      }
      return
    }
    await chrome.tabs.create({
      url: BROWSEROS_HOME_URL,
      active: activatePreferred,
    })
    return
  }

  let preferredTab =
    browserOSTabs.find((tab) => isBrowserOSHomeUrl(tab.url)) ?? browserOSTabs[0]

  // If only onboarding is open, convert it to home so startup always lands on main UI.
  if (isBrowserOSOnboardingUrl(preferredTab.url) && preferredTab.id) {
    await chrome.tabs
      .update(preferredTab.id, {
        url: BROWSEROS_HOME_URL,
        active: activatePreferred,
      })
      .catch(() => null)
    preferredTab = { ...preferredTab, url: BROWSEROS_HOME_URL }
  }

  // Session restore can leave the active tab on workflows/settings/etc. with no #/home tab.
  if (
    activatePreferred &&
    preferredTab.id &&
    shouldResetAppTabToHomeOnStartup(preferredTab.url)
  ) {
    await chrome.tabs
      .update(preferredTab.id, {
        url: BROWSEROS_HOME_URL,
        active: true,
      })
      .catch(() => null)
    preferredTab = { ...preferredTab, url: BROWSEROS_HOME_URL }
  }

  if (activatePreferred && preferredTab.id) {
    await chrome.tabs
      .update(preferredTab.id, { active: true })
      .catch(() => null)
  }
  if (activatePreferred && preferredTab.windowId !== undefined) {
    await chrome.windows
      .update(preferredTab.windowId, { focused: true })
      .catch(() => null)
  }

  const redundantTabs = allTabs
    .filter((tab) => {
      if (isLikelyUserInitiatedBlankOrChildTab(tab)) return false
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

  /** Keep the AI side panel open when automation changes the active tab (CDP). */
  chrome.tabs.onActivated.addListener((activeInfo) => {
    const previousTabId = (
      activeInfo as Parameters<
        Parameters<typeof chrome.tabs.onActivated.addListener>[0]
      >[0] & { previousTabId?: number }
    ).previousTabId

    chrome.storage.session
      .get([SHIMMY_AGENT_SIDEPANEL_BUSY_KEY, SHIMMY_SIDEPANEL_LAST_TAB_KEY])
      .then((v) => {
        const agentBusy = Boolean(v[SHIMMY_AGENT_SIDEPANEL_BUSY_KEY])
        return migrateSidePanelIfOpenBetweenTabs(
          activeInfo.tabId,
          previousTabId,
          agentBusy,
        )
      })
      .catch(() => null)
  })

  /**
   * When the agent creates a brand-new tab and navigates it, onActivated fires
   * before the tab has a real URL, so sidePanel.open can fail. Re-try once the
   * tab reaches 'complete' status so the panel follows reliably.
   */
  chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
    if (changeInfo.status !== 'complete') return
    if (!tab.active) return
    chrome.storage.session
      .get([SHIMMY_AGENT_SIDEPANEL_BUSY_KEY, SHIMMY_SIDEPANEL_LAST_TAB_KEY])
      .then((v) => {
        const agentBusy = Boolean(v[SHIMMY_AGENT_SIDEPANEL_BUSY_KEY])
        const lastOpenTabId = v[SHIMMY_SIDEPANEL_LAST_TAB_KEY] as
          | number
          | undefined
        if (!agentBusy && lastOpenTabId !== tabId) return
        if (lastOpenTabId === tabId) return
        markRestorePending()
        return openSidePanel(tabId)
      })
      .catch(() => null)
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
      initializeSidePanelOptions().catch(() => null)
      onboardingShownOnceStorage
        .getValue()
        .then((shownOnce) => {
          if (shownOnce) return
          chrome.tabs.create({ url: chrome.runtime.getURL('app.html#/onboarding') })
          onboardingShownOnceStorage.setValue(true)
        })
        .catch(() => null)
    }

    if (details.reason === chrome.runtime.OnInstalledReason.UPDATE) {
      cleanupLegacyToolApprovalStorage().catch(() => null)
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
    if (
      !currentTab ||
      isLikelyUserInitiatedBlankOrChildTab(currentTab) ||
      !shouldReplaceWithBrowserOSHome(currentTab.url)
    ) {
      return
    }

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
      enforceSingleBrowserOSHomeTab({ activatePreferred: false }).catch(
        () => null,
      )
    }, 300)
  })

  chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
    if (!changeInfo.url && changeInfo.status !== 'complete') return
    redirectOnboardingToHomeIfCompleted(tabId, changeInfo.url).catch(() => null)
    normalizeStaleOnboardingTabs().catch(() => null)
    closeRedundantNewTabIfNeeded(tabId).catch(() => null)
    enforceSingleBrowserOSHomeTab({ activatePreferred: false }).catch(
      () => null,
    )
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
