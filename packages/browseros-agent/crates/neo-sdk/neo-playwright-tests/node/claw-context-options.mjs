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

/** Lend the real context once; every unsupported request calls the real Browser. */
export function createClawContextFactory({
  browser,
  context,
  combined,
  implicit,
  video,
  testInfo,
}) {
  let calls = 0
  return async (requested) => {
    const explicit = clawContextOptions(combined, requested, implicit)
    if ((typeof video === 'string' ? video : video.mode) !== 'off')
      explicit.recordVideo = { dir: testInfo.outputPath('video') }
    calls++
    const forced = Object.keys(explicit).length
      ? `explicit options: ${JSON.stringify(explicit)}`
      : calls > 1
        ? 'second context in one test'
        : null
    if (forced) {
      testInfo.annotations.push({ type: 'claw-context', description: forced })
      const created = await browser.newContext(explicit)
      return { context: created, close: () => created.close() }
    }
    return { context, close: async () => {} }
  }
}
