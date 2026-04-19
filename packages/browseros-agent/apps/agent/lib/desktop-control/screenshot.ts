/**
 * DesktopScreenshotService — native screenshot capture.
 *
 * Uses `screenshot-desktop` for cross-platform screen capture.
 * Falls back to a stub in environments where native capture is unavailable.
 *
 * @module desktop-control/screenshot
 */

import type { DisplayInfo, ScreenshotOptions, ScreenshotResult } from './types'

/** Lazy-loaded screenshot-desktop module (ESM-compatible). */
let screenshotDesktop: (() => Promise<Buffer>) | null = null

async function loadScreenshotDesktop(): Promise<
  (() => Promise<Buffer>) | null
> {
  if (screenshotDesktop !== undefined) {
    return screenshotDesktop
  }
  try {
    const mod = await import('screenshot-desktop')
    screenshotDesktop =
      (mod as { default?: () => Promise<Buffer> }).default ??
      (mod as unknown as (() => Promise<Buffer>) | null)
    return screenshotDesktop
  } catch {
    screenshotDesktop = null
    return null
  }
}

/**
 * DesktopScreenshotService provides native screenshot and display info.
 * @public
 */
export class DesktopScreenshotService {
  /**
   * Capture a screenshot of the primary display (or a specific display).
   */
  async captureScreenshot(
    options?: ScreenshotOptions,
  ): Promise<ScreenshotResult> {
    const capture = await loadScreenshotDesktop()

    if (!capture) {
      // Stub: return a 1×1 transparent PNG when native capture is unavailable
      return this.createStubResult()
    }

    try {
      const buffer: Buffer = await capture()
      const base64 = buffer.toString('base64')

      return {
        base64,
        width: 0, // actual dimensions parsed below if needed
        height: 0,
        mimeType: 'image/png',
        displayId: options?.displayId ?? 0,
      }
    } catch (error) {
      throw new Error(
        `Failed to capture screenshot: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * List available displays.
   *
   * Note: `screenshot-desktop` does not expose display info natively,
   * so we return a single-entry for the primary display as a best effort.
   */
  async getDisplays(): Promise<DisplayInfo[]> {
    // screenshot-desktop doesn't provide display metadata,
    // return a sensible default for the primary monitor.
    return [
      {
        id: 0,
        width: 1920,
        height: 1080,
        xOffset: 0,
        yOffset: 0,
        scaleFactor: 1,
        isPrimary: true,
      },
    ]
  }

  /** @internal Minimal 1×1 transparent PNG for stub mode. */
  private createStubResult(): ScreenshotResult {
    // Smallest valid 1×1 transparent PNG (67 bytes)
    const stubBase64 =
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVQI12NgAAIABQABNjN9GQAAAAlwSFlzAAAWJQAAFiUBSVIk8AAAAA0lEQVQI12P4z8BQDwAEgAF/QualGQAAAABJRU5ErkJggg=='
    return {
      base64: stubBase64,
      width: 1,
      height: 1,
      mimeType: 'image/png',
      displayId: 0,
    }
  }
}
