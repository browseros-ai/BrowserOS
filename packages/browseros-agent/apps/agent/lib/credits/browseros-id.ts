import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'

const STORAGE_KEY = 'browseros_id'

async function readInstallIdFromPrefs(): Promise<string | null> {
  try {
    const adapter = getBrowserOSAdapter()
    const pref = await adapter.getPref(BROWSEROS_PREFS.INSTALL_ID)
    if (typeof pref?.value === 'string' && pref.value.length > 0) {
      return pref.value
    }
  } catch {
    // BrowserOS prefs API not available (non-BrowserOS Chrome)
  }
  return null
}

async function getFallbackId(): Promise<string> {
  const result = await chrome.storage.local.get(STORAGE_KEY)
  const existing = result[STORAGE_KEY]
  if (typeof existing === 'string' && existing.length > 0) {
    return existing
  }
  const id = crypto.randomUUID()
  await chrome.storage.local.set({ [STORAGE_KEY]: id })
  return id
}

export async function getOrCreateBrowserosId(): Promise<string> {
  const installId = await readInstallIdFromPrefs()
  if (installId) return installId
  return getFallbackId()
}
