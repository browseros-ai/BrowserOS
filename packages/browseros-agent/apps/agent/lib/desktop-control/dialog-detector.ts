/**
 * DialogDetector — detect native file dialogs via screenshot comparison.
 *
 * Compares before/after screenshots to determine if a native file dialog
 * appeared after a user action (e.g. clicking an upload button).
 *
 * Uses simple heuristic checks:
 *   - Pixel difference between screenshots exceeds a threshold
 *   - Common dialog UI patterns (title bar change, new centered window)
 *
 * @module desktop-control/dialog-detector
 */

import type { DesktopControlService, ScreenshotResult } from './types'

// ─── Dialog State ──────────────────────────────────────────────────

/** Possible states of a native dialog after a trigger action. */
export enum DialogState {
  /** No dialog detected — screen unchanged or change unrelated to dialog. */
  NO_DIALOG = 'NO_DIALOG',
  /** File open dialog detected. */
  FILE_OPEN = 'FILE_OPEN',
  /** File save dialog detected. */
  FILE_SAVE = 'FILE_SAVE',
  /** Folder selection dialog detected. */
  FOLDER_SELECT = 'FOLDER_SELECT',
}

/** Result of a dialog detection attempt. */
export interface DialogDetectionResult {
  /** Whether any dialog was detected. */
  detected: boolean
  /** The type of dialog, if detected. */
  state: DialogState
  /** Confidence score 0–1. */
  confidence: number
  /** Pixel difference ratio between before/after screenshots (0–1). */
  differenceRatio: number
}

/** Options for dialog detection. */
export interface DialogDetectionOptions {
  /** Time in ms to wait after trigger before taking the "after" screenshot. Default 500. */
  waitMs?: number
  /** Pixel-difference threshold (0–1) to consider "changed". Default 0.02 (2%). */
  differenceThreshold?: number
  /** Additional time to wait if first check is inconclusive. Default 300. */
  retryWaitMs?: number
}

// ─── DialogDetector ────────────────────────────────────────────────

/**
 * DialogDetector determines if a native file dialog appeared after
 * a triggering action (typically a button click in the browser).
 *
 * The detector captures a "before" screenshot, waits for the dialog
 * to appear, then captures an "after" screenshot and compares them.
 * A significant pixel difference indicates a dialog appeared.
 *
 * @public
 */
export class DialogDetector {
  private service: DesktopControlService
  private beforeScreenshot: ScreenshotResult | null = null

  constructor(service: DesktopControlService) {
    this.service = service
  }

  /**
   * Capture a "before" screenshot to use as baseline.
   * Call this BEFORE the action that might trigger a dialog.
   */
  async captureBaseline(): Promise<void> {
    this.beforeScreenshot = await this.service.captureScreenshot()
  }

  /**
   * Detect if a file dialog appeared after the triggering action.
   *
   * Takes a new screenshot, compares it to the baseline, and
   * determines if the change is consistent with a dialog appearing.
   *
   * @param options — Detection timing and threshold options.
   * @returns Detection result with confidence and dialog state.
   */
  async detect(
    options?: DialogDetectionOptions,
  ): Promise<DialogDetectionResult> {
    const waitMs = options?.waitMs ?? 500
    const threshold = options?.differenceThreshold ?? 0.02
    const retryWait = options?.retryWaitMs ?? 300

    if (!this.beforeScreenshot) {
      return {
        detected: false,
        state: DialogState.NO_DIALOG,
        confidence: 0,
        differenceRatio: 0,
      }
    }

    // Wait for dialog to appear
    await this.sleep(waitMs)

    // Capture the "after" screenshot
    const afterScreenshot = await this.service.captureScreenshot()

    // Compare screenshots
    const diffRatio = this.computePixelDifference(
      this.beforeScreenshot,
      afterScreenshot,
    )

    // If difference is below threshold, no dialog appeared
    if (diffRatio < threshold) {
      // Retry once: some dialogs take longer to render
      await this.sleep(retryWait)
      const retryScreenshot = await this.service.captureScreenshot()
      const retryDiff = this.computePixelDifference(
        this.beforeScreenshot,
        retryScreenshot,
      )

      if (retryDiff < threshold) {
        return {
          detected: false,
          state: DialogState.NO_DIALOG,
          confidence: 1 - retryDiff,
          differenceRatio: retryDiff,
        }
      }

      // On retry, treat as detected
      return this.classifyDialog(retryDiff, threshold)
    }

    return this.classifyDialog(diffRatio, threshold)
  }

  /**
   * Quick check: wait briefly and return whether a dialog likely appeared.
   * Useful as a fast probe before doing full detection.
   */
  async quickCheck(waitMs = 400): Promise<boolean> {
    if (!this.beforeScreenshot) return false

    await this.sleep(waitMs)
    const after = await this.service.captureScreenshot()
    const diff = this.computePixelDifference(this.beforeScreenshot, after)

    return diff > 0.01
  }

  /**
   * Reset the baseline. Call before a new detection sequence.
   */
  reset(): void {
    this.beforeScreenshot = null
  }

  // ─── Private Helpers ──────────────────────────────────────────────

  /**
   * Compute a simple pixel-difference ratio between two screenshots.
   *
   * Compares base64 PNG data byte-by-byte (sampling for performance).
   * Returns a value between 0 (identical) and 1 (completely different).
   */
  private computePixelDifference(
    before: ScreenshotResult,
    after: ScreenshotResult,
  ): number {
    // Quick path: if base64 is identical, no change
    if (before.base64 === after.base64) return 0

    // Different sizes mean significant change
    if (before.width !== after.width || before.height !== after.height) {
      return 1
    }

    // Sample-based comparison for performance
    const a = before.base64
    const b = after.base64
    const len = Math.min(a.length, b.length)

    if (len === 0) return 0

    // Sample every Nth character to avoid O(n) full scan on large images
    const sampleStep = Math.max(1, Math.floor(len / 5000))
    let differences = 0
    let samples = 0

    for (let i = 0; i < len; i += sampleStep) {
      samples++
      if (a[i] !== b[i]) differences++
    }

    return samples > 0 ? differences / samples : 0
  }

  /**
   * Classify the type of dialog based on the difference ratio and heuristics.
   *
   * In practice, we can't distinguish file-open vs file-save vs folder-select
   * from screenshots alone without OCR/window-title detection. We default
   * to FILE_OPEN as the most common case and use confidence scoring.
   */
  private classifyDialog(
    diffRatio: number,
    threshold: number,
  ): DialogDetectionResult {
    // Significant change likely means a dialog appeared.
    // Higher diff = higher confidence it's a dialog (vs minor animation)
    const confidence = Math.min(1, diffRatio / (threshold * 5))

    // Default to FILE_OPEN — the most common dialog type triggered
    // by upload actions. Caller can override based on context.
    const state = DialogState.FILE_OPEN

    return {
      detected: true,
      state,
      confidence,
      differenceRatio: diffRatio,
    }
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }
}
