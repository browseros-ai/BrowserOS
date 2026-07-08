import { onSwitcherMessage } from '@/lib/messaging/switcher/switcherMessages'

const MRU_STORAGE_KEY = 'tabSwitcherMruList'
const THUMBNAIL_CACHE_MAX = 20

const thumbCache = new Map<number, string>()

function updateMru(tabId: number) {
  chrome.storage.session
    .get(MRU_STORAGE_KEY)
    .then(({ [MRU_STORAGE_KEY]: list }) => {
      const mruList: number[] = (list as number[]) ?? []
      const idx = mruList.indexOf(tabId)
      if (idx > -1) mruList.splice(idx, 1)
      mruList.unshift(tabId)
      chrome.storage.session.set({ [MRU_STORAGE_KEY]: mruList })
    })
}

function removeFromMru(tabId: number) {
  chrome.storage.session
    .get(MRU_STORAGE_KEY)
    .then(({ [MRU_STORAGE_KEY]: list }) => {
      const mruList: number[] = (list as number[]) ?? []
      const idx = mruList.indexOf(tabId)
      if (idx > -1) {
        mruList.splice(idx, 1)
        chrome.storage.session.set({ [MRU_STORAGE_KEY]: mruList })
      }
    })
}

async function captureThumbnail(tabId: number, windowId: number) {
  try {
    const dataUrl = await chrome.tabs.captureVisibleTab(windowId, {
      format: 'jpeg',
      quality: 30,
    })
    thumbCache.set(tabId, dataUrl)
    if (thumbCache.size > THUMBNAIL_CACHE_MAX) {
      const first = thumbCache.keys().next().value
      if (first !== undefined) thumbCache.delete(first)
    }
  } catch {
    // Permission denied or other error — skip thumbnail
  }
}

export function setupMruStore() {
  chrome.tabs.onActivated.addListener(({ tabId, windowId }) => {
    updateMru(tabId)
    captureThumbnail(tabId, windowId)
  })

  chrome.tabs.onRemoved.addListener((tabId) => {
    removeFromMru(tabId)
    thumbCache.delete(tabId)
  })

  onSwitcherMessage('tab-switcher:get-tabs', async () => {
    const allTabs = await chrome.tabs.query({})
    const { [MRU_STORAGE_KEY]: stored } =
      await chrome.storage.session.get(MRU_STORAGE_KEY)
    const order: number[] = (stored as number[]) ?? []

    const sorted = [...allTabs].sort((a, b) => {
      const aId = a.id ?? 0
      const bId = b.id ?? 0
      const ai = order.indexOf(aId)
      const bi = order.indexOf(bId)
      if (ai !== -1 && bi !== -1) return ai - bi
      if (ai !== -1) return -1
      if (bi !== -1) return 1
      return (
        (a.windowId ?? 0) - (b.windowId ?? 0) || (a.index ?? 0) - (b.index ?? 0)
      )
    })

    const windows = await chrome.windows.getAll()

    const thumbnails: Record<number, string> = {}
    thumbCache.forEach((value, key) => {
      thumbnails[key] = value
    })

    return {
      tabs: sorted,
      windows,
      thumbnails,
    }
  })

  onSwitcherMessage(
    'tab-switcher:switch',
    async ({
      data: { tabId, windowId },
    }: {
      data: { tabId: number; windowId: number }
    }) => {
      await chrome.tabs.update(tabId, { active: true })
      await chrome.windows.update(windowId, { focused: true })
    },
  )
}
