/** Alt text for brand image assets (`public/brand/wordmark.png`, `public/icon/*.png`). */
export const PRODUCT_LOGO_ALT = 'Shimmy-Browser' as const

/** Visible wordmark next to logo (header, onboarding, etc.). */
export const PRODUCT_DISPLAY_NAME = 'Shimmy-Browser' as const

const WORDMARK_PATH = 'brand/wordmark.png' as const

/**
 * Full brand image (repo `red_logo.png`, copied to `public/brand/wordmark.png`).
 * Use for in-app headers, auth, new tab, and provider chrome.
 */
export function getProductLogoUrl(): string {
  try {
    if (typeof chrome !== 'undefined' && chrome.runtime?.getURL) {
      return chrome.runtime.getURL(WORDMARK_PATH)
    }
  } catch {
    // Fall through to static path fallback.
  }

  return `/${WORDMARK_PATH}`
}

/**
 * Square toolbar / favicon-style icon (generated under `public/icon/`).
 */
export function getProductMarkUrl(size: 16 | 32 | 48 | 96 | 128 = 128): string {
  const path = `icon/${size}.png`
  try {
    if (typeof chrome !== 'undefined' && chrome.runtime?.getURL) {
      return chrome.runtime.getURL(path)
    }
  } catch {
    // Fall through
  }
  return `/${path}`
}
