declare namespace chrome.browserOS {
  interface PrefObject {
    key: string
    type: string
    value: unknown
  }

  interface RequestAttentionOptions {
    /** Keep bouncing until BrowserOS is the active app. */
    critical?: boolean
  }

  function getPref(name: string, callback: (pref: PrefObject) => void): void

  /** Present from the BrowserOS build that adds the attention API. */
  function requestAttention(
    options: RequestAttentionOptions,
    callback: (requested: boolean) => void,
  ): void
  function cancelAttention(callback: () => void): void
  function setDockBadge(text: string, callback: (set: boolean) => void): void
}
