import { resolveBrowserOSServerBaseUrl } from '@/modules/api/browseros-ports'
import createExtensionUpdateApi, {
  type ExtensionUpdateApi,
} from '../../../../contracts/extension-updates/api.js'

/** Registers the durable update signal and the shared worker/message API.
 * The sidecar owns scheduling; notifications merely wake its independent CDP check.
 */
export function registerExtensionUpdates() {
  // A sidecar may have installed the compatibility API before background startup.
  // Reuse its queue so both entry points share the same activation guard.
  const worker = globalThis as typeof globalThis & {
    browserosExtensionUpdates?: ExtensionUpdateApi
  }
  worker.browserosExtensionUpdates ??= createExtensionUpdateApi(chrome)
  const api = worker.browserosExtensionUpdates

  const notify = async () => {
    const base = await resolveBrowserOSServerBaseUrl()
    const response = await fetch(`${base}/api/v1/extension/update-ready`, {
      method: 'POST',
      signal: AbortSignal.timeout(5000),
    })
    // Older sidecars lack this endpoint. Their later upgrade reconciles the
    // persisted native state, so a missed notification cannot lose the update.
    if (!response.ok && response.status !== 404) {
      throw new Error(`Update notification failed: ${response.status}`)
    }
  }
  const warn = (error: unknown) =>
    console.warn('Extension update check failed', error)
  chrome.runtime.onInstalled.addListener(({ reason }) => {
    if (reason === 'update') void api.restoreContentScripts().catch(warn)
  })
  chrome.runtime.onUpdateAvailable.addListener(({ version }) => {
    void api.recordUpdate(version).then(notify).catch(warn)
  })
  void api
    .getStatus()
    .then((status) => {
      if (status.pendingVersion) return notify()
    })
    .catch(warn)

  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    // This management API is for our extension pages, not content scripts or
    // externally_connectable peers. CDP uses the same API directly on the worker.
    if (
      sender.id !== chrome.runtime.id ||
      !sender.url?.startsWith(chrome.runtime.getURL(''))
    )
      return false
    if (!message || typeof message !== 'object') return false
    let result: Promise<unknown>
    if (message.type === 'extension-update.getStatus') {
      result = api.getStatus()
    } else if (
      message.type === 'extension-update.apply' &&
      typeof message.version === 'string'
    ) {
      result = api.applyPendingUpdate(message.version)
    } else return false
    void result
      .then((value) => sendResponse({ ok: true, value }))
      .catch((error) => sendResponse({ ok: false, error: String(error) }))
    return true
  })
}
