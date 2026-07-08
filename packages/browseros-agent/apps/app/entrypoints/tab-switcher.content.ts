import type { ContentScriptContext } from 'wxt/utils/content-script-context'
import { TabSwitcherController } from './tab-switcher/useTabSwitcher'

let controller: TabSwitcherController | null = null

const INJECTED_FLAG = '__browseros_tab_switcher'

export default defineContentScript({
  matches: ['*://*/*'],
  runAt: 'document_idle',
  main(ctx: ContentScriptContext) {
    if ((globalThis as Record<string, unknown>)[INJECTED_FLAG]) return
    ;(globalThis as Record<string, unknown>)[INJECTED_FLAG] = true

    controller = new TabSwitcherController(ctx)

    chrome.runtime.onMessage.addListener((message) => {
      if (message?.type === 'tab-switcher:toggle') {
        if (!controller) return
        if (controller.state.phase === 'idle') {
          controller.open(1)
        } else if (controller.state.phase === 'open') {
          controller.advance(1)
        }
      }
    })

    document.addEventListener('keyup', (e) => {
      if (e.key === 'Alt' && controller?.state.phase === 'open') {
        controller.select()
      }
    })
  },
})
