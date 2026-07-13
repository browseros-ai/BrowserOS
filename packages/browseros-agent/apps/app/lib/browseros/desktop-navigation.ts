type DesktopRoute =
  | 'connect-apps'
  | 'settings/ai'
  | 'settings/usage'
  | 'settings/chat'
  | 'settings/mcp'
  | 'settings/customization'
  | 'settings/survey'
  | 'scheduled'
  | `scheduled?${string}`

/**
 * Opens an app surface inside the Electron shell when available.
 * The extension build falls back to opening its normal app route in a tab.
 */
export function openDesktopRoute(route: DesktopRoute): void {
  const runtime = chrome.runtime as typeof chrome.runtime & {
    openRoute?: (route: DesktopRoute) => Promise<void>
  }

  if (runtime.openRoute) {
    void runtime.openRoute(route)
    return
  }

  void chrome.tabs.create({
    url: chrome.runtime.getURL(`app.html#/${route}`),
  })
}

export function openDesktopSurface(surface: 'connect-apps' | 'settings'): void {
  openDesktopRoute(surface === 'connect-apps' ? 'connect-apps' : 'settings/ai')
}
