import { selectedTextStorage } from '@/lib/selected-text/selectedTextStorage'

const MAX_SELECTED_TEXT_LENGTH = 5000

export default defineContentScript({
  matches: ['*://*/*'],
  runAt: 'document_idle',
  async main() {
    const response = await chrome.runtime.sendMessage({ type: 'get-tab-id' })
    const tabId: number | undefined = response?.tabId
    if (!tabId) return

    const key = String(tabId)

    document.addEventListener('mouseup', () => {
      const text = window.getSelection()?.toString().trim()

      if (text && text.length > 0) {
        selectedTextStorage.getValue().then((map) => {
          selectedTextStorage.setValue({
            ...map,
            [key]: {
              text: text.slice(0, MAX_SELECTED_TEXT_LENGTH),
              pageUrl: window.location.href,
              pageTitle: document.title,
              tabId,
              timestamp: Date.now(),
            },
          })
        })
      } else {
        // User clicked without selecting — clear this tab's entry only
        selectedTextStorage.getValue().then((map) => {
          if (map[key]) {
            const { [key]: _, ...rest } = map
            selectedTextStorage.setValue(rest)
          }
        })
      }
    })
  },
})
