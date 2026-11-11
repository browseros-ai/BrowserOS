/**
 * Screenshot client work around capture: infer the format first, then persist the returned
 * bytes. Saving is a separate targetless write so capture remains readable during takeover.
 */
import { invoke } from './bridge'
import { decode } from './encodings'
import { plainError } from './errors'
import type { SignalLike } from './signal'
import { getMimeTypeForPath } from './vendored'
import type { ScreenshotOptions } from './wire'

/** Playwright's MIME table, with the case-insensitive filename inference added in 1.66. */
export function screenshotType({
  api,
  path,
}: {
  api: string
  path: string
}): ScreenshotOptions['type'] {
  // Fold only the lookup key. The file write keeps the caller's exact destination spelling.
  const mime = getMimeTypeForPath(path.toLowerCase())
  if (mime === 'image/png') return 'png'
  if (mime === 'image/jpeg') return 'jpeg'
  if (mime === 'image/webp') return 'webp'
  throw plainError(api, `path: unsupported mime type "${mime}"`)
}

/** Await the product's filesystem write before returning the original screenshot Buffer. */
export async function saveScreenshot({
  api,
  bytes,
  path,
  signal,
}: {
  api: string
  bytes: Uint8Array
  path: string | undefined
  signal: SignalLike | undefined
}): Promise<void> {
  if (!path) return
  // Upstream writes after the channel's capture timeout has ended. This step keeps the
  // run budget and cancellation, without starting another screenshot timeout for disk I/O.
  await invoke(
    'page.saveScreenshot',
    { api, timeoutMs: 0 },
    { path, base64: decode(bytes, 'base64') },
    signal,
  )
}
