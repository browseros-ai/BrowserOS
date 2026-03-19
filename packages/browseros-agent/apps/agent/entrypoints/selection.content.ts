import { selectedTextStorage } from '@/lib/selected-text/selectedTextStorage'

const MAX_SELECTED_TEXT_LENGTH = 5000

export default defineContentScript({
  matches: ['*://*/*'],
  runAt: 'document_idle',
  async main() {
    const response = await chrome.runtime.sendMessage({ type: 'get-tab-id' })
    const tabId: number | undefined = response?.tabId
    if (!tabId) return

    document.addEventListener('mouseup', () => {
      const text = window.getSelection()?.toString().trim()

      if (text && text.length > 0) {
        selectedTextStorage.setValue({
          text: text.slice(0, MAX_SELECTED_TEXT_LENGTH),
          pageUrl: window.location.href,
          pageTitle: document.title,
          tabId,
          timestamp: Date.now(),
        })
      } else {
        // User clicked without selecting — clear stale selection from this tab
        selectedTextStorage.getValue().then((current) => {
          if (current?.tabId === tabId) {
            selectedTextStorage.setValue(null)
          }
        })
      }
    })
  },
})
