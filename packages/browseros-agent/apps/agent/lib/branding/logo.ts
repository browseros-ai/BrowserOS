export function getProductLogoUrl(): string {
  try {
    if (typeof chrome !== 'undefined' && chrome.runtime?.getURL) {
      return chrome.runtime.getURL('icon/128.png')
    }
  } catch {
    // Fall through to static path fallback.
  }

  return '/icon/128.png'
}
