import { buildAccentCssBlocks } from './accentPalette'

const STYLE_ID = 'shimmy-accent-theme-overrides'

function ensureStyleEl(): HTMLStyleElement {
  const existing = document.getElementById(STYLE_ID)
  if (existing instanceof HTMLStyleElement) return existing
  const el = document.createElement('style')
  el.id = STYLE_ID
  document.head.appendChild(el)
  return el
}

/**
 * Apply or clear custom accent overrides on `document.documentElement`.
 * @public
 */
export function mountAccentThemeCss(hex: string | null): void {
  if (!hex) {
    document.getElementById(STYLE_ID)?.remove()
    return
  }

  const { light, dark } = buildAccentCssBlocks(hex)
  if (!light || !dark) {
    document.getElementById(STYLE_ID)?.remove()
    return
  }

  const el = ensureStyleEl()
  /* ThemeProvider sets `light` or `dark` on <html>, not “absence of .dark”. */
  el.textContent = `html.light{${light}} html.dark{${dark}}`
}
