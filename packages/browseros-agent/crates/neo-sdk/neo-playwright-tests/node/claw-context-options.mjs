/** Defaults of the pinned Playwright Test fixtures, verified against the owned v1 browser. */
import { isDeepStrictEqual } from 'node:util'

export const clawDefaults = {
  acceptDownloads: true,
  bypassCSP: false,
  colorScheme: 'light',
  contrast: 'no-preference',
  forcedColors: 'none',
  hasTouch: false,
  ignoreHTTPSErrors: false,
  isMobile: false,
  javaScriptEnabled: true,
  locale: 'en-US',
  offline: false,
  reducedMotion: 'no-preference',
  serviceWorkers: 'allow',
  viewport: { width: 1280, height: 720 },
}

export function clawContextOptions(combined, requested, implicit) {
  const options = { ...combined, ...requested }
  for (const [name, value] of Object.entries(clawDefaults)) {
    if (
      implicit?.has(name) &&
      !Object.hasOwn(requested || {}, name) &&
      isDeepStrictEqual(options[name], value)
    )
      delete options[name]
  }
  return options
}
